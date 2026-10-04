// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a key, a click or a drag does to the selection — portable, and tested with no window.
//!
//! The rules are `grind_sheet::nav`'s; what is here is the Mac's reading of its own input onto
//! them. A selector arrives as a [`GridAction`] (`keys.rs`), a click as a point in the grid view,
//! a click on a header band as a track — and each comes back as the selection it produces, which
//! the view stores and draws. The grid view's own coordinates keep a margin the header bands
//! float over, so a point is taken in the **sheet's** coordinates here and the view subtracts the
//! margin first.

use grind_sheet::nav::{self, Extent, Selection};
use grind_sheet::{App, MAX_COLS, MAX_ROWS, Pos};

use super::geom::{Grid, HEADER_H, HEADER_W, Rect};
use crate::keys::GridAction;

/// How many rows a Page Up or Page Down moves, for a view `height` points tall looking at the
/// sheet from `top`: however many rows fit, less one, so the row read last is still on screen.
pub fn page(grid: &Grid, top: f64, height: f64) -> u32 {
    let fit = grid.rows_in(top, height).len() as u32;
    fit.saturating_sub(1).max(1)
}

/// The selection a grid action leaves.
pub fn apply(
    app: &App,
    sheet: usize,
    grid: &Grid,
    selection: Selection,
    action: GridAction,
    page: u32,
) -> Selection {
    match action {
        GridAction::Move { motion, extend } => {
            let (rows, cols) = app.used_extent(sheet).unwrap_or((0, 0));
            let extent = Extent { rows, cols, page };
            let occupied = nav::occupied(app, sheet);
            let moved = nav::moved(selection, motion, extend, extent, &occupied);
            // A hidden track is drawn as gone, so a cursor may not stop on one.
            let moved = nav::onto_visible(moved, motion, &grid.rows, &grid.cols);
            // A merge is one cell: a step leaves it from its far edge.
            nav::through_merges(selection, moved, motion, &nav::merges(app, sheet))
        }
        GridAction::Collapse => Selection::at(selection.active),
        // Emptying cells is an edit, and M4's; until then the key does nothing to the selection.
        GridAction::Clear => selection,
    }
}

/// A click at `(x, y)` in the sheet's coordinates: the cell under it, or — with Shift, or while
/// dragging — the range from the anchor to it.
pub fn click(grid: &Grid, selection: Selection, x: f64, y: f64, extend: bool) -> Selection {
    let (row, col) = grid.hit(x, y);
    let at = Pos::new(row, col);
    match extend {
        true => Selection {
            anchor: selection.anchor,
            active: at,
        },
        false => Selection::at(at),
    }
}

/// A click on the column header band at `x`: the whole column, or with Shift the columns from the
/// anchor's to it.
pub fn column_click(grid: &Grid, selection: Selection, x: f64, extend: bool) -> Selection {
    let (_, col) = grid.hit(x, 0.0);
    let whole = Selection::whole_col(col);
    match extend {
        // Every row of every column from the anchor's to this one: the anchor keeps its column
        // and goes to the foot of the sheet, as `Selection::whole_col` puts it.
        true => Selection {
            anchor: Pos::new(MAX_ROWS - 1, selection.anchor.col),
            active: whole.active,
        },
        false => whole,
    }
}

/// A click on the row header band at `y`: the whole row, or with Shift the rows from the
/// anchor's to it.
pub fn row_click(grid: &Grid, selection: Selection, y: f64, extend: bool) -> Selection {
    let (row, _) = grid.hit(0.0, y);
    let whole = Selection::whole_row(row);
    match extend {
        true => Selection {
            anchor: Pos::new(selection.anchor.row, MAX_COLS - 1),
            active: whole.active,
        },
        false => whole,
    }
}

