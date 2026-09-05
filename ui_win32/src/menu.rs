// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every verb this shell has, as data — and the menu bar it hangs in.
//!
//! **Portable, and tested on any host.** `doc/windows-shell.md`'s decision 4 makes the menu bar
//! this platform's growable surface, where the GNOME window needed a Ctrl+K palette: Windows
//! draws a menu itself, so it scales with DPI, follows the theme and gets Alt-key navigation for
//! nothing. The price of a surface that is *meant* to grow is a rule about what may go in it, and
//! it is the GTK window's rule unchanged — **a verb goes in a menu, a property of the selection
//! goes on the format strip** (which is W5's, since `CharStyle` and `CellStyle` bound it).
//!
//! Two things follow from the menus being a table rather than a sequence of `AppendMenuW` calls:
//!
//! * The **id** of a command is derived from its position in [`Command::ALL`] and never written
//!   down twice, so the classic Win32 bug of two menu items sharing a `WM_COMMAND` id cannot
//!   happen here.
//! * The **check that every command is reachable** runs on Linux with no window at all. Its other
//!   half — that every command has a handler — is the Rust compiler's: `win.rs`'s dispatcher
//!   matches on [`Command`] exhaustively, so a command with nowhere to go fails the build.
//!
//! The accelerators live here too rather than in `sheet/keymap.rs`, because they are *verbs*
//! and that file is navigation. `sheet/state.rs` consults this first and the keymap second,
//! which is what makes Ctrl+S mean Save whatever the grid would otherwise have done with `S`.
//!
//! ponytail: the accelerators are matched by [`accelerator`] rather than by a real `HACCEL` and
//! `TranslateAccelerator`. The table is the same either way; what a real accelerator table would
//! add is Windows drawing the key next to the menu item automatically instead of this file
//! spelling it. W7 is where that lands, along with the context menus.

use crate::sheet::keymap::{Key, Mods};

/// One verb. Everything the shell can be *asked* to do that is not a movement.
///
/// Deliberately not a home for anything that reads or writes a property of the selection —
/// alignment, a number format, bold — which is the format strip's admission test and W5's work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    New,
    Open,
    Save,
    SaveAs,
    Exit,
    Undo,
    Redo,
    /// Copy the selection to the clipboard as `CF_UNICODETEXT`, then clear it —
    /// `App::clear_range` under a `crate::clipboard::set_text`.
    Cut,
    /// Copy the selection to the clipboard, formulas and all, as tab-separated
    /// `App::input_text` — `doc/windows-shell.md` decision 6.
    Copy,
    /// Fill from the clipboard at the selection's corner — `App::enter_range` under a
    /// `crate::clipboard::get_text`.
    Paste,
    /// Empty the selected cells, keeping their formatting — `App::clear_range`.
    ClearCells,
    /// Put the caret in the name box. A menu item as well as F5, because a verb nobody can
    /// find is a verb this shell does not have.
    GoTo,
    Recalculate,
    SheetAdd,
    SheetRename,
    SheetDelete,
    SheetNext,
    SheetPrevious,
}

impl Command {
    /// Every command, in the order that decides their `WM_COMMAND` ids.
    ///
    /// Adding one here and nowhere else fails two checks at once: the test below says it is in no
    /// menu, and `win.rs`'s exhaustive match says it has no handler.
    pub const ALL: &'static [Command] = &[
        Command::New,
        Command::Open,
        Command::Save,
        Command::SaveAs,
        Command::Exit,
        Command::Undo,
        Command::Redo,
        Command::Cut,
        Command::Copy,
        Command::Paste,
        Command::ClearCells,
        Command::GoTo,
        Command::Recalculate,
        Command::SheetAdd,
        Command::SheetRename,
        Command::SheetDelete,
        Command::SheetNext,
        Command::SheetPrevious,
    ];

    /// The `WM_COMMAND` id this verb arrives as.
    ///
    /// Offset by 100 so that a control notification's id — the name box is 1, the editor 2 —
    /// can never collide with a menu command's. Win32 puts both through the same message and
    /// tells them apart by the high word, and this makes a mistake there visible rather than
    /// silent.
    pub fn id(self) -> u16 {
        let at = Self::ALL
            .iter()
            .position(|c| *c == self)
            .expect("every command is in ALL");
        FIRST_ID + at as u16
    }
}

/// The first id a menu command may take. Below it are the child controls' ids.
pub const FIRST_ID: u16 = 100;

/// Which command a `WM_COMMAND` id names, or `None` for one that is not a menu command.
pub fn command_for(id: u16) -> Option<Command> {
    id.checked_sub(FIRST_ID)
        .and_then(|at| Command::ALL.get(usize::from(at)))
        .copied()
}

