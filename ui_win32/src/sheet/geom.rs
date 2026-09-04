// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a cell is, in pixels — and which cell a pixel is in.
//!
//! **No Windows types, and no `unsafe`.** This is the grid's whole pixel arithmetic as pure
//! functions, which is the half of a native shell that can be unit-tested on the Linux machine
//! this repository is developed on (`doc/windows-shell.md`, "The crate": the `[W]` split is the
//! design rather than an accident of it).
//!
//! It is deliberately a *smaller* module than `ui_sheet_gtk/src/geom.rs`, whose shape it
//! follows: the same prefix-sum axis, the same two coordinate spaces, and none of the parts
//! that belong to features this shell does not have yet — no resize edges (W1 draws, it does
//! not drag), no fill handle, no filter buttons, no name-hint placement.
//!
//! Two coordinate spaces, and mixing them is the bug this module exists to prevent:
//!
//! * **content** — `(0, 0)` is cell A1's top-left corner, and it extends to the sheet's full
//!   1048576 × 16384. Nothing scrolls here.
//! * **client** — `(0, 0)` is the window's client-area top-left, so the header band sits at
//!   `x < header_w` / `y < header_h` and the content is offset by the scroll position.
//!
//! [`GridGeom::cell_rect`] converts one way and [`GridGeom::hit`] the other; they are tested as
//! a round trip, because a rectangle that does not contain the point that produced it is the
//! whole class of off-by-one this module can have.

use std::ops::Range;

/// ODF's sheet bounds, which are also the scrollable extent (§3.2). The core's, not a second
/// opinion: a scrollbar that ended somewhere the reader does not is a bug waiting.
pub use grind_sheet::{MAX_COLS, MAX_ROWS};

/// Pixels to an ODF millimetre, at the notional 96 dpi every Windows API calls 100%.
///
/// A length in a document is physical (§5.4) and a window is not, so something has to choose.
/// The DPI of the monitor the window is on multiplies this — see [`scale`] — so a column set to
/// 2.5cm is 2.5cm on a correctly configured screen and consistent everywhere else.
pub const PX_PER_MM: f64 = 96.0 / 25.4;

/// A length at 100% scaling, in the pixels a display at `dpi` actually has.
///
/// The one place this shell turns a design measurement into a device one. Per-monitor DPI v2
/// means the answer changes when the window is dragged between monitors, so nothing measured is
/// ever *stored* scaled — the same rule the GTK window's zoom follows.
pub fn scale(value: f64, dpi: u32) -> f64 {
    value * f64::from(dpi) / 96.0
}

/// A rectangle in client space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// The same rectangle as the integer edges GDI actually draws between.
    ///
    /// Rounded rather than truncated, and as *edges* rather than as an origin and a size, so
    /// that two adjacent cells share a boundary instead of leaving a seam that lights up at
    /// some scroll offsets and not others.
    pub fn edges(&self) -> (i32, i32, i32, i32) {
        let left = self.x.round() as i32;
        let top = self.y.round() as i32;
        let right = (self.x + self.w).round() as i32;
        let bottom = (self.y + self.h).round() as i32;
        (left, top, right, bottom)
    }
}

/// What sits under a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Cell { row: u32, col: u32 },
    RowHeader(u32),
    ColHeader(u32),
    Corner,
}

/// How big each track on one axis is — column widths, or row heights.
///
/// One type for both, because the arithmetic is the same and a sheet that got its columns right
/// and its rows wrong is the bug two copies would produce. A document sizes a handful of tracks
/// out of sixteen thousand, so the sparse list plus a running total is what makes [`Sizes::at`]
/// a binary search rather than a walk from column A.
#[derive(Clone, Debug, PartialEq)]
pub struct Sizes {
    default: f64,
    count: u32,
    /// Ascending by index, distinct: the tracks the document gave a size of their own.
    sizes: Vec<(u32, f64)>,
    /// `run[k]` is how much the first `k` entries have displaced everything after them — the
    /// sum of their sizes *minus* what the default would have been. An offset is then
    /// `index * default + run[entries before it]`, with no walk over the sheet.
    run: Vec<f64>,
}

