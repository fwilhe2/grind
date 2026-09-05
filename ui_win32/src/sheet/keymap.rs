// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what, and where it moves the selection — as pure functions.
//!
//! **No Windows types outside one `#[cfg(windows)]` test.** The window turns a `WM_KEYDOWN`
//! into a [`Key`] with [`key_for`] and hands it over, so the whole of what a keystroke *means*
//! is decided in code that compiles and tests on the Linux machine this repository is developed
//! on (`doc/windows-shell.md`, "The crate").
//!
//! ## The virtual-key codes are written down here, not imported
//!
//! [`key_for`] takes a bare `u32` and compares it against constants declared below, rather than
//! against `windows::Win32::UI::Input::KeyboardAndMouse::VK_LEFT` and its neighbours. That
//! looks backwards and is the point: importing them would drag the `windows` crate into this
//! file and take the whole key table off the portable side, which is the half of this shell
//! that can actually be run here.
//!
//! The risk that buys — a number copied out of `winuser.h` wrongly — is answered rather than
//! accepted: `tests::the_virtual_key_codes_are_the_ones_windows_uses` is compiled **only on
//! Windows** and asserts every constant below against Windows' own metadata. So the table is
//! portable, and it is still pinned to the platform by the runner that can see the platform.
//!
//! ## The selection is presentation state
//!
//! An anchor and an active cell, and nothing else. The core is never told about it: a range
//! reaches `App` as two positions when something is actually *done* to it. That is the same
//! arrangement `ui_sheet_gtk/src/keymap.rs` has, and this module deliberately mirrors its
//! vocabulary — [`Selection`], [`Motion`], [`Extent`], [`moved`] and the Ctrl+arrow rule read
//! the same in both, so a difference between the two shells' navigation would be visible as a
//! difference between two files rather than hidden in an event handler.
//!
//! ponytail: that mirroring *is* a second copy. The two cannot share one today, because the
//! GTK version lives in a crate that needs GTK to compile at all and this one must not. The
//! upgrade path is `grind-sheet` — navigation over a used extent is spreadsheet vocabulary, not
//! toolkit vocabulary — and the trigger is a **third** copy, or the first time the two answer a
//! motion differently. Two copies with the same tests is a cost; three is a bug waiting.

use grind_sheet::{MAX_COLS, MAX_ROWS, Pos};

use super::geom::Sizes;

/// A key, as this shell cares about it.
///
/// W2's set was navigation plus the two verbs that are navigation in disguise (select
/// everything, go to an address). W3 adds the four keys that only mean anything once there is
/// something to edit — `F2`, `Delete`, `Backspace` and `F9` — and `sheet/state.rs` is the mode
/// that interprets them. The printable characters are **not** here: a `WM_KEYDOWN` carries a
/// key rather than a character, and typed text comes from `WM_CHAR`, which has been through the
/// keyboard layout and the IME.
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
    Escape,
    Delete,
    Backspace,
    /// Amend the cell rather than replace it — Excel's, and everybody's.
    F2,
    /// Go to an address — the name box's key, and Excel's.
    F5,
    /// Recalculate. Excel's key, and the one this shell needs most, since a document whose
    /// cached values this build cannot reproduce is left stale on purpose.
    F9,
    /// A letter or digit, as the keyboard reports it: **already upper case**, because a
    /// `WM_KEYDOWN` carries the key rather than the character. W3's typed text comes from
    /// `WM_CHAR` instead, which is the message that has been through the layout and the IME.
    Char(char),
    /// Anything this shell does not claim, which must go back to `DefWindowProc` so that
    /// Alt+F4, the system menu and the accelerators Windows owns keep working.
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
    /// The next edge of the data — Ctrl+arrow, and [`data_edge`] has the rule.
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

/// What a keystroke asks for. `None` means this shell does not own the key and the message
/// must keep travelling — which is what leaves Alt+F4 and the system menu alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move {
        motion: Motion,
        extend: bool,
    },
    /// Ctrl+A — everything the sheet uses.
    SelectAll,
    /// F5 / Ctrl+G — put the caret in the name box.
    GoTo,
}

