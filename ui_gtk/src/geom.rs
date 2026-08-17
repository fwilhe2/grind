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

/// ODF's sheet bounds, which are also the scrollable extent (§3.2).
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

/// A rectangle in widget space. Not `graphene::Rect`, so this module stays pure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    /// Used by the round-trip test below, and by the next milestone's click handling.
    #[allow(dead_code)]
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
#[allow(dead_code)]
pub enum Hit {
    Cell { row: u32, col: u32 },
    RowHeader(u32),
    ColHeader(u32),
    /// The boundary at the *right* edge of this column, in the column header.
    ColEdge(u32),
    /// The boundary at the *bottom* edge of this row, in the row header.
    RowEdge(u32),
    Corner,
}

/// How wide each column is.
///
/// ponytail: one width for every column, because the model carries none yet. The variable
/// case is a prefix-sum vector behind the same three methods — `doc/gtk-shell.md`'s widths
/// milestone adds a variant here and nothing else in the shell changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColWidths {
    Uniform(f64),
}

impl ColWidths {
    pub fn width_of(&self, _col: u32) -> f64 {
        match self {
            ColWidths::Uniform(w) => *w,
        }
    }

    /// Content-space x of a column's left edge.
    pub fn x_of(&self, col: u32) -> f64 {
        match self {
            ColWidths::Uniform(w) => f64::from(col) * w,
        }
    }

    /// The column containing a content-space x, clamped to the sheet.
    pub fn col_at(&self, x: f64) -> u32 {
        match self {
            ColWidths::Uniform(w) => ((x.max(0.0) / w) as u32).min(MAX_COLS - 1),
        }
    }

    /// Content-space width of the whole sheet.
    pub fn total(&self) -> f64 {
        self.x_of(MAX_COLS)
    }
}

/// Everything needed to place a cell: the header band, the row pitch, the column widths
/// and where the view is scrolled to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridGeom {
    pub header_w: f64,
    pub header_h: f64,
    pub row_height: f64,
    pub cols: ColWidths,
    pub scroll_x: f64,
    pub scroll_y: f64,
}

/// How close to a boundary a pointer counts as being *on* it.
#[allow(dead_code)]
const EDGE_GRAB: f64 = 4.0;

impl GridGeom {
    /// Content-space y of a row's top edge.
    pub fn y_of(&self, row: u32) -> f64 {
        f64::from(row) * self.row_height
    }

    /// The row containing a content-space y, clamped to the sheet.
    pub fn row_at(&self, y: f64) -> u32 {
        ((y.max(0.0) / self.row_height) as u32).min(MAX_ROWS - 1)
    }

    /// A cell's rectangle in **widget** space. May lie outside the widget entirely — the
    /// caller clips, because a cell one row above the viewport is exactly what an
    /// overflowing neighbour is drawn from.
    pub fn cell_rect(&self, row: u32, col: u32) -> Rect {
        Rect {
            x: self.header_w + self.cols.x_of(col) - self.scroll_x,
            y: self.header_h + self.y_of(row) - self.scroll_y,
            w: self.cols.width_of(col),
            h: self.row_height,
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
        let first = self.cols.col_at(self.scroll_x);
        let last = self.cols.col_at(self.scroll_x + (width - self.header_w).max(0.0));
        first..(last + 1).min(MAX_COLS)
    }

    /// The scroll position that brings a cell fully into view, given the content area's
    /// size — unchanged on the axes where it already is.
    ///
    /// Pure, and tested, because "the active cell scrolled off the bottom" and "the view
    /// jumps a row every keypress" are the same off-by-one seen from two sides.
    pub fn scroll_into_view(&self, row: u32, col: u32, page_w: f64, page_h: f64) -> (f64, f64) {
        let x = self.cols.x_of(col);
        let y = self.y_of(row);
        (
            keep_in(self.scroll_x, x, self.cols.width_of(col), page_w),
            keep_in(self.scroll_y, y, self.row_height, page_h),
        )
    }

    /// What is under a point in **widget** space.
    ///
    /// Nothing calls this yet — selection arrives with the keyboard and mouse milestone.
    /// It is here because it is [`GridGeom::cell_rect`]'s inverse, and testing the two
    /// against each other is what catches an off-by-one that would otherwise only show up
    /// as a grid that looks very slightly wrong.
    #[allow(dead_code)]
    pub fn hit(&self, x: f64, y: f64) -> Hit {
        let in_row_header = x < self.header_w;
        let in_col_header = y < self.header_h;
        let content_x = x - self.header_w + self.scroll_x;
        let content_y = y - self.header_h + self.scroll_y;

        match (in_row_header, in_col_header) {
            (true, true) => Hit::Corner,
            (false, true) => {
                let col = self.cols.col_at(content_x);
                // Within grabbing distance of this column's right edge, or of the previous
                // column's — a boundary belongs to the column left of it.
                let right = self.cols.x_of(col) + self.cols.width_of(col);
                if right - content_x <= EDGE_GRAB {
                    Hit::ColEdge(col)
                } else if col > 0 && content_x - self.cols.x_of(col) <= EDGE_GRAB {
                    Hit::ColEdge(col - 1)
                } else {
                    Hit::ColHeader(col)
                }
            }
            (true, false) => {
                let row = self.row_at(content_y);
                let bottom = self.y_of(row) + self.row_height;
                if bottom - content_y <= EDGE_GRAB {
                    Hit::RowEdge(row)
                } else if row > 0 && content_y - self.y_of(row) <= EDGE_GRAB {
                    Hit::RowEdge(row - 1)
                } else {
                    Hit::RowHeader(row)
                }
            }
            (false, false) => Hit::Cell {
                row: self.row_at(content_y),
                col: self.cols.col_at(content_x),
            },
        }
    }
}

/// Move `scroll` the least it takes to show `start .. start + size` inside `page`.
fn keep_in(scroll: f64, start: f64, size: f64, page: f64) -> f64 {
    if start < scroll {
        return start;
    }
    // A cell taller or wider than the view is shown from its top-left corner rather than
    // its far edge, which is the reading order.
    if start + size > scroll + page {
        return (start + size - page).min(start).max(0.0);
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
            row_height: 20.0,
            cols: ColWidths::Uniform(80.0),
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
        assert_eq!(g.scroll_into_view(6, 1, 400.0, 100.0), (0.0, 100.0));
        // Above the view: its top edge.
        assert_eq!(g.scroll_into_view(2, 1, 400.0, 100.0).1, 40.0);
        // Below it: just far enough that its bottom edge shows.
        assert_eq!(g.scroll_into_view(10, 1, 400.0, 100.0).1, 120.0);
        // Wider than the page: the left edge wins, or the cell is unreadable.
        assert_eq!(g.scroll_into_view(6, 3, 50.0, 100.0).0, 240.0);
    }

    #[test]
    fn a_point_past_the_sheet_clamps_rather_than_overflowing() {
        let g = geom();
        assert_eq!(g.cols.col_at(f64::from(u32::MAX)), MAX_COLS - 1);
        assert_eq!(g.row_at(1e18), MAX_ROWS - 1);
        assert_eq!(g.cols.col_at(-5.0), 0);
    }
}
