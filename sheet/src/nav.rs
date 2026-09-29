// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a motion moves the selection — the spreadsheet's navigation rules, as pure functions.
//!
//! Hoisted out of `ui_sheet_gtk/src/keymap.rs` and `ui_win32/src/sheet/keymap.rs`, whose
//! vocabularies were kept identical for this day (the Windows copy's `ponytail:` named "a
//! **third** copy" as the trigger, and the macOS shell was it — `doc/macos-shell.md`, M1).
//! Navigation over a used extent is spreadsheet vocabulary rather than toolkit vocabulary, so
//! what stays in a shell is only which *key* means which [`Motion`]: a `gdk::Key`, a
//! virtual-key code or an AppKit selector.
//!
//! The selection is **presentation state**: an anchor and an active cell, and nothing else. The
//! document is never told about it — a range reaches `App` as two positions when something is
//! actually *done* to it.

use crate::tracks::Sizes;
use crate::{MAX_COLS, MAX_ROWS, Pos};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// Where a key wants the active cell to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// One cell.
    By(Dir),
    /// One screenful.
    Page(Dir),
    /// The next edge of the data — Ctrl+arrow (⌘-arrow on a Mac), and `data_edge` has the rule.
    Edge(Dir),
    /// Column A of this row.
    RowStart,
    /// The last used column, in this row.
    RowEnd,
    /// A1.
    SheetStart,
    /// The last used cell of the sheet.
    SheetEnd,
}

/// An anchor and an active cell — the whole of what a selection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Where the selection started; a plain click puts both here.
    pub anchor: Pos,
    /// The cell that has the cursor, and the one an edit will land in.
    pub active: Pos,
}

impl Default for Selection {
    fn default() -> Self {
        Self::at(Pos::new(0, 0))
    }
}

impl Selection {
    pub fn at(pos: Pos) -> Self {
        Self {
            anchor: pos,
            active: pos,
        }
    }

    /// The selection as an inclusive rectangle, top-left first — the order every `App` method
    /// taking two positions wants.
    pub fn rect(&self) -> (Pos, Pos) {
        (
            Pos::new(
                self.anchor.row.min(self.active.row),
                self.anchor.col.min(self.active.col),
            ),
            Pos::new(
                self.anchor.row.max(self.active.row),
                self.anchor.col.max(self.active.col),
            ),
        )
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        let (start, end) = self.rect();
        (start.row..=end.row).contains(&row) && (start.col..=end.col).contains(&col)
    }

    pub fn is_single(&self) -> bool {
        self.anchor == self.active
    }

    /// How many cells are inside — a `u64`, because a whole-sheet selection is not a `u32`.
    pub fn cells(&self) -> u64 {
        let (start, end) = self.rect();
        u64::from(end.row - start.row + 1) * u64::from(end.col - start.col + 1)
    }

    /// Every row of one column, with the active cell at the top so that revealing it scrolls
    /// to the head of the column rather than to row 1048576.
    pub fn whole_col(col: u32) -> Self {
        Self {
            anchor: Pos::new(MAX_ROWS - 1, col),
            active: Pos::new(0, col),
        }
    }

    pub fn whole_row(row: u32) -> Self {
        Self {
            anchor: Pos::new(row, MAX_COLS - 1),
            active: Pos::new(row, 0),
        }
    }
}

/// What the sheet's occupied region is, as far as navigation cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// One past the last used row and column — `App::used_extent`, verbatim.
    pub rows: u32,
    pub cols: u32,
    /// How many rows a PageUp/PageDown moves — however many fit on screen.
    pub page: u32,
}

impl Extent {
    fn last_row(&self) -> u32 {
        self.rows.saturating_sub(1)
    }

    fn last_col(&self) -> u32 {
        self.cols.saturating_sub(1)
    }
}

/// Apply a motion, returning the selection it produces.
///
/// `occupied` answers "does this cell hold anything", and is a closure because the answer comes
/// from the document: a shell backs it with `App::get` or viewport-sized reads, and a test backs
/// it with a set. That is what keeps the *rule* here and the *reads* there.
pub fn moved(
    selection: Selection,
    motion: Motion,
    extend: bool,
    extent: Extent,
    occupied: &dyn Fn(Pos) -> bool,
) -> Selection {
    let from = selection.active;
    let page = extent.page.max(1);
    let active = match motion {
        Motion::By(dir) => step(from, dir, 1),
        Motion::Page(dir) => step(from, dir, page),
        Motion::Edge(dir) => data_edge(from, dir, extent, occupied),
        Motion::RowStart => Pos::new(from.row, 0),
        Motion::RowEnd => Pos::new(from.row, extent.last_col()),
        Motion::SheetStart => Pos::new(0, 0),
        Motion::SheetEnd => Pos::new(extent.last_row(), extent.last_col()),
    };
    match extend {
        true => Selection {
            anchor: selection.anchor,
            active,
        },
        false => Selection::at(active),
    }
}

