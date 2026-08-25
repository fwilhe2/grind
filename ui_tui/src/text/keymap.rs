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

/// What a Normal- or Visual-mode key asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    /// `i` — type at the caret. `a` is the same thing one character along.
    Insert,
    Append,
    /// `o` — a new paragraph below this block, and start typing in it.
    OpenBelow,
    /// `x` — erase the character at the caret, or the selection in Visual mode.
    EraseChar,
    /// `X` — delete the whole block.
    DeleteBlock,
    /// `J` — join this block with the one after it, vi's own spelling.
    Join,
    Undo,
    Redo,
    /// `:` — open the command line.
    Command,
    /// `v` — start selecting, or stop. The terminal's answer to Shift+arrow, and what every
    /// verb over a *range* needs before it can exist.
    Visual,
    /// `y` — copy the selection into the register; `p` puts it back.
    Yank,
    Put,
    /// A marker key over a selection: `*` bold, `_` underline, `~` strikethrough, `` ` ``
    /// monospace, `/` italic.
    ///
    /// **The same notation `markdown.rs` recognises while typing**, on one key — a terminal has
    /// no toolbar, so the toolbar's vocabulary is the notation. (`/` rather than a lone `*`
    /// for italic: one asterisk is bold's own key here, and doubling a keystroke to mean
    /// *less* emphasis reads backwards.)
    Emphasise(crate::text::markdown::Emphasis),
    /// `-` over a selection: back to no formatting at all.
    Plain,
    /// `Esc` — leave Visual mode without doing anything to what was selected.
    Escape,
}

/// The key map. One table, no state — `None` means the shell does not claim the key.
///
/// `visual` says whether a selection is being dragged out, and changes only what the keys that
/// need a range mean: everything else is the same key in both modes, which is what makes
/// Visual feel like Normal with a highlight rather than like a different editor.
pub fn normal_action(code: KeyCode, mods: KeyModifiers, visual: bool) -> Option<Action> {
    use crate::text::markdown::Emphasis;
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
        KeyCode::Char('v') => Some(Action::Visual),
        KeyCode::Esc => Some(Action::Escape),
        // The formatting keys only mean formatting when there is something to format;
        // otherwise `*` and `-` are ordinary characters nobody has claimed in Normal mode.
        KeyCode::Char('*') if visual => Some(Action::Emphasise(Emphasis::Bold)),
        KeyCode::Char('/') if visual => Some(Action::Emphasise(Emphasis::Italic)),
        KeyCode::Char('_') if visual => Some(Action::Emphasise(Emphasis::Underline)),
        KeyCode::Char('~') if visual => Some(Action::Emphasise(Emphasis::Strike)),
        KeyCode::Char('`') if visual => Some(Action::Emphasise(Emphasis::Code)),
        KeyCode::Char('-') if visual => Some(Action::Plain),
        KeyCode::Char('y') => Some(Action::Yank),
        KeyCode::Char('p') => Some(Action::Put),
        KeyCode::Char('i') => Some(Action::Insert),
        KeyCode::Char('a') => Some(Action::Append),
        KeyCode::Char('o') => Some(Action::OpenBelow),
        KeyCode::Char('x') | KeyCode::Char('d') => Some(Action::EraseChar),
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
            assert_eq!(
                normal_action(KeyCode::Char(ch), KeyModifiers::NONE, false),
                want
            );
            assert_eq!(normal_action(arrow, KeyModifiers::NONE, false), want);
        }
    }

    /// The difference from the spreadsheet's map, and the point of the whole layout decision:
    /// vertical movement is a *line* motion, and horizontal is a *character* one. In a grid
    /// both are the same kind of step.
    #[test]
    fn vertical_is_lines_and_horizontal_is_characters() {
        assert_eq!(
            normal_action(KeyCode::Char('j'), KeyModifiers::NONE, false),
            Some(Action::Move(Motion::Line(1)))
        );
        assert_eq!(
            normal_action(KeyCode::Char('l'), KeyModifiers::NONE, false),
            Some(Action::Move(Motion::Char(1)))
        );
    }

    #[test]
    fn paging_is_a_line_motion_with_a_bigger_number() {
        assert_eq!(
            normal_action(KeyCode::Char('f'), KeyModifiers::CONTROL, false),
            Some(Action::Move(Motion::Line(PAGE)))
        );
        assert_eq!(
            normal_action(KeyCode::Char('b'), KeyModifiers::CONTROL, false),
            Some(Action::Move(Motion::Line(-PAGE)))
        );
        // Unmodified, they are not bound.
        assert_eq!(
            normal_action(KeyCode::Char('f'), KeyModifiers::NONE, false),
            None
        );
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
                normal_action(KeyCode::Char(ch), KeyModifiers::NONE, false),
                Some(action),
                "{ch}"
            );
        }
        assert_eq!(
            normal_action(KeyCode::Char('r'), KeyModifiers::CONTROL, false),
            Some(Action::Redo)
        );
    }

    /// The keys a *range* needs, and the rule that keeps them out of the way until there is
    /// one: `*` and `-` are formatting in Visual mode and nothing at all in Normal, so they
    /// stay available to whatever wants them next.
    #[test]
    fn the_formatting_keys_only_bind_once_something_is_selected() {
        use crate::text::markdown::Emphasis;
        for (ch, emphasis) in [
            ('*', Emphasis::Bold),
            ('/', Emphasis::Italic),
            ('_', Emphasis::Underline),
            ('~', Emphasis::Strike),
            ('`', Emphasis::Code),
        ] {
            assert_eq!(
                normal_action(KeyCode::Char(ch), KeyModifiers::NONE, true),
                Some(Action::Emphasise(emphasis)),
                "{ch}"
            );
            assert_eq!(
                normal_action(KeyCode::Char(ch), KeyModifiers::NONE, false),
                None,
                "{ch} means nothing without a selection"
            );
        }
        assert_eq!(
            normal_action(KeyCode::Char('-'), KeyModifiers::NONE, true),
            Some(Action::Plain)
        );
        // `v` and Esc are the same key in both modes: one starts a selection, the other drops
        // it, and neither depends on which mode asked.
        for visual in [false, true] {
            assert_eq!(
                normal_action(KeyCode::Char('v'), KeyModifiers::NONE, visual),
                Some(Action::Visual)
            );
            assert_eq!(
                normal_action(KeyCode::Esc, KeyModifiers::NONE, visual),
                Some(Action::Escape)
            );
        }
    }

    /// `i` and `a` are different here and the same in the spreadsheet's map, because a cell has
    /// no caret to be one character along from.
    #[test]
    fn i_and_a_differ_because_a_paragraph_has_a_caret() {
        assert_ne!(
            normal_action(KeyCode::Char('i'), KeyModifiers::NONE, false),
            normal_action(KeyCode::Char('a'), KeyModifiers::NONE, false)
        );
    }
}
