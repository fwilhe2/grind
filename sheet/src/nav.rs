// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a motion moves the selection — the spreadsheet's navigation rules, as pure functions. A
//! shell decides only which key means which [`Motion`]. The selection is presentation state, an
//! anchor and an active cell; the document only sees two positions when something is done to a
//! range.

use crate::tracks::Sizes;
use crate::{App, MAX_COLS, MAX_ROWS, Pos};

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

    /// The selection as an inclusive rectangle, top-left first, as `App` methods take two
    /// positions.
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

    /// Every row of one column, with the active cell at the top so revealing it scrolls to the
    /// head of the column.
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

/// Apply a motion, returning the selection it produces. `occupied` answers whether a cell holds
/// anything — from the document in a shell, from a set in a test.
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

/// The rectangle an operation over `selection` acts on — a format, a style, a copy — when the
/// sheet uses rows `0..used.0` and columns `0..used.1`. Only a whole row or column is cut to the
/// part in use, so a header click followed by `€` or a copy does not mean a million rows. A
/// dragged rectangle past the last value is left as it is; an empty sheet still acts on the first
/// cell of the row or column.
pub fn target(selection: Selection, used: (u32, u32)) -> (Pos, Pos) {
    let (start, end) = selection.rect();
    let (rows, cols) = used;
    let row = match start.row == 0 && end.row >= MAX_ROWS - 1 {
        true => end.row.min(rows.saturating_sub(1)).max(start.row),
        false => end.row,
    };
    let col = match start.col == 0 && end.col >= MAX_COLS - 1 {
        true => end.col.min(cols.saturating_sub(1)).max(start.col),
        false => end.col,
    };
    (start, Pos::new(row, col))
}

/// The fills Fill Down (`down`) or Fill Right makes over `selection`: each column's top cell
/// copied down the rest, or each row's leftmost across, as `(source, start, end)` for
/// [`App::fill`]. Nothing outside the selection is read.
///
/// ponytail: one `App::fill`, so one undo step, per line; a multi-source fill in the core is the
/// upgrade.
pub fn fills(selection: Selection, down: bool) -> Vec<(Pos, Pos, Pos)> {
    let (start, end) = selection.rect();
    fill_lines(start, end, down)
}

