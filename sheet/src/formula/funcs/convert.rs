// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.19 Number Representation Conversion Functions: Roman numerals, a number in any base from 2
//! to 36, and Excel's binary, octal and hexadecimal family, each of the last a two's complement of
//! ten digits.

use super::super::value::{FormulaError, Value};
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        "ROMAN" => roman(args),
        "BASE" => base(args),
        "DECIMAL" => decimal(args),
        "BIN2DEC" => to_decimal(args, Radix::Bin),
        "OCT2DEC" => to_decimal(args, Radix::Oct),
        "HEX2DEC" => to_decimal(args, Radix::Hex),
        "DEC2BIN" => from_decimal(args, Radix::Bin),
        "DEC2OCT" => from_decimal(args, Radix::Oct),
        "DEC2HEX" => from_decimal(args, Radix::Hex),
        "BIN2OCT" => between(args, Radix::Bin, Radix::Oct),
        "BIN2HEX" => between(args, Radix::Bin, Radix::Hex),
        "OCT2BIN" => between(args, Radix::Oct, Radix::Bin),
        "OCT2HEX" => between(args, Radix::Oct, Radix::Hex),
        "HEX2BIN" => between(args, Radix::Hex, Radix::Bin),
        "HEX2OCT" => between(args, Radix::Hex, Radix::Oct),
        _ => return None,
    })
}

const NUMERALS: [(char, i64); 7] = [
    ('M', 1000),
    ('D', 500),
    ('C', 100),
    ('L', 50),
    ('X', 10),
    ('V', 5),
    ('I', 1),
];

/// §6.19.17 `ROMAN( N [ ; Format = 0 ] )`: `N` from 0 to 3999 (0 is the empty text), and `Format`
/// from 0, the classic numerals, to 4, the fewest; TRUE is 0 and FALSE 4. Each level lets one
/// more numeral below the classic one be subtracted from a larger one (level 1 `LM`, `VL`; level
/// 2 `XM`, `IC`; …), and the largest value that fits is written first.
fn roman(args: &mut Args) -> Answer {
    args.arity(1..=2)?;
    let n = args.integer(0)?;
    let level = if args.len() < 2 || args.omitted(1) {
        0
    } else {
        match args.value(1) {
            Value::Bool(b) => {
                if b {
                    0
                } else {
                    4
                }
            }
            other => other.to_number()?.trunc() as i64,
        }
    };
    if !(0..4000).contains(&n) || !(0..=4).contains(&level) {
        return Err(FormulaError::Num);
    }
    // Every numeral, and every pair this level allows, by value.
    let mut steps: Vec<(String, i64)> = NUMERALS.iter().map(|(c, v)| (c.to_string(), *v)).collect();
    for (big, &(symbol, value)) in NUMERALS.iter().enumerate().take(6) {
        // The classic subtrahend: the power of ten just below.
        let classic = if big % 2 == 0 { big + 2 } else { big + 1 };
        for (small, value_small) in NUMERALS
            .iter()
            .enumerate()
            .skip(classic)
            .take(level as usize + 1)
            .map(|(i, (_, v))| (i, *v))
        {
            // A five (`V`, `L`, `D`) is never subtracted from the next numeral up.
            if value - value_small != value_small {
                steps.push((
                    format!("{}{symbol}", NUMERALS[small].0),
                    value - value_small,
                ));
            }
        }
    }
    steps.sort_by_key(|step| std::cmp::Reverse(step.1));
    let (mut rest, mut out) = (n, String::new());
    for (text, value) in &steps {
        while rest >= *value {
            out.push_str(text);
            rest -= value;
        }
    }
    Ok(Value::Text(out))
}

/// §6.19.3 `BASE( X ; Radix [ ; MinimumLength ] )`: `X` in base `Radix`, digits then upper-case
/// letters, padded with zeros to `MinimumLength`.
fn base(args: &mut Args) -> Answer {
    args.arity(2..=3)?;
    let x = args.number(0)?.trunc();
    let radix = args.integer(1)?;
    let length = if args.len() > 2 && !args.omitted(2) {
        args.integer(2)?
    } else {
        0
    };
    if x < 0.0 || x >= 2f64.powi(53) || !(2..=36).contains(&radix) || length < 0 {
        return Err(FormulaError::Num);
    }
    Ok(Value::Text(pad(
        digits(x as u64, radix as u64),
        length as usize,
    )))
}

fn digits(mut x: u64, radix: u64) -> String {
    let mut out = Vec::new();
    loop {
        out.push(
            char::from_digit((x % radix) as u32, radix as u32)
                .map_or('?', |c| c.to_ascii_uppercase()),
        );
        x /= radix;
        if x == 0 {
            break;
        }
    }
    out.iter().rev().collect()
}

fn pad(text: String, length: usize) -> String {
    if text.len() >= length {
        text
    } else {
        format!("{}{text}", "0".repeat(length - text.len()))
    }
}

