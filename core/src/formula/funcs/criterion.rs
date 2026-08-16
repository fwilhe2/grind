// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §4.11.8 Criterion — the little matching language `COUNTIF`, `SUMIF` and `AVERAGEIF`
//! share, and the reason those three live apart from their categories' other functions.
//!
//! A criterion is a value, and if it is text it may *begin with a comparator*: `">=3"`
//! compares, `"apple"` matches. The interesting half is what happens to empty cells, and
//! §4.11.8 is precise where intuition is not:
//!
//! | Criterion | Matches an empty cell? |
//! |---|---|
//! | `"="` (empty value) | yes — this is how you count blanks |
//! | `"<>"` (empty value) | no — it matches everything non-empty |
//! | `"=0"` | **no**, stated outright, even though an empty cell converts to 0 elsewhere |
//! | `"<>7"` | yes — "any cell content except the value, including empty cells" |
//!
//! A text criterion under `=` or `<>` is a **pattern**, not a literal — see [`wildcard`]
//! for which of §3.4's host properties that settles and why. Regular expressions remain
//! off; LibreOffice makes the two mutually exclusive and wildcards are its default.

use std::cmp::Ordering;

use crate::model::Pos;

use super::super::eval::{Address, Operand, order};
use super::super::lex::Op;
use super::super::value::{FormulaError, Value};
use super::super::wildcard;
use super::Args;

/// What the three conditional functions do with the cells that matched.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// `COUNTIF` (§6.13.9) — how many matched, whatever they hold.
    Count,
    /// `SUMIF` (§6.16.62).
    Sum,
    /// `AVERAGEIF` (§6.18.5).
    Average,
}

/// `COUNTIF`, `SUMIF` and `AVERAGEIF`, which differ only in [`Mode`].
///
/// The optional third parameter is the trap worth naming: §6.16.62 says the summed range is
/// taken *by geometry*, "starting from the top left cell and matching the geometry of R".
/// Its own extent is ignored — `SUMIF([.A1:.A9];">2";[.C1])` sums nine cells of column C,
/// not one. Offsetting from `S`'s corner rather than intersecting with it is that rule.
pub fn conditional(args: &mut Args, mode: Mode) -> Result<Value, FormulaError> {
    args.arity(if mode == Mode::Count { 2..=2 } else { 2..=3 })?;
    // "Does not accept constant values as the reference parameter" (§6.13.9 Constraints).
    let range = args.area(0).ok_or(FormulaError::Value)?;
    // §4.11.8's opening line: "A reference to an empty cell is interpreted as the numeric
    // value 0" — so a criterion *cell* that is blank counts zeros, where the literal `""`
    // counts blanks. The two look identical by the time they are values, which is why the
    // operand is inspected before it is reduced.
    let criterion = match args.operand(1) {
        Operand::Area(_) => match args.value(1) {
            Value::Empty => Criterion::new(Value::Number(0.0)),
            value => Criterion::new(value),
        },
        Operand::Value(value) => Criterion::new(value),
    };
    let values = if args.len() > 2 {
        Some(args.area(2).ok_or(FormulaError::Value)?)
    } else {
        None
    };

    let mut matched = 0usize;
    let mut counted = 0usize;
    let mut total = 0.0;
    for (r, row) in range.rows.clone().enumerate() {
        for (c, col) in range.cols.clone().enumerate() {
            let cell = args.value_at(Address::new(range.sheet, Pos::new(row, col)));
            if !criterion.matches(&cell) {
                continue;
            }
            matched += 1;
            let value = match &values {
                None => cell,
                Some(area) => args.value_at(Address::new(
                    area.sheet,
                    Pos::new(area.rows.start + r as u32, area.cols.start + c as u32),
                )),
            };
            if let Value::Number(n) = value {
                counted += 1;
                total += n;
            }
        }
    }
    Ok(match mode {
        Mode::Count => Value::number(matched as f64),
        Mode::Sum => Value::number(total),
        Mode::Average if counted == 0 => return Err(FormulaError::DivZero),
        Mode::Average => Value::number(total / counted as f64),
    })
}

pub struct Criterion {
    op: Op,
    /// [`Value::Empty`] when the criterion was a bare `"="` or `"<>"`, which is a matcher
    /// for blankness rather than a comparison against nothing.
    operand: Value,
}

impl Criterion {
    pub fn new(value: Value) -> Self {
        let Value::Text(text) = &value else {
            // A Number or Logical criterion is an equality test (§4.11.8).
            return Criterion {
                op: Op::Eq,
                operand: value,
            };
        };
        // Longest first: `<=` must win over `<`.
        for (prefix, op) in [
            ("<=", Op::Le),
            (">=", Op::Ge),
            ("<>", Op::Ne),
            ("<", Op::Lt),
            (">", Op::Gt),
            ("=", Op::Eq),
        ] {
            if let Some(rest) = text.strip_prefix(prefix) {
                return Criterion {
                    op,
                    operand: operand(rest),
                };
            }
        }
        Criterion {
            op: Op::Eq,
            operand: operand(text),
        }
    }

