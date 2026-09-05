// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what in the text pane, and where a word ends — as pure functions.
//!
//! **No Windows types at all**, and not even a virtual-key table: [`crate::sheet::keymap::key_for`]
//! already turns a `WM_KEYDOWN` into a [`Key`], and its constants are pinned against `winuser.h`
//! by a `cfg(windows)` test there. A second table here would be a second thing to get wrong, so
//! this module starts where that one stops — a [`Key`] and the modifiers, in, and what it means
//! to a caret, out.
//!
//! ## The selection is presentation state
//!
//! An anchor and an active caret, and nothing else; the core is never told about it. A range
//! reaches `App` as two [`Caret`]s when something is actually *done* to it — copying it, erasing
//! it, formatting it — which is the same arrangement the grid has and the same one the other
//! three shells have.

use grind_text::Caret;

use crate::sheet::keymap::{Key, Mods};

/// What a motion moves by.
///
/// Every one of these is defined in terms of a **line** except the two that are defined in terms
/// of characters, and that is `doc/text-layout.md`'s whole argument for Path C: Down, Home and End
/// are meaningless without line breaking, and line breaking is `grind_core::layout`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// One character left or right, across a block boundary at either end.
    Char(i8),
    /// One word left or right — Ctrl+Left and Ctrl+Right.
    Word(i8),
    /// N lines down (positive) or up — the arrows, and Page Up/Down with a bigger number.
    Line(isize),
    /// The visual ends of the caret's own line, which on a wrapped line are not the paragraph's.
    LineStart,
    LineEnd,
    /// The first and last caret position in the document — Ctrl+Home and Ctrl+End.
    DocStart,
    DocEnd,
}

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
    /// A literal tab, which is a character in the model (`text:tab`) rather than an indent.
    Tab,
}

/// What a key means, or `None` for one this pane does not claim — which must go back to
/// `DefWindowProc` so that Alt+F4, the system menu and Windows' own accelerators keep working.
///
/// `page` is how many lines a Page Up/Down moves, which the window knows and this does not.
pub fn action_for(key: Key, mods: Mods, page: isize) -> Option<Action> {
    if mods.alt {
        return None;
    }
    let extend = mods.shift;
    let motion = |motion| Some(Action::Move { motion, extend });
    match (key, mods.ctrl) {
        (Key::Left, false) => motion(Motion::Char(-1)),
        (Key::Right, false) => motion(Motion::Char(1)),
        (Key::Left, true) => motion(Motion::Word(-1)),
        (Key::Right, true) => motion(Motion::Word(1)),
        (Key::Up, _) => motion(Motion::Line(-1)),
        (Key::Down, _) => motion(Motion::Line(1)),
        (Key::PageUp, _) => motion(Motion::Line(-page)),
        (Key::PageDown, _) => motion(Motion::Line(page)),
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
        (Key::Tab, false) => Some(Action::Tab),
        _ => None,
    }
}

/// The two ends of a selection, in document order.
///
/// A caret is a block and an offset, so "which is first" is a tuple comparison and not a
/// subtraction — a later block always wins however long the earlier one is.
pub fn ordered(anchor: Caret, active: Caret) -> (Caret, Caret) {
    match (anchor.block, anchor.offset) <= (active.block, active.offset) {
        true => (anchor, active),
        false => (active, anchor),
    }
}

/// Where the word boundary is, `offset` characters into `text`, going one way or the other.
///
/// Windows' own rule, which is not the same as vi's: moving **right** stops at the start of the
/// next word (so repeated Ctrl+Right walks the beginnings), and moving **left** stops at the
/// start of the word the caret is in or just past. Punctuation counts as a word so that walking
/// through `a, b` does not skip the comma.
///
/// Returns `None` at the end of the block in that direction, which is the caller's cue to carry
/// into the neighbouring block — the same shape [`grind_text::App::caret_line`] uses.
pub fn word_boundary(text: &str, offset: usize, forward: bool) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut at = offset.min(chars.len());
    if forward {
        if at >= chars.len() {
            return None;
        }
        // Out of whatever the caret is in the middle of, then across the space after it.
        let kind = class(chars[at]);
        while at < chars.len() && class(chars[at]) == kind {
            at += 1;
        }
        while at < chars.len() && class(chars[at]) == Class::Space {
            at += 1;
        }
        Some(at)
    } else {
        if at == 0 {
            return None;
        }
        while at > 0 && class(chars[at - 1]) == Class::Space {
            at -= 1;
        }
        if at == 0 {
            return Some(0);
        }
        let kind = class(chars[at - 1]);
        while at > 0 && class(chars[at - 1]) == kind {
            at -= 1;
        }
        Some(at)
    }
}

