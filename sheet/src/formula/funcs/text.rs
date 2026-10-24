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
        "LOWER" => map(args, |s| s.to_lowercase()),
        "UPPER" => map(args, |s| s.to_uppercase()),
        "PROPER" => map(args, proper),
        "TRIM" => map(args, trim),
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
        "SEARCH" => search(args),
        _ => return None,
    })
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
