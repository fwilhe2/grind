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
    // Measured: X may not be left empty, and an empty Alternative is a zero, as an empty
    // branch of `IF` is.
    assert_eq!(error("=IFERROR(;\"A\")"), FormulaError::Value);
    assert_eq!(error("=IFNA(;\"A\")"), FormulaError::Value);
    assert_eq!(number("=IFNA(NA();)"), 0.0);
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

#[test]
fn roundup_and_rounddown_step_away_from_and_towards_zero() {
    // §6.17.7, §6.17.6.
    assert_eq!(number("=ROUNDUP(2.1)"), 3.0);
    assert_eq!(number("=ROUNDUP(-2.1)"), -3.0);
    assert_eq!(number("=ROUNDUP(1.23456;2)"), 1.24);
    assert_eq!(number("=ROUNDUP(1234;-2)"), 1300.0);
    assert_eq!(number("=ROUNDUP(2)"), 2.0);
    assert_eq!(number("=ROUNDUP(2.5;1.9)"), 2.5); // Digits truncates, as ROUND's does
    assert_eq!(number("=ROUNDUP(-0.0001;3)"), -0.001);
    assert_eq!(number("=ROUNDDOWN(2.9)"), 2.0);
    assert_eq!(number("=ROUNDDOWN(-2.9)"), -2.0);
    assert_eq!(number("=ROUNDDOWN(1.23456;3)"), 1.234);
    assert_eq!(number("=ROUNDDOWN(1299;-2)"), 1200.0);
    // A 4 in the twelfth place rounds up to the eleventh, as §6.17.7 says and Excel agrees;
    // LibreOffice answers 8.23456789013.
    assert_eq!(number("=ROUNDUP(8.234567890134;11)"), 8.23456789014);
}

#[test]
fn rounding_reads_a_value_to_fifteen_digits_before_it_steps() {
    // Measured (`doc/ods-format.md` §4.1): 4.35*100 is 434.99999999999994 in binary, and the
    // oracle rounds it down to 435 — the scaled value is read to the fifteen significant
    // digits a spreadsheet shows before it is stepped, or a value that *displays* as 435
    // would round down to 434.
    assert_eq!(number("=ROUNDDOWN(4.35*100;0)"), 435.0);
    assert_eq!(number("=ROUNDDOWN(0.29*100)"), 29.0);
    assert_eq!(number("=ROUNDUP(0.1+0.2;1)"), 0.3);
    assert_eq!(number("=ROUNDUP(1.005;2)"), 1.01);
    assert_eq!(number("=QUOTIENT(0.3;0.1)"), 3.0);
    // Fifteen digits, not fewer: a difference in the fifteenth still counts.
    assert_eq!(number("=ROUNDUP(1.00000000000001)"), 2.0);
    assert_eq!(number("=ROUNDDOWN(0.999999999999999)"), 0.0);
    // ... and only below 10^11. A 16-digit integer is not "shown" as 15 digits and rounded
    // to a neighbour, and a large fraction is stepped as it is (`doc/ods-format.md` §4.1).
    assert_eq!(number("=ROUNDDOWN(2^52+1)"), 4_503_599_627_370_497.0);
    assert_eq!(number("=ROUND(2^52+1;0)"), 4_503_599_627_370_497.0);
    assert_eq!(
        number("=ROUNDDOWN(123456789012345+0.67)"),
        123_456_789_012_345.0
    );
    assert_eq!(
        number("=ROUNDDOWN(99999999999999+0.96)"),
        99_999_999_999_999.0
    );
    assert_eq!(number("=ROUNDDOWN(10^11*(1-4E-16))"), 100_000_000_000.0);
    assert_eq!(number("=ROUNDDOWN(10^14*(1-4E-16))"), 99_999_999_999_999.0);
}

#[test]
fn mround_takes_the_nearest_multiple_and_the_greater_on_a_tie() {
    // §6.17.4.
    assert_eq!(number("=MROUND(10;3)"), 9.0);
    assert_eq!(number("=MROUND(-10;-3)"), -9.0);
    assert_eq!(number("=MROUND(7.5;5)"), 10.0);
    assert_eq!(number("=MROUND(2.5;5)"), 5.0);
    assert_eq!(number("=MROUND(0;3)"), 0.0);
    assert!((number("=MROUND(1.3;0.2)") - 1.4).abs() < 1e-15); // 6.5 steps, read as such
    // The spec puts no sign constraint on A and B, and nor does the oracle.
    assert_eq!(number("=MROUND(-10;3)"), -9.0);
    assert_eq!(number("=MROUND(10;-3)"), 9.0);
    // Measured: a multiple of zero is zero.
    assert_eq!(number("=MROUND(5;0)"), 0.0);
    // "In case that two such X exist, the greater one is the result": -5, not the oracle's
    // -10, which rounds a tie away from zero as Excel does.
    assert_eq!(number("=MROUND(-7.5;-5)"), -5.0);
}

#[test]
fn ceiling_and_floor_step_towards_an_infinity_unless_a_mode_says_away_from_zero() {
    // §6.17.1, §6.17.3: with no Mode, the mathematical ceiling and floor of N/Significance.
    assert_eq!(number("=CEILING(2.5)"), 3.0);
    assert_eq!(number("=CEILING(-2.5)"), -2.0);
    assert_eq!(number("=CEILING(2.5;2)"), 4.0);
    assert_eq!(number("=CEILING(-2.5;-2)"), -2.0);
    assert_eq!(number("=CEILING(-4.1;-2;0)"), -4.0);
    assert_eq!(number("=CEILING(4;2)"), 4.0);
    assert_eq!(number("=FLOOR(2.5)"), 2.0);
    assert_eq!(number("=FLOOR(-2.5)"), -3.0);
    assert_eq!(number("=FLOOR(2.5;2)"), 2.0);
    assert_eq!(number("=FLOOR(-2.5;-2)"), -4.0);
    // A Mode other than zero rounds the magnitude — CEILING away from zero, FLOOR towards it
    // — and an empty Significance is still ±1.
    assert_eq!(number("=CEILING(-2.5;-2;1)"), -4.0);
    assert_eq!(number("=CEILING(-2.5;-2;-1)"), -4.0);
    assert_eq!(number("=CEILING(2.5;2;-1)"), 4.0);
    assert_eq!(number("=CEILING(-2.5;;1)"), -3.0);
    assert_eq!(number("=CEILING(2.5;;1)"), 3.0);
    assert_eq!(number("=CEILING(2.5;;)"), 3.0);
    assert_eq!(number("=FLOOR(-2.5;-2;1)"), -2.0);
    assert_eq!(number("=FLOOR(-2.5;;1)"), -2.0);
    // "If any of the two parameters N or Significance is zero, the result is zero" — and an
    // empty N is a zero, measured.
    assert_eq!(number("=CEILING(0;2)"), 0.0);
    assert_eq!(number("=CEILING(2.5;0)"), 0.0);
    assert_eq!(number("=CEILING(;2)"), 0.0);
    assert_eq!(number("=CEILING([.D9];2)"), 0.0);
    assert_eq!(number("=FLOOR(0;2)"), 0.0);
    assert_eq!(number("=FLOOR(2.5;0)"), 0.0);
    // The quotient is read as it is shown before it is stepped, as every rounding here is;
    // the multiple itself is a product, as near as a double holds it (the oracle writes
    // fifteen digits, so it cannot say which neighbour it holds).
    close("=CEILING(1.25;0.1)", 1.3);
    close("=CEILING(0.1+0.2;0.1)", 0.3);
    close("=FLOOR(1.35;0.1)", 1.3);
    close("=FLOOR(0.3;0.1)", 0.3);
    assert_eq!(number("=FLOOR(4.35*100;1)"), 435.0);
    close("=FLOOR(-0.3;-0.1;1)", -0.3);
    // The multiple is the one shown, too: three times 2.3 is 6.9 and not 6.8999999999999995,
    // which LibreOffice's own test document compares to a typed 6.9.
    assert_eq!(number("=FLOOR(7.9;2.3)"), 6.9);
    assert_eq!(number("=FLOOR(-7.9;-2.3;1)"), -6.9);
    assert_eq!(number("=MROUND(7.9;2.3)"), 6.9);
    assert_eq!(number("=CEILING(\"3.5\";1)"), 4.0);
    // "Both N and Significance shall … have the same sign": the oracle refuses the other
    // signs whatever the Mode, where Excel's two-parameter CEILING would round -2.5 to -2.
    assert_eq!(error("=CEILING(-2.5;2)"), FormulaError::Num);
    assert_eq!(error("=CEILING(2.5;-2)"), FormulaError::Num);
    assert_eq!(error("=CEILING(-2.5;2;1)"), FormulaError::Num);
    assert_eq!(error("=CEILING(-2.5;2;0)"), FormulaError::Num);
    assert_eq!(error("=FLOOR(-2.5;2)"), FormulaError::Num);
    assert_eq!(error("=FLOOR(2.5;-2)"), FormulaError::Num);
    assert_eq!(error("=FLOOR(-2.5;2;1)"), FormulaError::Num);
    assert_eq!(error("=CEILING(2.5;2;\"x\")"), FormulaError::Value);
    assert_eq!(error("=CEILING(2.5;[.C1])"), FormulaError::Value);
}

#[test]
fn excels_ceiling_and_floor_keep_their_own_names_and_their_own_rule() {
    // What an Excel workbook's CEILING and FLOOR are imported as (`doc/xlsx-format.md`
    // §5.1), measured against the oracle's functions of the same names: the quotient stepped
    // up or down on the number line, so a negative N with a positive Significance is allowed
    // where §6.17 refuses it, and two negatives round the magnitude.
    assert_eq!(number("=COM.MICROSOFT.CEILING(2.5;2)"), 4.0);
    assert_eq!(number("=COM.MICROSOFT.CEILING(-2.5;2)"), -2.0);
    assert_eq!(number("=COM.MICROSOFT.CEILING(-2.5;-2)"), -4.0);
    assert_eq!(number("=COM.MICROSOFT.CEILING(-4.5;1)"), -4.0);
    assert_eq!(number("=COM.MICROSOFT.CEILING(-0.5;-1)"), -1.0);
    assert_eq!(number("=COM.MICROSOFT.CEILING(7.9;2.3)"), 9.2);
    assert_eq!(number("=COM.MICROSOFT.CEILING(0.1+0.2;0.1)"), 0.3);
    assert_eq!(number("=COM.MICROSOFT.CEILING(\"3.5\";1)"), 4.0);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(2.5;2)"), 2.0);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(-2.5;2)"), -4.0);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(-2.5;-2)"), -2.0);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(-4.5;1)"), -5.0);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(-0.5;-1)"), 0.0);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(7.9;2.3)"), 6.9);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(-7.9;-2.3)"), -6.9);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(0.3;0.1)"), 0.3);
    // A zero: CEILING's answer is zero either way, FLOOR's only for a zero N — dividing by
    // a zero Significance is Excel's #DIV/0!, where the oracle answers its generic Err:502.
    assert_eq!(number("=COM.MICROSOFT.CEILING(2.5;0)"), 0.0);
    assert_eq!(number("=COM.MICROSOFT.CEILING(0;2)"), 0.0);
    assert_eq!(number("=COM.MICROSOFT.CEILING(2.5;)"), 0.0);
    assert_eq!(number("=COM.MICROSOFT.CEILING(;2)"), 0.0);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(0;0)"), 0.0);
    assert_eq!(number("=COM.MICROSOFT.FLOOR(0;2)"), 0.0);
    assert_eq!(error("=COM.MICROSOFT.FLOOR(2.5;0)"), FormulaError::DivZero);
    // A positive N with a negative Significance is refused, and both parameters are needed.
    assert_eq!(error("=COM.MICROSOFT.CEILING(2.5;-2)"), FormulaError::Num);
    assert_eq!(error("=COM.MICROSOFT.CEILING(4.5;-1)"), FormulaError::Num);
    assert_eq!(error("=COM.MICROSOFT.FLOOR(2.5;-2)"), FormulaError::Num);
    assert_eq!(error("=COM.MICROSOFT.CEILING(2.5)"), FormulaError::Value);
    assert_eq!(error("=COM.MICROSOFT.FLOOR(2.5)"), FormulaError::Value);
    assert_eq!(
        error("=COM.MICROSOFT.CEILING(2.5;2;1)"),
        FormulaError::Value
    );
    // Lower case is the same name, as for every function (§5.6).
    assert_eq!(number("=com.microsoft.ceiling(-2.5;2)"), -2.0);
}

