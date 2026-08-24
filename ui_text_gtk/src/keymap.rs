// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what, as a pure function. **No GTK types.**
//!
//! The widget turns a `gdk::Key` into [`Key`] and hands it over, so every navigation and
//! editing decision unit-tests with no display — the rule `ui_sheet_gtk/src/keymap.rs` follows and
//! the reason both shells' keyboards are testable at all.
//!
//! **Nothing here does arithmetic**, which is the difference from the spreadsheet's map and
//! the whole point of `doc/text-layout.md`: Down-arrow means "the next *line*", a line is an
//! output of layout, and the answer comes from `grind_text::App::caret_line`. This file names
//! the question. `ui_tui/src/text/keymap.rs` names the same ones in vi's spelling — two
//! keyboards over one editing model, which is what stops the shells from disagreeing.
//!
//! Typed characters are **not** here. They arrive through the input method rather than as
//! keys, so that dead keys, compose sequences and IME candidate windows work; a shell that
//! turns keyvals into text has quietly decided only ASCII exists.

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
    Return,
    Backspace,
    Delete,
    /// Anything else, including every printable character — see the module note.
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

/// Where a key wants the caret to go, in *document* terms rather than screen ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// One character, crossing into the neighbouring block at either end.
    Char(i32),
    /// Whole lines. Answered by the core, in the shell's own units.
    Line(i32),
    /// One screenful of lines, however many that turns out to be — the widget knows its
    /// height, this file does not.
    Page(i32),
    /// The visual line's ends, which on a wrapped paragraph are not the paragraph's.
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

/// What a keystroke asks for. `None` means the shell does not claim the key, and the event
/// must keep travelling — which is what leaves the toolkit's own bindings working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    /// Backspace — erase the character before the caret, joining onto the block above when
    /// there is none.
    EraseBack,
    /// Delete — erase the character at the caret, joining the next block on at the end.
    EraseForward,
    /// Enter — a new block, split at the caret.
    Split,
}

pub fn action_for(key: Key, mods: Mods) -> Option<Action> {
    let go = |motion| Some(Action::Move(motion));
    match key {
        Key::Left => go(Motion::Char(-1)),
        Key::Right => go(Motion::Char(1)),
        Key::Up => go(Motion::Line(-1)),
        Key::Down => go(Motion::Line(1)),
        Key::PageUp => go(Motion::Page(-1)),
        Key::PageDown => go(Motion::Page(1)),
        Key::Home if mods.ctrl => go(Motion::DocStart),
        Key::End if mods.ctrl => go(Motion::DocEnd),
        Key::Home => go(Motion::LineStart),
        Key::End => go(Motion::LineEnd),
        Key::Return => Some(Action::Split),
        Key::Backspace => Some(Action::EraseBack),
        Key::Delete => Some(Action::EraseForward),
        Key::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Mods = Mods {
        ctrl: false,
        shift: false,
    };
    const CTRL: Mods = Mods {
        ctrl: true,
        shift: false,
    };

    /// The difference from a grid's keymap: vertical movement is a *line* motion and
    /// horizontal is a *character* one. In a spreadsheet both are the same kind of step.
    #[test]
    fn vertical_is_lines_and_horizontal_is_characters() {
        assert_eq!(
            action_for(Key::Down, PLAIN),
            Some(Action::Move(Motion::Line(1)))
        );
        assert_eq!(
            action_for(Key::Right, PLAIN),
            Some(Action::Move(Motion::Char(1)))
        );
    }

    #[test]
    fn home_and_end_are_the_line_until_ctrl_makes_them_the_document() {
        assert_eq!(
            action_for(Key::Home, PLAIN),
            Some(Action::Move(Motion::LineStart))
        );
        assert_eq!(
            action_for(Key::Home, CTRL),
            Some(Action::Move(Motion::DocStart))
        );
        assert_eq!(
            action_for(Key::End, CTRL),
            Some(Action::Move(Motion::DocEnd))
        );
    }

    #[test]
    fn the_editing_keys_are_the_three_a_caret_has() {
        assert_eq!(action_for(Key::Return, PLAIN), Some(Action::Split));
        assert_eq!(action_for(Key::Backspace, PLAIN), Some(Action::EraseBack));
        assert_eq!(action_for(Key::Delete, PLAIN), Some(Action::EraseForward));
    }

    /// A key this shell does not claim has to keep travelling, or the input method never
    /// sees the letter that was typed.
    #[test]
    fn an_unclaimed_key_is_not_claimed() {
        assert_eq!(action_for(Key::Other, PLAIN), None);
        assert_eq!(action_for(Key::Other, CTRL), None);
    }
}
