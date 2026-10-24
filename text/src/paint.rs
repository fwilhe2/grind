// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a laid-out line is drawn as — the painter's decisions before any pixel, the same in every
//! shell that draws its own page. [`pieces`] cuts a line at run boundaries, [`drawable`] cuts a
//! piece around the characters that are measured but never drawn, [`covered`] and [`band`] place a
//! selection's wash on one line, and [`bullet`] marks a list item. The browser cuts lines into
//! `<span>`s itself (`ui_web/src/text/runs.rs`) and the terminal draws cells.

use grind_core::layout::{Layout, Line};

use crate::style::CharStyle;
use crate::{Caret, RunView};

/// One run of uniform formatting, clipped to a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece<'a> {
    /// Where it starts, in characters from the start of the block — the unit [`Layout::x_at`]
    /// takes.
    pub start: usize,
    pub text: &'a str,
    pub props: &'a CharStyle,
}

/// Cut `from..to` characters of a block into the runs it crosses: in order, none empty, together
/// exactly the block's characters in that range.
pub fn pieces(runs: &[RunView], from: usize, to: usize) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    for run in runs {
        let start = run.start.max(from);
        let end = run.end().min(to);
        if start >= end {
            continue;
        }
        // `RunView` offsets are characters and `&str` offsets bytes; this is where they meet.
        let mut indices = run.text.char_indices().map(|(byte, _)| byte);
        let head = indices
            .by_ref()
            .nth(start - run.start)
            .unwrap_or(run.text.len());
        let tail = match end - start {
            0 => head,
            n => indices.nth(n - 1).unwrap_or(run.text.len()),
        };
        out.push(Piece {
            start,
            text: &run.text[head..tail],
            // What it looks like — its named style under its direct formatting (`RunView::shown`).
            props: &run.shown,
        });
    }
    out
}

/// One piece's text split at the characters that are in the model and never drawn — `text:tab` and
/// `text:line-break`, which a font would draw as a box — each segment with its starting offset, so
/// each side is placed where the core measured it. Empty segments are dropped.
pub fn drawable(start: usize, text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut at = start;
    for segment in text.split(['\t', '\n']) {
        if !segment.is_empty() {
            out.push((at, segment));
        }
        // The segment's own characters, plus the one that ended it.
        at += segment.chars().count() + 1;
    }
    out
}

/// The part of `block` a selection from `from` to `to` covers, as two offsets into it, or `None`.
/// A block the selection runs through is covered to `usize::MAX`, which [`band`] clips per line.
pub fn covered(block: usize, from: Caret, to: Caret) -> Option<(usize, usize)> {
    if from == to || block < from.block || block > to.block {
        return None;
    }
    let start = match block == from.block {
        true => from.offset,
        false => 0,
    };
    let end = match block == to.block {
        true => to.offset,
        false => usize::MAX,
    };
    Some((start, end))
}

/// Where the wash for `start..end` goes on `line`, as left and right x from the line's own left
/// edge, or `None`. The far end is the line's own width, not [`Layout::x_at`], which resolves an
/// offset at a break to the later line and would leave the first line of a two-line selection
/// unwashed.
pub fn band(layout: &Layout, line: &Line, start: usize, end: usize) -> Option<(f32, f32)> {
    let from = start.max(line.start);
    let to = end.min(line.end);
    if from >= to {
        return None;
    }
    let right = match to == line.end {
        true => line.width,
        false => layout.x_at(to),
    };
    Some((layout.x_at(from), right))
}

