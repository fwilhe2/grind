// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How a typed number is read: in the document's own spelling first, then — where that cannot
//! be meant — in the other one, and with a [`Reading`] saying so whenever the answer is one a
//! person might not have expected (`doc/ods-format.md` §5.2, "The document's own language").
//!
//! Which sign is the decimal one is the commonest confusion a spreadsheet has, and refusing the
//! other spelling only moves it: `1.5` typed into a German document used to become the *text*
//! `1.5`, which sums to nothing. So the rule is to read what can only mean one thing as that
//! thing, and to say so:
//!
//! * **The document's spelling wins** wherever it reads: `1,5` and `1.234,5` in German.
//! * **The other spelling is read where the document's cannot be meant** — `1.5`, `3.14159` and
//!   `1,234.5` in German, `1,5` and `1.234,5` in English — and is a [`Reading::Other`].
//! * **The one truly ambiguous shape** is one separator with exactly three digits after it
//!   (`1.234`, `1,234`), which is a thousand in one language and a little over one in the
//!   other. It is read the document's way, and is a [`Reading::Ambiguous`] — the case where a
//!   misreading is a factor of a thousand, so it is the one most worth saying out loud.
//!
//! `'` before anything keeps it as text, as it always has; that rule is the typing rule's
//! (`App::enter`), ahead of this one.

use crate::csv;
use crate::locale::{self, Locale};
use crate::numfmt;

/// A number read in a way the person typing it should be told about.
#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    /// Read with the separators the document does *not* use, since its own could not mean
    /// it — `1.5` in a German document is one and a half.
    Other { typed: String, value: f64 },
    /// One separator and three digits after it: read the document's way, though the other way
    /// would have been a number a thousand times larger or smaller.
    Ambiguous { typed: String, value: f64 },
}

impl Reading {
    /// The sentence every shell shows after the entry — one wording, so six clients cannot
    /// explain the same reading six ways.
    pub fn sentence(&self, locale: Option<&Locale>) -> String {
        let (decimal, group) = locale::separators(locale);
        let decimal_name = locale::separator_name(decimal);
        match self {
            Reading::Other { typed, value } => format!(
                "Read {typed} as {}: this document writes decimals with a {decimal_name}. \
                 Type '{typed} to keep it as text.",
                numfmt::spell_number(*value, locale)
            ),
            Reading::Ambiguous { typed, value } => {
                let shown = numfmt::spell_number(*value, locale);
                match typed.contains(decimal) {
                    true => format!(
                        "Read {typed} as {shown}, a decimal: this document writes decimals with \
                         a {decimal_name}. For a thousand and more, type {}.",
                        typed.replace(decimal, "")
                    ),
                    false => format!(
                        "Read {typed} as {shown}: in this document a {} groups thousands. For a \
                         decimal, type {}.",
                        locale::separator_name(group),
                        typed.replace(group, &decimal.to_string())
                    ),
                }
            }
        }
    }
}

/// `input` as a number, and what is worth saying about how it was read — `None` when it is not
/// one at all. `strict` reads the document's own spelling and nothing else, which is what a
/// value somebody did not type wants (a CSV field, already spelled the ISO way).
pub(crate) fn read(
    input: &str,
    locale: Option<&Locale>,
    strict: bool,
) -> Option<(f64, Option<Reading>)> {
    let plain = |n: f64| Some((n, None));
    // With no locale, every typed number has always been Rust's own reading — `1.5`, `-3`,
    // `1e5`, `007` — and still is, before anything else is tried.
    if locale.is_none()
        && let Ok(n) = input.parse::<f64>()
    {
        return plain(n);
    }
    let (decimal, group) = locale::separators(locale);
    if let Some(n) = csv::number(input, decimal, group).and_then(|iso| iso.parse().ok()) {
        let note = (!strict && ambiguous(input)).then(|| Reading::Ambiguous {
            typed: input.to_owned(),
            value: n,
        });
        return Some((n, note));
    }
    // What the document's spelling refuses is still a number if it has no separator in it the
    // locale could mean otherwise — `007` and `1e5`.
    if !input.contains(['.', ',', ' ', '\u{a0}', '\u{202f}']) {
        return input.parse().ok().map(|n| (n, None));
    }
    if strict {
        return None;
    }
    // The other spelling: the point and the comma swapped, whatever the document groups with.
    let other = match decimal {
        ',' => '.',
        '.' => ',',
        _ => return None,
    };
    let n: f64 = csv::number(input, other, decimal)?.parse().ok()?;
    Some((
        n,
        Some(Reading::Other {
            typed: input.to_owned(),
            value: n,
        }),
    ))
}