impl Sizes {
    /// `sizes` is taken in any order; ties keep the last one given.
    pub fn new(default: f64, count: u32, mut sizes: Vec<(u32, f64)>) -> Self {
        // Reversed first, so that a stable sort leaves the *last* entry given for an index at
        // the front of its run and `dedup` keeps that one.
        sizes.reverse();
        sizes.sort_by_key(|(i, _)| *i);
        sizes.dedup_by_key(|(i, _)| *i);
        // A zero is kept: that is a hidden track — a row a filter excludes (§9.4) — and it has
        // to displace nothing rather than fall back to the default.
        sizes.retain(|(i, size)| *i < count && *size >= 0.0);
        let mut run = Vec::with_capacity(sizes.len() + 1);
        let mut acc = 0.0;
        run.push(acc);
        for (_, size) in &sizes {
            acc += size - default;
            run.push(acc);
        }
        Self {
            default,
            count,
            sizes,
            run,
        }
    }

    /// The axis a document describes: its sized tracks as ODF lengths, turned into pixels.
    ///
    /// The one place a physical length becomes a device one, so that "columns are as wide as
    /// the document says" is a property of this function rather than of every caller. A length
    /// this build cannot parse falls back to the default width rather than to zero, because
    /// zero means *hidden* and silently hiding a column is worse than mis-sizing one.
    ///
    /// `hidden` is the tracks the document hides, which is a **separate question from their
    /// size** — ODF hides a track with `table:visibility="collapse"` (§5.4), not by giving it a
    /// width of zero, so a hidden column usually still carries a perfectly ordinary
    /// `style:column-width`. Reading only the widths is why `hidden-rows-cols.fods` first drew
    /// with every column showing. They go on *after* the lengths, so a hidden track is hidden
    /// whatever size it was given: [`Sizes::new`] keeps the last entry for an index, and this
    /// relies on it.
    pub fn from_lengths(
        default: f64,
        count: u32,
        lengths: &[(u32, String)],
        hidden: &[u32],
        dpi: u32,
    ) -> Self {
        let mut sizes: Vec<(u32, f64)> = lengths
            .iter()
            .filter_map(|(index, length)| {
                Some((
                    *index,
                    scale(grind_core::style::length_mm(length)? * PX_PER_MM, dpi),
                ))
            })
            .collect();
        sizes.extend(hidden.iter().map(|index| (*index, 0.0)));
        Self::new(default, count, sizes)
    }

    /// How many entries lie strictly before `index`.
    fn before(&self, index: u32) -> usize {
        self.sizes.partition_point(|(i, _)| *i < index)
    }

    pub fn size_of(&self, index: u32) -> f64 {
        match self.sizes.binary_search_by_key(&index, |(i, _)| *i) {
            Ok(k) => self.sizes[k].1,
            Err(_) => self.default,
        }
    }

    /// Content-space offset of a track's leading edge. `count` itself is the far end.
    pub fn offset_of(&self, index: u32) -> f64 {
        f64::from(index) * self.default + self.run[self.before(index)]
    }

    /// The track containing a content-space offset, clamped to the sheet.
    pub fn at(&self, offset: f64) -> u32 {
        let offset = offset.max(0.0);
        // The last sized track that starts at or before `offset`; everything else is one of the
        // uniform runs, before it or after it.
        let k = self
            .sizes
            .partition_point(|(i, _)| self.offset_of(*i) <= offset);
        let (from, at) = match k {
            0 => (0, 0.0),
            k => {
                let (i, size) = self.sizes[k - 1];
                let end = self.offset_of(i) + size;
                if offset < end {
                    return i;
                }
                (i + 1, end)
            }
        };
        let index = u64::from(from) + ((offset - at) / self.default) as u64;
        index.min(u64::from(self.count.saturating_sub(1))) as u32
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    /// Whether this track is a hidden one — zero size, kept explicitly rather than falling back
    /// to the default (see [`Sizes::new`]).
    pub fn is_hidden(&self, index: u32) -> bool {
        self.size_of(index) == 0.0
    }

    /// The first track at or after `from` that has any width, or `None` past the end.
    ///
    /// Scrolling has to skip hidden tracks or the view stops moving: pressing the scrollbar's
    /// arrow lands on a zero-width column, which occupies no pixels, and the screen does not
    /// change while the position number does.
    pub fn next_visible(&self, from: u32) -> Option<u32> {
        (from..self.count).find(|i| !self.is_hidden(*i))
    }

    /// The last track at or before `from` that has any width, or `None` before the start.
    pub fn prev_visible(&self, from: u32) -> Option<u32> {
        (0..=from.min(self.count.saturating_sub(1)))
            .rev()
            .find(|i| !self.is_hidden(*i))
    }

    /// The first track that can sit at the top (or left) of a `span`-pixel view without
    /// leaving blank space after the last one — the scrollbar's maximum position.
    ///
    /// Answered by walking back from the end rather than by dividing, because the tracks near
    /// the end may be any size at all.
    pub fn last_start(&self, span: f64) -> u32 {
        let mut used = 0.0;
        let mut index = self.count;
        while index > 0 {
            let size = self.size_of(index - 1);
            if used + size > span && used > 0.0 {
                break;
            }
            used += size;
            index -= 1;
        }
        index.min(self.count.saturating_sub(1))
    }
}

/// Everything needed to place a cell: the header band, the two axes' sizes, and which track is
/// at the top-left corner of the view.
///
/// The scroll position is a **track index rather than a pixel offset**, which is a decision and
/// not an omission. A Win32 scrollbar's range is an `i32`, and this sheet is 1048576 rows tall:
/// in pixels that is twenty million and the thumb is quantised to something coarse anyway,
/// whereas in rows it is exact. Excel scrolls by whole rows for the same reason, so the
/// arithmetic and the convention agree here.
#[derive(Clone, Debug, PartialEq)]
pub struct GridGeom {
    pub header_w: f64,
    pub header_h: f64,
    /// The height of the status bar at the foot of the window; the grid stops above it.
    pub status_h: f64,
    pub rows: Sizes,
    pub cols: Sizes,
    pub first_row: u32,
    pub first_col: u32,
    /// The client area, in pixels.
    pub width: f64,
    pub height: f64,
    /// The DPI every measurement above was built at.
    ///
    /// Carried here rather than passed alongside, because the painter needs it too and two
    /// copies of one number is how a `WM_DPICHANGED` that rebuilt the geometry but not the
    /// fonts happens.
    pub dpi: u32,
}

impl GridGeom {
    /// The rectangle the cells occupy — the client area less the headers and the status bar.
    pub fn body(&self) -> Rect {
        Rect {
            x: self.header_w,
            y: self.header_h,
            w: (self.width - self.header_w).max(0.0),
            h: (self.height - self.header_h - self.status_h).max(0.0),
        }
    }

