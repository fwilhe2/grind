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
    /// Move the caret. `extend` is Shift held down: the anchor stays where it is and the
    /// selection grows, which is the same word `ui_sheet_gtk`'s grid uses for the same idea.
    Move {
        motion: Motion,
        extend: bool,
    },
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
    /// Tab, which means two different things depending on what the caret is in — a list item
    /// nests, anything else takes a tab character. The pane decides, because this file has no
    /// document to ask.
    Tab {
        back: bool,
    },
    /// A command, by the id [`crate::command::TEXT`] names it with. Every modifier chord
    /// this shell claims is one of these, so a key and a palette row run the same code.
    Run(&'static str),
}

pub fn action_for<'a>(chord: &Chord<'a>) -> Option<Action<'a>> {
    let go = |motion| {
        Some(Action::Move {
            motion,
            extend: chord.shift,
        })
    };
    // The chrome's shortcuts first: a document that swallowed Ctrl+S would type an `s`.
    if chord.primary {
        return match chord.key {
            "z" | "Z" if chord.shift => Some(Action::Run("doc.redo")),
            "z" | "Z" => Some(Action::Run("doc.undo")),
            "y" | "Y" => Some(Action::Run("doc.redo")),
            "s" | "S" => Some(Action::Run("doc.save")),
            "o" | "O" => Some(Action::Run("doc.open")),
            "a" | "A" => Some(Action::Run("edit.select-all")),
            "b" | "B" => Some(Action::Run("char.bold")),
            "i" | "I" => Some(Action::Run("char.italic")),
            "u" | "U" => Some(Action::Run("char.underline")),
            "0" => Some(Action::Run("block.body")),
            "1" => Some(Action::Run("block.h1")),
            "2" => Some(Action::Run("block.h2")),
            "3" => Some(Action::Run("block.h3")),
            "Home" => go(Motion::DocStart),
            "End" => go(Motion::DocEnd),
            // Ctrl+T and F5 still belong to the browser — and so do Ctrl+C, Ctrl+X and
            // Ctrl+V, whose *events* are how this shell reaches the clipboard at all.
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
        // A tab is a character in this model (`text:tab`) — except in a list, where it is
        // what nests one. The alternative is a key that moves focus out of the document you
        // are writing in.
        "Tab" => Some(Action::Tab { back: chord.shift }),
        // Exactly one character is a typed character; anything longer is a named key this
        // shell does not claim — "F5", "Escape", "Shift".
        //
        // TODO: **and "Dead"**, which is what a dead key reports — `` ` ``, `´`, `^` and `~` on
        // German, French, Spanish and other layouts. Four characters is no character here, so
        // the key is dropped and nothing is ever composed from it: on those keyboards
        // `` `code` `` cannot be typed into this pane at all. The prime suspect in
        // `text/src/markdown.rs`'s TODO. The fix is a composition, not a bigger match arm —
        // `compositionend` carries what the dead key finally produced, and this shell has no
        // input method at all (`doc/web-shell.md` names that gap).
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

    fn shifted(key: &str) -> Option<Action<'_>> {
        action_for(&Chord {
            key,
            primary: false,
            shift: true,
        })
    }

    #[test]
    fn vertical_is_lines_and_horizontal_is_characters() {
        assert_eq!(
            plain("ArrowDown"),
            Some(Action::Move {
                motion: Motion::Line(1),
                extend: false
            })
        );
        assert_eq!(
            plain("ArrowRight"),
            Some(Action::Move {
                motion: Motion::Char(1),
                extend: false
            })
        );
    }

    /// Shift extends rather than moving — every motion, not a special list of them.
    #[test]
    fn shift_extends_whatever_the_motion_is() {
        for key in ["ArrowLeft", "ArrowDown", "Home", "End", "PageUp"] {
            let Some(Action::Move { extend, .. }) = shifted(key) else {
                panic!("{key} is not a motion");
            };
            assert!(extend, "{key}");
        }
    }

    #[test]
    fn the_formatting_chords_are_commands_rather_than_typing() {
        assert_eq!(ctrl("b"), Some(Action::Run("char.bold")));
        assert_eq!(ctrl("i"), Some(Action::Run("char.italic")));
        assert_eq!(ctrl("u"), Some(Action::Run("char.underline")));
        assert_eq!(ctrl("1"), Some(Action::Run("block.h1")));
        assert_eq!(ctrl("0"), Some(Action::Run("block.body")));
        // The clipboard's own three stay with the browser, whose events this shell listens
        // for — claiming them here would stop those events ever firing.
        for key in ["c", "x", "v"] {
            assert_eq!(ctrl(key), None, "{key}");
        }
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
        assert_eq!(ctrl("s"), Some(Action::Run("doc.save")));
        assert_eq!(ctrl("o"), Some(Action::Run("doc.open")));
        assert_eq!(ctrl("z"), Some(Action::Run("doc.undo")));
        assert_eq!(ctrl("y"), Some(Action::Run("doc.redo")));
        assert_eq!(
            action_for(&Chord {
                key: "z",
                primary: true,
                shift: true
            }),
            Some(Action::Run("doc.redo"))
        );
        assert_eq!(ctrl("t"), None, "and nothing else is claimed");
    }

    #[test]
    fn ctrl_home_is_the_document_and_home_alone_is_the_line() {
        let motion = |action| match action {
            Some(Action::Move { motion, .. }) => Some(motion),
            _ => None,
        };
        assert_eq!(motion(plain("Home")), Some(Motion::LineStart));
        assert_eq!(motion(ctrl("Home")), Some(Motion::DocStart));
        assert_eq!(motion(ctrl("End")), Some(Motion::DocEnd));
    }

    #[test]
    fn the_editing_keys_are_the_three_a_caret_has() {
        assert_eq!(plain("Enter"), Some(Action::Split));
        assert_eq!(plain("Backspace"), Some(Action::EraseBack));
        assert_eq!(plain("Delete"), Some(Action::EraseForward));
        assert_eq!(plain("Tab"), Some(Action::Tab { back: false }));
        assert_eq!(shifted("Tab"), Some(Action::Tab { back: true }));
    }
}
