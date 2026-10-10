// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spelling's generic half (`doc/spelling.md`): what a word is, which words are worth asking a
//! dictionary about, and the [`Lexicon`] a dictionary is handed in through.
//!
//! Hoisted out of `grind_text::spell` the day the spreadsheet wanted the same answers, the way
//! `search::score` came out of a shell: a word in a cell is a word in a paragraph, and two
//! applications deciding differently what `ODF` or `e.g.` is would underline different things in
//! the same suite. What stays with each application is where its words *are* — a paragraph's
//! runs, some of them a link or code; a sheet's text cells — and the address a misspelling is
//! reported at (R8).
//!
//! **No dictionary lives here.** A [`Lexicon`] is handed in, the way a `layout::Metrics` is: this
//! crate decides *what a word is* and *which ones are checked*, and `grind-spell` decides whether
//! a word is spelled right. That keeps the word lists out of every build that only reads
//! documents.
//!
//! **What is checked is deliberately narrower than "every word".** A spell checker earns its
//! place by being believed, and it stops being believed the third time it underlines `ODF`. So a
//! word is skipped when it is plainly not prose: it has a digit in it, it is all capitals (an
//! acronym), it has a capital inside it (`OpenDocument`, `iPhone`, an identifier), it is one
//! letter long, or it sits in a chunk with `://`, `@`, `/`, `\`, `_` or `=` in it, which is a
//! URL, an address, a path or a piece of code. Each of those is a named rule in [`checkable`]
//! and [`prose_chunk`], with a test.

use std::ops::Range;
use std::sync::Arc;

/// A dictionary: whether a word is spelled right, and what it might have been instead.
///
/// `Send + Sync` because an application's `App` holds one behind its lock and a shell shares the
/// `App` between threads. Implemented by `grind-spell` over Hunspell dictionaries, and by a plain
/// word list in tests.
pub trait Lexicon: Send + Sync {
    /// Whether `word` is spelled right. Asked for every word [`words`] finds, so it must be cheap.
    fn knows(&self, word: &str) -> bool;
    /// What `word` might have been, best first. Asked for one word at a time — when somebody
    /// right-clicks it, or `grind text spell --suggest` names it — and may be slow.
    fn suggest(&self, word: &str) -> Vec<String>;
    /// The BCP 47 tag of the language this lexicon checks (`en-US`, `de-DE`), for messages.
    fn language(&self) -> &str;
}

/// A document spelling can be checked in — each application's `App`. What `grind-spell` needs
/// to choose a language for one and attach a dictionary to it, without knowing which kind of
/// document it is (R8 in the other direction: the dictionaries crate names no application).
pub trait Spelled: Send + Sync {
    /// Check spelling against `lexicon` from now on, or with `None` stop checking.
    fn set_lexicon(&self, lexicon: Option<Arc<dyn Lexicon>>);
    /// The attached lexicon's language, or `None` when nothing is checked.
    fn spelling_language(&self) -> Option<String>;
    /// The language the document **states**, as a BCP 47 tag — `zxx` for "no language" — or
    /// `None` when it states none.
    fn stated_language(&self) -> Option<String>;
    /// The first [`GUESS_SAMPLE`] words that would be checked, in document order — what a guess
    /// at the language reads.
    fn sample(&self) -> Vec<String>;
}

/// A shell holds its `App` in an `Arc` to share it with observers, and hands that over as it is.
impl<T: Spelled + ?Sized> Spelled for Arc<T> {
    fn set_lexicon(&self, lexicon: Option<Arc<dyn Lexicon>>) {
        (**self).set_lexicon(lexicon);
    }

    fn spelling_language(&self) -> Option<String> {
        (**self).spelling_language()
    }

    fn stated_language(&self) -> Option<String> {
        (**self).stated_language()
    }

    fn sample(&self) -> Vec<String> {
        (**self).sample()
    }
}

/// The apostrophes a word may contain: the typewriter one and the typographer's `’`, which is
/// what a word processor's own smart quotes type. A dictionary spells `don't` with the first, so
/// [`normalise`] folds the second onto it before asking.
fn apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}

