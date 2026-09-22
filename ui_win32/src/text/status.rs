// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the text pane's status bar says — a pure function, tested on any host.
//!
//! `sheet/status.rs`'s counterpart, and the same rule applies: the sentence is built here from
//! numbers the core already answers, so that nothing about it is decided inside a paint. The
//! spreadsheet's bar carries Sum/Count/Average over the selection; this one carries what the
//! caret is in and how long the document is, which are the two questions a word processor's bar
//! answers everywhere.

use grind_text::Counts;

/// The text pane's status bar, in words: what the caret is in and how long the document is —
/// `Heading 1  ·  90 words`, `Paragraph — Text body  ·  90 words  ·  12 characters selected`.
///
/// It used to lead with the caret's address and count blocks (`p1+0   90 words   22 blocks`):
/// the address is Go To's (F5 opens holding it) and a block is the model's word, not a reader's.
/// `ui_text_gtk` and `ui_web` made the same change, and the style name is decoded by the one
/// function all three share (`grind_text::style::readable_name`).
pub fn status_line(here: &str, selected: usize, counts: Counts) -> String {
    let mut parts = Vec::new();
    if !here.is_empty() {
        parts.push(here.to_owned());
    }
    parts.push(format!(
        "{} {}",
        counts.words,
        plural(counts.words, "word", "words")
    ));
    if selected > 0 {
        parts.push(format!(
            "{selected} {} selected",
            plural(selected, "character", "characters")
        ));
    }
    parts.join("  \u{00b7}  ")
}

/// What a block is, for [`status_line`] — its kind, and a named style after it.
pub fn describe(kind: &grind_text::BlockKind, style: Option<&str>) -> String {
    use grind_text::BlockKind;
    let name = match (kind, style) {
        (BlockKind::Paragraph, Some(named @ ("Title" | "Subtitle"))) => return named.to_owned(),
        (BlockKind::Paragraph, _) => "Paragraph".to_owned(),
        (BlockKind::Heading { level }, _) => format!("Heading {level}"),
        (BlockKind::ListItem { depth: 1 }, _) => "List item".to_owned(),
        (BlockKind::ListItem { depth }, _) => format!("List item, level {depth}"),
    };
    match style {
        Some(style) => format!(
            "{name} \u{2014} {}",
            grind_text::style::readable_name(style)
        ),
        None => name,
    }
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
    fn the_bar_says_what_the_caret_is_in_and_how_long_the_document_is() {
        let line = status_line("Heading 1", 0, counts(120, 9));
        assert_eq!(line, "Heading 1  \u{00b7}  120 words");
        assert!(
            !line.contains("block"),
            "a block is the model's word: {line}"
        );
    }

    #[test]
    fn a_selection_is_mentioned_only_when_there_is_one() {
        let line = status_line("Paragraph", 42, counts(5, 1));
        assert!(line.ends_with("42 characters selected"), "{line}");
        assert!(status_line("", 1, counts(1, 1)).ends_with("1 character selected"));
    }

    #[test]
    fn a_block_is_named_the_way_a_reader_would() {
        use grind_text::BlockKind;
        assert_eq!(
            describe(&BlockKind::Heading { level: 2 }, None),
            "Heading 2"
        );
        assert_eq!(describe(&BlockKind::Paragraph, Some("Title")), "Title");
        assert_eq!(
            describe(&BlockKind::Paragraph, Some("Text_20_body")),
            "Paragraph \u{2014} Text body"
        );
    }
}
