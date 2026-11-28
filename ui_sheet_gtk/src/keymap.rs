// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what, and what it does to the selection — pure functions with **no GTK types**:
//! the widget turns a `gdk::Key` into [`Key`], so navigation tests without a display.
//!
//! The selection is **presentation state** — an anchor and an active cell; the core sees a range
//! only as two positions when something is done to it. [`Selection`], [`Motion`], [`Extent`] and
//! [`moved`] are `grind_sheet::nav`'s; what is here is which key means which motion, and the fill
//! handle's arithmetic. The keys while editing are `state.rs`'s.

use grind_sheet::nav;
pub use grind_sheet::nav::{Dir, Extent, Motion, Selection, moved};

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
    Tab,
    Return,
    Escape,
    Delete,
    Backspace,
    F2,
    /// Cycles the `$` markers of a reference while editing (`state::cycle_absolute`).
    F4,
    /// A character, exactly as it was typed — [`crate::state`] seeds an edit with it, so
    /// case-folding here would make every typed capital arrive in lower case.
    Char(char),
    /// Anything this shell does not claim, which must keep travelling.
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// What a keystroke asks for. Returning `None` means the shell does not own this key, and
/// the event must keep travelling — which is what leaves the toolkit's own bindings, and
/// later the editor's input method, working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move {
        motion: Motion,
        extend: bool,
    },
    /// Ctrl+A — everything the sheet uses.
    SelectAll,
    Copy,
    /// Copy, then empty what was copied.
    Cut,
    Paste,
    /// Ctrl+Shift+C — a calculated cell's result rather than its formula.
    CopyValue,
    /// Ctrl+D / Ctrl+R — the selection's top row or left column, replicated into the rest
    /// of the selection (`doc/sheet-shell.md`'s "extend a calculation").
    Fill(Dir),
}

/// The key map. One table, no state.
pub fn action_for(key: Key, mods: Mods) -> Option<Action> {
    if mods.alt {
        return None;
    }
    let walking = match key {
        Key::Left => Some(nav::Key::Left),
        Key::Right => Some(nav::Key::Right),
        Key::Up => Some(nav::Key::Up),
        Key::Down => Some(nav::Key::Down),
        Key::Home => Some(nav::Key::Home),
        Key::End => Some(nav::Key::End),
        Key::PageUp => Some(nav::Key::PageUp),
        Key::PageDown => Some(nav::Key::PageDown),
        Key::Tab => Some(nav::Key::Tab),
        Key::Return => Some(nav::Key::Return),
        _ => None,
    };
    if let Some(motion) = walking.and_then(|k| nav::motion_for(k, mods.ctrl, mods.shift)) {
        return Some(Action::Move {
            motion: motion.0,
            extend: motion.1,
        });
    }
    match key {
        Key::Char(c) if mods.ctrl && !mods.shift => match c.to_ascii_lowercase() {
            'a' => Some(Action::SelectAll),
            'c' => Some(Action::Copy),
            'x' => Some(Action::Cut),
            'v' => Some(Action::Paste),
            'd' => Some(Action::Fill(Dir::Down)),
            'r' => Some(Action::Fill(Dir::Right)),
            _ => None,
        },
        Key::Char(c) if mods.ctrl && mods.shift && c.eq_ignore_ascii_case(&'c') => {
            Some(Action::CopyValue)
        }
        _ => None,
    }
}

/// What a fill acts on along one axis, given the selection's first and last line: the line to copy
/// *from*, and the first and last to copy *into*. The first line is always the source: a deeper
/// selection fills itself, and a one-line one fills the line after it. Excel copies the line
/// before a single cell; this never reads a cell that was not selected.
pub fn fill_span(first: u32, last: u32) -> Option<(u32, u32, u32)> {
    let next = first.checked_add(1)?;
    Some((first, next, last.max(next)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_ctrl_modified_key_never_also_moves_one_cell() {
        assert_eq!(
            action_for(Key::Down, ctrl()),
            Some(Action::Move {
                motion: Motion::Edge(Dir::Down),
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Down, Mods::default()),
            Some(Action::Move {
                motion: Motion::By(Dir::Down),
                extend: false
            })
        );
    }

    /// Everything the shell does not claim has to keep travelling, or the toolkit's own
    /// bindings — and later the editor's input method — stop working.
    #[test]
    fn unclaimed_keys_are_left_alone() {
        assert_eq!(action_for(Key::Char('x'), Mods::default()), None);
        assert_eq!(action_for(Key::Other, ctrl()), None);
        // Alt belongs to the window manager and the menus.
        assert_eq!(
            action_for(
                Key::Left,
                Mods {
                    alt: true,
                    ..Default::default()
                }
            ),
            None
        );
    }

    #[test]
    fn the_clipboard_keys_are_the_ones_everybody_has() {
        for (c, action) in [
            ('c', Action::Copy),
            ('x', Action::Cut),
            ('v', Action::Paste),
            ('a', Action::SelectAll),
        ] {
            assert_eq!(action_for(Key::Char(c), ctrl()), Some(action));
            // The keyval's case is the keyboard's business, not the shortcut's.
            assert_eq!(
                action_for(Key::Char(c.to_ascii_uppercase()), ctrl()),
                Some(action)
            );
        }
    }

    #[test]
    fn fill_and_copy_value_are_the_excel_and_calc_keys() {
        assert_eq!(
            action_for(Key::Char('d'), ctrl()),
            Some(Action::Fill(Dir::Down))
        );
        assert_eq!(
            action_for(Key::Char('r'), ctrl()),
            Some(Action::Fill(Dir::Right))
        );
        let ctrl_shift = Mods {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            action_for(Key::Char('c'), ctrl_shift),
            Some(Action::CopyValue)
        );
        // Plain Ctrl+C stays the formula-preserving copy.
        assert_eq!(action_for(Key::Char('c'), ctrl()), Some(Action::Copy));
    }

    #[test]
    fn the_selected_line_is_the_source_whatever_the_selection_is() {
        // Three rows selected: the top one is the source, the other two the targets.
        assert_eq!(fill_span(2, 4), Some((2, 3, 4)));
        // One cell: it is the source, and the cell after it the one target.
        assert_eq!(fill_span(2, 2), Some((2, 3, 3)));
        assert_eq!(fill_span(0, 0), Some((0, 1, 1)));
    }
}
