// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a cell is, in pixels — and which cell a pixel is in.
//!
//! **No GTK types.** This is the shell's pixel arithmetic as pure functions, which is what
//! makes it the part of a GUI that unit-tests with no display, no compositor and no CI
//! runner that has either (doc/gtk-shell.md, following `editor`'s widget-free keymap).
//! Everything the grid widget knows about layout it asks this module.
//!
//! Two coordinate spaces, and mixing them is the bug this module exists to prevent:
//!
//! * **content** — `(0, 0)` is cell A1's top-left corner, and it extends to the sheet's
//!   full 1048576 × 16384. Nothing scrolls here.
//! * **widget** — `(0, 0)` is the widget's top-left corner, so the header band sits at
//!   `x < header_w` / `y < header_h` and the content is offset by the scroll position.
//!
//! [`GridGeom::cell_rect`] converts one way and [`GridGeom::hit`] the other; they are
//! tested as a round trip, because a rectangle that does not contain the point that
//! produced it is the whole class of off-by-one this module can have.

/// ODF's sheet bounds, which are also the scrollable extent (§3.2). The core's, not a
/// second opinion: a scrollbar that ended somewhere the reader does not is a bug waiting.
pub use sheet_core::{MAX_COLS, MAX_ROWS};

/// A rectangle in widget space. Not `graphene::Rect`, so this module stays pure.
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
}

/// What sits under a point.
///
/// The `*Edge` variants take precedence over the header they are inside, because a
/// resize drag starts within a few pixels of a boundary that is also part of a header
/// button — the narrower target has to win or resizing is unreachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Cell {
        row: u32,
        col: u32,
    },
    RowHeader(u32),
    ColHeader(u32),
    /// The boundary at the *right* edge of this column, in the column header.
    ColEdge(u32),
    /// The boundary at the *bottom* edge of this row, in the row header.
    RowEdge(u32),
    /// The marker over a run of columns hidden by hand, `(from, to)` half-open — what a
    /// click unhides, standing where the run collapsed to nothing between the two headers
    /// that still show.
    HiddenCols(u32, u32),
    /// The row twin of [`Hit::HiddenCols`].
    HiddenRows(u32, u32),
    Corner,
}

/// How big each track on one axis is — column widths, or row heights.
///
/// One type for both, because the arithmetic is the same and a sheet that got its columns
/// right and its rows wrong is the bug two copies would produce. A document sizes a handful
/// of tracks out of sixteen thousand, so the sparse list plus a running total is what makes
/// [`Sizes::at`] a binary search rather than a walk from A.
#[derive(Clone, Debug, PartialEq)]
pub struct Sizes {
    default: f64,
    count: u32,
    /// Ascending by index, distinct: the tracks the document gave a size of their own.
    sizes: Vec<(u32, f64)>,
    /// `run[k]` is how much the first `k` entries have displaced everything after them —
    /// the sum of their sizes *minus* what the default would have been. An offset is then
    /// `index * default + run[entries before it]`, with no walk over the sheet.
    run: Vec<f64>,
}

