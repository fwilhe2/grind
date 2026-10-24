// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.16 Mathematical Functions and §6.17 Rounding Functions — the Small Group's 24 and 3, and
//! `QUOTIENT`, `SIGN`, `SUBTOTAL`, `SUMPRODUCT`, `ROUNDUP`, `ROUNDDOWN`, `MROUND`, `CEILING` and
//! `FLOOR` from beyond it, with Excel's `CEILING` and `FLOOR` under LibreOffice's names for them.
//!
//! Domain errors mostly need no code: a non-finite result becomes `#NUM!` in [`Value::number`].
//! Only where IEEE 754 disagrees with §6 — a division that must be `#DIV/0!` — is it written out.

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
        "SUMIFS" => super::criterion::several(args, super::criterion::Mode::Sum),
        "PRODUCT" => sequence(args, |ns| ns.iter().product()),
        "SUBTOTAL" => subtotal(args),
        "SUMPRODUCT" => sumproduct(args),
        "ROUND" => scaled(args, |x| shown(x).round()),
        "TRUNC" => scaled(args, |x| shown(x).trunc()),
        "ROUNDUP" => scaled(args, |x| shown(x).abs().ceil().copysign(x)),
        "ROUNDDOWN" => scaled(args, |x| shown(x).trunc()),
        "MROUND" => mround(args),
        "CEILING" => multiple(args, f64::ceil),
        "FLOOR" => multiple(args, f64::floor),
        "COM.MICROSOFT.CEILING" => excel_multiple(args, f64::ceil, None),
        "COM.MICROSOFT.FLOOR" => excel_multiple(args, f64::floor, Some(FormulaError::DivZero)),
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

/// §6.16.60 `SUBTOTAL( Function ; Sequence … )`: one of eleven aggregates over the referenced
/// cells that are not hidden by the filter (nor, for 101 to 111, by hand) and hold no subtotal of
/// their own ([`Args::counted`]).
///
/// Several references are accepted, as Excel and the oracle do. The function number is truncated;
/// one outside both bands, an empty one, or a non-reference sequence is `#VALUE!` where the oracle
/// answers `Err:502` (`doc/ods-format.md` §4.1).
fn subtotal(args: &mut Args) -> Answer {
    args.arity(2..)?;
    if args.omitted(0) {
        return Err(FormulaError::Value);
    }
    let (function, collapsed) = match args.integer(0)? {
        id @ 1..=11 => (id, false),
        id @ 101..=111 => (id - 100, true),
        _ => return Err(FormulaError::Value),
    };
    let mut values = Vec::new();
    for i in 1..args.len() {
        values.extend(args.counted(i, collapsed).ok_or(FormulaError::Value)?);
    }
    let count = |keep: fn(&Value) -> bool| {
        Ok(Value::number(
            values.iter().filter(|v| keep(v)).count() as f64
        ))
    };
    match function {
        2 => return count(|v| matches!(v, Value::Number(_))),
        3 => return count(|v| !matches!(v, Value::Empty)),
        _ => {}
    }
    let mut numbers = Vec::new();
    for value in &values {
        match value {
            Value::Number(n) => numbers.push(*n),
            Value::Error(e) => return Err(*e),
            Value::Empty | Value::Text(_) | Value::Bool(_) => {}
        }
    }
    use super::stat::{extreme_of, mean, moment_of};
    match function {
        1 => mean(&numbers),
        4 => extreme_of(&numbers, true),
        5 => extreme_of(&numbers, false),
        6 => Ok(Value::number(numbers.iter().product())),
        7 => moment_of(&numbers, true, true),
        8 => moment_of(&numbers, false, true),
        9 => Ok(Value::number(numbers.iter().sum())),
        10 => moment_of(&numbers, true, false),
        _ => moment_of(&numbers, false, false),
    }
}

