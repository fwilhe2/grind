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

use grind_sheet::Pos;
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
    let go = |motion, extend| Some(Action::Move { motion, extend });
    let arrow = |dir| {
        go(
            match mods.ctrl {
                true => Motion::Edge(dir),
                false => Motion::By(dir),
            },
            mods.shift,
        )
    };
    match key {
        Key::Left => arrow(Dir::Left),
        Key::Right => arrow(Dir::Right),
        Key::Up => arrow(Dir::Up),
        Key::Down => arrow(Dir::Down),
        Key::Home if mods.ctrl => go(Motion::SheetStart, mods.shift),
        Key::Home => go(Motion::RowStart, mods.shift),
        Key::End if mods.ctrl => go(Motion::SheetEnd, mods.shift),
        Key::End => go(Motion::RowEnd, mods.shift),
        Key::PageUp => go(Motion::Page(Dir::Up), mods.shift),
        Key::PageDown => go(Motion::Page(Dir::Down), mods.shift),
        // Tab and Return walk the sheet the way they do while typing, so the habit is the
        // same before there is anything to type. Shift reverses rather than extends.
        Key::Tab if !mods.ctrl => go(
            Motion::By(match mods.shift {
                true => Dir::Left,
                false => Dir::Right,
            }),
            false,
        ),
        Key::Return if !mods.ctrl => go(
            Motion::By(match mods.shift {
                true => Dir::Up,
                false => Dir::Down,
            }),
            false,
        ),
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

/// Where a fill-handle drag points, given the selection and the cell under the pointer: which way
/// to fill and the last line. **One axis at a time**, the one the pointer left the selection
/// furthest on, a tie going vertical. `None` while the pointer is inside the selection.
pub fn fill_target(start: Pos, end: Pos, at: Pos) -> Option<(Dir, u32)> {
    // Horizontals first, because `max_by_key` keeps the *last* of equal keys — which is how
    // the tie goes to the vertical.
    let (distance, dir, line) = [
        (at.col.saturating_sub(end.col), Dir::Right, at.col),
        (start.col.saturating_sub(at.col), Dir::Left, at.col),
        (at.row.saturating_sub(end.row), Dir::Down, at.row),
        (start.row.saturating_sub(at.row), Dir::Up, at.row),
    ]
    .into_iter()
    .max_by_key(|(distance, ..)| *distance)?;
    (distance > 0).then_some((dir, line))
}

/// What a fill from `start`..`end` to `line` covers, source included — the outline a
/// handle drag paints, and the selection it leaves behind.
pub fn fill_rect(start: Pos, end: Pos, dir: Dir, line: u32) -> (Pos, Pos) {
    match dir {
        Dir::Down => (start, Pos::new(line, end.col)),
        Dir::Up => (Pos::new(line, start.col), end),
        Dir::Right => (start, Pos::new(end.row, line)),
        Dir::Left => (Pos::new(start.row, line), end),
    }
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

    /// The handle drag: one axis, the furthest one, and nothing while still inside.
    #[test]
    fn a_handle_drag_fills_the_axis_it_travelled_furthest_on() {
        // B2:C3 selected.
        let (start, end) = (Pos::new(1, 1), Pos::new(2, 2));
        assert_eq!(
            fill_target(start, end, Pos::new(6, 2)),
            Some((Dir::Down, 6))
        );
        assert_eq!(
            fill_target(start, end, Pos::new(1, 9)),
            Some((Dir::Right, 9))
        );
        assert_eq!(fill_target(start, end, Pos::new(0, 1)), Some((Dir::Up, 0)));
        assert_eq!(
            fill_target(start, end, Pos::new(2, 0)),
            Some((Dir::Left, 0))
        );
        // Diagonal: three rows down beats one column right.
        assert_eq!(
            fill_target(start, end, Pos::new(5, 3)),
            Some((Dir::Down, 5))
        );
        assert_eq!(
            fill_target(start, end, Pos::new(3, 6)),
            Some((Dir::Right, 6))
        );
        // A tie goes to the vertical, and inside the selection nothing happens.
        assert_eq!(
            fill_target(start, end, Pos::new(4, 4)),
            Some((Dir::Down, 4))
        );
        assert_eq!(fill_target(start, end, Pos::new(2, 1)), None);
    }
}