impl Sizes {
    /// `sizes` is taken in any order; ties keep the last one given.
    pub fn new(default: f64, count: u32, mut sizes: Vec<(u32, f64)>) -> Self {
        // Reversed first, so that a stable sort leaves the *last* entry given for an index at
        // the front of its run and `dedup` keeps that one. `with` relies on it.
        sizes.reverse();
        sizes.sort_by_key(|(i, _)| *i);
        sizes.dedup_by_key(|(i, _)| *i);
        // A zero is kept: that is a hidden track — a row a filter excludes (§9.4) — and it
        // has to displace nothing rather than fall back to the default.
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
        // The last sized track that starts at or before `offset`; everything else is one of
        // the uniform runs, before it or after it.
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

    /// Content-space size of the whole axis.
    pub fn total(&self) -> f64 {
        self.offset_of(self.count)
    }

    /// The same axis at a zoom factor. Every size scales, including the default, so the
    /// arithmetic above stays in one space and a zoomed grid is not a second layout path.
    pub fn scaled(&self, factor: f64) -> Self {
        Self::new(
            self.default * factor,
            self.count,
            self.sizes
                .iter()
                .map(|(i, size)| (*i, size * factor))
                .collect(),
        )
    }

    /// The same axis with one track resized — what a resize drag paints before it commits.
    pub fn with(&self, index: u32, size: f64) -> Self {
        let mut sizes = self.sizes.clone();
        sizes.push((index, size));
        Self::new(self.default, self.count, sizes)
    }

    /// Whether this track is a hidden one — zero size, kept explicitly rather than falling
    /// back to the default (see [`Sizes::new`]).
    pub fn is_hidden(&self, index: u32) -> bool {
        self.size_of(index) == 0.0
    }

    /// The maximal contiguous run of hidden tracks containing `index`, half-open — or
    /// `None` when `index` is not itself hidden.
    ///
    /// A linear walk in both directions rather than a binary search either side, because a
    /// document hides a handful of tracks at a time (§5.4) and this is never asked about
    /// anything else — [`GridGeom::hit`] only calls it once it already knows `index` is
    /// hidden.
    pub fn hidden_run(&self, index: u32) -> Option<(u32, u32)> {
        if !self.is_hidden(index) {
            return None;
        }
        let mut from = index;
        while from > 0 && self.is_hidden(from - 1) {
            from -= 1;
        }
        let mut to = index + 1;
        while to < self.count && self.is_hidden(to) {
            to += 1;
        }
        Some((from, to))
    }
}

/// Everything needed to place a cell: the header band, the two axes' sizes, and where the
/// view is scrolled to.
#[derive(Clone, Debug, PartialEq)]
pub struct GridGeom {
    pub header_w: f64,
    pub header_h: f64,
    pub rows: Sizes,
    pub cols: Sizes,
    pub scroll_x: f64,
    pub scroll_y: f64,
}

/// How close to a boundary a pointer counts as being *on* it.
const EDGE_GRAB: f64 = 4.0;

/// The fill handle's side, in pixels — big enough to grab, small enough not to hide the
/// cell corner it sits on.
pub const HANDLE: f64 = 7.0;

/// What a filter button's side is clamped to. It tracks the cell's height rather than being
/// a constant, so it grows with the zoom like everything else drawn in the grid; the floor
/// is the smallest thing a pointer reliably hits and the ceiling stops a tall row from
/// getting a button the size of a cell.
const FILTER_BUTTON: (f64, f64) = (9.0, 18.0);

/// A hidden run's marker's width (columns) or height (rows), in **widget** space — a run
/// collapses to a single point, so this is drawn *and* grabbed straddling it, the same way
/// [`EDGE_GRAB`] carves a boundary's hit zone out of the header on either side.
pub const HIDDEN_MARKER: f64 = 6.0;

impl GridGeom {
    /// The row containing a content-space y, clamped to the sheet.
    pub fn row_at(&self, y: f64) -> u32 {
        self.rows.at(y)
    }

    /// A cell's rectangle in **widget** space. May lie outside the widget entirely — the
    /// caller clips, because a cell one row above the viewport is exactly what an
    /// overflowing neighbour is drawn from.
    pub fn cell_rect(&self, row: u32, col: u32) -> Rect {
        Rect {
            x: self.header_w + self.cols.offset_of(col) - self.scroll_x,
            y: self.header_h + self.rows.offset_of(row) - self.scroll_y,
            w: self.cols.size_of(col),
            h: self.rows.size_of(row),
        }
    }

    /// The fill handle's square, in **widget** space: centred on the bottom-right corner of
    /// the cell a selection ends at, so it is grabbable from inside the selection and from
    /// just outside it alike. Not [`Hit`]'s business — what is under a point here depends on
    /// where the selection is, which this module deliberately knows nothing about.
    pub fn fill_handle(&self, row: u32, col: u32) -> Rect {
        let cell = self.cell_rect(row, col);
        Rect {
            x: cell.x + cell.w - HANDLE / 2.0,
            y: cell.y + cell.h - HANDLE / 2.0,
            w: HANDLE,
            h: HANDLE,
        }
    }

