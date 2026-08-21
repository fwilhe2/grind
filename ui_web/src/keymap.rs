// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a keystroke means, as a pure function — the same shape as
//! `ui_gtk/src/keymap.rs` and `ui_tui/src/keymap.rs`, and for the same reason: this
//! is the part everyone has an opinion about and nobody needs a browser to check.
//!
//! A browser key arrives as a *string* (`"a"`, `"ArrowLeft"`, `"F2"`), so there is no
//! keyval table to translate — [`Chord`] is the event's own vocabulary with the
//! platform question already answered: ⌘ on macOS and Ctrl everywhere else are one
//! `primary` flag, resolved by the caller, so this file never asks what it is
//! running on.
//!
//! The two-mode split `ui_gtk/src/state.rs` documents (Enter vs Edit) is *not* here.
//! This shell edits in one place, the formula bar, and an arrow key there moves the
//! text caret — the browser's own behaviour, which is the right one for an `<input>`
//! and is why editing claims only the three keys that end an edit.

use sheet_core::Pos;

use crate::{MAX_COLS, MAX_ROWS};

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

/// A key as the page reports it, with ⌘/Ctrl already collapsed into one flag.
#[derive(Clone, Copy, Debug)]
pub struct Chord<'a> {
    pub key: &'a str,
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
}

/// What the shell should do about a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move {
        motion: Motion,
        extend: bool,
    },
    /// Start editing. `Some(c)` is a printable key, which replaces the cell and is
    /// its first character; `None` is F2, which keeps what the cell holds.
    Begin(Option<char>),
    /// Store what is being edited, then move — Enter and Tab.
    Commit(Option<Dir>),
    Cancel,
    /// Empty the selection, which `App::enter` spells as entering nothing.
    Clear,
    Undo,
    Redo,
    Open,
    Save,
    Recalc,
}

/// The whole map. `None` means the shell does not claim the key, and the browser
/// keeps it — Ctrl+T and F5 are not ours to take.
pub fn action_for(chord: &Chord, editing: bool) -> Option<Action> {
    let reverse = |a, b| match chord.shift {
        true => a,
        false => b,
    };
    // These end an edit whatever else is going on, so they are decided first and in
    // both modes: Tab and Return are motions below, and editing must not lose them.
    match chord.key {
        "Enter" => return Some(Action::Commit(Some(reverse(Dir::Up, Dir::Down)))),
        "Tab" => return Some(Action::Commit(Some(reverse(Dir::Left, Dir::Right)))),
        "Escape" => return Some(Action::Cancel),
        _ => {}
    }
    // Everything else while editing belongs to the `<input>`: its caret, its
    // selection, its input method.
    if editing {
        return None;
    }

    if chord.primary {
        return match chord.key {
            "z" | "Z" if chord.shift => Some(Action::Redo),
            "z" | "Z" => Some(Action::Undo),
            "y" | "Y" => Some(Action::Redo),
            "o" | "O" => Some(Action::Open),
            "s" | "S" => Some(Action::Save),
            "Home" => Some(Action::Move {
                motion: Motion::SheetStart,
                extend: chord.shift,
            }),
            "End" => Some(Action::Move {
                motion: Motion::SheetEnd,
                extend: chord.shift,
            }),
            _ => None,
        };
    }
    if chord.alt {
        return None;
    }

    let motion = |motion| {
        Some(Action::Move {
            motion,
            extend: chord.shift,
        })
    };
    match chord.key {
        "ArrowLeft" => motion(Motion::By(Dir::Left)),
        "ArrowRight" => motion(Motion::By(Dir::Right)),
        "ArrowUp" => motion(Motion::By(Dir::Up)),
        "ArrowDown" => motion(Motion::By(Dir::Down)),
        "PageDown" => motion(Motion::Page(Dir::Down)),
        "PageUp" => motion(Motion::Page(Dir::Up)),
        "Home" => motion(Motion::RowStart),
        "End" => motion(Motion::RowEnd),
        "F2" => Some(Action::Begin(None)),
        "F9" => Some(Action::Recalc),
        "Delete" | "Backspace" => Some(Action::Clear),
        // A printable key starts an edit and *is* its first character. The browser
        // spells every named key with more than one character, which is exactly the
        // test for "printable" — `"a"` is a key, `"ArrowUp"` is a name.
        key => match key.chars().count() == 1 {
            true => key
                .chars()
                .next()
                .filter(|c| !c.is_control())
                .map(|c| Action::Begin(Some(c))),
            false => None,
        },
    }
}

/// Apply a motion. Clamped to the sheet's own limits for a plain move, and to
/// `extent` — the used region — for the edges, matching the other two shells.
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