/// [`table`] with rows hidden the two ways §6.16.60 tells apart, and column G holding what
/// a subtotal must step over: G1 `=SUBTOTAL(9;[.B1:.B7])`, G2 5, G3 `=SUM([.B1:.B2])`, G4 7.
///
/// A filter on column C keeps red and yellow, so rows 2 and 3 — the green ones — are hidden
/// by it; row 4 is hidden by hand, which ODF writes as `table:visibility="collapse"`. The
/// oracle's document says the same with those two attributes on the rows.
fn hidden() -> Document {
    let mut document = table();
    let sheet = &mut document.sheets[0];
    sheet.set_filter(Some(crate::filter::Filter {
        name: crate::filter::ANONYMOUS.into(),
        start: Pos::new(0, 0),
        end: Pos::new(6, 5),
        contains_header: false,
        buttons: true,
        keep: [(2, ["red", "Red", "yellow"].map(String::from).into())].into(),
    }));
    sheet.set_row_hidden(3, true);
    sheet.set_formula(Pos::new(0, 6), "=SUBTOTAL(9;[.B1:.B7])".into());
    sheet.set(Pos::new(1, 6), CellValue::Number(5.0));
    sheet.set_formula(Pos::new(2, 6), "=SUM([.B1:.B2])".into());
    sheet.set(Pos::new(3, 6), CellValue::Number(7.0));
    document
}

fn subtotal(formula: &str) -> Value {
    let document = hidden();
    Engine::new(&document).eval(formula, Address::new(0, Pos::new(20, 0)))
}

#[track_caller]
fn subtotal_is(formula: &str, want: f64) {
    match subtotal(formula) {
        Value::Number(n) => assert!(
            (n - want).abs() <= 1e-12 * want.abs().max(1.0),
            "{formula}: {n}, not {want}"
        ),
        other => panic!("{formula} evaluated to {other:?}"),
    }
}

#[test]
fn subtotal_steps_over_filtered_rows_and_other_subtotals() {
    // §6.16.60, over B1:B7 = 10, 20, 30, 40, "x", 5, 30 with 20 and 30 filtered out and 40
    // hidden by hand: 1–11 skip what the filter hides, 101–111 what is collapsed as well.
    subtotal_is("=SUBTOTAL(9;[.B1:.B7])", 85.0);
    subtotal_is("=SUBTOTAL(109;[.B1:.B7])", 45.0);
    subtotal_is("=SUBTOTAL(1;[.B1:.B7])", 21.25);
    subtotal_is("=SUBTOTAL(101;[.B1:.B7])", 15.0);
    subtotal_is("=SUBTOTAL(2;[.B1:.B7])", 4.0);
    subtotal_is("=SUBTOTAL(102;[.B1:.B7])", 3.0);
    subtotal_is("=SUBTOTAL(3;[.A1:.A7])", 5.0);
    subtotal_is("=SUBTOTAL(103;[.A1:.A7])", 4.0);
    subtotal_is("=SUBTOTAL(3;[.B1:.B7])", 5.0); // the text "x" counts, as COUNTA's does
    subtotal_is("=SUBTOTAL(4;[.B1:.B7])", 40.0);
    subtotal_is("=SUBTOTAL(104;[.B1:.B7])", 30.0);
    subtotal_is("=SUBTOTAL(5;[.B1:.B7])", 5.0);
    subtotal_is("=SUBTOTAL(6;[.B1:.B7])", 60000.0);
    subtotal_is("=SUBTOTAL(106;[.B1:.B7])", 1500.0);
    subtotal_is("=SUBTOTAL(7;[.B1:.B7])", 16.5201896679992);
    subtotal_is("=SUBTOTAL(8;[.B1:.B7])", 14.3069039278245);
    subtotal_is("=SUBTOTAL(10;[.B1:.B7])", 272.916666666667);
    subtotal_is("=SUBTOTAL(11;[.B1:.B7])", 204.6875);
    subtotal_is("=SUBTOTAL(111;[.B1:.B7])", 116.666666666667);
    // A cell whose formula calls SUBTOTAL is never counted, so a grand total over a column
    // of subtotals does not count them twice; G3 is a SUM in a filtered row, and G4 is
    // hidden by hand.
    subtotal_is("=SUBTOTAL(9;[.G1:.G4])", 7.0);
    subtotal_is("=SUBTOTAL(109;[.G1:.G4])", 0.0);
    subtotal_is("=SUM([.G1:.G4])", 127.0); // SUM itself skips nothing
    // Several references, as Excel allows, and a whole rectangle — a hidden *row* is hidden
    // in every column of it.
    subtotal_is("=SUBTOTAL(9;[.B1:.B7];[.G1:.G4])", 92.0);
    subtotal_is("=SUBTOTAL(9;[.A1:.F7])", 46473.0);
    subtotal_is("=SUBTOTAL(9;[.B:.B])", 85.0);
    subtotal_is("=SUBTOTAL(9;[.B1])", 10.0);
    subtotal_is("=SUBTOTAL(9;[.B2])", 0.0);
    // The function number is truncated and converted; anything outside the two bands is an
    // error (the oracle's Err:502), and so is a sequence that is not a reference.
    subtotal_is("=SUBTOTAL(9.7;[.B1:.B7])", 85.0);
    subtotal_is("=SUBTOTAL(109.2;[.B1:.B7])", 45.0);
    subtotal_is("=SUBTOTAL(\"9\";[.B1:.B7])", 85.0);
    for bad in [
        "=SUBTOTAL(0;[.B1:.B7])",
        "=SUBTOTAL(12;[.B1:.B7])",
        "=SUBTOTAL(100;[.B1:.B7])",
        "=SUBTOTAL(112;[.B1:.B7])",
        "=SUBTOTAL(;[.B1:.B7])",
        "=SUBTOTAL(9)",
        "=SUBTOTAL(9;5;\"7\")",
        "=SUBTOTAL(2;[.B1:.B7]*1)",
    ] {
        assert_eq!(subtotal(bad), Value::Error(FormulaError::Value), "{bad}");
    }
    // An average or a deviation of nothing divides by zero, as AVERAGE and STDEV do.
    assert_eq!(
        subtotal("=SUBTOTAL(1;[.C1:.C7])"),
        Value::Error(FormulaError::DivZero)
    );
    assert_eq!(
        subtotal("=SUBTOTAL(7;[.B4:.B5])"),
        Value::Error(FormulaError::DivZero)
    );
}

#[test]
fn sumproduct_multiplies_arrays_element_by_element_and_adds_the_products() {
    // §6.16.64 over the fixture's columns: B = 10, 20, 30, 40, "x", 5, 30; E = 0 × 5, 1, 1;
    // F = 1 × 7. An element that is not a number is a zero — the "x" and every text — and
    // a logical is 1 or 0.
    assert_eq!(number("=SUMPRODUCT([.B1:.B4];[.E1:.E4])"), 0.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B4];[.F1:.F4])"), 100.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B7];[.F1:.F7])"), 135.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B7])"), 135.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.C4];[.D1:.E4])"), 1_391_410.0);
    assert_eq!(number("=SUMPRODUCT([.D1:.D4];[.B1:.B4])"), 1_391_410.0);
    assert_eq!(number("=SUMPRODUCT([.B5:.B6];[.F5:.F6])"), 5.0);
    assert_eq!(number("=SUMPRODUCT([.A1:.A2];[.B1:.B2])"), 0.0);
    assert_eq!(number("=SUMPRODUCT(2;3)"), 6.0);
    assert_eq!(number("=SUMPRODUCT(\"2\";3)"), 0.0);
    // ForceArray (§6.3.4): an operator inside is applied element by element (§3.3 2.2),
    // a single value is repeated, and a comparison is an array of logicals.
    assert_eq!(number("=SUMPRODUCT([.B1:.B4]*2)"), 200.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B4]+[.B1])"), 140.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B4]^2)"), 3000.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B4]%)"), 1.0);
    assert_eq!(number("=SUMPRODUCT(-[.B1:.B4])"), -100.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B4]&\"\")"), 0.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B4]*TRUE())"), 100.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B4]*[.F1:.F4];[.B1:.B4])"), 3000.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B4]>15)"), 3.0);
    assert_eq!(number("=SUMPRODUCT(--([.B1:.B4]>15))"), 3.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B4]=[.B1:.B4])"), 4.0);
    assert_eq!(number("=SUMPRODUCT([.B1:.B7]=\"x\")"), 1.0);
    assert_eq!(number("=SUMPRODUCT(([.B1:.B4]>15)*([.B1:.B4]))"), 90.0);
    assert_eq!(number("=SUMPRODUCT(([.B1:.B4]>15)+0;[.B1:.B4])"), 90.0);
    // The idiom SUMIFS replaced, under this build's case-insensitive comparison (the oracle
    // agrees once `table:case-sensitive` is false; ODF's default is true).
    assert_eq!(
        number("=SUMPRODUCT(([.A1:.A7]=\"apple\")*1;[.B1:.B7])"),
        70.0
    );
    assert_eq!(number("=SUMPRODUCT(([.A1:.A7]=\"apple\");[.B1:.B7])"), 70.0);
    assert_eq!(
        number("=SUMPRODUCT(--([.A1:.A7]=\"apple\");[.B1:.B7])"),
        70.0
    );
    assert_eq!(
        number("=SUMPRODUCT(([.A1:.A7]=\"apple\")*([.C1:.C7]=\"red\"))"),
        3.0
    );
    // A function given an array where it expects one value is evaluated once per element
    // (§3.3 2.2); one whose parameter is a sequence takes the range whole.
    assert_eq!(number("=SUMPRODUCT(LEN([.A1:.A4]))"), 18.0);
    assert_eq!(number("=SUMPRODUCT(ABS(-[.B1:.B4]))"), 100.0);
    assert_eq!(number("=SUMPRODUCT(IF([.B1:.B4]>15;1;0))"), 3.0);
    assert_eq!(number("=SUMPRODUCT(IF([.B1:.B4]>15;[.B1:.B4];0))"), 90.0);
    assert_eq!(number("=SUMPRODUCT(ROUND([.B1:.B4]/3;0))"), 33.0);
    assert_eq!(number("=SUMPRODUCT(MID([.A1:.A2];1;1)=\"a\")"), 1.0);
    assert_eq!(number("=SUMPRODUCT(ISNUMBER([.B1:.B7])*1)"), 6.0);
    assert_eq!(number("=SUMPRODUCT(SUM([.B1:.B4])*[.F1:.F4])"), 400.0);
    assert_eq!(number("=SUMPRODUCT(COUNTIF([.A1:.A7];[.A1:.A2]))"), 5.0);
    // An error in any element is the answer, as it is for an operator.
    assert_eq!(
        error("=SUMPRODUCT([.B1:.B7]*[.F1:.F7])"),
        FormulaError::Value
    );
    assert_eq!(
        error("=SUMPRODUCT(([.A1:.A7]=\"apple\")*[.B1:.B7])"),
        FormulaError::Value
    );
    assert_eq!(
        error("=SUMPRODUCT([.B1:.B4]/[.E1:.E4])"),
        FormulaError::DivZero
    );
    assert_eq!(
        error("=SUMPRODUCT([.B1:.B4];1/[.E1:.E4])"),
        FormulaError::DivZero
    );
    assert_eq!(error("=SUMPRODUCT(1/0)"), FormulaError::DivZero);
    // "All matrices shall have the same dimensions."
    assert_eq!(
        error("=SUMPRODUCT([.B1:.B4];[.F1:.F3])"),
        FormulaError::Value
    );
    assert_eq!(
        error("=SUMPRODUCT([.B1:.B4];[.D1:.E2])"),
        FormulaError::Value
    );
    assert_eq!(
        error("=SUMPRODUCT([.B1:.B2];[.B1:.C1])"),
        FormulaError::Value
    );
    assert_eq!(error("=SUMPRODUCT([.B1:.B4];\"a\")"), FormulaError::Value);
    // An argument that is one error is the answer before any shape is compared — a
    // function this build lacks says #NAME?, not that its one value is the wrong size.
    assert_eq!(error("=SUMPRODUCT([.B1:.B4];1/0)"), FormulaError::DivZero);
    assert_eq!(
        error("=SUMPRODUCT([.B1:.B4];NOSUCH(1))"),
        FormulaError::Name
    );
    assert_eq!(error("=SUMPRODUCT([.B1:.B4];NA())"), FormulaError::NA);
    assert_eq!(
        error("=SUMPRODUCT([.B1:.B4];[.F1:.F3];1/0)"),
        FormulaError::DivZero
    );
    assert_eq!(error("=SUMPRODUCT([.B1:.B4];)"), FormulaError::Value);
    assert_eq!(error("=SUMPRODUCT()"), FormulaError::Value);
    // An operator over arrays of different lengths fills the shorter one's missing places
    // with #N/A (§3.3 2.2.2, Note 8), and the sum carries it: the spec, where the oracle
    // quietly sums the overlap and answers 60.
    assert_eq!(error("=SUMPRODUCT([.B1:.B4]*[.F1:.F3])"), FormulaError::NA);
}

