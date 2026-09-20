// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel number formats — at X1, only the question **is this a date, a time, or neither?**
//!
//! Excel stores a date as a plain number and only the format says otherwise, so the value
//! reader cannot put a number in the model without asking. That one question is all this
//! milestone takes from a format; translating the rest of a format code onto
//! `grind_sheet::numfmt::Format` is X3's, and will grow here (`doc/xlsx-import.md` Part II §4).
//!
//! Both halves are ECMA-376's: the built-in ids by §18.8.30's table, a custom code by
//! §18.8.31's grammar. `doc/xlsx-format.md` §3.1 records which ids are counted and why the
//! East Asian ones are not, and §3.2 the positional rule for `m`.

use grind_sheet::model::NumberKind;

/// What a built-in format id means, for the value reader. `SPEC`, ECMA-376 §18.8.30:
///
/// - 14–17 are dates (`mm-dd-yy`, `d-mmm-yy`, `d-mmm`, `mmm-yy`) and 22 is a date with a
///   time (`m/d/yy h:mm`) — a date either way, because the value is a point in time.
/// - 18–21 and 45–47 are clock and elapsed times (`h:mm AM/PM`, `h:mm:ss`, `mm:ss`,
///   `[h]:mm:ss`, `mmss.0`) — a duration.
///
/// Every other id below 164 is a number, text or currency format, and so is an id the table
/// does not list. The East Asian date ids (27–36, 50–58) are **not** here: §18.8.30 does not
/// define them, they are locale-dependent in Excel, and `doc/xlsx-format.md`'s rule is that
/// an unverified fact is not implemented. A file whose cell uses one carries its serial as a
/// number, which is visible and recoverable rather than a wrong date.
pub fn builtin(id: u32) -> Option<NumberKind> {
    match id {
        14..=17 | 22 => Some(NumberKind::Date),
        18..=21 | 45..=47 => Some(NumberKind::Time),
        _ => None,
    }
}

/// What a custom format code means, for the value reader. §18.8.31.
///
/// Only the **first section** is read — it is the one a positive number is shown in, and a
/// code whose sections disagree about being a date is not something any producer writes.
/// Within it, everything that is not a format *token* is skipped: quoted literals (`"Day "`),
/// escaped characters (`\d`), the character after `_` (a width) and after `*` (a fill), and
/// bracketed modifiers (`[Red]`, `[$€-407]`, `[>=100]`) — except the elapsed-time ones, `[h]`,
/// `[mm]`, `[ss]`, which *are* tokens. Then:
///
/// - any `y` or `d`, or an `m` that is a **month**, → a **date**. `e` is deliberately not
///   counted: in a format code it is an exponent (`0.00E+00`) before it is ever an era year.
/// - otherwise any `h`, `s`, an `m` that is **minutes**, an elapsed bracket, or `AM/PM` → a
///   **time**.
/// - otherwise, a number.
///
/// **Which `m` is which** is positional, and it is what a first version of this function got
/// wrong: `m` and `mm` are minutes when the nearest token before them is an hour or the nearest
/// after is a second (`h:mm`, `mm:ss`), and a month everywhere else — so `mm` alone is a month,
/// and so is `mmm` and longer, always. `numfmt/datetime-codes.xlsx` has one cell per spelling
/// and is where the first version was caught. `doc/xlsx-format.md` §3.2 records the rule.
///
/// Case-insensitive, as Excel reads them. `General` has none of the letters, which is not a
/// coincidence anybody should rely on and is tested.
pub fn classify(code: &str) -> Option<NumberKind> {
    let tokens = tokens(code);
    let mut date = false;
    let mut time = false;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Year | Token::Day => date = true,
            Token::Hour | Token::Second | Token::Elapsed | Token::AmPm => time = true,
            Token::M(len) => {
                let minutes = *len <= 2
                    && (matches!(
                        tokens.get(i.wrapping_sub(1)),
                        Some(Token::Hour | Token::Elapsed)
                    ) || matches!(tokens.get(i + 1), Some(Token::Second)));
                match minutes {
                    true => time = true,
                    false => date = true,
                }
            }
        }
    }
    if date {
        Some(NumberKind::Date)
    } else if time {
        Some(NumberKind::Time)
    } else {
        None
    }
}

/// A date or time token of a format code, with everything between them — separators, digits,
/// literals — already gone, so "the token before" is a question about this list.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Year,
    /// A run of `m`, and its length: the one letter whose meaning depends on its neighbours.
    M(usize),
    Day,
    Hour,
    Second,
    /// `[h]`, `[mm]`, `[ss]` — an elapsed count, which is a duration however it is spelled.
    Elapsed,
    /// `AM/PM` or `A/P`, consumed whole so that their `M` and `P` are not read as tokens.
    AmPm,
}

