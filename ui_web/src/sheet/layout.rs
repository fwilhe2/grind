// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pixels ↔ cells: how much of the sheet fits, and where the view has to be to show
//! the selection.
//!
//! One of the two halves of this shell that runs without a browser, and it is
//! deliberately small: *which* cell the pointer landed on is answered by the DOM
//! (every cell carries its address, and a click reads it off the target), because
//! hit-testing is something the platform already does. What is left is the part the
//! platform cannot know — how many rows and columns to ask [`grind_sheet::App::get_viewport`]
//! for, and how to keep the active cell on screen.
//!
//! **A column is as wide as the document says it is** ([`Tracks`]). That used to be a
//! `ponytail` here — every column one width — and it was the difference between opening
//! somebody's spreadsheet and opening an approximation of it. The viewport arithmetic
//! accumulates widths from the scroll position rather than dividing by one, which is the
//! same prefix-sum shape `ui_sheet_gtk/src/geom.rs` grew in M8.

use std::collections::HashMap;

use grind_sheet::Pos;
use grind_sheet::style::length_mm;

/// How many CSS pixels one millimetre is drawn as.
///
/// The number the whole page is sized in: a document's own lengths are physical
/// (`"2.258cm"`) and a browser's are not, so one conversion has to be declared. 96 CSS
/// pixels per inch is the one the CSS specification itself fixes, which makes this the
/// browser's own answer rather than this shell's opinion.
pub const PX_PER_MM: f64 = 96.0 / 25.4;

/// The size of one cell, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub cell_w: f64,
    pub cell_h: f64,
}

/// The cell size the grid is drawn at, and the shell's own number rather than the
/// page's: `start` writes it into `--cell-w`/`--cell-h`, so the stylesheet cannot
/// disagree with the arithmetic here.
///
/// It is *declared*, not measured, and that is the point. Measuring a rendered cell
/// makes the viewport a function of the layout the viewport produced, and the loop
/// has no fixed point: a table stretched to its container puts one more column on
/// screen every repaint, each one narrower than the last. Both sides of the page's
/// size question stay in CSS pixels — `clientWidth` is too — so nothing is lost by
/// not measuring, page zoom included.
pub const CELL: Metrics = Metrics {
    cell_w: 96.0,
    cell_h: 24.0,
};

/// The sizes a document gave particular columns or rows, over a default for the rest.
///
/// Sparse, because a spreadsheet sizes a handful of tracks and leaves thousands alone —
/// `App::col_widths` hands back exactly the ones it stored, in the ODF lengths it stored them
/// as, and this is where those become pixels.
#[derive(Clone, Debug, Default)]
pub struct Tracks {
    sized: HashMap<u32, f64>,
    default: f64,
}

impl Tracks {
    /// Build from `(index, length)` pairs as the core reports them. A length nothing can
    /// parse is left at the default rather than treated as zero, which is the same §9
    /// tolerance the readers apply — a column that vanished would be worse than one that is
    /// the ordinary width.
    pub fn new(default: f64, sized: impl IntoIterator<Item = (u32, String)>) -> Self {
        Tracks {
            sized: sized
                .into_iter()
                .filter_map(|(at, length)| Some((at, length_mm(&length)? * PX_PER_MM)))
                .collect(),
            default,
        }
    }

    /// One track's size, in CSS pixels. Never zero: a hidden track is hidden by the *core*
    /// (`hidden_rows`/`hidden_cols`), and a zero here would be a track nothing could click.
    pub fn size(&self, at: u32) -> f64 {
        self.sized
            .get(&at)
            .copied()
            .filter(|px| *px > 0.5)
            .unwrap_or(self.default)
    }

    /// Whether this track was given a size of its own — what decides between writing a width
    /// into the `<col>` and letting the stylesheet's own default stand.
    pub fn is_sized(&self, at: u32) -> bool {
        self.sized.contains_key(&at)
    }

    /// How wide (or tall) the tracks from `from` up to but not including `to` are, together.
    ///
    /// What a chart's own position is measured against: the frame sits at a distance from the
    /// table's corner, and the corner may have been scrolled past.
    pub fn span(&self, from: u32, to: u32) -> f64 {
        // The sized ones are the only ones worth looking up; the rest are one multiplication.
        let count = to.saturating_sub(from);
        let sized: f64 = self
            .sized
            .iter()
            .filter(|(at, _)| (from..to).contains(*at))
            .map(|(_, px)| *px)
            .sum();
        let sized_count = self
            .sized
            .keys()
            .filter(|at| (from..to).contains(at))
            .count() as u32;
        sized + f64::from(count - sized_count) * self.default
    }

    /// How many tracks starting at `from` fit in `px`, and one more — the same "draw the one
    /// being scrolled onto" rule the uniform version had, and never fewer than one.
    pub fn fit(&self, from: u32, px: f64) -> u32 {
        if !px.is_finite() || px < 1.0 {
            return 1;
        }
        let mut used = 0.0;
        let mut count = 0;
        // Bounded: a page a hundred thousand pixels wide of one-pixel columns is still a
        // page, but asking the core for that viewport is not what anybody meant.
        while used < px && count < MAX_VISIBLE {
            used += self.size(from.saturating_add(count));
            count += 1;
        }
        count.max(1) + 1
    }
}

/// The most tracks either axis will ask for in one viewport.
const MAX_VISIBLE: u32 = 512;