/// §6.16.64 `SUMPRODUCT( { ForceArray Array A }+ )`: the arrays multiplied place by place and the
/// products added. Each argument is evaluated in array mode (`array.rs`). A non-number element is
/// zero and a logical is 1 or 0, as in the oracle; an error anywhere is the answer; different
/// shapes or an empty argument are `#VALUE!`.
fn sumproduct(args: &mut Args) -> Answer {
    args.arity(1..)?;
    let mut arrays = Vec::with_capacity(args.len());
    for i in 0..args.len() {
        if args.omitted(i) {
            return Err(FormulaError::Value);
        }
        arrays.push(args.array(i));
    }
    // An argument that came to one error is the answer before any shape is compared, as the
    // oracle has it: `SUMPRODUCT([.B1:.B4];NOSUCH(1))` is #NAME?, not #VALUE!.
    if let Some(e) = arrays.iter().find_map(|a| match a.values() {
        [Value::Error(e)] => Some(*e),
        _ => None,
    }) {
        return Err(e);
    }
    let (rows, cols) = (arrays[0].rows, arrays[0].cols);
    if arrays.iter().any(|a| (a.rows, a.cols) != (rows, cols)) {
        return Err(FormulaError::Value);
    }
    let mut sum = 0.0;
    for place in 0..rows * cols {
        let mut product = 1.0;
        for array in &arrays {
            product *= match &array.values()[place] {
                Value::Number(n) => *n,
                Value::Bool(b) => f64::from(u8::from(*b)),
                Value::Error(e) => return Err(*e),
                Value::Empty | Value::Text(_) => 0.0,
            };
        }
        sum += product;
    }
    Ok(Value::number(sum))
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

/// A value as a spreadsheet shows it — rounded to fifteen significant digits — for a function
/// about to step it to a whole number. `4.35*100` is `434.99999999999994` and displays as `435`,
/// and LibreOffice steps the shown value (`doc/ods-format.md` §4.1).
///
/// Only below 10^11, where the measurement is consistent; above it the oracle's answer depends on
/// its own binary steps, and a 16-digit integer such as `2^52+1` must not become its 15-digit
/// neighbour.
pub fn shown(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() || x.abs() >= 1e11 {
        return x;
    }
    format!("{x:.14e}").parse().unwrap_or(x)
}

/// §6.17.4 `MROUND(A; B)`: the multiple of `B` nearest `A`, and on a tie "the greater one". Both
/// the quotient and the multiple are read as [`shown`], so `MROUND(1.3;0.2)` is the tie it looks
/// like. A multiple of zero is zero, as measured.
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
    Ok(Value::number(shown(k * b)))
}

/// §6.17.1 `CEILING` and §6.17.3 `FLOOR`: `N` stepped to a multiple of `Significance`. With no or
/// a zero `Mode`, on the number line (`CEILING(-2.5)` is -2); otherwise the magnitude, sign
/// restored (`CEILING(-2.5;;1)` is -3). An empty `Significance` is ±1 with `N`'s sign, a zero in
/// either gives zero, and opposite signs are `#NUM!` whatever the Mode, as the spec and the oracle
/// say. The multiple is read as [`shown`].
fn multiple(args: &mut Args, step: fn(f64) -> f64) -> Answer {
    args.arity(1..=3)?;
    let n = args.number(0)?;
    let significance = if args.omitted(1) {
        if n < 0.0 { -1.0 } else { 1.0 }
    } else {
        args.number(1)?
    };
    let magnitude = !args.omitted(2) && args.number(2)? != 0.0;
    if n == 0.0 || significance == 0.0 {
        return Ok(Value::Number(0.0));
    }
    if (n < 0.0) != (significance < 0.0) {
        return Err(FormulaError::Num);
    }
    let q = shown(n / significance);
    // Both signs agree, so the quotient is positive and `step` on it steps the magnitude;
    // on the number line a negative pair needs the opposite step.
    let k = if magnitude || n > 0.0 {
        step(q)
    } else {
        -step(-q)
    };
    Ok(Value::number(shown(k * significance)))
}

/// Excel's `CEILING` and `FLOOR`, carried as `COM.MICROSOFT.CEILING` and `COM.MICROSOFT.FLOOR`
/// (`doc/xlsx-format.md` §5.1): two parameters, the quotient stepped on the number line whatever
/// the signs. A positive `N` with a negative `Significance` is `#NUM!`; a zero gives zero, except
/// `by_zero` for a zero `Significance` (`FLOOR`'s `#DIV/0!`).
fn excel_multiple(args: &mut Args, step: fn(f64) -> f64, by_zero: Option<FormulaError>) -> Answer {
    args.arity(2..=2)?;
    let (n, significance) = (args.number(0)?, args.number(1)?);
    if n == 0.0 {
        return Ok(Value::Number(0.0));
    }
    if significance == 0.0 {
        return by_zero.map_or(Ok(Value::Number(0.0)), Err);
    }
    if n > 0.0 && significance < 0.0 {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(shown(
        step(shown(n / significance)) * significance,
    )))
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