#[test]
fn quotient_truncates_and_sign_is_three_valued() {
    // §6.16.48, §6.16.54.
    assert_eq!(number("=QUOTIENT(7;2)"), 3.0);
    assert_eq!(number("=QUOTIENT(-7;2)"), -3.0); // towards zero, unlike INT
    assert_eq!(number("=QUOTIENT(7;-2)"), -3.0);
    assert_eq!(number("=QUOTIENT(7.9;2.5)"), 3.0);
    assert_eq!(error("=QUOTIENT(1;0)"), FormulaError::DivZero);
    // Neither parameter is optional, and an empty one is not a zero (measured).
    assert_eq!(error("=QUOTIENT(7;)"), FormulaError::Value);
    assert_eq!(error("=MROUND(15.5;)"), FormulaError::Value);
    assert_eq!(error("=MROUND(;3)"), FormulaError::Value);
    assert_eq!(number("=SIGN(-3)"), -1.0);
    assert_eq!(number("=SIGN(0)"), 0.0);
    assert_eq!(number("=SIGN(2.5)"), 1.0);
    assert_eq!(number("=SIGN([.C9])"), 0.0);
    assert_eq!(error("=SIGN(\"x\")"), FormulaError::Value);
}

#[test]
fn large_and_small_count_from_either_end_of_the_numbers() {
    // §6.18.40, §6.18.70, over B1:B7 = 10, 20, 30, 40, "x", 5, 30 — text is skipped.
    assert_eq!(number("=LARGE([.B1:.B7];1)"), 40.0);
    assert_eq!(number("=LARGE([.B1:.B7];2)"), 30.0);
    assert_eq!(number("=LARGE([.B1:.B7];3)"), 30.0); // a repeated value fills two places
    assert_eq!(number("=LARGE([.B1:.B7];6)"), 5.0);
    assert_eq!(number("=SMALL([.B1:.B7];1)"), 5.0);
    assert_eq!(number("=SMALL([.B1:.B7];2)"), 10.0);
    assert_eq!(number("=LARGE(5;1)"), 5.0);
    assert_eq!(number("=LARGE([.B1:.B7];[.B6])"), 10.0); // N from a cell: B6 is 5
    // A fractional N: "ROUNDUP(N;0) = N" for LARGE and "ROUNDDOWN(N;0) = N" for SMALL are
    // the two constraints, and the oracle applies each as the rounding it names.
    assert_eq!(number("=LARGE([.B1:.B7];1.5)"), 30.0);
    assert_eq!(number("=LARGE([.B1:.B7];0.2)"), 40.0);
    assert_eq!(number("=SMALL([.B1:.B7];1.9)"), 5.0);
    assert_eq!(number("=SMALL([.B1:.B7];6.9)"), 40.0);
    // "If the resulting N is <1 or larger than the size of List, Error is returned."
    assert_eq!(error("=LARGE([.B1:.B7];7)"), FormulaError::Value);
    assert_eq!(error("=LARGE([.B1:.B7];0)"), FormulaError::Value);
    assert_eq!(error("=LARGE([.B1:.B7];-1)"), FormulaError::Value);
    assert_eq!(error("=SMALL([.B1:.B7];0.5)"), FormulaError::Value);
    assert_eq!(error("=SMALL([.C1:.C7];1)"), FormulaError::Value); // no numbers at all
    assert_eq!(error("=LARGE([.B1:.B7])"), FormulaError::Value);
}

#[test]
fn median_is_the_middle_number_or_the_mean_of_the_middle_two() {
    // §6.18.47.
    assert_eq!(number("=MEDIAN([.B1:.B7])"), 25.0); // 5 10 20 | 30 30 40 → (20+30)/2
    assert_eq!(number("=MEDIAN(1;2;3;4)"), 2.5);
    assert_eq!(number("=MEDIAN(3;1;2)"), 2.0);
    assert_eq!(number("=MEDIAN(5)"), 5.0);
    assert_eq!(number("=MEDIAN([.B1:.B4];TRUE())"), 20.0); // an argument TRUE is 1
    assert_eq!(number("=MEDIAN([.B1:.B7];[.B1:.B7])"), 25.0);
    // Measured: nothing to rank is #VALUE!, not AVERAGE's #DIV/0!.
    assert_eq!(error("=MEDIAN([.C1:.C7])"), FormulaError::Value);
}

#[test]
fn rank_counts_the_numbers_ahead_of_a_value() {
    // §6.18.65, over the same column: descending by default, ties share a rank.
    assert_eq!(number("=RANK(40;[.B1:.B7])"), 1.0);
    assert_eq!(number("=RANK(30;[.B1:.B7])"), 2.0);
    assert_eq!(number("=RANK(5;[.B1:.B7])"), 6.0);
    assert_eq!(number("=RANK(5;[.B1:.B7];0)"), 6.0);
    assert_eq!(number("=RANK(30;[.B1:.B7];)"), 2.0); // an empty Order is 0
    assert_eq!(number("=RANK(30;[.B1:.B7];1)"), 4.0); // any non-zero order ascends
    assert_eq!(number("=RANK(10;[.B1:.B7];2)"), 2.0);
    assert_eq!(number("=RANK(\"10\";[.B1:.B7])"), 5.0); // Value is a Number, converted
    assert_eq!(number("=RANK([.B1];[.B1:.B7])"), 5.0);
    assert_eq!(number("=RANK(30;30)"), 1.0);
    // "If Value does not exist in Data an Error is returned" — #N/A, measured; and an
    // exact comparison, so a near miss is a miss.
    assert_eq!(error("=RANK(7;[.B1:.B7])"), FormulaError::NA);
    assert_eq!(error("=RANK(30.0000000000001;[.B1:.B7])"), FormulaError::NA);
    // No numbers to rank among is #VALUE! rather than "not found" (measured).
    assert_eq!(error("=RANK(30;[.C1:.C7])"), FormulaError::Value);
    assert_eq!(error("=RANK(\"x\";[.B1:.B7])"), FormulaError::Value);
}

#[test]
fn the_ifs_functions_match_every_criterion_at_one_position() {
    // §6.16.63, §6.13.10, §6.18.6: one cell position counts only if each range's cell at it
    // matches that range's criterion. "apple" matches APPLE too: a criterion is `=`, and `=`
    // folds case (§6.4.7).
    assert_eq!(number("=SUMIFS([.B1:.B7];[.A1:.A7];\"apple\")"), 70.0); // B5's "x" is no number
    assert_eq!(
        number("=SUMIFS([.B1:.B7];[.A1:.A7];\"apple\";[.C1:.C7];\"red\")"),
        40.0
    );
    assert_eq!(
        number("=SUMIFS([.B1:.B7];[.A1:.A7];\"app*\";[.B1:.B7];\">10\")"),
        60.0
    );
    assert_eq!(number("=SUMIFS([.B1:.B7];[.A1:.A7];\"zzz\")"), 0.0);
    assert_eq!(number("=SUMIFS([.B1:.B7];[.A1:.A7];[.A1])"), 70.0); // a criterion cell
    assert_eq!(number("=SUMIFS([.B1:.B7];[.A1:.A7];\"<>apple\")"), 65.0);
    // Two columns wide, position by position: only column A says "apple".
    assert_eq!(number("=SUMIFS([.B1:.C7];[.A1:.B7];\"apple\")"), 70.0);

    assert_eq!(number("=COUNTIFS([.A1:.A7];\"apple\")"), 4.0);
    assert_eq!(
        number("=COUNTIFS([.A1:.A7];\"apple\";[.C1:.C7];\"red\")"),
        3.0
    );
    assert_eq!(
        number("=COUNTIFS([.B1:.B7];\">=20\";[.B1:.B7];\"<40\")"),
        3.0
    );
    // §4.11.8's table for empty cells, unchanged from COUNTIF.
    assert_eq!(number("=COUNTIFS([.D1:.D7];\"\")"), 5.0);
    assert_eq!(number("=COUNTIFS([.D1:.D7];\"=\")"), 5.0);
    assert_eq!(number("=COUNTIFS([.D1:.D7];\"<>\")"), 2.0);

    assert!((number("=AVERAGEIFS([.B1:.B7];[.A1:.A7];\"apple\")") - 70.0 / 3.0).abs() < 1e-12);
    assert_eq!(
        number("=AVERAGEIFS([.B1:.B7];[.C1:.C7];\"red\";[.A1:.A7];\"apple\")"),
        20.0
    );
    // "If no numbers are in the result set to be averaged, an Error is returned."
    assert_eq!(
        error("=AVERAGEIFS([.B1:.B7];[.A1:.A7];\"zzz\")"),
        FormulaError::DivZero
    );
    assert_eq!(
        error("=AVERAGEIFS([.B5];[.A5];\"apple\")"),
        FormulaError::DivZero
    );
}

#[test]
fn the_ifs_functions_refuse_ranges_of_different_shapes() {
    // "All reference ranges shall have the same dimension and size, else an Error."
    assert_eq!(
        error("=SUMIFS([.B1:.B7];[.A1:.A6];\"apple\")"),
        FormulaError::Value
    );
    assert_eq!(
        error("=COUNTIFS([.A1:.A7];\"apple\";[.B1:.B6];\">1\")"),
        FormulaError::Value
    );
    // "Does not accept constant values as the reference parameter."
    assert_eq!(error("=SUMIFS(5;[.A1:.A7];\"apple\")"), FormulaError::Value);
    assert_eq!(error("=COUNTIFS(5;5)"), FormulaError::Value);
    // A range without its criterion is a malformed call.
    assert_eq!(error("=SUMIFS([.B1:.B7];[.A1:.A7])"), FormulaError::Value);
    assert_eq!(
        error("=SUMIFS([.B1:.B7];[.A1:.A7];\"apple\";[.C1:.C7])"),
        FormulaError::Value
    );
    assert_eq!(error("=COUNTIFS([.A1:.A7])"), FormulaError::Value);
}

/// The oracle's document for Excel's `MAXIFS` and `MINIFS`, and for what a matched error does,
/// which [`table`] has no cell for:
///
/// | | A | B | C | D |
/// |---|---|---|---|---|
/// | 1 | x | 5 | 1 | -3 |
/// | 2 | x | 9 | 2 | -7 |
/// | 3 | y | 100 | 3 | -1 |
/// | 4 | x | "12" | 4 | -9 |
/// | 5 | x | TRUE | 5 | |
/// | 6 | y | 3 | 6 | |
/// | 7 | x | | 7 | |
/// | 8 | x | =1/0 | 8 | |
fn mixed(formula: &str) -> Value {
    use CellValue::{Number as N, Text as T};
    let mut sheet = Sheet::new("P");
    let a = ["x", "x", "y", "x", "x", "y", "x", "x"];
    let b = [
        N(5.0),
        N(9.0),
        N(100.0),
        T("12".into()),
        CellValue::Bool(true),
        N(3.0),
    ];
    for (r, a) in a.iter().enumerate() {
        let row = r as u32;
        sheet.set(Pos::new(row, 0), T((*a).into()));
        if let Some(b) = b.get(r) {
            sheet.set(Pos::new(row, 1), b.clone());
        }
        sheet.set(Pos::new(row, 2), N(f64::from(row + 1)));
    }
    sheet.set_formula(Pos::new(7, 1), "=1/0".into());
    for (r, d) in [-3.0, -7.0, -1.0, -9.0].into_iter().enumerate() {
        sheet.set(Pos::new(r as u32, 3), N(d));
    }
    let document = Document {
        sheets: vec![sheet],
        ..Default::default()
    };
    Engine::new(&document).eval(formula, Address::new(0, Pos::new(20, 5)))
}

#[track_caller]
fn mixed_is(formula: &str, want: Value) {
    assert_eq!(mixed(formula), want, "{formula}");
}