    /// The autofilter dropdown's square, in **widget** space: inset at the right-hand end of
    /// a header-row cell, which is where every spreadsheet puts it.
    ///
    /// Like [`GridGeom::fill_handle`] and for the same reason, deliberately not [`Hit`]'s
    /// business — *which* cells carry a button depends on the document's filter, and this
    /// module knows nothing about the document.
    ///
    /// `None` when the cell cannot spare the room: a button wider than the cell it sits in
    /// would cover the heading it belongs to, and an unhittable sliver is worse than
    /// nothing.
    pub fn filter_button(&self, row: u32, col: u32) -> Option<Rect> {
        let cell = self.cell_rect(row, col);
        let (min, max) = FILTER_BUTTON;
        let size = (cell.h - 2.0).clamp(min, max);
        // Room for the button and at least a little of the text it belongs beside.
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

    /// Where a hidden run of columns is drawn and grabbed, in **widget** space: a narrow
    /// bar straddling the boundary the run collapsed to, the full height of the column
    /// header. `from` is the run's first hidden index — [`Sizes::offset_of`] gives the same
    /// pixel for every index in the run, since none of them displace anything.
    pub fn hidden_col_marker(&self, from: u32) -> Rect {
        let x = self.header_w + self.cols.offset_of(from) - self.scroll_x;
        Rect {
            x: x - HIDDEN_MARKER / 2.0,
            y: 0.0,
            w: HIDDEN_MARKER,
            h: self.header_h,
        }
    }

    /// The row twin of [`GridGeom::hidden_col_marker`].
    pub fn hidden_row_marker(&self, from: u32) -> Rect {
        let y = self.header_h + self.rows.offset_of(from) - self.scroll_y;
        Rect {
            x: 0.0,
            y: y - HIDDEN_MARKER / 2.0,
            w: self.header_w,
            h: HIDDEN_MARKER,
        }
    }

    /// The rows visible in a widget `height`, end-exclusive. A partially visible row at
    /// either edge is included: half a row still has to be drawn.
    pub fn visible_rows(&self, height: f64) -> std::ops::Range<u32> {
        let first = self.row_at(self.scroll_y);
        let last = self.row_at(self.scroll_y + (height - self.header_h).max(0.0));
        first..(last + 1).min(MAX_ROWS)
    }

    /// The columns visible in a widget `width`, end-exclusive.
    pub fn visible_cols(&self, width: f64) -> std::ops::Range<u32> {
        let first = self.cols.at(self.scroll_x);
        let last = self
            .cols
            .at(self.scroll_x + (width - self.header_w).max(0.0));
        first..(last + 1).min(MAX_COLS)
    }

    /// The scroll position that brings a cell fully into view, given the content area's
    /// size — unchanged on the axes where it already is.
    ///
    /// `margin_x`/`margin_y` are how much *past* the cell the view goes whenever it does
    /// move: a row of context beyond the cursor, so a jump never lands flush against the
    /// edge with whatever comes next hidden. The margin never pushes the cell itself back
    /// out of view.
    ///
    /// Pure, and tested, because "the active cell scrolled off the bottom" and "the view
    /// jumps a row every keypress" are the same off-by-one seen from two sides.
    pub fn scroll_into_view(
        &self,
        row: u32,
        col: u32,
        page_w: f64,
        page_h: f64,
        margin_x: f64,
        margin_y: f64,
    ) -> (f64, f64) {
        (
            keep_in(
                self.scroll_x,
                self.cols.offset_of(col),
                self.cols.size_of(col),
                page_w,
                margin_x,
            ),
            keep_in(
                self.scroll_y,
                self.rows.offset_of(row),
                self.rows.size_of(row),
                page_h,
                margin_y,
            ),
        )
    }

    /// What is under a point in **widget** space.
    ///
    /// [`GridGeom::cell_rect`]'s inverse, and the two are tested against each other —
    /// an off-by-one here would otherwise only show up as a grid that looks very slightly
    /// wrong under the pointer.
    pub fn hit(&self, x: f64, y: f64) -> Hit {
        let in_row_header = x < self.header_w;
        let in_col_header = y < self.header_h;
        let content_x = x - self.header_w + self.scroll_x;
        let content_y = y - self.header_h + self.scroll_y;

        match (in_row_header, in_col_header) {
            (true, true) => Hit::Corner,
            (false, true) => {
                let col = self.cols.at(content_x);
                // Within grabbing distance of this column's right edge, or of the previous
                // column's — a boundary belongs to the column left of it. Either side, a
                // hidden run sitting right at that boundary beats a plain resize: there is
                // nothing of the run left to grab, so the boundary is the only way back to
                // it.
                let left = self.cols.offset_of(col);
                if left + self.cols.size_of(col) - content_x <= EDGE_GRAB {
                    match self.cols.hidden_run(col + 1) {
                        Some((from, to)) => Hit::HiddenCols(from, to),
                        None => Hit::ColEdge(col),
                    }
                } else if col > 0 && content_x - left <= EDGE_GRAB {
                    match self.cols.hidden_run(col - 1) {
                        Some((from, to)) => Hit::HiddenCols(from, to),
                        None => Hit::ColEdge(col - 1),
                    }
                } else {
                    Hit::ColHeader(col)
                }
            }
            (true, false) => {
                let row = self.rows.at(content_y);
                let top = self.rows.offset_of(row);
                if top + self.rows.size_of(row) - content_y <= EDGE_GRAB {
                    match self.rows.hidden_run(row + 1) {
                        Some((from, to)) => Hit::HiddenRows(from, to),
                        None => Hit::RowEdge(row),
                    }
                } else if row > 0 && content_y - top <= EDGE_GRAB {
                    match self.rows.hidden_run(row - 1) {
                        Some((from, to)) => Hit::HiddenRows(from, to),
                        None => Hit::RowEdge(row - 1),
                    }
                } else {
                    Hit::RowHeader(row)
                }
            }
            (false, false) => Hit::Cell {
                row: self.rows.at(content_y),
                col: self.cols.at(content_x),
            },
        }
    }
}

/// Move `scroll` the least it takes to show `start .. start + size` inside `page`, plus
/// `margin` of context past it when it moves at all.
fn keep_in(scroll: f64, start: f64, size: f64, page: f64, margin: f64) -> f64 {
    if start < scroll {
        return (start - margin).max(0.0);
    }
    // A cell taller or wider than the view is shown from its top-left corner rather than
    // its far edge, which is the reading order — the `.min(start)` is also what keeps the
    // margin from pushing the cell itself back out.
    if start + size > scroll + page {
        return (start + size - page + margin).min(start).max(0.0);
    }
    scroll
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom() -> GridGeom {
        GridGeom {
            header_w: 50.0,
            header_h: 24.0,
            rows: Sizes::new(20.0, MAX_ROWS, Vec::new()),
            cols: Sizes::new(80.0, MAX_COLS, Vec::new()),
            scroll_x: 0.0,
            scroll_y: 0.0,
        }
    }

    #[test]
    fn a1_sits_just_past_the_headers() {
        let r = geom().cell_rect(0, 0);
        assert_eq!(
            r,
            Rect {
                x: 50.0,
                y: 24.0,
                w: 80.0,
                h: 20.0
            }
        );
    }

    #[test]
    fn scrolling_moves_cells_towards_the_origin() {
        let g = GridGeom {
            scroll_x: 160.0,
            scroll_y: 40.0,
            ..geom()
        };
        // Two columns and two rows scrolled away, so C3 is now the first cell drawn.
        assert_eq!(g.cell_rect(2, 2).x, 50.0);
        assert_eq!(g.cell_rect(2, 2).y, 24.0);
    }

    /// The pair that matters: whatever `hit` names, `cell_rect` must put back around the
    /// point. An off-by-one in either direction fails here rather than by looking wrong.
    #[test]
    fn hit_and_cell_rect_are_inverses() {
        let g = GridGeom {
            scroll_x: 37.0,
            scroll_y: 11.0,
            ..geom()
        };
        for x in [50.0, 51.0, 130.0, 199.0, 500.0] {
            for y in [24.0, 25.0, 44.0, 63.0, 300.0] {
                let Hit::Cell { row, col } = g.hit(x, y) else {
                    panic!("({x}, {y}) is inside the content area");
                };
                assert!(
                    g.cell_rect(row, col).contains(x, y),
                    "({x}, {y}) hit {row},{col} whose rect is {:?}",
                    g.cell_rect(row, col)
                );
            }
        }
    }

    #[test]
    fn the_header_band_is_not_the_content() {
        let g = geom();
        assert_eq!(g.hit(10.0, 10.0), Hit::Corner);
        assert_eq!(g.hit(200.0, 10.0), Hit::ColHeader(1));
        // Mid-row, deliberately clear of the boundary grab zone the next test covers.
        assert_eq!(g.hit(10.0, 94.0), Hit::RowHeader(3));
    }

    /// A boundary belongs to the column left of it, and beats the header underneath —
    /// otherwise the resize target is unreachable.
    #[test]
    fn a_column_boundary_is_grabbable_from_either_side() {
        let g = geom();
        assert_eq!(g.hit(50.0 + 79.0, 10.0), Hit::ColEdge(0));
        assert_eq!(g.hit(50.0 + 81.0, 10.0), Hit::ColEdge(0));
        // The sheet's left edge is not a boundary: there is no column to its left.
        assert_eq!(g.hit(50.0 + 1.0, 10.0), Hit::ColHeader(0));
    }

    /// The handle straddles the corner, and is nowhere near the middle of its cell.
    #[test]
    fn the_fill_handle_sits_on_the_selections_corner() {
        let g = geom();
        let h = g.fill_handle(0, 0);
        assert!(h.contains(50.0 + 80.0 - 1.0, 24.0 + 20.0 - 1.0), "inside");
        assert!(h.contains(50.0 + 80.0 + 1.0, 24.0 + 20.0 + 1.0), "outside");
        assert!(!h.contains(50.0 + 40.0, 24.0 + 10.0), "cell centre");
    }

    /// The dropdown sits at the cell's right-hand end, clear of the text, inside the cell —
    /// and gives up rather than covering a cell too narrow to spare the room.
    #[test]
    fn the_filter_button_sits_at_the_right_end_of_its_cell() {
        let g = geom();
        let cell = g.cell_rect(0, 0);
        let b = g.filter_button(0, 0).expect("an 80x20 cell has room");
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
            cols: Sizes::new(80.0, MAX_COLS, vec![(0, 12.0)]),
            ..geom()
        };
        assert_eq!(narrow.filter_button(0, 0), None, "no room beside the text");
    }

