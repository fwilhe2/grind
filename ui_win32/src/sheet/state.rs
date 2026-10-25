// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a keystroke means *while editing* — the modes, and the one function that decides.
//! Portable: everyone notices when this is wrong, and no window is needed to check it. `win.rs`
//! turns the answer into Win32 calls. The modes are Excel's:
//!
//! * **Ready** — the grid has the keys. A printable character starts an edit seeded with it, F2
//!   and a double-click one seeded with the cell, Delete empties the selection, everything else
//!   navigates or is a verb.
//! * **Enter** — an edit begun by typing. An arrow *commits and moves*.
//! * **Edit** — an edit begun from the cell's content (F2, double-click, the formula bar). An
//!   arrow moves the caret. F2 toggles.
//!
//! ## Two messages, not one
//!
//! `WM_KEYDOWN` reports a *key* — `A` is `0x41` whatever Shift and the layout say — and `WM_CHAR`
//! the character it produced, after the layout and the IME. So the machine has two doors:
//! [`on_key`] for keys that mean something regardless of layout, and [`typed`] for a character
//! that starts an edit. Deciding "printable" on a virtual-key code would leave an accented
//! character or a Japanese composition unable to start one.
//!
//! ## Not here
//!
//! **Point mode** — an arrow building a reference into a half-typed formula — is answered in
//! `win.rs`'s `on_key` before this table, over `assist::ref_eligible`; [`Outcome`] has no `Point`.
//! Autocomplete and the signature hint are [`super::assist`], also asked first, since the keys a
//! completion list claims already mean something here.
//!
//! ponytail: the GNOME window and the Mac mirror this machine rather than share it: no one key
//! type fits a GTK `Key` carrying its character, these two messages with accelerators first, and
//! AppKit's selectors. What is not about keys is shared
//! (`grind_sheet::formula::display::to_input`). The trigger is a fourth copy, or two answering one
//! keystroke differently for a reason other than input.

use crate::menu::{self, Command};

use super::keymap::{self, Dir, Key, Mods};

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

    /// What F2 does to a mode that is already editing.
    pub fn toggled(self) -> Self {
        match self {
            Mode::Enter => Mode::Edit,
            Mode::Edit | Mode::Ready => Mode::Enter,
        }
    }
}

/// What an edit starts with, and therefore which mode it starts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    /// Typing over a cell replaces it — the character is the whole new content. Enter mode.
    Char(char),
    /// F2, a double-click, or a click in the formula bar: the cell's own text, to amend. Edit
    /// mode.
    Cell,
}

impl Seed {
    pub fn mode(self) -> Mode {
        match self {
            Seed::Char(_) => Mode::Enter,
            Seed::Cell => Mode::Edit,
        }
    }
}

/// What the window should do about a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not ours: in Ready mode back to `DefWindowProc`, so Alt+F4 and the system menu work; while
    /// editing the child `EDIT` gets the key — its caret, selection and own Ctrl+Z.
    Passthrough,
    /// Move the selection; Ready mode only.
    Navigate(keymap::Action),
    /// Run a verb. The same enum a menu item carries, so a keystroke and a menu click reach
    /// one handler (`doc/windows-shell.md`, decision 4).
    Do(Command),
    Begin(Seed),
    /// Store what the editor holds, then move the cursor that way.
    Commit(Option<Dir>),
    /// Throw the edit away. The document is not touched.
    Cancel,
    /// F2 while editing: Enter ↔ Edit.
    ToggleMode,
}

/// The whole state machine. One `match`, and no memory beyond the mode it is told.
pub fn on_key(mode: Mode, key: Key, mods: Mods) -> Outcome {
    match mode {
        Mode::Ready => ready(key, mods),
        Mode::Enter | Mode::Edit => editing(mode, key, mods),
    }
}