/// Move the active cell off any hidden track it landed on, in the direction it was travelling.
///
/// [`moved`] works in cell coordinates and knows nothing about widths, which is right — the rule
/// for where Ctrl+Down stops has nothing to do with pixels. But a hidden row occupies none, so a
/// cursor that lands on one is invisible: a shell that draws a hidden track as *gone* has a
/// cursor parked on nothing as that decision's sharp edge. Every spreadsheet steps over them,
/// and this is where it happens.
///
/// The **anchor is left alone**. A selection's corner may perfectly well sit on a hidden track —
/// the rectangle it describes is still the rectangle the user dragged out, and moving the corner
/// would silently change what a later operation covers.
pub fn onto_visible(selection: Selection, motion: Motion, rows: &Sizes, cols: &Sizes) -> Selection {
    let (row_forward, col_forward) = match motion {
        Motion::By(dir) | Motion::Page(dir) | Motion::Edge(dir) => match dir {
            // On the axis being travelled, keep going the way the key pointed. On the other, the
            // cell did not move, so the direction only matters if it was already hidden — and
            // forwards is as good an answer as any.
            Dir::Up => (false, true),
            Dir::Left => (true, false),
            Dir::Down | Dir::Right => (true, true),
        },
        // Home and Ctrl+Home arrive from the right, so they carry on leftwards past a hidden
        // column A; End and Ctrl+End arrive from the left and carry on rightwards.
        Motion::RowStart | Motion::SheetStart => (true, true),
        Motion::RowEnd | Motion::SheetEnd => (false, false),
    };
    let active = selection.active;
    let active = Pos::new(
        rows.nearest_visible(active.row, row_forward),
        cols.nearest_visible(active.col, col_forward),
    );
    match selection.is_single() {
        true => Selection::at(active),
        false => Selection {
            anchor: selection.anchor,
            active,
        },
    }
}

/// `by` cells in a direction, stopping at the sheet's edges.
fn step(from: Pos, dir: Dir, by: u32) -> Pos {
    let (rows, cols) = (MAX_ROWS - 1, MAX_COLS - 1);
    match dir {
        Dir::Left => Pos::new(from.row, from.col.saturating_sub(by)),
        Dir::Right => Pos::new(from.row, from.col.saturating_add(by).min(cols)),
        Dir::Up => Pos::new(from.row.saturating_sub(by), from.col),
        Dir::Down => Pos::new(from.row.saturating_add(by).min(rows), from.col),
    }
}

