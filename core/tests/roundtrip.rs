// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop C — round-trip differential against LibreOffice.
//!
//! The strictest of the three loops, and the only one that checks the *writer*. Reading
//! the spec tells you what is legal; only LibreOffice tells you what it does with it, and
//! the gap between those two is where a writer's bugs live.
//!
//! Both directions:
//!
//! * **out** — build a document, write it, have LO convert it, read LO's output back, and
//!   assert we get the same document. Catches everything we emit that LO drops, collapses
//!   or reinterprets.
//! * **back** — take an LO-authored file, read it, write it, convert *that*, read it back,
//!   and assert it still matches what we first read. Catches the writer losing something
//!   the reader understood.
//!
//! This is also the enforcement mechanism for the anti-bloat rule (doc/plan.md, rule 7): a
//! feature that does not survive this fails CI, so the feature line is defended by a
//! machine rather than by discipline.
//!
//! Needs `soffice` on `PATH`; skips with a notice without one.

use std::path::{Path, PathBuf};
use std::process::Command;

use sheet_core::model::NumberKind;
use sheet_core::numfmt::{Format, Kind, Part};
use sheet_core::{CellValue, Document, Form, Pos, Sheet};

const DEFAULT_CORPUS: &str = "/home/florian/code/github.com/LibreOffice/core/sc/qa/unit/data";

/// How many corpus documents the "back" direction takes. Each soffice conversion is
/// seconds, so this is a sample rather than the whole corpus — loop A already reads all
/// 361, and what is under test here is the writer, which does not vary per file.
const SAMPLE: usize = 20;

/// Skip corpus documents bigger than this, measured as rows × columns. A cell-by-cell
/// comparison over a sheet the size of a whole grid is not a better test, only a slower one.
const MAX_COMPARED_CELLS: u64 = 200_000;

// --- driving LibreOffice ---------------------------------------------------------------

/// A scratch directory that cleans itself up.
struct Lab {
    dir: PathBuf,
}

