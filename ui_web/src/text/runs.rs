// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cutting one block into the `<span>`s of the reader layer: the **formatting**, the misspelt
//! words and the grammar findings each change part-way along a block and none line up, so each is
//! a set of boundaries in the block's offsets and a piece is what falls between two. Pure
//! arithmetic, tested on the host.

use std::fmt::Write as _;
use std::ops::Range;

use grind_text::RunView;
use grind_text::style::CharStyle;

/// One stretch of a block that is uniform in everything the reader layer marks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    /// In the block's own character offsets, the same ones a [`grind_text::Caret`] counts.
    pub range: Range<usize>,
    /// The formatting the document gave it.
    pub props: CharStyle,
    /// `xlink:href`, when this piece is inside a link.
    pub href: Option<String>,
    /// Whether it is (part of) a word the spell checker marks.
    pub wrong: bool,
    /// Whether it is (part of) a grammar finding, and if so whether a quiet one (typography or
    /// style).
    pub grammar: Option<bool>,
}

/// What a block is marked with besides its formatting: the misspelt words, and the grammar
/// findings with whether each is quiet — all in the block's offsets.
#[derive(Clone, Copy, Debug, Default)]
pub struct Marks<'a> {
    pub misspelt: &'a [Range<usize>],
    pub flagged: &'a [(Range<usize>, bool)],
}

/// Cut `span` of a block into pieces. `runs` are the block's, in order; every mark is a boundary
/// at both ends, so it starts and stops exactly on what it marks.
pub fn cut(span: Range<usize>, runs: &[RunView], marks: &Marks<'_>) -> Vec<Piece> {
    if span.start >= span.end {
        return Vec::new();
    }
    let mut bounds = vec![span.start, span.end];
    let mut mark = |at: usize| {
        if at > span.start && at < span.end {
            bounds.push(at);
        }
    };
    for run in runs {
        mark(run.start);
        mark(run.start + run.text.chars().count());
    }
    for word in marks
        .misspelt
        .iter()
        .chain(marks.flagged.iter().map(|(range, _)| range))
    {
        mark(word.start);
        mark(word.end);
    }
    bounds.sort_unstable();
    bounds.dedup();

    bounds
        .array_windows()
        .map(|&[start, end]| {
            let run = runs.iter().find(|run| {
                (run.start..run.start + run.text.chars().count().max(1)).contains(&start)
            });
            Piece {
                range: start..end,
                props: run.map_or_default(|run| run.shown.clone()),
                href: run.and_then(|run| run.href.clone()),
                wrong: marks
                    .misspelt
                    .iter()
                    .any(|word| word.start <= start && end <= word.end),
                grammar: marks
                    .flagged
                    .iter()
                    .find(|(range, _)| range.start <= start && end <= range.end)
                    .map(|(_, quiet)| *quiet),
            }
        })
        .collect()
}

/// The classes a piece's formatting asks for — the four the stylesheet spells, plus a link.
pub fn classes(piece: &Piece) -> String {
    let mut class = String::from("run");
    let on = |value: &Option<String>, off: &str| value.as_deref().is_some_and(|value| value != off);
    if piece
        .props
        .font_weight
        .as_deref()
        .is_some_and(|w| w != "normal")
    {
        class.push_str(" b");
    }
    if on(&piece.props.font_style, "normal") {
        class.push_str(" i");
    }
    if on(&piece.props.underline, "none") {
        class.push_str(" u");
    }
    if on(&piece.props.line_through, "none") {
        class.push_str(" s");
    }
    if piece.href.is_some() {
        class.push_str(" link");
    }
    if let Some(quiet) = piece.grammar {
        class.push_str(if quiet { " gram quiet" } else { " gram" });
    }
    if piece.wrong {
        class.push_str(" wrong");
    }
    class
}

