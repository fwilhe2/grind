// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.16 Mathematical Functions and §6.17 Rounding Functions — the Small Group's 24 and 3,
//! and `QUOTIENT`, `SIGN`, `ROUNDUP`, `ROUNDDOWN` and `MROUND` from beyond it.
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
        // §6.17.2: rounding towards negative infinity, which is floor and not truncation —
        // of the value as it is shown, as every step to a whole number here is.
        "INT" => unary(args, |n| shown(n).floor()),
        "EVEN" => unary(args, |n| step_away(n, 2.0, 0.0)),
        "ODD" => unary(args, |n| step_away(n, 2.0, 1.0)),

        "PI" => nullary(args, std::f64::consts::PI),
        "ATAN2" => atan2(args),
        "POWER" => power(args),
        "MOD" => modulo(args),
        "LOG" => log(args),
        "FACT" => fact(args),
        "SUM" => sequence(args, |ns| ns.iter().sum()),
        "SUMIF" => super::criterion::conditional(args, super::criterion::Mode::Sum),
        "PRODUCT" => sequence(args, |ns| ns.iter().product()),
        "ROUND" => scaled(args, |x| shown(x).round()),
        "TRUNC" => scaled(args, |x| shown(x).trunc()),
        "ROUNDUP" => scaled(args, |x| shown(x).abs().ceil().copysign(x)),
        "ROUNDDOWN" => scaled(args, |x| shown(x).trunc()),
        "MROUND" => mround(args),
        "QUOTIENT" => quotient(args),
        "SIGN" => unary(args, |n| if n == 0.0 { 0.0 } else { n.signum() }),
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

/// A value as a spreadsheet *shows* it — rounded to fifteen significant digits, the most a
/// double reliably carries — for a function about to step it to a whole number.
///
/// `4.35*100` is `434.99999999999994` in binary and displays as `435`; truncating the binary
/// value makes `ROUNDDOWN` answer 434 to a number the user can see is 435. LibreOffice reads
/// the value to fifteen digits first, which is measured rather than specified
/// (`doc/ods-format.md` §4.1), and so does every function here that floors, ceils or
/// truncates a quotient.
///
/// Only below 10^11, which is where the measurement is consistent. Above it a double's
/// noise is in the fifth decimal and the oracle's answer depends on its own binary steps —
/// `9.5E11·(1−4E−16)` snaps going down and not going up — so nothing written in digits
/// reproduces it, and a 16-digit integer such as `2^52+1` must not become its 15-digit
/// neighbour. A value that large is stepped as it is.
pub fn shown(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() || x.abs() >= 1e11 {
        return x;
    }
    format!("{x:.14e}").parse().unwrap_or(x)
}

/// §6.17.4 `MROUND(A; B)`: the multiple of `B` nearest `A`, and on a tie "the greater one".
///
/// The quotient is read as [`shown`] before the tie is judged, so `MROUND(1.3;0.2)` — whose
/// quotient is `6.499999999999999` — is the tie it looks like. Neither sign is constrained;
/// a multiple of zero is zero, as measured.
fn mround(args: &mut Args) -> Answer {
    let (a, b) = two_required(args)?;
    if b == 0.0 {
        return Ok(Value::Number(0.0));
    }
    let q = shown(a / b);
    let below = q.floor();
    let k = match (q - below).partial_cmp(&0.5) {
        Some(std::cmp::Ordering::Less) => below,
        Some(std::cmp::Ordering::Greater) => below + 1.0,
        // A tie: of `b·below` and `b·(below + 1)`, the greater — which is the upper step for
        // a positive `b` and the lower one for a negative.
        _ if b > 0.0 => below + 1.0,
        _ => below,
    };
    Ok(Value::number(k * b))
}

/// §6.16.48 `QUOTIENT(A; B)`: the integer portion of `A/B`, towards zero.
fn quotient(args: &mut Args) -> Answer {
    let (a, b) = two_required(args)?;
    if b == 0.0 {
        return Err(FormulaError::DivZero);
    }
    Ok(Value::number(shown(a / b).trunc()))
}

/// Two numbers neither of which may be left empty: `MROUND(15.5;)` is an error rather than
/// a multiple of zero, which is the oracle's reading of a parameter with no default.
fn two_required(args: &mut Args) -> Result<(f64, f64), FormulaError> {
    args.arity(2..=2)?;
    if args.omitted(0) || args.omitted(1) {
        return Err(FormulaError::Value);
    }
    Ok((args.number(0)?, args.number(1)?))
}