/// Ctrl+arrow: the far end of a run of cells, or the first cell across a gap.
///
/// The rule every spreadsheet has: from a cell whose neighbour is occupied, go to the last cell
/// of that run; from one whose neighbour is empty, go to the next occupied cell. The stop is the
/// **used extent** rather than row 1048576, so Ctrl+Down in an empty column lands on the last
/// row that has anything rather than a million rows into blank space — the same bound a
/// scrollbar uses.
fn data_edge(from: Pos, dir: Dir, extent: Extent, occupied: &dyn Fn(Pos) -> bool) -> Pos {
    let (limit, index, at): (u32, u32, &dyn Fn(u32) -> Pos) = match dir {
        Dir::Left | Dir::Right => (extent.cols, from.col, &|c| Pos::new(from.row, c)),
        Dir::Up | Dir::Down => (extent.rows, from.row, &|r| Pos::new(r, from.col)),
    };
    let forward = matches!(dir, Dir::Right | Dir::Down);
    let next = |i: u32| match forward {
        true => (i + 1 < limit.max(1)).then(|| i + 1),
        false => i.checked_sub(1),
    };

    let Some(mut i) = next(index) else {
        return from;
    };
    if occupied(at(i)) {
        // Inside a run: stop on its last cell.
        while let Some(n) = next(i).filter(|n| occupied(at(*n))) {
            i = n;
        }
    } else {
        // Across a gap: stop on the first thing found, or at the boundary.
        while let Some(n) = next(i) {
            i = n;
            if occupied(at(i)) {
                break;
            }
        }
    }
    at(i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn extent() -> Extent {
        Extent {
            rows: 20,
            cols: 8,
            page: 10,
        }
    }

    fn sheet(cells: &[(u32, u32)]) -> impl Fn(Pos) -> bool + use<> {
        let set: HashSet<(u32, u32)> = cells.iter().copied().collect();
        move |pos: Pos| set.contains(&(pos.row, pos.col))
    }

    fn go(from: Pos, motion: Motion, occupied: &dyn Fn(Pos) -> bool) -> Pos {
        moved(Selection::at(from), motion, false, extent(), occupied).active
    }

    #[test]
    fn shift_extends_from_the_anchor_and_a_plain_move_collapses() {
        let start = Selection::at(Pos::new(4, 4));
        let occupied = sheet(&[]);
        let wider = moved(start, Motion::By(Dir::Right), true, extent(), &occupied);
        assert_eq!(wider.anchor, Pos::new(4, 4));
        assert_eq!(wider.active, Pos::new(4, 5));
        let collapsed = moved(wider, Motion::By(Dir::Down), false, extent(), &occupied);
        assert!(collapsed.is_single());
        assert_eq!(collapsed.active, Pos::new(5, 5));
    }

    #[test]
    fn a_rectangle_reads_top_left_first_whichever_way_it_was_dragged() {
        let up_and_left = Selection {
            anchor: Pos::new(7, 5),
            active: Pos::new(2, 1),
        };
        assert_eq!(
            up_and_left.rect(),
            (Pos::new(2, 1), Pos::new(7, 5)),
            "the rectangle is normalised, not the anchor"
        );
        assert!(up_and_left.contains(4, 3));
        assert!(!up_and_left.contains(4, 6));
        assert_eq!(up_and_left.cells(), 6 * 5);
    }

    #[test]
    fn ctrl_arrow_stops_at_the_end_of_a_run_and_at_the_start_of_the_next() {
        // Column 0: rows 1,2,3 filled, a gap, then row 8.
        let occupied = sheet(&[(1, 0), (2, 0), (3, 0), (8, 0)]);
        // Next to the run: inside it, so its last cell.
        assert_eq!(
            go(Pos::new(0, 0), Motion::Edge(Dir::Down), &occupied).row,
            3
        );
        assert_eq!(
            go(Pos::new(1, 0), Motion::Edge(Dir::Down), &occupied).row,
            3
        );
        // In the gap: the next occupied cell, however far away.
        assert_eq!(
            go(Pos::new(5, 0), Motion::Edge(Dir::Down), &occupied).row,
            8
        );
        // At its end: across the gap to the next thing.
        assert_eq!(
            go(Pos::new(3, 0), Motion::Edge(Dir::Down), &occupied).row,
            8
        );
        // Past everything: the used extent, not row 1048576.
        assert_eq!(
            go(Pos::new(8, 0), Motion::Edge(Dir::Down), &occupied).row,
            extent().rows - 1
        );
        // And upwards, which is the same rule read backwards.
        assert_eq!(go(Pos::new(8, 0), Motion::Edge(Dir::Up), &occupied).row, 3);
        assert_eq!(go(Pos::new(1, 0), Motion::Edge(Dir::Up), &occupied).row, 0);
    }

    #[test]
    fn the_ends_of_the_sheet_are_the_used_extent() {
        let occupied = sheet(&[]);
        assert_eq!(
            go(Pos::new(5, 5), Motion::SheetEnd, &occupied),
            Pos::new(19, 7)
        );
        assert_eq!(
            go(Pos::new(5, 5), Motion::SheetStart, &occupied),
            Pos::new(0, 0)
        );
        assert_eq!(
            go(Pos::new(5, 5), Motion::RowStart, &occupied),
            Pos::new(5, 0)
        );
        assert_eq!(
            go(Pos::new(5, 5), Motion::RowEnd, &occupied),
            Pos::new(5, 7)
        );
    }

    #[test]
    fn paging_moves_a_screenful_and_stops_at_the_top() {
        let occupied = sheet(&[]);
        assert_eq!(
            go(Pos::new(3, 0), Motion::Page(Dir::Down), &occupied).row,
            13
        );
        assert_eq!(go(Pos::new(3, 0), Motion::Page(Dir::Up), &occupied).row, 0);
    }

    /// The sheet's far corner is a `u32` away, and stepping past it must clamp rather than
    /// wrap — a `PageDown` from the last row is the shortest way to find out.
    #[test]
    fn the_far_corner_clamps_rather_than_wrapping() {
        let occupied = sheet(&[]);
        let big = Extent {
            rows: MAX_ROWS,
            cols: MAX_COLS,
            page: 1000,
        };
        let at = Selection::at(Pos::new(MAX_ROWS - 1, MAX_COLS - 1));
        let down = moved(at, Motion::Page(Dir::Down), false, big, &occupied);
        assert_eq!(down.active, Pos::new(MAX_ROWS - 1, MAX_COLS - 1));
        let right = moved(at, Motion::By(Dir::Right), false, big, &occupied);
        assert_eq!(right.active, Pos::new(MAX_ROWS - 1, MAX_COLS - 1));
    }

    /// An empty sheet has a used extent of `(0, 0)`, and every motion has to survive it —
    /// `last_row()` of nothing is a subtraction that would underflow.
    #[test]
    fn an_empty_sheet_navigates_without_underflowing() {
        let occupied = sheet(&[]);
        let nothing = Extent {
            rows: 0,
            cols: 0,
            page: 10,
        };
        for motion in [
            Motion::SheetEnd,
            Motion::RowEnd,
            Motion::Edge(Dir::Down),
            Motion::Edge(Dir::Right),
        ] {
            let to = moved(
                Selection::at(Pos::new(0, 0)),
                motion,
                false,
                nothing,
                &occupied,
            );
            assert_eq!(to.active, Pos::new(0, 0), "{motion:?}");
        }
    }

    /// The bug this exists for, found by *running* the shell rather than by reading it: two
    /// presses of Down through the sample document's hidden rows put the cursor on row 5, which
    /// is hidden, and the screen showed no cursor at all while the status bar cheerfully
    /// reported one.
    #[test]
    fn a_cursor_never_stops_on_a_track_that_is_not_drawn() {
        let rows = Sizes::new(20.0, 20, vec![(1, 0.0), (4, 0.0), (5, 0.0), (6, 0.0)]);
        let cols = Sizes::new(80.0, 10, vec![(3, 0.0)]);
        let at = |row, col| Selection::at(Pos::new(row, col));

        // Down into a hidden row carries on down.
        let to = onto_visible(at(1, 0), Motion::By(Dir::Down), &rows, &cols);
        assert_eq!(to.active, Pos::new(2, 0));
        // Up into the same run carries on up.
        let to = onto_visible(at(6, 0), Motion::By(Dir::Up), &rows, &cols);
        assert_eq!(to.active, Pos::new(3, 0));
        // Right into a hidden column carries on right.
        let to = onto_visible(at(0, 3), Motion::By(Dir::Right), &rows, &cols);
        assert_eq!(to.active, Pos::new(0, 4));
        // A visible cell is left exactly where it is.
        let to = onto_visible(at(0, 0), Motion::By(Dir::Down), &rows, &cols);
        assert_eq!(to.active, Pos::new(0, 0));
    }

    /// The anchor is the corner the user put down and stays there, hidden or not — moving it
    /// would change what the selection covers behind their back.
    #[test]
    fn stepping_over_a_hidden_track_never_moves_the_anchor() {
        let rows = Sizes::new(20.0, 20, vec![(4, 0.0)]);
        let cols = Sizes::new(80.0, 10, vec![]);
        let selection = Selection {
            anchor: Pos::new(4, 0),
            active: Pos::new(4, 2),
        };
        let to = onto_visible(selection, Motion::By(Dir::Right), &rows, &cols);
        assert_eq!(to.anchor, Pos::new(4, 0), "the anchor is where it was put");
        assert_eq!(to.active, Pos::new(5, 2), "the cursor is somewhere visible");
    }

    #[test]
    fn a_whole_track_selection_puts_the_active_cell_at_its_head() {
        let col = Selection::whole_col(2);
        assert_eq!(col.active, Pos::new(0, 2));
        assert_eq!(col.rect(), (Pos::new(0, 2), Pos::new(MAX_ROWS - 1, 2)));
        let row = Selection::whole_row(7);
        assert_eq!(row.active, Pos::new(7, 0));
        assert_eq!(row.rect(), (Pos::new(7, 0), Pos::new(7, MAX_COLS - 1)));
    }
}