    /// A partially visible row is a visible row, and the range never runs past the sheet.
    #[test]
    fn visible_ranges_include_the_partial_edges_and_stop_at_the_sheet() {
        let g = GridGeom {
            scroll_y: 30.0,
            ..geom()
        };
        // 30px down is halfway into row 2, and 124px of widget leaves 100px of content.
        assert_eq!(g.visible_rows(124.0), 1..7);

        let bottom = GridGeom {
            scroll_y: f64::from(MAX_ROWS) * 20.0,
            ..geom()
        };
        assert_eq!(bottom.visible_rows(124.0).end, MAX_ROWS);
    }

    /// Only as far as it has to, and never past the cell's own corner.
    #[test]
    fn scrolling_a_cell_into_view_moves_the_least_it_can() {
        let g = GridGeom {
            scroll_x: 0.0,
            scroll_y: 100.0,
            ..geom()
        };
        // Already inside: nothing moves.
        assert_eq!(
            g.scroll_into_view(6, 1, 400.0, 100.0, 0.0, 0.0),
            (0.0, 100.0)
        );
        // Above the view: its top edge.
        assert_eq!(g.scroll_into_view(2, 1, 400.0, 100.0, 0.0, 0.0).1, 40.0);
        // Below it: just far enough that its bottom edge shows.
        assert_eq!(g.scroll_into_view(10, 1, 400.0, 100.0, 0.0, 0.0).1, 120.0);
        // Wider than the page: the left edge wins, or the cell is unreadable.
        assert_eq!(g.scroll_into_view(6, 3, 50.0, 100.0, 0.0, 0.0).0, 240.0);
    }

