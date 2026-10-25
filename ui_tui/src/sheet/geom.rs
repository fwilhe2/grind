// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where each column sits across the screen, as pure arithmetic with **no ratatui types**, so the
//! layout decisions test without a terminal.
//!
//! **A column is as wide as the document says**, so `2026-01-31` and `x` get the columns their
//! author gave them; widths accumulate from the scroll position as a prefix sum. [`pad`] measures
//! in **terminal cells** (`unicode-width`), not `char`s, so a CJK column lines up — the measure
//! [`crate::text::Cells`] gives the layout engine.

use std::collections::{HashMap, HashSet};

use grind_core::style::{length_mm, mm_length};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A column nobody sized, in terminal cells — including the one column separator every cell
/// keeps, so nine characters are visible.
pub const DEFAULT_WIDTH: u16 = 10;

/// How many millimetres one terminal cell stands for — derived: the suite's default column is one
/// inch, drawn here as [`DEFAULT_WIDTH`] cells, so a cell is a tenth of an inch.
pub const MM_PER_CELL: f64 = 25.4 / DEFAULT_WIDTH as f64;

/// The narrowest and widest a sized column is drawn. A document may hold a 0.5mm spacer column,
/// and three cells is the least that shows a cut value and its separator; the ceiling stops one
/// absurd column taking the whole window. The document keeps its own width either way.
pub const MIN_WIDTH: u16 = 3;
pub const MAX_WIDTH: u16 = 60;

/// The sizes a document gave particular columns, over a default for the rest, and which are
/// hidden. Sparse: `App::col_widths` returns exactly the ones stored, as ODF lengths, and this is
/// where they become cells.
#[derive(Clone, Debug, Default)]
pub struct Tracks {
    sized: HashMap<u32, u16>,
    hidden: HashSet<u32>,
}

impl Tracks {
    /// Build from the core's `(index, length)` pairs and its hidden columns. A length that will
    /// not parse is the default, not zero, as the readers tolerate it.
    pub fn new(
        sized: impl IntoIterator<Item = (u32, String)>,
        hidden: impl IntoIterator<Item = u32>,
    ) -> Self {
        Tracks {
            sized: sized
                .into_iter()
                .filter_map(|(at, length)| Some((at, cells(&length)?)))
                .collect(),
            hidden: hidden.into_iter().collect(),
        }
    }

    /// How wide one column is drawn, in cells. Never zero — a hidden column is not *narrow*, it is
    /// absent, and [`columns`] skips it rather than drawing it as nothing.
    pub fn width(&self, at: u32) -> u16 {
        self.sized.get(&at).copied().unwrap_or(DEFAULT_WIDTH)
    }

    pub fn is_hidden(&self, at: u32) -> bool {
        self.hidden.contains(&at)
    }
}

/// An ODF length as a whole number of terminal cells, clamped to what a grid can draw; `None` for
/// a length that will not parse.
pub fn cells(length: &str) -> Option<u16> {
    let mm = length_mm(length)?;
    if !mm.is_finite() || mm <= 0.0 {
        return None;
    }
    Some(
        ((mm / MM_PER_CELL).round() as i64).clamp(i64::from(MIN_WIDTH), i64::from(MAX_WIDTH))
            as u16,
    )
}

/// The ODF length a column of `cells` terminal cells is, for `:width` to write.
pub fn length(cells: u16) -> String {
    mm_length(f64::from(cells.max(1)) * MM_PER_CELL)
}

/// The columns visible from `first`, and how wide each is drawn: as many as fit in `room`,
/// skipping hidden ones, never fewer than one. The last may be **clipped**, as every spreadsheet
/// draws the right edge, rather than dropped and jumping a column per resize.
pub fn columns(tracks: &Tracks, first: u32, room: u16, limit: u32) -> Vec<(u32, u16)> {
    let mut out = Vec::new();
    let mut used = 0u16;
    let mut at = first;
    while used < room && at < limit {
        if tracks.is_hidden(at) {
            at += 1;
            continue;
        }
        let width = tracks.width(at).min(room.saturating_sub(used));
        if width == 0 {
            break;
        }
        out.push((at, width));
        used += width;
        at += 1;
    }
    if out.is_empty() {
        out.push((first.min(limit.saturating_sub(1)), room.max(1)));
    }
    out
}

