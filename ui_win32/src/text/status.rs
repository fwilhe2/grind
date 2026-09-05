// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the text pane's status bar says — a pure function, tested on any host.
//!
//! `sheet/status.rs`'s counterpart, and the same rule applies: the sentence is built here from
//! numbers the core already answers, so that nothing about it is decided inside a paint. The
//! spreadsheet's bar carries Sum/Count/Average over the selection; this one carries where the
//! caret is and how big the document is, which are the two questions a word processor's bar
//! answers everywhere.

use grind_text::Counts;

/// The status line: where the caret is, then the document's size.
///
/// `selected` is how many characters the selection covers, and it is only mentioned when there
/// **is** one — a bar that permanently says "0 selected" is a bar nobody reads.
pub fn status_line(address: &str, selected: usize, counts: Counts) -> String {
    let mut parts = vec![address.to_owned()];
    if selected > 0 {
        parts.push(format!("{selected} selected"));
    }
    parts.push(format!(
        "{} {}",
        counts.words,
        plural(counts.words, "word", "words")
    ));
    parts.push(format!(
        "{} {}",
        counts.blocks,
        plural(counts.blocks, "block", "blocks")
    ));
    parts.join("   ")
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    match n {
        1 => one,
        _ => many,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(words: usize, blocks: usize) -> Counts {
        Counts {
            words,
            characters: 0,
            blocks,
            headings: 0,
        }
    }

    #[test]
    fn the_bar_says_where_the_caret_is_and_how_big_the_document_is() {
        let line = status_line("p3+7", 0, counts(120, 9));
        assert!(line.starts_with("p3+7"), "{line}");
        assert!(line.contains("120 words"), "{line}");
        assert!(line.contains("9 blocks"), "{line}");
        assert!(!line.contains("selected"), "nothing is: {line}");
    }

    #[test]
    fn a_selection_is_mentioned_only_when_there_is_one() {
        let line = status_line("p1+0", 42, counts(5, 1));
        assert!(line.contains("42 selected"), "{line}");
    }

    /// One word is a word. The alternative is "1 words", which is the sort of thing that makes a
    /// window look unfinished for the sake of two lines.
    #[test]
    fn one_of_a_thing_is_singular() {
        let line = status_line("p1+0", 0, counts(1, 1));
        assert!(line.contains("1 word "), "{line}");
        assert!(line.ends_with("1 block"), "{line}");
    }
}
