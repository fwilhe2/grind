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
//! platform cannot know — how many rows and columns to ask [`sheet_core::App::get_viewport`]
//! for, and how to keep the active cell on screen.
//!
//! ponytail: the grid is uniform — every column one width, every row one height —
//! so the document's own `col_widths`/`row_heights` are read by nobody here. The
//! upgrade is a prefix-sum over those runs, the same shape `ui_gtk/src/geom.rs`
//! grew in M8; it is a real feature, not a rounding error, and it is written down
//! in the README's gap list rather than half-built.

use sheet_core::Pos;

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

impl Metrics {
    /// How many rows and columns fit in a box that size — the one number only the
    /// shell can know, and what sizes the viewport.
    ///
    /// Never zero in either axis: before the first layout the surface has no size,
    /// and an empty viewport would render a blank page that never recovers. One
    /// extra of each, so the row being scrolled onto is drawn rather than appearing
    /// only after the scroll finishes.
    pub fn visible(&self, width_px: f64, height_px: f64) -> (u32, u32) {
        let fit = |px: f64, cell: f64| {
            let n = (px / cell).floor();
            match n.is_finite() && n >= 1.0 {
                true => n as u32 + 1,
                false => 1,
            }
        };
        (fit(height_px, self.cell_h), fit(width_px, self.cell_w))
    }
}

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

    #[test]
    fn a_zero_sized_page_still_asks_for_a_cell() {
        assert_eq!(CELL.visible(0.0, 0.0), (1, 1));
        assert_eq!(CELL.visible(f64::NAN, f64::NAN), (1, 1));
    }

    #[test]
    fn the_viewport_covers_the_box_and_one_more() {
        let metrics = Metrics {
            cell_w: 100.0,
            cell_h: 20.0,
        };
        // Ten rows fit exactly, and the eleventh is the one being scrolled onto.
        assert_eq!(metrics.visible(500.0, 200.0), (11, 6));
    }

    /// The bug this shell shipped with: the cell size was measured from a rendered
    /// cell, and a table stretched to its container reports a *narrower* cell than
    /// the one the shell drew — so every repaint asked for one more column, and
    /// typing grew the grid without end. Declared metrics have a fixed point;
    /// measured ones did not.
    #[test]
    fn the_column_count_does_not_depend_on_what_was_drawn() {
        let width = 1280.0;
        let first = CELL.visible(width, 800.0);
        assert_eq!(
            CELL.visible(width, 800.0),
            first,
            "a repaint changes nothing"
        );
        // What a stretched table reports back: the columns it was asked for, sharing
        // the width they had to fit in. Feeding that in is what diverged.
        let squeezed = Metrics {
            cell_w: (width - 56.0) / f64::from(first.1),
            ..CELL
        };
        assert!(squeezed.cell_w < CELL.cell_w);
        assert!(
            squeezed.visible(width, 800.0).1 > first.1,
            "measuring the drawn grid asks for one more column than it drew"
        );
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
