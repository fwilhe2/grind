// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Vi-style Normal-mode key handling, as a pure function — the same shape as
//! `ui_sheet_gtk/src/keymap.rs`: key plus modifiers in, an [`Action`] out, no ratatui `Frame` or
//! terminal handle anywhere near it, so the whole map unit-tests with no terminal attached.
//!
//! `KeyCode`/`KeyModifiers` are used directly rather than through a shell-local `Key` enum —
//! unlike GTK's `gdk::Key`, crossterm's is already a plain data type with nothing to
//! translate away.
//!
//! ponytail: `g` alone stands in for vi's `gg` (a real two-key chord would need the caller
//! to track a pending first key). One key does the same job here; upgrade if a document ever
//! needs a literal `g` bound to something else.

use super::{MAX_COLS, MAX_ROWS};
use grind_sheet::Pos;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    By(Dir),
    Page(Dir),
    RowStart,
    RowEnd,
    SheetStart,
    SheetEnd,
}

/// What a Normal-mode key asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    /// `i` / `a` — start editing, keeping the cell's current text.
    Insert,
    /// `c` — start editing from empty.
    Change,
    /// `x` — empty the cell, the typing rule's own "empty input clears it" (`App::enter`).
    Clear,
    Undo,
    Redo,
    /// `:` — open the command line.
    Command,
}

/// The key map. One table, no state — `None` means the shell does not claim the key.
pub fn normal_action(code: KeyCode, mods: KeyModifiers) -> Option<Action> {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let by = |dir| Some(Action::Move(Motion::By(dir)));
    match code {
        KeyCode::Char('h') | KeyCode::Left => by(Dir::Left),
        KeyCode::Char('l') | KeyCode::Right => by(Dir::Right),
        KeyCode::Char('k') | KeyCode::Up => by(Dir::Up),
        KeyCode::Char('j') | KeyCode::Down => by(Dir::Down),
        KeyCode::Char('f') if ctrl => Some(Action::Move(Motion::Page(Dir::Down))),
        KeyCode::Char('b') if ctrl => Some(Action::Move(Motion::Page(Dir::Up))),
        KeyCode::PageDown => Some(Action::Move(Motion::Page(Dir::Down))),
        KeyCode::PageUp => Some(Action::Move(Motion::Page(Dir::Up))),
        KeyCode::Char('0') | KeyCode::Home => Some(Action::Move(Motion::RowStart)),
        KeyCode::Char('$') | KeyCode::End => Some(Action::Move(Motion::RowEnd)),
        KeyCode::Char('g') => Some(Action::Move(Motion::SheetStart)),
        KeyCode::Char('G') => Some(Action::Move(Motion::SheetEnd)),
        KeyCode::Char('x') => Some(Action::Clear),
        KeyCode::Char('i') | KeyCode::Char('a') => Some(Action::Insert),
        KeyCode::Char('c') => Some(Action::Change),
        KeyCode::Char('u') => Some(Action::Undo),
        KeyCode::Char('r') if ctrl => Some(Action::Redo),
        KeyCode::Char(':') => Some(Action::Command),
        _ => None,
    }
}

/// Apply a motion, clamped to the sheet's own limits — [`super::MAX_ROWS`]/[`super::MAX_COLS`]
/// for a plain move, `extent` (the used region) for the row/sheet edges, matching
/// `ui_sheet_gtk/src/keymap.rs`'s `moved`.
pub fn moved(from: Pos, motion: Motion, extent: (u32, u32), page: u32) -> Pos {
    let page = page.max(1);
    match motion {
        Motion::By(dir) => step(from, dir, 1),
        Motion::Page(dir) => step(from, dir, page),
        Motion::RowStart => Pos::new(from.row, 0),
        Motion::RowEnd => Pos::new(from.row, extent.1.saturating_sub(1)),
        Motion::SheetStart => Pos::new(0, 0),
        Motion::SheetEnd => Pos::new(extent.0.saturating_sub(1), extent.1.saturating_sub(1)),
    }
}

