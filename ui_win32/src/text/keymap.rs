// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what in the text pane — a pure function, **no Windows types**:
//! [`crate::sheet::keymap::key_for`] makes the [`Key`], `grind_text::caret::intent_for` says what
//! it means, and this keeps what is Windows' own: Alt is the menu bar's, Ctrl+A selects everything,
//! and a word ends where the next one starts.

use grind_text::caret::{self, Intent, Motion};

use crate::sheet::keymap::{Key, Mods};

/// What a keystroke asked the pane to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Edit(Intent),
    SelectAll,
}

/// What a key means, or `None` for one this pane does not claim, which goes back to
/// `DefWindowProc` so Alt+F4, the system menu and Windows' accelerators work.
pub fn action_for(key: Key, mods: Mods) -> Option<Action> {
    if mods.alt {
        return None;
    }
    if key == Key::Char('A') && mods.ctrl {
        return Some(Action::SelectAll);
    }
    let key = match key {
        Key::Left => caret::Key::Left,
        Key::Right => caret::Key::Right,
        Key::Up => caret::Key::Up,
        Key::Down => caret::Key::Down,
        Key::Home => caret::Key::Home,
        Key::End => caret::Key::End,
        Key::PageUp => caret::Key::PageUp,
        Key::PageDown => caret::Key::PageDown,
        Key::Return => caret::Key::Return,
        Key::Tab => caret::Key::Tab,
        Key::Backspace => caret::Key::Backspace,
        Key::Delete => caret::Key::Delete,
        _ => return None,
    };
    let intent = match caret::intent_for(key, mods.ctrl, mods.shift)? {
        // Ctrl+Right stops at the start of the next word on this platform.
        Intent::Move {
            motion: Motion::Word(step),
            extend,
        } => Intent::Move {
            motion: Motion::WordStart(step),
            extend,
        },
        intent => intent,
    };
    Some(Action::Edit(intent))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, shift: bool) -> Mods {
        Mods {
            ctrl,
            shift,
            alt: false,
        }
    }

    /// Ctrl+Left and Ctrl+Right move as Windows does, to a word's start either way.
    #[test]
    fn a_word_is_where_the_next_one_starts() {
        assert_eq!(
            action_for(Key::Right, mods(true, true)),
            Some(Action::Edit(Intent::Move {
                motion: Motion::WordStart(1),
                extend: true
            }))
        );
    }

    /// Alt belongs to Windows: it opens the menu bar, and a pane that claimed Alt+F would take
    /// the File menu away from every keyboard user.
    #[test]
    fn alt_is_never_claimed() {
        for key in [Key::Left, Key::Home, Key::Return, Key::Char('A')] {
            assert_eq!(
                action_for(
                    key,
                    Mods {
                        alt: true,
                        ..mods(false, false)
                    },
                ),
                None
            );
        }
    }

    #[test]
    fn a_key_this_pane_does_not_own_goes_back_to_windows() {
        assert_eq!(action_for(Key::F5, mods(false, false)), None);
        assert_eq!(action_for(Key::Other, mods(false, false)), None);
        // Ctrl+C is the clipboard's, and the clipboard's verbs arrive as menu commands.
        assert_eq!(action_for(Key::Char('C'), mods(true, false)), None);
        assert_eq!(
            action_for(Key::Char('A'), mods(true, false)),
            Some(Action::SelectAll)
        );
    }
}
