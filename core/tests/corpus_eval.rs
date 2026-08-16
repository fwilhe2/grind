// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop B, second half — the formula scoreboard, and phase 4's own plan.
//!
//! Every fixture in LibreOffice's `functions/` corpus is a sheet of formulas beside the
//! values LO computed for them. Recalculating each formula from scratch and comparing
//! against that cached value is a conformance test we did not have to write: 509 files,
//! one per function, already categorised the way Part 4 categorises functions.
//!
//! The number goes up and never down — [`FLOOR`] is the ratchet, and the scoreboard printed
//! alongside it says which function to implement next.
//!
//!     SHEET_LO_CORPUS=/path/to/libreoffice/core/sc/qa/unit/data cargo test

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sheet_core::formula::eval::{Address, Engine};
use sheet_core::formula::value::{FormulaError, Value};
use sheet_core::{CellValue, Document};

const DEFAULT_CORPUS: &str = "/home/florian/code/github.com/LibreOffice/core/sc/qa/unit/data";

/// Cells that must keep matching. Raise it when the scoreboard rises; never lower it.
///
/// At 13197 with all 110 of the Small Group written. The gap to 52213 is mostly `missing` —
/// fixtures for functions outside the Small Group entirely (`FOURIER`, `LINEST`, the whole
/// `addin` category) — so the number to watch is the scoreboard's `wrong` column, not this
/// one.
const FLOOR: usize = 13_200;

