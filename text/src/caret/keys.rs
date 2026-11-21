// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a key does to a text view on a desktop that follows the common keyboard conventions —
//! GNOME, Windows and the browser alike — one table, so no window can drift from the others. A
//! shell turns its toolkit's key into a [`Key`] and keeps for itself only what is its own: the
//! menu chords, Select All where the toolkit does not send it as a command, typed characters.

use super::{Motion, Unit};

/// A key, as a text view cares about it.
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
    Tab,
    Backspace,
    Delete,
}

/// What a keystroke asks of an [`super::Editor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Move the caret; `extend` (Shift) grows the selection from where it was.
    Move { motion: Motion, extend: bool },
    /// Backspace and Delete: the selection, or as far as `unit` goes that way.
    Erase { unit: Unit, forward: bool },
    /// Enter: one block becomes two.
    Split,
    /// Ctrl+Enter: the same, the second starting a page.
    PageBreak,
    /// Tab and Shift+Tab: a list nested or un-nested, or a tab typed.
    Tab { back: bool },
}

/// What `key` means with Ctrl and Shift held or not. Ctrl makes the horizontal keys and the
/// erasers work by word and Home and End the document's; Shift extends a motion. `None` for Ctrl+Tab,
/// which is the window's.
#[must_use]
pub fn intent_for(key: Key, ctrl: bool, shift: bool) -> Option<Intent> {
    let go = |motion| {
        Some(Intent::Move {
            motion,
            extend: shift,
        })
    };
    let erase = |forward| {
        Some(Intent::Erase {
            unit: pick(ctrl, Unit::Char, Unit::Word),
            forward,
        })
    };
    match key {
        Key::Left => go(pick(ctrl, Motion::Char(-1), Motion::Word(-1))),
        Key::Right => go(pick(ctrl, Motion::Char(1), Motion::Word(1))),
        Key::Up => go(Motion::Line(-1)),
        Key::Down => go(Motion::Line(1)),
        Key::PageUp => go(Motion::Page(-1)),
        Key::PageDown => go(Motion::Page(1)),
        Key::Home => go(pick(ctrl, Motion::LineStart, Motion::DocStart)),
        Key::End => go(pick(ctrl, Motion::LineEnd, Motion::DocEnd)),
        Key::Return => Some(pick(ctrl, Intent::Split, Intent::PageBreak)),
        Key::Tab => (!ctrl).then_some(Intent::Tab { back: shift }),
        Key::Backspace => erase(false),
        Key::Delete => erase(true),
    }
}

/// The first without Ctrl, the second with it.
fn pick<T>(ctrl: bool, plain: T, held: T) -> T {
    if ctrl { held } else { plain }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(key: Key, ctrl: bool) -> Option<Motion> {
        match intent_for(key, ctrl, false) {
            Some(Intent::Move { motion, .. }) => Some(motion),
            _ => None,
        }
    }

    /// The difference from a grid: vertical movement is by line and horizontal by character.
    #[test]
    fn vertical_is_lines_and_horizontal_is_characters() {
        assert_eq!(moved(Key::Down, false), Some(Motion::Line(1)));
        assert_eq!(moved(Key::Right, false), Some(Motion::Char(1)));
        assert_eq!(moved(Key::PageUp, false), Some(Motion::Page(-1)));
    }

    #[test]
    fn ctrl_makes_words_and_the_document() {
        assert_eq!(moved(Key::Left, true), Some(Motion::Word(-1)));
        assert_eq!(moved(Key::Right, true), Some(Motion::Word(1)));
        assert_eq!(moved(Key::Home, false), Some(Motion::LineStart));
        assert_eq!(moved(Key::Home, true), Some(Motion::DocStart));
        assert_eq!(moved(Key::End, true), Some(Motion::DocEnd));
        assert_eq!(
            intent_for(Key::Backspace, true, false),
            Some(Intent::Erase {
                unit: Unit::Word,
                forward: false
            })
        );
    }

    #[test]
    fn shift_extends_and_the_editing_keys_are_a_carets() {
        assert_eq!(
            intent_for(Key::Down, false, true),
            Some(Intent::Move {
                motion: Motion::Line(1),
                extend: true
            })
        );
        assert_eq!(intent_for(Key::Return, false, false), Some(Intent::Split));
        assert_eq!(
            intent_for(Key::Return, true, false),
            Some(Intent::PageBreak)
        );
        assert_eq!(
            intent_for(Key::Delete, false, false),
            Some(Intent::Erase {
                unit: Unit::Char,
                forward: true
            })
        );
        assert_eq!(
            intent_for(Key::Tab, false, true),
            Some(Intent::Tab { back: true })
        );
        assert_eq!(intent_for(Key::Tab, true, false), None, "the window's");
    }
}