    pub fn status_rect(&self) -> Rect {
        Rect {
            x: 0.0,
            y: (self.height - self.status_h).max(0.0),
            w: self.width,
            h: self.status_h.min(self.height),
        }
    }

    /// Content-space distance from the sheet's origin to the top-left of the body.
    fn scroll_x(&self) -> f64 {
        self.cols.offset_of(self.first_col)
    }

    fn scroll_y(&self) -> f64 {
        self.rows.offset_of(self.first_row)
    }

    /// Where a cell is drawn, in client space. Off-screen answers are returned rather than
    /// clipped — the caller is iterating a viewport it already asked for.
    pub fn cell_rect(&self, row: u32, col: u32) -> Rect {
        Rect {
            x: self.header_w + self.cols.offset_of(col) - self.scroll_x(),
            y: self.header_h + self.rows.offset_of(row) - self.scroll_y(),
            w: self.cols.size_of(col),
            h: self.rows.size_of(row),
        }
    }

    /// One column's header button.
    pub fn col_header_rect(&self, col: u32) -> Rect {
        let cell = self.cell_rect(0, col);
        Rect {
            x: cell.x,
            y: 0.0,
            w: cell.w,
            h: self.header_h,
        }
    }

    /// One row's header button.
    pub fn row_header_rect(&self, row: u32) -> Rect {
        let cell = self.cell_rect(row, 0);
        Rect {
            x: 0.0,
            y: cell.y,
            w: self.header_w,
            h: cell.h,
        }
    }

    /// The rows that intersect the body, as a half-open range.
    ///
    /// One past the last *partly* visible row, so a half-drawn row at the bottom edge is drawn
    /// rather than left as a gap — which is what a viewport request wants.
    pub fn visible_rows(&self) -> Range<u32> {
        let body = self.body();
        let end = self.rows.at(self.scroll_y() + body.h) + 1;
        self.first_row..end.min(self.rows.count()).max(self.first_row)
    }

    pub fn visible_cols(&self) -> Range<u32> {
        let body = self.body();
        let end = self.cols.at(self.scroll_x() + body.w) + 1;
        self.first_col..end.min(self.cols.count()).max(self.first_col)
    }

    /// What sits under a client-space point.
    pub fn hit(&self, x: f64, y: f64) -> Hit {
        let body = self.body();
        let col = || self.cols.at(x - body.x + self.scroll_x());
        let row = || self.rows.at(y - body.y + self.scroll_y());
        if body.contains(x, y) {
            return Hit::Cell {
                row: row(),
                col: col(),
            };
        }
        // Outside the cells, so one of the three bands. The status bar counts as the corner:
        // it is not the grid, and reporting a row for a point in it would be a lie a resize
        // drag (W2) would then act on.
        match (x >= body.x, y >= body.y && y < body.y + body.h) {
            (true, false) if y < body.y => Hit::ColHeader(col()),
            (false, true) => Hit::RowHeader(row()),
            _ => Hit::Corner,
        }
    }