fn corpus_root() -> Option<PathBuf> {
    let root = PathBuf::from(
        std::env::var("SHEET_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CORPUS.to_owned()),
    );
    root.is_dir().then_some(root)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("fods" | "ods")
        ) {
            out.push(path);
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Tally {
    matched: usize,
    /// Answered with `#NAME?` where the document has a value: a function we do not have.
    missing: usize,
    wrong: usize,
    /// Reads the clock, so the cached value is a claim about the day the fixture was
    /// saved and no evaluator can reproduce it. See [`volatile`].
    volatile: usize,
}

/// Does this formula read the clock?
///
/// `NOW` and `TODAY` (§6.10.16, §6.10.20) are the Small Group's only two functions that
/// are not a pure function of the document, so a fixture's cached value for one is the
/// instant LibreOffice last recalculated it. Counting those as `wrong` would put cells
/// that *cannot* agree into the one column that means something is broken.
///
/// Named as a construct rather than as a file, and it is the whole list — anything else
/// that disagrees still counts against us.
fn volatile(formula: &str) -> bool {
    let upper = formula.to_uppercase();
    upper.contains("TODAY(") || upper.contains("NOW(")
}

/// Is the recalculated value the one the document already carries?
///
/// Doubles compare at 15 significant digits for loop C's reason (doc/ods-format.md §3.4):
/// that is all LibreOffice writes, so a tighter comparison would test LO's serialiser.
///
/// Errors need a translation. §4.6 says an error result is *stored as a string*, so LO's
/// cached `#DIV/0!` arrives as text — matching it against a computed error is reading the
/// document as it was meant, not a loosening.
fn agrees(stored: &CellValue, computed: &Value) -> bool {
    match (stored, computed) {
        (CellValue::Number(x), Value::Number(y)) => {
            x == y || (x - y).abs() <= 1e-14 * x.abs().max(y.abs())
        }
        (CellValue::Text(s), Value::Error(e)) => {
            FormulaError::from_name(s) == Some(*e)
            // LO also writes errors as `Err:502`, a numeric code with no name in §5.12.
            // Which of the seven that maps onto is not knowable, so any error agrees.
            || s.starts_with("Err:")
        }
        // An empty string result has no distinct serialisation — LO writes the cell blank —
        // so a stored empty cell and a computed "" are the same document.
        (CellValue::Empty, Value::Text(s)) => s.is_empty(),
        (CellValue::Text(s), Value::Text(t)) => s == t,
        (CellValue::Bool(a), Value::Bool(b)) => a == b,
        // LO stores logicals as floats in plenty of documents, and `TRUE` is 1.
        (CellValue::Number(x), Value::Bool(b)) => *x == u8::from(*b) as f64,
        (CellValue::Empty, Value::Empty) => true,
        _ => false,
    }
}

/// Which function a fixture is about. `functions/text/fods/left.fods` is `LEFT`.
fn subject(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_uppercase()
}

fn category(path: &Path) -> String {
    path.ancestors()
        .nth(2)
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_owned()
}

#[test]
fn recalculating_the_corpus_agrees_with_libreoffice() {
    let Some(root) = corpus_root() else {
        eprintln!(
            "skipping: no LibreOffice corpus at {DEFAULT_CORPUS}; \
             set SHEET_LO_CORPUS to run loop B"
        );
        return;
    };

    let mut files = Vec::new();
    collect(&root.join("functions"), &mut files);
    files.sort();
    assert!(!files.is_empty(), "corpus at {} is empty", root.display());

    // A work list is only useful if you can then look at one row of it:
    //     SHEET_LOOP_B_DUMP=LOG cargo test --test corpus_eval -- --nocapture
    let dump = std::env::var("SHEET_LOOP_B_DUMP")
        .ok()
        .map(|s| s.to_uppercase());

    let mut by_category: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_function: BTreeMap<String, Tally> = BTreeMap::new();
    for path in &files {
        let Ok(doc) = sheet_core::read_file(path) else {
            continue;
        };
        let tally = check(&doc, dump.as_deref() == Some(subject(path).as_str()));
        let category = by_category.entry(category(path)).or_default();
        category.matched += tally.matched;
        category.missing += tally.missing;
        category.wrong += tally.wrong;
        category.volatile += tally.volatile;
        let function = by_function.entry(subject(path)).or_default();
        function.matched += tally.matched;
        function.missing += tally.missing;
        function.wrong += tally.wrong;
        function.volatile += tally.volatile;
    }

    let total = |f: fn(&Tally) -> usize| by_category.values().map(f).sum::<usize>();
    let (matched, missing, wrong, volatile) = (
        total(|t| t.matched),
        total(|t| t.missing),
        total(|t| t.wrong),
        total(|t| t.volatile),
    );
    eprintln!(
        "loop B (evaluate): {} formula cells in {} fixtures — {matched} match, \
         {missing} need a function we do not have, {wrong} disagree, {volatile} read the clock",
        matched + missing + wrong + volatile,
        files.len(),
    );
    eprintln!(
        "  {:<16} {:>7} {:>8} {:>7} {:>9}",
        "category", "match", "missing", "wrong", "volatile"
    );
    for (category, tally) in &by_category {
        eprintln!(
            "  {category:<16} {:>7} {:>8} {:>7} {:>9}",
            tally.matched, tally.missing, tally.wrong, tally.volatile
        );
    }

    // The work list: what we claim to implement and still get wrong, worst first.
    let mut worst: Vec<_> = by_function
        .iter()
        .filter(|(_, t)| t.wrong > 0)
        .map(|(name, t)| (t.wrong, name))
        .collect();
    worst.sort_unstable_by(|a, b| b.cmp(a));
    eprintln!("  disagreements by fixture:");
    for (wrong, name) in worst.iter().take(15) {
        eprintln!("    {wrong:>5}  {name}");
    }

    assert!(
        matched >= FLOOR,
        "loop B went backwards: {matched} cells match, the floor is {FLOOR}"
    );
}

fn check(doc: &Document, dump: bool) -> Tally {
    let mut tally = Tally::default();
    for (index, sheet) in doc.sheets.iter().enumerate() {
        let formulas: Vec<_> = sheet
            .formulas()
            .map(|(pos, formula)| (pos, sheet.get(pos), formula.to_owned()))
            .collect();
        let mut engine = Engine::new(doc);
        for (pos, stored, formula) in formulas {
            let computed = engine.value(Address::new(index, pos));
            if agrees(&stored, &computed) {
                tally.matched += 1;
            } else if volatile(&formula) {
                tally.volatile += 1;
            } else if computed == Value::Error(FormulaError::Name) {
                // Either a function we have not written or a name we cannot resolve. Both
                // are "not implemented yet" rather than "computed the wrong answer" — the
                // distinction that makes this a work list instead of a red light.
                tally.missing += 1;
            } else {
                tally.wrong += 1;
                if dump {
                    eprintln!(
                        "    {}.{}{}: {formula}\n      want {stored:?}, got {computed:?}",
                        sheet.name,
                        (b'A' + (pos.col % 26) as u8) as char,
                        pos.row + 1,
                    );
                }
            }
        }
    }
    tally
}
