// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.13 Information Functions — the Small Group's, minus `COUNTIF` (which needs §4.11.8
//! Criterion matching, still unbuilt).
//!
//! The `IS*` family is the one place an error must **not** propagate: `ISERROR` exists to
//! look at one, so these read the value rather than converting it.

use super::super::eval::Operand;
use super::super::value::{FormulaError, Value};
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        "NA" => na(args),
        "COUNT" => count(args),
        "COUNTA" => count_a(args),
        "COUNTBLANK" => count_blank(args),
        "ROWS" => shape(args, true),
        "COLUMNS" => shape(args, false),
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
        _ => return None,
    })
}

/// §6.13.6: how many *numbers*.
///
/// The one function in this file that cannot use [`Args::numbers`], and the spec says why
/// in one sentence: "all other types are ignored. This function does not propagate Error
/// values." So `COUNT(2;4;6;"eight")` is 3 rather than `#VALUE!`, and an error sitting in
/// the middle of a range is ignored rather than returned — the opposite of §4.6's default,
/// which is exactly why it is written out here.
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
