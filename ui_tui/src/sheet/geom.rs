// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where each column sits across the screen, as pure arithmetic. **No ratatui types.**
//!
//! `ui_sheet_gtk/src/geom.rs` and `ui_web/src/sheet/layout.rs`'s counterpart, and it exists for
//! their reason: the layout decisions are the part most worth testing and the part hardest to test
//! through a terminal, so they live in a module that has never heard of one.
//!
//! **A column is as wide as the document says it is.** That used to be a `ponytail` in
//! `sheet/app.rs` — every column ten cells — and it was the difference between opening somebody's
//! spreadsheet and opening an approximation of it: a column sized to hold `2026-01-31` and a
//! column sized to hold `x` were the same width, and a wide one truncated its own contents while
//! the narrow one sat mostly empty. Widths accumulate from the scroll position rather than
//! dividing by one, which is the same prefix-sum shape both other shells grew.
//!
//! [`pad`] is the other half of the same `ponytail`: padding is measured in **terminal cells**
//! (`unicode-width`), not in `char`s, so a column of CJK text lines up with the one beside it. It
//! is the same measure [`crate::text::Cells`] hands the layout engine, which is what stops the two
//! halves of this shell disagreeing about how wide text is.

use std::collections::{HashMap, HashSet};

use grind_core::style::{length_mm, mm_length};
use unicode_width::UnicodeWidthChar;

/// A column nobody sized, in terminal cells — including the one column separator every cell
/// keeps, so nine characters are visible.
pub const DEFAULT_WIDTH: u16 = 10;

/// How many millimetres one terminal cell stands for.
///
/// **Derived, not chosen**: the suite's own default column is one inch wide (`ui_web`'s
/// `CELL.cell_w` is 96 CSS pixels at 96 per inch), and this shell draws an unsized column as
/// [`DEFAULT_WIDTH`] cells. A cell is therefore a tenth of an inch, and the two shells agree about
/// what an unsized column is without either of them saying so twice.
pub const MM_PER_CELL: f64 = 25.4 / DEFAULT_WIDTH as f64;

/// The narrowest a sized column is drawn, and the widest.
///
/// A document may legitimately hold a 0.5mm column — LibreOffice writes one for a spacer — and
/// three cells is the least that can show a truncated value plus its separator. The ceiling is so
/// that one absurd column cannot take the whole window and leave nothing to scroll with; the
/// document keeps its own width either way, because nothing here is ever written back.
pub const MIN_WIDTH: u16 = 3;
pub const MAX_WIDTH: u16 = 60;

/// The sizes a document gave particular columns, over a default for the rest, plus which of them
/// it says are not there at all.
///
/// Sparse, because a spreadsheet sizes a handful of columns and leaves thousands alone —
/// `App::col_widths` hands back exactly the ones it stored, in the ODF lengths it stored them as,
/// and this is where those become cells.
#[derive(Clone, Debug, Default)]
pub struct Tracks {
    sized: HashMap<u32, u16>,
    hidden: HashSet<u32>,
}

impl Tracks {
    /// Build from `(index, length)` pairs as the core reports them, and the columns it says are
    /// hidden.
    ///
    /// A length nothing can parse is left at the default rather than treated as zero, which is the
    /// same tolerance the readers apply: a column that vanished because of a typo in a style would
    /// be worse than one that is the ordinary width.
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

/// An ODF length as a whole number of terminal cells, clamped to what a grid can draw.
///
/// `None` for a length this build cannot read, which [`Tracks::new`] turns into the default.
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

/// The other direction: the ODF length a column of `cells` terminal cells is, for `:width` to
/// write back.
///
/// A width set here is a width every other shell honours, because it goes into the document in the
/// document's own unit rather than as a terminal's idea of one.
pub fn length(cells: u16) -> String {
    mm_length(f64::from(cells.max(1)) * MM_PER_CELL)
}

/// The columns visible from `first`, and how wide each is drawn: as many as fit in `room` cells,
/// skipping the hidden ones, and never fewer than one.
///
/// The last column may be **clipped** rather than dropped — a grid that showed only whole columns
/// would jump by a whole column at a time when the window is resized by one cell, and a partial
/// column is what every spreadsheet draws at the right-hand edge.
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

/// The first column to show so that `active` is on screen, given where the view is now.
///
/// Moves the least it can — a cell already in view leaves the grid exactly where it is, which is
/// the difference between moving a cursor and having the sheet jump under you. The same rule
/// `follow_cursor` always applied, now with columns that are not all the same width.
///
/// Scrolling *forward* walks back from `active` accumulating widths rather than trying one first
/// column after another: `G` on a wide sheet is a jump of sixteen thousand columns, and a loop
/// that stepped one at a time would do that arithmetic sixteen thousand times per frame.
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

/// The rows drawn from `first` down: `count` of them, skipping the ones the document folds away.
///
/// A filtered row is *absent*, not blank — the same thing [`columns`] does to a hidden column on
/// the other axis, and the reason a filtered sheet fills the window instead of trailing off into
/// gaps a third of the way down.
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

/// Pad or truncate `text` to exactly `width` **terminal cells**, one trailing space as the column
/// separator, with the text pushed to one side of what is left.
///
/// Measured with `unicode-width` rather than `chars().count()`: a CJK ideograph is two cells and a
/// combining mark is none, so counting characters would draw a column of Japanese one cell wider
/// per character than the header above it. Truncation stops on a whole character and then pads,
/// so a wide character never half-lands in the last cell.
pub fn pad(text: &str, width: usize, align: Align) -> String {
    let room = width.saturating_sub(1);
    let mut kept = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > room {
            break;
        }
        used += w;
        kept.push(c);
    }
    let spare = room - used;
    let (before, after) = match align {
        Align::Left => (0, spare),
        Align::Right => (spare, 0),
        Align::Centre => (spare / 2, spare - spare / 2),
    };
    let mut out = " ".repeat(before);
    out.push_str(&kept);
    out.push_str(&" ".repeat(after));
    // The column separator, which every alignment keeps.
    out.push(' ');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

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
        assert_eq!(pad("12", 6, Align::Right), "   12 ");
        assert_eq!(pad("ab", 6, Align::Left), "ab    ");
        assert_eq!(pad("ab", 7, Align::Centre), "  ab   ");
        for align in [Align::Left, Align::Centre, Align::Right] {
            assert_eq!(pad("overlong text", 6, align).width(), 6);
        }
    }

    /// The half of the old `ponytail` that was about *characters*: two ideographs are four cells,
    /// so a ten-cell column holds four of them and a space, not nine.
    #[test]
    fn a_wide_character_takes_two_cells_of_its_column() {
        let text = "\u{4e16}\u{754c}\u{4e16}\u{754c}\u{4e16}";
        let padded = pad(text, 10, Align::Left);
        assert_eq!(padded.width(), 10, "{padded:?}");
        assert_eq!(
            padded.chars().filter(|c| *c == '\u{4e16}').count(),
            2,
            "four cells of ideograph fit in nine, not nine of them: {padded:?}"
        );
        // And never half a character in the last cell.
        assert_eq!(pad("\u{4e16}", 2, Align::Left).width(), 2);
    }
}
