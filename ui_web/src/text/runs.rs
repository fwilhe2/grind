// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cutting one laid-out line into the pieces the page draws it as.
//!
//! A line is drawn as a run of `<span>`s rather than as one string, because three different
//! things change part-way along it and none of them lines up with the others: the document's
//! own **formatting** (a bold word), the **selection**, and the **caret**. Each of those is a
//! set of boundaries in the block's own character offsets; the pieces are what falls between
//! two adjacent ones.
//!
//! Pure arithmetic over offsets, so it runs on the host with no browser — which is where the
//! interesting cases are: a bold run that starts mid-selection, a selection that starts
//! mid-word, a caret sitting exactly on either boundary.

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
}

/// Cut `line` into pieces. `runs` are the block's own, in order; `selection` is in the same
/// offsets and may lie entirely outside this line.
///
/// A `caret` inside the line is a boundary too, so the caret element can be appended between
/// two pieces rather than inside one — which is what lets the browser place it against its own
/// kerning (`text/mod.rs`).
pub fn cut(
    line: Range<usize>,
    runs: &[RunView],
    selection: Option<Range<usize>>,
    caret: Option<usize>,
) -> Vec<Piece> {
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
                props: run.map(|run| run.props.clone()).unwrap_or_default(),
                href: run.and_then(|run| run.href.clone()),
                selected: selection
                    .as_ref()
                    .is_some_and(|sel| sel.start <= start && end <= sel.end),
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
    class
}

/// The inline style a piece's formatting asks for: the values the *document* chose, which is
/// the half a class cannot carry. Empty when it chose none.
pub fn css(piece: &Piece) -> String {
    let mut css = String::new();
    if let Some(color) = &piece.props.color {
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

    #[test]
    fn a_line_of_one_run_is_one_piece() {
        let runs = [run(0, "hello world", CharStyle::default())];
        let pieces = cut(0..11, &runs, None, None);
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
        let pieces = cut(0..15, &runs, None, None);
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
        let pieces = cut(0..10, &runs, Some(3..8), None);
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
        let pieces = cut(0..6, &runs, None, Some(3));
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].range, 0..3);
        assert_eq!(pieces[1].range, 3..6);
        // At either end it adds nothing: there is already a boundary there.
        assert_eq!(cut(0..6, &runs, None, Some(0)).len(), 1);
        assert_eq!(cut(0..6, &runs, None, Some(6)).len(), 1);
    }

    /// A selection that covers the whole line, and one that misses it entirely.
    #[test]
    fn a_selection_outside_the_line_selects_nothing_in_it() {
        let runs = [run(0, "abcdef", CharStyle::default())];
        let all = cut(0..6, &runs, Some(0..6), None);
        assert!(all.iter().all(|p| p.selected));
        let none = cut(0..6, &runs, Some(20..30), None);
        assert!(none.iter().all(|p| !p.selected));
    }

    #[test]
    fn an_empty_line_has_no_pieces_at_all() {
        assert!(cut(4..4, &[], None, Some(4)).is_empty());
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
        };
        let inline = css(&piece);
        assert!(inline.contains("color:#ff4136;"), "{inline}");
        assert!(inline.contains("background-color:#ffdc00;"), "{inline}");
        // Under a selection the highlight steps aside, or neither reads.
        let selected = Piece {
            selected: true,
            ..piece
        };
        assert!(!css(&selected).contains("background-color"));
    }
}
