// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The functions beyond the Small Group (`doc/small-group.md`'s second half), held to the
//! answers the oracle gives.
//!
//! Every expectation here was measured: the same fixture written as a flat ODF document with
//! `table:use-wildcards="true"` — the host settings `wildcard.rs` settles on — converted by
//! LibreOffice 26.8 and its cached values read back. Where the spec says nothing (an empty
//! search text, a fractional `N`, a `Mode` out of range) the oracle is the answer, and
//! `doc/ods-format.md` §4.1 records each such measurement; where the two disagree the spec
//! wins and the test says so at the line.

use super::super::eval::{Address, Engine};
use super::super::value::{FormulaError, Value};
use crate::model::{CellValue, Document, Pos, Sheet};

/// Seven rows, the oracle's fixture exactly:
///
/// | | A | B | C | D | E | F |
/// |---|---|---|---|---|---|---|
/// | 1 | apple | 10 | red | 2026-12-25 | 0 | 1 |
/// | 2 | pear | 20 | green | 2026-12-24 | 0 | 1 |
/// | 3 | apple | 30 | green | | 0 | 1 |
/// | 4 | plum | 40 | red | | 0 | 1 |
/// | 5 | apple | x | red | | 0 | 1 |
/// | 6 | banana | 5 | yellow | | 1 | 1 |
/// | 7 | APPLE | 30 | Red | | 1 | 1 |
///
/// Column D is two holidays, E a work week with Friday and Saturday off, F one with no
/// working day at all.
fn table() -> Document {
    use CellValue::{Number as N, Text as T};
    let rows: [[CellValue; 6]; 7] = [
        [
            T("apple".into()),
            N(10.0),
            T("red".into()),
            N(46381.0),
            N(0.0),
            N(1.0),
        ],
        [
            T("pear".into()),
            N(20.0),
            T("green".into()),
            N(46380.0),
            N(0.0),
            N(1.0),
        ],
        [
            T("apple".into()),
            N(30.0),
            T("green".into()),
            CellValue::Empty,
            N(0.0),
            N(1.0),
        ],
        [
            T("plum".into()),
            N(40.0),
            T("red".into()),
            CellValue::Empty,
            N(0.0),
            N(1.0),
        ],
        [
            T("apple".into()),
            T("x".into()),
            T("red".into()),
            CellValue::Empty,
            N(0.0),
            N(1.0),
        ],
        [
            T("banana".into()),
            N(5.0),
            T("yellow".into()),
            CellValue::Empty,
            N(1.0),
            N(1.0),
        ],
        [
            T("APPLE".into()),
            N(30.0),
            T("Red".into()),
            CellValue::Empty,
            N(1.0),
            N(1.0),
        ],
    ];
    let mut sheet = Sheet::new("Sheet1");
    for (r, row) in rows.into_iter().enumerate() {
        for (c, value) in row.into_iter().enumerate() {
            sheet.set(Pos::new(r as u32, c as u32), value);
        }
    }
    Document {
        sheets: vec![sheet],
        ..Default::default()
    }
}

fn eval(formula: &str) -> Value {
    let document = table();
    // Row 20, as the oracle's document has it: clear of the fixture.
    Engine::new(&document).eval(formula, Address::new(0, Pos::new(20, 0)))
}

fn number(formula: &str) -> f64 {
    match eval(formula) {
        Value::Number(n) => n,
        other => panic!("{formula} evaluated to {other:?}"),
    }
}

fn text(formula: &str) -> String {
    match eval(formula) {
        Value::Text(s) => s,
        other => panic!("{formula} evaluated to {other:?}"),
    }
}

fn error(formula: &str) -> FormulaError {
    match eval(formula) {
        Value::Error(e) => e,
        other => panic!("{formula} evaluated to {other:?}, not an error"),
    }
}