    /// The margin: a move overshoots by a row of context, an axis that did not need to
    /// move stays put, and the cell itself is never pushed back out by its own margin.
    #[test]
    fn scrolling_with_a_margin_leaves_context_past_the_cell() {
        let g = GridGeom {
            scroll_x: 0.0,
            scroll_y: 100.0,
            ..geom()
        };
        // Already inside: the margin does not shove a view that is not moving.
        assert_eq!(
            g.scroll_into_view(6, 1, 400.0, 100.0, 80.0, 20.0),
            (0.0, 100.0)
        );
        // Below: the bottom edge plus one row of context.
        assert_eq!(g.scroll_into_view(10, 1, 400.0, 100.0, 0.0, 20.0).1, 140.0);
        // Above: the top edge minus it, clamped at the sheet's own top.
        assert_eq!(g.scroll_into_view(2, 1, 400.0, 100.0, 0.0, 20.0).1, 20.0);
        assert_eq!(g.scroll_into_view(0, 1, 400.0, 100.0, 0.0, 20.0).1, 0.0);
        // Wider than the page: the left edge still wins over the margin.
        assert_eq!(g.scroll_into_view(6, 3, 50.0, 100.0, 80.0, 0.0).0, 240.0);
    }

    #[test]
    fn a_point_past_the_sheet_clamps_rather_than_overflowing() {
        let g = geom();
        assert_eq!(g.cols.at(f64::from(u32::MAX)), MAX_COLS - 1);
        assert_eq!(g.row_at(1e18), MAX_ROWS - 1);
        assert_eq!(g.cols.at(-5.0), 0);
    }

