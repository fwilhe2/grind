// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The knowledge-base corpus — eight hand-written documents that are a *requirement*.
//!
//! Loop A's corpus is LibreOffice's, lives outside the repo and skips when it is not
//! there. These eight are named in `doc/plan.md`'s requirements as documents this build
//! must handle, so they are vendored (`core/tests/data/kb/`, MIT, declared in
//! `REUSE.toml`) and this test never skips. Upstream is
//! <https://github.com/fwilhe2/open-document-knowledge-base>.
//!
//! They are worth having *because* they are not LibreOffice's output. They are written by
//! hand against the spec, so they exercise the tolerant reader from the other side: an
//! `office:version` of 1.3, `calcext:` attributes the ODF schema does not allow, a table
//! with no `table:table-column` at all, formulas with no cached value, `<table:table-cell/>`
//! self-closed with nothing in it.
//!
//! Two things are checked, and they are the two normative requirements:
//!
//! * every one of them **reads**, and survives write → read with its values, formulas and
//!   names intact;
//! * everything this build **writes** validates against the ODF 1.4 RELAX NG schema.
//!
//! The second needs `jing` on `PATH` and says so when it is missing. That is the one part
//! that skips, because a validator is not vendorable.

use std::path::{Path, PathBuf};
use std::process::Command;

use sheet_core::{Document, Form, Pos};

/// Every document named in `doc/plan.md`'s requirements. Listed rather than globbed: the
/// requirement is these eight, so a file going missing must fail rather than shrink the run.
const REQUIRED: [&str; 8] = [
    "filter.fods",
    "fizzbuzz.fods",
    "formula.fods",
    "minimal.fods",
    "minimal-libreoffice.fods",
    "minimal-libreoffice-cleanup.fods",
    "minimal-with-styles.fods",
    "named-range.fods",
];

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/kb")
        .join(name)
}

/// Everything about a document that a round trip must not change, as one comparable value.
///
/// Deliberately coarser than loop C's cell-by-cell `differences`: formats and styles are
/// loop C's job against LibreOffice, and what these eight are here to pin is that a
/// hand-written document does not lose its *contents* on the way through.
fn digest(doc: &Document) -> Vec<String> {
    let mut out = vec![format!("names {:?}", doc.names)];
    for sheet in &doc.sheets {
        out.push(format!("sheet {:?}", sheet.name));
        for row in 0..sheet.used_rows() {
            for col in 0..sheet.used_cols() {
                let pos = Pos::new(row, col);
                let (value, formula) = (sheet.get(pos), sheet.formula(pos));
                if !matches!(value, sheet_core::CellValue::Empty) || formula.is_some() {
                    out.push(format!(
                        "r{row}c{col} {value:?} {formula:?} {:?}",
                        sheet.kind(pos)
                    ));
                }
            }
        }
    }
    out
}

#[test]
fn every_required_document_reads_and_round_trips() {
    for name in REQUIRED {
        let path = data(name);
        let doc = sheet_core::read_file(&path).unwrap_or_else(|e| panic!("{name}: {e}"));

        // Both forms, because the requirement is about ODF and not about flat XML. A
        // package is the same content model in a zip (§7.3).
        for form in [Form::Flat, Form::Package] {
            let bytes = sheet_core::write_bytes(&doc, form)
                .unwrap_or_else(|e| panic!("{name} as {form:?}: {e}"));
            let back = sheet_core::read_bytes(name, &bytes)
                .unwrap_or_else(|e| panic!("{name} as {form:?}, reading back: {e}"));
            assert_eq!(
                digest(&doc),
                digest(&back),
                "{name} as {form:?}: contents changed on the way through"
            );
        }
    }
}

/// `fizzbuzz.fods` is eighteen formulas and not one cached value, which is legal and which
/// LibreOffice renders blank until it recalculates. Pinned separately because it is the one
/// file whose *point* is that reading it correctly produces an empty grid.
///
/// It also recalculates to eighteen `#NAME?`, and that is **correct today**: the formula is
/// `IF(MOD(ROW();15)=0;…)`, and `ROW` (§6.13.29) is not in the Small Group — §2.3.2 admits
/// `ROWS` and `COLUMNS`, not `ROW` and `COLUMN`. So this asserts the scope line rather than
/// a bug, and it is the test to change on the day `ROW` moves in by explicit decision.
#[test]
fn a_document_of_formulas_with_no_cached_values_recalculates() {
    let app = sheet_core::App::new();
    app.open_file(&data("fizzbuzz.fods")).unwrap();
    assert_eq!(app.formula_count(0).unwrap(), 18);
    assert_eq!(app.used_extent(0).unwrap(), (0, 0), "no cached values to read");

    app.recalc().unwrap();
    assert_eq!(app.used_extent(0).unwrap(), (18, 1));
    let shown: Vec<_> = (0..18)
        .map(|row| format!("{:?}", app.get(0, Pos::new(row, 0)).unwrap()))
        .collect();
    assert!(
        shown.iter().all(|v| v.contains("#NAME?")),
        "expected #NAME? throughout while `ROW` is out of scope, got {shown:?}"
    );
}

// --- schema validity ---------------------------------------------------------------------

/// `jing -i` rather than `xmllint --relaxng`: the ODF schema uses RELAX NG constructs
/// xmllint does not implement, and `-i` turns off the ID/IDREF checking that makes jing
/// reject the schema itself over `draw:control` (an ODF quirk, not ours).
fn jing(path: &Path) -> Option<Result<(), String>> {
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("../doc/OpenDocument-v1.4-schema.rng");
    let out = Command::new("jing").arg("-i").arg(schema).arg(path).output().ok()?;
    Some(if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

#[test]
fn everything_we_write_is_valid_odf() {
    let dir = std::env::temp_dir().join(format!("sheet-kb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut failures = Vec::new();
    for name in REQUIRED {
        let doc = sheet_core::read_file(&data(name)).unwrap();
        let path = dir.join(name);
        sheet_core::write_file(&doc, &path).unwrap();

        match jing(&path) {
            None => {
                eprintln!("skipping: no `jing` on PATH; schema validity unchecked");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            Some(Ok(())) => {}
            Some(Err(report)) => failures.push(format!("{name}:\n{report}")),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        failures.is_empty(),
        "documents we wrote are not valid ODF 1.4:\n{}",
        failures.join("\n")
    );
}