#[test]
fn iferror_and_ifna_catch_what_they_name_and_nothing_else() {
    // §6.15.5, §6.15.6.
    assert_eq!(text("=IFERROR(1/0;\"z\")"), "z");
    assert_eq!(number("=IFERROR(5;\"z\")"), 5.0);
    assert_eq!(number("=IFERROR(NA();7)"), 7.0); // #N/A is an error too
    assert_eq!(number("=IFERROR([.B5]*2;0)"), 0.0); // "x"*2 is #VALUE!
    assert_eq!(text("=IFNA(NA();\"n\")"), "n");
    assert_eq!(error("=IFNA(1/0;\"n\")"), FormulaError::DivZero); // only #N/A is caught
    assert_eq!(number("=IFNA(3;\"n\")"), 3.0);
    // The alternative is not evaluated unless it is taken, as `IF`'s branch is not.
    assert_eq!(number("=IFERROR(1;1/0)"), 1.0);
    assert_eq!(error("=IFERROR(1)"), FormulaError::Value);
}

#[test]
fn concatenate_joins_the_text_of_each_argument() {
    // §6.20.6, `&` as a function: every argument is converted to text (§6.3.14).
    // LibreOffice answers "a11", having no Logical type to convert; §6.3.14 says a true
    // value is "TRUE", and `&` here already says so.
    assert_eq!(text("=CONCATENATE(\"a\";1;TRUE())"), "a1TRUE");
    assert_eq!(text("=CONCATENATE([.A1];\"-\";[.B1])"), "apple-10");
    assert_eq!(text("=CONCATENATE([.C9];\"x\")"), "x"); // an empty cell is ""
    assert_eq!(text("=CONCATENATE(1.5;0.1+0.2)"), "1.50.3");
    assert_eq!(error("=CONCATENATE(1/0;\"x\")"), FormulaError::DivZero);
    // A range is not a Text: §6.3.2 cannot reduce a column to one value off its row.
    assert_eq!(error("=CONCATENATE([.A1:.A2])"), FormulaError::Value);
}

#[test]
fn search_is_find_without_case_and_with_wildcards() {
    // §6.20.20, under HOST-USE-WILDCARDS (see `wildcard.rs`).
    assert_eq!(number("=SEARCH(\"P\";\"apple\")"), 2.0);
    assert_eq!(error("=SEARCH(\"p\";\"apple\";4)"), FormulaError::Value);
    assert_eq!(error("=SEARCH(\"z\";\"apple\")"), FormulaError::Value);
    assert_eq!(number("=SEARCH(\"É\";\"café\")"), 4.0);
    // A pattern matches a stretch of the text, not the whole of it: the anchoring
    // `wildcard::matches` does for a criterion is not SEARCH's.
    assert_eq!(number("=SEARCH(\"a?p\";\"xapple\")"), 2.0);
    assert_eq!(number("=SEARCH(\"p*e\";\"apple\")"), 2.0);
    assert_eq!(number("=SEARCH(\"e*\";\"apple\")"), 5.0);
    assert_eq!(number("=SEARCH(\"*\";\"abc\")"), 1.0);
    assert_eq!(number("=SEARCH(\"?\";\"abc\";2)"), 2.0);
    assert_eq!(number("=SEARCH(\"p?\";\"apple\";3)"), 3.0);
    assert_eq!(number("=SEARCH(\"~*\";\"a*b\")"), 2.0);
    assert_eq!(number("=SEARCH(\"~?\";\"a?b\")"), 2.0);
    // Numbers are searched as their text.
    assert_eq!(number("=SEARCH(2;1234)"), 2.0);
    // Measured: an empty search text is found nowhere, and a start outside the text is an
    // error rather than a clamp.
    assert_eq!(error("=SEARCH(\"\";\"abc\")"), FormulaError::Value);
    assert_eq!(error("=SEARCH(\"a\";\"abc\";0)"), FormulaError::Value);
    assert_eq!(error("=SEARCH(\"a\";\"abc\";4)"), FormulaError::Value);
    assert_eq!(error("=SEARCH(\"a\";\"\")"), FormulaError::Value);
}

#[test]
fn hyperlink_is_worth_its_second_argument() {
    // §6.11.3: a host may make a link of it; the value is FunctionResult, else the IRI.
    assert_eq!(text("=HYPERLINK(\"http://x\")"), "http://x");
    assert_eq!(text("=HYPERLINK(\"http://x\";\"label\")"), "label");
    assert_eq!(number("=HYPERLINK(\"http://x\";42)"), 42.0);
    assert_eq!(number("=HYPERLINK(\"http://x\";[.B1])"), 10.0);
}
