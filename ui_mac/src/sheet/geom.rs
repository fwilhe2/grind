// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where every cell of a sheet is, in points — pure arithmetic, **no AppKit types**.
//!
//! The Mac's grid is a flipped `NSView` that is as large as the part of the sheet worth
//! scrolling to, inside an `NSScrollView` that does the scrolling (decision 3: AppKit draws the
//! chrome). So unlike `ui_win32`'s `GridGeom`, whose scroll position is a track index, everything
//! here is in **document coordinates**: a cell's rectangle is where it is on the sheet, and the
//! scroll view's clip decides which of them are on screen. The two header bands are floating
//! subviews of the scroll view and use the same coordinates along their own axis.
//!
//! The axes are `grind_sheet::tracks::Sizes` (M1), at 72 points to the inch.

use std::ops::Range;

use grind_core::layout::{Fragment, Metrics, wrap};
use grind_sheet::tracks::Sizes;
use grind_sheet::{App, MAX_COLS, MAX_ROWS, look};

/// A column nobody sized: **one inch**, the suite's own default column (`ui_web`'s 96 CSS pixels,
/// `ui_tui`'s ten cells), in points.
pub const COL_W: f64 = 72.0;

/// A row nobody sized: a quarter of an inch, `ui_web`'s 24 CSS pixels.
pub const ROW_H: f64 = 18.0;

/// The row header's width and the column header's height — the two bands the scroll view floats
/// over the grid.
pub const HEADER_W: f64 = 44.0;
pub const HEADER_H: f64 = 22.0;

/// A millimetre in points, which is what [`Sizes::from_lengths`] is handed.
pub const PT_PER_MM: f64 = 72.0 / 25.4;

/// How much sheet lies beyond the last used cell before the grid stops — room to see that a
/// sheet ends where it ends, and to scroll a little past it, without a view 18 million points
/// tall. `NSScrollView` scrolls anything, but a document view sized to the whole sheet would ask
/// every overlay, ruler and accessibility client to think about a million rows.
pub const MARGIN_ROWS: u32 = 100;
pub const MARGIN_COLS: u32 = 26;

/// A rectangle in points, origin top left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    /// The part of `self` inside `other`, which is empty when they do not meet.
    pub fn intersection(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Rect {
            x,
            y,
            w: (self.right().min(other.right()) - x).max(0.0),
            h: (self.bottom().min(other.bottom()) - y).max(0.0),
        }
    }

    /// The same rectangle moved by `(dx, dy)`.
    pub fn offset(&self, dx: f64, dy: f64) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }
}

/// One sheet's two axes, and how far the grid reaches along each.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub cols: Sizes,
    pub rows: Sizes,
    /// How many columns and rows the grid view is sized to hold — the used extent and a margin,
    /// never the whole sheet.
    pub shown_cols: u32,
    pub shown_rows: u32,
}

impl Grid {
    /// The grid of `sheet` in `app`: the document's own widths and heights, its hidden tracks
    /// hidden, and the used part of the sheet with a margin after it.
    ///
    /// A sheet that cannot be read — a stale index after a sheet was deleted — is an empty
    /// default grid rather than an error, since this answers what to *draw* and the next
    /// observer notification brings the right sheet.
    pub fn of(app: &App, sheet: usize) -> Grid {
        let widths = app.col_widths(sheet).unwrap_or_default();
        let heights = app.row_heights(sheet).unwrap_or_default();
        let hidden_cols = app.hidden_cols(sheet).unwrap_or_default();
        // Hidden by hand, or by the sheet's filter — two questions `App` answers separately, and
        // a row either one hides takes no room.
        let mut hidden_rows = app.manually_hidden_rows(sheet).unwrap_or_default();
        hidden_rows.extend(app.hidden_rows(sheet).unwrap_or_default());
        let (used_rows, used_cols) = app.used_extent(sheet).unwrap_or((0, 0));
        let mm = |mm: f64| mm * PT_PER_MM;
        Grid {
            cols: Sizes::from_lengths(COL_W, MAX_COLS, &widths, &hidden_cols, mm),
            rows: Sizes::from_lengths(ROW_H, MAX_ROWS, &heights, &hidden_rows, mm),
            shown_cols: used_cols.saturating_add(MARGIN_COLS).min(MAX_COLS),
            shown_rows: used_rows.saturating_add(MARGIN_ROWS).min(MAX_ROWS),
        }
    }

