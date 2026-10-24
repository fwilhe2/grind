// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.15 Logical Functions — all six of the Small Group's, `IFERROR` and `IFNA` from beyond
//! it, and §6.11.3 `HYPERLINK`, which is a logical function in all but its chapter: it
//! chooses which of its arguments is the value.

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
        "IFERROR" => if_error(args, |_| true),
        "IFNA" => if_error(args, |e| e == FormulaError::NA),
        "HYPERLINK" => hyperlink(args),
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

/// §6.15.2 `AND` and §6.15.8 `OR`. Not short-circuiting: an error anywhere propagates (§4.6), so
/// `AND(FALSE();1/0)` is `#DIV/0!`.
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

/// §6.15.4, all seven of its parameter shapes. The branch not taken is never evaluated, which
/// makes `IF([.A1]=0;0;1/[.A1])` a working guard.
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

/// §6.15.5 `IFERROR` and §6.15.6 `IFNA`, which differ only in which errors they catch. X is
/// computed once, and the alternative only when taken.
fn if_error(args: &mut Args, catches: impl Fn(FormulaError) -> bool) -> Answer {
    args.arity(2..=2)?;
    if args.omitted(0) {
        return Err(FormulaError::Value);
    }
    match args.value(0) {
        // An empty alternative is 0, as an empty branch of `IF` is.
        Value::Error(e) if catches(e) && args.omitted(1) => Ok(Value::Number(0.0)),
        Value::Error(e) if catches(e) => Ok(args.value(1)),
        value => Ok(value),
    }
}

/// §6.11.3 `HYPERLINK( IRI [ ; FunctionResult ] )`: the value is the second argument, else
/// the first. Making a link of it is the host's business, and this host draws none.
fn hyperlink(args: &mut Args) -> Answer {
    args.arity(1..=2)?;
    let iri = args.text(0)?;
    if args.len() == 1 || args.omitted(1) {
        return Ok(Value::Text(iri));
    }
    Ok(args.value(1))
}
