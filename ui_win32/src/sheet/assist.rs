// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help while a formula is being typed: what may be completed, and what the call under the caret
//! wants next — `doc/sheet-shell.md`'s M6 for this shell, and W9's half of it.
//!
//! **Portable, and tested on any host**, like [`super::keymap`], [`super::state`] and
//! [`super::status`]. What to offer, how the list narrows, what accepting replaces and the runs
//! the band draws are `grind_sheet::formula::assist`'s — this file's own until the Mac's cell
//! editor would have copied them — and what is left here is which **keys** steer the list, since
//! a key code is this platform's. `win.rs` reads the editor's caret and `sheet/draw.rs` puts the
//! answer on screen.
//!
//! ## Where it is shown, and why not in a popup
//!
//! `ui_sheet_gtk` floats a `gtk::Popover` under the cell being edited. This shell draws a **band**
//! instead — one line under the strip, in the same place and the same manner as the notice bar —
//! and that is a decision rather than a shortcut. A popup list on Win32 is a second top-level
//! window with its own class, its own theming (`WM_CTLCOLORLISTBOX`, since a `LISTBOX` paints
//! itself), its own DPI answer, and a focus problem: the whole point is that typing carries on
//! into the editor underneath while the list narrows, so the popup must never take the keyboard.
//! A band this window draws itself has none of those, follows the theme by construction, sits
//! next to the formula bar the text is also mirrored in, and — being drawn rather than a
//! control — is visible in `--render-to`'s windowless frame like every other read-out here.
//!
//! What it costs is the vertical list: five names on one line rather than eight rows with their
//! summaries. So the chosen offer's own summary is drawn after the names, which is the line a
//! reader actually wants, and the rest are names alone.
//!
//! ## What is *not* here
//!
//! **Point mode** — arrow keys building a reference into a half-typed formula. It stays on
//! `doc/windows-shell.md`'s gap list: it is a third editing mode rather than a read-out, and
//! [`super::state::Outcome`] has no `Point` for it to arrive as.

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

/// What a keystroke means **while a list of offers is up** — asked before
/// [`super::state::on_key`], and `None` for everything this list has no answer for, which is
/// almost every key.
///
/// The three it claims are the three that would otherwise do something the user did not mean:
/// Tab commits the cell and moves right, Up/Down commit in Enter mode, and Escape throws the
/// whole edit away. **Enter is deliberately not claimed**: it commits, as it always does, because
/// a formula finished by pressing Enter is the common case and an accidental completion in its
/// place would store the wrong thing.
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
