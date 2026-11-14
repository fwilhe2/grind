// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a word may be hyphenated (`doc/page-model.md` PM6 item 5): Liang's patterns, as
//! libhyphen's `.dic` files spell them — an encoding line, `LEFTHYPHENMIN` and `RIGHTHYPHENMIN`,
//! then one pattern a line, letters with the digits between them. A position takes the highest
//! digit any pattern puts there, and an odd one is a place to break.

use std::collections::HashMap;

/// One language's patterns.
#[derive(Debug)]
pub struct Patterns {
    /// Each pattern's letters, and the digit before each letter and after the last.
    values: HashMap<Box<str>, Box<[u8]>>,
    /// The fewest letters a break leaves before it and after it.
    left: usize,
    right: usize,
    /// The longest pattern, in letters.
    longest: usize,
}

impl Patterns {
    /// Read a `.dic` file. Its first level only: a `NEXTLEVEL` (compound words) ends what is
    /// read, and a non-standard pattern (one that changes letters, with `/`) is left out.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Patterns {
        let first = bytes
            .iter()
            .position(|&b| b == b'\n')
            .unwrap_or(bytes.len());
        let latin = !String::from_utf8_lossy(&bytes[..first])
            .trim()
            .eq_ignore_ascii_case("UTF-8");
        let text: String = if latin {
            bytes.iter().map(|&b| char::from(b)).collect()
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        };
        let mut patterns = Patterns {
            values: HashMap::new(),
            // The format's default when a file states none.
            left: 2,
            right: 2,
            longest: 0,
        };
        for line in text.lines().skip(1).map(str::trim) {
            let mut words = line.split_whitespace();
            match (
                words.next(),
                words.next().and_then(|n| n.parse::<usize>().ok()),
            ) {
                (Some("LEFTHYPHENMIN"), Some(n)) => patterns.left = n,
                (Some("RIGHTHYPHENMIN"), Some(n)) => patterns.right = n,
                (Some("NEXTLEVEL"), _) => break,
                _ if line.is_empty()
                    || line.starts_with('%')
                    || line.contains('/')
                    || line.chars().next().is_some_and(char::is_uppercase) => {}
                _ => patterns.add(line),
            }
        }
        patterns
    }

    fn add(&mut self, pattern: &str) {
        let mut letters = String::new();
        let mut digits = vec![0u8];
        for c in pattern.chars() {
            if let Some(d) = c.to_digit(10) {
                *digits.last_mut().expect("starts with one") = d as u8;
            } else {
                letters.push(c);
                digits.push(0);
            }
        }
        self.longest = self.longest.max(letters.chars().count());
        self.values.insert(letters.into(), digits.into());
    }

    /// Where `word` may break: each offset, in characters, a hyphen may go before, leaving at
    /// least `head` letters before it and `tail` after it — and at least what the patterns' own
    /// minimums ask.
    #[must_use]
    pub fn points(&self, word: &str, head: usize, tail: usize) -> Vec<usize> {
        let letters: Vec<char> = word.chars().flat_map(char::to_lowercase).collect();
        let len = letters.len();
        if len != word.chars().count() {
            return Vec::new();
        }
        let dotted: Vec<char> = std::iter::once('.')
            .chain(letters.iter().copied())
            .chain(std::iter::once('.'))
            .collect();
        let mut values = vec![0u8; dotted.len() + 1];
        let mut key = String::new();
        for start in 0..dotted.len() {
            key.clear();
            for &letter in dotted.iter().skip(start).take(self.longest) {
                key.push(letter);
                if let Some(digits) = self.values.get(key.as_str()) {
                    for (at, &digit) in digits.iter().enumerate() {
                        let value = &mut values[start + at];
                        *value = (*value).max(digit);
                    }
                }
            }
        }
        let (head, tail) = (head.max(self.left).max(1), tail.max(self.right).max(1));
        // A break before letter `p` of the word sits after `.` and `p` letters.
        (head..=len.saturating_sub(tail))
            .filter(|&p| values[p + 1] % 2 == 1)
            .collect()
    }
}

/// The bundled patterns for a language, by its ODF code (`fo:language`, `en`), whatever the
/// country; `None` for a language there are none for, and always in a build without them.
#[cfg(not(feature = "bundled-hyphenation"))]
#[must_use]
pub fn bundled(_language: &str) -> Option<&'static Patterns> {
    None
}

/// The bundled patterns for a language, by its ODF code (`fo:language`, `en`), whatever the
/// country; `None` for a language there are none for.
#[cfg(feature = "bundled-hyphenation")]
#[must_use]
pub fn bundled(language: &str) -> Option<&'static Patterns> {
    use std::sync::OnceLock;
    static EN: OnceLock<Patterns> = OnceLock::new();
    static DE: OnceLock<Patterns> = OnceLock::new();
    let language = language.split(['-', '_']).next()?.to_ascii_lowercase();
    match language.as_str() {
        "en" => Some(
            EN.get_or_init(|| Patterns::parse(include_bytes!("../hyphenation/hyph_en_US.dic"))),
        ),
        "de" => Some(
            DE.get_or_init(|| Patterns::parse(include_bytes!("../hyphenation/hyph_de_DE.dic"))),
        ),
        _ => None,
    }
}

#[cfg(all(test, feature = "bundled-hyphenation"))]
mod tests {
    use super::*;

    fn split(patterns: &Patterns, word: &str, head: usize, tail: usize) -> String {
        let points = patterns.points(word, head, tail);
        let mut out = String::new();
        for (at, c) in word.chars().enumerate() {
            if points.contains(&at) {
                out.push('-');
            }
            out.push(c);
        }
        out
    }

    /// The breaks Writer made in `doc/odt-format.md` §5c fact 55's document are among these.
    #[test]
    fn english_words_break_where_writer_broke_them() {
        let en = bundled("en").expect("English is bundled");
        for (word, writer) in [
            ("characteristics", "characteris-tics"),
            ("extraordinary", "extraordi-nary"),
            ("considerations", "considera-tions"),
            ("incomprehensibilities", "in-comprehensibilities"),
            ("accumulate", "accumu-late"),
            ("accumulate", "accu-mulate"),
        ] {
            let points = en.points(word, 0, 0);
            let at = writer.find('-').unwrap();
            assert!(
                points.contains(&at),
                "{} has no break at {at}: {}",
                word,
                split(en, word, 0, 0)
            );
        }
        assert_eq!(split(en, "hyphenation", 0, 0), "hy-phen-ation");
    }

    #[test]
    fn the_counts_keep_letters_on_either_side() {
        let en = bundled("en-US").unwrap();
        assert!(en.points("incomprehensibilities", 0, 0).contains(&2));
        assert!(!en.points("incomprehensibilities", 4, 4).contains(&2));
        // The file's own minimum: three letters after the last break.
        assert!(en.points("hyphenation", 0, 0).iter().all(|&p| 11 - p >= 3));
    }

    #[test]
    fn german_patterns_are_read_from_latin_1() {
        let de = bundled("de").expect("German is bundled");
        let points = de.points("Silbentrennung", 0, 0);
        assert!(!points.is_empty(), "{}", split(de, "Silbentrennung", 0, 0));
        assert!(
            !de.points("Größenänderung", 0, 0).is_empty(),
            "umlauts read"
        );
        assert!(bundled("fr").is_none());
    }
}
