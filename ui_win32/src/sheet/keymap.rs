// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which key means what, and where it moves the selection — pure functions, **no Windows types
//! outside one `#[cfg(windows)]` test**. The window turns a `WM_KEYDOWN` into a [`Key`] with
//! [`key_for`], so what a keystroke means is decided in code that tests on Linux.
//!
//! ## The virtual-key codes are written down here, not imported
//!
//! Importing `VK_LEFT` and its neighbours would pull the `windows` crate in and take the key table
//! off the portable side. The risk — a number copied from `winuser.h` wrongly — is answered by
//! `tests::the_virtual_key_codes_are_the_ones_windows_uses`, compiled **only on Windows**, which
//! checks every constant against Windows' metadata.
//!
//! ## The selection is presentation state
//!
//! An anchor and an active cell; the core sees a range as two positions when something is done to
//! it. [`Selection`], [`Motion`], [`Extent`], [`moved`], [`onto_visible`] and the Ctrl+arrow rule
//! are `grind_sheet::nav`'s; what is here is which key means which motion.

pub use grind_sheet::nav::{Dir, Extent, Motion, Selection, moved, onto_visible};

/// A key, as this shell cares about it: navigation, the verbs that are navigation in disguise, and
/// the keys that mean something while editing (`F2`, `Delete`, `Backspace`, `F9`), which
/// `sheet/state.rs` interprets. Printable characters are **not** here: a `WM_KEYDOWN` carries a
/// key, and typed text comes from `WM_CHAR`, through the layout and the IME.
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
    /// Amend the cell rather than replace it — Excel's, and everybody's.
    F2,
    /// Find the next cell holding the last word searched for; Shift+F3 the previous.
    F3,
    /// Go to an address — the name box's key, and Excel's.
    F5,
    /// The next misspelt word — Word's spelling key since the first Windows version, and the
    /// text pane's (`doc/spelling.md`).
    F7,
    /// Check the document — the "next problem" key, F8 in every client.
    F8,
    /// Recalculate — Excel's key, and needed, since a document whose cached values grind cannot
    /// reproduce is left stale on purpose.
    F9,
    /// A letter or digit as the keyboard reports it: **already upper case**, since a `WM_KEYDOWN`
    /// carries the key. Typed text comes from `WM_CHAR`.
    Char(char),
    /// Anything this shell does not claim, which goes back to `DefWindowProc` so Alt+F4, the
    /// system menu and Windows' accelerators keep working.
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// What a keystroke asks for. `None` means the key is not ours and the message keeps travelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move {
        motion: Motion,
        extend: bool,
    },
    /// Ctrl+A — everything the sheet uses.
    SelectAll,
    /// F5 / Ctrl+G — put the caret in the name box.
    GoTo,
}

// The virtual-key codes, from `winuser.h`. See the module comment for why they are written
// here rather than imported, and `tests` for what pins them.
const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0d;
const VK_ESCAPE: u32 = 0x1b;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_DELETE: u32 = 0x2e;
const VK_F2: u32 = 0x71;
const VK_F3: u32 = 0x72;
const VK_F5: u32 = 0x74;
const VK_F7: u32 = 0x76;
const VK_F8: u32 = 0x77;
const VK_F9: u32 = 0x78;

