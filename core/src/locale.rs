// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Locale — `number:language` and `number:country`, and the two characters they decide.
//!
//! A number format carries its own locale (`doc/ods-format.md` §5.2): `1234.5` displays as
//! `1,234.50` in `en-US` and `1.234,50` in `de-DE` from the same element. Language and country are
//! kept verbatim, so an unknown locale survives a round trip and falls back to the default
//! separators.
//!
//! ponytail: one table from the decimal-separator convention rather than CLDR. Right for the
//! common European and Anglophone cases, wrong in the details (Swiss apostrophe grouping, Indian
//! lakh grouping, the narrow no-break space). The upgrade is CLDR or `icu`, which nothing else
//! here needs.
//!
//! Text→number conversion in formulas stays ISO-only (`formula::value`): Part 4 §6.3.6 makes it
//! host-locale dependent, which needs a locale in the evaluator's value model.

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
const COMMA_DECIMAL: [&str; 30] = [
    "af", "az", "be", "bg", "ca", "cs", "da", "de", "el", "es", "et", "fi", "fr", "hr", "hu", "id",
    "is", "it", "lt", "lv", "nb", "nl", "nn", "no", "pl", "pt", "ro", "ru", "sv", "tr",
];

/// Languages that group with a space rather than with the other separator.
const SPACE_GROUPING: [&str; 11] = [
    "fr", "cs", "fi", "hu", "lv", "nb", "nn", "no", "pl", "ru", "sv",
];

/// The locales a picker offers, by name — exactly the ones whose separators this module gets right
/// (hence no Switzerland, Portugal or India). A document may still state any locale.
pub const KNOWN: [(&str, &str); 29] = [
    ("zh-CN", "Chinese (China)"),
    ("cs-CZ", "Czech (Czechia)"),
    ("da-DK", "Danish (Denmark)"),
    ("nl-BE", "Dutch (Belgium)"),
    ("nl-NL", "Dutch (Netherlands)"),
    ("en-AU", "English (Australia)"),
    ("en-CA", "English (Canada)"),
    ("en-IE", "English (Ireland)"),
    ("en-NZ", "English (New Zealand)"),
    ("en-GB", "English (United Kingdom)"),
    ("en-US", "English (United States)"),
    ("fi-FI", "Finnish (Finland)"),
    ("fr-BE", "French (Belgium)"),
    ("fr-CA", "French (Canada)"),
    ("fr-FR", "French (France)"),
    ("de-DE", "German (Germany)"),
    ("el-GR", "Greek (Greece)"),
    ("hu-HU", "Hungarian (Hungary)"),
    ("it-IT", "Italian (Italy)"),
    ("ja-JP", "Japanese (Japan)"),
    ("ko-KR", "Korean (South Korea)"),
    ("nb-NO", "Norwegian Bokmål (Norway)"),
    ("pl-PL", "Polish (Poland)"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("ro-RO", "Romanian (Romania)"),
    ("ru-RU", "Russian (Russia)"),
    ("es-ES", "Spanish (Spain)"),
    ("sv-SE", "Swedish (Sweden)"),
    ("tr-TR", "Turkish (Türkiye)"),
];

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
        // `none` is ODF's word for no country and stays as spelled, or a file and its projection
        // would be two locales (`tdf160368.ods`, loop F).
        let country = match country.eq_ignore_ascii_case("none") {
            true => "none".to_owned(),
            false => country.to_uppercase(),
        };
        (!language.is_empty() && ok(language) && ok(&country))
            .then(|| Locale::new(language.to_lowercase(), country))
    }

    /// [`Locale::parse`]'s inverse — how a shell shows and takes a locale back.
    pub fn tag(&self) -> String {
        match self.country.is_empty() {
            true => self.language.clone(),
            false => format!("{}-{}", self.language, self.country),
        }
    }

    /// The name a person knows this locale by, when it is one of [`KNOWN`].
    pub fn name(&self) -> Option<&'static str> {
        let tag = self.tag();
        KNOWN
            .iter()
            .find(|(known, _)| *known == tag)
            .map(|(_, name)| *name)
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

/// The app's locale when nothing more specific says otherwise: `GRIND_LOCALE`, then the XDG
/// config file, then none at all — the separators an unmarked format already uses. A CLI flag
/// or a picker's own entry outranks this; a caller with one of those just skips calling it.
pub fn from_environment() -> Option<Locale> {
    std::env::var("GRIND_LOCALE")
        .ok()
        .and_then(|tag| Locale::parse(&tag))
        .or_else(from_config_file)
}

/// A POSIX locale name as the environment spells one — `de_DE.UTF-8`, `sv_SE@euro`, `fr` — as
/// a locale. `C` and `POSIX` are the absence of one, and so is anything that is not a language.
pub fn from_posix(value: &str) -> Option<Locale> {
    let name = value.split(['.', '@']).next().unwrap_or_default().trim();
    match name {
        "" | "C" | "POSIX" => None,
        name => Locale::parse(name),
    }
}

/// The desktop's locale for numbers: `LC_ALL`, then `LC_NUMERIC`, then `LANG`. What a window gives
/// a new document (`doc/ods-format.md` §5.2); the CLI states none unless told, so a script's
/// output does not depend on whose shell ran it.
pub fn from_desktop() -> Option<Locale> {
    ["LC_ALL", "LC_NUMERIC", "LANG"]
        .into_iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|value| !value.is_empty())
        .and_then(|value| from_posix(&value))
}