/// The inline style a piece's formatting asks for: the values the document chose, which a class
/// cannot carry; empty when it chose none. `dark` is whether the page is dark, which a document's
/// colour must read against ([`crate::ink`]).
pub fn css(piece: &Piece, dark: bool) -> String {
    let mut css = String::new();
    let fill = piece.props.background.as_deref();
    if let Some(color) = crate::ink::color(piece.props.color.as_deref(), fill, dark) {
        let _ = write!(css, "color:{color};");
    }
    if let Some(background) = &piece.props.background {
        let _ = write!(css, "background-color:{background};");
    }
    if let Some(family) = &piece.props.font_family {
        let _ = write!(css, "font-family:{family};");
    }
    if let Some(size) = &piece.props.font_size {
        let _ = write!(css, "font-size:{size};");
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(start: usize, text: &str, props: CharStyle) -> RunView {
        RunView {
            start,
            text: text.to_owned(),
            shown: props.clone(),
            props,
            style: None,
            href: None,
            image: None,
        }
    }

    fn bold() -> CharStyle {
        CharStyle {
            font_weight: Some("bold".into()),
            ..CharStyle::default()
        }
    }

    /// A grammar finding is a stretch of its own, marked plainly — or quietly, for style
    /// (`doc/grammar.md` decision 2).
    #[test]
    fn a_grammar_finding_is_cut_out_and_marked() {
        let runs = [run(0, "the the cat sat", CharStyle::default())];
        let flagged = [(3..7, false), (12..15, true)];
        let marks = Marks {
            misspelt: &[],
            flagged: &flagged,
        };
        let pieces = cut(0..15, &runs, &marks);
        assert_eq!(
            pieces.iter().map(|p| p.range.clone()).collect::<Vec<_>>(),
            vec![0..3, 3..7, 7..12, 12..15]
        );
        assert_eq!(
            pieces.iter().map(|p| p.grammar).collect::<Vec<_>>(),
            [None, Some(false), None, Some(true)]
        );
        assert_eq!(classes(&pieces[1]), "run gram");
        assert_eq!(classes(&pieces[3]), "run gram quiet");
    }

    #[test]
    fn a_misspelt_word_is_cut_out_and_marked() {
        let runs = [run(0, "a wrongg word", CharStyle::default())];
        let word: Range<usize> = 2..8;
        let marks = Marks {
            misspelt: std::slice::from_ref(&word),
            flagged: &[],
        };
        let pieces = cut(0..13, &runs, &marks);
        assert_eq!(
            pieces.iter().map(|p| p.range.clone()).collect::<Vec<_>>(),
            vec![0..2, 2..8, 8..13]
        );
        assert_eq!(
            pieces.iter().map(|p| p.wrong).collect::<Vec<_>>(),
            [false, true, false]
        );
        assert!(classes(&pieces[1]).contains(" wrong"));
        assert!(!classes(&pieces[0]).contains("wrong"));
    }

    #[test]
    fn formatting_cuts_where_the_run_changes() {
        let runs = [
            run(0, "plain ", CharStyle::default()),
            run(6, "bold", bold()),
            run(10, " tail", CharStyle::default()),
        ];
        let pieces = cut(0..15, &runs, &Marks::default());
        assert_eq!(
            pieces.iter().map(|p| p.range.clone()).collect::<Vec<_>>(),
            vec![0..6, 6..10, 10..15]
        );
        assert!(classes(&pieces[1]).contains(" b"));
        assert!(!classes(&pieces[0]).contains(" b"));
    }

    #[test]
    fn an_empty_block_has_no_pieces_at_all() {
        assert_eq!(cut(4..4, &[], &Marks::default()), Vec::new());
    }

    /// A document's own colour is a value, not a class — it has to reach the page verbatim.
    #[test]
    fn a_colour_the_document_chose_becomes_inline_css() {
        let piece = Piece {
            range: 0..1,
            props: CharStyle {
                color: Some("#ff4136".into()),
                background: Some("#ffdc00".into()),
                ..CharStyle::default()
            },
            href: None,
            wrong: false,
            grammar: None,
        };
        let inline = css(&piece, false);
        assert!(inline.contains("color:#ff4136;"), "{inline}");
        assert!(inline.contains("background-color:#ffdc00;"), "{inline}");
    }
}
