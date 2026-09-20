// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop D — import fidelity: our conversion of a workbook against **LibreOffice's** conversion
//! of the same workbook, cell by cell (`doc/xlsx-import.md`, "Verification").
//!
//! ```text
//! ours   = grind_xlsx::import_bytes(bytes)              → Document
//! theirs = soffice --headless --convert-to fods <file>  → read with our own reader → Document
//! compare(ours, theirs)
//! ```
//!
//! The corpus is the vendored generated one (`tests/data/corpus/`, `ooxmlgen.rs`), every
//! fixture whose manifest row says `oracleOpens` — so the loop needs `soffice` and nothing
//! else, no LibreOffice checkout. `ooxmlgen.rs` holds the same files to the manifest's claims;
//! this holds them to the oracle's *behaviour*, which is the independent half: a manifest is
//! somebody's belief about a file, and the oracle is a program that opened it.
//!
//! **Needs `soffice` on `PATH`; skips with a notice without one**, exactly as loop C does. The
//! oracle is pinned to a container image in CI (`doc/differential-fuzz.md`, "Pinning
//! LibreOffice") and `scripts/soffice-tests.sh` puts the same one first on `PATH` locally. Its
//! output is **cached** under the temp directory by the fixture's SHA-256 — which the
//! manifest already carries — and the oracle's own version string, so a second run costs no
//! conversion and a different LibreOffice never reuses another one's answer.
//!
//! **What is compared at X1: every cell's value and kind.** Formulas are X2's, number formats
//! X3's and styles X4's; each milestone widens [`differences`] rather than adding a loop. The
//! value rule is loop C's — equal at 15 significant digits, since that is all LibreOffice
//! writes — and a kind (date, time) must match exactly.
//!
//! **Where the oracle is wrong**, the disagreement is a named [`Divergence`] — a *construct*,
//! never a file name (`CLAUDE.md`'s rule for every loop) — asserted to still occur, so that a
//! divergence LibreOffice stops having fails this test and has to be deleted.
//!
//!     cargo test -p grind-xlsx --test loop_d -- --nocapture

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use grind_sheet::model::{CellValue, Document, NumberKind, Pos, Sheet};

/// How many fixtures must reach the comparison. The ratchet, as in every loop: raise it,
/// never lower it. 69 of the 76 import; the seven that do not are refused on purpose.
const FLOOR: usize = 60;

/// A construct on which LibreOffice's conversion is known to differ from ours, with the
/// evidence. Each is a predicate over one difference, and each must still match at least one
/// in the corpus — the day it does not, the entry is deleted. **First match wins**, so a
/// narrow entry goes before a broad one.
///
/// Every entry here was *looked at* before it was written: the oracle's `.fods` read for the
/// cell or sheet in question, and the fixture's own bytes beside it. None is a tolerance.
struct Divergence {
    name: &'static str,
    why: &'static str,
    scope: Scope,
}

enum Scope {
    /// One cell both sides have a sheet for.
    Cell(fn(ours: &Cell, theirs: &Cell) -> bool),
    /// A sheet of ours, by name, that the oracle's conversion does not have.
    OnlyOurs(fn(name: &str) -> bool),
    /// A sheet of the oracle's, by name, that ours does not have.
    OnlyTheirs(fn(name: &str) -> bool),
    /// A sheet both have, which the oracle's conversion holds nothing in and ours does.
    Unreached,
}

