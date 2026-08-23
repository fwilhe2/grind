// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The value model, the error set, and Part 4's implicit conversion operators.
//!
//! Everything downstream inherits its correctness from this file (doc/plan.md, phase 4),
//! so every rule below cites the section of ODF 1.4 Part 4 it comes from, and the three
//! places the spec says "implementation-defined" are named as choices rather than left as
//! accidents:
//!
//! | Left open by | Choice | Why |
//! |---|---|---|
//! | §6.3.5 Text → Number | strict parse, else `#VALUE!` | 0 turns a typo into a plausible wrong answer |
//! | §6.3.12 Text → Logical | `TRUE`/`FALSE` only, else `#VALUE!` | same, and locale-free |
//! | §6.3.14 Number → Text | 15 significant digits, `%G`-style exponent switch | 15 is all LO writes (doc/ods-format.md §3.4) |
//!
//! Not here on purpose: comparison and collation belong with the operators (§6.4), and
//! integer conversion (§6.3.6) is defined *per function* — `INT`, `ROUND` and `TRUNC` all
//! differ — so it lives with the functions that name a rounding mode.

use std::fmt;

/// An error value (§4.6). The seven names of §5.12 Table 4.
///
/// `#N/A` is the only one an evaluator is required to support; the rest are what every
/// existing implementation uses, so they are the set we read, write and produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormulaError {
    /// `#NULL!` — an intersection of ranges produced zero cells.
    Null,
    /// `#DIV/0!` — division by zero, including by an empty cell.
    DivZero,
    /// `#VALUE!` — a parameter is of the wrong type.
    Value,
    /// `#REF!` — a reference to a cell that cannot exist.
    Ref,
    /// `#NAME?` — an unrecognised or deleted name. Also where unknown error names land
    /// when read (§5.12).
    Name,
    /// `#NUM!` — domain constraints unmet; input too large or too small.
    Num,
    /// `#N/A` — not available. `NA()` and every failed lookup.
    NA,
}

impl FormulaError {
    /// The name as it is written into a formula and displayed (§5.12).
    pub fn name(self) -> &'static str {
        match self {
            FormulaError::Null => "#NULL!",
            FormulaError::DivZero => "#DIV/0!",
            FormulaError::Value => "#VALUE!",
            FormulaError::Ref => "#REF!",
            FormulaError::Name => "#NAME?",
            FormulaError::Num => "#NUM!",
            FormulaError::NA => "#N/A",
        }
    }

    /// The inverse of [`FormulaError::name`], for reading.
    ///
    /// An error name we do not know is still an error, and §5.12 says to map it onto one we
    /// support rather than to reject the document — so anything error-shaped that is not in
    /// the table becomes `#NAME?`. `None` means "not an error name at all".
    pub fn from_name(s: &str) -> Option<Self> {
        const KNOWN: [FormulaError; 7] = [
            FormulaError::Null,
            FormulaError::DivZero,
            FormulaError::Value,
            FormulaError::Ref,
            FormulaError::Name,
            FormulaError::Num,
            FormulaError::NA,
        ];
        if !s.starts_with('#') {
            return None;
        }
        Some(
            KNOWN
                .into_iter()
                .find(|e| e.name().eq_ignore_ascii_case(s))
                .unwrap_or(FormulaError::Name),
        )
    }
}

impl fmt::Display for FormulaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A value an expression evaluates to.
///
/// [`Empty`](Value::Empty) is the value of an empty cell, and §4.7 is emphatic that it is
/// neither zero nor the empty string nor an error — it only *converts* to 0, `""` and
/// `FALSE`. Keeping it a distinct variant is what lets `COUNTBLANK`, `ISBLANK` and the
/// sequence conversions below tell it apart from a cell someone typed a 0 into.
///
/// References are deliberately absent: by the time a value exists, a reference has been
/// resolved to a cell's value or to a sequence of them. That resolution needs the document,
/// which this file does not have and does not want.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(FormulaError),
}