fn ready(key: Key, mods: Mods) -> Outcome {
    // Verbs first, so Ctrl+S is Save rather than whatever `S` would otherwise be, and
    // Ctrl+PageDown changes sheet rather than paging.
    if let Some(command) = menu::accelerator(key, mods) {
        return Outcome::Do(command);
    }
    if mods.ctrl || mods.alt {
        return navigate(key, mods);
    }
    match key {
        Key::F2 => Outcome::Begin(Seed::Cell),
        // Backspace clears too, which is what every spreadsheet does and what a user who
        // reaches for the key above Enter expects.
        Key::Delete | Key::Backspace => Outcome::Do(Command::ClearCells),
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
        // An arrow commits and moves in Enter mode and moves the caret in Edit mode — the reason
        // for two modes: someone who typed `12` and pressed → meant the next cell, someone who
        // pressed F2 to fix a typo the next character.
        Key::Left | Key::Right | Key::Up | Key::Down
            if mode == Mode::Enter && !mods.ctrl && !mods.shift =>
        {
            Outcome::Commit(Some(match key {
                Key::Left => Dir::Left,
                Key::Right => Dir::Right,
                Key::Up => Dir::Up,
                _ => Dir::Down,
            }))
        }
        // Everything else belongs to the editor: its caret, its selection, its own undo. That
        // includes Ctrl+S — a verb this shell would rather not run in the middle of a
        // half-typed formula, which is one keystroke away from being stored.
        _ => Outcome::Passthrough,
    }
}

/// A character from `WM_CHAR`: does it start an edit? Only in Ready mode, without Ctrl or Alt, and
/// only for something visible — Escape (`\u{1b}`) and Enter (`\r`) arrive too, since
/// `TranslateMessage` makes a `WM_CHAR` for both.
pub fn typed(mode: Mode, c: char, mods: Mods) -> Option<Seed> {
    let usable = mode == Mode::Ready && !mods.ctrl && !mods.alt && !c.is_control();
    usable.then_some(Seed::Char(c))
}

/// Where the editor's caret goes for a byte offset the parser reported: `EM_SETSEL` counts UTF-16
/// units and a `DisplayError` bytes, so this is [`grind_core::utf16::units_before`], in the `i32`
/// the control takes.
pub fn caret_at(text: &str, byte: usize) -> i32 {
    i32::try_from(grind_core::utf16::units_before(text, byte)).unwrap_or(i32::MAX)
}