/// Where the view has to be for `active` to be on screen, given where it is now.
///
/// Moves as little as possible — a selection already inside the view leaves it
/// alone, which is what keeps a click from scrolling the sheet out from under the
/// pointer. `visible` is `(rows, cols)` and comes from [`Metrics::visible`], minus
/// the spare row and column it adds: a cell only half drawn is not on screen.
pub fn follow(scroll: Pos, active: Pos, visible: (u32, u32)) -> Pos {
    let axis = |offset: u32, at: u32, span: u32| {
        let span = span.max(1);
        match at {
            at if at < offset => at,
            at if at >= offset + span => at + 1 - span,
            _ => offset,
        }
    };
    Pos::new(
        axis(scroll.row, active.row, visible.0.saturating_sub(1)),
        axis(scroll.col, active.col, visible.1.saturating_sub(1)),
    )
}

/// Scroll by whole cells, clamped at the top-left corner. The sheet has no bottom
/// to clamp against — scrolling into blank space is normal (`Viewport`'s own rule).
pub fn scrolled_by(scroll: Pos, rows: i64, cols: i64) -> Pos {
    let shift = |at: u32, by: i64| (at as i64 + by).clamp(0, u32::MAX as i64) as u32;
    Pos::new(shift(scroll.row, rows), shift(scroll.col, cols))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this shell shipped with: the cell size was *measured* from a rendered
    /// cell, and a table stretched to its container reports a narrower cell than the
    /// one the shell drew — so every repaint asked for one more column, and typing
    /// grew the grid without end. Every width here is declared instead: the
    /// stylesheet's default, or the document's own. Declared sizes have a fixed
    /// point; measured ones did not.
    #[test]
    fn the_column_count_does_not_depend_on_what_was_drawn() {
        let tracks = Tracks::new(CELL.cell_w, [(0, "8cm".to_owned())]);
        let first = tracks.fit(0, 1280.0);
        assert_eq!(tracks.fit(0, 1280.0), first, "a repaint changes nothing");
    }

    #[test]
    fn an_unsized_track_is_the_default_and_a_sized_one_is_itself() {
        let tracks = Tracks::new(96.0, [(2, "2.54cm".to_owned()), (4, "nonsense".to_owned())]);
        assert_eq!(tracks.size(0), 96.0);
        // One inch, in the pixels the CSS specification fixes an inch at.
        assert!((tracks.size(2) - 96.0).abs() < 0.5, "{}", tracks.size(2));
        assert!(tracks.is_sized(2));
        // A length this build cannot parse leaves the column ordinary rather than gone.
        assert_eq!(tracks.size(4), 96.0);
        assert!(!tracks.is_sized(4));
    }

    #[test]
    fn the_viewport_accumulates_widths_rather_than_dividing_by_one() {
        // Three narrow columns then the default: 40 + 40 + 40 = 120 of a 200px page, so the
        // fourth (96px) is what fills it — four that fit, plus the spare.
        let narrow = "1.058cm".to_owned(); // ≈ 40px
        let tracks = Tracks::new(
            96.0,
            [(0, narrow.clone()), (1, narrow.clone()), (2, narrow)],
        );
        assert_eq!(tracks.fit(0, 200.0), 5);
        // From the fourth column on it is the plain 96px grid again.
        assert_eq!(tracks.fit(3, 200.0), 4);
    }

    #[test]
    fn a_span_adds_the_sized_tracks_to_the_default_ones() {
        let tracks = Tracks::new(10.0, [(1, "2.54cm".to_owned())]);
        assert_eq!(tracks.span(0, 0), 0.0);
        assert_eq!(tracks.span(0, 1), 10.0);
        // One default and one inch.
        assert!(
            (tracks.span(0, 2) - 106.0).abs() < 0.5,
            "{}",
            tracks.span(0, 2)
        );
        // Past the sized one it is the default again.
        assert_eq!(tracks.span(2, 5), 30.0);
    }

    #[test]
    fn a_page_with_no_size_yet_still_asks_for_a_track() {
        let tracks = Tracks::new(96.0, []);
        assert_eq!(tracks.fit(0, 0.0), 1);
        assert_eq!(tracks.fit(0, f64::NAN), 1);
    }

    /// A run of hairline columns must not ask the core for a million cells.
    #[test]
    fn the_viewport_is_bounded_however_narrow_the_columns_are() {
        let tracks = Tracks::new(0.4, []);
        assert_eq!(tracks.fit(0, 100_000.0), MAX_VISIBLE + 1);
    }

    #[test]
    fn a_selection_already_on_screen_does_not_scroll() {
        let scroll = Pos::new(10, 4);
        assert_eq!(follow(scroll, Pos::new(12, 5), (11, 6)), scroll);
    }

    #[test]
    fn the_view_follows_the_selection_by_the_least_it_can() {
        // Above the view: the row becomes the top one.
        assert_eq!(follow(Pos::new(10, 0), Pos::new(3, 0), (11, 6)).row, 3);
        // Below it: the row becomes the last fully drawn one, not the top.
        assert_eq!(follow(Pos::new(0, 0), Pos::new(20, 0), (11, 6)).row, 11);
        // The same rule sideways.
        assert_eq!(follow(Pos::new(0, 8), Pos::new(0, 2), (11, 6)).col, 2);
        assert_eq!(follow(Pos::new(0, 0), Pos::new(0, 9), (11, 6)).col, 5);
    }

    #[test]
    fn scrolling_stops_at_the_first_cell() {
        assert_eq!(scrolled_by(Pos::new(2, 2), -5, -5), Pos::new(0, 0));
        assert_eq!(scrolled_by(Pos::new(2, 2), 3, 1), Pos::new(5, 3));
    }
}