impl Value {
    /// A number, or `#NUM!` if it is not one.
    ///
    /// The one funnel for arithmetic results. Every operator and function returns through
    /// here, because an infinity or a NaN that escapes into the grid propagates silently
    /// and is then indistinguishable from a real answer — where `#NUM!` says which cell
    /// went wrong. §4.3.1 admits only finite numbers; the error set is how a computation
    /// leaves the number line.
    pub fn number(n: f64) -> Self {
        if n.is_finite() {
            Value::Number(n)
        } else {
            Value::Error(FormulaError::Num)
        }
    }

    pub fn error(&self) -> Option<FormulaError> {
        match self {
            Value::Error(e) => Some(*e),
            _ => None,
        }
    }

    /// §6.3.5 Conversion to Number.
    ///
    /// Empty is 0 — the spec says so for the reference case, and an empty cell is the only
    /// way an `Empty` reaches a conversion at all.
    pub fn to_number(&self) -> Result<f64, FormulaError> {
        match self {
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Empty => Ok(0.0),
            // §6.3.5 leaves Text implementation-defined between 0, an Error, and a parse.
            Value::Text(s) => parse_number(s).ok_or(FormulaError::Value),
            Value::Error(e) => Err(*e),
        }
    }

    /// §6.3.12 Conversion to Logical.
    pub fn to_logical(&self) -> Result<bool, FormulaError> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Empty => Ok(false),
            // Implementation-defined again. Only the two spellings §6.3.14 writes are
            // accepted, so text→logical→text is the identity and no locale is involved.
            Value::Text(s) => {
                if s.eq_ignore_ascii_case("TRUE") {
                    Ok(true)
                } else if s.eq_ignore_ascii_case("FALSE") {
                    Ok(false)
                } else {
                    Err(FormulaError::Value)
                }
            }
            Value::Error(e) => Err(*e),
        }
    }

    /// §6.3.14 Conversion to Text.
    pub fn to_text(&self) -> Result<String, FormulaError> {
        match self {
            Value::Text(s) => Ok(s.clone()),
            Value::Number(n) => Ok(format_number(*n)),
            Value::Bool(b) => Ok(if *b { "TRUE" } else { "FALSE" }.to_owned()),
            Value::Empty => Ok(String::new()),
            Value::Error(e) => Err(*e),
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_owned())
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<FormulaError> for Value {
    fn from(e: FormulaError) -> Self {
        Value::Error(e)
    }
}

/// §6.3.7 Conversion to NumberSequence, the reference case — `SUM(A1:C9)` and every
/// function shaped like it.
///
/// The filter *is* the semantics: text that would parse as a number is skipped, empty cells
/// are skipped, and — because our `Bool` is a distinguished type (§4.5) — logicals are
/// skipped too. This is why `SUM(A1:A3)` and `SUM(A1,A2,A3)` legitimately differ when one
/// of the cells holds `"7"`: a scalar argument goes through [`Value::to_number`] instead.
///
/// The spec includes errors *in* the sequence and then has every function propagate them
/// (§4.6), which is observably the same as stopping at the first one and saves each caller
/// the check.
pub fn number_sequence<'a>(
    cells: impl IntoIterator<Item = &'a Value>,
) -> Result<Vec<f64>, FormulaError> {
    let mut out = Vec::new();
    for cell in cells {
        match cell {
            Value::Number(n) => out.push(*n),
            Value::Error(e) => return Err(*e),
            Value::Empty | Value::Text(_) | Value::Bool(_) => {}
        }
    }
    Ok(out)
}

