// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what in the document pane, as a pure function — [`Chord`] in, [`Action`] out,
//! no `web_sys` — tested on the host. **No arithmetic here**: Down means the next *line*, and the
//! answer is `grind_text::App::caret_line`'s (`doc/text-layout.md`).
//!
//! **A composed character does not come from a key.** A dead key (`` ` ``, `´`, `^`, `~` on many
//! layouts) reports `KeyboardEvent.key == "Dead"`, and every keystroke inside its composition
//! reports `isComposing`; neither carries the final character, which arrives on `compositionend`.
//! So both are refused here and the text comes from that event ([`Chord::composing`], `wire` in
//! `super`). `beforeinput` would need an editable host, which this shell deliberately lacks.

use grind_text::caret::{Intent, Key, intent_for};

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
    /// `KeyboardEvent.isComposing`: this keystroke is part of a composition the browser is
    /// running, and its `key` is therefore about the *composition* rather than about the
    /// document. Nothing is claimed while it is set — see the module documentation.
    pub composing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action<'a> {
    /// A key the core's table claims ([`intent_for`]): a motion, an erase, Enter, Tab.
    Edit(Intent),
    /// A character was typed — borrowed from the event (`KeyboardEvent.key`), and a string, since
    /// a composed `é` arrives whole, from `compositionend`. An input method's candidate window
    /// does not arrive (`doc/web-shell.md`).
    Type(&'a str),
    /// A command, by the id [`crate::command::TEXT`] names it with. Every modifier chord
    /// this shell claims is one of these, so a key and a palette row run the same code.
    Run(&'static str),
}

#[must_use]
pub fn action_for<'a>(chord: &Chord<'a>) -> Option<Action<'a>> {
    // A composition owns the keyboard until it ends: the dead key that opens one (no character
    // yet) and every keystroke inside one (whose `key` differs by browser, dropping `` ` `` on one
    // and doubling `è` on another) are refused. `compositionend` carries the result exactly once.
    if chord.composing || chord.key == "Dead" {
        return None;
    }
    // The chrome's shortcuts first: a document that swallowed Ctrl+S would type an `s`.
    let command = if chord.primary {
        match chord.key {
            "z" | "Z" if chord.shift => Some(Action::Run("doc.redo")),
            "z" | "Z" => Some(Action::Run("doc.undo")),
            "y" | "Y" => Some(Action::Run("doc.redo")),
            "s" | "S" => Some(Action::Run("doc.save")),
            "o" | "O" => Some(Action::Run("doc.open")),
            "a" | "A" => Some(Action::Run("edit.select-all")),
            // Replace's key in every other client. Ctrl+F is not here: it opens the palette
            // (`wire_palette`), where typing a word lists the places that hold it.
            "h" | "H" => Some(Action::Run("edit.replace")),
            // The browser's own print would print this page — the editor's chrome and all —
            // rather than the document; ours prints the document's PDF (`doc/pdf-export.md`).
            "p" | "P" => Some(Action::Run("doc.print-pdf")),
            "e" | "E" if chord.shift => Some(Action::Run("doc.export-pdf")),
            "b" | "B" => Some(Action::Run("char.bold")),
            "i" | "I" => Some(Action::Run("char.italic")),
            "u" | "U" => Some(Action::Run("char.underline")),
            "0" => Some(Action::Run("block.body")),
            "1" => Some(Action::Run("block.h1")),
            "2" => Some(Action::Run("block.h2")),
            "3" => Some(Action::Run("block.h3")),
            _ => None,
        }
    } else {
        match chord.key {
            "F3" if chord.shift => Some(Action::Run("edit.find-previous")),
            "F3" => Some(Action::Run("edit.find-next")),
            _ => None,
        }
    };
    if command.is_some() {
        return command;
    }
    // The keys every text view shares. A tab is a character in this model (`text:tab`) — except
    // in a list, where it is what nests one; the alternative is a key that moves focus out of the
    // document you are writing in.
    if let Some(key) = named(chord.key) {
        return intent_for(key, chord.primary, chord.shift).map(Action::Edit);
    }
    // Exactly one character is a typed character; anything longer is a named key this shell does
    // not claim — "F5", "Escape", "Shift", and "Dead", which the guard at the top has already
    // turned away. Ctrl+T and F5 belong to the browser — and so do Ctrl+C, Ctrl+X and Ctrl+V,
    // whose *events* are how this shell reaches the clipboard at all.
    let key = chord.key;
    (!chord.primary && key.chars().count() == 1).then_some(Action::Type(key))
}