const DIVERGENCES: &[Divergence] = &[
    Divergence {
        name: "an error value #REF! becomes a formula",
        why: "`<c t=\"e\"><v>#REF!</v></c>` — a cached error with no `<f>` — arrives in the \
              oracle's output as `table:formula=\"of:=#ref!\"`, lower-cased and unparseable, \
              which it then evaluates to `#NAME?`. The other five error values in \
              `values/types.xlsx` survive. Read from the oracle's `.fods` on 2026-09-19.",
        scope: Scope::Cell(|ours, theirs| {
            ours.value == CellValue::Text("#REF!".into())
                && theirs.value == CellValue::Text("#NAME?".into())
        }),
    },
    Divergence {
        name: "a formula cell's value is the oracle's own evaluation",
        why: "The oracle **recalculates** these workbooks on load rather than carrying the \
              cached `<v>`: `formulas/shared-groups.xlsx` caches C2 = 4 and the oracle writes \
              6, which is A2+B2. So for a formula cell this loop would compare two evaluators \
              rather than two importers. The cached values are not unchecked — `ooxmlgen.rs` \
              holds every one of them to the manifest, C2 = 4 included — and X2, which carries \
              the formula as well, is when these cells become comparable here.",
        scope: Scope::Cell(|_, theirs| theirs.formula),
    },
    Divergence {
        name: "the 1900 system below the phantom day",
        why: "Excel's serials 1–60 are one day later than ODF's epoch makes them, because Excel \
              believes 1900-02-29 existed (`doc/xlsx-format.md` §2.1). The oracle applies no \
              correction and so shows every date before 1900-03-01 a day early — 1900-01-01 \
              arrives as 1899-12-31. `values/dates-1900.xlsx`'s manifest records the oracle's \
              answer in an `oracle` field for exactly this reason. Ours shifts by one.",
        scope: Scope::Cell(|ours, theirs| {
            ours.kind == Some(NumberKind::Date)
                && theirs.kind == Some(NumberKind::Date)
                && matches!((&ours.value, &theirs.value), (CellValue::Number(a), CellValue::Number(b))
                    if *a <= 61.0 && (a - b - 1.0).abs() < 1e-9)
        }),
    },
    Divergence {
        name: "an empty string is dropped",
        why: "`<t/>` is a string cell whose text is empty — not an empty cell, which is what the \
              manifest says and what this import carries. The oracle writes a bare \
              `<table:table-cell/>`, so the cell is gone. The hostile entity fixtures come out \
              the same way: an entity nothing defines resolves to no text, leaving an empty \
              string.",
        scope: Scope::Cell(|ours, theirs| {
            ours.value == CellValue::Text(String::new()) && theirs.value == CellValue::Empty
        }),
    },
    Divergence {
        name: "a pivot table's output is regenerated",
        why: "The oracle's conversion of a pivot table carries its own labels rather than the \
              workbook's — `Total Result` where the cell says `grand total` — so the output is \
              regenerated rather than read. This import \
              carries the cells the workbook holds and counts the pivot table as dropped \
              (X5), which is the decision that nothing is evaluated on import.",
        scope: Scope::Cell(|ours, theirs| {
            matches!(ours.value, CellValue::Text(_))
                && theirs.value == CellValue::Text("Total Result".into())
        }),
    },
    Divergence {
        name: "a sheet whose name the oracle does not allow is dropped",
        why: "`document/sheets.xlsx` has ten sheets and the oracle's conversion has eight: \
              `Has[Brackets]` and `Has/Slash` are gone, cells and all. This import keeps both, \
              and renaming them to something ODF can address is X5's (`ooxmlgen.rs` PENDING).",
        scope: Scope::OnlyOurs(|name| name.contains(['[', ']', '/', '\\', '?', '*', ':'])),
    },
    Divergence {
        name: "an external link's cache becomes a sheet",
        why: "The oracle keeps the cached values of an external workbook link in a sheet of \
              its own, named `'file:///…'#Sheet1`. It is not a sheet of the workbook; this \
              import follows no link and counts it as dropped (X5).",
        scope: Scope::OnlyTheirs(|name| name.starts_with("'file:")),
    },
    Divergence {
        name: "a part reached through a backslash is not found",
        why: "`realworld/part-targets.xlsx` reaches its third sheet through a relationship \
              target written with `\\`. The oracle's conversion has the sheet, by name, with \
              nothing in it; this import normalises the separator (`package.rs`) and reads the \
              cell. ECMA-376 Part 2 does not allow `\\` in a part name, and real producers \
              write it anyway.",
        scope: Scope::Unreached,
    },
];

/// One side's view of one cell.
#[derive(Clone, Debug, PartialEq)]
struct Cell {
    value: CellValue,
    kind: Option<NumberKind>,
    /// Whether the cell holds a formula. Ours never does at X1; the oracle's does wherever the
    /// workbook's did.
    formula: bool,
}

fn cell(sheet: &Sheet, pos: Pos) -> Cell {
    Cell {
        value: sheet.get(pos),
        kind: sheet.kind(pos),
        formula: sheet.formula(pos).is_some(),
    }
}

/// Loop C's rule, restated: equal at 15 significant digits (`sheet/tests/roundtrip.rs`'s
/// `same`, which has the arithmetic for why 1e-14 is that), everything else exactly.
fn same(a: &Cell, b: &Cell) -> bool {
    let values = match (&a.value, &b.value) {
        (CellValue::Number(x), CellValue::Number(y)) => {
            x == y || (x - y).abs() <= 1e-14 * x.abs().max(y.abs())
        }
        (x, y) => x == y,
    };
    values && a.kind == b.kind
}