/// Where the cursor goes after a commit — `ui_gtk`'s `state::after_commit` without
/// the tab-column memory, which needs a run of Tabs to be worth anything and is one
/// more thing to keep in step across three shells.
pub fn after_commit(from: Pos, direction: Option<Dir>) -> Pos {
    match direction {
        Some(dir) => step(from, dir, 1),
        None => from,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(key: &str) -> Chord<'_> {
        Chord {
            key,
            primary: false,
            shift: false,
            alt: false,
        }
    }

    fn primary(key: &str) -> Chord<'_> {
        Chord {
            primary: true,
            ..plain(key)
        }
    }

    #[test]
    fn the_arrows_move_and_shift_extends() {
        assert_eq!(
            action_for(&plain("ArrowDown"), false),
            Some(Action::Move {
                motion: Motion::By(Dir::Down),
                extend: false
            })
        );
        let shifted = Chord {
            shift: true,
            ..plain("ArrowRight")
        };
        assert_eq!(
            action_for(&shifted, false),
            Some(Action::Move {
                motion: Motion::By(Dir::Right),
                extend: true
            })
        );
    }

    #[test]
    fn a_printable_key_starts_an_edit_and_a_named_one_does_not() {
        assert_eq!(
            action_for(&plain("7"), false),
            Some(Action::Begin(Some('7')))
        );
        assert_eq!(
            action_for(&plain("="), false),
            Some(Action::Begin(Some('=')))
        );
        assert_eq!(action_for(&plain("F2"), false), Some(Action::Begin(None)));
        // Named keys this shell has no use for stay with the browser.
        for key in ["Shift", "CapsLock", "F5", "ArrowLeftFake"] {
            assert_eq!(action_for(&plain(key), false), None, "{key}");
        }
    }

    #[test]
    fn a_shortcut_is_not_typing() {
        // Ctrl+S must save, not enter an "s" into the cell.
        assert_eq!(action_for(&primary("s"), false), Some(Action::Save));
        assert_eq!(action_for(&primary("o"), false), Some(Action::Open));
        assert_eq!(action_for(&primary("z"), false), Some(Action::Undo));
        assert_eq!(
            action_for(
                &Chord {
                    shift: true,
                    ..primary("z")
                },
                false
            ),
            Some(Action::Redo)
        );
        assert_eq!(action_for(&primary("y"), false), Some(Action::Redo));
        // And an unclaimed one is still the browser's.
        assert_eq!(action_for(&primary("t"), false), None);
    }

    /// The rule that makes editing work at all: the three keys that end an edit are
    /// claimed in both modes, and every other key is left to the `<input>`.
    #[test]
    fn editing_claims_only_the_keys_that_end_it() {
        assert_eq!(
            action_for(&plain("Enter"), true),
            Some(Action::Commit(Some(Dir::Down)))
        );
        assert_eq!(
            action_for(&plain("Tab"), true),
            Some(Action::Commit(Some(Dir::Right)))
        );
        assert_eq!(action_for(&plain("Escape"), true), Some(Action::Cancel));
        for key in ["a", "ArrowLeft", "Home", "Delete", "F2"] {
            assert_eq!(action_for(&plain(key), true), None, "{key}");
        }
    }

    #[test]
    fn shift_reverses_the_keys_that_commit() {
        let shifted = |key| {
            action_for(
                &Chord {
                    shift: true,
                    ..plain(key)
                },
                true,
            )
        };
        assert_eq!(shifted("Enter"), Some(Action::Commit(Some(Dir::Up))));
        assert_eq!(shifted("Tab"), Some(Action::Commit(Some(Dir::Left))));
    }

    #[test]
    fn the_edges_use_the_used_extent_and_a_plain_move_the_sheet_limit() {
        let extent = (10, 5);
        assert_eq!(
            moved(Pos::new(2, 3), Motion::RowEnd, extent, 1),
            Pos::new(2, 4)
        );
        assert_eq!(
            moved(Pos::new(2, 3), Motion::RowStart, extent, 1),
            Pos::new(2, 0)
        );
        assert_eq!(
            moved(Pos::new(2, 3), Motion::SheetEnd, extent, 1),
            Pos::new(9, 4)
        );
        assert_eq!(
            moved(Pos::new(2, 3), Motion::SheetStart, extent, 1),
            Pos::new(0, 0)
        );
        // An empty sheet must not underflow into the last row of the address space.
        assert_eq!(
            moved(Pos::new(0, 0), Motion::SheetEnd, (0, 0), 1),
            Pos::new(0, 0)
        );
    }

    #[test]
    fn a_move_stops_at_the_edge_of_the_sheet() {
        assert_eq!(
            moved(Pos::new(0, 0), Motion::By(Dir::Up), (0, 0), 1),
            Pos::new(0, 0)
        );
        assert_eq!(
            moved(Pos::new(0, 0), Motion::By(Dir::Left), (0, 0), 1),
            Pos::new(0, 0)
        );
        assert_eq!(
            moved(Pos::new(MAX_ROWS - 1, 0), Motion::By(Dir::Down), (0, 0), 1).row,
            MAX_ROWS - 1
        );
        assert_eq!(
            moved(Pos::new(1, 0), Motion::Page(Dir::Down), (0, 0), 20).row,
            21
        );
        assert_eq!(
            moved(Pos::new(1, 0), Motion::Page(Dir::Up), (0, 0), 20).row,
            0
        );
    }

    #[test]
    fn enter_goes_down_and_tab_goes_right() {
        assert_eq!(
            after_commit(Pos::new(3, 3), Some(Dir::Down)),
            Pos::new(4, 3)
        );
        assert_eq!(
            after_commit(Pos::new(3, 3), Some(Dir::Right)),
            Pos::new(3, 4)
        );
        assert_eq!(after_commit(Pos::new(0, 0), Some(Dir::Up)), Pos::new(0, 0));
        assert_eq!(after_commit(Pos::new(3, 3), None), Pos::new(3, 3));
    }
}
