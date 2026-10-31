// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cutting one laid-out line into the `<span>`s the page draws: the **formatting**, the
//! **selection** and the **caret** each change part-way along a line and none line up, so each is
//! a set of boundaries in the block's offsets and a piece is what falls between two. Pure
//! arithmetic, tested on the host — a bold run starting mid-selection, a caret on a boundary.

use std::ops::Range;

use grind_text::RunView;
use grind_text::style::CharStyle;

/// One stretch of a line that is uniform in everything the page can draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// In the block's own character offsets, the same ones a [`grind_text::Caret`] counts.
    pub range: Range<usize>,
    /// The formatting the document gave it.
    pub props: CharStyle,
    /// `xlink:href`, when this piece is inside a link.
    pub href: Option<String>,
    pub selected: bool,
    /// Whether it is (part of) a word the spell checker underlines.
    pub wrong: bool,
    /// Whether it is (part of) a grammar finding, and if so whether a quiet one (typography or
    /// style).
    pub grammar: Option<bool>,
}

/// What a line is marked with besides its formatting: the misspelt words, and the grammar
/// findings with whether each is quiet — all in the block's offsets.
#[derive(Clone, Copy, Debug, Default)]
pub struct Marks<'a> {
    pub misspelt: &'a [Range<usize>],
    pub flagged: &'a [(Range<usize>, bool)],
}

/// Cut `line` into pieces. `runs` are the block's, in order; `selection` is in the same offsets
/// and may lie outside this line. A `caret` and the bookmark `anchors` are boundaries too, so
/// their elements sit between two pieces and the browser places them against its own kerning.
#[cfg(test)]
pub fn cut(
    line: Range<usize>,
    runs: &[RunView],
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    anchors: &[usize],
) -> Vec<Piece> {
    cut_marked(line, runs, selection, caret, anchors, &Marks::default())
}