/// One entry in a menu: a verb, or the line between two groups of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// The label carries its own `&` mnemonic, which is what Windows draws underlined, and the
    /// accelerator's *name* after a tab — the convention every Win32 menu follows.
    Verb {
        command: Command,
        label: &'static str,
    },
    Separator,
}

/// One top-level menu.
#[derive(Clone, Copy, Debug)]
pub struct Menu {
    pub title: &'static str,
    pub items: &'static [Item],
}

/// The menu bar.
///
/// Four menus and nothing that is not a verb. What is deliberately absent: a View menu (this
/// shell's overlays are W6), a Format menu (the format strip's, W5), and anything resembling a
/// ribbon — `doc/sheet-shell.md`'s tab strip was removed for being one, and the argument carries.
pub const MENUS: &[Menu] = &[
    Menu {
        title: "&File",
        items: &[
            Item::Verb {
                command: Command::New,
                label: "&New\tCtrl+N",
            },
            Item::Verb {
                command: Command::Open,
                label: "&Open…\tCtrl+O",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Save,
                label: "&Save\tCtrl+S",
            },
            Item::Verb {
                command: Command::SaveAs,
                label: "Save &As…\tCtrl+Shift+S",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Exit,
                label: "E&xit\tAlt+F4",
            },
        ],
    },
    Menu {
        title: "&Edit",
        items: &[
            Item::Verb {
                command: Command::Undo,
                label: "&Undo\tCtrl+Z",
            },
            Item::Verb {
                command: Command::Redo,
                label: "&Redo\tCtrl+Y",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Cut,
                label: "Cu&t\tCtrl+X",
            },
            Item::Verb {
                command: Command::Copy,
                label: "&Copy\tCtrl+C",
            },
            Item::Verb {
                command: Command::Paste,
                label: "&Paste\tCtrl+V",
            },
            Item::Separator,
            Item::Verb {
                command: Command::ClearCells,
                label: "&Delete\tDel",
            },
            Item::Separator,
            Item::Verb {
                command: Command::GoTo,
                label: "&Go To…\tF5",
            },
        ],
    },
    Menu {
        title: "&Sheet",
        items: &[
            Item::Verb {
                command: Command::SheetAdd,
                label: "&Add…",
            },
            Item::Verb {
                command: Command::SheetRename,
                label: "&Rename…",
            },
            Item::Verb {
                command: Command::SheetDelete,
                label: "&Delete",
            },
            Item::Separator,
            Item::Verb {
                command: Command::SheetNext,
                label: "&Next\tCtrl+PgDn",
            },
            Item::Verb {
                command: Command::SheetPrevious,
                label: "&Previous\tCtrl+PgUp",
            },
        ],
    },
    Menu {
        title: "&Data",
        items: &[Item::Verb {
            command: Command::Recalculate,
            label: "&Recalculate\tF9",
        }],
    },
];