fn step(from: Pos, dir: Dir, by: u32) -> Pos {
    let (rows, cols) = (MAX_ROWS - 1, MAX_COLS - 1);
    match dir {
        Dir::Left => Pos::new(from.row, from.col.saturating_sub(by)),
        Dir::Right => Pos::new(from.row, (from.col + by).min(cols)),
        Dir::Up => Pos::new(from.row.saturating_sub(by), from.col),
        Dir::Down => Pos::new((from.row + by).min(rows), from.col),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hjkl_and_the_arrows_agree() {
        for (ch, arrow, dir) in [
            ('h', KeyCode::Left, Dir::Left),
            ('l', KeyCode::Right, Dir::Right),
            ('k', KeyCode::Up, Dir::Up),
            ('j', KeyCode::Down, Dir::Down),
        ] {
            let want = Some(Action::Move(Motion::By(dir)));
            assert_eq!(normal_action(KeyCode::Char(ch), KeyModifiers::NONE), want);
            assert_eq!(normal_action(arrow, KeyModifiers::NONE), want);
        }
    }

    #[test]
    fn ctrl_f_and_b_page_like_vi() {
        assert_eq!(
            normal_action(KeyCode::Char('f'), KeyModifiers::CONTROL),
            Some(Action::Move(Motion::Page(Dir::Down)))
        );
        assert_eq!(
            normal_action(KeyCode::Char('b'), KeyModifiers::CONTROL),
            Some(Action::Move(Motion::Page(Dir::Up)))
        );
        // Unmodified 'f'/'b' are not bound to anything.
        assert_eq!(normal_action(KeyCode::Char('f'), KeyModifiers::NONE), None);
    }

    #[test]
    fn insert_change_and_clear() {
        assert_eq!(
            normal_action(KeyCode::Char('i'), KeyModifiers::NONE),
            Some(Action::Insert)
        );
        assert_eq!(
            normal_action(KeyCode::Char('a'), KeyModifiers::NONE),
            Some(Action::Insert)
        );
        assert_eq!(
            normal_action(KeyCode::Char('c'), KeyModifiers::NONE),
            Some(Action::Change)
        );
        assert_eq!(
            normal_action(KeyCode::Char('x'), KeyModifiers::NONE),
            Some(Action::Clear)
        );
    }

    #[test]
    fn undo_redo_and_command_line() {
        assert_eq!(
            normal_action(KeyCode::Char('u'), KeyModifiers::NONE),
            Some(Action::Undo)
        );
        assert_eq!(
            normal_action(KeyCode::Char('r'), KeyModifiers::CONTROL),
            Some(Action::Redo)
        );
        assert_eq!(
            normal_action(KeyCode::Char(':'), KeyModifiers::NONE),
            Some(Action::Command)
        );
    }

    #[test]
    fn a_plain_move_stops_at_the_sheet_edge() {
        assert_eq!(step(Pos::new(0, 0), Dir::Left, 1), Pos::new(0, 0));
        assert_eq!(step(Pos::new(0, 0), Dir::Up, 1), Pos::new(0, 0));
        assert_eq!(
            step(Pos::new(MAX_ROWS - 1, 0), Dir::Down, 1),
            Pos::new(MAX_ROWS - 1, 0)
        );
    }

    #[test]
    fn g_and_shift_g_are_the_sheet_ends() {
        let extent = (10, 5);
        assert_eq!(
            moved(Pos::new(3, 3), Motion::SheetStart, extent, 1),
            Pos::new(0, 0)
        );
        assert_eq!(
            moved(Pos::new(3, 3), Motion::SheetEnd, extent, 1),
            Pos::new(9, 4)
        );
    }

    #[test]
    fn row_start_and_end_use_the_used_extent() {
        let extent = (10, 5);
        assert_eq!(
            moved(Pos::new(2, 3), Motion::RowStart, extent, 1),
            Pos::new(2, 0)
        );
        assert_eq!(
            moved(Pos::new(2, 3), Motion::RowEnd, extent, 1),
            Pos::new(2, 4)
        );
    }

    #[test]
    fn paging_moves_a_screenful_and_stops_at_the_top() {
        assert_eq!(
            moved(Pos::new(1, 0), Motion::Page(Dir::Down), (100, 10), 4).row,
            5
        );
        assert_eq!(
            moved(Pos::new(1, 0), Motion::Page(Dir::Up), (100, 10), 4).row,
            0
        );
    }

    #[test]
    fn an_empty_sheet_navigates_without_underflowing() {
        let extent = (0, 0);
        for motion in [Motion::SheetEnd, Motion::RowEnd] {
            assert_eq!(moved(Pos::new(0, 0), motion, extent, 1), Pos::new(0, 0));
        }
    }
}