/// [`cut`], with the misspelt words and grammar findings of the block — in the same offsets —
/// each a boundary at both ends, so an underline starts and stops exactly on what it marks.
pub fn cut_marked(
    line: Range<usize>,
    runs: &[RunView],
    selection: Option<Range<usize>>,
    caret: Option<usize>,
    anchors: &[usize],
    marks: &Marks<'_>,
) -> Vec<Piece> {
    let misspelt = marks.misspelt;
    if line.start >= line.end {
        return Vec::new();
    }
    let mut bounds = vec![line.start, line.end];
    let mut mark = |at: usize| {
        if at > line.start && at < line.end {
            bounds.push(at);
        }
    };
    for run in runs {
        mark(run.start);
        mark(run.start + run.text.chars().count());
    }
    if let Some(selection) = &selection {
        mark(selection.start);
        mark(selection.end);
    }
    if let Some(caret) = caret {
        mark(caret);
    }
    for at in anchors {
        mark(*at);
    }
    for word in misspelt
        .iter()
        .chain(marks.flagged.iter().map(|(range, _)| range))
    {
        mark(word.start);
        mark(word.end);
    }
    bounds.sort_unstable();
    bounds.dedup();

    bounds
        .windows(2)
        .map(|pair| {
            let (start, end) = (pair[0], pair[1]);
            let run = runs.iter().find(|run| {
                (run.start..run.start + run.text.chars().count().max(1)).contains(&start)
            });
            Piece {
                range: start..end,
                props: run.map(|run| run.shown.clone()).unwrap_or_default(),
                href: run.and_then(|run| run.href.clone()),
                selected: selection
                    .as_ref()
                    .is_some_and(|sel| sel.start <= start && end <= sel.end),
                wrong: misspelt
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
    if piece.selected {
        class.push_str(" sel");
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
/// colour and a highlight's text must read against ([`crate::ink`]).
pub fn css(piece: &Piece, dark: bool) -> String {
    let mut css = String::new();
    let fill = piece
        .props
        .background
        .as_deref()
        .filter(|_| !piece.selected);
    if let Some(color) = crate::ink::color(piece.props.color.as_deref(), fill, dark) {
        css.push_str(&format!("color:{color};"));
    }
    // A highlight loses to the selection, which has to stay legible as a selection.
    if let Some(background) = &piece.props.background
        && !piece.selected
    {
        css.push_str(&format!("background-color:{background};"));
    }
    if let Some(family) = &piece.props.font_family {
        css.push_str(&format!("font-family:{family};"));
    }
    if let Some(size) = &piece.props.font_size {
        css.push_str(&format!("font-size:{size};"));
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

    /// A grammar finding is a stretch of its own, drawn in the accent — or quietly, for style
    /// (`doc/grammar.md` decision 2).
    #[test]
    fn a_grammar_finding_is_cut_out_of_the_line_and_marked() {
        let runs = [run(0, "the the cat sat", CharStyle::default())];
        let flagged = [(3..7, false), (12..15, true)];
        let marks = Marks {
            misspelt: &[],
            flagged: &flagged,
        };
        let pieces = cut_marked(0..15, &runs, None, None, &[], &marks);
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
    fn a_misspelt_word_is_cut_out_of_the_line_and_marked() {
        let runs = [run(0, "a wrongg word", CharStyle::default())];
        let word: Range<usize> = 2..8;
        let marks = Marks {
            misspelt: std::slice::from_ref(&word),
            flagged: &[],
        };
        let pieces = cut_marked(0..13, &runs, None, None, &[], &marks);
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
    fn a_line_of_one_run_is_one_piece() {
        let runs = [run(0, "hello world", CharStyle::default())];
        let pieces = cut(0..11, &runs, None, None, &[]);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].range, 0..11);
        assert!(!pieces[0].selected);
    }

    #[test]
    fn formatting_cuts_the_line_where_the_run_changes() {
        let runs = [
            run(0, "plain ", CharStyle::default()),
            run(6, "bold", bold()),
            run(10, " tail", CharStyle::default()),
        ];
        let pieces = cut(0..15, &runs, None, None, &[]);
        assert_eq!(
            pieces.iter().map(|p| p.range.clone()).collect::<Vec<_>>(),
            vec![0..6, 6..10, 10..15]
        );
        assert!(classes(&pieces[1]).contains(" b"));
        assert!(!classes(&pieces[0]).contains(" b"));
    }

    #[test]
    fn a_selection_cuts_across_the_formatting_rather_than_with_it() {
        let runs = [
            run(0, "plain ", CharStyle::default()),
            run(6, "bold", bold()),
        ];
        // Selected from the middle of the plain run into the middle of the bold one.
        let pieces = cut(0..10, &runs, Some(3..8), None, &[]);
        assert_eq!(
            pieces.iter().map(|p| p.range.clone()).collect::<Vec<_>>(),
            vec![0..3, 3..6, 6..8, 8..10]
        );
        assert_eq!(
            pieces.iter().map(|p| p.selected).collect::<Vec<_>>(),
            vec![false, true, true, false]
        );
    }

    /// The caret is a boundary of its own, so it never has to be drawn inside a piece.
    #[test]
    fn the_caret_splits_the_piece_it_sits_in() {
        let runs = [run(0, "abcdef", CharStyle::default())];
        let pieces = cut(0..6, &runs, None, Some(3), &[]);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].range, 0..3);
        assert_eq!(pieces[1].range, 3..6);
        // At either end it adds nothing: there is already a boundary there.
        assert_eq!(cut(0..6, &runs, None, Some(0), &[]).len(), 1);
        assert_eq!(cut(0..6, &runs, None, Some(6), &[]).len(), 1);
    }

    /// A bookmark's own offset splits the piece it sits in too, the same as the caret —
    /// `doc/view-modes.md` §3.6's tick needs a boundary to sit against, not a spot mid-run.
    #[test]
    fn a_bookmark_anchor_splits_the_piece_it_sits_in() {
        let runs = [run(0, "abcdef", CharStyle::default())];
        let pieces = cut(0..6, &runs, None, None, &[3]);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].range, 0..3);
        assert_eq!(pieces[1].range, 3..6);
    }

    /// A selection that covers the whole line, and one that misses it entirely.
    #[test]
    fn a_selection_outside_the_line_selects_nothing_in_it() {
        let runs = [run(0, "abcdef", CharStyle::default())];
        let all = cut(0..6, &runs, Some(0..6), None, &[]);
        assert!(all.iter().all(|p| p.selected));
        let none = cut(0..6, &runs, Some(20..30), None, &[]);
        assert!(none.iter().all(|p| !p.selected));
    }

    #[test]
    fn an_empty_line_has_no_pieces_at_all() {
        assert!(cut(4..4, &[], None, Some(4), &[]).is_empty());
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
            selected: false,
            wrong: false,
            grammar: None,
        };
        let inline = css(&piece, false);
        assert!(inline.contains("color:#ff4136;"), "{inline}");
        assert!(inline.contains("background-color:#ffdc00;"), "{inline}");
        // Under a selection the highlight steps aside, or neither reads.
        let selected = Piece {
            selected: true,
            ..piece
        };
        assert!(!css(&selected, false).contains("background-color"));
    }
}