/// Every candidate word in `text`, as a `[start, end)` *character* range.
///
/// A word is a run of alphanumeric characters (so a letter with a combining mark, an umlaut, an
/// `ß` are all inside it), joined across a single apostrophe or hyphen that has a letter on both
/// sides — `don't`, `l'eau`, `E-Mail-Adresse`. Digits are kept inside a word so that [`checkable`]
/// can see them and skip the whole thing: `mp3` is not `mp` and a stray `3`.
pub fn words(text: &str) -> Vec<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if !chars[at].is_alphanumeric() {
            at += 1;
            continue;
        }
        let start = at;
        loop {
            while at < chars.len() && chars[at].is_alphanumeric() {
                at += 1;
            }
            let joins = at + 1 < chars.len()
                && (apostrophe(chars[at]) || chars[at] == '-')
                && chars[at - 1].is_alphabetic()
                && chars[at + 1].is_alphabetic();
            match joins {
                true => at += 1,
                false => break,
            }
        }
        out.push(start..at);
    }
    out
}

/// Whether a word is one a dictionary should be asked about at all — the "plainly not prose"
/// rules from the module's own documentation.
pub fn checkable(word: &str) -> bool {
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 2 || word.chars().any(|c| c.is_numeric()) {
        return false;
    }
    // An acronym: `ODF`, `PDF`, `GTK`, and `PDFs`' stem — two capitals or more and no lowercase.
    if word
        .chars()
        .filter(|c| c.is_alphabetic())
        .all(char::is_uppercase)
    {
        return false;
    }
    // A capital after the first letter of any hyphen-separated part: `OpenDocument`, `iPhone`,
    // `McDonald`. A name or an identifier, and a dictionary has neither.
    !word
        .split('-')
        .any(|part| part.chars().skip(1).any(char::is_uppercase))
}

/// The form a dictionary is asked about: `’` folded to `'`.
pub fn normalise(word: &str) -> String {
    word.chars()
        .map(|c| if apostrophe(c) { '\'' } else { c })
        .collect()
}

/// Whether `lexicon` accepts `word`, trying the whole word first and then, for a hyphenated one,
/// every part — `well-known` is in an English dictionary and `Rechtschreib-Prüfung` is two German
/// words, and neither should be underlined. A one-letter part (`E-Mail`) is accepted on its own.
pub fn known(lexicon: &dyn Lexicon, word: &str) -> bool {
    let word = normalise(word);
    lexicon.knows(&word)
        || (word.contains('-')
            && word
                .split('-')
                .all(|part| part.chars().count() < 2 || lexicon.knows(part)))
}

/// Whether the whitespace-delimited chunk around a word is prose: not a URL, an e-mail address, a
/// path or a piece of code. `e.g.` and `lib.rs` are caught by an inner `.` with a letter after it.
pub fn prose_chunk(chunk: &str) -> bool {
    let chars: Vec<char> = chunk.chars().collect();
    let inner_dot = chars
        .windows(2)
        .any(|pair| pair[0] == '.' && pair[1].is_alphanumeric());
    !inner_dot
        && !chunk.contains("://")
        && !chunk
            .chars()
            .any(|c| matches!(c, '@' | '/' | '\\' | '_' | '='))
}

/// Every word of `text` that should be checked, as `(range, word)` in characters — [`words`],
/// less what [`checkable`] and [`prose_chunk`] rule out, and less any word overlapping one of the
/// `skip` ranges (what an application knows is not prose: a link, a run of code).
pub fn candidates(text: &str, skip: &[Range<usize>]) -> Vec<(Range<usize>, String)> {
    let chars: Vec<char> = text.chars().collect();
    words(text)
        .into_iter()
        .filter(|range| {
            !skip
                .iter()
                .any(|s| s.start < range.end && range.start < s.end)
        })
        .filter_map(|range| {
            let word: String = chars[range.clone()].iter().collect();
            let mut from = range.start;
            while from > 0 && !chars[from - 1].is_whitespace() {
                from -= 1;
            }
            let mut to = range.end;
            while to < chars.len() && !chars[to].is_whitespace() {
                to += 1;
            }
            let chunk: String = chars[from..to].iter().collect();
            (checkable(&word) && prose_chunk(&chunk)).then_some((range, word))
        })
        .collect()
}