/// The first section's date and time tokens, in order.
fn tokens(code: &str) -> Vec<Token> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c.to_ascii_lowercase() {
            ';' => break,
            '"' => {
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
                i += 1;
            }
            '\\' | '_' | '*' => i += 1,
            '[' => {
                let close = chars[i..]
                    .iter()
                    .position(|&c| c == ']')
                    .map_or(chars.len(), |p| i + p);
                let inner: String = chars[i..close]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase();
                if !inner.is_empty() && inner.chars().all(|c| matches!(c, 'h' | 'm' | 's')) {
                    out.push(Token::Elapsed);
                }
                i = close + 1;
            }
            'a' => {
                let rest: String = chars[i - 1..]
                    .iter()
                    .take(5)
                    .collect::<String>()
                    .to_ascii_lowercase();
                if rest.starts_with("am/pm") {
                    out.push(Token::AmPm);
                    i += 4;
                } else if rest.starts_with("a/p") {
                    out.push(Token::AmPm);
                    i += 2;
                }
            }
            'y' => out.push(Token::Year),
            'd' => out.push(Token::Day),
            'h' => out.push(Token::Hour),
            's' => out.push(Token::Second),
            'm' => {
                let mut len = 1;
                while i < chars.len() && chars[i].eq_ignore_ascii_case(&'m') {
                    len += 1;
                    i += 1;
                }
                out.push(Token::M(len));
            }
            _ => {}
        }
    }
    // A run of one letter is one token: `yyyy` is a year, not four of them. Only `m` needs its
    // length, and it was counted above.
    out.dedup_by(|b, a| a == b && !matches!(a, Token::M(_)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_date_and_time_ids() {
        assert_eq!(builtin(0), None, "General");
        assert_eq!(builtin(14), Some(NumberKind::Date));
        assert_eq!(
            builtin(22),
            Some(NumberKind::Date),
            "a date with a time is a date"
        );
        assert_eq!(builtin(18), Some(NumberKind::Time));
        assert_eq!(builtin(46), Some(NumberKind::Time), "[h]:mm:ss");
        assert_eq!(builtin(49), None, "@");
        // East Asian ids are not in §18.8.30 and are not guessed at.
        assert_eq!(builtin(31), None);
    }

    #[test]
    fn a_code_is_a_date_when_it_has_a_year_or_a_day() {
        assert_eq!(classify("yyyy-mm-dd"), Some(NumberKind::Date));
        assert_eq!(classify("yyyy-mm-dd hh:mm:ss"), Some(NumberKind::Date));
        assert_eq!(classify("DD.MM.YYYY"), Some(NumberKind::Date));
        assert_eq!(classify("mmm-yy"), Some(NumberKind::Date));
    }

    #[test]
    fn a_code_is_a_time_when_it_only_has_a_clock() {
        assert_eq!(classify("hh:mm:ss"), Some(NumberKind::Time));
        assert_eq!(classify("[h]:mm:ss"), Some(NumberKind::Time));
        assert_eq!(classify("[mm]:ss"), Some(NumberKind::Time));
        assert_eq!(classify("h AM/PM"), Some(NumberKind::Time));
        assert_eq!(classify("h:mm A/P"), Some(NumberKind::Time));
        assert_eq!(
            classify("mm:ss.0"),
            Some(NumberKind::Time),
            "minutes, before a second"
        );
        assert_eq!(
            classify("h:mm"),
            Some(NumberKind::Time),
            "minutes, after an hour"
        );
        assert_eq!(
            classify("[h]:mm"),
            Some(NumberKind::Time),
            "after an elapsed hour"
        );
    }

    /// The positional rule, over `numfmt/datetime-codes.xlsx`'s own spellings: an `m` with no
    /// hour before it and no second after it is a month, at any length.
    #[test]
    fn an_m_on_its_own_is_a_month() {
        for code in [
            "m",
            "mm",
            "mmm",
            "mmmm",
            "mmmmm",
            "[$-409]mmmm\\ d\\,\\ yyyy",
        ] {
            assert_eq!(classify(code), Some(NumberKind::Date), "{code}");
        }
        // German letters for day and year are not tokens, so what is left is a lone `MM`.
        assert_eq!(classify("[$-407]TT.MM.JJJJ"), Some(NumberKind::Date));
        // A month, however long, is never minutes — even after an hour.
        assert_eq!(classify("h mmm"), Some(NumberKind::Date));
    }

    #[test]
    fn literals_and_modifiers_are_not_tokens() {
        assert_eq!(classify("General"), None);
        assert_eq!(classify("0.00"), None);
        assert_eq!(classify("#,##0.00;[Red]-#,##0.00"), None);
        assert_eq!(classify("0.00E+00"), None, "an exponent, not an era");
        assert_eq!(classify(r#""days: "0"#), None, "a quoted d is a letter");
        assert_eq!(classify(r"0\d"), None, "an escaped d is a letter");
        assert_eq!(classify("[$€-407] #,##0.00"), None, "a currency bracket");
        assert_eq!(
            classify("_(* #,##0_);_(* (#,##0)"),
            None,
            "widths and fills"
        );
        assert_eq!(classify("@"), None);
    }

    #[test]
    fn only_the_first_section_decides() {
        assert_eq!(classify(r#"0;"d""#), None);
        assert_eq!(classify("yyyy;0"), Some(NumberKind::Date));
    }
}
