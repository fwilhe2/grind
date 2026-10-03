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
//! not drag), no fill handle, no name-hint placement. [`GridGeom::filter_button`] is the one
//! exception, `ui_sheet_gtk`'s own `GridGeom::filter_button` mirrored.
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

/// A filter dropdown button's (min, max) side length, in design units — `GridGeom::filter_button`
/// clamps a cell's own height into this band, the same pair `ui_sheet_gtk/src/geom.rs` uses.
const FILTER_BUTTON: (f64, f64) = (9.0, 18.0);

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
    Cell {
        row: u32,
        col: u32,
    },
    RowHeader(u32),
    ColHeader(u32),
    /// The button where the two header bands meet, which selects the whole sheet.
    Corner,
    /// Window furniture that is not the grid — the name-box strip above it and the status bar
    /// below. A separate answer from [`Hit::Corner`] rather than a convenient synonym for it,
    /// because the corner *does* something when it is clicked and the status bar must not do
    /// that thing.
    Chrome,
}

/// A header boundary that can be grabbed to size a track: the column or row whose far edge it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Col(u32),
    Row(u32),
}

/// How big each track on one axis is — `grind_sheet::tracks`, hoisted out of this file and
/// `ui_sheet_gtk`'s when the macOS shell would have been a third copy (`doc/macos-shell.md`, M1).
pub use grind_sheet::tracks::Sizes;

/// A millimetre in this shell's unit, at `dpi` — what [`Sizes::from_lengths`] is handed.
///
/// Converted to 96-dpi pixels first and *then* scaled, the order this shell always had, so
/// that a frame rendered before the hoist and after it is the same bytes.
pub fn mm_to_px(dpi: u32) -> impl Fn(f64) -> f64 {
    move |mm| scale(mm * PX_PER_MM, dpi)
}

/// How close to a header boundary a point counts as being on it, in pixels at 100%.
const EDGE_GRAB: f64 = 4.0;

/// The number picker's width at 100% — room for `Date Time` or `General` and the chevron.
pub const NUMBER_W: f64 = 104.0;

/// The two decimal buttons' width at 100% — `.00` and a little air, not a square.
pub const DECIMALS_W: f64 = 40.0;

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
    /// The **format strip** at the very top — `doc/windows-shell.md` decision 4's drawn strip for
    /// a property of the selection, over the grid at last (`sheet/format.rs`). Above the name box
    /// and the formula bar rather than below them, which is where a spreadsheet keeps its
    /// formatting and what keeps the formula bar against the cells it reads; and where the text
    /// pane's own strip is, so the two panes' strips sit in one place in one window.
    pub format_h: f64,
    /// The strip under it that holds the name box and, beside it, the
    /// formula bar. Everything below is offset by it, which is why it is part of the geometry
    /// rather than a constant the painter knows.
    pub strip_h: f64,
    /// The notice bar under the strip, and **zero when there is no notice** — which is the whole
    /// of how a banner appears and disappears. Carried as a height rather than as a flag so that
    /// every rectangle below it is one arithmetic expression whether or not it is showing.
    pub banner_h: f64,
    /// The assist band under the notice bar, and zero when there is nothing to assist with —
    /// the same appear-and-disappear arrangement as `banner_h`, for the same reason.
    ///
    /// A second band rather than a second use of the first, because the two answer different
    /// questions and are routinely up together: a formula that would not parse leaves a notice on
    /// screen *while the edit is still open*, which is exactly when the signature hint is most
    /// worth having. Sharing one row would mean the more useful of the two hiding the other.
    pub hint_h: f64,
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
    /// The zoom, a factor on every cell (the tracks are built already scaled by it; this is what
    /// the painter needs to scale what it draws in them). 1.0 is 100%.
    pub zoom: f64,
}

impl GridGeom {
    /// Where the column header band starts — under the strip, and under whichever of the two
    /// bands are showing. Named because five rectangles below depend on it and a banner that
    /// moved four of them would be a very confusing bug.
    pub fn header_top(&self) -> f64 {
        self.format_h + self.strip_h + self.banner_h + self.hint_h
    }

    /// The rectangle the cells occupy — the client area less the strip, the banner, the headers
    /// and the status bar.
    pub fn body(&self) -> Rect {
        Rect {
            x: self.header_w,
            y: self.header_top() + self.header_h,
            w: (self.width - self.header_w).max(0.0),
            h: (self.height - self.header_top() - self.header_h - self.status_h).max(0.0),
        }
    }