/// One way the two conversions differ.
enum Difference {
    Cell(String, Pos, Cell, Cell),
    OnlyOurs(String),
    OnlyTheirs(String),
    Unreached(String),
}

impl Difference {
    fn explained_by(&self, divergence: &Divergence) -> bool {
        match (self, &divergence.scope) {
            (Difference::Cell(_, _, a, b), Scope::Cell(applies)) => applies(a, b),
            (Difference::OnlyOurs(name), Scope::OnlyOurs(applies)) => applies(name),
            (Difference::OnlyTheirs(name), Scope::OnlyTheirs(applies)) => applies(name),
            (Difference::Unreached(_), Scope::Unreached) => true,
            _ => false,
        }
    }

    fn describe(&self) -> String {
        match self {
            Difference::Cell(sheet, pos, a, b) => format!(
                "{sheet}!{} is {:?} {:?}, the oracle's {:?} {:?}",
                grind_sheet::a1::format(None, *pos),
                a.value,
                a.kind,
                b.value,
                b.kind
            ),
            Difference::OnlyOurs(name) => format!("sheet {name:?} is not in the oracle's"),
            Difference::OnlyTheirs(name) => format!("sheet {name:?} is only in the oracle's"),
            Difference::Unreached(name) => {
                format!("sheet {name:?} holds nothing in the oracle's and cells in ours")
            }
        }
    }
}

/// Every way the two documents disagree. Sheets are matched **by name** — the oracle drops
/// some and invents others, and matching by position would then compare the wrong pairs.
///
/// Cells are walked over the rows either side *carries* rather than over the used rectangle,
/// which for `scale/wide-and-sparse.xlsx` is the whole grid — seventeen billion cells.
fn differences(ours: &Document, theirs: &Document) -> Vec<Difference> {
    let mut out = Vec::new();
    for other in &theirs.sheets {
        if !ours.sheets.iter().any(|s| s.name == other.name) {
            out.push(Difference::OnlyTheirs(other.name.clone()));
        }
    }
    for mine in &ours.sheets {
        let Some(other) = theirs.sheets.iter().find(|s| s.name == mine.name) else {
            out.push(Difference::OnlyOurs(mine.name.clone()));
            continue;
        };
        if other.rows_carrying().is_empty() && !mine.rows_carrying().is_empty() {
            out.push(Difference::Unreached(mine.name.clone()));
            continue;
        }
        let cols = mine.used_cols().max(other.used_cols());
        let mut rows: Vec<u32> = mine
            .rows_carrying()
            .into_iter()
            .chain(other.rows_carrying())
            .flatten()
            .collect();
        rows.sort_unstable();
        rows.dedup();
        for row in rows {
            for col in 0..cols {
                let pos = Pos::new(row, col);
                let (a, b) = (cell(mine, pos), cell(other, pos));
                if !same(&a, &b) {
                    out.push(Difference::Cell(mine.name.clone(), pos, a, b));
                }
            }
        }
    }
    out
}

// ---- the oracle ----