/// What kind of character this is, as far as word motion cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punctuation,
}

fn class(c: char) -> Class {
    match c {
        c if c.is_whitespace() => Class::Space,
        c if c.is_alphanumeric() || c == '_' => Class::Word,
        _ => Class::Punctuation,
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
        let plain = action_for(Key::Right, mods(false, false), 20);
        assert_eq!(
            plain,
            Some(Action::Move {
                motion: Motion::Char(1),
                extend: false
            })
        );
        let held = action_for(Key::Down, mods(false, true), 20);
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
            action_for(Key::Left, mods(true, false), 20),
            Some(Action::Move {
                motion: Motion::Word(-1),
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Home, mods(true, false), 20),
            Some(Action::Move {
                motion: Motion::DocStart,
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Home, mods(false, false), 20),
            Some(Action::Move {
                motion: Motion::LineStart,
                extend: false
            })
        );
    }

    /// A page is the window's answer, not this table's — a taller window pages further.
    #[test]
    fn a_page_is_as_many_lines_as_the_window_holds() {
        assert_eq!(
            action_for(Key::PageDown, mods(false, false), 37),
            Some(Action::Move {
                motion: Motion::Line(37),
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
                    20
                ),
                None
            );
        }
    }

    #[test]
    fn a_key_this_pane_does_not_own_goes_back_to_windows() {
        assert_eq!(action_for(Key::F5, mods(false, false), 20), None);
        assert_eq!(action_for(Key::Other, mods(false, false), 20), None);
        // Ctrl+C is the clipboard's, and the clipboard's verbs arrive as menu commands.
        assert_eq!(action_for(Key::Char('C'), mods(true, false), 20), None);
    }

    #[test]
    fn a_selection_is_ordered_by_block_first() {
        let a = Caret {
            block: 1,
            offset: 90,
        };
        let b = Caret {
            block: 2,
            offset: 0,
        };
        assert_eq!(ordered(a, b), (a, b));
        assert_eq!(ordered(b, a), (a, b), "however long the earlier block is");
    }

    #[test]
    fn moving_right_by_a_word_lands_on_the_next_beginning() {
        let text = "the quick  brown";
        assert_eq!(word_boundary(text, 0, true), Some(4));
        assert_eq!(
            word_boundary(text, 4, true),
            Some(11),
            "two spaces, one step"
        );
        assert_eq!(word_boundary(text, 11, true), Some(16), "up to the end");
        assert_eq!(word_boundary(text, 16, true), None, "and no further");
    }

    #[test]
    fn moving_left_by_a_word_lands_on_the_beginning_behind_the_caret() {
        let text = "the quick  brown";
        assert_eq!(word_boundary(text, 16, false), Some(11));
        assert_eq!(
            word_boundary(text, 11, false),
            Some(4),
            "across both spaces"
        );
        assert_eq!(word_boundary(text, 2, false), Some(0), "inside the first");
        assert_eq!(word_boundary(text, 0, false), None);
    }

    /// Punctuation is a word of its own, so `a, b` has a stop at the comma rather than skipping
    /// it — which is what makes Ctrl+Shift+Right a usable way to select an argument.
    #[test]
    fn punctuation_is_a_word() {
        assert_eq!(word_boundary("a, b", 0, true), Some(1));
        assert_eq!(word_boundary("a, b", 1, true), Some(3));
    }

    #[test]
    fn word_motion_counts_characters_and_not_bytes() {
        // Four characters, six bytes: an offset is a caret's unit, and `Caret::offset` is chars.
        assert_eq!(word_boundary("héllo wörld", 0, true), Some(6));
    }
}