/// Text → Number, the strict half of §6.3.5.
///
/// Deliberately locale-free: `.` is the decimal separator and there are no group
/// separators, because a formula's *text* is document content and does not change meaning
/// when the document crosses a border. (A locale belongs in the number **format** layer,
/// phase 5, where the user's own typing is parsed.)
fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim_matches(|c: char| c.is_ascii_whitespace());
    let (s, scale) = match s.strip_suffix('%') {
        Some(rest) => (rest.trim_end(), 0.01),
        None => (s, 1.0),
    };
    // Rust's parser accepts `inf`, `infinity` and `nan`; none of those is a Number (§4.3.1),
    // and letting them through would put a non-finite straight into the grid past
    // `Value::number`'s guard.
    if s.is_empty()
        || s.chars()
            .any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E')
    {
        return None;
    }
    s.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| n * scale)
}

/// Number → Text (§6.3.14: "transform into Text, with no whitespace").
///
/// Two rules, both borrowed rather than invented: 15 significant digits, because that is
/// what LO writes and what loop C therefore compares (doc/ods-format.md §3.4), and C's
/// `%G` switch to exponent form outside `[1e-5, 1e15)`, which is what a general-format
/// display does. Rounding first is the load-bearing half — `0.1 + 0.2` has to come back
/// `"0.3"`, not `"0.30000000000000004"`.
///
/// ponytail: the exponent spelling (`1E+20`) is asserted against the spec's "no whitespace"
/// and nothing else. Loop B is what will pin it; fix it there when a text function fails,
/// not by guessing now.
pub fn format_number(n: f64) -> String {
    if n == 0.0 {
        return "0".to_owned(); // also normalises -0.0
    }
    if !n.is_finite() {
        return FormulaError::Num.name().to_owned();
    }
    let exponent = n.abs().log10().floor() as i32;
    if !(-5..15).contains(&exponent) {
        let scientific = format!("{n:.14E}");
        let (mantissa, exponent) = scientific.split_once('E').unwrap_or((&scientific, "0"));
        let exponent: i32 = exponent.parse().unwrap_or(0);
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{}E{sign}{}", trim_zeros(mantissa), exponent.abs());
    }
    // Round to 15 significant digits, then let Rust print the shortest text that reads back
    // as that same double.
    let rounded: f64 = format!("{n:.14E}").parse().unwrap_or(n);
    format!("{rounded}")
}