/// §6.19.10 `DECIMAL( X ; Radix )`: text in base `Radix` as a number, letters in either case.
/// Leading spaces and tabs are ignored, and so are base 16's `0x` or `x` before and `h` after,
/// and base 2's `b` after. A character outside the base is `#NUM!`.
fn decimal(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let text = args.text(0)?;
    let radix = args.integer(1)?;
    if !(2..=36).contains(&radix) {
        return Err(FormulaError::Num);
    }
    let mut text = text.trim_start_matches([' ', '\t']);
    if radix == 16 {
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("0x") {
            text = &text[2..];
        } else if lower.starts_with('x') {
            text = &text[1..];
        }
        text = text.strip_suffix(['h', 'H']).unwrap_or(text);
    }
    if radix == 2 {
        text = text.strip_suffix(['b', 'B']).unwrap_or(text);
    }
    let mut n = 0f64;
    for c in text.chars() {
        let digit = c.to_digit(radix as u32).ok_or(FormulaError::Num)?;
        n = n * radix as f64 + f64::from(digit);
    }
    Ok(Value::number(n))
}

/// Excel's three bases: ten digits each, the top bit the sign.
#[derive(Clone, Copy)]
enum Radix {
    Bin,
    Oct,
    Hex,
}

impl Radix {
    fn radix(self) -> u32 {
        match self {
            Radix::Bin => 2,
            Radix::Oct => 8,
            Radix::Hex => 16,
        }
    }

    /// 2¹⁰, 8¹⁰ or 16¹⁰: the count of numbers ten digits hold.
    fn span(self) -> i64 {
        i64::from(self.radix()).pow(10)
    }
}

/// The digits of an argument, as text however it came: `HEX2DEC(10)` reads `"10"`, and a logical
/// is its number, which §6.19 leaves to the implementation and the oracle does.
fn number_text(args: &mut Args, i: usize) -> Result<String, FormulaError> {
    Ok(match args.value(i) {
        Value::Number(n) => super::super::value::format_number(n),
        Value::Bool(b) => if b { "1" } else { "0" }.to_owned(),
        other => other.to_text()?,
    })
}

/// Ten digits in `from` as a signed number: at least one digit, and nothing else — no sign.
fn signed(text: &str, from: Radix) -> Result<i64, FormulaError> {
    if text.is_empty()
        || text.chars().count() > 10
        || !text.chars().all(|c| c.is_digit(from.radix()))
    {
        return Err(FormulaError::Num);
    }
    let raw = i64::from_str_radix(text, from.radix()).map_err(|_| FormulaError::Num)?;
    Ok(if raw >= from.span() / 2 {
        raw - from.span()
    } else {
        raw
    })
}

/// `n` in ten-digit two's complement of `to`, padded to `places` when it is not negative — `#NUM!`
/// when `n` does not fit, or needs more than `places` digits.
fn spelled(n: i64, to: Radix, places: Option<i64>) -> Answer {
    let span = to.span();
    if n < -span / 2 || n >= span / 2 {
        return Err(FormulaError::Num);
    }
    let text = digits(n.rem_euclid(span) as u64, u64::from(to.radix()));
    if n < 0 {
        return Ok(Value::Text(text));
    }
    match places {
        Some(p) if p < text.len() as i64 || p > 10 => Err(FormulaError::Num),
        Some(p) => Ok(Value::Text(pad(text, p as usize))),
        None => Ok(Value::Text(text)),
    }
}

fn places(args: &mut Args, i: usize) -> Result<Option<i64>, FormulaError> {
    if args.len() > i && !args.omitted(i) {
        Ok(Some(args.integer(i)?))
    } else {
        Ok(None)
    }
}

/// §6.19.4, §6.19.15, §6.19.12 `BIN2DEC`, `OCT2DEC`, `HEX2DEC`.
fn to_decimal(args: &mut Args, from: Radix) -> Answer {
    args.arity(1..=1)?;
    let text = number_text(args, 0)?;
    Ok(Value::number(signed(&text, from)? as f64))
}

/// §6.19.7, §6.19.9, §6.19.8 `DEC2BIN`, `DEC2OCT`, `DEC2HEX( X [ ; Digits ] )`, `X` truncated.
fn from_decimal(args: &mut Args, to: Radix) -> Answer {
    args.arity(1..=2)?;
    let x = args.number(0)?.trunc();
    if x.abs() > 1e12 {
        return Err(FormulaError::Num);
    }
    let places = places(args, 1)?;
    spelled(x as i64, to, places)
}

/// §6.19's six conversions between Excel's bases, through the signed number.
fn between(args: &mut Args, from: Radix, to: Radix) -> Answer {
    args.arity(1..=2)?;
    let text = number_text(args, 0)?;
    let n = signed(&text, from)?;
    let places = places(args, 1)?;
    spelled(n, to, places)
}