/// What the grid view scrolls to keep in sight after the selection moves: the active cell, in the
/// **view's** coordinates, grown up and to the left by the header bands — so the cell ends up
/// beside them rather than under them.
pub fn reveal(grid: &Grid, selection: Selection) -> Rect {
    let cell = grid.cell(selection.active.row, selection.active.col);
    Rect::new(cell.x, cell.y, cell.w + HEADER_W, cell.h + HEADER_H)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::grid_action;
    use crate::sheet::geom::{COL_W, ROW_H};
    use grind_sheet::RecalcMode;
    use grind_sheet::nav::{Dir, Motion};

    /// A block of data, B2:D6, and nothing else.
    fn book() -> App {
        let app = App::new();
        for row in 1..6 {
            for col in 1..4 {
                app.enter(0, Pos::new(row, col), "1", RecalcMode::Document)
                    .unwrap();
            }
        }
        app
    }

    fn after(app: &App, from: Selection, selector: &str) -> Selection {
        let grid = Grid::of(app, 0);
        let action = grid_action(selector).unwrap_or_else(|| panic!("{selector} is answered"));
        apply(app, 0, &grid, from, action, 10)
    }

    /// M3's exit criterion, the keyboard half: arrows, ⌘-arrows and Shift-extension.
    #[test]
    fn arrows_command_arrows_and_shift_go_where_a_spreadsheet_goes() {
        let app = book();
        let b2 = Selection::at(Pos::new(1, 1));
        assert_eq!(after(&app, b2, "moveRight:").active, Pos::new(1, 2));
        assert_eq!(
            after(&app, b2, "moveToEndOfDocument:").active,
            Pos::new(5, 1),
            "the data's edge"
        );
        assert_eq!(
            after(&app, b2, "moveToRightEndOfLine:").active,
            Pos::new(1, 3)
        );
        let grown = after(&app, b2, "moveDownAndModifySelection:");
        assert_eq!(
            grown.rect(),
            (Pos::new(1, 1), Pos::new(2, 1)),
            "Shift extends"
        );
        assert_eq!(
            after(&app, grown, "cancelOperation:"),
            Selection::at(Pos::new(2, 1))
        );
        assert_eq!(
            after(&app, b2, "scrollToBeginningOfDocument:").active,
            Pos::new(0, 0)
        );
        assert_eq!(
            after(&app, b2, "insertNewline:").active,
            Pos::new(2, 1),
            "Return goes down"
        );
        assert_eq!(
            after(&app, b2, "insertBacktab:").active,
            Pos::new(1, 0),
            "⇧Tab goes left"
        );
    }

    #[test]
    fn a_page_is_what_fits_less_one() {
        let grid = Grid::of(&book(), 0);
        assert_eq!(page(&grid, 0.0, 10.0 * ROW_H), 9);
        assert_eq!(page(&grid, 0.0, ROW_H / 2.0), 1, "never less than a row");
        let app = book();
        let moved = apply(
            &app,
            0,
            &grid,
            Selection::at(Pos::new(0, 0)),
            GridAction::Move {
                motion: Motion::Page(Dir::Down),
                extend: false,
            },
            9,
        );
        assert_eq!(moved.active, Pos::new(9, 0));
    }

    /// A hidden row is stepped over, not landed on.
    #[test]
    fn the_cursor_never_stops_on_a_hidden_row() {
        let app = book();
        app.set_row_hidden(0, 2..3, true).unwrap();
        assert_eq!(
            after(&app, Selection::at(Pos::new(1, 1)), "moveDown:").active,
            Pos::new(3, 1)
        );
    }

    #[test]
    fn a_click_selects_a_cell_and_shift_or_a_drag_grows_from_the_anchor() {
        let grid = Grid::of(&book(), 0);
        let one = click(&grid, Selection::default(), COL_W * 2.5, ROW_H * 3.5, false);
        assert_eq!(one, Selection::at(Pos::new(3, 2)));
        let grown = click(&grid, one, COL_W * 0.5, ROW_H * 0.5, true);
        assert_eq!(grown.rect(), (Pos::new(0, 0), Pos::new(3, 2)));
        assert_eq!(
            grown.anchor,
            Pos::new(3, 2),
            "the anchor stays where the click began"
        );
    }

    #[test]
    fn a_header_click_selects_the_whole_track_and_shift_the_run() {
        let grid = Grid::of(&book(), 0);
        let c = column_click(&grid, Selection::default(), COL_W * 2.5, false);
        assert_eq!(c, Selection::whole_col(2));
        let b_to_d = column_click(&grid, Selection::whole_col(1), COL_W * 3.5, true);
        assert_eq!(
            b_to_d.rect(),
            (Pos::new(0, 1), Pos::new(MAX_ROWS - 1, 3)),
            "every row of B to D"
        );
        let two_rows = row_click(&grid, Selection::whole_row(1), ROW_H * 2.5, true);
        assert_eq!(two_rows.rect(), (Pos::new(1, 0), Pos::new(2, MAX_COLS - 1)));
        let row = row_click(&grid, Selection::default(), ROW_H * 4.5, false);
        assert_eq!(row, Selection::whole_row(4));
    }

    #[test]
    fn revealing_a_cell_leaves_room_for_the_bands() {
        let grid = Grid::of(&book(), 0);
        let rect = reveal(&grid, Selection::at(Pos::new(3, 2)));
        assert_eq!((rect.x, rect.y), (2.0 * COL_W, 3.0 * ROW_H));
        assert_eq!((rect.w, rect.h), (COL_W + HEADER_W, ROW_H + HEADER_H));
    }
}
