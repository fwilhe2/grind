// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spelling — which words of a block are worth asking a dictionary about, and which of those it
//! does not know (`doc/spelling.md`).
//!
//! **No dictionary lives here.** A [`Lexicon`] is handed in, the way a [`crate::Metrics`] is: this
//! crate decides *what a word is* and *which ones are checked*, and `grind-spell` decides whether
//! a word is spelled right. That keeps two megabytes of word lists out of every build that only
//! reads documents — the terminal, the browser, a `grind` built without `spell` — and it keeps the
//! rules that decide what gets underlined in one place, so four shells cannot underline four
//! different sets of words.
//!
//! **What is checked is deliberately narrower than "every word".** A spell checker earns its
//! place by being believed, and it stops being believed the third time it underlines `ODF`. So a
//! word is skipped when it is plainly not prose: it has a digit in it, it is all capitals (an
//! acronym), it has a capital inside it (`OpenDocument`, `iPhone`, an identifier), it is one
//! letter long, or it sits in something that is not a sentence — a link's text, a run set in a
//! monospace face (`` `code` ``), or a chunk with `://`, `@`, `/`, `\`, `_` or `=` in it, which
//! is a URL, an address, a path or a piece of code. Each of those is a named rule in
//! [`checkable`] and `prose_chunk`, with a test.

use std::ops::Range;

use crate::model::{Block, Run};
use crate::style::CharStyle;

/// A dictionary: whether a word is spelled right, and what it might have been instead.
///
/// `Send + Sync` because an [`crate::App`] holds one behind its lock and a shell shares the `App`
/// between threads. Implemented by `grind-spell` over Hunspell dictionaries, and by a plain word
/// list in this crate's tests.
pub trait Lexicon: Send + Sync {
    /// Whether `word` is spelled right. Asked for every word [`words`] finds, so it must be cheap.
    fn knows(&self, word: &str) -> bool;
    /// What `word` might have been, best first. Asked for one word at a time — when somebody
    /// right-clicks it, or `grind text spell --suggest` names it — and may be slow.
    fn suggest(&self, word: &str) -> Vec<String>;
    /// The BCP 47 tag of the language this lexicon checks (`en-US`, `de-DE`), for messages.
    fn language(&self) -> &str;
}

/// One word a [`Lexicon`] does not know, where it is.
///
/// `offset` and `len` count characters, as every offset in `loc.rs` does, so `p12+40` is where it
/// starts and a shell selects `[offset, offset + len)` to replace it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    pub block: usize,
    pub offset: usize,
    pub len: usize,
    pub word: String,
}

impl Misspelling {
    /// `p12+40` — the address `grind text get`, a go-to box and a problems pane all take.
    pub fn address(&self) -> String {
        format!("{}+{}", crate::loc::format(self.block), self.offset)
    }

    /// Where the word starts — what [`crate::find::step`] steps between, so "the next
    /// misspelling" wraps the way "the next match" does.
    pub fn caret(&self) -> crate::Caret {
        crate::Caret {
            block: self.block,
            offset: self.offset,
        }
    }

