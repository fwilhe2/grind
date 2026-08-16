// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Locale — `number:language` and `number:country`, and the two characters they decide.
//!
//! A number format carries its own locale (doc/ods-format.md §5.2), and it is not
//! decoration: `1234.5` displays as `1,234.50` in `en-US` and `1.234,50` in `de-DE` from the
//! *same* `number:number` element. Everything else about the format is already explicit, so
//! this is a very small module by design — the separators, and nothing else.
//!
//! **The language and country are kept verbatim, as the document spells them.** They are
//! BCP 47 subtags in ODF and they go back out unchanged, so a locale this build has never
//! heard of survives a round trip and merely falls back to the default separators.
//!
//! ponytail: one table, three groups, from the decimal-separator convention rather than from
//! CLDR. It gets the common European and Anglophone cases right and will be wrong in the
//! details — Switzerland's apostrophe grouping, India's lakh digit grouping, the narrow
//! no-break space several standards bodies now prefer to French's plain space. The upgrade
//! is a real CLDR table (or `icu`), and the reason not to have one yet is that nothing here
//! needs collation, plurals or calendars — the rest of a locale library — and a dependency
//! that large for two characters is the trade this project exists not to make.
//!
//! Not here either, and named where it belongs: **text→number conversion stays ISO-only**
//! (`formula::value`). Part 4 §6.3.6 makes it `HOST-LOCALE`-dependent, so LibreOffice reads
//! `"0,005"` as a number in a German document and this build does not. That one is a phase 4
//! conformance item, not a formatting one: it needs the locale threaded into the evaluator's
//! value model, where nothing carries a document today.

use serde::{Deserialize, Serialize};

/// A document's spelling of a locale: `number:language="de" number:country="DE"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Locale {
    pub language: String,
    /// May be empty: ODF allows a language without a country, and so does BCP 47.
    pub country: String,
}

/// Languages that write the decimal point as a comma, and therefore group with a stop.
///
/// The list is the discriminator, not an inventory of supported locales — anything absent
/// gets the `.`/`,` pair, which is what English, Chinese, Japanese and Korean use.
const COMMA_DECIMAL: [&str; 26] = [
    "af", "az", "be", "bg", "ca", "cs", "da", "de", "el", "es", "et", "fi", "fr", "hr", "hu",
    "id", "is", "it", "lt", "lv", "nb", "nl", "pl", "pt", "ro", "ru",
];

/// Languages that group with a space rather than with the other separator.
const SPACE_GROUPING: [&str; 6] = ["fr", "cs", "fi", "lv", "pl", "ru"];

impl Locale {
    pub fn new(language: impl Into<String>, country: impl Into<String>) -> Self {
        Self {
            language: language.into(),
            country: country.into(),
        }
    }

    /// A BCP 47 tag as a shell writes one — `de-DE`, or a bare `de`.
    pub fn parse(tag: &str) -> Option<Locale> {
        let (language, country) = match tag.split_once(['-', '_']) {
            Some((language, country)) => (language, country),
            None => (tag, ""),
        };
        let ok = |s: &str| s.chars().all(|c| c.is_ascii_alphabetic());
        (!language.is_empty() && ok(language) && ok(country))
            .then(|| Locale::new(language.to_lowercase(), country.to_uppercase()))
    }

    fn comma_decimal(&self) -> bool {
        COMMA_DECIMAL.contains(&self.language.to_lowercase().as_str())
    }

    pub fn decimal(&self) -> char {
        match self.comma_decimal() {
            true => ',',
            false => '.',
        }
    }

    pub fn group(&self) -> char {
        if SPACE_GROUPING.contains(&self.language.to_lowercase().as_str()) {
            // A no-break space: a thousands separator that wraps is not one.
            return '\u{a0}';
        }
        match self.comma_decimal() {
            true => '.',
            false => ',',
        }
    }
}

/// What a format with no locale of its own uses — the separators of an unmarked document.
pub const DEFAULT: (char, char) = ('.', ',');

/// The decimal and grouping characters for an optional locale.
pub fn separators(locale: Option<&Locale>) -> (char, char) {
    locale.map_or(DEFAULT, |l| (l.decimal(), l.group()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_separators_swap_with_the_language_and_not_with_the_country() {
        let de = Locale::new("de", "DE");
        assert_eq!((de.decimal(), de.group()), (',', '.'));
        // Swiss German writes German numbers as far as this table is concerned; the country
        // is carried but does not decide.
        let ch = Locale::new("de", "CH");
        assert_eq!((ch.decimal(), ch.group()), (',', '.'));
        let en = Locale::new("en", "GB");
        assert_eq!((en.decimal(), en.group()), ('.', ','));
        let fr = Locale::new("fr", "FR");
        assert_eq!((fr.decimal(), fr.group()), (',', '\u{a0}'));
        // A language nobody here has heard of falls back rather than failing.
        assert_eq!(separators(Some(&Locale::new("zz", "ZZ"))), DEFAULT);
        assert_eq!(separators(None), DEFAULT);
    }

    #[test]
    fn a_tag_parses_in_the_spellings_a_person_types() {
        assert_eq!(Locale::parse("de-DE"), Some(Locale::new("de", "DE")));
        assert_eq!(Locale::parse("de_de"), Some(Locale::new("de", "DE")));
        assert_eq!(Locale::parse("DE"), Some(Locale::new("de", "")));
        assert_eq!(Locale::parse("de-DE-x"), None);
        assert_eq!(Locale::parse("12"), None);
        assert_eq!(Locale::parse(""), None);
    }
}