    pub fn matches(&self, cell: &Value) -> bool {
        // An error in the data is not a match; it is also not a reason to stop counting.
        if cell.error().is_some() {
            return false;
        }
        if self.operand == Value::Empty {
            // Blankness, not equality — and a formula that returned `""` is blank for this
            // one test. §4.11.8's `"="` is "how you count blanks", and a cell displaying
            // nothing because its formula said `=""` is one; §4.7's distinction between an
            // empty cell and an empty string is about the *cell*, and no formula result is
            // ever an empty cell (see `eval::eval`).
            let blank = matches!(cell, Value::Empty) || *cell == Value::Text(String::new());
            return match self.op {
                Op::Eq => blank,
                _ => !blank,
            };
        }
        if *cell == Value::Empty {
            // §4.11.8: "=0" does not match empty cells, and neither does any ordered
            // comparison — but "<>7" does.
            return self.op == Op::Ne;
        }
        // Only `=` and `<>` take a pattern (§3.4 item 6 says "comparisons and searching";
        // an ordered comparison against a pattern has no meaning to give).
        if let (Value::Text(pattern), Value::Text(text)) = (&self.operand, cell)
            && matches!(self.op, Op::Eq | Op::Ne)
            && wildcard::is_pattern(pattern)
        {
            let hit = wildcard::matches(pattern, text);
            return if self.op == Op::Eq { hit } else { !hit };
        }
        // A criterion compares within one type. `">1"` counts numbers above 1, not every
        // text cell — even though §6.4.9's cross-type order puts all text above all numbers,
        // which is about *sorting* and would make `COUNTIF` count words as large numbers.
        // §6.4.7's "if the values differ in type, return FALSE" is the rule that applies.
        let same_kind = matches!(
            (cell, &self.operand),
            (Value::Number(_), Value::Number(_))
                | (Value::Text(_), Value::Text(_))
                | (Value::Bool(_), Value::Bool(_))
        );
        if !same_kind {
            return self.op == Op::Ne;
        }
        holds(self.op, order(cell, &self.operand))
    }
}

/// Does this ordering satisfy the comparator?
fn holds(op: Op, ordering: Option<Ordering>) -> bool {
    match ordering {
        None => false,
        Some(ordering) => match op {
            Op::Eq => ordering == Ordering::Equal,
            Op::Ne => ordering != Ordering::Equal,
            Op::Lt => ordering == Ordering::Less,
            Op::Le => ordering != Ordering::Greater,
            Op::Gt => ordering == Ordering::Greater,
            _ => ordering != Ordering::Less,
        },
    }
}

/// The right-hand side of a criterion: a number when it reads as one, so that `">=3"`
/// compares numerically against numeric cells rather than as the text `"3"`.
fn operand(text: &str) -> Value {
    if text.is_empty() {
        return Value::Empty;
    }
    match Value::Text(text.to_owned()).to_number() {
        Ok(n) => Value::Number(n),
        Err(_) => Value::Text(text.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(criterion: &str, cell: Value) -> bool {
        Criterion::new(Value::Text(criterion.into())).matches(&cell)
    }

    #[test]
    fn a_comparator_prefix_compares_and_anything_else_equals() {
        assert!(matches(">=3", Value::Number(3.0)));
        assert!(!matches(">3", Value::Number(3.0)));
        assert!(matches("<>3", Value::Number(4.0)));
        assert!(matches("3", Value::Number(3.0))); // no prefix: equality
        assert!(matches("apple", Value::Text("APPLE".into()))); // case-insensitive (§6.4.7)
        assert!(!matches("apple", Value::Number(3.0)));
        // Text is not a large number: a numeric criterion ignores text cells entirely.
        assert!(!matches(">1", Value::Text("apple".into())));
        assert!(matches("<>1", Value::Text("apple".into())));
    }

    #[test]
    fn empty_cells_follow_the_table_in_the_module_docs() {
        assert!(matches("=", Value::Empty));
        assert!(!matches("=", Value::Number(0.0)));
        assert!(matches("<>", Value::Number(0.0)));
        assert!(!matches("<>", Value::Empty));
        assert!(!matches("=0", Value::Empty)); // §4.11.8 says so outright
        // And a bare 0 is no different, even though an empty cell converts to 0 everywhere
        // else (§6.3.5). LO agrees: `COUNTIF` over a column of blanks with criterion 0 is 0.
        assert!(!Criterion::new(Value::Number(0.0)).matches(&Value::Empty));
        assert!(!matches(">0", Value::Empty));
        assert!(matches("<>7", Value::Empty)); // ... but this one does
    }

    #[test]
    fn a_numeric_criterion_matches_numbers_not_their_spelling() {
        assert!(Criterion::new(Value::Number(3.0)).matches(&Value::Number(3.0)));
        assert!(matches("=3", Value::Number(3.0)));
        assert!(!matches("=3", Value::Text("3".into()))); // text is not the number
    }
}
