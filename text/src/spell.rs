// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spelling in a text document (`doc/spelling.md`) — where a block's words are, which of its
//! runs are not prose, and the address a misspelling is reported at.
//!
//! What a word is and which words are checked at all is `grind_core::spell`'s, shared with the
//! spreadsheet, and re-exported here so this crate's callers reach it by the paths they always
//! used. What only a word processor has is a *run*: a link's text and a run set in a monospace
//! face (`` `code` ``) are not prose, and neither is a field or a footnote kept as the text it
//! shows.

use std::ops::Range;

pub use grind_core::spell::{
    GUESS_SAMPLE, Lexicon, checkable, known, normalise, prose_chunk, words,
};

use crate::model::{Block, Run};
use crate::style::CharStyle;

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

/// Every word of one block that should be checked, as `(range, word)` — the core's
/// [`grind_core::spell::candidates`], less the block's own non-prose runs.
pub fn candidates(block: &Block) -> Vec<(Range<usize>, String)> {
    grind_core::spell::candidates(&block.text(), &skipped(block))
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

/// The first [`GUESS_SAMPLE`] checkable words of `blocks` — what a guess at the language reads
/// (`grind_core::spell::guess`).
pub fn sample<'a>(blocks: impl IntoIterator<Item = &'a Block>) -> Vec<String> {
    blocks
        .into_iter()
        .flat_map(|block| candidates(block).into_iter().map(|(_, word)| word))
        .take(GUESS_SAMPLE)
        .collect()
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
