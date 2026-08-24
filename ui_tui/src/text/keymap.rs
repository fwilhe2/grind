// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Vi-style Normal-mode keys for the word processor, as a pure function.
//!
//! Same shape as [`crate::sheet::keymap`] — key plus modifiers in, an [`Action`] out, nothing
//! from ratatui anywhere near it, so the whole map unit-tests with no terminal attached.
//!
//! **What is deliberately *not* here: any arithmetic.** The spreadsheet's map computes the cell
//! a motion lands on, because a grid's geometry is the grid. A document's is not: `j` means
//! "the next *line*", and a line is an output of layout, so the answer comes from
//! `grind_text::App::caret_line` and this file only names the question
//! (`doc/text-layout.md`). That split is the whole reason the terminal shell was built before
//! the GTK one — if a motion needed arithmetic here, it would need the same arithmetic again in
//! GTK and in the browser, and the three would drift.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// Which way a motion goes, in *document* terms rather than screen terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// One character left or right, crossing a block boundary at the ends.
    Char(i32),
    /// Whole lines — `j`/`k`, and paging with a bigger number. Answered by the core.
    Line(i32),
    /// The visual line's own ends, which on a wrapped paragraph are not the paragraph's.
    LineStart,
    LineEnd,
    /// The first and last block of the document.
    DocStart,
    DocEnd,
}

/// What a Normal-mode key asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    /// `i` — type at the caret. `a` is the same thing one character along.
    Insert,
    Append,
    /// `o` — a new paragraph below this block, and start typing in it.
    OpenBelow,
    /// `x` — erase the character at the caret.
    EraseChar,
    /// `X` — delete the whole block.
    DeleteBlock,
    /// `J` — join this block with the one after it, vi's own spelling.
    Join,
    Undo,
    Redo,
    /// `:` — open the command line.
    Command,
}

/// The key map. One table, no state — `None` means the shell does not claim the key.
pub fn normal_action(code: KeyCode, mods: KeyModifiers) -> Option<Action> {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let go = |motion| Some(Action::Move(motion));
    match code {
        KeyCode::Char('h') | KeyCode::Left => go(Motion::Char(-1)),
        KeyCode::Char('l') | KeyCode::Right => go(Motion::Char(1)),
        KeyCode::Char('k') | KeyCode::Up => go(Motion::Line(-1)),
        KeyCode::Char('j') | KeyCode::Down => go(Motion::Line(1)),
        KeyCode::Char('f') if ctrl => go(Motion::Line(PAGE)),
        KeyCode::Char('b') if ctrl => go(Motion::Line(-PAGE)),
        KeyCode::PageDown => go(Motion::Line(PAGE)),
        KeyCode::PageUp => go(Motion::Line(-PAGE)),
        KeyCode::Char('0') | KeyCode::Home => go(Motion::LineStart),
        KeyCode::Char('$') | KeyCode::End => go(Motion::LineEnd),
        KeyCode::Char('g') => go(Motion::DocStart),
        KeyCode::Char('G') => go(Motion::DocEnd),
        KeyCode::Char('i') => Some(Action::Insert),
        KeyCode::Char('a') => Some(Action::Append),
        KeyCode::Char('o') => Some(Action::OpenBelow),
        KeyCode::Char('x') => Some(Action::EraseChar),
        KeyCode::Char('X') => Some(Action::DeleteBlock),
        KeyCode::Char('J') => Some(Action::Join),
        KeyCode::Char('u') => Some(Action::Undo),
        KeyCode::Char('r') if ctrl => Some(Action::Redo),
        KeyCode::Char(':') => Some(Action::Command),
        _ => None,
    }
}

/// How many lines a page is.
///
/// A fixed number rather than the window's height, unlike the spreadsheet's — the shell would
/// have to hand its geometry to a pure function to do better, and the map exists to have no
/// geometry in it. ponytail: pass the height in when somebody notices.
const PAGE: i32 = 20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hjkl_and_the_arrows_agree() {
        for (ch, arrow, motion) in [
            ('h', KeyCode::Left, Motion::Char(-1)),
            ('l', KeyCode::Right, Motion::Char(1)),
            ('k', KeyCode::Up, Motion::Line(-1)),
            ('j', KeyCode::Down, Motion::Line(1)),
        ] {
            let want = Some(Action::Move(motion));
            assert_eq!(normal_action(KeyCode::Char(ch), KeyModifiers::NONE), want);
            assert_eq!(normal_action(arrow, KeyModifiers::NONE), want);
        }
    }

    /// The difference from the spreadsheet's map, and the point of the whole layout decision:
    /// vertical movement is a *line* motion, and horizontal is a *character* one. In a grid
    /// both are the same kind of step.
    #[test]
    fn vertical_is_lines_and_horizontal_is_characters() {
        assert_eq!(
            normal_action(KeyCode::Char('j'), KeyModifiers::NONE),
            Some(Action::Move(Motion::Line(1)))
        );
        assert_eq!(
            normal_action(KeyCode::Char('l'), KeyModifiers::NONE),
            Some(Action::Move(Motion::Char(1)))
        );
    }

    #[test]
    fn paging_is_a_line_motion_with_a_bigger_number() {
        assert_eq!(
            normal_action(KeyCode::Char('f'), KeyModifiers::CONTROL),
            Some(Action::Move(Motion::Line(PAGE)))
        );
        assert_eq!(
            normal_action(KeyCode::Char('b'), KeyModifiers::CONTROL),
            Some(Action::Move(Motion::Line(-PAGE)))
        );
        // Unmodified, they are not bound.
        assert_eq!(normal_action(KeyCode::Char('f'), KeyModifiers::NONE), None);
    }

    #[test]
    fn the_editing_keys_are_the_four_caret_edits_plus_undo() {
        for (ch, action) in [
            ('i', Action::Insert),
            ('a', Action::Append),
            ('o', Action::OpenBelow),
            ('x', Action::EraseChar),
            ('X', Action::DeleteBlock),
            ('J', Action::Join),
            ('u', Action::Undo),
            (':', Action::Command),
        ] {
            assert_eq!(
                normal_action(KeyCode::Char(ch), KeyModifiers::NONE),
                Some(action),
                "{ch}"
            );
        }
        assert_eq!(
            normal_action(KeyCode::Char('r'), KeyModifiers::CONTROL),
            Some(Action::Redo)
        );
    }

    /// `i` and `a` are different here and the same in the spreadsheet's map, because a cell has
    /// no caret to be one character along from.
    #[test]
    fn i_and_a_differ_because_a_paragraph_has_a_caret() {
        assert_ne!(
            normal_action(KeyCode::Char('i'), KeyModifiers::NONE),
            normal_action(KeyCode::Char('a'), KeyModifiers::NONE)
        );
    }
}
