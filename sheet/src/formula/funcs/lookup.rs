// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.14 Lookup Functions — the Small Group's five.
//!
//! `MATCH`, `VLOOKUP` and `HLOOKUP` share one search, described three times in §6.14 with
//! the same three modes:
//!
//! * **exact** (`MatchType` 0, `RangeLookup` FALSE) — the first equal value, unsorted data.
//! * **ascending** (1, TRUE, and the default) — the *largest* value ≤ the search key, and
//!   from a run of equal values the **last**. Not the first: `MATCH(2;{1;2;2;3};1)` is 3.
//! * **descending** (-1) — the mirror image, smallest value ≥ the key, last of a run.
//!
//! Sorted-mode searches also refuse a cross-type hit: §6.14.9 says a Text key that lands on
//! a Number is `#N/A` rather than a match, which falls out of comparing the two values'
//! kinds once the position is found. Without that, every text lookup over a numeric column
//! would "succeed" on the last row, because §6.4.9 sorts all numbers below all text.
//!
//! ponytail: linear search. §6.14.9 explicitly permits binary search on sorted data; a
//! lookup table big enough for that to matter is not what makes a spreadsheet slow yet.

use std::cmp::Ordering;

use crate::model::Pos;

use super::super::eval::{Address, Area, order};
use super::super::value::{FormulaError, Value};
use super::super::wildcard;
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        "CHOOSE" => choose(args),
        "MATCH" => match_(args),
        "VLOOKUP" => lookup(args, true),
        "HLOOKUP" => lookup(args, false),
        "INDEX" => index(args),
        _ => return None,
    })
}

/// How a search key is matched against a sorted or unsorted vector.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Exact,
    Ascending,
    Descending,
}

/// The one search behind `MATCH`, `VLOOKUP` and `HLOOKUP`. Returns a 0-based position.
fn search(key: &Value, values: &[Value], mode: Mode) -> Result<usize, FormulaError> {
    let mut found = None;
    for (i, value) in values.iter().enumerate() {
        // An error in the data is not a candidate; propagating it would make one broken
        // cell hide every row below it.
        if value.error().is_some() || *value == Value::Empty {
            continue;
        }
        let Some(ordering) = order(value, key) else {
            continue;
        };
        let candidate = match mode {
            // §6.14.9 and §6.14.12 both defer to HOST-USE-WILDCARDS, and only the unsorted
            // search can honour it — a pattern has no place in an ordering (see `wildcard`).
            Mode::Exact => match (key, value) {
                (Value::Text(pattern), Value::Text(text)) if wildcard::is_pattern(pattern) => {
                    wildcard::matches(pattern, text)
                }
                _ => ordering == Ordering::Equal,
            },
            Mode::Ascending => ordering != Ordering::Greater,
            Mode::Descending => ordering != Ordering::Less,
        };
        if candidate {
            found = Some(i);
            // Exact mode takes the first; the sorted modes take the last of a run.
            if mode == Mode::Exact {
                break;
            }
        }
    }
    let found = found.ok_or(FormulaError::NA)?;
    // §6.14.9: a sorted search that lands on a value of another type is #N/A, not a match.
    if mode != Mode::Exact && order(&values[found], key) != Some(Ordering::Equal) {
        let same_kind = matches!(
            (&values[found], key),
            (Value::Number(_), Value::Number(_))
                | (Value::Text(_), Value::Text(_))
                | (Value::Bool(_), Value::Bool(_))
        );
        if !same_kind {
            return Err(FormulaError::NA);
        }
    }
    Ok(found)
}

/// §6.14.3: an index into the *unevaluated* parameters, so only the chosen one is computed.
fn choose(args: &mut Args) -> Answer {
    args.arity(2..)?;
    let index = args.integer(0)?;
    if index < 1 || index as usize >= args.len() {
        return Err(FormulaError::Value);
    }
    let operand = args.operand(index as usize);
    Ok(args.scalar(operand))
}

/// §6.14.9. `SearchRegion` shall be a vector — a single row or a single column.
fn match_(args: &mut Args) -> Answer {
    args.arity(2..=3)?;
    let key = args.value(0);
    let area = args.area(1).ok_or(FormulaError::Value)?;
    if area.rows.len() > 1 && area.cols.len() > 1 {
        return Err(FormulaError::Value);
    }
    let mode = match if args.len() > 2 && !args.omitted(2) {
        args.integer(2)?
    } else {
        1
    } {
        0 => Mode::Exact,
        1 => Mode::Ascending,
        -1 => Mode::Descending,
        _ => return Err(FormulaError::Value),
    };
    let values = args.values_in(&area);
    Ok(Value::number(search(&key, &values, mode)? as f64 + 1.0))
}

