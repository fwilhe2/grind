// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.20 Text Functions — the Small Group's 14, and `CONCATENATE` and `SEARCH` from beyond it.
//! Positions and lengths count characters, not bytes, and are 1-based; `LEFT`/`MID`/`RIGHT` clamp
//! rather than panic.

use super::super::value::{FormulaError, Value};
use super::super::wildcard;
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        "LEN" => len(args),
        "LOWER" => map(args, str::to_lowercase),
        "UPPER" => map(args, str::to_uppercase),
        "PROPER" => map(args, proper),
        "TRIM" => map(args, trim),
        // §6.20.4: control characters (Unicode's Cc) go. Its Cn, unassigned code points, needs a
        // table `std` does not carry, and stays.
        "CLEAN" => map(args, |s| s.chars().filter(|c| !c.is_control()).collect()),
        "LEFT" => end(args, true),
        "RIGHT" => end(args, false),
        "MID" => mid(args),
        "EXACT" => exact(args),
        "FIND" => find(args),
        "REPT" => rept(args),
        "REPLACE" => replace(args),
        "SUBSTITUTE" => substitute(args),
        "T" => t(args),
        "CONCATENATE" => concatenate(args),
        "COM.MICROSOFT.CONCAT" => concat(args),
        "CHAR" => character(args, 255),
        "UNICHAR" => character(args, 0x10_FFFF),
        "CODE" | "UNICODE" => code(args),
        "COM.MICROSOFT.TEXTJOIN" => textjoin(args),
        "SEARCH" => search(args),
        "FIXED" => fixed(args),
        "TEXT" => text(args),
        _ => return None,
    })
}

/// §6.20.10 `FIXED(N; D; OmitSeparators)`: `N` rounded as `ROUND` rounds, spelled with `D`
/// decimals in the document's locale. More than fifteen places either way is refused, as the
/// oracle refuses it: a double holds no more.
fn fixed(args: &mut Args) -> Answer {
    use crate::numfmt::{Kind, preset};
    args.arity(1..=3)?;
    let n = args.number(0)?;
    let digits = if args.omitted(1) { 2 } else { args.integer(1)? };
    let grouping = args.omitted(2) || !args.logical(2)?;
    if digits.abs() > 15 {
        return Err(FormulaError::Num);
    }
    let rounded = super::math::at_digits(n, digits, |v| super::math::shown(v).round());
    // A value rounded to zero is spelled without its sign.
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let format = preset(Kind::Number, digits.max(0) as u8, grouping, "");
    Ok(Value::Text(format.render_in(
        &crate::CellValue::Number(rounded),
        0,
        args.locale(),
    )))
}

/// §6.20.23 `TEXT(Value; Format)`: the value spelled in a format code, read as
/// [`crate::numfmt::of_code`] reads one and rendered in the document's locale. An empty code is empty
/// text and `General` the plain number; text is its own spelling unless the code has a text
/// section. A code with a part the model cannot spell — a fraction, an exponent, a display factor —
/// is `#VALUE!` rather than a different number.
fn text(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let value = match args.value(0) {
        Value::Error(e) => return Err(e),
        Value::Bool(b) => crate::CellValue::Number(f64::from(u8::from(b))),
        Value::Number(n) => crate::CellValue::Number(n),
        Value::Text(t) => crate::CellValue::Text(t),
        Value::Empty => crate::CellValue::Number(0.0),
    };
    let code = args.text(1)?;
    if code.trim().is_empty() {
        return Ok(Value::Text(String::new()));
    }
    let translated = crate::numfmt::of_code(&code);
    if translated
        .lost
        .iter()
        .copied()
        .any(crate::numfmt::Unspellable::refuses)
    {
        return Err(FormulaError::Value);
    }
    let Some(format) = translated.format else {
        return Ok(Value::Text(match value {
            crate::CellValue::Number(n) => super::super::value::format_number(n),
            crate::CellValue::Text(t) => t,
            _ => String::new(),
        }));
    };
    Ok(Value::Text(format.render_in(
        &value,
        args.null_date(),
        args.locale(),
    )))
}

fn len(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    Ok(Value::number(args.text(0)?.chars().count() as f64))
}

fn map(args: &mut Args, f: impl Fn(&str) -> String) -> Answer {
    args.arity(1..=1)?;
    Ok(Value::Text(f(&args.text(0)?)))
}