/// One point or comma, with one to three digits before it (not a lone `0`, which cannot lead
/// a group) and exactly three after: `1.234`, `-12,500`.
fn ambiguous(input: &str) -> bool {
    let unsigned = input.strip_prefix(['+', '-']).unwrap_or(input);
    let mut parts = unsigned.split(['.', ',']);
    let (Some(whole), Some(rest), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    digits(whole) && whole.len() <= 3 && whole != "0" && digits(rest) && rest.len() == 3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn de() -> Option<Locale> {
        Some(Locale::new("de", "DE"))
    }

    fn en() -> Option<Locale> {
        Some(Locale::new("en", "US"))
    }

    fn value(input: &str, locale: Option<Locale>) -> Option<f64> {
        read(input, locale.as_ref(), false).map(|(n, _)| n)
    }

    fn note(input: &str, locale: Option<Locale>) -> Option<Reading> {
        read(input, locale.as_ref(), false).and_then(|(_, r)| r)
    }

    #[test]
    fn the_document_spelling_wins_and_is_said_nothing_about() {
        assert_eq!(read("1,5", de().as_ref(), false), Some((1.5, None)));
        assert_eq!(read("1.234,5", de().as_ref(), false), Some((1234.5, None)));
        assert_eq!(read("1.5", en().as_ref(), false), Some((1.5, None)));
        assert_eq!(read("1,234.5", en().as_ref(), false), Some((1234.5, None)));
        assert_eq!(read("007", de().as_ref(), false), Some((7.0, None)));
        assert_eq!(read("1e5", de().as_ref(), false), Some((1e5, None)));
        assert_eq!(read("1.5", None, false), Some((1.5, None)));
    }

    #[test]
    fn the_other_spelling_is_read_where_the_documents_cannot_be_meant() {
        for (typed, n) in [
            ("1.5", 1.5),
            ("3.25", 3.25),
            ("1,234.5", 1234.5),
            ("-0.5", -0.5),
        ] {
            assert_eq!(value(typed, de()), Some(n), "{typed}");
            assert!(
                matches!(note(typed, de()), Some(Reading::Other { .. })),
                "{typed}"
            );
        }
        assert_eq!(value("1,5", en()), Some(1.5));
        assert_eq!(value("1.234,5", en()), Some(1234.5));
        assert_eq!(value("1,5", None), Some(1.5));
        let french = Some(Locale::new("fr", "FR"));
        assert_eq!(value("2.5", french), Some(2.5));
    }

    #[test]
    fn what_neither_spelling_reads_stays_text() {
        for typed in ["1.2.3", "17.10.2026", "1,2,3", "1.5.", "a.5", "1. 5"] {
            assert_eq!(value(typed, de()), None, "{typed}");
            assert_eq!(value(typed, en()), None, "{typed}");
        }
    }

    #[test]
    fn one_separator_and_three_digits_is_read_the_documents_way_and_said() {
        assert_eq!(value("1.234", de()), Some(1234.0));
        assert_eq!(value("1,234", de()), Some(1.234));
        assert_eq!(value("1,234", en()), Some(1234.0));
        assert_eq!(value("1.234", en()), Some(1.234));
        for (typed, locale) in [("1.234", de()), ("1,234", de()), ("-12,500", en())] {
            assert!(
                matches!(note(typed, locale), Some(Reading::Ambiguous { .. })),
                "{typed}"
            );
        }
        // Two groups, a leading zero or four digits are not ambiguous.
        assert_eq!(note("1.234.567", de()), None);
        assert_eq!(note("0,500", de()), None);
        assert_eq!(note("1,2345", de()), None);
        assert_eq!(note("1,23", de()), None);
    }

    #[test]
    fn strict_reads_the_document_spelling_and_says_nothing() {
        assert_eq!(read("1.5", de().as_ref(), true), None);
        assert_eq!(read("1.234", de().as_ref(), true), Some((1234.0, None)));
    }

    #[test]
    fn every_reading_is_explained_in_one_sentence() {
        let de = de();
        assert_eq!(
            note("1.5", de.clone()).unwrap().sentence(de.as_ref()),
            "Read 1.5 as 1,5: this document writes decimals with a comma. Type '1.5 to keep \
             it as text."
        );
        assert_eq!(
            note("1.234", de.clone()).unwrap().sentence(de.as_ref()),
            "Read 1.234 as 1234: in this document a point groups thousands. For a decimal, \
             type 1,234."
        );
        assert_eq!(
            note("1,234", de.clone()).unwrap().sentence(de.as_ref()),
            "Read 1,234 as 1,234, a decimal: this document writes decimals with a comma. For a \
             thousand and more, type 1234."
        );
    }
}