/// [`fills`] over a rectangle, for a shell whose selection is not this type.
pub fn fill_lines(start: Pos, end: Pos, down: bool) -> Vec<(Pos, Pos, Pos)> {
    match down {
        true if end.row > start.row => (start.col..=end.col)
            .map(|col| {
                (
                    Pos::new(start.row, col),
                    Pos::new(start.row + 1, col),
                    Pos::new(end.row, col),
                )
            })
            .collect(),
        false if end.col > start.col => (start.row..=end.row)
            .map(|row| {
                (
                    Pos::new(row, start.col),
                    Pos::new(row, start.col + 1),
                    Pos::new(row, end.col),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether a cell of `sheet` holds anything, for [`moved`]'s `occupied`.
///
/// ponytail: a data edge across a sparse million-row sheet is a million point reads; the fix is a
/// used-extent walk in the core.
pub fn occupied(app: &App, sheet: usize) -> impl Fn(Pos) -> bool + '_ {
    move |pos| app.get(sheet, pos).is_ok_and(|value| !value.is_empty())
}

/// Select All: everything `sheet` uses, with the active cell at A1 so the view goes home. An empty
/// sheet selects its one cell.
pub fn all(app: &App, sheet: usize) -> Selection {
    let (rows, cols) = app.used_extent(sheet).unwrap_or((0, 0));
    Selection {
        anchor: Pos::new(rows.saturating_sub(1), cols.saturating_sub(1)),
        active: Pos::new(0, 0),
    }
}

/// Move the active cell off any hidden track it landed on, in the direction it was travelling: a
/// hidden track has no width, so a cursor on it is invisible. The anchor stays where it is, since
/// moving it would change what the selection covers.
pub fn onto_visible(selection: Selection, motion: Motion, rows: &Sizes, cols: &Sizes) -> Selection {
    let (row_forward, col_forward) = match motion {
        Motion::By(dir) | Motion::Page(dir) | Motion::Edge(dir) => match dir {
            // Keep going the way the key pointed on the axis travelled; on the other, forwards.
            Dir::Up => (false, true),
            Dir::Left => (true, false),
            Dir::Down | Dir::Right => (true, true),
        },
        // Home and Ctrl+Home carry on leftwards past a hidden column A; End and Ctrl+End
        // rightwards.
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

/// The merge holding a cell, as its top-left and bottom-right cells, from the document.
pub fn merges(app: &App, sheet: usize) -> impl Fn(Pos) -> Option<(Pos, Pos)> + '_ {
    move |pos| app.merge_at(sheet, pos).ok().flatten()
}

/// Land a plain selection on a merge's top-left cell when it is inside one — what a click on a
/// covered cell means. An extended selection is left alone.
pub fn onto_merge(selection: Selection, merge_at: &dyn Fn(Pos) -> Option<(Pos, Pos)>) -> Selection {
    match merge_at(selection.active) {
        Some((anchor, _)) if selection.is_single() => Selection::at(anchor),
        _ => selection,
    }
}

/// [`moved`]'s result with merges taken into account: one step out of a merge leaves from its far
/// edge (Right from `B2:D2` lands on `E2`), and a landing inside a merge goes to its top-left
/// cell. An extended selection is left alone; growing it to whole merges is a gap
/// (`doc/not-doing.md` §3).
pub fn through_merges(
    before: Selection,
    after: Selection,
    motion: Motion,
    merge_at: &dyn Fn(Pos) -> Option<(Pos, Pos)>,
) -> Selection {
    if !after.is_single() {
        return after;
    }
    let mut active = after.active;
    if let (Motion::By(dir), Some((anchor, end))) = (motion, merge_at(before.active)) {
        let from = before.active;
        let inside = (anchor.row..=end.row).contains(&active.row)
            && (anchor.col..=end.col).contains(&active.col);
        if inside {
            let edge = match dir {
                Dir::Right => Pos::new(from.row, end.col),
                Dir::Down => Pos::new(end.row, from.col),
                Dir::Left => Pos::new(from.row, anchor.col),
                Dir::Up => Pos::new(anchor.row, from.col),
            };
            active = step(edge, dir, 1);
            // At the sheet's edge there is nowhere to go: stay on the merge.
            if merge_at(active).is_some_and(|(a, _)| a == anchor) {
                active = anchor;
            }
        }
    }
    onto_merge(Selection::at(active), merge_at)
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

/// Ctrl+arrow: from a cell whose neighbour is occupied, the last cell of that run; otherwise the
/// next occupied cell. The stop is the used extent, not row 1048576.
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
    #[test]
    fn a_merge_is_one_cell_to_the_arrow_keys() {
        // B2:D2 and a block B4:C5.
        let merge_at = |pos: Pos| {
            [
                (Pos::new(1, 1), Pos::new(1, 3)),
                (Pos::new(3, 1), Pos::new(4, 2)),
            ]
            .into_iter()
            .find(|(a, e)| (a.row..=e.row).contains(&pos.row) && (a.col..=e.col).contains(&pos.col))
        };
        let go = |from: Pos, dir: Dir| {
            let before = Selection::at(from);
            let after = Selection::at(step(from, dir, 1));
            through_merges(before, after, Motion::By(dir), &merge_at).active
        };
        // Into a merge from any side lands on its top-left cell…
        assert_eq!(go(Pos::new(1, 0), Dir::Right), Pos::new(1, 1));
        assert_eq!(go(Pos::new(0, 3), Dir::Down), Pos::new(1, 1));
        assert_eq!(go(Pos::new(4, 3), Dir::Left), Pos::new(3, 1));
        // …and out of one leaves from its far edge.
        assert_eq!(go(Pos::new(1, 1), Dir::Right), Pos::new(1, 4));
        assert_eq!(go(Pos::new(3, 1), Dir::Down), Pos::new(5, 1));
        assert_eq!(go(Pos::new(1, 1), Dir::Left), Pos::new(1, 0));
        assert_eq!(go(Pos::new(3, 1), Dir::Up), Pos::new(2, 1));
        // A plain step between plain cells is untouched.
        assert_eq!(go(Pos::new(7, 7), Dir::Right), Pos::new(7, 8));

        // A click on a covered cell is a click on the merge.
        let clicked = onto_merge(Selection::at(Pos::new(4, 2)), &merge_at);
        assert_eq!(clicked, Selection::at(Pos::new(3, 1)));
        // A drag's corners are where they were put.
        let dragged = Selection {
            anchor: Pos::new(0, 0),
            active: Pos::new(4, 2),
        };
        assert_eq!(onto_merge(dragged, &merge_at), dragged);
    }

    #[test]
    fn fill_down_copies_each_columns_top_cell_and_right_each_rows_first() {
        let sel = |a: (u32, u32), b: (u32, u32)| Selection {
            anchor: Pos::new(a.0, a.1),
            active: Pos::new(b.0, b.1),
        };
        let b2_c4 = sel((1, 1), (3, 2));
        assert_eq!(
            fills(b2_c4, true),
            [
                (Pos::new(1, 1), Pos::new(2, 1), Pos::new(3, 1)),
                (Pos::new(1, 2), Pos::new(2, 2), Pos::new(3, 2)),
            ]
        );
        assert_eq!(fills(b2_c4, false).len(), 3, "a fill per row");
        assert_eq!(
            fills(b2_c4, false)[0],
            (Pos::new(1, 1), Pos::new(1, 2), Pos::new(1, 2))
        );
        assert!(
            fills(sel((0, 0), (0, 4)), true).is_empty(),
            "nothing below to fill"
        );
    }

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

    /// Stepping past the far corner clamps rather than wraps.
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

    /// An empty sheet's extent is `(0, 0)`, and every motion must survive it.
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

    /// Two presses of Down through the sample document's hidden rows must not leave the cursor on
    /// hidden row 5.
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

    /// The anchor stays where the user put it, hidden or not.
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

    #[test]
    fn a_cell_is_occupied_when_the_document_holds_something_in_it() {
        let app = App::new();
        app.enter(0, Pos::new(2, 1), "x", crate::RecalcMode::Document)
            .unwrap();
        let occupied = occupied(&app, 0);
        assert!(occupied(Pos::new(2, 1)));
        assert!(!occupied(Pos::new(0, 0)));
        assert!(!super::occupied(&app, 7)(Pos::new(2, 1)), "no such sheet");
    }

    #[test]
    fn select_all_takes_what_is_used_and_goes_home() {
        let app = App::new();
        assert_eq!(
            all(&app, 0),
            Selection::at(Pos::new(0, 0)),
            "an empty sheet"
        );
        app.enter(0, Pos::new(4, 2), "x", crate::RecalcMode::Document)
            .unwrap();
        let every = all(&app, 0);
        assert_eq!(every.active, Pos::new(0, 0), "the view goes home");
        assert_eq!(every.rect(), (Pos::new(0, 0), Pos::new(4, 2)));
    }

    #[test]
    fn only_a_whole_row_or_column_is_cut_to_the_sheet_in_use() {
        let used = (10, 4);
        assert_eq!(
            target(Selection::whole_col(2), used),
            (Pos::new(0, 2), Pos::new(9, 2))
        );
        assert_eq!(
            target(Selection::whole_row(3), used),
            (Pos::new(3, 0), Pos::new(3, 3))
        );
        let dragged = Selection {
            anchor: Pos::new(40, 7),
            active: Pos::new(0, 0),
        };
        assert_eq!(target(dragged, used), (Pos::new(0, 0), Pos::new(40, 7)));
        // An empty sheet still acts on the cell the column starts at, rather than nothing.
        assert_eq!(
            target(Selection::whole_col(2), (0, 0)),
            (Pos::new(0, 2), Pos::new(0, 2))
        );
    }
}