/// `$XDG_CONFIG_HOME/grind/locale` (or `~/.config/grind/locale`): a bare BCP 47 tag such as
/// `de-DE`. The old `sheet/locale` path is read when the new one is absent.
///
/// ponytail: drop that fallback once there has been a release under the new name.
fn from_config_file() -> Option<Locale> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })?;
    let tag = std::fs::read_to_string(base.join("grind/locale"))
        .or_else(|_| std::fs::read_to_string(base.join("sheet/locale")))
        .ok()?;
    Locale::parse(tag.trim())
}

/// What a format with no locale of its own uses — the separators of an unmarked document.
pub const DEFAULT: (char, char) = ('.', ',');

/// The decimal and grouping characters for an optional locale.
pub fn separators(locale: Option<&Locale>) -> (char, char) {
    locale.map_or(DEFAULT, |l| (l.decimal(), l.group()))
}

/// One number spelled with `locale`'s separators — `1.234,5`, `1,234.5`, `1 234,5` — which shows
/// both at once.
pub fn sample(locale: Option<&Locale>) -> String {
    let (decimal, group) = separators(locale);
    format!("1{group}234{decimal}5")
}

/// What a status bar shows for a document's number spelling: [`sample`], then the locale's tag
/// when the document states one — `1.234,5 · de-DE`, or `1,234.5` for an unmarked document.
pub fn badge(locale: Option<&Locale>) -> String {
    match locale {
        Some(locale) => format!("{} \u{00b7} {}", sample(Some(locale)), locale.tag()),
        None => sample(None),
    }
}

/// A separator's name in a sentence — `comma`, `point`, `space`.
pub fn separator_name(c: char) -> &'static str {
    match c {
        ',' => "comma",
        '.' => "point",
        '\'' | '\u{2019}' => "apostrophe",
        c if c.is_whitespace() => "space",
        _ => "sign",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_round_trips_through_parse() {
        for tag in ["de-DE", "de", "pt-BR"] {
            assert_eq!(Locale::parse(tag).unwrap().tag(), tag);
        }
        // A shell may spell it either way; the tag comes back canonical.
        assert_eq!(Locale::parse("DE_de").unwrap().tag(), "de-DE");
        // ODF's "no country" keeps its own spelling.
        assert_eq!(Locale::parse("zxx-none").unwrap().country, "none");
    }

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
    fn the_environment_outranks_the_config_file() {
        let dir = std::env::temp_dir().join(format!("sheet-locale-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sheet")).unwrap();
        std::fs::write(dir.join("sheet/locale"), "fr-FR\n").unwrap();

        // SAFETY: this test owns these two variables for its duration, restores them before
        // returning, and nothing else in this binary reads them.
        unsafe {
            std::env::remove_var("GRIND_LOCALE");
            std::env::set_var("XDG_CONFIG_HOME", &dir);
        }
        assert_eq!(from_environment(), Locale::parse("fr-FR"));

        unsafe {
            std::env::set_var("GRIND_LOCALE", "de-DE");
        }
        assert_eq!(from_environment(), Locale::parse("de-DE"));

        unsafe {
            std::env::remove_var("GRIND_LOCALE");
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every locale a picker offers renders with separators this table decides, parses back
    /// from its own tag, and names itself.
    #[test]
    fn every_known_locale_is_one_this_table_gets_right_and_names() {
        let mut names: Vec<&str> = KNOWN.iter().map(|(_, name)| *name).collect();
        let sorted = {
            let mut s = names.clone();
            s.sort_unstable();
            s
        };
        assert_eq!(
            names, sorted,
            "KNOWN is read as a list, so it is sorted by name"
        );
        names.dedup();
        assert_eq!(names.len(), KNOWN.len());
        for (tag, name) in KNOWN {
            let locale = Locale::parse(tag).unwrap_or_else(|| panic!("{tag}"));
            assert_eq!(locale.tag(), tag);
            assert_eq!(locale.name(), Some(name));
        }
        assert_eq!(
            Locale::new("de", "CH").name(),
            None,
            "Switzerland groups with ’"
        );
        let sv = Locale::new("sv", "SE");
        assert_eq!((sv.decimal(), sv.group()), (',', '\u{a0}'));
        let tr = Locale::new("tr", "TR");
        assert_eq!((tr.decimal(), tr.group()), (',', '.'));
    }

    #[test]
    fn a_posix_locale_name_reads_as_a_locale_and_c_as_none() {
        assert_eq!(from_posix("de_DE.UTF-8"), Locale::parse("de-DE"));
        assert_eq!(from_posix("sv_SE@euro"), Locale::parse("sv-SE"));
        assert_eq!(from_posix("fr"), Locale::parse("fr"));
        assert_eq!(from_posix("C.UTF-8"), None);
        assert_eq!(from_posix("POSIX"), None);
        assert_eq!(from_posix(""), None);
    }

    #[test]
    fn the_sample_shows_both_separators_and_the_badge_names_the_locale() {
        assert_eq!(sample(None), "1,234.5");
        let de = Locale::new("de", "DE");
        assert_eq!(sample(Some(&de)), "1.234,5");
        assert_eq!(badge(Some(&de)), "1.234,5 \u{00b7} de-DE");
        assert_eq!(badge(None), "1,234.5");
        assert_eq!(sample(Some(&Locale::new("fr", "FR"))), "1\u{a0}234,5");
        assert_eq!(separator_name(','), "comma");
        assert_eq!(separator_name('\u{a0}'), "space");
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