    /// Whether this is the word somebody is still typing: the caret at its end. Every shell
    /// leaves that one alone while nothing is selected, as every word processor waits.
    pub fn being_typed(&self, caret: crate::Caret) -> bool {
        caret.block == self.block && caret.offset == self.offset + self.len
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
fn prose_chunk(chunk: &str) -> bool {
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

/// Whether a run of text is set as something other than prose: a link's text, or a run in a
/// monospace face — what the `` `code` `` notation types, or a family that says it is one.
fn prose_run(props: &CharStyle, href: Option<&str>) -> bool {
    if href.is_some() {
        return false;
    }
    let Some(family) = props.font_family.as_deref() else {
        return true;
    };
    let family = family.to_ascii_lowercase();
    !(family == crate::markdown::MONOSPACE
        || family.contains("mono")
        || family.contains("courier")
        || family.contains("code")
        || family.contains("consol"))
}

/// The character ranges of `block` that are not prose: every run that [`prose_run`] refuses, and
/// every run that is not text at all (a field, a footnote kept as the text it shows).
fn skipped(block: &Block) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut at = 0;
    for run in &block.runs {
        let len = run.len();
        let prose = match run {
            Run::Text { props, href, .. } => prose_run(props, href.as_deref()),
            Run::Kept { .. } => false,
            _ => true,
        };
        if !prose {
            out.push(at..at + len);
        }
        at += len;
    }
    out
}

/// Every word of one block that should be checked, as `(range, word)` — [`words`], less what
/// [`checkable`], `prose_chunk` and the block's own non-prose runs rule out.
pub fn candidates(block: &Block) -> Vec<(Range<usize>, String)> {
    let text: Vec<char> = block.text().chars().collect();
    let skip = skipped(block);
    let joined: String = text.iter().collect();
    words(&joined)
        .into_iter()
        .filter(|range| {
            !skip
                .iter()
                .any(|s| s.start < range.end && range.start < s.end)
        })
        .filter_map(|range| {
            let word: String = text[range.clone()].iter().collect();
            let mut from = range.start;
            while from > 0 && !text[from - 1].is_whitespace() {
                from -= 1;
            }
            let mut to = range.end;
            while to < text.len() && !text[to].is_whitespace() {
                to += 1;
            }
            let chunk: String = text[from..to].iter().collect();
            (checkable(&word) && prose_chunk(&chunk)).then_some((range, word))
        })
        .collect()
}

/// Every word of `block` that `lexicon` does not know.
pub fn check_block(block: &Block, index: usize, lexicon: &dyn Lexicon) -> Vec<Misspelling> {
    candidates(block)
        .into_iter()
        .filter(|(_, word)| !known(lexicon, word))
        .map(|(range, word)| Misspelling {
            block: index,
            offset: range.start,
            len: range.len(),
            word,
        })
        .collect()
}

/// How many words a guess at the language reads before it decides — enough to tell German from
/// English with a wide margin, few enough that opening a book does not check all of it twice.
pub const GUESS_SAMPLE: usize = 400;

/// Which of `lexicons` knows the most of the first [`GUESS_SAMPLE`] checkable words of `blocks`
/// — a guess at the language of a document that does not state one. `None` when there are no
/// words to judge by or no lexicon knows any of them; a tie goes to the earlier lexicon, so the
/// caller's order is its preference.
pub fn guess<'a>(
    blocks: impl IntoIterator<Item = &'a Block>,
    lexicons: &[&dyn Lexicon],
) -> Option<usize> {
    let sample: Vec<String> = blocks
        .into_iter()
        .flat_map(|block| candidates(block).into_iter().map(|(_, word)| word))
        .take(GUESS_SAMPLE)
        .collect();
    let scores: Vec<usize> = lexicons
        .iter()
        .map(|lexicon| sample.iter().filter(|w| known(*lexicon, w)).count())
        .collect();
    let best = *scores.iter().max()?;
    (best > 0).then(|| scores.iter().position(|s| *s == best).unwrap())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{BlockId, BlockKind};

    /// A lexicon over a fixed list, lower-casing the way Hunspell accepts a capitalised word at
    /// the start of a sentence.
    pub(crate) struct List(pub &'static [&'static str]);

    impl Lexicon for List {
        fn knows(&self, word: &str) -> bool {
            self.0.contains(&word) || self.0.contains(&word.to_lowercase().as_str())
        }
        fn suggest(&self, word: &str) -> Vec<String> {
            self.0
                .iter()
                .filter(|w| w.len() == word.len() && w.chars().next() == word.chars().next())
                .map(|w| (*w).to_owned())
                .collect()
        }
        fn language(&self) -> &str {
            "en-US"
        }
    }

    fn block(runs: Vec<Run>) -> Block {
        let mut block = Block::new(BlockId(0), BlockKind::Paragraph);
        block.runs = runs;
        block
    }

    fn text(s: &str) -> Run {
        Run::Text {
            text: s.to_owned(),
            style: None,
            props: CharStyle::default(),
            href: None,
        }
    }

    fn found(b: &Block) -> Vec<String> {
        candidates(b).into_iter().map(|(_, w)| w).collect()
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
        let b = block(vec![text(
            "see https://exmaple.org or me@exmaple.org in src/lib.rs, e.g. snake_case a=b ok",
        )]);
        assert_eq!(found(&b), ["see", "or", "in", "ok"]);
    }