    /// The grid of `sheet` as [`Grid::of`] reads it, and every row with no height of its own
    /// grown to hold what is in it — a cell that wraps onto more lines, or one set in a larger
    /// face — measured in `metrics` by `grind_core::layout::wrap`, the breaker that draws the
    /// wrapped lines (L3, the GNOME window's rule). A row is only ever grown, never shrunk below
    /// [`ROW_H`]; one the document sized keeps its size, and a hidden one stays hidden.
    pub fn measured(app: &App, sheet: usize, metrics: &dyn Metrics) -> Grid {
        let mut grid = Grid::of(app, sheet);
        let grown = auto_heights(app, sheet, &grid, metrics);
        if !grown.is_empty() {
            let heights = app.row_heights(sheet).unwrap_or_default();
            let mut sizes: Vec<(u32, f64)> = grown;
            // The document's own heights, and its hidden rows, go on after — `Sizes::new` keeps
            // the last entry given for a row, so they win.
            sizes.extend(heights.iter().filter_map(|(row, length)| {
                Some((*row, grind_core::style::length_mm(length)? * PT_PER_MM))
            }));
            let mut hidden = app.manually_hidden_rows(sheet).unwrap_or_default();
            hidden.extend(app.hidden_rows(sheet).unwrap_or_default());
            sizes.extend(hidden.into_iter().map(|row| (row, 0.0)));
            grid.rows = Sizes::new(ROW_H, MAX_ROWS, sizes);
        }
        grid
    }

    /// How large the grid view is: every shown column's width and every shown row's height.
    // Reached from the grid view, which sizes itself by this, and not yet written.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn size(&self) -> (f64, f64) {
        (
            self.cols.offset_of(self.shown_cols),
            self.rows.offset_of(self.shown_rows),
        )
    }

    /// Where one cell is on the sheet. A hidden track gives it no width or no height.
    pub fn cell(&self, row: u32, col: u32) -> Rect {
        Rect {
            x: self.cols.offset_of(col),
            y: self.rows.offset_of(row),
            w: self.cols.size_of(col),
            h: self.rows.size_of(row),
        }
    }

    /// The columns whose boxes meet `x..x + w`, as a range to read with `App::get_viewport`.
    pub fn cols_in(&self, x: f64, w: f64) -> Range<u32> {
        span(&self.cols, x, w, self.shown_cols)
    }

    /// The rows whose boxes meet `y..y + h`.
    pub fn rows_in(&self, y: f64, h: f64) -> Range<u32> {
        span(&self.rows, y, h, self.shown_rows)
    }

    /// Which cell a point on the sheet is in — nearest, never nothing, so a click past the last
    /// shown row still lands on a cell.
    // Reached from a click on the grid, which is M3's.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn hit(&self, x: f64, y: f64) -> (u32, u32) {
        let row = self
            .rows
            .at(y.max(0.0))
            .min(self.shown_rows.saturating_sub(1));
        let col = self
            .cols
            .at(x.max(0.0))
            .min(self.shown_cols.saturating_sub(1));
        (row, col)
    }
}

/// Space above and below a cell's text — `paint::PAD_Y` either side.
const ROW_PAD: f64 = 2.0 * super::paint::PAD_Y;

/// How much sheet is measured for natural row heights. A row above the view still displaces
/// the ones below it, so the pass cannot be limited to what is on screen — past this much
/// document every row keeps its height, the GNOME window's bound.
const AUTO_HEIGHT_CELLS: u64 = 200_000;