impl Lab {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sheet-loop-c-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        std::fs::create_dir_all(dir.join("out")).unwrap();
        Self { dir }
    }

    fn input(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join("in").join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Convert every staged input to flat XML in **one** soffice invocation.
    ///
    /// One invocation because startup dominates: a couple of seconds each time, against
    /// milliseconds per document once it is up. The private `UserInstallation` profile is
    /// not optional — without it this fights the developer's own running LibreOffice for
    /// the profile lock and either blocks or silently does nothing.
    fn convert(&self, inputs: &[PathBuf]) -> PathBuf {
        let out = self.dir.join("out");
        let status = Command::new("soffice")
            .arg("--headless")
            .arg(format!(
                "-env:UserInstallation=file://{}",
                self.dir.join("profile").display()
            ))
            .args(["--convert-to", "fods", "--outdir"])
            .arg(&out)
            .args(inputs)
            .status()
            .expect("soffice failed to start");
        assert!(status.success(), "soffice exited with {status}");
        out
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn have_soffice() -> bool {
    Command::new("soffice")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Read back what LO produced for one staged input, by name.
fn converted(out: &Path, input: &Path) -> Document {
    let name = input.file_stem().unwrap().to_str().unwrap();
    let path = out.join(format!("{name}.fods"));
    assert!(
        path.exists(),
        "LibreOffice produced no output for {}: it could not open what we wrote",
        input.display()
    );
    sheet_core::read_file(&path).unwrap_or_else(|e| panic!("re-reading {}: {e}", path.display()))
}

// --- comparison ------------------------------------------------------------------------

/// Are these the same value, to the precision LibreOffice is capable of preserving?
///
/// The one loosening in this loop, named here rather than hidden in a tolerance constant.
/// LO writes every ODF double at 15 significant digits, where a double needs 17 to
/// round-trip (doc/ods-format.md §3.4, citing `sal/rtl/math.cxx:364-366` — LO's own reader
/// special-cases the `DBL_MAX` its writer mangles). Comparing exactly here would not be a
/// stricter test of our writer; it would be a test of LibreOffice's, and one it fails on
/// `1/3`. Everything else compares exactly.
///
/// The tolerance follows from the 15 digits: rounding there costs up to half an ulp at
/// that precision, which is a relative 5e-15 when the leading digit is 1. `1e-14` is that
/// with one factor of two in hand — still four orders of magnitude tighter than any real
/// writer bug, which loses a digit or a decimal point rather than a fifteenth place.
fn same(a: &CellValue, b: &CellValue) -> bool {
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => {
            x == y || (x - y).abs() <= 1e-14 * x.abs().max(y.abs())
        }
        _ => a == b,
    }
}

/// Every way two documents differ, as sentences. Not `assert_eq!` on the whole document:
/// the interesting output is *which cell* moved, and a `Debug` dump of two sheets is not
/// something anyone reads.
fn differences(label: &str, want: &Document, got: &Document) -> Vec<String> {
    let mut out = Vec::new();
    if want.sheets.len() != got.sheets.len() {
        out.push(format!(
            "{label}: {} sheets in, {} out",
            want.sheets.len(),
            got.sheets.len()
        ));
        return out;
    }
    // §5.11 names, which are document-level rather than per-sheet.
    if want.names != got.names {
        out.push(format!(
            "{label}: names {:?}, back as {:?}",
            want.names, got.names
        ));
    }
    // The epoch (§3.4 `HOST-NULL-DATE`), which changes what every serial number *means* —
    // a document that came back with a different one has silently moved all its dates.
    if (want.null_date, want.null_year) != (got.null_date, got.null_year) {
        out.push(format!(
            "{label}: epoch {:?}, back as {:?}",
            (want.null_date, want.null_year),
            (got.null_date, got.null_year)
        ));
    }
    for (i, (w, g)) in want.sheets.iter().zip(&got.sheets).enumerate() {
        if w.name != g.name {
            out.push(format!(
                "{label}: sheet {i} named {:?}, back as {:?}",
                w.name, g.name
            ));
        }
        let rows = w.used_rows().max(g.used_rows());
        let cols = w.used_cols().max(g.used_cols());
        for row in 0..rows {
            for col in 0..cols {
                let pos = Pos::new(row, col);
                if !same(&w.get(pos), &g.get(pos)) {
                    out.push(format!(
                        "{label}: sheet {i} r{row}c{col} was {:?}, back as {:?}",
                        w.get(pos),
                        g.get(pos)
                    ));
                }
                if w.formula(pos) != g.formula(pos) {
                    out.push(format!(
                        "{label}: sheet {i} r{row}c{col} formula was {:?}, back as {:?}",
                        w.formula(pos),
                        g.formula(pos)
                    ));
                }
                // §4.3.2/§4.3.3: a date and a time are Numbers, so a lost `NumberKind` is
                // invisible to `same` above — and is exactly how a date document comes back
                // as a column of five-digit integers.
                if w.kind(pos) != g.kind(pos) {
                    out.push(format!(
                        "{label}: sheet {i} r{row}c{col} kind was {:?}, back as {:?}",
                        w.kind(pos),
                        g.kind(pos)
                    ));
                }
                // §5, compared as the text the user sees rather than as a `Format` struct.
                // Style *names* are LibreOffice's to renumber, and the struct is one step
                // too literal in the other direction: a date cell we write with no format of
                // its own comes back carrying the ISO date style the writer supplies for it,
                // which is not a difference in what the document says. What must not change
                // is the rendering.
                let (before, after) = (shown(w, pos, want), shown(g, pos, got));
                if before != after {
                    out.push(format!(
                        "{label}: sheet {i} r{row}c{col} displayed {before:?}, back as \
                         {after:?}"
                    ));
                }
                if out.len() > 20 {
                    out.push(format!("{label}: ... and more"));
                    return out;
                }
            }
        }
    }
    out
}

/// What a cell displays: through its number format, or through the plain spelling of its
/// value when it has none.
fn shown(sheet: &Sheet, pos: Pos, doc: &Document) -> String {
    match sheet.format(pos) {
        Some(format) => format.render(&sheet.get(pos), doc.null_date),
        None => sheet_core::numfmt::general(&sheet.get(pos), sheet.kind(pos), doc.null_date),
    }
}

// --- direction "out": ours -> LibreOffice -> ours ---------------------------------------

/// One named document to push through LO, built cell by cell.
fn case(name: &str, cells: &[(u32, u32, CellValue)]) -> (String, Document) {
    let mut doc = Document {
        sheets: vec![Sheet::new("Data")],
        ..Default::default()
    };
    let sheet = doc.sheet_mut(0).unwrap();
    for (row, col, value) in cells {
        sheet.set(Pos::new(*row, *col), value.clone());
    }
    (name.to_owned(), doc)
}

/// Dates and times — §4.3.2, §4.3.3, and the reason [`Sheet::kind`] exists.
///
/// A date is stored as a Number, so every one of these cells would survive `same()` as a
/// bare float. The check that matters is the `NumberKind` beside it: if LibreOffice gets a
/// serial number where the document meant a date, the user sees 30347 instead of a date and
/// the feature is not real.
fn dates() -> (String, Document) {
    let e = sheet_core::formula::date::DEFAULT_NULL_DATE;
    let day = |y, m, d| sheet_core::formula::date::serial(y, m, d, e);

    let mut doc = Document {
        sheets: vec![Sheet::new("Data")],
        ..Default::default()
    };
    let sheet = doc.sheet_mut(0).unwrap();
    for (row, col, value, kind) in [
        (0, 0, day(1983, 1, 31), NumberKind::Date),
        // The epoch itself, and the day either side of it — where an off-by-one lands.
        (0, 1, day(1899, 12, 30), NumberKind::Date),
        (0, 2, day(1899, 12, 31), NumberKind::Date),
        // Before the epoch: a negative serial, which §4.3.3 says evaluators *may* support
        // and which the corpus contains.
        (0, 3, day(1899, 1, 1), NumberKind::Date),
        // 1900 is not a leap year here; a reader that thinks it is lands a day out.
        (0, 4, day(1900, 3, 1), NumberKind::Date),
        // A date carrying a time is a DateTime (§4.3.4) and takes the combined spelling.
        (1, 0, day(2026, 8, 16) + 0.5, NumberKind::Date),
        (2, 0, 0.5, NumberKind::Time),
        (2, 1, 0.0, NumberKind::Time),
        // Sub-second precision, and a duration longer than a day — both real corpus cells
        // and both things a naive formatter quietly rounds away.
        (
            2,
            2,
            (5.0 * 3600.0 + 35.0 * 60.0 + 31.2) / 86_400.0,
            NumberKind::Time,
        ),
        (2, 3, 1.40625, NumberKind::Time),
    ] {
        sheet.set(Pos::new(row, col), CellValue::Number(value));
        sheet.set_kind(Pos::new(row, col), kind);
    }
    ("dates".to_owned(), doc)
}

/// Number formats — §5.2, and the phase that makes a date print as a date.
///
/// Every family in the Small Group's reach, each on a cell whose *value* also has to
/// survive: a format that comes back changed shows the user something else, and a format
/// that comes back attached to a different cell is worse.
fn formats() -> (String, Document) {
    let mut doc = Document {
        sheets: vec![Sheet::new("Data")],
        ..Default::default()
    };
    let epoch = doc.null_date;

    let number = |decimals, min_decimals, grouping| {
        let mut f = Format::new(Kind::Number);
        f.push(Part::Number {
            decimals,
            min_decimals,
            min_int: 1,
            grouping,
        });
        f
    };
    let mut percent = Format::new(Kind::Percentage);
    percent.push(Part::Number {
        decimals: 1,
        min_decimals: 1,
        min_int: 1,
        grouping: false,
    });
    percent.push(Part::Text("%".into()));

    let mut currency = Format::new(Kind::Currency);
    currency.push(Part::Number {
        decimals: 2,
        min_decimals: 2,
        min_int: 1,
        grouping: true,
    });
    currency.push(Part::Text("\u{a0}".into()));
    currency.push(Part::Currency("\u{20ac}".into()));

    // A date spelled the way a European locale spells it — the case that proves the format
    // is carried rather than the value being re-derived from `office:date-value`.
    let mut date = Format::new(Kind::Date);
    date.push(Part::Day { long: true });
    date.push(Part::Text(".".into()));
    date.push(Part::Month { long: true, textual: false });
    date.push(Part::Text(".".into()));
    date.push(Part::Year { long: true });

    let mut clock = Format::new(Kind::Time);
    clock.push(Part::Hours { long: true });
    clock.push(Part::Text(":".into()));
    clock.push(Part::Minutes { long: true });
    clock.push(Part::Text(" ".into()));
    clock.push(Part::AmPm);

    let sheet = doc.sheet_mut(0).unwrap();
    let cells: Vec<(CellValue, Option<NumberKind>, Format)> = vec![
        (CellValue::Number(1234.5), None, number(2, 2, true)),
        // The same family with different attributes: two styles, not one, and a pool that
        // conflates them shows one of the cells the wrong number of digits.
        (CellValue::Number(1234.5), None, number(0, 0, false)),
        (CellValue::Number(0.075), None, percent),
        (CellValue::Number(-19.99), None, currency),
        (
            CellValue::Number(sheet_core::formula::date::serial(2026, 8, 16, epoch)),
            Some(NumberKind::Date),
            date,
        ),
        (CellValue::Number(0.5), Some(NumberKind::Time), clock),
    ];
    for (row, (value, kind, format)) in cells.into_iter().enumerate() {
        let pos = Pos::new(row as u32, 0);
        sheet.set(pos, value);
        if let Some(kind) = kind {
            sheet.set_kind(pos, kind);
        }
        sheet.set_format(pos, format);
    }
    // A formatted cell beside an unformatted one holding the same value: the style must not
    // spread sideways, which is the failure a repeated-cell optimisation introduces.
    sheet.set(Pos::new(0, 1), CellValue::Number(1234.5));

    ("formats".to_owned(), doc)
}

fn cases() -> Vec<(String, Document)> {
    let n = |x: f64| CellValue::Number(x);
    let t = |s: &str| CellValue::Text(s.to_owned());

    let mut named = case(
        "named-expressions",
        &[(0, 0, n(1.0)), (1, 0, n(2.0)), (2, 0, n(3.0))],
    );
    // A named range and a named expression, which ODF spells differently and we store the
    // same way (§5.11, `Document::names`). LO has to give both back recognisably.
    named
        .1
        .names
        .insert("data_range".into(), "[$Data.$A$1:.$A$3]".into());
    named
        .1
        .names
        .insert("total".into(), "of:=SUM([$Data.$A$1:.$A$3])".into());

    let mut all = vec![
        // The degenerate document: one sheet, nothing in it. Legal, and the shape most
        // likely to be written as something LO rejects outright (§3.2).
        ("empty".to_owned(), Document::default()),
        named,
        formats(),
        case(
            "numbers",
            &[
                (0, 0, n(0.0)),
                (0, 1, n(-1.5)),
                (0, 2, n(1e15)),
                (0, 3, n(0.1)),
                // Precision is the point: a double that needs all 17 significant digits
                // to survive is where a writer that formats with `{:.6}` gets caught.
                (0, 4, n(0.123_456_789_012_345_67)),
                (1, 0, n(1.0 / 3.0)),
            ],
        ),
        case(
            "text",
            &[
                (0, 0, t("plain")),
                // XML metacharacters, in an attribute and in text.
                (0, 1, t("<tag> & \"quoted\" 'single'")),
                (0, 2, t("Grüße — ünïcodé ✓")),
                // Whitespace: readers collapse runs inside text:p, so all three of these
                // come back wrong unless text:s is written.
                (0, 3, t("  leading")),
                (0, 4, t("trailing  ")),
                (1, 0, t("inner    spaces")),
                (1, 1, t("tab\there")),
                // Multi-line, including a blank line, which is the case that dies if
                // paragraphs are joined by testing whether the buffer is empty.
                (1, 2, t("line1\nline2")),
                (1, 3, t("\nleading blank")),
                (1, 4, t("a\n\nb")),
                // Text that looks like a number must stay text.
                (2, 0, t("42")),
                (2, 1, t("")),
            ],
        ),
        case(
            "booleans",
            &[
                (0, 0, CellValue::Bool(true)),
                (0, 1, CellValue::Bool(false)),
            ],
        ),
        dates(),
        // Sparse: gaps are written as repeats, and a repeat that is off by one moves every
        // later cell silently.
        case(
            "sparse",
            &[
                (0, 0, n(1.0)),
                (0, 7, n(2.0)),
                (40, 0, n(3.0)),
                (40, 7, n(4.0)),
            ],
        ),
    ];

    let mut many = Document {
        sheets: vec![
            Sheet::new("First"),
            Sheet::new("Second"),
            Sheet::new("Third"),
        ],
        ..Default::default()
    };
    many.sheet_mut(0).unwrap().set(Pos::new(0, 0), n(1.0));
    many.sheet_mut(2).unwrap().set(Pos::new(2, 2), t("third"));
    all.push(("sheets".to_owned(), many));

    all
}

#[test]
fn documents_we_write_survive_libreoffice() {
    if !have_soffice() {
        eprintln!("skipping loop C: no soffice on PATH");
        return;
    }

    let lab = Lab::new("out");
    let cases = cases();
    // Both physical forms, every case. They share a content writer but not a container,
    // and the container is where §1.1's byte-level rules live.
    let staged: Vec<_> = cases
        .iter()
        .flat_map(|(name, doc)| {
            [(Form::Flat, "fods"), (Form::Package, "ods")].map(|(form, ext)| {
                let bytes = sheet_core::write_bytes(doc, form).unwrap();
                // Distinct stems: LO names its output after the input's stem, so
                // `x.ods` and `x.fods` would both convert onto `x.fods`.
                let path = lab.input(&format!("{name}-{ext}.{ext}"), &bytes);
                (name.clone(), doc, path)
            })
        })
        .collect();

    let out = lab.convert(&staged.iter().map(|(_, _, p)| p.clone()).collect::<Vec<_>>());

    let mut failures = Vec::new();
    for (_, doc, path) in &staged {
        let label = path.file_name().unwrap().to_str().unwrap();
        failures.extend(differences(label, doc, &converted(&out, path)));
    }

    for f in &failures {
        eprintln!("  {f}");
    }
    assert!(
        failures.is_empty(),
        "loop C (out): {} differences",
        failures.len()
    );
}

// --- direction "back": LibreOffice -> ours -> LibreOffice -> ours ------------------------

/// How many cells in this document actually hold something.
fn values(doc: &Document) -> usize {
    doc.sheets
        .iter()
        .map(|s| {
            (0..s.used_rows())
                .flat_map(|r| (0..s.used_cols()).map(move |c| Pos::new(r, c)))
                .filter(|p| !s.get(*p).is_empty())
                .count()
        })
        .sum()
}

/// Corpus documents to push back out through the writer.
///
/// Formula-bearing documents are excluded, and this is the phase-3 exit criterion talking
/// (doc/plan.md: "loop C green for value-only documents"), not a convenience: LO
/// recalculates on load, so a formula cell's value after a round trip is a statement about
/// an evaluator that does not exist until phase 4. Phase 4 lifts this filter.
fn sample(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in ["ods", "fods"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("ods" | "fods")
            ) {
                files.push(path);
            }
        }
    }
    let mut eligible: Vec<_> = files
        .iter()
        .filter_map(|path| {
            // Unreadable or encrypted: loop A owns that verdict, not this one.
            let doc = sheet_core::read_file(path).ok()?;
            let biggest = doc
                .sheets
                .iter()
                .map(|s| u64::from(s.used_rows()) * u64::from(s.used_cols()))
                .max()?;
            let formula_free = doc.sheets.iter().all(|s| s.formula_count() == 0);
            (formula_free && biggest < MAX_COMPARED_CELLS).then(|| (values(&doc), path.clone()))
        })
        .collect();

    // Densest first. Taking the alphabetically first twenty instead gives a sample of
    // regression fixtures — single-cell documents that reproduce one import bug each — and
    // three hundred cells to check the whole writer against.
    eligible.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    eligible.truncate(SAMPLE);
    eligible.sort_by(|a, b| a.1.cmp(&b.1));
    eligible.into_iter().map(|(_, path)| path).collect()
}