/// The words of `text` that `lexicon` does not know, as `(range, word)` in characters.
pub fn unknown(
    text: &str,
    skip: &[Range<usize>],
    lexicon: &dyn Lexicon,
) -> Vec<(Range<usize>, String)> {
    candidates(text, skip)
        .into_iter()
        .filter(|(_, word)| !known(lexicon, word))
        .collect()
}

/// How many words a guess at the language reads before it decides — enough to tell German from
/// English with a wide margin, few enough that opening a book does not check all of it twice.
pub const GUESS_SAMPLE: usize = 400;

/// Which of `lexicons` knows the most of `sample` — a guess at the language of a document that
/// does not state one. `None` when there are no words to judge by or no lexicon knows any of
/// them; a tie goes to the earlier lexicon, so the caller's order is its preference.
pub fn guess(sample: &[String], lexicons: &[&dyn Lexicon]) -> Option<usize> {
    let scores: Vec<usize> = lexicons
        .iter()
        .map(|lexicon| sample.iter().filter(|w| known(*lexicon, w)).count())
        .collect();
    let best = *scores.iter().max()?;
    (best > 0).then(|| scores.iter().position(|s| *s == best).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lexicon over a fixed list, lower-casing the way Hunspell accepts a capitalised word at
    /// the start of a sentence.
    struct List(&'static [&'static str]);

    impl Lexicon for List {
        fn knows(&self, word: &str) -> bool {
            self.0.contains(&word) || self.0.contains(&word.to_lowercase().as_str())
        }
        fn suggest(&self, _: &str) -> Vec<String> {
            Vec::new()
        }
        fn language(&self) -> &str {
            "en-US"
        }
    }

    fn found(text: &str) -> Vec<String> {
        candidates(text, &[]).into_iter().map(|(_, w)| w).collect()
    }

    #[test]
    fn a_word_joins_across_one_apostrophe_or_hyphen_between_letters() {
        let s = "don't l’eau E-Mail-Adresse 'quoted' - dash";
        let got: Vec<String> = words(s)
            .into_iter()
            .map(|r| s.chars().skip(r.start).take(r.len()).collect())
            .collect();
        assert_eq!(got, ["don't", "l’eau", "E-Mail-Adresse", "quoted", "dash"]);
    }

    #[test]
    fn what_is_plainly_not_prose_is_never_checked() {
        for skipped in ["ODF", "PDFs", "OpenDocument", "iPhone", "mp3", "a", "x2"] {
            assert!(!checkable(skipped), "{skipped}");
        }
        for checked in ["hello", "Hello", "don't", "Straße", "well-known", "E-Mail"] {
            assert!(checkable(checked), "{checked}");
        }
    }

    #[test]
    fn urls_addresses_paths_and_code_are_not_prose() {
        assert_eq!(
            found(
                "see https://exmaple.org or me@exmaple.org in src/lib.rs, e.g. snake_case a=b ok"
            ),
            ["see", "or", "in", "ok"]
        );
    }

    #[test]
    fn a_skipped_range_takes_every_word_it_touches() {
        let text = "run cargoo and clik here";
        let got: Vec<String> = candidates(text, &[4..10, 15..19])
            .into_iter()
            .map(|(_, w)| w)
            .collect();
        assert_eq!(got, ["run", "and", "here"]);
    }

    #[test]
    fn an_unknown_word_is_where_it_is_counted_in_characters() {
        let list = List(&["i", "it", "receive", "grüße"]);
        assert_eq!(
            unknown("Grüße, I recieve it", &[], &list),
            [(9..16, "recieve".to_owned())]
        );
    }

    #[test]
    fn a_hyphenated_word_is_known_when_its_parts_are_and_curly_apostrophes_fold() {
        let list = List(&["well", "known", "don't", "mail"]);
        assert!(known(&list, "well-known"));
        assert!(known(&list, "E-Mail"));
        assert!(known(&list, "don’t"));
        assert!(!known(&list, "well-knwon"));
    }

    #[test]
    fn the_guess_is_the_lexicon_that_knows_the_most_words() {
        let english = List(&["the", "house", "is", "red"]);
        let german = List(&["das", "haus", "ist", "rot"]);
        let sample = found("Das Haus ist rot, the end");
        assert_eq!(guess(&sample, &[&english, &german]), Some(1));
        assert_eq!(guess(&found("123"), &[&english, &german]), None);
    }
}