    #[test]
    fn a_link_and_a_monospace_run_are_skipped_and_the_rest_of_the_block_is_not() {
        let code = CharStyle {
            font_family: Some(crate::markdown::MONOSPACE.to_owned()),
            ..CharStyle::default()
        };
        let b = block(vec![
            text("run "),
            Run::Text {
                text: "cargoo".to_owned(),
                style: None,
                props: code,
                href: None,
            },
            text(" and "),
            Run::Text {
                text: "clik".to_owned(),
                style: None,
                props: CharStyle::default(),
                href: Some("https://x.test".to_owned()),
            },
            Run::Tab,
            text("here"),
        ]);
        assert_eq!(found(&b), ["run", "and", "here"]);
    }

    #[test]
    fn a_misspelling_is_where_the_word_is_counted_in_characters() {
        let b = block(vec![text("Grüße, I recieve it")]);
        let list = List(&["i", "it", "receive", "grüße"]);
        let got = check_block(&b, 4, &list);
        assert_eq!(
            got,
            [Misspelling {
                block: 4,
                offset: 9,
                len: 7,
                word: "recieve".to_owned(),
            }]
        );
        assert_eq!(got[0].address(), "p5+9");
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
        let doc = [block(vec![text("Das Haus ist rot, the end")])];
        assert_eq!(guess(&doc, &[&english, &german]), Some(1));
        assert_eq!(
            guess(&[block(vec![text("123")])], &[&english, &german]),
            None
        );
    }

    #[test]
    fn a_correction_keeps_the_words_formatting_and_is_one_undo() {
        let app = crate::App::new();
        app.insert(0, BlockKind::Paragraph, "I recieve it").unwrap();
        let at = crate::Caret {
            block: 0,
            offset: 2,
        };
        let bold = CharStyle {
            font_weight: Some("bold".to_owned()),
            ..CharStyle::default()
        };
        let end = crate::Caret { offset: 9, ..at };
        app.set_char_style(at, end, &bold).unwrap();
        assert!(
            app.correct(at, "recieved", "received").is_err(),
            "a stale word is refused"
        );
        app.correct(at, "recieve", "receive").unwrap();
        assert_eq!(app.input_text(0).unwrap(), "I receive it");
        assert_eq!(
            app.char_style(at, end).unwrap().font_weight.as_deref(),
            Some("bold")
        );
        app.undo();
        assert_eq!(app.input_text(0).unwrap(), "I recieve it");
    }

    #[test]
    fn the_app_checks_only_once_a_lexicon_is_attached() {
        let app = crate::App::new();
        app.insert(0, BlockKind::Paragraph, "I recieve it").unwrap();
        assert!(app.misspellings(0..10).is_empty());
        assert_eq!(app.spelling_language(), None);
        app.set_lexicon(Some(std::sync::Arc::new(List(&["i", "it", "receive"]))));
        assert_eq!(app.spelling_language().as_deref(), Some("en-US"));
        let found = app.misspellings(0..10);
        assert_eq!(found.len(), 1);
        assert_eq!(app.suggest("recieve"), ["receive"]);
        assert_eq!(
            app.lint(&grind_core::lint::Options::default()).diagnostics[0].rule,
            "misspelt"
        );
    }

    #[test]
    fn the_stated_language_is_the_standard_styles_over_the_default() {
        let fodt = |styles: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:styles>{styles}</office:styles>
 <office:body><office:text><text:p>Hallo</text:p></office:text></office:body>
</office:document>"#
            )
        };
        let app = crate::App::new();
        app.open_bytes("a.fodt", fodt("").as_bytes()).unwrap();
        assert_eq!(app.language(), None);
        let default = r#"<style:default-style style:family="paragraph"><style:text-properties fo:language="de" fo:country="DE"/></style:default-style>"#;
        app.open_bytes("a.fodt", fodt(default).as_bytes()).unwrap();
        assert_eq!(app.language().as_deref(), Some("de-DE"));
        let standard = format!(
            r#"{default}<style:style style:name="Standard" style:family="paragraph"><style:text-properties fo:language="en" fo:country="GB"/></style:style>"#
        );
        app.open_bytes("a.fodt", fodt(&standard).as_bytes())
            .unwrap();
        assert_eq!(app.language().as_deref(), Some("en-GB"));
    }
}