/// The rows `grid` should grow, and to what: for each, the tallest of its cells that wraps or
/// names a font size, when that is taller than the row. A cell with no style is never laid out,
/// which is what keeps this a cheap pass over a sheet where nine cells in ten are plain.
fn auto_heights(app: &App, sheet: usize, grid: &Grid, metrics: &dyn Metrics) -> Vec<(u32, f64)> {
    let Ok((rows, cols)) = app.used_extent(sheet) else {
        return Vec::new();
    };
    if rows == 0 || cols == 0 || u64::from(rows) * u64::from(cols) > AUTO_HEIGHT_CELLS {
        return Vec::new();
    }
    let Ok(viewport) = app.get_viewport(sheet, 0..rows, 0..cols) else {
        return Vec::new();
    };
    let mut grown = Vec::new();
    for row in 0..rows {
        let mut tallest: f64 = 0.0;
        for col in 0..cols {
            let Some(style) = viewport.style(row, col) else {
                continue;
            };
            let wrapping = look::wraps(Some(style));
            if !wrapping && style.font_size.is_none() {
                continue;
            }
            let Some(text) = viewport.text(row, col).filter(|text| !text.is_empty()) else {
                continue;
            };
            let text_style = look::text_style(Some(style));
            // A width of zero is `wrap`'s own "do not wrap": one line per hard break, which is
            // what a cell here only for its larger face wants.
            let width = match wrapping {
                true => (grid.cols.size_of(col) - 2.0 * super::paint::PAD_X).max(1.0) as f32,
                false => 0.0,
            };
            let fragment = Fragment {
                text,
                style: &text_style,
            };
            let laid = wrap(std::slice::from_ref(&fragment), width, metrics);
            tallest = tallest.max(f64::from(laid.height()));
        }
        if tallest + ROW_PAD > ROW_H {
            grown.push((row, (tallest + ROW_PAD).ceil()));
        }
    }
    grown
}

/// The tracks of `sizes` meeting `start..start + len`, clamped to the first `shown`.
fn span(sizes: &Sizes, start: f64, len: f64, shown: u32) -> Range<u32> {
    let first = sizes.at(start.max(0.0)).min(shown);
    let end = start + len.max(0.0);
    let mut last = sizes.at(end.max(0.0));
    // `at` answers the track an offset falls *in*; the one it starts is past the range.
    if last < shown && sizes.offset_of(last) < end {
        last += 1;
    }
    first..last.clamp(first, shown)
}

/// The label a column header shows — `A`, `Z`, `AA` — spelled by the formula lexer, as every
/// other grid's header is.
pub fn column_label(col: u32) -> String {
    grind_sheet::formula::lex::column_name(col)
}