    /// The sparse case, which is every real document: `offset_of` and `at` are inverses
    /// across the sized tracks *and* the uniform runs either side of them, and the total is
    /// the default everywhere plus what the overrides changed.
    #[test]
    fn sized_tracks_displace_the_ones_after_them_and_nothing_else() {
        // B is narrow, D is wide; A, C and everything past E is the 80px default.
        let s = Sizes::new(80.0, 100, vec![(3, 200.0), (1, 20.0)]);
        assert_eq!(s.size_of(0), 80.0);
        assert_eq!(s.size_of(1), 20.0);
        assert_eq!(s.size_of(3), 200.0);
        assert_eq!(s.offset_of(0), 0.0);
        assert_eq!(s.offset_of(1), 80.0);
        assert_eq!(s.offset_of(2), 100.0);
        assert_eq!(s.offset_of(3), 180.0);
        assert_eq!(s.offset_of(4), 380.0);
        assert_eq!(s.offset_of(5), 460.0);
        assert_eq!(s.total(), 100.0 * 80.0 - 60.0 + 120.0);

        for i in 0..12u32 {
            let (start, end) = (s.offset_of(i), s.offset_of(i + 1));
            assert_eq!(s.at(start), i, "start of {i}");
            assert_eq!(s.at(end - 0.5), i, "end of {i}");
        }
        // Past the last track, not one past it.
        assert_eq!(s.at(1e12), 99);
    }

    /// A resize drag repaints through `with`, so setting a track that already has a size has
    /// to replace it rather than pile up beside it.
    #[test]
    fn resizing_the_same_track_twice_keeps_the_last_size() {
        let s = Sizes::new(80.0, 10, Vec::new()).with(2, 30.0).with(2, 50.0);
        assert_eq!(s.size_of(2), 50.0);
        assert_eq!(s.total(), 10.0 * 80.0 - 30.0);
    }

    /// Zooming moves every edge by the factor and keeps the tracks in the same order — a
    /// track that was twice the default still is.
    #[test]
    fn scaling_an_axis_scales_every_track_and_every_offset() {
        let s = Sizes::new(80.0, 100, vec![(1, 20.0), (3, 200.0)]).scaled(1.5);
        assert_eq!(s.size_of(0), 120.0);
        assert_eq!(s.size_of(1), 30.0);
        assert_eq!(s.offset_of(4), 380.0 * 1.5);
        assert_eq!(
            s.total(),
            Sizes::new(80.0, 100, vec![(1, 20.0), (3, 200.0)]).total() * 1.5
        );
        assert_eq!(s.at(s.offset_of(3) + 1.0), 3);
    }

    /// A cell in a row of its own height still contains the point that named it — the same
    /// round trip as the uniform case, which is where an off-by-one would hide.
    #[test]
    fn hit_and_cell_rect_are_inverses_with_sized_tracks() {
        let g = GridGeom {
            rows: Sizes::new(20.0, MAX_ROWS, vec![(1, 60.0)]),
            cols: Sizes::new(80.0, MAX_COLS, vec![(0, 30.0), (2, 150.0)]),
            scroll_x: 13.0,
            scroll_y: 7.0,
            ..geom()
        };
        for x in [50.0, 60.0, 120.0, 300.0, 900.0] {
            for y in [24.0, 30.0, 70.0, 110.0, 400.0] {
                let Hit::Cell { row, col } = g.hit(x, y) else {
                    panic!("({x}, {y}) is inside the content area");
                };
                assert!(
                    g.cell_rect(row, col).contains(x, y),
                    "({x}, {y}) hit {row},{col} whose rect is {:?}",
                    g.cell_rect(row, col)
                );
            }
        }
    }

