// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what, and what it does to the selection — as pure functions.
//!
//! **No GTK types.** The widget translates a `gdk::Key` into [`Key`] and hands it over, so
//! everything decidable about navigation unit-tests with no display and no compositor
//! (doc/gtk-shell.md, the same rule `geom.rs` follows). A second shell needs this logic
//! unchanged, which is the other reason it is not written inline in an event handler.
//!
//! The selection is **presentation state**: an anchor and an active cell, and nothing else.
//! The core is not told about it and does not need to be — a range is passed to `App` as two
//! positions when something is actually done to it.
//!
//! ponytail: [`action_for`] takes no mode, because Ready is the only one there is. Editing
//! adds `Enter`, `Edit` and `Point`, and the signature grows a `mode` parameter then —
//! `state.rs` in the plan owns that machine, and this one keeps the keys.

use sheet_core::Pos;

/// A key, as this shell cares about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    Return,
    /// A printable character, already case-folded for matching.
    Char(char),
    /// Anything this shell does not claim, which must keep travelling.
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

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
    /// The next edge of the data — Ctrl+arrow, and `data_edge` below has the rule.
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

/// What a keystroke asks for. Returning `None` means the shell does not own this key, and
/// the event must keep travelling — which is what leaves the toolkit's own bindings, and
/// later the editor's input method, working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move { motion: Motion, extend: bool },
    /// Ctrl+A — everything the sheet uses.
    SelectAll,
}

/// The key map. One table, no state.
pub fn action_for(key: Key, mods: Mods) -> Option<Action> {
    if mods.alt {
        return None;
    }
    let go = |motion, extend| Some(Action::Move { motion, extend });
    let arrow = |dir| {
        go(
            match mods.ctrl {
                true => Motion::Edge(dir),
                false => Motion::By(dir),
            },
            mods.shift,
        )
    };
    match key {
        Key::Left => arrow(Dir::Left),
        Key::Right => arrow(Dir::Right),
        Key::Up => arrow(Dir::Up),
        Key::Down => arrow(Dir::Down),
        Key::Home if mods.ctrl => go(Motion::SheetStart, mods.shift),
        Key::Home => go(Motion::RowStart, mods.shift),
        Key::End if mods.ctrl => go(Motion::SheetEnd, mods.shift),
        Key::End => go(Motion::RowEnd, mods.shift),
        Key::PageUp => go(Motion::Page(Dir::Up), mods.shift),
        Key::PageDown => go(Motion::Page(Dir::Down), mods.shift),
        // Tab and Return walk the sheet the way they do while typing, so the habit is the
        // same before there is anything to type. Shift reverses rather than extends.
        Key::Tab if !mods.ctrl => go(
            Motion::By(match mods.shift {
                true => Dir::Left,
                false => Dir::Right,
            }),
            false,
        ),
        Key::Return if !mods.ctrl => go(
            Motion::By(match mods.shift {
                true => Dir::Up,
                false => Dir::Down,
            }),
            false,
        ),
        Key::Char('a') if mods.ctrl && !mods.shift => Some(Action::SelectAll),
        _ => None,
    }
}

/// An anchor and an active cell — the whole of what a selection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Where the selection started; a plain click puts both here.
    pub anchor: Pos,
    /// The cell that has the cursor, and the one an edit would land in.
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

    /// The selection as an inclusive rectangle, top-left first — which is the order every
    /// `App` method that takes two positions wants.
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

    /// How many cells are inside, saturating — a whole-sheet selection is not a `u32`.
    pub fn cells(&self) -> u64 {
        let (start, end) = self.rect();
        u64::from(end.row - start.row + 1) * u64::from(end.col - start.col + 1)
    }
}

/// What the sheet's occupied region is, as far as navigation cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// One past the last used row and column — `App::used_extent`, verbatim.
    pub rows: u32,
    pub cols: u32,
    /// How many rows fit on screen, for `Page`.
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
/// `occupied` answers "does this cell hold anything", and is a closure because the answer
/// comes from the document: the widget backs it with viewport-sized reads, and a test backs
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

/// `by` cells in a direction, stopping at the sheet's edges.
fn step(from: Pos, dir: Dir, by: u32) -> Pos {
    let (rows, cols) = (crate::geom::MAX_ROWS - 1, crate::geom::MAX_COLS - 1);
    match dir {
        Dir::Left => Pos::new(from.row, from.col.saturating_sub(by)),
        Dir::Right => Pos::new(from.row, (from.col + by).min(cols)),
        Dir::Up => Pos::new(from.row.saturating_sub(by), from.col),
        Dir::Down => Pos::new((from.row + by).min(rows), from.col),
    }
}

