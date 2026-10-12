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
    assert_eq!(number("=ROUNDDOWN(2^52+1)"), 4503599627370497.0);
    assert_eq!(number("=ROUND(2^52+1;0)"), 4503599627370497.0);
    assert_eq!(
        number("=ROUNDDOWN(123456789012345+0.67)"),
        123456789012345.0
    );
    assert_eq!(number("=ROUNDDOWN(99999999999999+0.96)"), 99999999999999.0);
    assert_eq!(number("=ROUNDDOWN(10^11*(1-4E-16))"), 100000000000.0);
    assert_eq!(number("=ROUNDDOWN(10^14*(1-4E-16))"), 99999999999999.0);
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
