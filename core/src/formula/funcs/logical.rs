// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.15 Logical Functions — all six of the Small Group's.

use super::super::eval::Operand;
use super::super::value::{FormulaError, Value};
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        "TRUE" => constant(args, true),
        "FALSE" => constant(args, false),
        "NOT" => not(args),
        "AND" => fold(args, true),
        "OR" => fold(args, false),
        "IF" => if_(args),
        _ => return None,
    })
}

/// §6.15.9 / §6.15.3: exactly zero parameters.
fn constant(args: &Args, value: bool) -> Answer {
    args.arity(0..=0)?;
    Ok(Value::Bool(value))
}

/// §6.15.7.
fn not(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    Ok(Value::Bool(!args.logical(0)?))
}

/// §6.15.2 `AND` and §6.15.8 `OR`, which differ only in which answer ends the fold.
///
/// Not short-circuiting: an error anywhere in the parameters propagates (§4.6), so
/// `AND(FALSE();1/0)` is `#DIV/0!` rather than `FALSE`.
fn fold(args: &mut Args, all: bool) -> Answer {
    args.arity(1..)?;
    let mut result = all;
    let mut seen = false;
    for value in args.logicals()? {
        seen = true;
        result = if all {
            result && value
        } else {
            result || value
        };
    }
    // Parameters that contribute nothing — empty cells, text — can leave the fold with no
    // input at all, which is not an answer about anything.
    if seen {
        Ok(Value::Bool(result))
    } else {
        Err(FormulaError::Value)
    }
}

/// §6.15.4, including all seven of its parameter shapes.
///
/// Short-circuits by construction: the branch not taken is never evaluated, which is what
/// makes `IF([.A1]=0;0;1/[.A1])` a working guard rather than a division by zero.
fn if_(args: &mut Args) -> Answer {
    args.arity(1..=3)?;
    let condition = args.logical(0)?;
    // "If there is only 1 parameter, IfTrue is TRUE and IfFalse is FALSE."
    if args.len() == 1 {
        return Ok(Value::Bool(condition));
    }
    let branch = if condition { 1 } else { 2 };
    // "If there are 2 parameters, IfFalse is FALSE"; an *omitted* branch — `;;` — is 0.
    if branch >= args.len() {
        return Ok(Value::Bool(false));
    }
    if args.omitted(branch) {
        return Ok(Value::Number(0.0));
    }
    // A branch may be a reference, and `IF(…;[.A1:.B2])` keeps it one for the caller.
    Ok(match args.operand(branch) {
        Operand::Value(v) => v,
        area => args.scalar(area),
    })
}
