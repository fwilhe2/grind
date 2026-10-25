// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pixels ↔ cells: how much of the sheet fits, and where the view must be to show the selection.
//! Which cell the pointer landed on is the DOM's answer (every cell carries its address); what is
//! left is what the platform cannot know — how many rows and columns to ask
//! [`grind_sheet::App::get_viewport`] for, and how to keep the active cell on screen. **A column
//! is as wide as the document says** ([`Tracks`]), widths accumulating from the scroll position.

use std::collections::HashMap;

use grind_sheet::Pos;
use grind_sheet::style::length_mm;

/// How many CSS pixels one millimetre is drawn as: 96 per inch, which the CSS specification itself
/// fixes.
pub const PX_PER_MM: f64 = 96.0 / 25.4;

/// The size of one cell, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub cell_w: f64,
    pub cell_h: f64,
}

/// The cell size the grid is drawn at, written into `--cell-w`/`--cell-h` by `start` so the
/// stylesheet cannot disagree. **Declared, not measured**: measuring a rendered cell makes the
/// viewport a function of the layout it produced, with no fixed point — a table stretched to its
/// container gained a narrower column every repaint. Both sides stay in CSS pixels, page zoom
/// included.
pub const CELL: Metrics = Metrics {
    cell_w: 96.0,
    cell_h: 24.0,
};

/// How wide one character of a cell's text is taken to be, in CSS pixels, and a cell's padding
/// either side — estimated, as the chart's tick text is, since measuring a rendered cell is the
/// loop `CELL` rules out.
const FIT_CHAR_W: f64 = 7.4;
const FIT_PAD: f64 = 14.0;

/// The grid's estimate as a `Metrics`, so `grind_sheet::fit` can measure with it: every
/// character [`text_cells`] wide at [`FIT_CHAR_W`] a cell, scaled by the cell's own font size.
pub struct Estimate;

impl grind_core::layout::Metrics for Estimate {
    fn advances(&self, text: &str, style: &grind_core::style::TextStyle, out: &mut Vec<f32>) {
        let scale = grind_sheet::look::font_scale(style.font_size.as_deref()).unwrap_or(1.0);
        let mut x = 0.0;
        for c in text.chars() {
            x += text_cells(c.encode_utf8(&mut [0; 4])) as f64 * FIT_CHAR_W * scale;
            out.push(x as f32);
        }
    }

    fn line_height(&self, _: &grind_core::style::TextStyle) -> f32 {
        CELL.cell_h as f32
    }
}

/// The length column `col` fits to, from [`Estimate`] — `None` for an empty column, which
/// goes back to the default width (`grind_sheet::fit::width`).
pub fn fit_width(app: &grind_sheet::App, sheet: usize, col: u32) -> Option<String> {
    let widest = grind_sheet::fit::widest_drawn(app, sheet, col, &Estimate);
    grind_sheet::fit::width(widest, FIT_PAD, 1.0 / PX_PER_MM)
}

/// The columns of a text, counted in cells: an East Asian ideograph or full-width form takes two.
pub fn text_cells(text: &str) -> usize {
    text.chars()
        .map(|c| match c as u32 {
            0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1FAFF => 2,
            _ => 1,
        })
        .sum()
}

/// The sizes a document gave particular columns or rows, over a default for the rest — sparse, as
/// `App::col_widths` returns only those stored, as ODF lengths.
#[derive(Clone, Debug, Default)]
pub struct Tracks {
    sized: HashMap<u32, f64>,
    default: f64,
}

impl Tracks {
    /// Build from the core's `(index, length)` pairs. A length that will not parse is the default,
    /// not zero.
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

/// Where the view must be for `active` to be on screen, moving as little as possible, so a click
/// never scrolls the sheet from under the pointer. `visible` is `(rows, cols)` from
/// [`Metrics::visible`] less the spare row and column it adds: a half-drawn cell is not on screen.
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
    #[test]
    fn a_fitted_column_grows_with_its_widest_text_and_an_empty_one_has_no_width() {
        use grind_sheet::{App, RecalcMode};
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "short", RecalcMode::Document)
            .unwrap();
        app.enter(
            0,
            Pos::new(0, 1),
            "a good deal longer",
            RecalcMode::Document,
        )
        .unwrap();
        let mm = |col| length_mm(&fit_width(&app, 0, col).unwrap()).unwrap();
        assert!(mm(1) > mm(0));
        assert_eq!(fit_width(&app, 0, 2), None);
        assert_eq!(text_cells("日本"), 4);
        assert_eq!(text_cells("abc"), 3);
    }

    use super::*;

    /// The column count must not depend on what was drawn: a cell size measured from a rendered
    /// table stretched to its container asked for one more column every repaint. Every width is
    /// declared instead — the stylesheet's default or the document's own.
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
        // A length that will not parse leaves the column ordinary rather than gone.
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