/// §6.20.16: first letter of each word upper case, the rest lower.
fn proper(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start_of_word = true;
    for c in s.chars() {
        if start_of_word {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        start_of_word = !c.is_alphabetic();
    }
    out
}

/// §6.20.24: strip leading and trailing spaces, and collapse interior runs to one.
fn trim(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// §6.20.12 `LEFT` and §6.20.19 `RIGHT`, whose length defaults to 1.
fn end(args: &mut Args, left: bool) -> Answer {
    args.arity(1..=2)?;
    let text: Vec<char> = args.text(0)?.chars().collect();
    let count = if args.len() > 1 && !args.omitted(1) {
        args.integer(1)?
    } else {
        1
    };
    if count < 0 {
        return Err(FormulaError::Value);
    }
    let count = (count as usize).min(text.len());
    let taken = if left {
        &text[..count]
    } else {
        &text[text.len() - count..]
    };
    Ok(Value::Text(taken.iter().collect()))
}

/// §6.20.15 `MID(T; Start; Length)`, 1-based.
fn mid(args: &mut Args) -> Answer {
    args.arity(3..=3)?;
    let text: Vec<char> = args.text(0)?.chars().collect();
    let start = args.integer(1)?;
    let length = args.integer(2)?;
    if start < 1 || length < 0 {
        return Err(FormulaError::Value);
    }
    let start = (start as usize - 1).min(text.len());
    let end = start.saturating_add(length as usize).min(text.len());
    Ok(Value::Text(text[start..end].iter().collect()))
}

/// §6.20.8: case-**sensitive**, unlike the `=` operator (§6.4.7). That contrast is the
/// reason the function exists.
fn exact(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let (a, b) = (args.text(0)?, args.text(1)?);
    Ok(Value::Bool(a == b))
}

/// §6.20.9 `FIND(Search; T [; Start])`: case-sensitive, 1-based, `#VALUE!` when absent.
fn find(args: &mut Args) -> Answer {
    args.arity(2..=3)?;
    let needle = args.text(0)?;
    let haystack: Vec<char> = args.text(1)?.chars().collect();
    let start = if args.len() > 2 && !args.omitted(2) {
        args.integer(2)?
    } else {
        1
    };
    if start < 1 {
        return Err(FormulaError::Value);
    }
    let from = start as usize - 1;
    if from > haystack.len() {
        return Err(FormulaError::Value);
    }
    let needle: Vec<char> = needle.chars().collect();
    let found = (from..=haystack.len().saturating_sub(needle.len()))
        .find(|i| haystack[*i..].starts_with(&needle))
        .ok_or(FormulaError::Value)?;
    Ok(Value::number(found as f64 + 1.0))
}

/// §6.20.18.
fn rept(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let text = args.text(0)?;
    let count = args.integer(1)?;
    if count < 0 {
        return Err(FormulaError::Value);
    }
    // A repeat count is a number a user can typo into gigabytes; refuse rather than swap.
    if text.len().saturating_mul(count as usize) > 1 << 20 {
        return Err(FormulaError::Num);
    }
    Ok(Value::Text(text.repeat(count as usize)))
}

/// §6.20.17 `REPLACE(T; Start; Count; New)`, 1-based.
fn replace(args: &mut Args) -> Answer {
    args.arity(4..=4)?;
    let text: Vec<char> = args.text(0)?.chars().collect();
    let start = args.integer(1)?;
    let count = args.integer(2)?;
    let new = args.text(3)?;
    if start < 1 || count < 0 {
        return Err(FormulaError::Value);
    }
    let start = (start as usize - 1).min(text.len());
    let end = start.saturating_add(count as usize).min(text.len());
    let mut out: String = text[..start].iter().collect();
    out.push_str(&new);
    out.extend(&text[end..]);
    Ok(Value::Text(out))
}

/// §6.20.21 `SUBSTITUTE(T; Old; New [; Which])`. Without `Which`, every occurrence.
fn substitute(args: &mut Args) -> Answer {
    args.arity(3..=4)?;
    let text = args.text(0)?;
    let old = args.text(1)?;
    let new = args.text(2)?;
    let which = if args.len() > 3 && !args.omitted(3) {
        Some(args.integer(3)?)
    } else {
        None
    };
    // Replacing the empty string would loop forever or insert between every character.
    if old.is_empty() {
        return Ok(Value::Text(text));
    }
    Ok(Value::Text(match which {
        None => text.replace(&old, &new),
        Some(n) if n < 1 => return Err(FormulaError::Value),
        Some(n) => match text.match_indices(&old).nth(n as usize - 1) {
            Some((at, _)) => format!("{}{new}{}", &text[..at], &text[at + old.len()..]),
            None => text,
        },
    }))
}

/// §6.20.22: text passes through, everything else becomes the empty string. An error still
/// propagates — `T` reports types, it does not swallow failures.
fn t(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    Ok(match args.value(0) {
        Value::Text(s) => Value::Text(s),
        Value::Error(e) => return Err(e),
        _ => Value::Text(String::new()),
    })
}

/// §6.20.6 `CONCATENATE( { Text T } + )` — the `&` operator (§6.4.10) as a function, every
/// argument converted by §6.3.14.
fn concatenate(args: &mut Args) -> Answer {
    args.arity(1..)?;
    let mut out = String::new();
    for i in 0..args.len() {
        out.push_str(&args.text(i)?);
    }
    Ok(Value::Text(out))
}

/// §6.20.3 `CHAR` and §6.20.25 `UNICHAR`: the character with that Unicode code point — `CHAR` up
/// to 255, so that `CODE(CHAR(N))` is `N` (§6.20.3 Note 1 allows Unicode). Zero, and a control
/// character XML 1.0 cannot carry in a cell, are empty text, as the oracle gives them.
fn character(args: &mut Args, max: u32) -> Answer {
    args.arity(1..=1)?;
    let n = args.number(0)?.trunc();
    if !(0.0..=f64::from(max)).contains(&n) {
        return Err(FormulaError::Value);
    }
    let c = char::from_u32(n as u32).ok_or(FormulaError::Value)?;
    let storable = !c.is_control() || matches!(c, '\t' | '\n' | '\r') || u32::from(c) >= 0x7f;
    Ok(Value::Text(if storable && c != '\0' {
        c.to_string()
    } else {
        String::new()
    }))
}

/// §6.20.5 `CODE` and §6.20.26 `UNICODE`: the first character's Unicode code point.
fn code(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    let text = args.text(0)?;
    let first = text.chars().next().ok_or(FormulaError::Value)?;
    Ok(Value::number(f64::from(u32::from(first))))
}

/// Excel's `CONCAT`: `CONCATENATE` that also takes ranges, each cell's text in reading order.
fn concat(args: &mut Args) -> Answer {
    args.arity(1..)?;
    let mut out = String::new();
    for value in args.values() {
        out.push_str(&value.to_text()?);
    }
    Ok(Value::Text(out))
}

/// Excel's `TEXTJOIN(Delimiter; IgnoreEmpty; Text1; …)`: every text, ranges cell by cell, joined
/// by the delimiter — the cells of a delimiter range used in turn — and empty ones skipped when
/// asked.
fn textjoin(args: &mut Args) -> Answer {
    args.arity(3..)?;
    let delimiters: Vec<String> = match args.area(0) {
        Some(area) => args
            .values_in(&area)
            .into_iter()
            .map(|v| v.to_text())
            .collect::<Result<_, _>>()?,
        None => vec![args.text(0)?],
    };
    let skip_empty = args.logical(1)?;
    let mut pieces = Vec::new();
    for i in 2..args.len() {
        let values = match args.operand(i) {
            super::super::eval::Operand::Area(area) => args.values_in(&area),
            other @ super::super::eval::Operand::Value(_) => vec![args.scalar(other)],
        };
        for value in values {
            let piece = value.to_text()?;
            if !(skip_empty && piece.is_empty()) {
                pieces.push(piece);
            }
        }
    }
    let mut out = String::new();
    for (i, piece) in pieces.iter().enumerate() {
        if i > 0 && !delimiters.is_empty() {
            out.push_str(&delimiters[(i - 1) % delimiters.len()]);
        }
        out.push_str(piece);
    }
    Ok(Value::Text(out))
}

/// §6.20.20 `SEARCH( Search ; T [ ; Start = 1 ] )`: `FIND` without case and with wildcards. An
/// empty search text is found nowhere, and a start outside the text is an error, where `FIND`
/// clamps (measured, `doc/ods-format.md` §4.1).
fn search(args: &mut Args) -> Answer {
    args.arity(2..=3)?;
    let needle = args.text(0)?;
    let haystack = args.text(1)?;
    let start = if args.len() > 2 && !args.omitted(2) {
        args.integer(2)?
    } else {
        1
    };
    let length = haystack.chars().count() as i64;
    if needle.is_empty() || start < 1 || start > length {
        return Err(FormulaError::Value);
    }
    let found =
        wildcard::find(&needle, &haystack, start as usize - 1).ok_or(FormulaError::Value)?;
    Ok(Value::number(found as f64 + 1.0))
}