    /// The format strip, across the very top of the window.
    pub fn format_rect(&self) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: self.width,
            h: self.format_h.min(self.height),
        }
    }

    /// Every control on the format strip, where it goes and the separator before it —
    /// [`crate::strip::lay_out`] over [`format::CONTROLS`](crate::sheet::format::CONTROLS), in that
    /// order, so an index into one is an index into the other.
    pub fn format_controls(&self) -> Vec<crate::strip::Placed> {
        use crate::sheet::format::{CONTROLS, Control, Shape};
        let widths: Vec<(f64, bool)> = CONTROLS
            .iter()
            .map(|&(control, shape, group)| {
                let width = match (shape, control) {
                    (Shape::Toggle | Shape::Swatch, _) => crate::theme::space::CONTROL_H,
                    (Shape::Picker, _) => NUMBER_W,
                    // The text strip's own, so the one verb both strips carry is one size.
                    (Shape::Button, Control::Clear) => crate::text::geom::CLEAR_W,
                    (Shape::Button, _) => DECIMALS_W,
                };
                (width, group)
            })
            .collect();
        crate::strip::lay_out(self.format_rect(), self.dpi, &widths)
    }

    /// Which format-strip control a point is on, if any.
    pub fn format_hit(&self, x: f64, y: f64) -> Option<crate::sheet::format::Control> {
        if !self.format_rect().contains(x, y) {
            return None;
        }
        let index = crate::strip::hit(&self.format_controls(), x, y)?;
        crate::sheet::format::CONTROLS
            .get(index)
            .map(|(control, ..)| *control)
    }

    /// The strip under the format strip, holding the name box and the formula bar.
    pub fn strip_rect(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.format_h,
            w: self.width,
            h: self.strip_h.min((self.height - self.format_h).max(0.0)),
        }
    }

    /// The notice bar, which is empty when [`GridGeom::banner_h`] is zero.
    pub fn banner_rect(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.format_h + self.strip_h,
            w: self.width,
            h: self.banner_h,
        }
    }

    /// The assist band, under the notice bar and over the headers, and empty when
    /// [`GridGeom::hint_h`] is zero.
    pub fn hint_rect(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.format_h + self.strip_h + self.banner_h,
            w: self.width,
            h: self.hint_h,
        }
    }

    /// The name box inside the strip: the width of the row header band plus a column, so that it
    /// is wide enough for `Sheet1.AA1234` without being wide enough to look like a formula bar.
    ///
    /// **A 32-pixel control centred in a 44-pixel strip** (W10), which is Fluent's own control
    /// height and its own 12-pixel margin rather than a fraction of whatever the strip happens to
    /// be — the previous arrangement made the field as tall as the strip less a twelfth, so
    /// growing the strip grew the field with it and the whole row read as one slab.
    pub fn name_box_rect(&self) -> Rect {
        let margin = scale(crate::theme::space::GROUP, self.dpi);
        let h = scale(crate::theme::space::CONTROL_H, self.dpi).min(self.strip_h);
        Rect {
            x: margin,
            y: self.format_h + ((self.strip_h - h) / 2.0).max(0.0),
            w: (self.header_w * 3.0).min((self.width - margin * 2.0).max(0.0)),
            h,
        }
    }

    /// The formula bar: the rest of the strip, after the name box.
    ///
    /// Two read-outs on one strip and nothing else, which is `doc/windows-shell.md` decision 4's
    /// rule for it — *where* the selection is and *what is in it*. It is not a place verbs may
    /// go, which is what keeps it a strip rather than a second toolbar.
    pub fn formula_rect(&self) -> Rect {
        let margin = scale(crate::theme::space::GROUP, self.dpi);
        let name = self.name_box_rect();
        let x = name.x + name.w + scale(crate::theme::space::GAP * 2.0, self.dpi);
        Rect {
            x,
            y: name.y,
            w: (self.width - x - margin).max(0.0),
            h: name.h,
        }
    }

    /// [`card_in`] at this window's scaling — the notice bar and the assist band.
    pub fn card_in(&self, band: Rect) -> Rect {
        card_in(band, self.dpi)
    }

    /// Where the in-cell editor goes: the active cell, clipped to the body.
    ///
    /// Clipped rather than clamped, because a child control drawn over the header band or the
    /// status bar would sit on top of chrome the painter owns. A cell scrolled entirely out of
    /// sight gives an empty rectangle, and the caller's answer to that is to put the editor on
    /// the formula bar instead.
    pub fn editor_rect(&self, row: u32, col: u32) -> Rect {
        let body = self.body();
        let cell = self.cell_rect(row, col);
        let left = cell.x.max(body.x);
        let top = cell.y.max(body.y);
        let right = (cell.x + cell.w).min(body.x + body.w);
        let bottom = (cell.y + cell.h).min(body.y + body.h);
        Rect {
            x: left,
            y: top,
            w: (right - left).max(0.0),
            h: (bottom - top).max(0.0),
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
            y: self.header_top() + self.header_h + self.rows.offset_of(row) - self.scroll_y(),
            w: self.cols.size_of(col),
            h: self.rows.size_of(row),
        }
    }

    /// Where an autofilter's dropdown button sits inside one cell of its heading row — a
    /// square anchored to the cell's right edge, the GTK grid's `filter_button` mirrored.
    /// `None` when the cell is too small to hold a button and still show any of its text.
    pub fn filter_button(&self, row: u32, col: u32) -> Option<Rect> {
        let cell = self.cell_rect(row, col);
        let (min, max) = (
            scale(FILTER_BUTTON.0, self.dpi),
            scale(FILTER_BUTTON.1, self.dpi),
        );
        let size = (cell.h - 2.0).clamp(min, max);
        if cell.h < min || cell.w < size * 2.0 {
            return None;
        }
        Some(Rect {
            x: cell.x + cell.w - size - 1.0,
            y: cell.y + (cell.h - size) / 2.0,
            w: size,
            h: size,
        })
    }

    /// One column's header button.
    pub fn col_header_rect(&self, col: u32) -> Rect {
        let cell = self.cell_rect(0, col);
        Rect {
            x: cell.x,
            y: self.header_top(),
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
    ///
    /// The two bands that are *not* the grid — the strip at the top and the status bar at the
    /// foot — answer [`Hit::Chrome`], and that is load-bearing rather than tidy: a click on the
    /// corner button selects the whole sheet, so folding the status bar in with it would make
    /// clicking the status bar select a million rows.
    pub fn hit(&self, x: f64, y: f64) -> Hit {
        let body = self.body();
        if y < self.header_top() || y >= body.y + body.h {
            return Hit::Chrome;
        }
        let col = || self.cols.at(x - body.x + self.scroll_x());
        let row = || self.rows.at(y - body.y + self.scroll_y());
        match (x >= body.x, y >= body.y) {
            (true, true) => Hit::Cell {
                row: row(),
                col: col(),
            },
            (true, false) => Hit::ColHeader(col()),
            (false, true) => Hit::RowHeader(row()),
            (false, false) => Hit::Corner,
        }
    }

    /// The header boundary under a client-space point, if one is within a few pixels of it: the
    /// right edge of a column in the column band, the bottom edge of a row in the row band. A
    /// hidden track has no edge to grab (it takes no room), and the boundary a point is nearer to
    /// wins, so two narrow columns can each be reached.
    pub fn edge_at(&self, x: f64, y: f64) -> Option<Edge> {
        let body = self.body();
        let grab = scale(EDGE_GRAB, self.dpi);
        let top = self.header_top();
        if y >= top && y < body.y && x >= body.x {
            let along = x - body.x + self.scroll_x();
            let col = self.cols.at(along);
            let near = |c: u32| {
                let edge = self.cols.offset_of(c + 1);
                (self.cols.size_of(c) > 0.0 && (edge - along).abs() <= grab)
                    .then_some((c, (edge - along).abs()))
            };
            let before = col.checked_sub(1).and_then(near);
            return [near(col), before]
                .into_iter()
                .flatten()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(c, _)| Edge::Col(c));
        }
        if x >= 0.0 && x < body.x && y >= body.y && y < body.y + body.h {
            let along = y - body.y + self.scroll_y();
            let row = self.rows.at(along);
            let near = |r: u32| {
                let edge = self.rows.offset_of(r + 1);
                (self.rows.size_of(r) > 0.0 && (edge - along).abs() <= grab)
                    .then_some((r, (edge - along).abs()))
            };
            let before = row.checked_sub(1).and_then(near);
            return [near(row), before]
                .into_iter()
                .flatten()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(r, _)| Edge::Row(r));
        }
        None
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

/// A band drawn as an **inset card** rather than edge to edge: the notice bar in either pane, and
/// the grid's assist band. Each of those is one sentence *about* something rather than a piece of
/// the window's structure, and Fluent's `InfoBar` is a rounded rectangle with a margin round it —
/// the difference between that and a full-bleed stripe is most of why the W9 chrome read as a
/// stack of toolbars.
///
/// The horizontal margin is the same [`crate::theme::space::GROUP`] the grid's strip keeps for
/// its fields, so the left edges of the name box and every band below it line up down the window.
/// A free function because both panes need it and neither owns the other's geometry.
pub fn card_in(band: Rect, dpi: u32) -> Rect {
    let margin = scale(crate::theme::space::GROUP, dpi);
    let gap = scale(crate::theme::space::GAP, dpi);
    Rect {
        x: margin,
        y: band.y + gap,
        w: (band.w - margin * 2.0).max(0.0),
        h: (band.h - gap * 2.0).max(0.0),
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
            format_h: 0.0,
            strip_h: 0.0,
            banner_h: 0.0,
            hint_h: 0.0,
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
            zoom: 1.0,
        }
    }

    /// A header boundary is grabbed within a few pixels either side of it, in its own band only,
    /// and not when the track it ends takes no room.
    #[test]
    fn a_header_boundary_can_be_grabbed() {
        let g = geom();
        // Column 0 is 30 wide, so its far edge is at x = 40 + 30 = 70, in the header band (y < 20).
        assert_eq!(g.edge_at(70.0, 10.0), Some(Edge::Col(0)));
        assert_eq!(
            g.edge_at(73.0, 10.0),
            Some(Edge::Col(0)),
            "a few pixels either side"
        );
        assert_eq!(g.edge_at(120.0, 10.0), None, "the middle of a column");
        assert_eq!(g.edge_at(70.0, 100.0), None, "not in the cells");
        // Row 0 is 20 tall under a 20 tall band: its far edge is at y = 40, in the row band.
        assert_eq!(g.edge_at(10.0, 40.0), Some(Edge::Row(0)));
        assert_eq!(g.edge_at(10.0, 25.0), None);
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
    fn the_filter_button_sits_at_the_right_end_of_its_cell() {
        let g = geom();
        let cell = g.cell_rect(0, 1); // an 80x20 cell — column 1 carries no size of its own
        let b = g.filter_button(0, 1).expect("an 80x20 cell has room");
        assert!(
            b.contains(cell.x + cell.w - 4.0, cell.y + cell.h / 2.0),
            "inside"
        );
        assert!(
            !b.contains(cell.x + 4.0, cell.y + cell.h / 2.0),
            "not over the text"
        );
        assert!(
            b.x >= cell.x && b.x + b.w <= cell.x + cell.w && b.y >= cell.y,
            "within the cell: {b:?} in {cell:?}"
        );

        let narrow = GridGeom {
            cols: Sizes::new(80.0, MAX_COLS, vec![(1, 12.0)]),
            ..geom()
        };
        assert_eq!(narrow.filter_button(0, 1), None, "no room at all");
    }

    #[test]
    fn the_headers_and_the_corner_are_where_they_look() {
        let g = geom();
        assert_eq!(g.hit(4.0, 4.0), Hit::Corner);
        assert_eq!(g.hit(45.0, 4.0), Hit::ColHeader(0));
        assert_eq!(g.hit(4.0, 25.0), Hit::RowHeader(0));
        assert_eq!(g.hit(4.0, 45.0), Hit::RowHeader(1));
        // The status bar is not the grid, and it is not the corner either — clicking the
        // corner selects the whole sheet, and clicking the status bar must not.
        assert_eq!(g.hit(4.0, 590.0), Hit::Chrome);
    }

    /// The strip pushes everything down by its own height, and is itself neither a header nor
    /// the corner — the name box lives in it and a click there belongs to the name box.
    #[test]
    fn the_strip_takes_its_height_off_the_top_and_owns_its_own_clicks() {
        let mut g = geom();
        g.strip_h = 30.0;
        assert_eq!(g.hit(4.0, 4.0), Hit::Chrome);
        assert_eq!(g.hit(200.0, 15.0), Hit::Chrome);
        // Everything below it has moved down by exactly the strip's height.
        assert_eq!(g.hit(45.0, 4.0 + 30.0), Hit::ColHeader(0));
        assert_eq!(g.hit(4.0, 25.0 + 30.0), Hit::RowHeader(0));
        assert_eq!(g.body().y, 50.0);
        assert_eq!(g.cell_rect(0, 0).y, 50.0);
        assert_eq!(g.col_header_rect(0).y, 30.0);
        // And it costs the body exactly that much height, so a page is shorter.
        assert_eq!(g.body().h, 600.0 - 30.0 - 20.0 - 22.0);
    }

    /// The two bands stack rather than share, and each one costs the grid exactly its own
    /// height — a notice and a signature hint are routinely up together.
    #[test]
    fn the_notice_bar_and_the_assist_band_stack_under_the_strip() {
        let mut g = geom();
        g.strip_h = 30.0;
        let plain = g.body();
        g.banner_h = 26.0;
        g.hint_h = 24.0;
        assert_eq!(g.banner_rect().y, 30.0);
        assert_eq!(g.hint_rect().y, 30.0 + 26.0, "the hint is under the notice");
        assert_eq!(g.header_top(), 30.0 + 26.0 + 24.0);
        assert_eq!(g.body().h, plain.h - 26.0 - 24.0);
        // Neither band is the grid: a click on one must not reach the corner button.
        assert_eq!(g.hit(4.0, 35.0), Hit::Chrome);
        assert_eq!(g.hit(4.0, 60.0), Hit::Chrome);
        assert_eq!(g.hit(4.0, g.header_top() + 2.0), Hit::Corner);
    }

    #[test]
    fn the_name_box_sits_inside_the_strip() {
        let mut g = geom();
        g.strip_h = crate::sheet::draw::STRIP_H;
        let box_ = g.name_box_rect();
        assert!(box_.y > 0.0 && box_.y + box_.h <= g.strip_h);
        assert!(box_.x > 0.0 && box_.w > 0.0);
        // A control of Fluent's own height, centred — not the strip less a fraction of itself,
        // which is what made the whole row read as one slab before W10.
        assert_eq!(box_.h, crate::theme::space::CONTROL_H);
        assert_eq!(box_.y, (g.strip_h - box_.h) / 2.0);
        assert_eq!(box_.x, crate::theme::space::GROUP);
    }

    /// A band that is a sentence is drawn as a card inside its own band, with the same margin the
    /// strip's fields keep — so the left edges of the name box, the notice bar and the assist
    /// band line up down the window.
    #[test]
    fn an_inset_band_keeps_the_strips_own_margin() {
        let mut g = geom();
        g.strip_h = crate::sheet::draw::STRIP_H;
        g.banner_h = crate::sheet::draw::BANNER_H;
        let card = g.card_in(g.banner_rect());
        assert_eq!(card.x, g.name_box_rect().x, "the left edges line up");
        assert_eq!(card.x + card.w, g.width - card.x, "and it is symmetrical");
        assert!(card.y > g.banner_rect().y);
        assert!(card.y + card.h < g.banner_rect().y + g.banner_h);
    }

    /// Two read-outs on one strip, side by side and not overlapping — the whole of decision 4's
    /// rule for it, as arithmetic.
    #[test]
    fn the_formula_bar_takes_the_rest_of_the_strip() {
        let mut g = geom();
        g.strip_h = crate::sheet::draw::STRIP_H;
        let name = g.name_box_rect();
        let formula = g.formula_rect();
        assert!(formula.x > name.x + name.w, "they overlap");
        assert!(formula.x + formula.w <= g.width);
        assert_eq!(formula.y, name.y, "one strip, one baseline");
        assert_eq!(formula.h, name.h);
        // A window narrower than the name box leaves no formula bar rather than a negative one.
        g.width = 10.0;
        assert_eq!(g.formula_rect().w, 0.0);
    }

    /// The banner takes its height out of the body and pushes the headers down with it, and a
    /// document with no notice pays nothing at all.
    #[test]
    fn a_banner_pushes_the_grid_down_and_costs_nothing_when_it_is_absent() {
        let mut g = geom();
        g.strip_h = 28.0;
        let without = g.body();
        assert_eq!(g.banner_rect().h, 0.0);
        g.banner_h = 24.0;
        assert_eq!(g.body().y, without.y + 24.0);
        assert_eq!(g.body().h, without.h - 24.0);
        assert_eq!(g.banner_rect().y, 28.0);
        assert_eq!(g.col_header_rect(0).y, 28.0 + 24.0);
        // And a click in it is chrome, not a cell — the same answer the strip gets.
        assert_eq!(g.hit(200.0, 40.0), Hit::Chrome);
        assert_eq!(g.hit(45.0, 28.0 + 24.0 + 4.0), Hit::ColHeader(0));
    }

    /// The editor sits on the cell, and never on the chrome around it: a control drawn over the
    /// header band would cover something the painter owns and cannot repaint under it.
    #[test]
    fn the_in_cell_editor_is_clipped_to_the_body() {
        let mut g = geom();
        g.strip_h = 28.0;
        let cell = g.editor_rect(2, 1);
        assert_eq!(
            cell,
            g.cell_rect(2, 1),
            "a cell in full view is its own rect"
        );

        // Scrolled so that the first visible row is half under the header band: the editor
        // stops at the band rather than starting above it.
        g.first_row = 4;
        let above = g.editor_rect(3, 1);
        assert_eq!(above.h, 0.0, "a cell scrolled out of sight has no editor");
        // The first visible row is whole, and the last one may be cut off by the body's foot.
        let last = g.visible_rows().last().expect("a row is visible");
        let clipped = g.editor_rect(last, 1);
        let body = g.body();
        assert!(clipped.y >= body.y);
        assert!(clipped.y + clipped.h <= body.y + body.h + 1e-9);
    }

    #[test]
    fn dpi_multiplies_every_width() {
        let lengths = vec![(0, "25.4mm".to_string())];
        let at_96 = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], mm_to_px(96));
        let at_192 = Sizes::from_lengths(80.0, MAX_COLS, &lengths, &[], mm_to_px(192));
        assert!((at_96.size_of(0) - 96.0).abs() < 1e-9);
        assert!((at_192.size_of(0) - 192.0).abs() < 1e-9);
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

    /// The whole of `reveal`, as arithmetic: the clamp only moves the view when the active
    /// cell is outside it, and moves it the least it can either way.
    #[test]
    fn revealing_a_cell_moves_the_view_only_when_it_has_to() {
        let g = geom();
        let show = |first: u32, row: u32| {
            let need = g.rows.start_showing(row, g.body().h);
            first.clamp(need, row.max(need))
        };
        assert_eq!(show(0, 3), 0, "already in view");
        assert_eq!(show(10, 3), 3, "above the view: scroll up to it");
        // 558px of body: row 1 is 60 tall and the rest 20, so from row 0 the last full row is
        // 25. Anything past it scrolls down by exactly what it takes.
        assert!(show(0, 40) > 0, "below the view: scroll down to it");
        assert!(show(0, 40) <= 40);
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

    /// The format strip is the top band, and every band under it moves down by exactly its
    /// height — the name box, the formula bar, the notice bar, the assist band and the headers.
    #[test]
    fn the_format_strip_is_the_top_band_and_pushes_the_rest_down() {
        let mut g = geom();
        g.strip_h = crate::sheet::draw::STRIP_H;
        g.banner_h = 30.0;
        let before = (
            g.name_box_rect(),
            g.formula_rect(),
            g.banner_rect(),
            g.hint_rect(),
            g.body(),
        );
        g.format_h = 44.0;
        assert_eq!(g.format_rect().y, 0.0);
        assert_eq!(g.strip_rect().y, 44.0);
        assert_eq!(g.name_box_rect().y, before.0.y + 44.0);
        assert_eq!(g.formula_rect().y, before.1.y + 44.0);
        assert_eq!(g.banner_rect().y, before.2.y + 44.0);
        assert_eq!(g.hint_rect().y, before.3.y + 44.0);
        assert_eq!(g.body().y, before.4.y + 44.0);
        assert_eq!(g.body().h, before.4.h - 44.0);
        // A click on the strip is chrome, never a cell or the select-all corner.
        assert_eq!(g.hit(4.0, 10.0), Hit::Chrome);
    }

    #[test]
    fn every_format_control_is_on_the_strip_and_findable() {
        use crate::sheet::format::CONTROLS;
        let mut g = geom();
        g.format_h = 44.0;
        g.strip_h = 44.0;
        let placed = g.format_controls();
        assert_eq!(placed.len(), CONTROLS.len());
        for (p, (control, ..)) in placed.iter().zip(CONTROLS) {
            let strip = g.format_rect();
            assert!(p.rect.y >= strip.y && p.rect.y + p.rect.h <= strip.y + strip.h);
            assert!(
                p.rect.x + p.rect.w <= g.width,
                "{control:?} fits an 800-wide window"
            );
            let (x, y) = (p.rect.x + p.rect.w / 2.0, p.rect.y + p.rect.h / 2.0);
            assert_eq!(g.format_hit(x, y), Some(control));
        }
        // Five groups, four separators between them.
        assert_eq!(placed.iter().filter(|p| p.separator.is_some()).count(), 4);
        // Not the name box's strip: that band belongs to the two read-outs.
        let name = g.name_box_rect();
        assert_eq!(g.format_hit(name.x + 1.0, name.y + 1.0), None);
    }
}