/// Ctrl+arrow: the far end of a run of cells, or the first cell across a gap.
///
/// The rule every spreadsheet has: from a cell whose neighbour is occupied, go to the last
/// cell of that run; from one whose neighbour is empty, go to the next occupied cell. What
/// differs here is the stop: this bounds the scan by the **used extent** rather than by row
/// 1048576, so Ctrl+Down in an empty column lands on the last row that has anything rather
/// than a million rows into blank space — the same bound the scrollbar uses.
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

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Default::default()
        }
    }

    fn extent() -> Extent {
        Extent {
            rows: 10,
            cols: 5,
            page: 4,
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
    fn a_ctrl_modified_key_never_also_moves_one_cell() {
        assert_eq!(
            action_for(Key::Down, ctrl()),
            Some(Action::Move {
                motion: Motion::Edge(Dir::Down),
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Down, Mods::default()),
            Some(Action::Move {
                motion: Motion::By(Dir::Down),
                extend: false
            })
        );
    }

    /// Everything the shell does not claim has to keep travelling, or the toolkit's own
    /// bindings — and later the editor's input method — stop working.
    #[test]
    fn unclaimed_keys_are_left_alone() {
        assert_eq!(action_for(Key::Char('x'), Mods::default()), None);
        assert_eq!(action_for(Key::Other, ctrl()), None);
        // Alt belongs to the window manager and the menus.
        assert_eq!(
            action_for(
                Key::Left,
                Mods {
                    alt: true,
                    ..Default::default()
                }
            ),
            None
        );
    }

    #[test]
    fn shift_extends_from_the_anchor_and_a_plain_move_collapses() {
        let start = Selection::at(Pos::new(2, 2));
        let occupied = sheet(&[]);
        let extended = moved(start, Motion::By(Dir::Down), true, extent(), &occupied);
        assert_eq!(extended.anchor, Pos::new(2, 2));
        assert_eq!(extended.active, Pos::new(3, 2));
        assert_eq!(extended.cells(), 2);

        let collapsed = moved(extended, Motion::By(Dir::Right), false, extent(), &occupied);
        assert!(collapsed.is_single());
        assert_eq!(collapsed.active, Pos::new(3, 3));
    }

    #[test]
    fn a_rectangle_reads_top_left_first_whichever_way_it_was_dragged() {
        let up_left = Selection {
            anchor: Pos::new(5, 5),
            active: Pos::new(2, 1),
        };
        assert_eq!(up_left.rect(), (Pos::new(2, 1), Pos::new(5, 5)));
        assert!(up_left.contains(3, 3) && !up_left.contains(6, 3));
        assert_eq!(up_left.cells(), 4 * 5);
    }

    /// The rule every spreadsheet has, in the three cases it has to get right.
    #[test]
    fn ctrl_arrow_stops_at_the_end_of_a_run_and_at_the_start_of_the_next() {
        // A2:A4 filled, A7 filled, from A1.
        let occupied = sheet(&[(1, 0), (2, 0), (3, 0), (6, 0)]);
        // Into a run: its last cell.
        assert_eq!(go(Pos::new(1, 0), Motion::Edge(Dir::Down), &occupied).row, 3);
        // From the end of a run across a gap: the next occupied cell.
        assert_eq!(go(Pos::new(3, 0), Motion::Edge(Dir::Down), &occupied).row, 6);
        // Past the last of them: the used extent's edge, not row 1048576.
        assert_eq!(go(Pos::new(6, 0), Motion::Edge(Dir::Down), &occupied).row, 9);
        // And backwards.
        assert_eq!(go(Pos::new(6, 0), Motion::Edge(Dir::Up), &occupied).row, 3);
        assert_eq!(go(Pos::new(0, 0), Motion::Edge(Dir::Up), &occupied).row, 0);
    }

    #[test]
    fn the_ends_of_the_sheet_are_the_used_extent() {
        let occupied = sheet(&[]);
        assert_eq!(go(Pos::new(3, 3), Motion::SheetStart, &occupied), Pos::new(0, 0));
        assert_eq!(go(Pos::new(3, 3), Motion::SheetEnd, &occupied), Pos::new(9, 4));
        assert_eq!(go(Pos::new(3, 3), Motion::RowStart, &occupied), Pos::new(3, 0));
        assert_eq!(go(Pos::new(3, 3), Motion::RowEnd, &occupied), Pos::new(3, 4));
    }

    #[test]
    fn paging_moves_a_screenful_and_stops_at_the_top() {
        let occupied = sheet(&[]);
        assert_eq!(go(Pos::new(1, 0), Motion::Page(Dir::Down), &occupied).row, 5);
        assert_eq!(go(Pos::new(1, 0), Motion::Page(Dir::Up), &occupied).row, 0);
    }

    /// An empty document has no used extent, and every motion still has to land somewhere.
    #[test]
    fn an_empty_sheet_navigates_without_underflowing() {
        let empty = Extent {
            rows: 0,
            cols: 0,
            page: 3,
        };
        let occupied = sheet(&[]);
        for motion in [
            Motion::SheetEnd,
            Motion::RowEnd,
            Motion::Edge(Dir::Down),
            Motion::Edge(Dir::Right),
        ] {
            let to = moved(Selection::default(), motion, false, empty, &occupied).active;
            assert_eq!(to, Pos::new(0, 0), "{motion:?}");
        }
    }
}
