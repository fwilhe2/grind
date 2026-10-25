// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help while a formula is typed: what may be completed, and what the call under the caret wants
//! next. Portable. What to offer, how the list narrows, what accepting replaces and the runs the
//! band draws are `grind_sheet::formula::assist`'s; what is here is which **keys** steer the list.
//! `win.rs` reads the editor's caret and `sheet/draw.rs` draws the answer.
//!
//! ## A band, not a popup
//!
//! A popup list on Win32 is a second top-level window with its own class, theming
//! (`WM_CTLCOLORLISTBOX`), DPI answer and a focus problem, when typing must carry on into the
//! editor while the list narrows. A **band** this window draws — a line under the strip, like the
//! notice bar — follows the theme by construction, sits by the formula bar the text is mirrored
//! in, and appears in `--render-to`'s frames. The cost is five names on a line rather than rows
//! with summaries, so the chosen offer's summary follows the names.

pub use grind_sheet::formula::assist::{
    Assist, Ink, Piece, band, friendly_line, function_insert, function_lines,
};

use super::keymap::{Key, Mods};

/// How far a key moves the highlighted offer, or what it does to the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Put the highlighted offer into the editor.
    Accept,
    /// Move the highlight by this much, wrapping.
    Step(i32),
    /// Close the list, keeping the edit.
    Dismiss,
}

/// What a keystroke means **while offers are up** — asked before [`super::state::on_key`], `None`
/// for nearly every key. It claims Tab (else commit and move right), Up/Down (else commit in Enter
/// mode) and Escape (else the edit is thrown away). **Enter is not claimed**: it commits, the
/// common way to finish a formula, where an accidental completion would store the wrong thing.
pub fn on_key(offering: bool, key: Key, mods: Mods) -> Option<Reply> {
    if !offering || mods.ctrl || mods.alt {
        return None;
    }
    match key {
        Key::Tab if !mods.shift => Some(Reply::Accept),
        Key::Down => Some(Reply::Step(1)),
        Key::Up => Some(Reply::Step(-1)),
        Key::Escape => Some(Reply::Dismiss),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys the list claims, and — just as important — the ones it does not.
    #[test]
    fn the_list_claims_tab_and_the_arrows_and_leaves_enter_alone() {
        let plain = Mods::default();
        assert_eq!(on_key(true, Key::Tab, plain), Some(Reply::Accept));
        assert_eq!(on_key(true, Key::Down, plain), Some(Reply::Step(1)));
        assert_eq!(on_key(true, Key::Up, plain), Some(Reply::Step(-1)));
        assert_eq!(on_key(true, Key::Escape, plain), Some(Reply::Dismiss));
        // Enter commits the cell, as it always does.
        assert_eq!(on_key(true, Key::Return, plain), None);
        // With no list up, every one of them means what it always meant.
        for key in [Key::Tab, Key::Down, Key::Up, Key::Escape] {
            assert_eq!(on_key(false, key, plain), None, "{key:?}");
        }
        // A modifier is somebody asking for something else.
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(on_key(true, Key::Tab, ctrl), None);
    }
}
