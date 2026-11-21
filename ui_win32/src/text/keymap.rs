// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what in the text pane — a pure function, **no Windows types** and no second
//! virtual-key table: [`crate::sheet::keymap::key_for`] makes the [`Key`], and this starts there.
//! Where a motion lands, and what an edit does to the selection, is `grind_text::caret::Editor`'s.

use grind_text::caret::Motion;

use crate::sheet::keymap::{Key, Mods};

/// What a keystroke asked the pane to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move {
        motion: Motion,
        extend: bool,
    },
    SelectAll,
    /// Backspace (`forward` false) and Delete. With a selection up, both erase that instead —
    /// which is the pane's decision rather than this table's, because it needs the selection.
    Erase {
        forward: bool,
    },
    /// Enter: one block becomes two.
    Split,
    /// Ctrl+Enter: the same, the second starting a page.
    PageBreak,
    /// Tab or Shift+Tab: structural where `grind_text::indent_kind` says there is structure to
    /// change — nesting, un-nesting, or starting a list at a block's front — and a literal
    /// `text:tab` otherwise.
    Tab {
        back: bool,
    },
}

/// What a key means, or `None` for one this pane does not claim, which goes back to
/// `DefWindowProc` so Alt+F4, the system menu and Windows' accelerators work.
pub fn action_for(key: Key, mods: Mods) -> Option<Action> {
    if mods.alt {
        return None;
    }
    let extend = mods.shift;
    let motion = |motion| Some(Action::Move { motion, extend });
    match (key, mods.ctrl) {
        (Key::Left, false) => motion(Motion::Char(-1)),
        (Key::Right, false) => motion(Motion::Char(1)),
        (Key::Left, true) => motion(Motion::WordStart(-1)),
        (Key::Right, true) => motion(Motion::WordStart(1)),
        (Key::Up, _) => motion(Motion::Line(-1)),
        (Key::Down, _) => motion(Motion::Line(1)),
        (Key::PageUp, _) => motion(Motion::Page(-1)),
        (Key::PageDown, _) => motion(Motion::Page(1)),
        // Ctrl+Home and Ctrl+End are the document's ends everywhere on this platform; without
        // Ctrl they are the line's, which on a wrapped line is the *visual* line.
        (Key::Home, true) => motion(Motion::DocStart),
        (Key::End, true) => motion(Motion::DocEnd),
        (Key::Home, false) => motion(Motion::LineStart),
        (Key::End, false) => motion(Motion::LineEnd),
        (Key::Char('A'), true) => Some(Action::SelectAll),
        (Key::Backspace, false) => Some(Action::Erase { forward: false }),
        (Key::Delete, false) => Some(Action::Erase { forward: true }),
        (Key::Return, false) => Some(Action::Split),
        (Key::Return, true) => Some(Action::PageBreak),
        (Key::Tab, false) => Some(Action::Tab { back: mods.shift }),
        _ => None,
    }
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

    #[test]
    fn the_arrows_move_and_shift_extends() {
        let plain = action_for(Key::Right, mods(false, false));
        assert_eq!(
            plain,
            Some(Action::Move {
                motion: Motion::Char(1),
                extend: false
            })
        );
        let held = action_for(Key::Down, mods(false, true));
        assert_eq!(
            held,
            Some(Action::Move {
                motion: Motion::Line(1),
                extend: true
            })
        );
    }

    #[test]
    fn control_turns_a_character_into_a_word_and_a_line_into_the_document() {
        assert_eq!(
            action_for(Key::Left, mods(true, false)),
            Some(Action::Move {
                motion: Motion::WordStart(-1),
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Home, mods(true, false)),
            Some(Action::Move {
                motion: Motion::DocStart,
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Home, mods(false, false)),
            Some(Action::Move {
                motion: Motion::LineStart,
                extend: false
            })
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
    }

    /// Tab and Shift+Tab are the same key, told apart only by `back` — `win.rs`'s `text_indent`
    /// decides whether that means nesting a list or typing a literal tab.
    #[test]
    fn tab_and_shift_tab_carry_their_own_direction() {
        assert_eq!(
            action_for(Key::Tab, mods(false, false)),
            Some(Action::Tab { back: false })
        );
        assert_eq!(
            action_for(Key::Tab, mods(false, true)),
            Some(Action::Tab { back: true })
        );
    }
}