#[test]
fn excels_maxifs_and_minifs_take_the_extreme_of_what_matched() {
    let n = Value::Number;
    // Text in the range is skipped, and so is a logical: the oracle answers 1 for MINIFS
    // because it has no logical type and saves TRUE as 1, and §6.3.7 leaves a distinct
    // logical out of a number sequence, as MIN does.
    mixed_is("=COM.MICROSOFT.MAXIFS([.B1:.B6];[.A1:.A6];\"x\")", n(9.0));
    mixed_is("=COM.MICROSOFT.MINIFS([.B1:.B6];[.A1:.A6];\"x\")", n(5.0));
    mixed_is("=MIN([.B1:.B2];[.B4:.B5])", n(5.0));
    mixed_is("=COM.MICROSOFT.MAXIFS([.B1:.B7];[.A1:.A7];\"x\")", n(9.0));
    // Nothing matched is 0, for either; all-negative numbers are not floored at 0.
    mixed_is("=COM.MICROSOFT.MAXIFS([.B1:.B6];[.A1:.A6];\"z\")", n(0.0));
    mixed_is("=COM.MICROSOFT.MINIFS([.B1:.B6];[.A1:.A6];\"z\")", n(0.0));
    mixed_is("=COM.MICROSOFT.MAXIFS([.D1:.D4];[.A1:.A4];\"x\")", n(-3.0));
    mixed_is("=COM.MICROSOFT.MINIFS([.D1:.D8];[.A1:.A8];\"x\")", n(-9.0));
    mixed_is("=COM.MICROSOFT.MAXIFS([.D1:.D8];[.A1:.A8];\"y\")", n(-1.0));
    // Every criterion at one position, wildcards, blanks and case as SUMIFS has them.
    mixed_is(
        "=COM.MICROSOFT.MAXIFS([.C1:.C8];[.A1:.A8];\"x\";[.C1:.C8];\"<5\")",
        n(4.0),
    );
    mixed_is("=COM.MICROSOFT.MINIFS([.C1:.C8];[.B1:.B8];\">4\")", n(1.0));
    mixed_is("=COM.MICROSOFT.MAXIFS([.C1:.C8];[.A1:.A8];\"x*\")", n(8.0));
    mixed_is("=COM.MICROSOFT.MAXIFS([.C1:.C8];[.A1:.A8];\"X\")", n(8.0));
    mixed_is("=COM.MICROSOFT.MAXIFS([.C1:.C8];[.A1:.A8];[.E1])", n(0.0));
    mixed_is("=COM.MICROSOFT.MAXIFS([.C1:.C8];[.B1:.B8];\"\")", n(7.0));
    mixed_is("=COM.MICROSOFT.MINIFS([.C1:.C8];[.B1:.B8];\"<>\")", n(1.0));
    mixed_is("=COM.MICROSOFT.MAXIFS([.C1:.D8];[.A1:.B8];\"x\")", n(8.0));
    // A matched error is the answer, one that did not match is skipped.
    let div = Value::Error(FormulaError::DivZero);
    mixed_is("=COM.MICROSOFT.MAXIFS([.B1:.B8];[.A1:.A8];\"x\")", div);
    mixed_is("=COM.MICROSOFT.MAXIFS([.B1:.B8];[.A1:.A8];\"y\")", n(100.0));
    mixed_is(
        "=COM.MICROSOFT.MAXIFS([.B1:.B8];[.A1:.A8];\"x\";[.C1:.C8];\"<>8\")",
        n(9.0),
    );
    // Err:502 and Err:511 to the oracle; #VALUE!, as SUMIFS answers both.
    let value = Value::Error(FormulaError::Value);
    for refused in [
        "=COM.MICROSOFT.MAXIFS([.C1:.C8];[.A1:.A7];\"x\")",
        "=COM.MICROSOFT.MAXIFS([.C1:.C8];[.A1:.A8])",
        "=COM.MICROSOFT.MAXIFS(5;[.A1:.A8];\"x\")",
        "=COM.MICROSOFT.MAXIFS([.C1:.C8])",
        "=COM.MICROSOFT.MAXIFS([.C1:.C8];[.A1:.A8];\"x\";[.C1:.C8])",
    ] {
        mixed_is(refused, value.clone());
    }
    // The oracle reads only the prefixed spelling.
    mixed_is(
        "=MAXIFS([.C1:.C8];[.A1:.A8];\"x\")",
        Value::Error(FormulaError::Name),
    );
}

#[test]
fn a_matched_error_is_what_sumif_and_its_kin_answer() {
    // §6.3.7 keeps an error in a number sequence, and the oracle answers it (§4.1).
    let div = Value::Error(FormulaError::DivZero);
    mixed_is("=SUMIFS([.B1:.B8];[.A1:.A8];\"x\")", div.clone());
    mixed_is("=AVERAGEIFS([.B1:.B8];[.A1:.A8];\"x\")", div.clone());
    mixed_is("=SUMIF([.A1:.A8];\"x\";[.B1:.B8])", div.clone());
    mixed_is("=AVERAGEIF([.A1:.A8];\"x\";[.B1:.B8])", div);
    // 5 + 9: TRUE is a logical, which a sum leaves out (§6.3.7), as above.
    mixed_is("=SUMIFS([.B1:.B6];[.A1:.A6];\"x\")", Value::Number(14.0));
    mixed_is("=SUMIF([.A1:.A8];\"y\";[.B1:.B8])", Value::Number(103.0));
}

// Serial numbers below are the document's default epoch, 1899-12-30: 46053 is 2026-01-31,
// 46380 is 2026-12-24 (a Thursday).

#[test]
fn eomonth_and_edate_add_whole_months() {
    // §6.10.10, §6.10.9: both truncate their arguments first.
    assert_eq!(number("=EOMONTH(DATE(2026;1;15);0)"), 46053.0); // 2026-01-31
    assert_eq!(number("=EOMONTH(DATE(2026;1;15);1)"), 46081.0); // 2026-02-28
    assert_eq!(number("=EOMONTH(DATE(2024;1;31);1)"), 45351.0); // 2024-02-29
    assert_eq!(number("=EOMONTH(DATE(2026;3;10);-14)"), 45688.0); // 2025-01-31
    assert_eq!(number("=EOMONTH(DATE(2026;1;15);1.9)"), 46081.0);
    assert_eq!(number("=EOMONTH(DATE(2026;1;15);-1.9)"), 46022.0); // 2025-12-31
    assert_eq!(number("=EOMONTH(\"2026-05-05\";0)"), 46173.0);
    assert_eq!(number("=EOMONTH(DATE(2026;1;15)+0.75;0)"), 46053.0);
    // A day past the end of the new month is "adjusted to the last day" of it.
    assert_eq!(number("=EDATE(DATE(2026;1;31);1)"), 46081.0); // 2026-02-28
    assert_eq!(number("=EDATE(DATE(2024;1;31);1)"), 45351.0); // 2024-02-29
    assert_eq!(number("=EDATE(DATE(2026;3;31);-1)"), 46081.0);
    assert_eq!(number("=EDATE(DATE(2026;1;15);0)"), 46037.0);
    assert_eq!(number("=EDATE(DATE(2026;1;15)+0.5;0)"), 46037.0); // the time is dropped
    assert_eq!(number("=EDATE(DATE(2026;1;15);2.7)"), 46096.0); // 2026-03-15
    assert_eq!(number("=EDATE(DATE(2026;1;15);-2.7)"), 45976.0); // 2025-11-15
    assert_eq!(number("=EDATE(DATE(2026;5;31);-12)"), 45808.0); // 2025-05-31
}

#[test]
fn weeknum_counts_from_the_week_holding_the_first_of_january() {
    // §6.10.22: Mode picks the day a week starts on; "the week containing January 1 is the
    // first week of the year". 2026-01-01 is a Thursday.
    assert_eq!(number("=WEEKNUM(DATE(2026;1;1))"), 1.0);
    assert_eq!(number("=WEEKNUM(DATE(2026;1;4))"), 2.0); // Sunday starts week 2
    assert_eq!(number("=WEEKNUM(DATE(2026;1;4);2)"), 1.0); // ... unless weeks start Monday
    assert_eq!(number("=WEEKNUM(DATE(2026;1;5);2)"), 2.0);
    assert_eq!(number("=WEEKNUM(DATE(2026;1;6);13)"), 1.0); // Wednesday starts
    assert_eq!(number("=WEEKNUM(DATE(2026;1;10);16)"), 3.0); // Saturday starts
    assert_eq!(number("=WEEKNUM(DATE(2026;1;10);17)"), 2.0); // Sunday, as mode 1
    assert_eq!(number("=WEEKNUM(DATE(2026;1;10);11)"), 2.0); // Monday, as mode 2
    assert_eq!(number("=WEEKNUM(DATE(2026;12;26))"), 52.0);
    assert_eq!(number("=WEEKNUM(DATE(2028;12;30))"), 53.0);
    // Read literally, and the oracle does: the last days of December are in the week holding
    // the *next* January 1, so they are week 1 — not Excel's 53.
    assert_eq!(number("=WEEKNUM(DATE(2026;12;27))"), 1.0);
    assert_eq!(number("=WEEKNUM(DATE(2026;12;31))"), 1.0);
    assert_eq!(number("=WEEKNUM(DATE(2026;12;27);2)"), 52.0); // a Sunday, weeks from Monday
    assert_eq!(number("=WEEKNUM(DATE(2026;12;28);2)"), 1.0);
    // Modes 21 and 150 are ISO 8601.
    assert_eq!(number("=WEEKNUM(DATE(2026;1;1);21)"), 1.0);
    assert_eq!(number("=WEEKNUM(DATE(2027;1;1);21)"), 53.0);
    assert_eq!(number("=WEEKNUM(DATE(2026;1;1);150)"), 1.0);
    assert_eq!(number("=WEEKNUM(DATE(2026;1;1);1.5)"), 1.0); // Mode truncates
    assert_eq!(error("=WEEKNUM(DATE(2026;1;1);3)"), FormulaError::Num);
    assert_eq!(error("=WEEKNUM(DATE(2026;1;1);0)"), FormulaError::Num);
    assert_eq!(error("=WEEKNUM(DATE(2026;1;1);\"x\")"), FormulaError::Value);
}

#[test]
fn isoweeknum_is_the_week_of_its_thursday() {
    // §6.10.12.
    assert_eq!(number("=ISOWEEKNUM(DATE(2026;1;1))"), 1.0);
    assert_eq!(number("=ISOWEEKNUM(DATE(2027;1;1))"), 53.0); // a Friday: 2026's last week
    assert_eq!(number("=ISOWEEKNUM(DATE(2020;12;31))"), 53.0);
    assert_eq!(number("=ISOWEEKNUM(DATE(2021;1;3))"), 53.0);
    assert_eq!(number("=ISOWEEKNUM(DATE(2024;12;30))"), 1.0); // a Monday: 2025's first
    assert_eq!(number("=ISOWEEKNUM(DATE(2026;6;15))"), 25.0);
}

#[test]
fn networkdays_counts_both_ends_and_skips_weekends_and_holidays() {
    // §6.10.15. 2026-12-21 is a Monday, 2026-12-31 a Thursday; D1:D2 holds the 25th and
    // the 24th, E1:E7 a week with Friday and Saturday off, F1:F7 one with no working day.
    let span = "DATE(2026;12;21);DATE(2026;12;31)";
    assert_eq!(number(&format!("=NETWORKDAYS({span})")), 9.0);
    assert_eq!(number(&format!("=NETWORKDAYS({span};[.D1:.D2])")), 7.0);
    assert_eq!(
        number(&format!("=NETWORKDAYS({span};DATE(2026;12;25))")),
        8.0
    );
    assert_eq!(
        number(&format!("=NETWORKDAYS({span};DATE(2026;12;26))")),
        9.0
    ); // a Saturday
    assert_eq!(number(&format!("=NETWORKDAYS({span};[.D1:.D7])")), 7.0); // blanks are nothing
    assert_eq!(number(&format!("=NETWORKDAYS({span};[.A1:.A2])")), 9.0); // and so is text
    assert_eq!(number(&format!("=NETWORKDAYS({span};;[.E1:.E7])")), 9.0);
    assert_eq!(
        number(&format!("=NETWORKDAYS({span};[.D1:.D2];[.E1:.E7])")),
        8.0
    );
    assert_eq!(number(&format!("=NETWORKDAYS({span};;[.F1:.F7])")), 0.0);
    // Backwards is negative; a date's time is dropped; one day counts itself.
    assert_eq!(
        number("=NETWORKDAYS(DATE(2026;12;31);DATE(2026;12;21))"),
        -9.0
    );
    assert_eq!(
        number("=NETWORKDAYS(DATE(2026;12;31);DATE(2026;12;21);[.D1:.D2])"),
        -7.0
    );
    assert_eq!(
        number("=NETWORKDAYS(DATE(2026;12;21)+0.9;DATE(2026;12;31)+0.1)"),
        9.0
    );
    assert_eq!(
        number("=NETWORKDAYS(DATE(2026;12;21);DATE(2026;12;21))"),
        1.0
    );
    assert_eq!(
        number("=NETWORKDAYS(DATE(2026;12;26);DATE(2026;12;27))"),
        0.0
    );
    assert_eq!(number("=NETWORKDAYS(\"2026-12-21\";\"2026-12-31\")"), 9.0);
    // A work week is seven flags, Sunday first, or it is not one.
    assert_eq!(
        error(&format!("=NETWORKDAYS({span};;[.E1:.E6])")),
        FormulaError::Value
    );
    assert_eq!(
        error(&format!("=NETWORKDAYS({span};[.D1];[.B1:.B7])")),
        FormulaError::Value
    );
    // Decades of days are counted, not walked.
    assert_eq!(
        number("=NETWORKDAYS(DATE(1900;1;1);DATE(9999;12;31))"),
        2_113_190.0
    );
}