    /// The scrollbar's maximum first row, so that the last row can reach the top of the body
    /// and no further.
    pub fn max_first_row(&self) -> u32 {
        self.rows.last_start(self.body().h)
    }

    pub fn max_first_col(&self) -> u32 {
        self.cols.last_start(self.body().w)
    }

    /// Move the view by whole tracks, skipping hidden ones and stopping at the ends.
    ///
    /// Both scroll paths go through here — the scrollbars and the wheel — so that a wheel
    /// notch and three clicks of the arrow land in the same place.
    pub fn scroll_rows(&mut self, delta: i64) {
        self.first_row = step(&self.rows, self.first_row, delta, self.max_first_row());
    }

    pub fn scroll_cols(&mut self, delta: i64) {
        self.first_col = step(&self.cols, self.first_col, delta, self.max_first_col());
    }

    /// How many rows a PageDown moves — the body's worth, and never fewer than one, so that a
    /// window shorter than a single row still goes somewhere.
    pub fn page_rows(&self) -> i64 {
        let body = self.body();
        let end = self.rows.at(self.scroll_y() + body.h);
        i64::from(end.saturating_sub(self.first_row)).max(1)
    }

    pub fn page_cols(&self) -> i64 {
        let body = self.body();
        let end = self.cols.at(self.scroll_x() + body.w);
        i64::from(end.saturating_sub(self.first_col)).max(1)
    }
}

/// One axis' scroll step: `delta` visible tracks from `from`, clamped to `[0, max]`.
fn step(sizes: &Sizes, from: u32, delta: i64, max: u32) -> u32 {
    let mut at = from;
    for _ in 0..delta.unsigned_abs() {
        let next = match delta > 0 {
            true => at.checked_add(1).and_then(|i| sizes.next_visible(i)),
            false => at.checked_sub(1).and_then(|i| sizes.prev_visible(i)),
        };
        match next {
            Some(next) => at = next,
            None => break,
        }
    }
    at.min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom() -> GridGeom {
        GridGeom {
            header_w: 40.0,
            header_h: 20.0,
            status_h: 22.0,
            rows: Sizes::new(20.0, MAX_ROWS, vec![(1, 60.0)]),
            cols: Sizes::new(80.0, MAX_COLS, vec![(0, 30.0), (2, 150.0)]),
            first_row: 0,
            first_col: 0,
            width: 800.0,
            height: 600.0,
            dpi: 96,
        }
    }

    #[test]
    fn an_offset_is_the_sum_of_everything_before_it() {
        let s = Sizes::new(80.0, 100, vec![(3, 200.0), (1, 20.0)]);
        assert_eq!(s.offset_of(0), 0.0);
        assert_eq!(s.offset_of(1), 80.0);
        assert_eq!(s.offset_of(2), 100.0);
        assert_eq!(s.offset_of(3), 180.0);
        assert_eq!(s.offset_of(4), 380.0);
        assert_eq!(s.total_check(), 380.0 + 96.0 * 80.0);
    }

    impl Sizes {
        fn total_check(&self) -> f64 {
            self.offset_of(self.count)
        }
    }

    #[test]
    fn every_offset_lands_back_in_its_own_track() {
        let s = Sizes::new(80.0, 100, vec![(3, 200.0), (1, 20.0), (7, 0.0)]);
        for i in 0..100u32 {
            if s.is_hidden(i) {
                continue;
            }
            let start = s.offset_of(i);
            assert_eq!(s.at(start), i, "start of {i}");
            assert_eq!(s.at(start + s.size_of(i) - 0.5), i, "end of {i}");
        }
    }

    /// The round trip this module exists for: every rectangle contains the point that made it.
    #[test]
    fn a_cell_rect_contains_its_own_hit() {
        let mut g = geom();
        for (first_row, first_col) in [(0, 0), (3, 1), (17, 9)] {
            g.first_row = first_row;
            g.first_col = first_col;
            for row in g.visible_rows() {
                for col in g.visible_cols() {
                    let r = g.cell_rect(row, col);
                    if r.w == 0.0 || r.h == 0.0 {
                        continue;
                    }
                    let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                    if !g.body().contains(x, y) {
                        continue; // partly scrolled off the bottom or right edge
                    }
                    assert_eq!(g.hit(x, y), Hit::Cell { row, col }, "{row},{col}");
                    assert!(r.contains(x, y));
                }
            }
        }
    }

    #[test]
    fn the_headers_and_the_corner_are_where_they_look() {
        let g = geom();
        assert_eq!(g.hit(4.0, 4.0), Hit::Corner);
        assert_eq!(g.hit(45.0, 4.0), Hit::ColHeader(0));
        assert_eq!(g.hit(4.0, 25.0), Hit::RowHeader(0));
        assert_eq!(g.hit(4.0, 45.0), Hit::RowHeader(1));
        // The status bar is not the grid: a point in it is neither a row header nor a cell.
        assert_eq!(g.hit(4.0, 590.0), Hit::Corner);
    }

    /// A column the document sized is that wide in pixels, and the ones after it move over.
    /// This is the W1 exit criterion, as arithmetic.
    #[test]
    fn a_documents_own_widths_decide_the_geometry() {
        let lengths = vec![(0, "2.5cm".to_string()), (2, "10mm".to_string())];
        let cols = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], 96);
        assert!((cols.size_of(0) - 25.0 * PX_PER_MM).abs() < 1e-9);
        assert!((cols.size_of(2) - 10.0 * PX_PER_MM).abs() < 1e-9);
        assert_eq!(cols.size_of(1), 80.0, "unsized columns keep the default");
        assert!((cols.offset_of(1) - 25.0 * PX_PER_MM).abs() < 1e-9);
    }

    /// A length this build cannot parse must not become a hidden column.
    #[test]
    fn an_unreadable_length_falls_back_to_the_default() {
        let lengths = vec![(0, "wide".to_string())];
        let cols = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], 96);
        assert_eq!(cols.size_of(0), 80.0);
    }

    #[test]
    fn dpi_multiplies_every_width() {
        let lengths = vec![(0, "25.4mm".to_string())];
        let at_96 = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], 96);
        let at_192 = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], 192);
        assert!((at_96.size_of(0) - 96.0).abs() < 1e-9);
        assert!((at_192.size_of(0) - 192.0).abs() < 1e-9);
    }

    /// A column ODF hides carries a perfectly ordinary width, and `table:visibility` is what
    /// makes it gone. Reading only the widths drew every column of `hidden-rows-cols.fods`.
    #[test]
    fn a_hidden_track_is_hidden_whatever_width_it_was_given() {
        let lengths = vec![(1, "2.5cm".to_string())];
        let cols = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[1], 96);
        assert_eq!(cols.size_of(1), 0.0);
        assert!(cols.is_hidden(1));
        // It displaces nothing: column C starts where column B would have.
        assert_eq!(cols.offset_of(2), 80.0);
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut g = geom();
        g.scroll_rows(-5);
        assert_eq!(g.first_row, 0, "there is nothing above row 1");
        g.scroll_rows(4);
        assert_eq!(g.first_row, 4);
        g.scroll_rows(i64::from(MAX_ROWS) * 2);
        assert_eq!(g.first_row, g.max_first_row());
        // The last row is reachable and the view does not scroll past it into blank space.
        assert!(g.visible_rows().contains(&(MAX_ROWS - 1)));
    }

    /// A hidden track occupies no pixels, so scrolling has to step over it — otherwise the
    /// scrollbar position changes and the screen does not.
    #[test]
    fn scrolling_steps_over_hidden_tracks() {
        let mut g = geom();
        g.cols = Sizes::new(80.0, MAX_COLS, vec![(1, 0.0), (2, 0.0)]);
        g.scroll_cols(1);
        assert_eq!(g.first_col, 3);
        g.scroll_cols(-1);
        assert_eq!(g.first_col, 0);
    }

    #[test]
    fn a_page_is_a_bodys_worth_of_rows() {
        let g = geom();
        // 600 tall, less a 20px header and a 22px status bar: 558px of body. Row 1 is 60 tall
        // and the rest are 20, so that is row 0, row 1, and 24 more.
        assert_eq!(g.page_rows(), 25);
        assert_eq!(g.visible_rows(), 0..26);
    }

    #[test]
    fn a_window_too_small_for_one_row_still_scrolls() {
        let mut g = geom();
        g.height = g.header_h + g.status_h;
        assert_eq!(g.page_rows(), 1);
        g.scroll_rows(g.page_rows());
        assert_eq!(g.first_row, 1);
    }

    #[test]
    fn edges_are_shared_so_adjacent_cells_leave_no_seam() {
        let g = geom();
        let a = g.cell_rect(0, 0).edges();
        let b = g.cell_rect(0, 1).edges();
        assert_eq!(a.2, b.0, "A's right edge is B's left edge");
    }
}