/// §6.14.12 `VLOOKUP` and §6.14.5 `HLOOKUP`, which are the same function transposed:
/// one searches the first column and returns from a numbered column, the other searches
/// the first row and returns from a numbered row.
fn lookup(args: &mut Args, vertical: bool) -> Answer {
    args.arity(3..=4)?;
    let key = args.value(0);
    let source = args.area(1).ok_or(FormulaError::Value)?;
    let offset = args.integer(2)?;
    if offset < 1 {
        return Err(FormulaError::Value);
    }
    // The default is the sorted search, which is the surprising half of both functions.
    let sorted = args.omitted(3) || args.logical(3)?;
    let mode = if sorted { Mode::Ascending } else { Mode::Exact };

    let first = Area {
        sheet: source.sheet,
        rows: if vertical {
            source.rows.clone()
        } else {
            source.rows.start..source.rows.start + 1
        },
        cols: if vertical {
            source.cols.start..source.cols.start + 1
        } else {
            source.cols.clone()
        },
    };
    let values = args.values_in(&first);
    let hit = search(&key, &values, mode)? as u32;

    let offset = offset as u32 - 1;
    let (row, col) = if vertical {
        (source.rows.start + hit, source.cols.start + offset)
    } else {
        (source.rows.start + offset, source.cols.start + hit)
    };
    if !source.rows.contains(&row) || !source.cols.contains(&col) {
        return Err(FormulaError::Ref);
    }
    Ok(args.value_at(Address::new(source.sheet, Pos::new(row, col))))
}

/// §6.14.6 `INDEX(DataSource; Row; Column; AreaNumber)`, 1-based and relative to the area.
///
/// ponytail: a 0 or omitted index means "the whole row/column" (§6.14.6), which is a
/// *reference* result — and a function here can only return a value. So the whole-axis
/// forms work exactly when they name one cell anyway, which is the common
/// `INDEX([.A1:.A9];3)` over a single column, and are an error otherwise. Returning
/// references from functions is what `OFFSET` and `INDIRECT` will need too; build it once,
/// for all three.
fn index(args: &mut Args) -> Answer {
    args.arity(1..=4)?;
    let source = args.area(0).ok_or(FormulaError::Value)?;
    let row = if args.omitted(1) { 0 } else { args.integer(1)? };
    let col = if args.omitted(2) { 0 } else { args.integer(2)? };
    if row < 0 || col < 0 {
        return Err(FormulaError::Value);
    }
    // We hold no reference lists, so the only area a DataSource has is its first.
    if !args.omitted(3) && args.integer(3)? != 1 {
        return Err(FormulaError::Value);
    }
    let axis = |index: i64, range: std::ops::Range<u32>| -> Result<u32, FormulaError> {
        match index {
            0 if range.len() == 1 => Ok(range.start),
            0 => Err(FormulaError::Value),
            i if (i as u32) <= range.len() as u32 => Ok(range.start + i as u32 - 1),
            _ => Err(FormulaError::Ref),
        }
    };
    let pos = Pos::new(axis(row, source.rows)?, axis(col, source.cols)?);
    Ok(args.value_at(Address::new(source.sheet, pos)))
}

#[cfg(test)]
mod tests {
    use super::super::super::eval::{Address, Engine};
    use super::super::super::value::{FormulaError, Value};
    use crate::model::{CellValue, Document, Pos, Sheet};

    /// A lookup table: A1:B5 is 1/"one", 2/"two", 2/"TWO", 5/"five", "apple"/"fruit".
    fn document() -> Document {
        let mut sheet = Sheet::new("Sheet1");
        for (row, key, label) in [
            (0, CellValue::Number(1.0), "one"),
            (1, CellValue::Number(2.0), "two"),
            (2, CellValue::Number(2.0), "TWO"),
            (3, CellValue::Number(5.0), "five"),
            (4, CellValue::Text("apple".into()), "fruit"),
        ] {
            sheet.set(Pos::new(row, 0), key);
            sheet.set(Pos::new(row, 1), CellValue::Text(label.into()));
        }
        Document {
            sheets: vec![sheet],
            ..Default::default()
        }
    }

    fn eval(formula: &str) -> Value {
        let document = document();
        Engine::new(&document).eval(formula, Address::new(0, Pos::new(20, 20)))
    }

    #[test]
    fn a_sorted_search_takes_the_last_of_a_run_and_the_largest_below() {
        // §6.14.9's rule, and the one everybody remembers backwards.
        assert_eq!(eval("=MATCH(2;[.A1:.A4];1)"), Value::Number(3.0));
        assert_eq!(eval("=MATCH(4;[.A1:.A4];1)"), Value::Number(3.0)); // largest ≤ 4
        assert_eq!(
            eval("=MATCH(0;[.A1:.A4];1)"),
            Value::Error(FormulaError::NA)
        );
        assert_eq!(eval("=MATCH(2;[.A1:.A4];0)"), Value::Number(2.0)); // exact: the first
    }