    /// A single hidden track is its own one-track run, and is not confused with its
    /// (visible) neighbours.
    #[test]
    fn a_hidden_track_is_a_run_of_one() {
        let s = Sizes::new(80.0, 100, vec![(2, 0.0)]);
        assert!(!s.is_hidden(1));
        assert!(s.is_hidden(2));
        assert!(!s.is_hidden(3));
        assert_eq!(s.hidden_run(2), Some((2, 3)));
        assert_eq!(s.hidden_run(1), None, "column 1 is not hidden at all");
    }

    /// Several hidden tracks in a row are one run, found the same way from any index
    /// inside it — which is what lets `hit` name the same run whichever side of the
    /// collapsed boundary the pointer approaches from.
    #[test]
    fn a_run_of_several_hidden_tracks_is_found_from_any_index_in_it() {
        let s = Sizes::new(80.0, 100, vec![(2, 0.0), (3, 0.0), (4, 0.0)]);
        for i in 2..5 {
            assert_eq!(s.hidden_run(i), Some((2, 5)), "from index {i}");
        }
        assert_eq!(s.hidden_run(1), None);
        assert_eq!(s.hidden_run(5), None);
    }

    /// The point in the sheet's own space where column C is hidden — both this and the
    /// widget-space tests below are the fact that column C occupies zero pixels: B and D
    /// touch, and the touching point is where the marker and its hit zone sit.
    #[test]
    fn hidden_columns_collapse_the_gap_between_their_neighbours() {
        let s = Sizes::new(80.0, 100, vec![(2, 0.0)]);
        assert_eq!(s.offset_of(2), 160.0, "B ends at 160");
        assert_eq!(s.offset_of(3), 160.0, "D starts at the same pixel");
        assert_eq!(s.size_of(2), 0.0);
    }

    /// A click right on a hidden run's collapsed boundary hits the marker rather than a
    /// resize edge, whichever of the two touching (visible) columns `at()` would otherwise
    /// have named — the whole reason `Hit::HiddenCols` exists rather than overloading
    /// `ColEdge`.
    #[test]
    fn a_hidden_column_run_is_hit_from_either_side_of_its_collapsed_boundary() {
        let g = GridGeom {
            cols: Sizes::new(80.0, MAX_COLS, vec![(2, 0.0)]),
            ..geom()
        };
        let boundary = g.header_w + 160.0; // B (0,1) ends, D (3) starts, both at 160.
        assert_eq!(
            g.hit(boundary - 1.0, 10.0),
            Hit::HiddenCols(2, 3),
            "from B's side"
        );
        assert_eq!(
            g.hit(boundary + 1.0, 10.0),
            Hit::HiddenCols(2, 3),
            "from D's side"
        );
        // Well inside a normal column, nothing has changed.
        assert_eq!(g.hit(g.header_w + 10.0, 10.0), Hit::ColHeader(0));
    }

    /// The row twin of the column test above.
    #[test]
    fn a_hidden_row_run_is_hit_from_either_side_of_its_collapsed_boundary() {
        let g = GridGeom {
            rows: Sizes::new(20.0, MAX_ROWS, vec![(1, 0.0), (2, 0.0)]),
            ..geom()
        };
        let boundary = g.header_h + 20.0; // Row 0 ends, row 3 starts, both at 20.
        assert_eq!(g.hit(10.0, boundary - 1.0), Hit::HiddenRows(1, 3));
        assert_eq!(g.hit(10.0, boundary + 1.0), Hit::HiddenRows(1, 3));
    }

    /// The marker sits exactly on the collapsed boundary, spanning the header band on the
    /// axis perpendicular to the run.
    #[test]
    fn hidden_run_markers_sit_on_the_collapsed_boundary() {
        let g = GridGeom {
            cols: Sizes::new(80.0, MAX_COLS, vec![(2, 0.0)]),
            rows: Sizes::new(20.0, MAX_ROWS, vec![(1, 0.0)]),
            ..geom()
        };
        let col_marker = g.hidden_col_marker(2);
        assert_eq!(col_marker.x + col_marker.w / 2.0, g.header_w + 160.0);
        assert_eq!((col_marker.y, col_marker.h), (0.0, g.header_h));

        let row_marker = g.hidden_row_marker(1);
        assert_eq!(row_marker.y + row_marker.h / 2.0, g.header_h + 20.0);
        assert_eq!((row_marker.x, row_marker.w), (0.0, g.header_w));
    }
}
