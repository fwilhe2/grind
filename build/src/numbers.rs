// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Numbers as text, for a locale — for a text document, which holds text and so has to spell a
//! figure itself ("Ihr Anteil: 1.234,50 €"). Over the core's own separators
//! (`grind_core::locale`), so a figure in a letter and the same figure in a cell cannot be spelled
//! two ways.
//!
//! The locale is an argument, never the machine's, so the same script writes the same letters
//! everywhere. Rounding is done once, on whole units of the last decimal place, so equal figures
//! print equally and `-0.004` at two decimals is `0,00`, not `−0,00`.

use grind_core::locale::Locale;
use rhai::{Dynamic, Engine, EvalAltResult};

use crate::hint::hint;

type Res<T> = Result<T, Box<EvalAltResult>>;

fn bad(message: impl Into<String>) -> Box<EvalAltResult> {
    message.into().into()
}

fn locale(tag: &str) -> Res<Locale> {
    Locale::parse(tag).ok_or_else(|| {
        bad(format!(
            "{tag:?} is not a locale: a language tag, `de-DE` or `en-US`"
        ))
    })
}

fn number_of(value: &Dynamic) -> Res<f64> {
    value
        .as_float()
        .ok()
        .or_else(|| value.as_int().ok().map(|i| i as f64))
        .ok_or_else(|| bad(format!("a number, not {}", value.type_name())))
}

/// `x` with exactly `decimals` fraction digits, the locale's separators, and a real minus sign
/// (U+2212) for a negative — which is what a letter's typography wants and what `-` is not.
pub fn number(x: f64, decimals: u32, tag: &str) -> Res<String> {
    let locale = locale(tag)?;
    if !x.is_finite() {
        return Err(bad("a number to print, not infinity or NaN"));
    }
    if decimals > 9 {
        return Err(bad("at most 9 decimals"));
    }
    let scale = 10_f64.powi(decimals as i32);
    let units = (x * scale).round();
    if units.abs() >= 9e15 {
        return Err(bad("that number is too large to print exactly"));
    }
    let units = units as i64;
    let negative = units < 0;
    let units = units.unsigned_abs();
    let whole = (units / scale as u64).to_string();
    let mut grouped = String::new();
    let digits: Vec<char> = whole.chars().collect();
    for (at, digit) in digits.iter().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            grouped.push(locale.group());
        }
        grouped.push(*digit);
    }
    if decimals > 0 {
        grouped.push(locale.decimal());
        grouped.push_str(&format!(
            "{:0width$}",
            units % scale as u64,
            width = decimals as usize
        ));
    }
    Ok(match negative {
        true => format!("\u{2212}{grouped}"),
        false => grouped,
    })
}

/// Whether a locale puts a following unit (currency symbol, percent sign) after its number, with a
/// space. The decimal-comma languages do; the others put a symbol before the amount and a percent
/// sign hard against it.
fn after(tag: &str) -> Res<bool> {
    Ok(locale(tag)?.decimal() == ',')
}

pub fn money(x: f64, symbol: &str, tag: &str) -> Res<String> {
    let amount = number(x, 2, tag)?;
    Ok(match after(tag)? {
        true => format!("{amount} {symbol}"),
        false => match amount.strip_prefix('\u{2212}') {
            Some(rest) => format!("\u{2212}{symbol}{rest}"),
            None => format!("{symbol}{amount}"),
        },
    })
}

/// A fraction as a percentage: `0.1648` is `16,48 %`.
pub fn percent(fraction: f64, decimals: u32, tag: &str) -> Res<String> {
    let amount = number(fraction * 100.0, decimals, tag)?;
    Ok(match after(tag)? {
        true => format!("{amount} %"),
        false => format!("{amount}%"),
    })
}

pub fn register(engine: &mut Engine) {
    hint(
        engine,
        "format_number",
        ["x: ?", "decimals: int", "locale: string", "string"],
        [
            "/// A number as text with exactly `decimals` fraction digits and the locale's own",
            "/// separators — `format_number(1234.5, 2, \"de-DE\")` is `1.234,50`. A negative gets a",
            "/// real minus sign (U+2212). The locale is an argument and never the machine's, so the",
            "/// same script writes the same text anywhere.",
        ],
        |x: Dynamic, decimals: i64, tag: &str| -> Res<String> {
            number(number_of(&x)?, decimals.max(0) as u32, tag)
        },
    );
    hint(
        engine,
        "format_money",
        ["x: ?", "symbol: string", "locale: string", "string"],
        [
            "/// An amount of money with two decimals: after the number with a space where the",
            "/// locale writes a decimal comma (`1.234,50 €`), before it elsewhere (`$1,234.50`).",
            "/// A plain space; a script that wants a no-break one says so.",
        ],
        |x: Dynamic, symbol: &str, tag: &str| -> Res<String> { money(number_of(&x)?, symbol, tag) },
    );
    hint(
        engine,
        "format_percent",
        ["fraction: ?", "decimals: int", "locale: string", "string"],
        [
            "/// A fraction as a percentage — `format_percent(0.1648, 2, \"de-DE\")` is `16,48 %`,",
            "/// and `16.48%` in English, where the sign is not set apart.",
        ],
        |x: Dynamic, decimals: i64, tag: &str| -> Res<String> {
            percent(number_of(&x)?, decimals.max(0) as u32, tag)
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn german_groups_with_a_dot_and_decimals_with_a_comma() {
        assert_eq!(number(1234.5, 2, "de-DE").unwrap(), "1.234,50");
        assert_eq!(number(1_234_567.0, 0, "de-DE").unwrap(), "1.234.567");
        assert_eq!(number(0.5, 3, "de-DE").unwrap(), "0,500");
        assert_eq!(number(999.0, 2, "de-DE").unwrap(), "999,00");
    }

    #[test]
    fn english_is_the_other_way_round() {
        assert_eq!(number(1234.5, 2, "en-US").unwrap(), "1,234.50");
    }

    #[test]
    fn a_negative_has_the_real_minus_and_zero_never_does() {
        assert_eq!(number(-12.5, 1, "de-DE").unwrap(), "\u{2212}12,5");
        assert_eq!(number(-0.004, 2, "de-DE").unwrap(), "0,00");
    }

    #[test]
    fn money_and_percent_follow_the_locale() {
        assert_eq!(money(1234.5, "€", "de-DE").unwrap(), "1.234,50 €");
        assert_eq!(money(1234.5, "$", "en-US").unwrap(), "$1,234.50");
        assert_eq!(money(-3.0, "$", "en-US").unwrap(), "\u{2212}$3.00");
        assert_eq!(percent(0.1648, 2, "de-DE").unwrap(), "16,48 %");
        assert_eq!(percent(0.1648, 2, "en-US").unwrap(), "16.48%");
    }

    #[test]
    fn rounding_is_once_and_half_away_from_zero() {
        assert_eq!(number(2.675, 2, "en-US").unwrap(), "2.68");
        assert_eq!(number(0.125, 2, "en-US").unwrap(), "0.13");
    }

    #[test]
    fn what_cannot_be_printed_is_refused() {
        assert!(number(f64::NAN, 2, "de-DE").is_err());
        assert!(number(1.0, 2, "").is_err());
        assert!(number(1.0, 12, "de-DE").is_err());
    }
}