#[test]
fn workday_steps_over_weekends_and_holidays() {
    // §6.10.23, from Thursday 2026-12-24 (46380).
    assert_eq!(number("=WORKDAY(DATE(2026;12;24);1)"), 46381.0); // Friday the 25th
    assert_eq!(number("=WORKDAY(DATE(2026;12;24);1;[.D1:.D2])"), 46384.0); // Monday the 28th
    assert_eq!(number("=WORKDAY(DATE(2026;12;28);-1;[.D1:.D2])"), 46379.0); // the 23rd
    assert_eq!(number("=WORKDAY(DATE(2026;12;24);5)"), 46387.0);
    assert_eq!(number("=WORKDAY(DATE(2026;12;24);2.9)"), 46384.0); // Offset truncates
    assert_eq!(number("=WORKDAY(DATE(2026;12;24);-2.9)"), 46378.0);
    assert_eq!(number("=WORKDAY(DATE(2026;12;24)+0.5;1)"), 46381.0);
    assert_eq!(number("=WORKDAY(DATE(2026;12;26);1)"), 46384.0); // from a Saturday
    assert_eq!(number("=WORKDAY(DATE(2026;12;26);-1)"), 46381.0);
    assert_eq!(
        number("=WORKDAY(DATE(2026;12;24);1;\"2026-12-25\")"),
        46384.0
    );
    // "If Offset is zero, then Date is returned" — a holiday or a Saturday included.
    assert_eq!(number("=WORKDAY(DATE(2026;12;26);0)"), 46382.0);
    assert_eq!(number("=WORKDAY(DATE(2026;12;25);0;[.D1])"), 46381.0);
    // The fourth parameter is §6.10.23's, and LibreOffice 26.8 refuses it (Err:504); these
    // follow the spec. Friday and Saturday off: Thursday + 1 is Sunday the 27th.
    assert_eq!(number("=WORKDAY(DATE(2026;12;24);1;;[.E1:.E7])"), 46383.0);
    assert_eq!(number("=WORKDAY(DATE(2026;12;24);0;;[.F1:.F7])"), 46380.0);
    // "If all seven numbers in Workdays are non-zero and Offset is also non-zero, WORKDAY
    // returns an error."
    assert_eq!(
        error("=WORKDAY(DATE(2026;12;24);1;;[.F1:.F7])"),
        FormulaError::Value
    );
    // A long offset is jumped, not walked: 260 working days is 52 weeks.
    assert_eq!(number("=WORKDAY(DATE(2026;12;24);260)"), 46380.0 + 364.0);
    assert_eq!(number("=WORKDAY(DATE(2026;12;24);-260)"), 46380.0 - 364.0);
    // The start day is not stepped onto, so of the two holidays only the 25th costs a day.
    assert_eq!(
        number("=WORKDAY(DATE(2026;12;24);260;[.D1:.D2])"),
        46380.0 + 365.0
    );
}

/// Fifteen significant digits, the most the oracle writes.
fn close(formula: &str, want: f64) {
    let got = number(formula);
    assert!(
        (got - want).abs() <= 1e-14 * want.abs().max(1.0),
        "{formula}: {got}, not {want}"
    );
}

#[test]
fn ipmt_and_ppmt_split_a_payment_into_interest_and_principal() {
    // §6.12.23, §6.12.37: the interest is the rate on the balance the period starts with,
    // and the principal is the rest of PMT's payment.
    close("=IPMT(0.1/12;1;36;8000)", -66.6666666666667);
    // The last period's interest is a small difference of large terms. Exact rational
    // arithmetic over the same doubles gives -2.1333677483528916…; LibreOffice writes
    // -2.13336774835317, off in the twelfth digit, and this is the exact one.
    close("=IPMT(0.1/12;36;36;8000)", -2.133_367_748_352_891_6);
    close("=IPMT(0.1;3;3;8000)", -292.447129909365);
    close("=IPMT(0.05;2;10;-1000;500)", 48.0123856258636);
    close("=IPMT(0.1;2;3;8000;1000)", -528.09667673716);
    close("=IPMT(-0.1;2;3;8000)", 504.79704797048);
    // Paid at the start of each period, the first owes no interest yet.
    assert_eq!(number("=IPMT(0.1;1;3;8000;0;1)"), 0.0);
    close("=IPMT(0.1;2;3;8000;0;1)", -507.552870090635);
    close("=IPMT(0.1;2;3;8000;;1)", -507.552870090635);
    close("=IPMT(0.1;2;3;8000;0;2)", -507.552870090635); // a flag, as PMT reads it
    assert_eq!(number("=IPMT(0;2;10;1000)"), 0.0);
    // A period between two whole ones is a balance part-way through, measured.
    close("=IPMT(0.1;1.5;3;8000)", -682.03299535915);

    close("=PPMT(0.1/12;1;24;2000)", -75.6231860083664);
    close("=PPMT(0.08;10;10;200000)", -27598.0534624214);
    close("=PPMT(0.1;1;3;8000;0;1)", -2924.47129909365);
    close("=PPMT(0.1;2;3;8000;0;1)", -2416.91842900302);
    close("=PPMT(0.05;2;10;-1000;500)", 41.7399018568648);
    close("=PPMT(0.1;2;3;8000;1000)", -2990.93655589124);
    assert_eq!(number("=PPMT(0;2;10;1000)"), -100.0);
    close("=PPMT(0.1;1.5;3;8000)", -2534.88543364387);

    // A period outside 1..Nper is no period of this loan.
    for f in ["IPMT", "PPMT"] {
        assert_eq!(error(&format!("={f}(0.1;0;3;8000)")), FormulaError::Num);
        assert_eq!(error(&format!("={f}(0.1;4;3;8000)")), FormulaError::Num);
        assert_eq!(error(&format!("={f}(0.1;3.5;3;8000)")), FormulaError::Num);
        assert_eq!(error(&format!("={f}(0.1;2;3)")), FormulaError::Value);
    }
}

#[test]
fn address_spells_a_reference_as_text() {
    // §6.14.2 and its Table 21.
    assert_eq!(text("=ADDRESS(1;1)"), "$A$1");
    assert_eq!(text("=ADDRESS(2;3;2)"), "C$2");
    assert_eq!(text("=ADDRESS(2;3;3)"), "$C2");
    assert_eq!(text("=ADDRESS(2;3;4)"), "C2");
    assert_eq!(text("=ADDRESS(2;3;1;FALSE())"), "R2C3");
    assert_eq!(text("=ADDRESS(2;3;2;FALSE())"), "R2C[3]");
    assert_eq!(text("=ADDRESS(2;3;3;FALSE())"), "R[2]C3");
    assert_eq!(text("=ADDRESS(2;3;4;FALSE())"), "R[2]C[3]");
    assert_eq!(text("=ADDRESS(1;27)"), "$AA$1");
    assert_eq!(text("=ADDRESS(1;16384)"), "$XFD$1");
    assert_eq!(text("=ADDRESS(1048576;1)"), "$A$1048576");
    assert_eq!(text("=ADDRESS(2.9;3.9)"), "$C$2"); // truncated
    assert_eq!(text("=ADDRESS(\"2\";\"3\")"), "$C$2");
    assert_eq!(text("=ADDRESS(1;1;;)"), "$A$1"); // empty Abs and A1Style are the defaults
    // "followed by a '.' … if A1Style is TRUE, or a '!' … if A1Style is FALSE".
    assert_eq!(text("=ADDRESS(2;3;1;TRUE();\"Sheet2\")"), "Sheet2.$C$2");
    assert_eq!(text("=ADDRESS(2;3;1;FALSE();\"Sheet2\")"), "Sheet2!R2C3");
    assert_eq!(text("=ADDRESS(1;1;1;TRUE();\"\")"), "$A$1");
    // Quoted when the name is not a plain word — measured, so INDIRECT can read it back.
    assert_eq!(
        text("=ADDRESS(2;3;1;TRUE();\"My Sheet\")"),
        "'My Sheet'.$C$2"
    );
    assert_eq!(text("=ADDRESS(2;3;4;TRUE();\"it's\")"), "'it''s'.C2");
    assert_eq!(text("=ADDRESS(1;1;1;1;\"Sheet-1\")"), "'Sheet-1'.$A$1");
    assert_eq!(text("=ADDRESS(1;1;1;1;\"1abc\")"), "'1abc'.$A$1");
    assert_eq!(text("=ADDRESS(1;1;1;1;\"a.b\")"), "'a.b'.$A$1");
    assert_eq!(text("=ADDRESS(1;1;1;TRUE();5)"), "'5'.$A$1");
    assert_eq!(text("=ADDRESS(1;1;1;1;\"Ä\")"), "Ä.$A$1");
    assert_eq!(text("=ADDRESS(1;1;1;1;\"A_b\")"), "A_b.$A$1");
    assert_eq!(text("=ADDRESS(1;1;1;0;\"My Sheet\")"), "'My Sheet'!R1C1");
    // Measured: 5 to 8 read as 1 to 4.
    assert_eq!(text("=ADDRESS(1;1;6)"), "A$1");
    assert_eq!(text("=ADDRESS(1;1;8)"), "A1");
    // Off the sheet, or an Abs that names nothing.
    assert_eq!(error("=ADDRESS(0;1)"), FormulaError::Value);
    assert_eq!(error("=ADDRESS(1;16385)"), FormulaError::Value);
    assert_eq!(error("=ADDRESS(1048577;1)"), FormulaError::Value);
    assert_eq!(error("=ADDRESS(;1)"), FormulaError::Value);
    assert_eq!(error("=ADDRESS(1;1;0)"), FormulaError::Value);
    assert_eq!(error("=ADDRESS(1;1;9)"), FormulaError::Value);
    assert_eq!(error("=ADDRESS(1;1;1;\"x\")"), FormulaError::Value);
}

/// The oracle's document for the two normal-distribution functions and Excel's `GAMMA.DIST`,
/// measured against LibreOffice 26.8 (`doc/ods-format.md` §4.1).
#[test]
fn the_standard_normal_and_its_inverse_agree_with_the_oracle() {
    // §6.18.55.
    close("=LEGACY.NORMSINV(0.025)", -1.959_963_984_540_05);
    close("=LEGACY.NORMSINV(0.975)", 1.959_963_984_540_05);
    close("=LEGACY.NORMSINV(0.75)", 0.674_489_750_196_082);
    close("=LEGACY.NORMSINV(0.001)", -3.090_232_306_167_81);
    close("=LEGACY.NORMSINV(1E-10)", -6.361_340_902_404_06);
    close("=LEGACY.NORMSINV(0.9999999)", 5.199_337_582_290_66);
    close("=LEGACY.NORMSINV(1E-300)", -37.047_096_299_361_2);
    close("=LEGACY.NORMSINV(\"0.3\")", -0.524_400_512_708_041);
    assert_eq!(number("=LEGACY.NORMSINV(0.5)"), 0.0);
    // `0 < P < 1`: the oracle answers #VALUE! at either end and Err:502 past them; the domain
    // error is #NUM!, as everywhere else here.
    for p in ["0", "1", "-0.1"] {
        assert_eq!(
            error(&format!("=LEGACY.NORMSINV({p})")),
            FormulaError::Num,
            "{p}"
        );
    }
    assert_eq!(error("=LEGACY.NORMSINV(\"x\")"), FormulaError::Value);
    assert_eq!(error("=LEGACY.NORMSINV()"), FormulaError::Value);

    // Excel's NORM.S.DIST: §6.18.52's NORMDIST with a mean of 0 and a deviation of 1.
    close(
        "=COM.MICROSOFT.NORM.S.DIST(1.96;TRUE())",
        0.975_002_104_851_78,
    );
    close(
        "=COM.MICROSOFT.NORM.S.DIST(-1;TRUE())",
        0.158_655_253_931_457,
    );
    close(
        "=COM.MICROSOFT.NORM.S.DIST(-3.2;TRUE())",
        0.000_687_137_937_915_847,
    );
    close(
        "=COM.MICROSOFT.NORM.S.DIST(-8;TRUE())",
        6.220_960_574_271_76e-16,
    );
    close(
        "=COM.MICROSOFT.NORM.S.DIST(8;TRUE())",
        0.999_999_999_999_999,
    );
    close(
        "=COM.MICROSOFT.NORM.S.DIST(0;FALSE())",
        0.398_942_280_401_433,
    );
    close(
        "=COM.MICROSOFT.NORM.S.DIST(1;FALSE())",
        0.241_970_724_519_143,
    );
    close("=COM.MICROSOFT.NORM.S.DIST(0.5;1)", 0.691_462_461_274_013);
    close("=COM.MICROSOFT.NORM.S.DIST(0.5;0)", 0.352_065_326_764_3);
    close("=COM.MICROSOFT.NORM.S.DIST(;1)", 0.5);
    assert_eq!(number("=COM.MICROSOFT.NORM.S.DIST(-40;TRUE())"), 0.0);
    // Excel's Cumulative is required.
    assert_eq!(error("=COM.MICROSOFT.NORM.S.DIST(1)"), FormulaError::Value);
    assert_eq!(
        error("=COM.MICROSOFT.NORM.S.DIST(\"x\";1)"),
        FormulaError::Value
    );
    assert_eq!(
        error("=COM.MICROSOFT.NORM.S.DIST(1;\"x\")"),
        FormulaError::Value
    );
}