/// The other way: the editor's caret as a **byte** offset. `EM_GETSEL` reports UTF-16 units and
/// everything reading the text counts bytes, so this converts ([`grind_core::utf16::byte_of`]), a
/// negative count (no selection) at the start.
pub fn byte_at(text: &str, units: i32) -> usize {
    usize::try_from(units).map_or(0, |units| grind_core::utf16::byte_of(text, units))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_printable_character_starts_an_edit_and_a_control_one_does_not() {
        assert_eq!(
            typed(Mode::Ready, '5', Mods::default()),
            Some(Seed::Char('5'))
        );
        // Through the layout and the IME, which is the whole reason this takes a character
        // rather than a virtual-key code.
        assert_eq!(
            typed(Mode::Ready, 'ä', Mods::default()),
            Some(Seed::Char('ä'))
        );
        // `TranslateMessage` produces a `WM_CHAR` for Enter and Escape as well.
        assert_eq!(typed(Mode::Ready, '\r', Mods::default()), None);
        assert_eq!(typed(Mode::Ready, '\u{1b}', Mods::default()), None);
        // A shortcut is not a value: Ctrl+S produces `\u{13}` and must not open an editor.
        assert_eq!(typed(Mode::Ready, 's', ctrl()), None);
        // And once an edit is open the control has the keys.
        assert_eq!(typed(Mode::Enter, '5', Mods::default()), None);
    }

    #[test]
    fn f2_opens_the_cell_and_typing_replaces_it() {
        assert_eq!(
            on_key(Mode::Ready, Key::F2, Mods::default()),
            Outcome::Begin(Seed::Cell)
        );
        assert_eq!(Seed::Cell.mode(), Mode::Edit);
        assert_eq!(Seed::Char('x').mode(), Mode::Enter);
        // And while editing it swaps the two, which is what makes an arrow change meaning.
        assert_eq!(
            on_key(Mode::Enter, Key::F2, Mods::default()),
            Outcome::ToggleMode
        );
        assert_eq!(Mode::Enter.toggled(), Mode::Edit);
        assert_eq!(Mode::Edit.toggled(), Mode::Enter);
    }

    /// The spelling keys reach the grid as well as the text pane: F7 the next misspelt word,
    /// Shift+F7 spelling off and on (`doc/spelling.md`, "In a spreadsheet").
    #[test]
    fn f7_and_shift_f7_are_the_spelling_verbs_over_the_grid() {
        assert_eq!(
            on_key(Mode::Ready, Key::F7, Mods::default()),
            Outcome::Do(Command::NextMisspelling)
        );
        assert_eq!(
            on_key(Mode::Ready, Key::F7, shift()),
            Outcome::Do(Command::ToggleSpelling)
        );
    }

    #[test]
    fn delete_and_backspace_empty_the_selection() {
        for key in [Key::Delete, Key::Backspace] {
            assert_eq!(
                on_key(Mode::Ready, key, Mods::default()),
                Outcome::Do(Command::ClearCells),
                "{key:?}"
            );
        }
        // Not while editing: there the key belongs to the text.
        assert_eq!(
            on_key(Mode::Edit, Key::Delete, Mods::default()),
            Outcome::Passthrough
        );
    }

    /// The reason the two editing modes exist, as one assertion.
    #[test]
    fn an_arrow_commits_in_enter_mode_and_moves_the_caret_in_edit_mode() {
        assert_eq!(
            on_key(Mode::Enter, Key::Right, Mods::default()),
            Outcome::Commit(Some(Dir::Right))
        );
        assert_eq!(
            on_key(Mode::Edit, Key::Right, Mods::default()),
            Outcome::Passthrough
        );
        // Shift+arrow in an editor selects text, in either mode — committing there would
        // throw away a selection the user was making.
        assert_eq!(
            on_key(Mode::Enter, Key::Right, shift()),
            Outcome::Passthrough
        );
    }

    #[test]
    fn return_and_tab_commit_and_shift_reverses_them() {
        assert_eq!(
            on_key(Mode::Edit, Key::Return, Mods::default()),
            Outcome::Commit(Some(Dir::Down))
        );
        assert_eq!(
            on_key(Mode::Edit, Key::Return, shift()),
            Outcome::Commit(Some(Dir::Up))
        );
        assert_eq!(
            on_key(Mode::Enter, Key::Tab, Mods::default()),
            Outcome::Commit(Some(Dir::Right))
        );
        assert_eq!(
            on_key(Mode::Enter, Key::Tab, shift()),
            Outcome::Commit(Some(Dir::Left))
        );
        assert_eq!(
            on_key(Mode::Enter, Key::Escape, Mods::default()),
            Outcome::Cancel
        );
    }

    /// A verb has to win over navigation, or Ctrl+PageDown pages instead of changing sheet.
    #[test]
    fn a_shortcut_is_a_verb_before_it_is_a_motion() {
        assert_eq!(
            on_key(Mode::Ready, Key::Char('S'), ctrl()),
            Outcome::Do(Command::Save)
        );
        assert_eq!(
            on_key(Mode::Ready, Key::PageDown, ctrl()),
            Outcome::Do(Command::SheetNext)
        );
        assert_eq!(
            on_key(Mode::Ready, Key::F9, Mods::default()),
            Outcome::Do(Command::Recalculate)
        );
        // Without Ctrl it is still a motion.
        assert!(matches!(
            on_key(Mode::Ready, Key::PageDown, Mods::default()),
            Outcome::Navigate(keymap::Action::Move { .. })
        ));
    }

    /// While editing, a shortcut belongs to the editor rather than to the window. Saving in the
    /// middle of a half-typed formula would store the *old* cell and look like a lost keystroke.
    #[test]
    fn editing_keeps_the_keys_the_editor_needs() {
        for key in [Key::Char('S'), Key::Char('A'), Key::Char('Z')] {
            assert_eq!(
                on_key(Mode::Edit, key, ctrl()),
                Outcome::Passthrough,
                "{key:?}"
            );
        }
        assert_eq!(
            on_key(Mode::Edit, Key::Home, Mods::default()),
            Outcome::Passthrough
        );
    }

    /// Ready mode hands back what it does not own, or Alt+F4 stops working.
    #[test]
    fn ready_mode_still_lets_go_of_what_it_does_not_own() {
        assert_eq!(
            on_key(Mode::Ready, Key::Other, Mods::default()),
            Outcome::Passthrough
        );
        assert_eq!(
            on_key(Mode::Ready, Key::Escape, Mods::default()),
            Outcome::Passthrough
        );
    }

    /// The conversion itself is `grind_core::utf16`'s and tested there; what is this shell's is
    /// the `i32` either side of it, which is what `EM_GETSEL` and `EM_SETSEL` carry.
    #[test]
    fn a_caret_offset_is_converted_in_the_controls_own_integers() {
        // `ä` is two bytes and one unit.
        assert_eq!(caret_at("=ä+1", 3), 2);
        assert_eq!(byte_at("=ä+1", 2), 3);
        // Nonsense from a control that has no selection, and past the end.
        assert_eq!(byte_at("=A1", -1), 0);
        assert_eq!(byte_at("=A1", 99), 3);
        assert_eq!(caret_at("=A1", 99), 3);
    }
}
