// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The function samples again, as Excel workbooks: `tests/data/corpus/functions/`, ooxmlgen's
//! `functions` family, built from the same odsgen samples as `sheet/tests/function_samples.rs`
//! and written in Excel's spelling with LibreOffice's answers as the cached values.
//!
//! `ooxmlgen.rs` already holds every cell to its manifest claim — the value carried and the
//! formula translated back to the OpenFormula it was written in. What it cannot say is whether
//! the translated formula *computes* what it should, since importing evaluates nothing. That is
//! this file: import, recalculate every formula, compare with the value the workbook carried.
//! So a function whose Excel spelling is translated to the wrong OpenFormula — a name, an
//! argument, a reference — fails here even where the text of the translation looked right.

use std::path::{Path, PathBuf};

use grind_sheet::formula::eval::{Address, Engine};
use grind_sheet::formula::value::{FormulaError, Value};
use grind_sheet::model::{CellValue, Pos};

/// Where this build answers otherwise than the oracle: `(file, OpenFormula, why)`. The same
/// four as `sheet/tests/function_samples.rs`'s, for the same reason — the cached values are
/// the oracle's, and these are where its precision is short of the formula's.
const DIVERGES: &[(&str, &str, &str)] = &[
    (
        "financial.xlsx",
        "PPMT([Loan.B4]/12;300;[Loan.B5]*12;-([Loan.B2]-[Loan.B3]))",
        "the oracle's precision (sheet/tests/function_samples.rs)",
    ),
    (
        "financial.xlsx",
        "FV([Loan.B7]/12;10*12;-[Loan.B6])",
        "the oracle's precision",
    ),
    (
        "financial.xlsx",
        "PV(0.04/12;20*12;-1500)",
        "the oracle's precision",
    ),
    (
        "financial.xlsx",
        "RATE(48;-450;18000)*12",
        "the oracle's iteration stops early",
    ),
];

fn workbooks() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/corpus/functions");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "xlsx"));
    paths.sort();
    paths
}

/// As `sheet/tests/function_samples.rs` compares: fifteen significant digits, errors by name.
fn agrees(stored: &CellValue, computed: &Value) -> bool {
    match (stored, computed) {
        (CellValue::Number(x), Value::Number(y)) => {
            x == y || (x - y).abs() <= 1e-14 * x.abs().max(y.abs())
        }
        (CellValue::Text(s), Value::Error(e)) => FormulaError::from_name(s) == Some(*e),
        (CellValue::Text(s), Value::Text(t)) => s == t,
        (CellValue::Empty, Value::Text(t)) => t.is_empty(),
        (CellValue::Bool(a), Value::Bool(b)) => a == b,
        (CellValue::Empty, Value::Empty) => true,
        _ => false,
    }
}

#[test]
fn every_sample_workbook_imports_every_formula() {
    let paths = workbooks();
    assert!(
        paths.len() >= 10,
        "{} sample workbooks — the vendoring failed",
        paths.len()
    );
    for path in paths {
        let (_, report) = grind_xlsx::import_file(&path).expect("imports");
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            report.refused.is_empty(),
            "{name}: refused {:?}",
            report.refused
        );
        assert!(
            report.unknown_functions.is_empty(),
            "{name}: unknown {:?}",
            report.unknown_functions
        );
    }
}

#[test]
fn every_imported_sample_recalculates_to_the_value_it_carried() {
    let mut failures = Vec::new();
    let mut reached = std::collections::BTreeSet::new();
    for path in workbooks() {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let (doc, _) = grind_xlsx::import_file(&path).expect("imports");
        let mut engine = Engine::new(&doc);
        for (index, sheet) in doc.sheets.iter().enumerate() {
            let formulas: Vec<(Pos, String)> = sheet
                .formulas()
                .map(|(pos, f)| (pos, f.to_owned()))
                .collect();
            for (pos, formula) in formulas {
                let bare = formula.trim_start_matches('=');
                let stored = sheet.get(pos);
                let computed = engine.value(Address::new(index, pos));
                let named = DIVERGES
                    .iter()
                    .any(|(f, text, _)| *f == file && *text == bare);
                match (agrees(&stored, &computed), named) {
                    (true, false) => {}
                    (false, true) => {
                        reached.insert((file.clone(), bare.to_owned()));
                    }
                    (true, true) => {
                        failures.push(format!("{file}: {bare} agrees now — delete its entry"))
                    }
                    (false, false) => failures.push(format!(
                        "{file} {}!{:?}: {bare} — carried {stored:?}, recalculated {computed:?}",
                        sheet.name, pos
                    )),
                }
            }
        }
    }
    for (file, formula, _) in DIVERGES {
        if !reached.contains(&((*file).to_owned(), (*formula).to_owned())) {
            failures.push(format!(
                "DIVERGES names {file}'s {formula}, which no workbook holds"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
