// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a laid-out line is drawn as — the decisions a painter makes before any pixel, which are
//! the same in every shell that sets a page with its own ink.
//!
//! [`pieces`] cuts a line at every run boundary, [`drawable`] cuts a piece around the two
//! characters that are in the model and never drawn, [`covered`] and [`band`] say where a
//! selection's wash goes on one line, and [`bullet`] what marks a list item. Only putting the
//! pixels down is a shell's.
//!
//! These were the Windows pane's (`ui_win32/src/text/draw.rs`), with the selection band a second
//! time in the GNOME window, and the bullet a third time in the terminal. The macOS page would
//! have copied all of it (`doc/macos-shell.md`, M6). Two things had drifted:
//!
//! * the GNOME window drew the same bullet at every depth, where the other two cycled — and they
//!   cycled through different third marks;
//! * the soft-break bug — a selection's band on every line but its last collapsing to nothing,
//!   because [`Layout::x_at`] puts an offset at a break on the *later* line — had been found by a
//!   screenshot in each desktop window separately, and fixed twice. [`band`] is the one fix.
//!
//! The browser draws a line as `<span>`s and cuts it at the caret and every bookmark as well
//! (`ui_web/src/text/runs.rs`), so it keeps its own cutter; the terminal draws cells.

use grind_core::layout::{Layout, Line};

use crate::style::CharStyle;
use crate::{Caret, RunView};

/// One run of uniform formatting, clipped to a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece<'a> {
    /// Where it starts, in characters from the beginning of the **block** — the unit every
    /// `grind_core::layout` offset is in, so it can be handed straight to [`Layout::x_at`].
    pub start: usize,
    pub text: &'a str,
    pub props: &'a CharStyle,
}

/// Cut `from..to` characters of a block into the runs it crosses.
///
/// The pieces are in order, none is empty, and together they are exactly the block's characters
/// in that range — which is what lets a painter walk a line once, placing each piece at the x the
/// core measured for its first character.
pub fn pieces(runs: &[RunView], from: usize, to: usize) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    for run in runs {
        let start = run.start.max(from);
        let end = run.end().min(to);
        if start >= end {
            continue;
        }
        // A `RunView`'s offsets are in characters and a `&str`'s are in bytes; this is the one
        // place a painter's two units meet.
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
            // What it looks like — its named style under its direct formatting — since that
            // is what a painter draws (`RunView::shown`).
            props: &run.shown,
        });
    }
    out
}

/// One piece's text split at the two characters that are **in the model and never drawn**, each
/// segment with the character offset it starts at.
///
/// A `text:tab` and a `text:line-break` are each one character with an advance of its own, and a
/// font would draw its glyph for U+0009 and U+000A — a box. So the drawing is cut around them and
/// each side is placed at the offset the **core** measured rather than wherever the pen ended
/// up. Empty segments are dropped, which is what makes two of them in a row cost nothing.
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

/// The part of `block` a selection from `from` to `to` (in document order) covers, as two
/// offsets into that block — `None` when the selection is empty or the block is outside it.
///
/// A block the selection runs *through* is covered to `usize::MAX`, which [`band`] clips against
/// each line — cheaper and less error-prone than carrying "the whole block" as a third case.
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

/// Where the wash for `start..end` (a block's [`covered`] offsets) goes on `line`: its left and
/// right x, from the line's own left edge — or `None` when the line holds none of it.
///
/// **The far end of a line is the line's own width**, not [`Layout::x_at`]: at a break an offset
/// belongs to two lines, and `x_at` resolves it to the later one, because that is where a caret
/// walking off a wrapped line should appear. Asked for this line's end it answers from the next
/// line's left margin, and a two-line selection's first line came out unwashed — found by a
/// screenshot in each desktop window, one at a time.
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

/// What marks a list item at `depth`: •, ◦, ▪, and round again — what every word processor
/// does, and what a document with a six-deep list needs.
///
/// The mark is **drawn and never stored**: a list's numbering lives in a list style this build
/// does not read (`doc/text-core.md`), so this is the page saying "there is a list item here"
/// rather than the document being given a character the caret could sit inside.
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

    /// A line in the middle of a long run gets the slice of it that is on the line, with the
    /// offset it really has in the block — which is what `Layout::x_at` is indexed by.
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

    /// The offsets are characters and the slicing is bytes — the one place a document in any
    /// language but English would break.
    #[test]
    fn a_run_is_cut_by_characters_and_not_by_bytes() {
        let runs = [run(0, "héllo wörld", false)];
        assert_eq!(pieces(&runs, 0, 5)[0].text, "héllo");
        assert_eq!(pieces(&runs, 6, 11)[0].text, "wörld");
    }

    /// A tab and a line break are measured and never drawn, so the drawing is cut around them —
    /// and each side keeps the offset the core measured it at. Found by running the Windows
    /// pane: both came out as the font's missing-glyph box.
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

    /// The bug found twice: a selection across a soft break washes every line to its own width,
    /// not to where the next line starts.
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
        // Depth is 1-based in the model, and a document claiming zero must not index past the
        // start of the table.
        assert_eq!(bullet(0), bullet(1));
    }
}