/// Which verb a keystroke asks for, if any.
///
/// Consulted **before** the navigation keymap, which is the whole reason it exists as a separate
/// question: Ctrl+S has to be Save even though `S` is a perfectly good key to type into a cell,
/// and Ctrl+PageDown has to be "next sheet" even though PageDown is a motion.
///
/// Deliberately silent about Delete: clearing the selection is a verb, but the *key* only means
/// it in Ready mode, and deciding that is `sheet/state.rs`'s job rather than this table's.
pub fn accelerator(key: Key, mods: Mods) -> Option<Command> {
    if mods.alt {
        return None;
    }
    match (key, mods.ctrl, mods.shift) {
        (Key::Char('N'), true, false) => Some(Command::New),
        (Key::Char('O'), true, false) => Some(Command::Open),
        (Key::Char('S'), true, false) => Some(Command::Save),
        (Key::Char('S'), true, true) => Some(Command::SaveAs),
        (Key::Char('Z'), true, false) => Some(Command::Undo),
        (Key::Char('Y'), true, false) => Some(Command::Redo),
        (Key::Char('X'), true, false) => Some(Command::Cut),
        (Key::Char('C'), true, false) => Some(Command::Copy),
        (Key::Char('V'), true, false) => Some(Command::Paste),
        (Key::PageDown, true, _) => Some(Command::SheetNext),
        (Key::PageUp, true, _) => Some(Command::SheetPrevious),
        (Key::F9, false, false) => Some(Command::Recalculate),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn verbs() -> Vec<Command> {
        MENUS
            .iter()
            .flat_map(|menu| menu.items)
            .filter_map(|item| match item {
                Item::Verb { command, .. } => Some(*command),
                Item::Separator => None,
            })
            .collect()
    }

    /// Decision 4's rule made mechanical, and the half of it a Linux machine can check: a verb
    /// that is in `ALL` and in no menu is a verb nobody can find. The other half — that every
    /// command has a handler — is `win.rs`'s exhaustive match, which is the compiler's job.
    #[test]
    fn every_command_is_reachable_from_exactly_one_menu_item() {
        let verbs = verbs();
        for command in Command::ALL {
            let found = verbs.iter().filter(|c| *c == command).count();
            assert_eq!(found, 1, "{command:?} appears in {found} menu items");
        }
        assert_eq!(
            verbs.len(),
            Command::ALL.len(),
            "a menu item names a command that is not in ALL"
        );
    }

    /// The ids are derived, so this is really a check that nothing has started writing them
    /// down: two items sharing a `WM_COMMAND` id is the classic Win32 menu bug, and it looks
    /// like one item quietly doing the other's work.
    #[test]
    fn every_command_has_an_id_of_its_own_and_it_round_trips() {
        let mut seen = HashSet::new();
        for command in Command::ALL {
            let id = command.id();
            assert!(
                id >= FIRST_ID,
                "{command:?} would collide with a control id"
            );
            assert!(seen.insert(id), "{command:?} shares an id");
            assert_eq!(command_for(id), Some(*command));
        }
        assert_eq!(command_for(0), None, "a control notification is not a verb");
        assert_eq!(command_for(FIRST_ID - 1), None);
        assert_eq!(command_for(u16::MAX), None);
    }

    /// A Win32 menu underlines the letter after `&`, and a menu with two items claiming the
    /// same one in the same menu makes Alt-navigation ambiguous.
    #[test]
    fn every_menu_has_distinct_mnemonics() {
        let mnemonic = |label: &str| {
            label
                .split('&')
                .nth(1)
                .and_then(|rest| rest.chars().next())
                .map(|c| c.to_ascii_uppercase())
        };
        let mut titles = HashSet::new();
        for menu in MENUS {
            let title = mnemonic(menu.title).unwrap_or_else(|| panic!("{}", menu.title));
            assert!(titles.insert(title), "two menus answer Alt+{title}");
            let mut keys = HashSet::new();
            for item in menu.items {
                let Item::Verb { label, .. } = item else {
                    continue;
                };
                let key = mnemonic(label).unwrap_or_else(|| panic!("{label} has no mnemonic"));
                assert!(keys.insert(key), "{}: two items answer {key}", menu.title);
            }
        }
    }

    /// An accelerator has to name a verb that is really in a menu, or the menu shows a key that
    /// does something the menu cannot.
    #[test]
    fn every_accelerator_names_a_command_that_is_in_a_menu() {
        let verbs = verbs();
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let ctrl_shift = Mods {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        for (key, mods, want) in [
            (Key::Char('N'), ctrl, Command::New),
            (Key::Char('O'), ctrl, Command::Open),
            (Key::Char('S'), ctrl, Command::Save),
            (Key::Char('S'), ctrl_shift, Command::SaveAs),
            (Key::Char('Z'), ctrl, Command::Undo),
            (Key::Char('Y'), ctrl, Command::Redo),
            (Key::Char('X'), ctrl, Command::Cut),
            (Key::Char('C'), ctrl, Command::Copy),
            (Key::Char('V'), ctrl, Command::Paste),
            (Key::PageDown, ctrl, Command::SheetNext),
            (Key::PageUp, ctrl, Command::SheetPrevious),
            (Key::F9, Mods::default(), Command::Recalculate),
        ] {
            assert_eq!(accelerator(key, mods), Some(want), "{key:?}");
            assert!(verbs.contains(&want), "{want:?} is in no menu");
        }
    }

    /// Alt belongs to the menu bar itself, so no accelerator may claim it — otherwise Alt+F
    /// would do something before the File menu ever opened.
    #[test]
    fn alt_is_left_to_the_menu_bar() {
        let alt = Mods {
            alt: true,
            ctrl: true,
            shift: false,
        };
        for key in [Key::Char('S'), Key::Char('N'), Key::F9, Key::PageDown] {
            assert_eq!(accelerator(key, alt), None, "{key:?}");
        }
    }

    /// A plain letter is text to type, not a command — the failure mode of an accelerator
    /// table that forgets to test its modifiers.
    #[test]
    fn an_unmodified_key_is_not_a_verb() {
        for key in [Key::Char('S'), Key::Char('N'), Key::Left, Key::PageDown] {
            assert_eq!(accelerator(key, Mods::default()), None, "{key:?}");
        }
    }
}