/// The label a row header shows, 1-based.
pub fn row_label(row: u32) -> String {
    (u64::from(row) + 1).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::{Pos, RecalcMode};

    fn app() -> App {
        let app = App::new();
        app.enter(0, Pos::new(4, 2), "x", RecalcMode::Document)
            .unwrap();
        app
    }

    #[test]
    fn an_unsized_sheet_is_inches_and_quarter_inches() {
        let grid = Grid::of(&app(), 0);
        assert_eq!(grid.cell(0, 0), Rect::new(0.0, 0.0, COL_W, ROW_H));
        assert_eq!(
            grid.cell(2, 3),
            Rect::new(3.0 * COL_W, 2.0 * ROW_H, COL_W, ROW_H)
        );
    }

    /// The view is the used part of the sheet and a margin, not a million rows.
    #[test]
    fn the_grid_reaches_past_the_used_extent_by_a_margin() {
        let grid = Grid::of(&app(), 0);
        assert_eq!(grid.shown_rows, 5 + MARGIN_ROWS);
        assert_eq!(grid.shown_cols, 3 + MARGIN_COLS);
        let (w, h) = grid.size();
        assert_eq!(w, f64::from(grid.shown_cols) * COL_W);
        assert_eq!(h, f64::from(grid.shown_rows) * ROW_H);
    }

    #[test]
    fn a_document_width_is_honoured_and_a_hidden_column_takes_no_room() {
        let app = app();
        app.set_col_width(0, 1..2, Some("1in".into())).unwrap();
        app.set_col_hidden(0, 2..3, true).unwrap();
        let grid = Grid::of(&app, 0);
        assert!(
            (grid.cell(0, 1).w - 72.0).abs() < 1e-9,
            "an inch is 72 points"
        );
        assert_eq!(grid.cell(0, 2).w, 0.0, "hidden");
        assert_eq!(
            grid.cell(0, 3).x,
            grid.cell(0, 2).x,
            "the next column closes up"
        );
    }

    #[test]
    fn the_tracks_in_a_rectangle_are_the_ones_it_meets() {
        let grid = Grid::of(&app(), 0);
        assert_eq!(grid.cols_in(0.0, COL_W), 0..1, "exactly one column");
        assert_eq!(grid.cols_in(10.0, COL_W), 0..2, "straddling two");
        assert_eq!(grid.rows_in(ROW_H * 3.0, ROW_H * 2.0), 3..5);
        assert_eq!(grid.rows_in(0.0, 0.0), 0..0, "nothing");
        let past = grid.size().1 + 500.0;
        assert!(grid.rows_in(past, 100.0).is_empty(), "past the shown rows");
    }

    #[test]
    fn a_click_lands_on_the_nearest_cell() {
        let grid = Grid::of(&app(), 0);
        assert_eq!(grid.hit(COL_W + 1.0, ROW_H * 2.0 + 1.0), (2, 1));
        assert_eq!(grid.hit(-5.0, -5.0), (0, 0));
        let (w, h) = grid.size();
        assert_eq!(
            grid.hit(w + 1000.0, h + 1000.0),
            (grid.shown_rows - 1, grid.shown_cols - 1)
        );
    }

    #[test]
    fn headers_are_labelled_the_way_a_person_counts() {
        assert_eq!(column_label(0), "A");
        assert_eq!(column_label(26), "AA");
        assert_eq!(row_label(0), "1");
    }

    #[test]
    fn rectangles_meet_and_move() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersection(&b), Rect::new(5.0, 5.0, 5.0, 5.0));
        assert!(a.intersection(&Rect::new(20.0, 20.0, 1.0, 1.0)).is_empty());
        assert_eq!(a.offset(1.0, 2.0), Rect::new(1.0, 2.0, 10.0, 10.0));
    }

    #[test]
    fn a_wrapping_row_grows_and_a_sized_one_keeps_its_height() {
        let app = App::new();
        let long = "one two three four five six seven eight nine ten eleven twelve";
        for row in 0..2 {
            app.enter(0, Pos::new(row, 0), long, RecalcMode::Document)
                .unwrap();
        }
        let wrapping = grind_sheet::style::CellStyle {
            wrap: Some("wrap".into()),
            ..Default::default()
        };
        app.set_style(0, Pos::new(0, 0), Pos::new(1, 0), Some(wrapping))
            .unwrap();
        app.set_row_height(0, 1..2, Some("10mm".into())).unwrap();
        /// Six points a character and thirteen a line, near enough a 10-point face.
        struct Body;
        impl Metrics for Body {
            fn advances(&self, text: &str, _: &grind_core::style::TextStyle, out: &mut Vec<f32>) {
                out.clear();
                out.extend((1..=text.chars().count()).map(|n| n as f32 * 6.0));
            }
            fn line_height(&self, _: &grind_core::style::TextStyle) -> f32 {
                13.0
            }
        }
        let grid = Grid::measured(&app, 0, &Body);
        assert!(grid.rows.size_of(0) > ROW_H, "{}", grid.rows.size_of(0));
        assert!((grid.rows.size_of(1) - 10.0 * PT_PER_MM).abs() < 1e-9);
        assert_eq!(grid.rows.size_of(2), ROW_H, "an empty row");
        assert_eq!(
            Grid::of(&app, 0).rows.size_of(0),
            ROW_H,
            "only the measured grid grows"
        );
    }
}