/// What marks a list item at `depth`: •, ◦, ▪, and round again. Drawn, never stored, and only a
/// fallback for a list whose style states nothing (`crate::numbering`, `BlockView::mark`). Lists
/// this build opens are written with a list style drawing the same marks (`Lgrind1`,
/// `odf::write`).
pub fn bullet(depth: u32) -> &'static str {
    const MARKS: [&str; 3] = ["\u{2022}", "\u{25e6}", "\u{25aa}"];
    MARKS[(depth.max(1) as usize - 1) % MARKS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::layout::{Fixed, Fragment, wrap};
    use grind_core::style::TextStyle;

    fn run(start: usize, text: &str, bold: bool) -> RunView {
        let props = CharStyle {
            font_weight: bold.then(|| "bold".to_owned()),
            ..CharStyle::default()
        };
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

    fn laid_out(text: &str, width: f32) -> Layout {
        let style = TextStyle::default();
        wrap(
            &[Fragment {
                text,
                style: &style,
            }],
            width,
            &Fixed,
        )
    }

    #[test]
    fn a_line_is_cut_at_every_run_boundary() {
        let runs = [run(0, "hello ", false), run(6, "world", true)];
        let cut = pieces(&runs, 0, 11);
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0].text, "hello ");
        assert_eq!((cut[1].start, cut[1].text), (6, "world"));
        assert!(cut[1].props.is_bold());
    }

    /// A line in the middle of a long run gets its slice, with its real offset in the block.
    #[test]
    fn a_run_is_clipped_to_the_line_and_keeps_its_own_offsets() {
        let runs = [run(0, "abcdefghij", false)];
        let cut = pieces(&runs, 3, 7);
        assert_eq!(cut.len(), 1);
        assert_eq!((cut[0].start, cut[0].text), (3, "defg"));
    }

    #[test]
    fn a_run_outside_the_line_is_not_drawn_at_all() {
        let runs = [run(0, "abc", false), run(3, "def", false)];
        assert!(pieces(&runs, 0, 0).is_empty());
        assert_eq!(pieces(&runs, 3, 6).len(), 1);
    }

    /// Offsets are characters and slicing is bytes.
    #[test]
    fn a_run_is_cut_by_characters_and_not_by_bytes() {
        let runs = [run(0, "héllo wörld", false)];
        assert_eq!(pieces(&runs, 0, 5)[0].text, "héllo");
        assert_eq!(pieces(&runs, 6, 11)[0].text, "wörld");
    }

    /// A tab and a line break cut the drawing and keep their measured offsets, rather than drawing
    /// a missing-glyph box.
    #[test]
    fn a_tab_and_a_break_cut_the_drawing_and_keep_the_offsets() {
        assert_eq!(drawable(0, "name\tvalue"), vec![(0, "name"), (5, "value")]);
        assert_eq!(
            drawable(0, "value\nsecond"),
            vec![(0, "value"), (6, "second")]
        );
        assert_eq!(drawable(10, "a\t\tb"), vec![(10, "a"), (13, "b")]);
        assert_eq!(drawable(0, "\tx"), vec![(1, "x")], "a leading tab");
        assert!(drawable(0, "\t").is_empty(), "nothing but a tab");
        assert_eq!(drawable(3, "plain"), vec![(3, "plain")]);
    }

    #[test]
    fn a_selection_covers_its_ends_partly_and_what_is_between_wholly() {
        let at = |block, offset| Caret { block, offset };
        let (from, to) = (at(1, 3), at(3, 2));
        assert_eq!(covered(0, from, to), None, "before it");
        assert_eq!(covered(1, from, to), Some((3, usize::MAX)));
        assert_eq!(covered(2, from, to), Some((0, usize::MAX)), "through it");
        assert_eq!(covered(3, from, to), Some((0, 2)));
        assert_eq!(covered(4, from, to), None, "after it");
        assert_eq!(covered(1, from, from), None, "an empty selection");
        assert_eq!(covered(1, at(1, 2), at(1, 5)), Some((2, 5)), "inside one");
    }

    #[test]
    fn only_the_selected_part_of_a_line_is_washed() {
        let layout = laid_out("0123456789", 0.0);
        let line = layout.lines()[0];
        assert_eq!(band(&layout, &line, 3, 7), Some((3.0, 7.0)));
        assert_eq!(band(&layout, &line, 0, usize::MAX), Some((0.0, 10.0)));
        assert_eq!(band(&layout, &line, 5, 5), None, "nothing");
        assert_eq!(band(&layout, &line, 20, 30), None, "past the line");
    }

    /// A selection across a soft break washes each line to its own width.
    #[test]
    fn a_band_reaching_a_break_ends_at_that_lines_own_width() {
        let layout = laid_out("aaa bbb ccc ddd", 8.0);
        assert!(layout.lines().len() > 1, "the fixture has to wrap");
        let first = layout.lines()[0];
        assert!(
            layout.x_at(first.end) < first.width,
            "x_at answers the next line"
        );
        assert_eq!(
            band(&layout, &first, 0, usize::MAX),
            Some((0.0, first.width))
        );
        assert_eq!(
            band(&layout, &first, 1, first.end),
            Some((1.0, first.width))
        );
        // A hard break too: a `text:line-break` forces the line the GNOME window found it on.
        let broken = laid_out("name\tvalue\nsecond line", 1000.0);
        assert_eq!(broken.lines().len(), 2);
        let (left, right) = band(&broken, &broken.lines()[0], 0, broken.len()).unwrap();
        assert!(right > left);
    }

    #[test]
    fn a_list_marks_each_depth_differently_and_cycles() {
        assert_ne!(bullet(1), bullet(2));
        assert_ne!(bullet(2), bullet(3));
        assert_eq!(bullet(1), bullet(4), "three marks, then round again");
        // Depth is 1-based; a document claiming zero must not index before the table.
        assert_eq!(bullet(0), bullet(1));
    }
}
