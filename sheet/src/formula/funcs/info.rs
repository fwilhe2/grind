// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.13 Information Functions. The `IS*` family is the one place an error must not propagate:
//! these read the value rather than converting it.

use super::super::eval::Operand;
use super::super::value::{FormulaError, Value};
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        "NA" => na(args),
        "COUNT" => count(args),
        "COUNTIF" => super::criterion::conditional(args, super::criterion::Mode::Count),
        "COUNTIFS" => super::criterion::several(args, super::criterion::Mode::Count),
        "COUNTA" => count_a(args),
        "COUNTBLANK" => count_blank(args),
        "ROWS" => shape(args, true),
        "COLUMNS" => shape(args, false),
        "ROW" => where_am_i(args, true),
        "COLUMN" => where_am_i(args, false),
        "N" => n(args),
        "VALUE" => value_of(args),
        "ISBLANK" => is(args, |v| matches!(v, Value::Empty)),
        "ISNUMBER" => is(args, |v| matches!(v, Value::Number(_))),
        "ISTEXT" => is(args, |v| matches!(v, Value::Text(_))),
        "ISNONTEXT" => is(args, |v| !matches!(v, Value::Text(_))),
        "ISLOGICAL" => is(args, |v| matches!(v, Value::Bool(_))),
        "ISERROR" => is(args, |v| v.error().is_some()),
        // §6.13.15: every error *except* #N/A, which is the one users enter on purpose.
        "ISERR" => is(args, |v| v.error().is_some_and(|e| e != FormulaError::NA)),
        "ISNA" => is(args, |v| v.error() == Some(FormulaError::NA)),
        "ISEVEN" => parity(args, 0),
        "ISODD" => parity(args, 1),
        "ISREF" => is_ref(args),
        "ISFORMULA" => is_formula(args),
        "FORMULA" => formula(args),
        "ERROR.TYPE" => error_type(args),
        _ => return None,
    })
}

/// §6.13.6: how many numbers. "This function does not propagate Error values", so it cannot use
/// [`Args::numbers`]: `COUNT(2;4;6;"eight")` is 3, and an error in a range is ignored.
fn count(args: &mut Args) -> Answer {
    let mut count = 0usize;
    for i in 0..args.len() {
        match args.operand(i) {
            Operand::Area(area) => {
                count += args
                    .values_in(&area)
                    .iter()
                    .filter(|v| matches!(v, Value::Number(_)))
                    .count();
            }
            // A scalar counts if it converts (§6.3.7 makes a lone value a sequence of one).
            Operand::Value(v) => count += usize::from(v.to_number().is_ok()),
        }
    }
    Ok(Value::number(count as f64))
}

/// §6.13.27: the whole point of the function is to produce an error.
fn na(args: &Args) -> Answer {
    args.arity(0..=0)?;
    Ok(Value::Error(FormulaError::NA))
}

fn is(args: &mut Args, test: impl Fn(&Value) -> bool) -> Answer {
    args.arity(1..=1)?;
    let value = args.value(0);
    Ok(Value::Bool(test(&value)))
}

/// §6.13.7: everything that is not an empty cell, errors included.
fn count_a(args: &mut Args) -> Answer {
    Ok(Value::number(
        args.values().iter().filter(|v| **v != Value::Empty).count() as f64,
    ))
}

/// §6.13.8. Unlike `COUNTA` this one is about *cells*, so a scalar argument has nothing to
/// count and says so.
fn count_blank(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    let area = args.area(0).ok_or(FormulaError::Value)?;
    let values = args.values_in(&area);
    Ok(Value::number(
        values.iter().filter(|v| **v == Value::Empty).count() as f64,
    ))
}

/// §6.13.30 `ROWS` and §6.13.5 `COLUMNS`: the shape of a reference, not its contents.
fn shape(args: &mut Args, rows: bool) -> Answer {
    args.arity(1..=1)?;
    let area = args.area(0).ok_or(FormulaError::Value)?;
    let size = if rows {
        area.rows.len()
    } else {
        area.cols.len()
    };
    Ok(Value::number(size as f64))
}