fn trim_zeros(mantissa: &str) -> &str {
    match mantissa.trim_end_matches('0').trim_end_matches('.') {
        "" | "-" => "0",
        trimmed => trimmed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- §4.6 / §5.12, the error set ---

    #[test]
    fn error_names_round_trip_and_unknown_ones_become_name() {
        for e in [
            FormulaError::Null,
            FormulaError::DivZero,
            FormulaError::Value,
            FormulaError::Ref,
            FormulaError::Name,
            FormulaError::Num,
            FormulaError::NA,
        ] {
            assert_eq!(FormulaError::from_name(e.name()), Some(e));
        }
        // §5.12: an unsupported error name is still an error, mapped onto one we have.
        assert_eq!(
            FormulaError::from_name("#GETTING_DATA"),
            Some(FormulaError::Name)
        );
        assert_eq!(FormulaError::from_name("NOT_AN_ERROR"), None);
    }

    #[test]
    fn every_conversion_propagates_an_error_unchanged() {
        // §6.3.1: "any conversion operation applied to a value of type Error returns the
        // same value" — the rule the whole error model rests on.
        let v = Value::Error(FormulaError::DivZero);
        assert_eq!(v.to_number(), Err(FormulaError::DivZero));
        assert_eq!(v.to_logical(), Err(FormulaError::DivZero));
        assert_eq!(v.to_text(), Err(FormulaError::DivZero));
        assert_eq!(
            number_sequence([&Value::Number(1.0), &v]),
            Err(FormulaError::DivZero)
        );
    }

    #[test]
    fn a_non_finite_result_becomes_num_rather_than_reaching_the_grid() {
        assert_eq!(
            Value::number(f64::INFINITY),
            Value::Error(FormulaError::Num)
        );
        assert_eq!(Value::number(f64::NAN), Value::Error(FormulaError::Num));
        assert_eq!(Value::number(1.5), Value::Number(1.5));
    }

    // --- §6.3.5 Conversion to Number ---

    #[test]
    fn conversion_to_number_follows_the_table() {
        assert_eq!(Value::Number(1.5).to_number(), Ok(1.5));
        assert_eq!(Value::Bool(true).to_number(), Ok(1.0));
        assert_eq!(Value::Bool(false).to_number(), Ok(0.0));
        assert_eq!(Value::Empty.to_number(), Ok(0.0)); // §6.3.5, empty cell
        assert_eq!(Value::from("  -2.5e3 ").to_number(), Ok(-2500.0));
        assert_eq!(Value::from("50%").to_number(), Ok(0.5)); // §4.3.5 Percentage
        assert_eq!(Value::from("seven").to_number(), Err(FormulaError::Value));
        assert_eq!(Value::from("").to_number(), Err(FormulaError::Value));
        assert_eq!(Value::from("1,000").to_number(), Err(FormulaError::Value));
    }

    #[test]
    fn infinity_cannot_be_spelled_into_existence_as_text() {
        // Rust's f64 parser accepts all of these; §4.3.1 does not.
        for s in ["inf", "-infinity", "NaN", "nan"] {
            assert_eq!(Value::from(s).to_number(), Err(FormulaError::Value), "{s}");
        }
    }

    // --- §6.3.12 Conversion to Logical ---

    #[test]
    fn conversion_to_logical_follows_the_table() {
        assert_eq!(Value::Number(0.0).to_logical(), Ok(false));
        assert_eq!(Value::Number(-3.0).to_logical(), Ok(true)); // nonzero, not "positive"
        assert_eq!(Value::Empty.to_logical(), Ok(false));
        assert_eq!(Value::from("true").to_logical(), Ok(true));
        assert_eq!(Value::from("FALSE").to_logical(), Ok(false));
        assert_eq!(Value::from("1").to_logical(), Err(FormulaError::Value));
    }

    // --- §6.3.14 Conversion to Text ---

    #[test]
    fn conversion_to_text_follows_the_table() {
        assert_eq!(Value::Bool(true).to_text(), Ok("TRUE".into()));
        assert_eq!(Value::Bool(false).to_text(), Ok("FALSE".into()));
        assert_eq!(Value::Empty.to_text(), Ok(String::new()));
        assert_eq!(Value::from("as is").to_text(), Ok("as is".into()));
    }

    #[test]
    fn numbers_print_at_fifteen_significant_digits() {
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1.0 / 3.0), "0.333333333333333");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(42.0), "42");
        assert_eq!(format_number(1e14), "100000000000000");
    }

    #[test]
    fn very_large_and_very_small_numbers_switch_to_exponent_form() {
        assert_eq!(format_number(1e20), "1E+20");
        assert_eq!(format_number(-1.5e-7), "-1.5E-7");
        assert_eq!(format_number(1e-5), "0.00001"); // the boundary stays plain
    }

    #[test]
    fn text_never_contains_whitespace() {
        // §6.3.14 says so outright, and it is the easy one to break with a thousands group.
        for n in [1234567.0, -0.5, 1e300, f64::MIN_POSITIVE] {
            let s = format_number(n);
            assert!(!s.contains(char::is_whitespace), "{s:?}");
        }
    }

    // --- §6.3.7 Conversion to NumberSequence ---

    #[test]
    fn a_number_sequence_from_cells_keeps_only_the_numbers() {
        // The distinction that makes SUM(A1:A3) differ from SUM(A1,A2,A3): a *referenced*
        // "7" is skipped, whereas the same text passed as a scalar converts.
        let cells = [
            Value::Number(1.0),
            Value::from("7"),
            Value::Empty,
            Value::Bool(true),
            Value::Number(2.0),
        ];
        assert_eq!(number_sequence(&cells), Ok(vec![1.0, 2.0]));
        assert_eq!(Value::from("7").to_number(), Ok(7.0));
    }
}