    #[test]
    fn a_sorted_search_across_types_is_na_rather_than_the_last_row() {
        // Without §6.14.9's type check this finds row 4, since every number sorts below
        // every text (§6.4.9) — a confidently wrong answer.
        assert_eq!(
            eval("=MATCH(\"zebra\";[.A1:.A4];1)"),
            Value::Error(FormulaError::NA)
        );
    }

    #[test]
    fn vlookup_defaults_to_the_sorted_search() {
        assert_eq!(eval("=VLOOKUP(2;[.A1:.B4];2)"), Value::Text("TWO".into()));
        assert_eq!(
            eval("=VLOOKUP(2;[.A1:.B4];2;FALSE())"),
            Value::Text("two".into())
        );
        assert_eq!(
            eval("=VLOOKUP(3;[.A1:.B4];2;FALSE())"),
            Value::Error(FormulaError::NA)
        );
        assert_eq!(
            eval("=VLOOKUP(2;[.A1:.B4];3)"),
            Value::Error(FormulaError::Ref)
        );
    }

    #[test]
    fn hlookup_is_vlookup_transposed() {
        // Read across instead of down: row 1 of A1:B2 is 1 and "one", so searching it for
        // 1 hits column A, and row 2 of that column is the number 2.
        assert_eq!(eval("=HLOOKUP(1;[.A1:.B2];2;FALSE())"), Value::Number(2.0));
        assert_eq!(
            eval("=HLOOKUP(\"one\";[.A1:.B2];2;FALSE())"),
            Value::Text("two".into())
        );
    }

    #[test]
    fn index_and_choose_pick_by_position() {
        assert_eq!(eval("=INDEX([.A1:.B4];2;2)"), Value::Text("two".into()));
        assert_eq!(eval("=INDEX([.B1:.B4];3)"), Value::Text("TWO".into())); // one column
        assert_eq!(
            eval("=INDEX([.A1:.B4];9;1)"),
            Value::Error(FormulaError::Ref)
        );
        assert_eq!(
            eval("=CHOOSE(2;\"a\";\"b\";\"c\")"),
            Value::Text("b".into())
        );
        // §6.14.3: only the chosen parameter is evaluated, so the division never happens.
        assert_eq!(eval("=CHOOSE(1;\"a\";1/0)"), Value::Text("a".into()));
        assert_eq!(eval("=CHOOSE(4;\"a\")"), Value::Error(FormulaError::Value));
    }

    #[test]
    fn an_unsorted_search_and_a_criterion_both_read_text_as_a_pattern() {
        // §3.4 item 6, and only where the host properties reach: the exact searches and
        // `=`/`<>`. B5 is "fruit", A5 is "apple".
        assert_eq!(eval("=MATCH(\"ap*\";[.A1:.A5];0)"), Value::Number(5.0));
        assert_eq!(eval("=MATCH(\"?pple\";[.A1:.A5];0)"), Value::Number(5.0));
        assert_eq!(
            eval("=MATCH(\"pple\";[.A1:.A5];0)"),
            Value::Error(FormulaError::NA) // anchored: no implicit leading `*`
        );
        assert_eq!(
            eval("=VLOOKUP(\"a*\";[.A1:.B5];2;FALSE())"),
            Value::Text("fruit".into())
        );
        assert_eq!(eval("=COUNTIF([.B1:.B5];\"t*\")"), Value::Number(2.0));
        assert_eq!(eval("=COUNTIF([.B1:.B5];\"<>t*\")"), Value::Number(3.0));
        assert_eq!(eval("=COUNTIF([.B1:.B5];\"~*\")"), Value::Number(0.0));
    }

    #[test]
    fn the_conditional_functions_count_sum_and_average_what_matches() {
        assert_eq!(eval("=COUNTIF([.A1:.A5];2)"), Value::Number(2.0));
        assert_eq!(eval("=COUNTIF([.A1:.A5];\">1\")"), Value::Number(3.0));
        assert_eq!(eval("=SUMIF([.A1:.A5];\">1\")"), Value::Number(9.0));
        // §6.16.62: the third range is taken by geometry from its top-left corner.
        assert_eq!(eval("=SUMIF([.A1:.A3];2;[.A2])"), Value::Number(7.0));
        assert_eq!(eval("=AVERAGEIF([.A1:.A5];\">1\")"), Value::Number(3.0));
    }
}