/// The first column to show so that `active` is on screen, moving the least it can — a cell in
/// view leaves the grid where it is. Scrolling forward walks back from `active` accumulating
/// widths, so `G` across sixteen thousand columns is not sixteen thousand steps.
pub fn follow(tracks: &Tracks, first: u32, active: u32, room: u16, limit: u32) -> u32 {
    if active < first {
        return active;
    }
    // Fully visible, not merely clipped: a cursor half off the right-hand edge is a cursor whose
    // contents cannot be read.
    let whole = tracks.width(active).min(room);
    if columns(tracks, first, room, limit)
        .iter()
        .any(|(col, width)| *col == active && *width == whole)
    {
        return first;
    }
    let mut used = 0u16;
    let mut at = active;
    loop {
        if !tracks.is_hidden(at) {
            let width = tracks.width(at);
            // One column wider than the window shows clipped and alone rather than not at all.
            if at != active && used + width > room {
                return at + 1;
            }
            used = used.saturating_add(width);
        }
        if at == 0 {
            return 0;
        }
        at -= 1;
    }
}

/// The rows drawn from `first` down: `count` of them, skipping folded ones — absent, not blank, so
/// a filtered sheet fills the window.
pub fn rows(hidden: &HashSet<u32>, first: u32, count: u32, limit: u32) -> Vec<u32> {
    let mut out = Vec::with_capacity(count as usize);
    let mut at = first;
    while (out.len() as u32) < count && at < limit {
        if !hidden.contains(&at) {
            out.push(at);
        }
        at += 1;
    }
    out
}

/// The most lines a wrapped row is drawn in; past it the cell is cut, as a short row cuts it in a
/// window.
pub const MAX_LINES: usize = 8;

/// `text` broken to fit a column `width` cells wide less its separator, as ranges of its
/// characters — one range when it fits, and at most [`MAX_LINES`].
pub fn wrapped(text: &str, width: u16) -> Vec<std::ops::Range<usize>> {
    use grind_core::layout::{Fragment, wrap};
    let style = grind_core::style::TextStyle::default();
    let fragment = Fragment {
        text,
        style: &style,
    };
    let room = f32::from(width.saturating_sub(1).max(1));
    let mut lines: Vec<_> = wrap(std::slice::from_ref(&fragment), room, &crate::text::Cells)
        .lines()
        .iter()
        .map(|line| line.start..line.end)
        .collect();
    lines.truncate(MAX_LINES);
    if lines.is_empty() {
        lines.push(0..text.chars().count());
    }
    lines
}

/// [`follow`]'s row twin: the first row to show so that `active` is on screen.
pub fn follow_row(hidden: &HashSet<u32>, first: u32, active: u32, count: u32, limit: u32) -> u32 {
    if active < first {
        return active;
    }
    if rows(hidden, first, count, limit).contains(&active) {
        return first;
    }
    // Put it on the last line, by walking back over `count` rows that are actually drawn.
    let mut seen = 0u32;
    let mut at = active;
    loop {
        if !hidden.contains(&at) {
            seen += 1;
        }
        if seen >= count.max(1) || at == 0 {
            return at;
        }
        at -= 1;
    }
}

/// Which way a cell's text sits in its column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Centre,
    Right,
}

/// What a cell's text *is*, for the one question [`pad`] asks when it does not fit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// Words, which may be cut — with an ellipsis saying so.
    Text,
    /// A number, a date or a time, which may not be cut: part of `2026-08-16` is a different date.
    /// One that does not fit is `###`, as LibreOffice and Excel draw it; `:width` shows it.
    Number,
}

/// Pad or cut `text` to exactly `width` **terminal cells**, one trailing space as the separator,
/// aligned in what is left. `unicode-width`, so a CJK ideograph is two cells and a combining mark
/// none. A cut stops on a whole character and says it is a cut: text ends in `…`, and a number
/// becomes `###` ([`Fit::Number`]).
pub fn pad(text: &str, width: usize, align: Align, fit: Fit) -> String {
    pad_marked(text, width, align, fit, &[])
        .into_iter()
        .map(|(piece, _)| piece)
        .collect()
}