// The virtual-key codes, from `winuser.h`. See the module comment for why they are written
// here rather than imported, and `tests` for what pins them.
const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0d;
const VK_ESCAPE: u32 = 0x1b;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_DELETE: u32 = 0x2e;
const VK_F2: u32 = 0x71;
const VK_F5: u32 = 0x74;
const VK_F9: u32 = 0x78;

/// A virtual-key code as this shell's [`Key`].
///
/// `0x30`–`0x39` and `0x41`–`0x5a` are the one range Windows does not give a `VK_` name: they
/// *are* the ASCII codes for `0`–`9` and `A`–`Z`, which is why [`Key::Char`] always arrives
/// upper case.
pub fn key_for(vk: u32) -> Key {
    match vk {
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        VK_TAB => Key::Tab,
        VK_RETURN => Key::Return,
        VK_ESCAPE => Key::Escape,
        VK_BACK => Key::Backspace,
        VK_DELETE => Key::Delete,
        VK_F2 => Key::F2,
        VK_F5 => Key::F5,
        VK_F9 => Key::F9,
        0x30..=0x39 | 0x41..=0x5a => Key::Char(char::from(vk as u8)),
        _ => Key::Other,
    }
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
        // Tab and Return walk the sheet the way they will while typing, so the habit is the
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
        // F5 is Excel's Go To and Ctrl+G is everybody's; both land in the name box, which is
        // the whole of this shell's go-to. There is no dialog and no palette (decision 4).
        Key::F5 if !mods.ctrl => Some(Action::GoTo),
        Key::Char(c) if mods.ctrl && !mods.shift => match c {
            'A' => Some(Action::SelectAll),
            'G' => Some(Action::GoTo),
            // X, C and V are `menu::accelerator`'s (W4's clipboard) and are deliberately not
            // claimed here: `sheet/state.rs::ready` consults that table first, so this arm
            // only ever sees a Ctrl+letter with no verb of its own.
            _ => None,
        },
        _ => None,
    }
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
    /// How many rows a PageUp/PageDown moves.
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
/// from the document: the window backs it with `App::get` and a test backs it with a set. That
/// is what keeps the *rule* here and the *reads* there.
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
/// cursor that lands on one is invisible: `doc/windows-shell.md` decides this shell draws a
/// hidden track as *gone*, and a cursor parked on nothing is that decision's sharp edge. Every
/// spreadsheet steps over them, and this is where it happens.
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
/// of that run; from one whose neighbour is empty, go to the next occupied cell. The stop is
/// the **used extent** rather than row 1048576, so Ctrl+Down in an empty column lands on the
/// last row that has anything rather than a million rows into blank space — the same bound the
/// scrollbar uses, and the same one `ui_sheet_gtk` chose.
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

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Default::default()
        }
    }

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

    /// The whole reason the constants may be written down in portable code: on Windows, and
    /// only there, they are checked against Windows' own metadata. A typo in the table above
    /// fails this build rather than producing a key that silently does nothing.
    #[test]
    #[cfg(windows)]
    fn the_virtual_key_codes_are_the_ones_windows_uses() {
        use windows::Win32::UI::Input::KeyboardAndMouse as vk;
        for (ours, theirs, name) in [
            (VK_BACK, vk::VK_BACK, "VK_BACK"),
            (VK_TAB, vk::VK_TAB, "VK_TAB"),
            (VK_RETURN, vk::VK_RETURN, "VK_RETURN"),
            (VK_ESCAPE, vk::VK_ESCAPE, "VK_ESCAPE"),
            (VK_PRIOR, vk::VK_PRIOR, "VK_PRIOR"),
            (VK_NEXT, vk::VK_NEXT, "VK_NEXT"),
            (VK_END, vk::VK_END, "VK_END"),
            (VK_HOME, vk::VK_HOME, "VK_HOME"),
            (VK_LEFT, vk::VK_LEFT, "VK_LEFT"),
            (VK_UP, vk::VK_UP, "VK_UP"),
            (VK_RIGHT, vk::VK_RIGHT, "VK_RIGHT"),
            (VK_DOWN, vk::VK_DOWN, "VK_DOWN"),
            (VK_DELETE, vk::VK_DELETE, "VK_DELETE"),
            (VK_F2, vk::VK_F2, "VK_F2"),
            (VK_F5, vk::VK_F5, "VK_F5"),
            (VK_F9, vk::VK_F9, "VK_F9"),
        ] {
            assert_eq!(ours, u32::from(theirs.0), "{name}");
        }
        // The unnamed range: Windows really does use ASCII for the letters and digits, which
        // is what `key_for` relies on to produce `Key::Char`.
        assert_eq!(key_for(u32::from(b'A')), Key::Char('A'));
        assert_eq!(key_for(u32::from(b'0')), Key::Char('0'));
    }

    #[test]
    fn the_arrows_are_the_codes_windows_sends() {
        assert_eq!(key_for(0x25), Key::Left);
        assert_eq!(key_for(0x28), Key::Down);
        assert_eq!(key_for(0x74), Key::F5);
        assert_eq!(key_for(0x71), Key::F2);
        assert_eq!(key_for(0x2e), Key::Delete);
        assert_eq!(key_for(0x08), Key::Backspace);
        // A key with no meaning here has to stay `Other`, so the window hands it back to
        // `DefWindowProc` rather than swallowing it.
        assert_eq!(key_for(0x12), Key::Other, "Alt");
        assert_eq!(key_for(0x00), Key::Other);
    }

    #[test]
    fn a_letter_arrives_upper_case_because_a_key_is_not_a_character() {
        // `WM_KEYDOWN` reports the *key*, so Ctrl+A and Ctrl+Shift+A are the same code and
        // the modifiers are what tell them apart. Lower-casing here would be inventing a
        // character the message did not carry.
        assert_eq!(key_for(u32::from(b'A')), Key::Char('A'));
        assert_eq!(action_for(Key::Char('A'), ctrl()), Some(Action::SelectAll));
    }

    #[test]
    fn ctrl_changes_an_arrow_from_a_step_into_an_edge() {
        assert_eq!(
            action_for(Key::Down, Mods::default()),
            Some(Action::Move {
                motion: Motion::By(Dir::Down),
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Down, ctrl()),
            Some(Action::Move {
                motion: Motion::Edge(Dir::Down),
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Down, shift()),
            Some(Action::Move {
                motion: Motion::By(Dir::Down),
                extend: true
            })
        );
    }

    /// Alt belongs to the menu bar (decision 4), so nothing here may claim it.
    #[test]
    fn alt_is_never_claimed() {
        let alt = Mods {
            alt: true,
            ..Default::default()
        };
        for key in [Key::Left, Key::Home, Key::Return, Key::Char('A'), Key::F5] {
            assert_eq!(action_for(key, alt), None, "{key:?}");
        }
    }

    /// A key this shell does not own must keep travelling, or Alt+F4 and the system menu stop
    /// working — the failure mode of a window that claims everything it is sent.
    #[test]
    fn unclaimed_keys_are_left_alone() {
        assert_eq!(action_for(Key::Other, Mods::default()), None);
        assert_eq!(action_for(Key::Escape, Mods::default()), None);
        // The editing keys belong to `state.rs` and the verbs to `menu.rs`; this table is
        // navigation and must not answer for either.
        for key in [Key::F2, Key::F9, Key::Delete, Key::Backspace] {
            assert_eq!(action_for(key, Mods::default()), None, "{key:?}");
        }
        // The clipboard is W4's; claiming the letters now would be a Ctrl+C that does nothing.
        for c in ['C', 'X', 'V'] {
            assert_eq!(action_for(Key::Char(c), ctrl()), None, "{c}");
        }
    }

    #[test]
    fn both_go_to_keys_reach_the_name_box() {
        assert_eq!(action_for(Key::F5, Mods::default()), Some(Action::GoTo));
        assert_eq!(action_for(Key::Char('G'), ctrl()), Some(Action::GoTo));
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