/// §6.13.29 `ROW` and §6.13.4 `COLUMN`: the cell the formula is in, or a reference's top-left
/// corner, 1-based. The array form for a multi-row reference is out with the rest of inline arrays
/// (§2.3.2). Beyond the Small Group because `fizzbuzz.fods`, an R7 document, is
/// `IF(MOD(ROW();15)=0;…)`.
fn where_am_i(args: &mut Args, rows: bool) -> Answer {
    args.arity(0..=1)?;
    let (row, col) = if args.is_empty() {
        (args.at.pos.row, args.at.pos.col)
    } else {
        let area = args.area(0).ok_or(FormulaError::Value)?;
        (area.rows.start, area.cols.start)
    };
    Ok(Value::number(f64::from(if rows { row } else { col }) + 1.0))
}

/// §6.13.26: a number stays itself, a logical becomes 1 or 0, and everything else — text
/// included, however numeric it looks — becomes 0. An error still propagates.
fn n(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    Ok(match args.value(0) {
        Value::Number(n) => Value::Number(n),
        Value::Bool(b) => Value::Number(if b { 1.0 } else { 0.0 }),
        Value::Error(e) => return Err(e),
        Value::Text(_) | Value::Empty => Value::Number(0.0),
    })
}

/// §6.13.34: text to number, the explicit form of §6.3.5's implicit conversion.
fn value_of(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    Ok(Value::number(args.number(0)?))
}

/// §6.13.17 `ISEVEN` and §6.13.23 `ISODD`: `X` truncated, then its remainder by two. A logical is a
/// number here, and an empty cell 0 (§6.3.5).
fn parity(args: &mut Args, remainder: i64) -> Answer {
    args.arity(1..=1)?;
    let x = args.number(0)?.trunc() as i64;
    Ok(Value::Bool(x.rem_euclid(2) == remainder))
}

/// §6.13.22 `ISREF`: whether the argument is a reference — never an error, even of one that failed.
fn is_ref(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    Ok(Value::Bool(matches!(args.operand(0), Operand::Area(_))))
}

/// The first cell of a reference argument, if it is one.
fn first_cell(args: &mut Args) -> Option<super::super::eval::Address> {
    match args.operand(0) {
        Operand::Area(area) => Some(super::super::eval::Address::new(
            area.sheet,
            crate::model::Pos::new(area.rows.start, area.cols.start),
        )),
        Operand::Value(_) => None,
    }
}

/// §6.13.18 `ISFORMULA`: whether the referenced cell holds a formula.
fn is_formula(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    let at = first_cell(args);
    Ok(Value::Bool(
        at.is_some_and(|at| args.formula_at(at).is_some()),
    ))
}

/// §6.13.12 `FORMULA`: the referenced cell's formula as the formula bar shows it; `#N/A` for a
/// cell without one, as the oracle answers.
fn formula(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    let at = first_cell(args).ok_or(FormulaError::Value)?;
    let stored = args.formula_at(at).ok_or(FormulaError::NA)?.to_owned();
    let shown = super::super::display::to_display(&stored).unwrap_or(stored);
    Ok(Value::Text(if shown.starts_with('=') {
        shown
    } else {
        format!("={shown}")
    }))
}

/// §6.13.11 `ERROR.TYPE`: an error's number, 1 `#NULL!` to 7 `#N/A`; anything else is `#N/A`.
fn error_type(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    Ok(Value::number(match args.value(0) {
        Value::Error(FormulaError::Null) => 1.0,
        Value::Error(FormulaError::DivZero) => 2.0,
        Value::Error(FormulaError::Value) => 3.0,
        Value::Error(FormulaError::Ref) => 4.0,
        Value::Error(FormulaError::Name) => 5.0,
        Value::Error(FormulaError::Num) => 6.0,
        Value::Error(FormulaError::NA) => 7.0,
        _ => return Err(FormulaError::NA),
    }))
}