#[test]
fn excels_gamma_dist_is_the_gamma_distribution_and_refuses_a_negative_x() {
    close(
        "=COM.MICROSOFT.GAMMA.DIST(2;3;2;TRUE())",
        0.080_301_397_071_394_2,
    );
    close(
        "=COM.MICROSOFT.GAMMA.DIST(2;3;2;FALSE())",
        0.091_969_860_292_860_6,
    );
    close(
        "=COM.MICROSOFT.GAMMA.DIST(5;0.5;1;TRUE())",
        0.998_434_597_741_998,
    );
    close(
        "=COM.MICROSOFT.GAMMA.DIST(60;50;1;TRUE())",
        0.915_593_318_906_311,
    );
    close(
        "=COM.MICROSOFT.GAMMA.DIST(0.0035;12.5;0.0002;TRUE())",
        0.911_796_788_750_756,
    );
    close(
        "=COM.MICROSOFT.GAMMA.DIST(150;200;1;TRUE())",
        5.709_688_574_208_72e-5,
    );
    close(
        "=COM.MICROSOFT.GAMMA.DIST(3;2.5;1.5;FALSE())",
        0.191_967_880_935_78,
    );
    close(
        "=COM.MICROSOFT.GAMMA.DIST(1E-300;0.5;1;TRUE())",
        1.128_379_167_095_52e-150,
    );
    close(
        "=COM.MICROSOFT.GAMMA.DIST(1;1E-5;1;TRUE())",
        0.999_997_806_138_208,
    );
    close(
        "=COM.MICROSOFT.GAMMA.DIST(1E+6;1E+6;1;TRUE())",
        0.500_132_980_418_817,
    );
    assert_eq!(number("=COM.MICROSOFT.GAMMA.DIST(0;3;2;TRUE())"), 0.0);
    assert_eq!(number("=COM.MICROSOFT.GAMMA.DIST(0;3;2;FALSE())"), 0.0);
    assert_eq!(number("=COM.MICROSOFT.GAMMA.DIST(0;1;1;FALSE())"), 1.0);
    assert_eq!(
        number("=COM.MICROSOFT.GAMMA.DIST(0.01;100;0.5;TRUE())"),
        0.0
    );
    assert_eq!(number("=COM.MICROSOFT.GAMMA.DIST(1000;3;2;TRUE())"), 1.0);
    // The density divides by zero at 0 when α < 1.
    assert_eq!(
        error("=COM.MICROSOFT.GAMMA.DIST(0;0.5;1;FALSE())"),
        FormulaError::DivZero
    );
    // Excel's, unlike §6.18.31's: a negative X is refused rather than 0.
    for refused in [
        "-1;3;2;TRUE()",
        "-1;3;2;FALSE()",
        "2;0;2;TRUE()",
        "2;3;0;TRUE()",
        "2;-3;2;TRUE()",
    ] {
        assert_eq!(
            error(&format!("=COM.MICROSOFT.GAMMA.DIST({refused})")),
            FormulaError::Num,
            "{refused}"
        );
    }
    assert_eq!(
        error("=COM.MICROSOFT.GAMMA.DIST(2;3;2)"),
        FormulaError::Value
    );
    assert_eq!(
        error("=COM.MICROSOFT.GAMMA.DIST(\"x\";3;2;TRUE())"),
        FormulaError::Value
    );
}

#[test]
fn fixed_rounds_as_round_does_and_spells_the_result_in_the_documents_locale() {
    // §6.20.10.
    assert_eq!(text("=FIXED(1234.567)"), "1,234.57");
    assert_eq!(text("=FIXED(1234.567;1)"), "1,234.6");
    assert_eq!(text("=FIXED(1234.567;-2)"), "1,200");
    assert_eq!(text("=FIXED(1234.567;2;TRUE())"), "1234.57");
    assert_eq!(text("=FIXED(1234.567;2;1)"), "1234.57");
    assert_eq!(text("=FIXED(1234.567;;TRUE())"), "1234.57");
    assert_eq!(text("=FIXED(1234567;0;FALSE())"), "1,234,567");
    assert_eq!(text("=FIXED(-1234567.891;2)"), "-1,234,567.89");
    assert_eq!(text("=FIXED(1E+15;0)"), "1,000,000,000,000,000");
    assert_eq!(text("=FIXED(1E+20;0)"), "100,000,000,000,000,000,000");
    assert_eq!(text("=FIXED(12;5)"), "12.00000");
    assert_eq!(text("=FIXED(1.5;15)"), "1.500000000000000");
    assert_eq!(text("=FIXED(0;2)"), "0.00");
    // Half away from zero, and a zero left after rounding has no sign.
    assert_eq!(text("=FIXED(2.5;0)"), "3");
    assert_eq!(text("=FIXED(-2.5;0)"), "-3");
    assert_eq!(text("=FIXED(0.5;0)"), "1");
    assert_eq!(text("=FIXED(-0.5;0)"), "-1");
    assert_eq!(text("=FIXED(0.4;0)"), "0");
    assert_eq!(text("=FIXED(0.125;2)"), "0.13");
    assert_eq!(text("=FIXED(2.675;2)"), "2.68");
    assert_eq!(text("=FIXED(-0.001;2)"), "0.00");
    // "Rounds value N": ROUND's rounding, which reads 1.005 as it shows. The oracle's FIXED
    // answers 1.00 here while its own ROUND answers 1.01; the spec says they are one rounding.
    assert_eq!(text("=FIXED(1.005;2)"), "1.01");
    // "If D is a fraction, it is rounded towards 0": -1.9 is -1. The oracle reads it as -2 and
    // answers 1,200; the spec wins.
    assert_eq!(text("=FIXED(1234.5;1.9)"), "1,234.5");
    assert_eq!(text("=FIXED(1234.5;-1.9)"), "1,230");
    assert_eq!(text("=FIXED(1234.567;-5)"), "0");
    assert_eq!(text("=FIXED(123;-3)"), "0");
    assert_eq!(text("=FIXED(15;-15)"), "0");
    assert_eq!(text("=FIXED(\"12.5\";0)"), "13");
    assert_eq!(text("=FIXED(TRUE();1)"), "1.0");
    // The oracle refuses more than fifteen places either way: a double has no more.
    for d in ["16", "-16", "128"] {
        assert_eq!(error(&format!("=FIXED(5;{d})")), FormulaError::Num, "{d}");
    }
    assert_eq!(error("=FIXED(\"x\";1)"), FormulaError::Value);
    assert_eq!(error("=FIXED()"), FormulaError::Value);
    assert_eq!(error("=FIXED(1;2;3;4)"), FormulaError::Value);

    // "using locale-specific settings": the document's. The oracle spells FIXED in its own user
    // interface's locale whatever the document says; the spec says locale, and a document's
    // locale is the one it has.
    let mut document = table();
    document.locale = Some(grind_core::locale::Locale::new("de", "DE"));
    let at = Address::new(0, Pos::new(20, 0));
    assert_eq!(
        Engine::new(&document).eval("=FIXED(-1234567.891;2)", at),
        Value::Text("-1.234.567,89".into())
    );
}

/// §6.14.9: `MATCH`'s search region is `ForceArray`, so an expression there is an array, and
/// `INDEX` over an array in that array mode picks a row or column of it (§6.14.6). Measured
/// against LibreOffice 26.8.
#[test]
fn match_searches_an_array_and_index_picks_from_one() {
    assert_eq!(number("=MATCH(TRUE();INDEX([.B1:.B7]>15;0);0)"), 2.0);
    assert_eq!(number("=MATCH(TRUE();[.B1:.B7]>15;0)"), 2.0);
    assert_eq!(number("=MATCH(TRUE();INDEX([.B1:.B7]>35;0);0)"), 4.0);
    // Text is greater than every number (§6.4.9).
    assert_eq!(number("=MATCH(TRUE();INDEX([.B1:.B7]>100;0);0)"), 5.0);
    assert_eq!(number("=MATCH(TRUE();INDEX([.B1:.B7]>15;0;0);0)"), 2.0);
    assert_eq!(number("=MATCH(60;[.B1:.B4]*2;0)"), 3.0);
    assert_eq!(number("=MATCH(60;INDEX([.B1:.B4]*2;0;1);0)"), 3.0);
    assert_eq!(number("=MATCH(35;[.B1:.B4]+0;1)"), 3.0);
    assert_eq!(number("=MATCH(\"pear\";[.A1:.A7]&\"\";0)"), 2.0);
    assert_eq!(number("=SUMPRODUCT(INDEX([.B1:.B4]*2;0;1))"), 200.0);
    assert_eq!(number("=SUMPRODUCT(INDEX([.B1:.B4]*2;0))"), 200.0);
    assert_eq!(number("=SUMPRODUCT(INDEX([.B1:.B4]*2;2;1))"), 40.0);
    assert_eq!(number("=SUMPRODUCT(INDEX([.B1:.C4]>0;0;1)*1)"), 4.0);
    assert_eq!(
        error("=SUMPRODUCT(INDEX([.B1:.B4]*2;5;1))"),
        FormulaError::Ref
    );
    // A region of two columns is no sequence to search.
    assert_eq!(
        error("=MATCH(TRUE();INDEX([.B1:.C2]>15;0);0)"),
        FormulaError::Value
    );
    // Outside an array argument the range meets the formula at one cell, as before.
    assert_eq!(error("=INDEX([.B1:.B4]*2;3)"), FormulaError::Value);
}

/// §6.14.6: `INDEX` over a reference returns the cell, row or column it picks as a reference —
/// one end of a range, a sequence a function takes whole. Measured against LibreOffice 26.8.
#[test]
fn index_over_a_reference_is_a_reference() {
    assert_eq!(number("=MAX([.B1]:INDEX([.B1:.B7];4))"), 40.0);
    assert_eq!(number("=SUM([.B2]:INDEX([.B1:.B7];4))"), 90.0);
    assert_eq!(number("=AVERAGE([.B1]:INDEX([.B1:.B7];[.B2]/10))"), 15.0);
    assert_eq!(number("=MIN(INDEX([.B1:.B7];2):[.B4])"), 20.0);
    assert_eq!(number("=SUM(INDEX([.B1:.B7];2):INDEX([.B1:.B7];3))"), 50.0);
    assert_eq!(number("=MAX([.B1]:INDEX([.B1:.B7];0))"), 40.0);
    assert_eq!(number("=SUM(INDEX([.B1:.B7];0))"), 135.0);
    assert_eq!(number("=SUM(INDEX([.A1:.B7];0;2))"), 135.0);
    assert_eq!(number("=ROWS(INDEX([.A1:.C7];0;2))"), 7.0);
    assert_eq!(number("=COLUMNS(INDEX([.A1:.C7];2;0))"), 3.0);
    assert_eq!(number("=INDEX([.B1:.B7];3)"), 30.0);
    assert_eq!(error("=INDEX([.B1:.B7];9):[.B1]"), FormulaError::Ref);
}

/// The oracle's document for `OFFSET` and `INDIRECT`: `S` holds `r1`…`r4` in A, 10…40 in B,
/// 1…4 in C and the text `B2` in D1; `Other.A1` is 111, `'My Sheet'.A1` is 222, and `total` names
/// `S.B1:B2`. Formulas sit in `S.B21`, where the oracle's did.
fn referring(formula: &str) -> Value {
    let mut s = Sheet::new("S");
    for i in 0..4u32 {
        s.set(Pos::new(i, 0), CellValue::Text(format!("r{}", i + 1)));
        s.set(Pos::new(i, 1), CellValue::Number(f64::from(i + 1) * 10.0));
        s.set(Pos::new(i, 2), CellValue::Number(f64::from(i + 1)));
    }
    s.set(Pos::new(0, 3), CellValue::Text("B2".into()));
    let mut other = Sheet::new("Other");
    other.set(Pos::new(0, 0), CellValue::Number(111.0));
    let mut mine = Sheet::new("My Sheet");
    mine.set(Pos::new(0, 0), CellValue::Number(222.0));
    let mut document = Document {
        sheets: vec![s, other, mine],
        ..Default::default()
    };
    document
        .names
        .insert("total".into(), "[$S.$B$1:.$B$2]".into());
    Engine::new(&document).eval(formula, Address::new(0, Pos::new(20, 1)))
}