/// `KeyboardEvent.key`'s name for a key a text view claims.
fn named(key: &str) -> Option<Key> {
    Some(match key {
        "ArrowLeft" => Key::Left,
        "ArrowRight" => Key::Right,
        "ArrowUp" => Key::Up,
        "ArrowDown" => Key::Down,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Enter" => Key::Return,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(key: &str) -> Option<Action<'_>> {
        action_for(&Chord {
            key,
            primary: false,
            shift: false,
            composing: false,
        })
    }

    fn ctrl(key: &str) -> Option<Action<'_>> {
        action_for(&Chord {
            key,
            primary: true,
            shift: false,
            composing: false,
        })
    }

    fn shifted(key: &str) -> Option<Action<'_>> {
        action_for(&Chord {
            key,
            primary: false,
            shift: true,
            composing: false,
        })
    }

    fn composing(key: &str) -> Option<Action<'_>> {
        action_for(&Chord {
            key,
            primary: false,
            shift: false,
            composing: true,
        })
    }

    #[test]
    fn find_replace_and_the_two_steps_are_commands() {
        assert_eq!(plain("F3"), Some(Action::Run("edit.find-next")));
        assert_eq!(shifted("F3"), Some(Action::Run("edit.find-previous")));
        assert_eq!(ctrl("h"), Some(Action::Run("edit.replace")));
    }

    /// The named keys are the core's table, Ctrl being the browser's primary modifier.
    #[test]
    fn a_named_key_is_the_cores_intent() {
        assert_eq!(
            plain("ArrowDown"),
            intent_for(Key::Down, false, false).map(Action::Edit)
        );
        assert_eq!(
            ctrl("ArrowLeft"),
            intent_for(Key::Left, true, false).map(Action::Edit)
        );
        assert_eq!(
            shifted("Tab"),
            intent_for(Key::Tab, false, true).map(Action::Edit)
        );
        assert_eq!(ctrl("Enter"), Some(Action::Edit(Intent::PageBreak)));
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
        assert_eq!(
            plain("é"),
            Some(Action::Type("é")),
            "a browser that hands the composed character straight to a key is believed"
        );
        assert_eq!(plain("\u{4e16}"), Some(Action::Type("\u{4e16}")));
        assert_eq!(plain("F5"), None);
        assert_eq!(plain("Shift"), None);
        assert_eq!(plain("Escape"), None);
    }

    /// The dead key, which is the whole of `` `code` `` on a German, French or Spanish
    /// layout: neither half of a composition is a keystroke this pane may act on, and the
    /// text comes from `compositionend` instead.
    #[test]
    fn a_composition_is_refused_at_both_ends_so_it_is_typed_once() {
        assert_eq!(plain("Dead"), None, "no character has been decided yet");
        // Whatever the browser puts in `key` while it composes — the base character on one,
        // the composed one on another — it is not this pane's to type.
        for key in ["e", "é", "Dead", " "] {
            assert_eq!(composing(key), None, "{key}");
        }
        // Including the chords, so a composition cannot trip Ctrl+S on its way past.
        assert_eq!(
            action_for(&Chord {
                key: "s",
                primary: true,
                shift: false,
                composing: true,
            }),
            None
        );
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
                shift: true,
                composing: false,
            }),
            Some(Action::Run("doc.redo"))
        );
        assert_eq!(
            ctrl("p"),
            Some(Action::Run("doc.print-pdf")),
            "print is ours, not the tab's"
        );
        assert_eq!(
            action_for(&Chord {
                key: "E",
                primary: true,
                shift: true,
                composing: false,
            }),
            Some(Action::Run("doc.export-pdf"))
        );
        assert_eq!(ctrl("t"), None, "and nothing else is claimed");
    }
}