/// [`pad`], cut into pieces at `marks` — character ranges of `text` — each piece saying whether
/// it is marked: what a misspelt word's underline is drawn over. A mark follows the text
/// wherever the alignment put it, stops where a cut does, and never covers the padding, the
/// ellipsis or the column separator.
pub fn pad_marked(
    text: &str,
    width: usize,
    align: Align,
    fit: Fit,
    marks: &[std::ops::Range<usize>],
) -> Vec<(String, bool)> {
    let room = width.saturating_sub(1);
    let needed = text.width();
    let kept = match (needed <= room, fit) {
        (true, _) => text.to_owned(),
        // The suite's one rule for it (`numfmt::overflow`), in this shell's unit: a cell.
        (false, Fit::Number) if room > 0 => grind_sheet::numfmt::overflow(room as f64, 1.0),
        (false, Fit::Number) => String::new(),
        (false, Fit::Text) => {
            // Leave a cell for the ellipsis, which is one cell wide.
            let mut kept = String::new();
            let mut used = 0usize;
            for c in text.chars() {
                let w = c.width().unwrap_or(0);
                if used + w + 1 > room {
                    break;
                }
                used += w;
                kept.push(c);
            }
            if room > 0 {
                kept.push('\u{2026}');
            }
            kept
        }
    };
    let spare = room.saturating_sub(kept.width());
    let (before, after) = match align {
        Align::Left => (0, spare),
        Align::Right => (spare, 0),
        Align::Centre => (spare / 2, spare - spare / 2),
    };
    // Only text kept as itself carries a mark: `###` is not the number's characters.
    let marking = matches!(fit, Fit::Text) && !marks.is_empty();
    let mut pieces: Vec<(String, bool)> = vec![(" ".repeat(before), false)];
    for (index, c) in kept.chars().enumerate() {
        let ellipsis = c == '\u{2026}' && index + 1 == kept.chars().count() && needed > room;
        let marked = marking && !ellipsis && marks.iter().any(|m| m.contains(&index));
        match pieces.last_mut() {
            Some((piece, was)) if *was == marked => piece.push(c),
            _ => pieces.push((c.to_string(), marked)),
        }
    }
    // The padding after, and the column separator, which every alignment keeps.
    let tail = format!("{} ", " ".repeat(after));
    match pieces.last_mut() {
        Some((piece, false)) => piece.push_str(&tail),
        _ => pieces.push((tail, false)),
    }
    pieces.retain(|(piece, _)| !piece.is_empty());
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn a_wrapped_cell_breaks_at_its_column_and_stops_at_max_lines() {
        assert_eq!(
            (wrapped("short", 10).len(), wrapped("short", 10)[0].clone()),
            (1, 0..5)
        );
        let lines = wrapped("one two three", 6);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(wrapped(&"word ".repeat(40), 6).len(), MAX_LINES);
        assert_eq!(wrapped("", 10)[0], 0..0, "an empty text is one empty line");
    }

    #[test]
    // One misspelt word is a slice of one range, which is what `pad_marked` takes.
    #[allow(clippy::single_range_in_vec_init)]
    fn a_mark_follows_the_text_and_stops_at_a_cut() {
        let marked = |pieces: Vec<(String, bool)>| -> Vec<String> {
            pieces
                .into_iter()
                .filter(|(_, m)| *m)
                .map(|(p, _)| p)
                .collect()
        };
        let left = pad_marked("We recieve it", 16, Align::Left, Fit::Text, &[3..10]);
        assert_eq!(marked(left.clone()), ["recieve"]);
        assert_eq!(
            left.iter().map(|(p, _)| p.as_str()).collect::<String>(),
            pad("We recieve it", 16, Align::Left, Fit::Text)
        );
        let right = pad_marked("Teh", 10, Align::Right, Fit::Text, &[0..3]);
        assert_eq!(right[0], ("      ".to_owned(), false));
        assert_eq!(marked(right), ["Teh"]);
        // Cut short: the part of the word that shows is marked, the ellipsis is not.
        let cut = pad_marked("Recieved from", 7, Align::Left, Fit::Text, &[0..8]);
        assert_eq!(marked(cut.clone()), ["Recie"]);
        assert_eq!(
            cut.iter().map(|(p, _)| p.as_str()).collect::<String>(),
            "Recie\u{2026} "
        );
        assert!(
            marked(pad_marked(
                "12345678",
                4,
                Align::Right,
                Fit::Number,
                &[0..8]
            ))
            .is_empty()
        );
    }

    fn tracks(sized: &[(u32, &str)], hidden: &[u32]) -> Tracks {
        Tracks::new(
            sized.iter().map(|(at, len)| (*at, (*len).to_owned())),
            hidden.iter().copied(),
        )
    }

    /// The derivation this file rests on: an unsized column is ten cells, and one inch is what an
    /// unsized column is everywhere else in the suite.
    #[test]
    fn an_inch_is_the_default_column_and_a_cell_is_a_tenth_of_one() {
        assert_eq!(cells("1in"), Some(DEFAULT_WIDTH));
        assert_eq!(cells("25.4mm"), Some(DEFAULT_WIDTH));
        assert_eq!(cells("2in"), Some(2 * DEFAULT_WIDTH));
        // And back, to within the rounding a whole number of cells costs.
        assert_eq!(cells(&length(DEFAULT_WIDTH)), Some(DEFAULT_WIDTH));
        assert_eq!(cells(&length(4)), Some(4));
    }

    #[test]
    fn a_length_this_build_cannot_read_leaves_the_column_at_the_default() {
        let tracks = tracks(&[(0, "wide"), (1, "0mm"), (2, "2in")], &[]);
        assert_eq!(tracks.width(0), DEFAULT_WIDTH);
        assert_eq!(tracks.width(1), DEFAULT_WIDTH, "zero is not a width");
        assert_eq!(tracks.width(2), 2 * DEFAULT_WIDTH);
        assert_eq!(tracks.width(99), DEFAULT_WIDTH, "nothing was said about it");
    }

    /// An absurd column cannot take the whole window, and a hairline one is still clickable.
    #[test]
    fn a_sized_column_is_clamped_at_both_ends() {
        assert_eq!(cells("0.5mm"), Some(MIN_WIDTH));
        assert_eq!(cells("500mm"), Some(MAX_WIDTH));
    }

    #[test]
    fn the_visible_columns_are_as_wide_as_the_document_says() {
        let tracks = tracks(&[(1, "2in")], &[]);
        let shown = columns(&tracks, 0, 45, 100);
        assert_eq!(shown, vec![(0, 10), (1, 20), (2, 10), (3, 5)]);
        // The last one is clipped rather than dropped, and the row is exactly `room` wide.
        assert_eq!(shown.iter().map(|(_, w)| w).sum::<u16>(), 45);
    }

    /// A hidden column is *absent*, not narrow — the same thing `hidden_rows` already does to a
    /// filtered row, on the other axis.
    #[test]
    fn a_hidden_column_is_skipped_rather_than_drawn_empty() {
        let tracks = tracks(&[], &[1, 2]);
        let shown = columns(&tracks, 0, 30, 100);
        assert_eq!(
            shown.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
            vec![0, 3, 4]
        );
    }

    #[test]
    fn a_window_with_no_room_still_draws_one_column() {
        let tracks = tracks(&[], &[]);
        assert_eq!(columns(&tracks, 0, 0, 100).len(), 1);
        assert_eq!(columns(&tracks, 0, 4, 100), vec![(0, 4)]);
    }

    /// Following moves the least it can, and stops when the active column is *wholly* on screen —
    /// a cursor clipped at the right-hand edge is a cursor whose contents cannot be read.
    #[test]
    fn the_view_follows_the_cursor_and_no_further() {
        let plain = tracks(&[], &[]);
        assert_eq!(follow(&plain, 3, 1, 40, 100), 1, "back up to it");
        assert_eq!(follow(&plain, 0, 2, 40, 100), 0, "already on screen");
        assert_eq!(follow(&plain, 0, 9, 40, 100), 6, "four fit, so 6..=9");

        // A wide column pushes the view further than a narrow one would.
        let wide = tracks(&[(3, "3in")], &[]);
        assert_eq!(follow(&wide, 0, 3, 40, 100), 2);
    }

    /// The window fills with rows that are actually there — a filtered sheet does not trail off
    /// into blanks a third of the way down.
    #[test]
    fn a_folded_row_is_skipped_rather_than_left_blank() {
        let hidden: HashSet<u32> = [1, 2, 3].into_iter().collect();
        assert_eq!(rows(&hidden, 0, 4, 100), vec![0, 4, 5, 6]);
        assert_eq!(
            rows(&HashSet::new(), 8, 3, 10),
            vec![8, 9],
            "the sheet ends"
        );
    }

    #[test]
    fn the_view_follows_the_cursor_down_past_the_folded_rows() {
        let hidden: HashSet<u32> = [1, 2, 3].into_iter().collect();
        assert_eq!(follow_row(&hidden, 0, 4, 4, 100), 0, "already on screen");
        assert_eq!(follow_row(&hidden, 0, 9, 4, 100), 6, "6,7,8,9");
        assert_eq!(follow_row(&hidden, 5, 0, 4, 100), 0, "back up to it");
        // A jump to the far end of a sheet is arithmetic, not a walk: this must return at once.
        assert_eq!(
            follow_row(&HashSet::new(), 0, 1_048_575, 20, 1_048_576),
            1_048_556
        );
    }

    #[test]
    fn a_number_sits_right_and_text_left_measured_in_cells() {
        assert_eq!(pad("12", 6, Align::Right, Fit::Number), "   12 ");
        assert_eq!(pad("ab", 6, Align::Left, Fit::Text), "ab    ");
        assert_eq!(pad("ab", 7, Align::Centre, Fit::Text), "  ab   ");
        for align in [Align::Left, Align::Centre, Align::Right] {
            for fit in [Fit::Text, Fit::Number] {
                assert_eq!(pad("overlong text", 6, align, fit).width(), 6);
            }
        }
    }

    /// A ten-cell column must not draw `2026-08-16` as `2026-08-1` or `3,710.00 €` as `3,710.00`:
    /// a number that does not fit is `###`, and cut text ends in an ellipsis.
    #[test]
    fn a_cut_says_it_is_a_cut_and_a_number_is_never_cut() {
        assert_eq!(
            pad("2026-08-16", 10, Align::Right, Fit::Number),
            "######### "
        );
        assert_eq!(
            pad("3,710.00 €", 10, Align::Right, Fit::Number),
            "######### "
        );
        assert_eq!(pad("612.34 €", 10, Align::Right, Fit::Number), " 612.34 € ");
        assert_eq!(
            pad("Groceries actual", 10, Align::Left, Fit::Text),
            "Grocerie\u{2026} "
        );
        // Exactly full is not a cut.
        assert_eq!(
            pad("123456789", 10, Align::Right, Fit::Number),
            "123456789 "
        );
        assert_eq!(pad("abcdefghi", 10, Align::Left, Fit::Text), "abcdefghi ");
        // A column too narrow for anything still keeps its separator.
        assert_eq!(pad("abc", 1, Align::Left, Fit::Text), " ");
    }

    /// The half of the old `ponytail` that was about *characters*: two ideographs are four cells,
    /// so a ten-cell column holds four of them and a space, not nine.
    #[test]
    fn a_wide_character_takes_two_cells_of_its_column() {
        let text = "\u{4e16}\u{754c}\u{4e16}\u{754c}\u{4e16}";
        let padded = pad(text, 10, Align::Left, Fit::Text);
        assert_eq!(padded.width(), 10, "{padded:?}");
        assert_eq!(
            padded.chars().filter(|c| *c == '\u{4e16}').count(),
            2,
            "four ideographs and an ellipsis fit in nine cells, not nine ideographs: {padded:?}"
        );
        // And never half a character in the last cell.
        assert_eq!(pad("\u{4e16}", 2, Align::Left, Fit::Text).width(), 2);
    }
}