/// §6.14.11 `OFFSET` and §6.14.7 `INDIRECT` return references, as `INDEX` does. Measured
/// against LibreOffice 26.8 (`doc/ods-format.md` §4.1).
#[test]
fn offset_and_indirect_build_references() {
    let n = |formula: &str| match referring(formula) {
        Value::Number(n) => n,
        other => panic!("{formula} evaluated to {other:?}"),
    };
    let e = |formula: &str| match referring(formula) {
        Value::Error(e) => e,
        other => panic!("{formula} evaluated to {other:?}, not an error"),
    };
    assert_eq!(n("=SUM(OFFSET([.B1];1;0;3))"), 90.0);
    assert_eq!(n("=OFFSET([.B1];2;0)"), 30.0);
    assert_eq!(n("=SUM(OFFSET([.A1:.B2];1;1))"), 55.0);
    assert_eq!(n("=ROWS(OFFSET([.B1];0;0;4;2))"), 4.0);
    assert_eq!(n("=COLUMNS(OFFSET([.B1];0;0;4;2))"), 2.0);
    assert_eq!(n("=SUM(OFFSET([.B1];0;0;;1))"), 10.0);
    assert_eq!(n("=SUM([.B1]:OFFSET([.B1];2;0))"), 60.0);
    assert_eq!(n("=OFFSET([.B1];1.9;0)"), 20.0);
    assert_eq!(n("=OFFSET([.B1];\"1\";0)"), 20.0);
    assert_eq!(n("=SUM(OFFSET([.B1:.B2];1;0;;))"), 50.0);
    // Inside an array argument, too.
    assert_eq!(n("=SUMPRODUCT(OFFSET([.B1];0;0;3)*2)"), 120.0);
    assert_eq!(n("=SUMPRODUCT(INDIRECT(\"B1:B2\")*2)"), 60.0);
    // Off the sheet, or no size at all: a reference that cannot exist. The oracle says Err:502.
    for refused in [
        "=OFFSET([.B1];-1;0)",
        "=OFFSET([.B1];0;-2)",
        "=OFFSET([.B1];0;0;0)",
        "=SUM(OFFSET([.B4];0;0;-2))",
    ] {
        assert_eq!(e(refused), FormulaError::Ref, "{refused}");
    }

    assert_eq!(n("=INDIRECT(\"B3\")"), 30.0);
    assert_eq!(n("=INDIRECT(\"$B$3\")"), 30.0);
    assert_eq!(n("=INDIRECT(\"b3\")"), 30.0);
    assert_eq!(n("=SUM(INDIRECT(\"B1:B3\"))"), 60.0);
    assert_eq!(n("=INDIRECT(\"Other.A1\")"), 111.0);
    assert_eq!(n("=INDIRECT(\"Other!A1\")"), 111.0);
    assert_eq!(n("=INDIRECT(\"$Other.$A$1\")"), 111.0);
    assert_eq!(n("=INDIRECT(\"'My Sheet'.A1\")"), 222.0);
    assert_eq!(n("=INDIRECT(\"'My Sheet'!A1\")"), 222.0);
    assert_eq!(n("=INDIRECT([.D1])"), 20.0);
    assert_eq!(n("=INDIRECT(\"B\"&2)"), 20.0);
    assert_eq!(n("=SUM(INDIRECT(\"total\"))"), 30.0);
    // A whole column, as the same column typed in the formula reads (the engine keeps one to the
    // rows the sheet uses).
    assert_eq!(n("=ROWS(INDIRECT(\"A:A\"))"), n("=ROWS([.A:.A])"));
    assert_eq!(n("=INDIRECT(\"R3C2\";FALSE())"), 30.0);
    assert_eq!(n("=SUM(INDIRECT(\"R1C2:R3C2\";0))"), 60.0);
    // Relative to the formula's own cell, B21: one row down and one column right is empty.
    assert_eq!(n("=INDIRECT(\"R[1]C[1]\";FALSE())"), 0.0);
    assert_eq!(n("=INDIRECT(\"R[-18]C[0]\";FALSE())"), 30.0);
    for refused in [
        "=INDIRECT(\"nonsense\")",
        "=INDIRECT(\"\")",
        "=INDIRECT(\"Other.A1\";FALSE())",
        "=INDIRECT(\"R3C2\")",
        "=INDIRECT(5)",
    ] {
        assert_eq!(e(refused), FormulaError::Ref, "{refused}");
    }
}

/// Excel's 2016–2019 functions, as LibreOffice spells them in ODF and the xlsx import writes them
/// (`doc/small-group.md`). Measured against LibreOffice 26.8.
#[test]
fn excels_ifs_switch_concat_and_textjoin() {
    let t = |f: &str| text(f);
    assert_eq!(
        t("=COM.MICROSOFT.IFS([.B1]>15;\"big\";[.B1]>5;\"mid\")"),
        "mid"
    );
    assert_eq!(
        t("=COM.MICROSOFT.IFS([.B2]>15;\"big\";TRUE();\"small\")"),
        "big"
    );
    assert_eq!(t("=COM.MICROSOFT.IFS(1;\"a\")"), "a");
    // Only the branch taken is evaluated.
    assert_eq!(t("=COM.MICROSOFT.IFS(0;1/0;1;\"ok\")"), "ok");
    assert_eq!(
        error("=COM.MICROSOFT.IFS([.B6]>15;\"big\";[.B6]>8;\"mid\")"),
        FormulaError::NA
    );
    assert_eq!(
        error("=COM.MICROSOFT.IFS(\"x\";\"a\")"),
        FormulaError::Value
    );
    assert_eq!(error("=COM.MICROSOFT.IFS([.B1]>5)"), FormulaError::Value);
    assert_eq!(
        error("=COM.MICROSOFT.IFS(1/0;1;1;2)"),
        FormulaError::DivZero
    );

    assert_eq!(
        number("=COM.MICROSOFT.SWITCH([.A2];\"pear\";1;\"apple\";2)"),
        1.0
    );
    assert_eq!(
        t("=COM.MICROSOFT.SWITCH([.A1];\"pear\";1;\"none\")"),
        "none"
    );
    assert_eq!(
        t("=COM.MICROSOFT.SWITCH([.B1];10;\"ten\";20;\"twenty\")"),
        "ten"
    );
    assert_eq!(
        number("=COM.MICROSOFT.SWITCH(\"APPLE\";\"apple\";1;0)"),
        1.0
    );
    assert_eq!(
        error("=COM.MICROSOFT.SWITCH([.A1];\"pear\";1)"),
        FormulaError::NA
    );
    assert_eq!(error("=COM.MICROSOFT.SWITCH(1;1)"), FormulaError::Value);
    assert_eq!(
        error("=COM.MICROSOFT.SWITCH(1/0;1;2;3)"),
        FormulaError::DivZero
    );

    assert_eq!(
        t("=COM.MICROSOFT.CONCAT([.A1:.A3];\"-\";[.B1])"),
        "applepearapple-10"
    );
    assert_eq!(t("=COM.MICROSOFT.CONCAT([.A1:.B2])"), "apple10pear20");
    assert_eq!(t("=COM.MICROSOFT.CONCAT(1;TRUE();\"\")"), "1TRUE");
    assert_eq!(t("=COM.MICROSOFT.CONCAT([.B5])"), "x");
    assert_eq!(
        error("=COM.MICROSOFT.CONCAT(1/0;\"a\")"),
        FormulaError::DivZero
    );

    assert_eq!(
        t("=COM.MICROSOFT.TEXTJOIN(\", \";TRUE();[.A1:.A4])"),
        "apple, pear, apple, plum"
    );
    assert_eq!(
        t("=COM.MICROSOFT.TEXTJOIN(\"-\";FALSE();[.A1];\"\";[.A2])"),
        "apple--pear"
    );
    assert_eq!(
        t("=COM.MICROSOFT.TEXTJOIN(\"-\";TRUE();[.A1];\"\";[.A2])"),
        "apple-pear"
    );
    assert_eq!(
        t("=COM.MICROSOFT.TEXTJOIN(\"\";TRUE();[.B1:.B3])"),
        "102030"
    );
    // A range of delimiters is used in turn.
    assert_eq!(
        t("=COM.MICROSOFT.TEXTJOIN([.A1:.A2];TRUE();\"x\";\"y\";\"z\")"),
        "xappleypearz"
    );
}

#[test]
fn excels_xlookup_and_xmatch_and_the_dotted_statistics() {
    let t = |f: &str| text(f);
    assert_eq!(
        number("=COM.MICROSOFT.XLOOKUP(\"plum\";[.A1:.A7];[.B1:.B7])"),
        40.0
    );
    assert_eq!(
        number("=COM.MICROSOFT.XLOOKUP(\"APPLE\";[.A1:.A7];[.B1:.B7])"),
        10.0
    );
    assert_eq!(
        error("=COM.MICROSOFT.XLOOKUP(\"kiwi\";[.A1:.A7];[.B1:.B7])"),
        FormulaError::NA
    );
    assert_eq!(
        t("=COM.MICROSOFT.XLOOKUP(\"kiwi\";[.A1:.A7];[.B1:.B7];\"none\")"),
        "none"
    );
    // From the last.
    assert_eq!(
        number("=COM.MICROSOFT.XLOOKUP(\"apple\";[.A1:.A7];[.B1:.B7];;0;-1)"),
        30.0
    );
    // Next smaller and next larger.
    assert_eq!(
        t("=COM.MICROSOFT.XLOOKUP(25;[.B1:.B4];[.A1:.A4];;-1)"),
        "pear"
    );
    assert_eq!(
        t("=COM.MICROSOFT.XLOOKUP(25;[.B1:.B4];[.A1:.A4];;1)"),
        "apple"
    );
    assert_eq!(
        t("=COM.MICROSOFT.XLOOKUP(5;[.B1:.B7];[.A1:.A7];;1)"),
        "banana"
    );
    assert_eq!(
        error("=COM.MICROSOFT.XLOOKUP(41;[.B1:.B4];[.A1:.A4];;1)"),
        FormulaError::NA
    );
    // Wildcards in mode 2 only.
    assert_eq!(
        number("=COM.MICROSOFT.XLOOKUP(\"p*\";[.A1:.A7];[.B1:.B7];;2)"),
        20.0
    );
    assert_eq!(
        error("=COM.MICROSOFT.XLOOKUP(\"p*\";[.A1:.A7];[.B1:.B7])"),
        FormulaError::NA
    );
    // Sorted data searched in halves finds what a scan finds.
    assert_eq!(
        t("=COM.MICROSOFT.XLOOKUP(30;[.B1:.B4];[.A1:.A4];;0;2)"),
        "apple"
    );
    assert_eq!(
        t("=COM.MICROSOFT.XLOOKUP(25;[.B1:.B4];[.A1:.A4];;-1;2)"),
        "pear"
    );
    // A wider return array gives the found row's first cell; a shorter one is no answer.
    assert_eq!(
        number("=COM.MICROSOFT.XLOOKUP(\"pear\";[.A1:.A7];[.B1:.C7])"),
        20.0
    );
    assert_eq!(
        error("=COM.MICROSOFT.XLOOKUP(\"pear\";[.A1:.A7];[.B1:.B5])"),
        FormulaError::Value
    );
    assert_eq!(
        error("=COM.MICROSOFT.XLOOKUP(25;[.B1:.B4];[.A1:.A4];;3)"),
        FormulaError::Value
    );

    assert_eq!(number("=COM.MICROSOFT.XMATCH(\"plum\";[.A1:.A7])"), 4.0);
    assert_eq!(
        number("=COM.MICROSOFT.XMATCH(\"apple\";[.A1:.A7];0;-1)"),
        7.0
    );
    assert_eq!(number("=COM.MICROSOFT.XMATCH(25;[.B1:.B4];1)"), 3.0);
    assert_eq!(number("=COM.MICROSOFT.XMATCH(25;[.B1:.B4];-1)"), 2.0);
    assert_eq!(number("=COM.MICROSOFT.XMATCH(\"p*\";[.A1:.A7];2)"), 2.0);
    assert_eq!(
        error("=COM.MICROSOFT.XMATCH(\"kiwi\";[.A1:.A7])"),
        FormulaError::NA
    );
    // An empty key matches nothing; a parameter's own error is the answer before a shape is.
    assert_eq!(
        error("=COM.MICROSOFT.XMATCH([.Z9];[.E1:.E7])"),
        FormulaError::NA
    );
    assert_eq!(
        error("=COM.MICROSOFT.XMATCH(1;[.A1:.C7];0;1/0)"),
        FormulaError::DivZero
    );
    assert_eq!(
        error("=COM.MICROSOFT.XMATCH(30;[.B1:.B7];0;3)"),
        FormulaError::Value
    );

    close("=COM.MICROSOFT.STDEV.S([.B1:.B7])", 13.322_912_594_474_2);
    close("=COM.MICROSOFT.STDEV.P([.B1:.B7])", 12.162_099_599_438_7);
    close("=COM.MICROSOFT.VAR.S([.B1:.B7])", 177.5);
    close("=COM.MICROSOFT.VAR.P([.B1:.B7])", 147.916_666_666_667);
    assert_eq!(error("=COM.MICROSOFT.STDEV.S(5)"), FormulaError::DivZero);
}

