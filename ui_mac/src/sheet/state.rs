// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a keystroke means **while a cell is being edited** — the modes, and the one function that
//! decides (M4). Portable, and tested on any host.
//!
//! The modes are Excel's, by name, as in `ui_sheet_gtk/src/state.rs` and
//! `ui_win32/src/sheet/state.rs`:
//!
//! * **Ready** — the grid has the keys (`keys.rs`). Text typed there starts an edit seeded with
//!   it; a double-click, ⌃U (Excel for Mac's key) or a click in the formula field starts one
//!   seeded with the cell.
//! * **Enter** — an edit that began by typing. An arrow *commits and moves*, because the caret has
//!   nowhere useful to go in text somebody has only just started.
//! * **Edit** — an edit that began from the cell's own content. An arrow moves the caret, because
//!   the text is worth navigating. ⌃U toggles between the two.
//!
//! **The Mac's input is selectors**, here as everywhere in this shell (decision 7): the cell
//! editor is a text field, and its field editor hands its delegate `insertNewline:`,
//! `insertTab:`, `cancelOperation:` and `moveLeft:` rather than keys. So this machine is fed
//! selector names, and answers what the delegate should do — take the key and act, or leave it
//! to the field editor, which is what makes the caret, the selection, dead keys and every input
//! method in the field the system's own.
//!
//! ponytail: this is the third copy of the edit-mode machine, after the GNOME window's and the
//! Windows pane's. `doc/macos-shell.md`'s M1 asked whether one abstract key type fits all three
//! shells' inputs and found that it does not — a GTK key carries its character, Windows splits
//! `WM_KEYDOWN` from `WM_CHAR` and asks its accelerators first, and this one is selectors — so
//! each is mirrored, and what is not about keys is shared: `display::to_input` is what a commit
//! stores. **The trigger is a fourth copy**, or two of them answering one keystroke differently
//! for a reason that is not their input model.

use grind_sheet::nav::Dir;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Ready,
    Enter,
    Edit,
}

impl Mode {
    /// What ⌃U does to a mode that is already editing.
    pub fn toggled(self) -> Self {
        match self {
            Mode::Enter => Mode::Edit,
            Mode::Edit | Mode::Ready => Mode::Enter,
        }
    }
}

/// What an edit starts with, and therefore which mode it starts in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seed {
    /// Typing over a cell replaces it — the text is the whole new content. Enter mode. A string
    /// rather than a character: an input method commits a word at a time.
    Text(String),
    /// A double-click, ⌃U or the formula field: the cell's own text, ready to be amended. Edit
    /// mode.
    Cell,
}

impl Seed {
    pub fn mode(&self) -> Mode {
        match self {
            Seed::Text(_) => Mode::Enter,
            Seed::Cell => Mode::Edit,
        }
    }
}

/// What the cell editor's delegate does about a selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not ours: the field editor moves its caret, selects, deletes a word — its own business.
    Passthrough,
    /// Store what the editor holds, then move that way (or stay, for `None`).
    Commit(Option<Dir>),
    /// Throw the edit away. The document is not touched.
    Cancel,
    /// ⌃U while editing: Enter ↔ Edit.
    ToggleMode,
}

/// The selector ⌃U sends — `NSStandardKeyBindingResponding` binds ⌃U to nothing, so a view sees
/// it as a key equivalent and not as a selector; the cell editor's own key handling turns it into
/// this name so one table decides everything.
pub const TOGGLE: &str = "grindToggleEditMode:";