#[test]
fn libreoffice_documents_survive_our_writer() {
    if !have_soffice() {
        eprintln!("skipping loop C: no soffice on PATH");
        return;
    }
    let root = PathBuf::from(
        std::env::var("SHEET_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CORPUS.to_owned()),
    );
    if !root.is_dir() {
        eprintln!(
            "skipping loop C (back): no LibreOffice corpus at {}",
            root.display()
        );
        return;
    }

    let files = sample(&root);
    assert!(
        !files.is_empty(),
        "no value-only documents found in {}",
        root.display()
    );

    let lab = Lab::new("back");
    let staged: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let doc = sheet_core::read_file(path).unwrap();
            let bytes = sheet_core::write_bytes(&doc, Form::Package).unwrap();
            // Numbered: corpus stems are not unique across `ods/` and `fods/`.
            let staged = lab.input(&format!("{i:03}.ods"), &bytes);
            (path.clone(), doc, staged)
        })
        .collect();

    // A comparison finds nothing in a document that holds nothing, so the sample has to be
    // shown to have substance — otherwise a drifting filter turns this into twenty empty
    // documents agreeing with twenty empty documents, which passes forever.
    let cells: usize = staged.iter().map(|(_, doc, _)| values(doc)).sum();
    eprintln!(
        "loop C (back): {} value-only documents, {cells} cells",
        files.len()
    );
    assert!(
        cells > 5_000,
        "sample holds only {cells} cells; it is not testing the writer"
    );

    let out = lab.convert(&staged.iter().map(|(_, _, p)| p.clone()).collect::<Vec<_>>());

    let mut failures = Vec::new();
    for (original, doc, path) in &staged {
        let label = original.file_name().unwrap().to_str().unwrap();
        failures.extend(differences(label, doc, &converted(&out, path)));
    }

    for f in failures.iter().take(30) {
        eprintln!("  {f}");
    }
    assert!(
        failures.is_empty(),
        "loop C (back): {} differences",
        failures.len()
    );
}
