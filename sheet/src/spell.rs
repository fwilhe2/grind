// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spelling in a spreadsheet (`doc/spelling.md`, "In a spreadsheet") — which cells are prose,
//! and where a misspelling is.
//!
//! What a word is and which words are checked at all is `grind_core::spell`'s, the same rules
//! the word processor underlines by, re-exported here. What is the spreadsheet's own is **which
//! cells**: a cell holding text somebody typed — a heading, a label, a note — and nothing else.
//! A number, a date and a boolean have no words; a formula's result is what the formula
//! computed, and underlining `#N/A`'s neighbour or a `CONCATENATE` would be underlining the
//! data rather than the writing. So a text cell with a formula behind it is never checked.

pub use grind_core::spell::{GUESS_SAMPLE, Lexicon, normalise};

use crate::a1;
use crate::model::{Document, Pos, Sheet};

/// One word a [`Lexicon`] does not know, and the cell it is in.
///
/// `offset` and `len` count characters into the cell's own text — what [`crate::App::input_text`]
/// shows for a text cell, and what [`crate::App::correct`] takes back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    pub sheet: usize,
    pub pos: Pos,
    pub offset: usize,
    pub len: usize,
    pub word: String,
}

impl Misspelling {
    /// `Sheet1.B12` — the cell, as `grind sheet get`, a go-to box and a problems pane take it.
    pub fn address(&self, sheet_name: &str) -> String {
        a1::format(Some(sheet_name), self.pos)
    }
}

/// The words of `text` that `lexicon` does not know, as `(range, word)` in characters. A cell
/// has nothing in it the way a paragraph has a link or a run of code, so nothing is skipped
/// beyond what `grind_core::spell` already skips — a URL, an address, a path, an identifier.
pub fn unknown(text: &str, lexicon: &dyn Lexicon) -> Vec<(std::ops::Range<usize>, String)> {
    grind_core::spell::unknown(text, &[], lexicon)
}

/// Every misspelt word on one sheet, in reading order.
pub fn check_sheet(sheet: usize, s: &Sheet, lexicon: &dyn Lexicon) -> Vec<Misspelling> {
    s.text_cells()
        .into_iter()
        .flat_map(|(pos, text)| {
            unknown(&text, lexicon)
                .into_iter()
                .map(move |(range, word)| Misspelling {
                    sheet,
                    pos,
                    offset: range.start,
                    len: range.len(),
                    word,
                })
        })
        .collect()
}

/// The first [`GUESS_SAMPLE`] checkable words of the document, sheet by sheet in reading order —
/// what a guess at its language reads.
pub fn sample(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    for s in &doc.sheets {
        for (_, text) in s.text_cells() {
            for (_, word) in grind_core::spell::candidates(&text, &[]) {
                out.push(word);
                if out.len() == GUESS_SAMPLE {
                    return out;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, CellValue, RecalcMode};
    use std::sync::Arc;

    /// A lexicon over a fixed list, lower-casing the way Hunspell accepts a capitalised word at
    /// the start of a sentence.
    struct List(&'static [&'static str]);

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

    const WORDS: &[&str] = &["total", "received", "receive", "we", "the", "goods"];

    fn app(cells: &[(&str, &str)]) -> App {
        let app = App::new();
        for (address, input) in cells {
            app.enter(0, at(address), input, RecalcMode::Document)
                .unwrap();
        }
        app
    }

    fn at(a: &str) -> Pos {
        let app = App::new();
        crate::a1::resolve(&app, &crate::a1::parse(a).unwrap())
            .unwrap()
            .1
    }

    #[test]
    fn only_typed_text_is_checked_never_a_number_a_date_or_a_formula() {
        let app = app(&[
            ("A1", "Totl recieved"),
            ("A2", "42"),
            ("A3", "2026-10-10"),
            ("A4", "=\"wrogn\""),
            ("B1", "We recieve the goods"),
            ("B2", "ODF https://exmaple.org e.g."),
        ]);
        assert!(
            app.misspellings(None).unwrap().is_empty(),
            "nothing attached"
        );
        app.set_lexicon(Some(Arc::new(List(WORDS))));
        let found: Vec<(String, usize, String)> = app
            .misspellings(None)
            .unwrap()
            .into_iter()
            .map(|m| (m.address("Sheet1"), m.offset, m.word))
            .collect();
        assert_eq!(
            found,
            [
                ("Sheet1.A1".to_owned(), 0, "Totl".to_owned()),
                ("Sheet1.A1".to_owned(), 5, "recieved".to_owned()),
                ("Sheet1.B1".to_owned(), 3, "recieve".to_owned()),
            ]
        );
        assert_eq!(app.suggest("recieve"), ["receive"]);
        let report = app.lint(&grind_core::lint::Options::default());
        let misspelt: Vec<&str> = report
            .diagnostics
            .iter()
            .filter(|d| d.rule == "misspelt")
            .map(|d| d.at.as_str())
            .collect();
        assert_eq!(misspelt, ["Sheet1.A1", "Sheet1.A1", "Sheet1.B1"]);
    }

    #[test]
    fn the_viewport_carries_the_ranges_a_shell_underlines_and_nothing_when_off() {
        let app = app(&[("A1", "Totl recieved"), ("B1", "the goods")]);
        let view = |app: &App| app.get_viewport(0, 0..2, 0..2).unwrap();
        assert!(view(&app).misspelt(0, 0).is_empty());
        app.set_lexicon(Some(Arc::new(List(WORDS))));
        assert_eq!(view(&app).misspelt(0, 0), [0..4, 5..13]);
        assert!(view(&app).misspelt(0, 1).is_empty());
        app.set_lexicon(None);
        assert!(view(&app).misspelt(0, 0).is_empty(), "off is off");
        assert_eq!(app.spelling_language(), None);
    }

    #[test]
    fn a_correction_is_one_undo_stays_text_and_refuses_a_stale_word() {
        let app = app(&[("A1", "We recieve 12"), ("B1", "=LEN([.A1])")]);
        app.set_lexicon(Some(Arc::new(List(WORDS))));
        assert!(app.correct(0, at("A1"), 3, "recieved", "received").is_err());
        assert!(app.correct(0, at("B1"), 0, "x", "y").is_err(), "not text");
        app.correct(0, at("A1"), 3, "recieve", "receive").unwrap();
        assert_eq!(
            app.get(0, at("A1")).unwrap(),
            CellValue::Text("We receive 12".to_owned())
        );
        assert_eq!(app.get(0, at("B1")).unwrap(), CellValue::Number(13.0));
        app.undo();
        assert_eq!(
            app.get(0, at("A1")).unwrap(),
            CellValue::Text("We recieve 12".to_owned())
        );
        // A correction that looks like a number stays the text it was typed as.
        let app = app_with_text("Twelv");
        app.correct(0, at("A1"), 0, "Twelv", "12").unwrap();
        assert_eq!(
            app.get(0, at("A1")).unwrap(),
            CellValue::Text("12".to_owned())
        );
    }

    fn app_with_text(text: &str) -> App {
        app(&[("A1", text)])
    }

    #[test]
    fn the_stated_language_is_the_documents_locale_and_the_sample_reads_text_cells() {
        use grind_core::spell::Spelled;
        let app = app(&[("A1", "Das Haus"), ("A2", "7"), ("B1", "ist rot")]);
        assert_eq!(app.stated_language(), None);
        assert_eq!(app.sample(), ["Das", "Haus", "ist", "rot"]);
        app.set_locale(crate::locale::Locale::parse("de-DE"))
            .unwrap();
        assert_eq!(app.stated_language().as_deref(), Some("de-DE"));
    }
}