/// The whole state machine while editing. One `match`, and no memory beyond the mode.
pub fn editing(mode: Mode, selector: &str) -> Outcome {
    match (selector, mode) {
        ("cancelOperation:", _) => Outcome::Cancel,
        (TOGGLE, _) => Outcome::ToggleMode,
        ("insertNewline:", _) => Outcome::Commit(Some(Dir::Down)),
        // ⇧Return, in the bindings Excel for Mac reads: up.
        ("insertNewlineIgnoringFieldEditor:", _) => Outcome::Commit(Some(Dir::Up)),
        ("insertTab:", _) => Outcome::Commit(Some(Dir::Right)),
        ("insertBacktab:", _) => Outcome::Commit(Some(Dir::Left)),
        // An arrow commits and moves in Enter mode and moves the caret in Edit mode. That
        // difference *is* the reason there are two modes: somebody who typed `12` and pressed →
        // meant the next cell, and somebody amending a typo meant the next character.
        ("moveLeft:", Mode::Enter) => Outcome::Commit(Some(Dir::Left)),
        ("moveRight:", Mode::Enter) => Outcome::Commit(Some(Dir::Right)),
        ("moveUp:", Mode::Enter) => Outcome::Commit(Some(Dir::Up)),
        ("moveDown:", Mode::Enter) => Outcome::Commit(Some(Dir::Down)),
        // Everything else belongs to the field editor: its caret, its selection, its own words.
        _ => Outcome::Passthrough,
    }
}

/// Text arriving at the grid (`insertText:`): does it start an edit? Only in Ready mode, and only
/// when it is something a person can see — a control character is not the start of a value.
pub fn typed(mode: Mode, text: &str) -> Option<Seed> {
    let usable = mode == Mode::Ready && !text.is_empty() && !text.chars().any(char::is_control);
    usable.then(|| Seed::Text(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_starts_an_edit_in_enter_mode_and_a_cell_edit_is_edit_mode() {
        let seed = typed(Mode::Ready, "1").unwrap();
        assert_eq!(seed, Seed::Text("1".into()));
        assert_eq!(seed.mode(), Mode::Enter);
        assert_eq!(Seed::Cell.mode(), Mode::Edit);
        assert_eq!(
            typed(Mode::Enter, "1"),
            None,
            "already editing: the field has it"
        );
        assert_eq!(typed(Mode::Ready, "\u{1b}"), None, "Esc is not a value");
        assert_eq!(typed(Mode::Ready, ""), None);
        assert_eq!(
            typed(Mode::Ready, "日本"),
            Some(Seed::Text("日本".into())),
            "an input method commits a word"
        );
    }

    #[test]
    fn return_and_tab_commit_onward_and_esc_throws_away() {
        for mode in [Mode::Enter, Mode::Edit] {
            assert_eq!(
                editing(mode, "insertNewline:"),
                Outcome::Commit(Some(Dir::Down))
            );
            assert_eq!(
                editing(mode, "insertNewlineIgnoringFieldEditor:"),
                Outcome::Commit(Some(Dir::Up))
            );
            assert_eq!(
                editing(mode, "insertTab:"),
                Outcome::Commit(Some(Dir::Right))
            );
            assert_eq!(
                editing(mode, "insertBacktab:"),
                Outcome::Commit(Some(Dir::Left))
            );
            assert_eq!(editing(mode, "cancelOperation:"), Outcome::Cancel);
            assert_eq!(editing(mode, TOGGLE), Outcome::ToggleMode);
        }
    }

    /// The one difference between the two editing modes.
    #[test]
    fn an_arrow_commits_in_enter_mode_and_moves_the_caret_in_edit_mode() {
        assert_eq!(
            editing(Mode::Enter, "moveRight:"),
            Outcome::Commit(Some(Dir::Right))
        );
        assert_eq!(editing(Mode::Edit, "moveRight:"), Outcome::Passthrough);
        assert_eq!(
            editing(Mode::Enter, "moveUp:"),
            Outcome::Commit(Some(Dir::Up))
        );
        assert_eq!(editing(Mode::Edit, "moveUp:"), Outcome::Passthrough);
        // Shift-arrows and the rest are the field's in both.
        assert_eq!(
            editing(Mode::Enter, "moveRightAndModifySelection:"),
            Outcome::Passthrough
        );
        assert_eq!(
            editing(Mode::Enter, "deleteBackward:"),
            Outcome::Passthrough
        );
    }

    #[test]
    fn the_toggle_swaps_the_two_editing_modes() {
        assert_eq!(Mode::Enter.toggled(), Mode::Edit);
        assert_eq!(Mode::Edit.toggled(), Mode::Enter);
    }
}