/// The information and text basics everyday workbooks call: parity, references, error types,
/// characters, a cell's formula, and `LOOKUP`. Measured against LibreOffice 26.8.
#[test]
fn parity_references_errors_characters_and_lookup() {
    let b = |f: &str| match eval(f) {
        Value::Bool(b) => b,
        other => panic!("{f} evaluated to {other:?}"),
    };
    assert!(b("=ISEVEN(4)") && !b("=ISEVEN(-3)") && b("=ISEVEN(2.9)") && b("=ISEVEN(-2.5)"));
    assert!(b("=ISEVEN([.B1])") && b("=ISODD(3)") && !b("=ISODD(0)") && b("=ISODD(TRUE())"));
    // §6.3.5: an empty cell is 0, which is even. The oracle refuses it (Err:504); Excel agrees.
    assert!(b("=ISEVEN([.Z9])"));
    assert_eq!(error("=ISEVEN(\"x\")"), FormulaError::Value);

    assert!(b("=ISREF([.A1])") && b("=ISREF([.A1:.B2])") && b("=ISREF(INDIRECT(\"A1\"))"));
    assert!(!b("=ISREF(5)") && !b("=ISREF(1/0)"));

    for (formula, code) in [
        ("=ERROR.TYPE(#NULL!)", 1.0),
        ("=ERROR.TYPE(1/0)", 2.0),
        ("=ERROR.TYPE(#VALUE!)", 3.0),
        ("=ERROR.TYPE(#REF!)", 4.0),
        ("=ERROR.TYPE(#NAME?)", 5.0),
        ("=ERROR.TYPE(#NUM!)", 6.0),
        ("=ERROR.TYPE(NA())", 7.0),
    ] {
        assert_eq!(number(formula), code, "{formula}");
    }
    assert_eq!(error("=ERROR.TYPE(5)"), FormulaError::NA);

    // §6.20.3/§6.20.5: Unicode code points, so CODE(CHAR(N)) is N for 1 to 255.
    assert_eq!(text("=CHAR(65)"), "A");
    assert_eq!(text("=CHAR(65.9)"), "A");
    assert_eq!(text("=CHAR(10)"), "\n");
    assert_eq!(text("=CHAR(169)"), "©");
    assert_eq!(number("=CODE(CHAR(233))"), 233.0);
    assert_eq!(error("=CHAR(256)"), FormulaError::Value);
    // Zero, and a control character XML 1.0 cannot carry in a cell, are empty text, as the
    // oracle gives them; a tab, a line feed and a carriage return are themselves.
    for empty in [
        "=CHAR(0)",
        "=CHAR(0.5)",
        "=CHAR(1)",
        "=UNICHAR(0)",
        "=UNICHAR(2)",
        "=UNICHAR(12)",
    ] {
        assert_eq!(text(empty), "", "{empty}");
    }
    assert_eq!(text("=CHAR(9)&CHAR(13)"), "\t\r");
    assert_eq!(number("=CODE(\"Apple\")"), 65.0);
    assert_eq!(number("=CODE(5)"), 53.0);
    assert_eq!(error("=CODE(\"\")"), FormulaError::Value);
    assert_eq!(text("=UNICHAR(8364)"), "€");
    assert_eq!(text("=UNICHAR(128512)"), "😀");
    assert_eq!(number("=UNICODE(\"€\")"), 8364.0);
    assert_eq!(number("=UNICODE(\"😀\")"), 128_512.0);
    assert_eq!(error("=UNICODE(\"\")"), FormulaError::Value);
    assert_eq!(error("=UNICHAR(55296)"), FormulaError::Value);

    // §6.14.8: the largest value at or below, the last of a run; the result row or column by
    // position, or the searched array's own last row or column.
    assert_eq!(text("=LOOKUP(25;[.B1:.B4];[.A1:.A4])"), "pear");
    assert_eq!(text("=LOOKUP(100;[.B1:.B4];[.A1:.A4])"), "plum");
    assert_eq!(number("=LOOKUP(40;[.B1:.B4])"), 40.0);
    assert_eq!(number("=LOOKUP(\"pear\";[.A1:.A2];[.B1:.B2])"), 20.0);
    assert_eq!(text("=LOOKUP(25;[.B1:.C4])"), "green");
    assert_eq!(text("=LOOKUP(25;[.B1:.B4];[.C1:.C2])"), "green");
    assert_eq!(error("=LOOKUP(5;[.B1:.B4];[.A1:.A4])"), FormulaError::NA);
}

#[test]
fn isformula_and_formula_read_a_cells_formula() {
    let mut sheet = Sheet::new("S");
    sheet.set_formula(Pos::new(0, 0), "=SUM([.B1:.B3])*2".into());
    sheet.set(Pos::new(0, 1), CellValue::Number(1.0));
    let document = Document {
        sheets: vec![sheet],
        ..Default::default()
    };
    let at = Address::new(0, Pos::new(5, 5));
    let eval = |f: &str| Engine::new(&document).eval(f, at);
    assert_eq!(eval("=ISFORMULA([.A1])"), Value::Bool(true));
    assert_eq!(eval("=ISFORMULA([.B1])"), Value::Bool(false));
    assert_eq!(eval("=ISFORMULA(5)"), Value::Bool(false));
    // As the formula bar shows it.
    assert_eq!(eval("=FORMULA([.A1])"), Value::Text("=SUM(B1:B3)*2".into()));
    assert_eq!(eval("=FORMULA([.B1])"), Value::Error(FormulaError::NA));
    assert_eq!(
        eval("=FORMULA([.A1:.B1])"),
        Value::Error(FormulaError::Value)
    );
}

/// §6.20.23 `TEXT`: a value spelled in a format code, read as `numfmt::code` reads one (the
/// language Excel's files and LibreOffice's dialog share), in the document's locale. Measured
/// against LibreOffice 26.8.
#[test]
fn text_spells_a_value_in_a_format_code() {
    for (formula, want) in [
        ("=TEXT(1234.567;\"#,##0.00\")", "1,234.57"),
        ("=TEXT(1234.567;\"0\")", "1235"),
        ("=TEXT(0.256;\"0%\")", "26%"),
        ("=TEXT(0.256;\"0.0%\")", "25.6%"),
        ("=TEXT(46000;\"dd.mm.yyyy\")", "09.12.2025"),
        ("=TEXT(46000;\"yyyy-mm-dd\")", "2025-12-09"),
        ("=TEXT(46000;\"mmmm d, yyyy\")", "December 9, 2025"),
        ("=TEXT(46000.75;\"hh:mm\")", "18:00"),
        ("=TEXT(46000;\"ddd\")", "Tue"),
        ("=TEXT(46000;\"mmm\")", "Dec"),
        ("=TEXT(0.75;\"h:mm AM/PM\")", "6:00 PM"),
        ("=TEXT(1234.5;\"$#,##0.00\")", "$1,234.50"),
        ("=TEXT(-5;\"0;(0)\")", "(5)"),
        ("=TEXT(-1234.5;\"#,##0.00\")", "-1,234.50"),
        ("=TEXT(3.14159;\"0.000\")", "3.142"),
        ("=TEXT(7;\"00\")", "07"),
        ("=TEXT(7;\"000-00\")", "000-07"),
        ("=TEXT([.B1];\"0.0\")", "10.0"),
        ("=TEXT(1;\"[Red]0\")", "1"),
        ("=TEXT(TRUE();\"0\")", "1"),
        ("=TEXT(1234.5;\"General\")", "1234.5"),
        ("=TEXT(1234.5;\"\")", ""),
        ("=TEXT(\"abc\";\"@\")", "abc"),
        ("=TEXT(\"abc\";\"0.00\")", "abc"),
        ("=TEXT(5;\"@\")", "5"),
    ] {
        assert_eq!(text(formula), want, "{formula}");
    }
    // A fraction, an exponent or a display factor has no part to spell it with: refused, where
    // the oracle shows 1/2, 1.23E+04 and 1.5.
    for refused in [
        "=TEXT(0.5;\"# ?/?\")",
        "=TEXT(12345;\"0.00E+00\")",
        "=TEXT(1500;\"0.0,\")",
    ] {
        assert_eq!(error(refused), FormulaError::Value, "{refused}");
    }
    // The document's locale spells the separators.
    let mut document = table();
    document.locale = Some(grind_core::locale::Locale::new("de", "DE"));
    let at = Address::new(0, Pos::new(20, 0));
    assert_eq!(
        Engine::new(&document).eval("=TEXT(1234.567;\"#,##0.00\")", at),
        Value::Text("1.234,57".into())
    );
}

/// §6.18's everyday statistics. `B1:B7` is 10, 20, 30, 40, `x`, 5, 30. Measured against
/// LibreOffice 26.8.
#[test]
fn percentiles_modes_regression_and_means() {
    for (formula, want) in [
        ("=PERCENTILE([.B1:.B4];0.25)", 17.5),
        ("=PERCENTILE([.B1:.B7];0.9)", 35.0),
        ("=PERCENTILE([.B1:.B4];0)", 10.0),
        ("=PERCENTILE([.B1:.B4];1)", 40.0),
        ("=QUARTILE([.B1:.B7];1)", 12.5),
        ("=QUARTILE([.B1:.B7];3)", 30.0),
        ("=QUARTILE([.B1:.B7];0)", 5.0),
        ("=QUARTILE([.B1:.B7];4)", 40.0),
        ("=QUARTILE([.B1:.B7];2.7)", 25.0),
        ("=MODE([.B1:.B7])", 30.0),
        // A tie goes to the smallest (§6.18.50).
        ("=MODE(3;1;1;3)", 1.0),
        ("=CORREL([.B1:.B4];[.B2:.B5])", 1.0),
        ("=PEARSON([.B1:.B3];[.B4:.B6])", -1.0),
        ("=RSQ([.B1:.B3];[.B4:.B6])", 1.0),
        ("=SUMSQ([.B1:.B4];2)", 3004.0),
        ("=SUMSQ([.B1:.B7])", 3925.0),
        ("=DEVSQ([.B1:.B7])", 887.5),
        ("=HARMEAN([.B1:.B4])", 19.2),
        ("=PERCENTRANK([.B1:.B4];25)", 0.5),
        ("=PERCENTRANK([.B1:.B4];20)", 0.333),
        ("=PERCENTRANK([.B1:.B4];30)", 0.667),
        ("=PERCENTRANK([.B1:.B4];36)", 0.867),
        ("=PERCENTRANK([.B1:.B4];36;1)", 0.9),
        ("=PERCENTRANK([.B1:.B4];36;5)", 0.866_67),
        ("=PERCENTRANK([.B1:.B7];30)", 0.6),
        ("=PERCENTRANK([.B1];10)", 1.0),
        ("=COMBIN(5;2)", 10.0),
        ("=COMBIN(5;0)", 1.0),
        ("=COMBIN(10.7;3.9)", 120.0),
        ("=PERMUT(5;2)", 20.0),
        ("=PERMUT(5;0)", 1.0),
    ] {
        close(formula, want);
    }
    close("=SLOPE([.B1:.B4];[.B4:.B7])", -0.384_615_384_615_385);
    close("=INTERCEPT([.B1:.B4];[.B4:.B7])", 36.282_051_282_051_3);
    close("=FORECAST(5;[.B1:.B4];[.B4:.B7])", 34.358_974_358_974_4);
    close("=AVEDEV([.B1:.B7])", 10.833_333_333_333_3);
    close("=GEOMEAN([.B1:.B4])", 22.133_638_394_006_4);
    for (formula, want) in [
        ("=PERCENTILE([.B1:.B4];1.5)", FormulaError::Num),
        ("=QUARTILE([.B1:.B7];5)", FormulaError::Num),
        ("=MODE(1;2;3)", FormulaError::Value),
        ("=CORREL([.B1:.B2];[.B3])", FormulaError::NA),
        ("=SLOPE([.B1:.B3];[.B4:.B4])", FormulaError::NA),
        ("=SLOPE([.A1:.A3];[.C1:.C3])", FormulaError::Value),
        ("=SLOPE([.B1];[.B2])", FormulaError::DivZero),
        ("=GEOMEAN(1;-1)", FormulaError::Num),
        ("=HARMEAN(0;1)", FormulaError::Num),
        ("=PERCENTRANK([.B1:.B4];5)", FormulaError::Value),
        ("=PERCENTRANK([.B1:.B4];36;0)", FormulaError::Num),
        ("=COMBIN(2;5)", FormulaError::Num),
        ("=COMBIN(-1;0)", FormulaError::Num),
        ("=PERMUT(2;5)", FormulaError::Num),
    ] {
        assert_eq!(error(formula), want, "{formula}");
    }
}