/// A virtual-key code as a [`Key`]. `0x30`–`0x39` and `0x41`–`0x5a` have no `VK_` names: they
/// *are* the ASCII codes for `0`–`9` and `A`–`Z`, which is why [`Key::Char`] arrives upper case.
pub fn key_for(vk: u32) -> Key {
    match vk {
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        VK_TAB => Key::Tab,
        VK_RETURN => Key::Return,
        VK_ESCAPE => Key::Escape,
        VK_BACK => Key::Backspace,
        VK_DELETE => Key::Delete,
        VK_F2 => Key::F2,
        VK_F3 => Key::F3,
        VK_F5 => Key::F5,
        VK_F7 => Key::F7,
        VK_F8 => Key::F8,
        VK_F9 => Key::F9,
        0x30..=0x39 | 0x41..=0x5a => Key::Char(char::from(vk as u8)),
        // The zoom keys: the main row's `=`/`+` and `-`, and the numeric pad's.
        0xBB | 0x6B => Key::Char('='),
        0xBD | 0x6D => Key::Char('-'),
        _ => Key::Other,
    }
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
        // Tab and Return walk the sheet the way they will while typing, so the habit is the
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
        // F5 is Excel's Go To and Ctrl+G is everybody's; both land in the name box, which is
        // the whole of this shell's go-to. There is no dialog and no palette (decision 4).
        Key::F5 if !mods.ctrl => Some(Action::GoTo),
        Key::Char(c) if mods.ctrl && !mods.shift => match c {
            'A' => Some(Action::SelectAll),
            'G' => Some(Action::GoTo),
            // X, C and V are `menu::accelerator`'s, consulted first by `sheet/state.rs::ready`, so
            // this arm only sees a Ctrl+letter with no verb.
            _ => None,
        },
        _ => None,
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

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Default::default()
        }
    }

    /// On Windows only, the constants are checked against Windows' own metadata, so a typo fails
    /// the build rather than making a key silently do nothing.
    #[test]
    #[cfg(windows)]
    fn the_virtual_key_codes_are_the_ones_windows_uses() {
        use windows::Win32::UI::Input::KeyboardAndMouse as vk;
        for (ours, theirs, name) in [
            (VK_BACK, vk::VK_BACK, "VK_BACK"),
            (VK_TAB, vk::VK_TAB, "VK_TAB"),
            (VK_RETURN, vk::VK_RETURN, "VK_RETURN"),
            (VK_ESCAPE, vk::VK_ESCAPE, "VK_ESCAPE"),
            (VK_PRIOR, vk::VK_PRIOR, "VK_PRIOR"),
            (VK_NEXT, vk::VK_NEXT, "VK_NEXT"),
            (VK_END, vk::VK_END, "VK_END"),
            (VK_HOME, vk::VK_HOME, "VK_HOME"),
            (VK_LEFT, vk::VK_LEFT, "VK_LEFT"),
            (VK_UP, vk::VK_UP, "VK_UP"),
            (VK_RIGHT, vk::VK_RIGHT, "VK_RIGHT"),
            (VK_DOWN, vk::VK_DOWN, "VK_DOWN"),
            (VK_DELETE, vk::VK_DELETE, "VK_DELETE"),
            (VK_F2, vk::VK_F2, "VK_F2"),
            (VK_F3, vk::VK_F3, "VK_F3"),
            (VK_F5, vk::VK_F5, "VK_F5"),
            (VK_F7, vk::VK_F7, "VK_F7"),
            (VK_F8, vk::VK_F8, "VK_F8"),
            (VK_F9, vk::VK_F9, "VK_F9"),
        ] {
            assert_eq!(ours, u32::from(theirs.0), "{name}");
        }
        // The unnamed range: Windows really does use ASCII for the letters and digits, which
        // is what `key_for` relies on to produce `Key::Char`.
        assert_eq!(key_for(u32::from(b'A')), Key::Char('A'));
        assert_eq!(key_for(u32::from(b'0')), Key::Char('0'));
    }

    #[test]
    fn the_arrows_are_the_codes_windows_sends() {
        assert_eq!(key_for(0x25), Key::Left);
        assert_eq!(key_for(0x28), Key::Down);
        assert_eq!(key_for(0x74), Key::F5);
        assert_eq!(key_for(0x71), Key::F2);
        assert_eq!(key_for(0x2e), Key::Delete);
        assert_eq!(key_for(0x08), Key::Backspace);
        // A key with no meaning here has to stay `Other`, so the window hands it back to
        // `DefWindowProc` rather than swallowing it.
        assert_eq!(key_for(0x12), Key::Other, "Alt");
        assert_eq!(key_for(0x00), Key::Other);
    }

    #[test]
    fn a_letter_arrives_upper_case_because_a_key_is_not_a_character() {
        // `WM_KEYDOWN` reports the *key*, so Ctrl+A and Ctrl+Shift+A are the same code and
        // the modifiers are what tell them apart. Lower-casing here would be inventing a
        // character the message did not carry.
        assert_eq!(key_for(u32::from(b'A')), Key::Char('A'));
        assert_eq!(action_for(Key::Char('A'), ctrl()), Some(Action::SelectAll));
    }

    #[test]
    fn ctrl_changes_an_arrow_from_a_step_into_an_edge() {
        assert_eq!(
            action_for(Key::Down, Mods::default()),
            Some(Action::Move {
                motion: Motion::By(Dir::Down),
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Down, ctrl()),
            Some(Action::Move {
                motion: Motion::Edge(Dir::Down),
                extend: false
            })
        );
        assert_eq!(
            action_for(Key::Down, shift()),
            Some(Action::Move {
                motion: Motion::By(Dir::Down),
                extend: true
            })
        );
    }

    /// Alt belongs to the menu bar (decision 4), so nothing here may claim it.
    #[test]
    fn alt_is_never_claimed() {
        let alt = Mods {
            alt: true,
            ..Default::default()
        };
        for key in [Key::Left, Key::Home, Key::Return, Key::Char('A'), Key::F5] {
            assert_eq!(action_for(key, alt), None, "{key:?}");
        }
    }

    /// A key this shell does not own must keep travelling, or Alt+F4 and the system menu stop
    /// working.
    #[test]
    fn unclaimed_keys_are_left_alone() {
        assert_eq!(action_for(Key::Other, Mods::default()), None);
        assert_eq!(action_for(Key::Escape, Mods::default()), None);
        // The editing keys belong to `state.rs` and the verbs to `menu.rs`; this table is
        // navigation and must not answer for either.
        for key in [Key::F2, Key::F9, Key::Delete, Key::Backspace] {
            assert_eq!(action_for(key, Mods::default()), None, "{key:?}");
        }
        // The clipboard letters are the accelerator table's, not the keymap's.
        for c in ['C', 'X', 'V'] {
            assert_eq!(action_for(Key::Char(c), ctrl()), None, "{c}");
        }
    }

    #[test]
    fn both_go_to_keys_reach_the_name_box() {
        assert_eq!(action_for(Key::F5, Mods::default()), Some(Action::GoTo));
        assert_eq!(action_for(Key::Char('G'), ctrl()), Some(Action::GoTo));
    }
}
