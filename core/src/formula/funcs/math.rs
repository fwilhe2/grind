// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.16 Mathematical Functions and §6.17 Rounding Functions — the Small Group's 24 and 3.
//!
//! Domain errors mostly need no code: `SQRT(-1)`, `LN(0)` and `POWER(-1;0.5)` all produce a
//! non-finite double, and [`Value::number`] turns every one of those into `#NUM!` at the
//! single place results are built. Only the cases where IEEE 754 disagrees with §6 — a
//! division that must be `#DIV/0!` rather than an infinity — are written out.

use super::super::value::{FormulaError, Value};
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        // One number in, one number out — §6.16.2 through §6.16.69.
        "ABS" => unary(args, f64::abs),
        "ACOS" => unary(args, f64::acos),
        "ASIN" => unary(args, f64::asin),
        "ATAN" => unary(args, f64::atan),
        "COS" => unary(args, f64::cos),
        "SIN" => unary(args, f64::sin),
        "TAN" => unary(args, f64::tan),
        "EXP" => unary(args, f64::exp),
        "LN" => unary(args, f64::ln),
        "LOG10" => unary(args, f64::log10),
        "SQRT" => unary(args, f64::sqrt),
        "DEGREES" => unary(args, f64::to_degrees),
        "RADIANS" => unary(args, f64::to_radians),
        // §6.17.2: rounding towards negative infinity, which is floor and not truncation.
        "INT" => unary(args, f64::floor),
        "EVEN" => unary(args, |n| step_away(n, 2.0, 0.0)),
        "ODD" => unary(args, |n| step_away(n, 2.0, 1.0)),

        "PI" => nullary(args, std::f64::consts::PI),
        "ATAN2" => atan2(args),
        "POWER" => power(args),
        "MOD" => modulo(args),
        "LOG" => log(args),
        "FACT" => fact(args),
        "SUM" => sequence(args, |ns| ns.iter().sum()),
        "PRODUCT" => sequence(args, |ns| ns.iter().product()),
        "ROUND" => scaled(args, f64::round),
        "TRUNC" => scaled(args, f64::trunc),
        _ => return None,
    })
}

/// A whole NumberSequence in, one number out — `SUM` and `PRODUCT` (§6.16.61, §6.16.47).
fn sequence(args: &mut Args, f: impl Fn(&[f64]) -> f64) -> Answer {
    Ok(Value::number(f(&args.numbers()?)))
}

fn unary(args: &mut Args, f: impl Fn(f64) -> f64) -> Answer {
    args.arity(1..=1)?;
    Ok(Value::number(f(args.number(0)?)))
}

fn nullary(args: &Args, value: f64) -> Answer {
    args.arity(0..=0)?;
    Ok(Value::Number(value))
}

/// §6.16.30 `EVEN` and §6.16.44 `ODD`: to the next integer of the given parity, away from
/// zero. `ODD(0)` is 1, and the sign of the input is kept.
fn step_away(n: f64, period: f64, offset: f64) -> f64 {
    let sign = if n < 0.0 { -1.0 } else { 1.0 };
    sign * (((n.abs() - offset) / period).ceil() * period + offset)
}

/// §6.16.10. Note the parameter order: `ATAN2(x; y)`, the opposite of the C function.
fn atan2(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let (x, y) = (args.number(0)?, args.number(1)?);
    if x == 0.0 && y == 0.0 {
        return Err(FormulaError::DivZero);
    }
    Ok(Value::number(y.atan2(x)))
}

/// §6.16.46, and the semantics of the `^` operator (§6.4.6).
fn power(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let (base, exponent) = (args.number(0)?, args.number(1)?);
    Ok(Value::number(base.powf(exponent)))
}

/// §6.16.42: the result carries the sign of the *divisor*, so `MOD(-3;2)` is 1 and not -1.
/// Rust's `%` keeps the sign of the dividend, which is the other convention.
fn modulo(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let (a, b) = (args.number(0)?, args.number(1)?);
    if b == 0.0 {
        return Err(FormulaError::DivZero);
    }
    Ok(Value::number(a - b * (a / b).floor()))
}

/// §6.16.40 `LOG(N; Base = 10)`.
fn log(args: &mut Args) -> Answer {
    args.arity(1..=2)?;
    let n = args.number(0)?;
    let base = if args.len() > 1 && !args.omitted(1) {
        args.number(1)?
    } else {
        10.0
    };
    Ok(Value::number(n.log(base)))
}

/// §6.16.32: `FACT(N)` for non-negative `N`, which is truncated to an integer first.
fn fact(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    let n = args.number(0)?.trunc();
    if n < 0.0 {
        return Err(FormulaError::Num);
    }
    // 171! overflows to infinity, which `Value::number` reports as #NUM! — the same answer
    // as a domain check, without a magic constant.
    Ok(Value::number(
        (1..=(n as u64)).fold(1.0, |a, b| a * b as f64),
    ))
}

/// §6.17.5 `ROUND` and §6.17.8 `TRUNC`, which differ only in what they do to the scaled
/// value. Both take an optional digit count, negative meaning left of the decimal point.
///
/// Rust's `f64::round` breaks ties away from zero, which is exactly what §6.17.5 asks for.
fn scaled(args: &mut Args, f: impl Fn(f64) -> f64) -> Answer {
    args.arity(1..=2)?;
    let x = args.number(0)?;
    let digits = if args.len() > 1 && !args.omitted(1) {
        args.integer(1)?
    } else {
        0
    };
    let scale = 10f64.powi(digits.clamp(-308, 308) as i32);
    Ok(Value::number(f(x * scale) / scale))
}