fn soffice_version() -> Option<String> {
    let out = Command::new("soffice").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Where one oracle build's answers are kept: under the **system temp directory**, named by the
/// oracle's version string with everything that is not safe in a path replaced.
///
/// The temp directory rather than `CARGO_TARGET_TMPDIR` because it is the one directory
/// `scripts/soffice-docker/soffice` mounts into the pinned oracle's container — loops C and E
/// stage there for the same reason, and a cache the container cannot see is a loop that
/// passes on a laptop and cannot run in CI.
fn cache_dir(version: &str) -> PathBuf {
    let safe: String = version
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    std::env::temp_dir().join("grind-loop-d").join(safe)
}

/// Convert whatever is not cached yet, in **one** `soffice` invocation — startup dominates, a
/// couple of seconds each time against milliseconds per document once it is up. The private
/// `UserInstallation` is not optional: without it this fights the developer's own running
/// LibreOffice for the profile lock and either blocks or silently does nothing.
fn convert(cache: &Path, pending: &[(String, PathBuf)]) {
    if pending.is_empty() {
        return;
    }
    let stage = cache.join("stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).unwrap();
    // Staged under their digest, so two fixtures called `sheets.xlsx` in different families
    // cannot overwrite each other's output.
    let inputs: Vec<PathBuf> = pending
        .iter()
        .map(|(digest, source)| {
            let ext = source
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("xlsx");
            let staged = stage.join(format!("{digest}.{ext}"));
            std::fs::copy(source, &staged).unwrap();
            staged
        })
        .collect();
    let status = Command::new("soffice")
        .arg("--headless")
        .arg(format!(
            "-env:UserInstallation=file://{}",
            cache.join("profile").display()
        ))
        .args(["--convert-to", "fods", "--outdir"])
        .arg(cache)
        .args(&inputs)
        .status()
        .expect("soffice failed to start");
    assert!(status.success(), "soffice exited with {status}");
    let _ = std::fs::remove_dir_all(&stage);
}

// ---- the corpus ----

struct Fixture {
    file: String,
    sha256: String,
}

fn corpus() -> (PathBuf, Vec<Fixture>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/corpus");
    let text = std::fs::read_to_string(root.join("manifest.json")).expect("the manifest");
    let json: serde_json::Value = serde_json::from_str(&text).expect("the manifest is JSON");
    let fixtures = json["fixtures"]
        .as_array()
        .expect("a fixtures array")
        .iter()
        // A fixture the oracle cannot open has no answer to compare against, and one that
        // must be refused has no document.
        .filter(|f| f["oracleOpens"].as_bool() == Some(true) && f.get("error").is_none())
        .map(|f| Fixture {
            file: f["file"].as_str().unwrap().to_owned(),
            sha256: f["sha256"].as_str().unwrap().to_owned(),
        })
        .collect();
    (root, fixtures)
}

#[test]
fn our_import_agrees_with_the_oracles() {
    let Some(version) = soffice_version() else {
        eprintln!("skipping loop D: no soffice on PATH");
        return;
    };
    let cache = cache_dir(&version);
    std::fs::create_dir_all(&cache).unwrap();

    let (root, fixtures) = corpus();
    let pending: Vec<(String, PathBuf)> = fixtures
        .iter()
        .filter(|f| !cache.join(format!("{}.fods", f.sha256)).exists())
        .map(|f| (f.sha256.clone(), root.join(&f.file)))
        .collect();
    let converted_now = pending.len();
    convert(&cache, &pending);

    let mut compared = 0usize;
    let mut cells = 0usize;
    let mut failures = Vec::new();
    let mut matched: BTreeMap<&str, usize> = BTreeMap::new();
    for fixture in &fixtures {
        let theirs_path = cache.join(format!("{}.fods", fixture.sha256));
        let Ok(theirs) = grind_sheet::read_file(&theirs_path) else {
            failures.push(format!(
                "{}: the oracle produced nothing we can read",
                fixture.file
            ));
            continue;
        };
        let bytes = std::fs::read(root.join(&fixture.file)).unwrap();
        let (ours, report) = match grind_xlsx::import_bytes(&bytes) {
            Ok(pair) => pair,
            Err(e) => {
                failures.push(format!(
                    "{}: the oracle opens it and we do not: {e}",
                    fixture.file
                ));
                continue;
            }
        };
        compared += 1;
        cells += report.cells;

        for difference in differences(&ours, &theirs) {
            match DIVERGENCES.iter().find(|d| difference.explained_by(d)) {
                Some(d) => *matched.entry(d.name).or_default() += 1,
                None => failures.push(format!("{}: {}", fixture.file, difference.describe())),
            }
        }
    }

    // The other direction: a divergence nothing matched any more is one LibreOffice fixed, or
    // a predicate that was never right.
    for d in DIVERGENCES {
        if !matched.contains_key(d.name) {
            failures.push(format!(
                "divergence `{}` matched no cell — delete it if the oracle changed ({})",
                d.name, d.why
            ));
        }
    }

    eprintln!(
        "loop D: {version}; {compared} workbooks, {cells} cells, {} disagreements, \
         {} named divergences ({converted_now} converted now, the rest cached)",
        failures.len(),
        matched.values().sum::<usize>(),
    );
    for (name, count) in &matched {
        eprintln!("  divergence ×{count}: {name}");
    }
    for failure in failures.iter().take(100) {
        eprintln!("  {failure}");
    }
    if failures.len() > 100 {
        eprintln!("  ... and {} more", failures.len() - 100);
    }
    assert!(
        compared >= FLOOR,
        "{compared} workbooks reached the comparison, fewer than the ratchet's {FLOOR}"
    );
    assert!(
        failures.is_empty(),
        "{} disagreements with the oracle",
        failures.len()
    );
}
