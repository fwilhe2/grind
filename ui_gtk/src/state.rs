// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a keystroke means *while editing* — the modes, and the one function that decides.
//!
//! Pure, like [`crate::keymap`], and for the same reason: this is the part of a spreadsheet
//! that everyone has an opinion about, everyone notices when it is wrong, and no display is
//! needed to check. The widget turns the answer into GTK calls.
//!
//! The modes are Excel's, by name (doc/gtk-shell.md):
//!
//! * **Ready** — the grid has the keys. A printable character starts an edit seeded with it,
//!   F2 and a double-click start one seeded with the cell, everything else navigates.
//! * **Enter** — an edit that began by typing. An arrow key *commits* and moves, because
//!   the caret has nowhere useful to go in text the user has only just started.
//! * **Edit** — an edit that began from the cell's own content (F2, double-click). An arrow
//!   key moves the caret instead, because the text is worth navigating. F2 toggles.
//!
//! That difference between Enter and Edit is the whole reason two modes exist, and typing
//! `=` then pressing → is the case it exists for: in M6 that becomes Point mode and starts
//! building a reference, which is why the mode lives here rather than as a bool in the
//! widget.
//!
//! ponytail: [`on_key`] does not take the text or the caret yet. Point mode needs both (its
//! predicate is over `(text, caret)`), and that is the milestone that adds them.

use crate::keymap::{self, Dir, Key, Mods};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Ready,
    Enter,
    Edit,
}

impl Mode {
    pub fn is_editing(self) -> bool {
        !matches!(self, Mode::Ready)
    }
}

/// What an edit starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    /// Typing over a cell replaces it — the character is the whole new content.
    Char(char),
    /// F2 or a double-click: the cell's own text, ready to be amended.
    Cell,
}

/// What the widget should do about a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not ours. The event keeps travelling, which is what leaves the editor's own key
    /// handling — selection, caret, input method — untouched.
    Passthrough,
    /// Move the selection; Ready mode only.
    Navigate(keymap::Action),
    Begin(Seed),
    /// Store what the editor holds, then move.
    Commit(Option<Dir>),
    /// Throw the edit away. The document is not touched.
    Cancel,
    /// Empty the selection.
    Clear,
    /// F2 while editing: Enter ↔ Edit.
    ToggleMode,
}

/// The whole state machine. One `match`, no memory beyond the mode it is given.
pub fn on_key(mode: Mode, key: Key, mods: Mods) -> Outcome {
    match mode {
        Mode::Ready => ready(key, mods),
        Mode::Enter | Mode::Edit => editing(mode, key, mods),
    }
}

fn ready(key: Key, mods: Mods) -> Outcome {
    match key {
        // Ctrl and Alt combinations belong to the window's actions and its menus; claiming
        // them here is how a shortcut ends up doing two things.
        _ if mods.ctrl || mods.alt => navigate(key, mods),
        Key::F2 => Outcome::Begin(Seed::Cell),
        Key::Delete | Key::Backspace => Outcome::Clear,
        // A printable character starts an edit and *is* the edit's first character. Control
        // characters reach here as themselves — Escape is `\u{1b}` — and are not printable.
        Key::Char(c) if !c.is_control() => Outcome::Begin(Seed::Char(c)),
        _ => navigate(key, mods),
    }
}

fn navigate(key: Key, mods: Mods) -> Outcome {
    keymap::action_for(key, mods).map_or(Outcome::Passthrough, Outcome::Navigate)
}

fn editing(mode: Mode, key: Key, mods: Mods) -> Outcome {
    let reverse = |a, b| match mods.shift {
        true => a,
        false => b,
    };
    match key {
        Key::Escape => Outcome::Cancel,
        Key::F2 => Outcome::ToggleMode,
        Key::Return if !mods.ctrl => Outcome::Commit(Some(reverse(Dir::Up, Dir::Down))),
        Key::Tab if !mods.ctrl => Outcome::Commit(Some(reverse(Dir::Left, Dir::Right))),
        // The Enter/Edit difference, and the only place it shows.
        Key::Left if mode == Mode::Enter => Outcome::Commit(Some(Dir::Left)),
        Key::Right if mode == Mode::Enter => Outcome::Commit(Some(Dir::Right)),
        Key::Up if mode == Mode::Enter => Outcome::Commit(Some(Dir::Up)),
        Key::Down if mode == Mode::Enter => Outcome::Commit(Some(Dir::Down)),
        _ => Outcome::Passthrough,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Motion;

    fn plain() -> Mods {
        Mods::default()
    }

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Default::default()
        }
    }

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Default::default()
        }
    }

    #[test]
    fn typing_starts_an_edit_that_replaces_the_cell() {
        assert_eq!(
            on_key(Mode::Ready, Key::Char('7'), plain()),
            Outcome::Begin(Seed::Char('7'))
        );
        assert_eq!(
            on_key(Mode::Ready, Key::F2, plain()),
            Outcome::Begin(Seed::Cell)
        );
        // A shortcut is not typing, and must not start an edit with a stray character.
        assert_eq!(on_key(Mode::Ready, Key::Char('s'), ctrl()), Outcome::Passthrough);
        assert_eq!(
            on_key(Mode::Ready, Key::Char('a'), ctrl()),
            Outcome::Navigate(keymap::Action::SelectAll)
        );
    }

    #[test]
    fn ready_mode_still_navigates() {
        assert_eq!(
            on_key(Mode::Ready, Key::Down, plain()),
            Outcome::Navigate(keymap::Action::Move {
                motion: Motion::By(Dir::Down),
                extend: false
            })
        );
        assert_eq!(on_key(Mode::Ready, Key::Delete, plain()), Outcome::Clear);
    }

    /// The reason there are two editing modes at all.
    #[test]
    fn an_arrow_commits_while_typing_and_moves_the_caret_while_amending() {
        assert_eq!(
            on_key(Mode::Enter, Key::Right, plain()),
            Outcome::Commit(Some(Dir::Right))
        );
        assert_eq!(on_key(Mode::Edit, Key::Right, plain()), Outcome::Passthrough);
        assert_eq!(on_key(Mode::Edit, Key::F2, plain()), Outcome::ToggleMode);
    }

    #[test]
    fn enter_and_tab_commit_and_shift_reverses_them() {
        for mode in [Mode::Enter, Mode::Edit] {
            assert_eq!(on_key(mode, Key::Return, plain()), Outcome::Commit(Some(Dir::Down)));
            assert_eq!(on_key(mode, Key::Return, shift()), Outcome::Commit(Some(Dir::Up)));
            assert_eq!(on_key(mode, Key::Tab, plain()), Outcome::Commit(Some(Dir::Right)));
            assert_eq!(on_key(mode, Key::Tab, shift()), Outcome::Commit(Some(Dir::Left)));
            assert_eq!(on_key(mode, Key::Escape, plain()), Outcome::Cancel);
        }
    }

    /// Everything the machine does not claim has to reach the editor, or typing stops
    /// working in the subtlest possible way.
    #[test]
    fn the_editor_keeps_every_key_this_does_not_claim() {
        for key in [Key::Char('x'), Key::Home, Key::End, Key::PageUp, Key::Other] {
            assert_eq!(on_key(Mode::Edit, key, plain()), Outcome::Passthrough, "{key:?}");
        }
    }
}
