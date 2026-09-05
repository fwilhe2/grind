// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a keystroke means *while editing* — the modes, and the one function that decides.
//!
//! **Portable, and tested on any host**, like [`super::keymap`] and for the same reason: this is
//! the part of a spreadsheet everyone has an opinion about, everyone notices when it is wrong,
//! and no window is needed to check. `win.rs` turns the answer into Win32 calls.
//!
//! The modes are Excel's, by name, and `ui_sheet_gtk/src/state.rs` has the same three:
//!
//! * **Ready** — the grid has the keys. A printable character starts an edit seeded with it, F2
//!   and a double-click start one seeded with the cell, Delete empties the selection, everything
//!   else navigates or is a verb.
//! * **Enter** — an edit that began by typing. An arrow key *commits and moves*, because the
//!   caret has nowhere useful to go in text the user has only just started.
//! * **Edit** — an edit that began from the cell's own content (F2, double-click, or clicking the
//!   formula bar). An arrow key moves the caret instead, because the text is worth navigating.
//!   F2 toggles between the two.
//!
//! ## Two messages, not one
//!
//! The GTK version takes a single `Key` that already carries a character. Win32 does not have
//! one: `WM_KEYDOWN` reports a *key* — `A` is `0x41` whether or not Shift is down and whatever
//! the keyboard layout says — and `WM_CHAR` reports the character that key produced, after the
//! layout and after the IME. So the machine has two doors: [`on_key`] for the keys that mean
//! something regardless of layout, and [`typed`] for a character that starts an edit. Deciding
//! "is this printable" on a virtual-key code is the bug that split buys, and it is the one that
//! makes an accented character or a Japanese composition unable to start an edit.
//!
//! ## What is deliberately missing
//!
//! **Point mode**, autocomplete and signature hints — `doc/sheet-shell.md`'s M6, the single
//! largest piece of the GTK window, and named in `doc/windows-shell.md` as deferred. That is
//! why [`Outcome`] has no `Point` and why an arrow in Enter mode simply commits: there is no
//! pending reference for it to move. The mode enum is still three variants rather than a
//! boolean, because Enter-versus-Edit is what the arrow keys turn on, and it is also where
//! pointing would attach if it is ever built.

use grind_sheet::formula::display::{self, DisplayError};

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
    /// F2, a double-click, or clicking the formula bar: the cell's own text, ready to be
    /// amended. Edit mode.
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
    /// Not ours. In Ready mode that means back to `DefWindowProc`, so Alt+F4 and the system
    /// menu keep working; while editing it means the child `EDIT` gets the key and does its
    /// own thing with it — the caret, the selection, its own Ctrl+Z.
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
        // An arrow commits and moves in Enter mode and moves the caret in Edit mode. That
        // difference *is* the reason there are two modes: someone who typed `12` and pressed →
        // meant the next cell, and someone who pressed F2 to fix a typo meant the next
        // character.
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

/// A character from `WM_CHAR`: does it start an edit?
///
/// Only in Ready mode, only without Ctrl or Alt, and only when it is something a person can
/// see. Control characters arrive here too — Escape is `\u{1b}` and Enter is `\r`, because
/// `TranslateMessage` produces a `WM_CHAR` for both — and none of them is the start of a value.
pub fn typed(mode: Mode, c: char, mods: Mods) -> Option<Seed> {
    let usable = mode == Mode::Ready && !mods.ctrl && !mods.alt && !c.is_control();
    usable.then_some(Seed::Char(c))
}

/// What the editor holds, as the string [`grind_sheet::App::enter`] takes.
///
/// The one conversion between the two, and the whole difference: a formula is typed in **display
/// syntax** (`=SUM(B2:B4)`) and stored in ODF's (`=SUM([.B2:.B4])`). Everything else is passed
/// through untouched, because the typing rule that decides what `12`, `'12` and `TRUE` mean is
/// the core's and there must not be a second copy of it here.
///
/// A formula that will not parse is an `Err` and **does not commit** — the edit stays open with
/// the caret on the problem, because silently storing `=SUM(B2` as a piece of text is how a
/// spreadsheet loses somebody's work.
pub fn to_store(text: &str) -> Result<String, DisplayError> {
    match text.starts_with('=') {
        true => display::from_display(text),
        false => Ok(text.to_owned()),
    }
}

/// Where a caret goes inside the editor for a byte offset the parser reported.
///
/// `EM_SETSEL` counts **UTF-16 code units** and a `DisplayError` is a byte offset into UTF-8,
/// so this is the conversion between them. Out-of-range offsets land at the end rather than
/// panicking: an error position is a hint about where to look, not an invariant.
pub fn caret_at(text: &str, byte: usize) -> i32 {
    let at = byte.min(text.len());
    let units: usize = text
        .get(..at)
        .unwrap_or(text)
        .chars()
        .map(char::len_utf16)
        .sum();
    i32::try_from(units).unwrap_or(i32::MAX)
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

    /// Ready mode must keep handing back what it does not own, or Alt+F4 stops working — the
    /// failure mode of a window that claims every key it is sent.
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

    #[test]
    fn a_formula_is_converted_and_everything_else_is_passed_through() {
        assert_eq!(to_store("=SUM(B2:B4)").unwrap(), "=SUM([.B2:.B4])");
        assert_eq!(to_store("12").unwrap(), "12");
        assert_eq!(to_store("'=not a formula").unwrap(), "'=not a formula");
        assert_eq!(to_store("").unwrap(), "");
    }

    /// A formula that will not parse comes back as an error with a place to put the caret,
    /// rather than being stored as a string that looks like a formula and is not one.
    #[test]
    fn a_broken_formula_reports_where_it_broke() {
        let error = to_store("=SUM(B2").unwrap_err();
        assert!(error.at <= "=SUM(B2".len(), "{error:?}");
        assert!(!error.message.is_empty());
    }

    /// `EM_SETSEL` counts UTF-16 units and the parser counts UTF-8 bytes; a formula with a
    /// non-ASCII sheet name in it is where the difference shows.
    #[test]
    fn a_caret_offset_is_converted_from_bytes_to_units() {
        assert_eq!(caret_at("=SUM(B2", 5), 5);
        // `ä` is two bytes and one unit.
        assert_eq!(caret_at("=ä+1", 3), 2);
        // Past the end is the end, not a panic.
        assert_eq!(caret_at("=A1", 99), 3);
        assert_eq!(caret_at("", 4), 0);
    }
}
