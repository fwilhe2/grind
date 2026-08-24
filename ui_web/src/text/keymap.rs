// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what in the document pane, as a pure function.
//!
//! The same shape as [`crate::sheet::keymap`] — a [`Chord`] in, an [`Action`] out, no
//! `web_sys` anywhere near it — so the whole map unit-tests on the host with no browser
//! (`cargo test -p grind-web`).
//!
//! **Nothing here does arithmetic.** Down means "the next *line*", a line is an output of
//! layout, and the answer comes from `grind_text::App::caret_line` (`doc/text-layout.md`).
//! `ui_tui/src/text/keymap.rs` and `ui_text_gtk/src/keymap.rs` name the same questions in
//! their own spellings; three keyboards, one editing model.

/// A keystroke, in the browser's own vocabulary.
///
/// `key` is `KeyboardEvent.key`: `"a"`, `"Enter"`, `"ArrowDown"` — the *typed* value rather
/// than a physical code, so a Dvorak keyboard and a French one both say what they mean.
#[derive(Clone, Copy, Debug)]
pub struct Chord<'a> {
    pub key: &'a str,
    /// ⌘ on macOS, Ctrl everywhere else, resolved by the caller so this never asks what it
    /// is running on.
    pub primary: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Char(i32),
    Line(i32),
    /// One screenful, in lines — how many is the pane's business, not this file's.
    Page(i32),
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action<'a> {
    Move(Motion),
    /// A character was typed. Borrowed from the event, because it is `KeyboardEvent.key` and
    /// copying it would be a `String` per keystroke.
    ///
    /// A composed character — `é` from a dead key — arrives here whole, which is why this is
    /// a string and not a `char`. What does *not* arrive is an IME composition; see the gap
    /// list in `doc/text-shell.md`.
    Type(&'a str),
    Split,
    EraseBack,
    EraseForward,
    Undo,
    Redo,
    Open,
    Save,
}

pub fn action_for<'a>(chord: &Chord<'a>) -> Option<Action<'a>> {
    let go = |motion| Some(Action::Move(motion));
    // The chrome's shortcuts first: a document that swallowed Ctrl+S would type an `s`.
    if chord.primary {
        return match chord.key {
            "z" | "Z" if chord.shift => Some(Action::Redo),
            "z" | "Z" => Some(Action::Undo),
            "y" | "Y" => Some(Action::Redo),
            "s" | "S" => Some(Action::Save),
            "o" | "O" => Some(Action::Open),
            "Home" => go(Motion::DocStart),
            "End" => go(Motion::DocEnd),
            // Ctrl+T and F5 still belong to the browser.
            _ => None,
        };
    }
    match chord.key {
        "ArrowLeft" => go(Motion::Char(-1)),
        "ArrowRight" => go(Motion::Char(1)),
        "ArrowUp" => go(Motion::Line(-1)),
        "ArrowDown" => go(Motion::Line(1)),
        "PageUp" => go(Motion::Page(-1)),
        "PageDown" => go(Motion::Page(1)),
        "Home" => go(Motion::LineStart),
        "End" => go(Motion::LineEnd),
        "Enter" => Some(Action::Split),
        "Backspace" => Some(Action::EraseBack),
        "Delete" => Some(Action::EraseForward),
        // A tab is a character in this model (`text:tab`), and the alternative is a key that
        // moves focus out of the document you are writing in.
        "Tab" => Some(Action::Type("\t")),
        // Exactly one character is a typed character; anything longer is a named key this
        // shell does not claim — "F5", "Escape", "Shift".
        key if key.chars().count() == 1 => Some(Action::Type(key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(key: &str) -> Option<Action<'_>> {
        action_for(&Chord {
            key,
            primary: false,
            shift: false,
        })
    }

    fn ctrl(key: &str) -> Option<Action<'_>> {
        action_for(&Chord {
            key,
            primary: true,
            shift: false,
        })
    }

    #[test]
    fn vertical_is_lines_and_horizontal_is_characters() {
        assert_eq!(plain("ArrowDown"), Some(Action::Move(Motion::Line(1))));
        assert_eq!(plain("ArrowRight"), Some(Action::Move(Motion::Char(1))));
    }

    /// The rule that decides typing: one character is text, more than one is a named key.
    #[test]
    fn a_single_character_is_typed_and_a_named_key_is_not() {
        assert_eq!(plain("a"), Some(Action::Type("a")));
        assert_eq!(plain(" "), Some(Action::Type(" ")));
        assert_eq!(plain("é"), Some(Action::Type("é")), "a dead key composes");
        assert_eq!(plain("\u{4e16}"), Some(Action::Type("\u{4e16}")));
        assert_eq!(plain("F5"), None);
        assert_eq!(plain("Shift"), None);
        assert_eq!(plain("Escape"), None);
    }

    /// A modifier chord belongs to the chrome, never to the document — otherwise Ctrl+S
    /// types an `s` into the paragraph it was meant to save.
    #[test]
    fn the_chrome_owns_the_modifier_chords() {
        assert_eq!(ctrl("s"), Some(Action::Save));
        assert_eq!(ctrl("o"), Some(Action::Open));
        assert_eq!(ctrl("z"), Some(Action::Undo));
        assert_eq!(ctrl("y"), Some(Action::Redo));
        assert_eq!(
            action_for(&Chord {
                key: "z",
                primary: true,
                shift: true
            }),
            Some(Action::Redo)
        );
        assert_eq!(ctrl("a"), None, "and nothing else is claimed");
    }

    #[test]
    fn ctrl_home_is_the_document_and_home_alone_is_the_line() {
        assert_eq!(plain("Home"), Some(Action::Move(Motion::LineStart)));
        assert_eq!(ctrl("Home"), Some(Action::Move(Motion::DocStart)));
        assert_eq!(ctrl("End"), Some(Action::Move(Motion::DocEnd)));
    }

    #[test]
    fn the_editing_keys_are_the_three_a_caret_has() {
        assert_eq!(plain("Enter"), Some(Action::Split));
        assert_eq!(plain("Backspace"), Some(Action::EraseBack));
        assert_eq!(plain("Delete"), Some(Action::EraseForward));
        assert_eq!(plain("Tab"), Some(Action::Type("\t")));
    }
}
