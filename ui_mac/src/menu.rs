// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The menu bar, as data — `ui_win32/src/menu.rs`'s idea in the Mac's vocabulary.
//!
//! **Portable, and tested on any host.** The menu bar is this platform's surface that grows
//! (decision 3), so it is also where "follow the platform's conventions" would rot first in a
//! shell nobody here can run. `doc/macos-shell.md`'s *Conventions made mechanical* is therefore a
//! list of tests at the foot of this file: standard items use the standard selectors, no two items
//! share a key equivalent, nothing takes a shortcut the system reserves, the platform's own
//! shortcuts are the platform's, an ellipsis means the item asks for more, and the application,
//! Window and Help menus are the ones AppKit is told about.
//!
//! An item's action is one of two things. A **standard selector** goes down the responder chain,
//! where AppKit, `NSDocument` and `NSDocumentController` answer most of them with no code here —
//! Open, Save, Close, Revert, Duplicate, Undo, the window list, Hide and Quit — and where an item
//! nobody answers is greyed by the system. A [`Command`] is a verb of this shell's own, sent as one
//! selector with the command's index as the item's tag, so the dispatcher in `app.rs` matches on
//! [`Command`] exhaustively and a command with no handler fails the build.

/// The application's name, as the menu bar, the Dock and About say it.
pub const APP_NAME: &str = "Grind";

/// The selector every [`Command`] item sends, with [`Command::tag`] as the item's tag.
pub const COMMAND_SELECTOR: &str = "performGrindCommand:";

/// A verb of this shell's own — one no standard selector already means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// A new, empty spreadsheet. Not `newDocument:`, which makes a document of the *first* type
    /// only: one application holding two kinds of document needs a New for each (decision 1).
    NewSheet,
    /// A new, empty text document.
    NewText,
}

impl Command {
    pub const ALL: [Command; 2] = [Command::NewSheet, Command::NewText];

    /// The item's tag: the command's place in [`Command::ALL`], so no two share one.
    pub fn tag(self) -> isize {
        Command::ALL
            .iter()
            .position(|command| *command == self)
            .expect("every command is in ALL") as isize
    }

    /// The command an item's tag names.
    pub fn from_tag(tag: isize) -> Option<Command> {
        usize::try_from(tag)
            .ok()
            .and_then(|at| Command::ALL.get(at).copied())
    }
}

/// What choosing an item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// A standard selector, sent to the first responder that answers it.
    Standard(&'static str),
    Command(Command),
}

/// The modifier keys of a key equivalent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub command: bool,
    pub shift: bool,
    pub option: bool,
    pub control: bool,
}

/// ⌘ alone, which is most of them.
pub const CMD: Mods = Mods {
    command: true,
    shift: false,
    option: false,
    control: false,
};
pub const SHIFT_CMD: Mods = Mods { shift: true, ..CMD };
pub const OPT_CMD: Mods = Mods {
    option: true,
    ..CMD
};
pub const CTRL_CMD: Mods = Mods {
    control: true,
    ..CMD
};

/// A key equivalent: the key, as `NSMenuItem.keyEquivalent` takes it (lower case — Shift is a
/// modifier here, never an upper-case letter), and its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub key: &'static str,
    pub mods: Mods,
}

impl Key {
    // Tested now, and a notice's from M4 (every notice spells a key as ⌘, never Ctrl).
    #[cfg(test)]
    /// How the menu draws it — `⇧⌘S` — which is also how a notice spells a key (⌘, never Ctrl).
    pub fn spelled(&self) -> String {
        let mut out = String::new();
        for (on, glyph) in [
            (self.mods.control, '⌃'),
            (self.mods.option, '⌥'),
            (self.mods.shift, '⇧'),
            (self.mods.command, '⌘'),
        ] {
            if on {
                out.push(glyph);
            }
        }
        out.push_str(&self.key.to_uppercase());
        out
    }
}

const fn key(key: &'static str, mods: Mods) -> Option<Key> {
    Some(Key { key, mods })
}

/// One row of a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Entry {
        title: &'static str,
        key: Option<Key>,
        action: Action,
    },
    Separator,
    /// A submenu of items this table lists.
    Submenu {
        title: &'static str,
        menu: &'static Menu,
    },
}

/// Which of AppKit's own menus a menu is, when it is one.
///
/// Telling AppKit is what makes them work: the Window menu gets the window list, the Help menu
/// gets Help ▸ Search over every item, Services fills itself, and `NSDocumentController` finds
/// Open Recent by its `clearRecentDocuments:` item and keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Application,
    Window,
    Help,
    Services,
    OpenRecent,
    Plain,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Menu {
    pub title: &'static str,
    pub role: Role,
    pub items: &'static [Item],
}

const fn standard(title: &'static str, key: Option<Key>, selector: &'static str) -> Item {
    Item::Entry {
        title,
        key,
        action: Action::Standard(selector),
    }
}

const fn command(title: &'static str, key: Option<Key>, command: Command) -> Item {
    Item::Entry {
        title,
        key,
        action: Action::Command(command),
    }
}

static SERVICES: Menu = Menu {
    title: "Services",
    role: Role::Services,
    items: &[],
};

static OPEN_RECENT: Menu = Menu {
    title: "Open Recent",
    role: Role::OpenRecent,
    items: &[standard("Clear Menu", None, "clearRecentDocuments:")],
};

static REVERT_TO: Menu = Menu {
    title: "Revert To",
    role: Role::Plain,
    items: &[
        standard("Last Saved Version", None, "revertDocumentToSaved:"),
        standard("Browse All Versions…", None, "browseDocumentVersions:"),
    ],
};

/// The menu bar, left to right.
pub static MENUS: &[Menu] = &[
    Menu {
        title: APP_NAME,
        role: Role::Application,
        items: &[
            standard("About Grind", None, "orderFrontStandardAboutPanel:"),
            Item::Separator,
            Item::Submenu {
                title: "Services",
                menu: &SERVICES,
            },
            Item::Separator,
            standard("Hide Grind", key("h", CMD), "hide:"),
            standard("Hide Others", key("h", OPT_CMD), "hideOtherApplications:"),
            standard("Show All", None, "unhideAllApplications:"),
            Item::Separator,
            standard("Quit Grind", key("q", CMD), "terminate:"),
        ],
    },
    Menu {
        title: "File",
        role: Role::Plain,
        items: &[
            command("New Spreadsheet", key("n", CMD), Command::NewSheet),
            command("New Text Document", key("n", OPT_CMD), Command::NewText),
            standard("Open…", key("o", CMD), "openDocument:"),
            Item::Submenu {
                title: "Open Recent",
                menu: &OPEN_RECENT,
            },
            Item::Separator,
            standard("Close", key("w", CMD), "performClose:"),
            standard("Save…", key("s", CMD), "saveDocument:"),
            standard("Duplicate", key("s", SHIFT_CMD), "duplicateDocument:"),
            standard("Rename…", None, "renameDocument:"),
            standard("Move To…", None, "moveDocument:"),
            Item::Submenu {
                title: "Revert To",
                menu: &REVERT_TO,
            },
        ],
    },
    Menu {
        title: "Edit",
        role: Role::Plain,
        items: &[
            standard("Undo", key("z", CMD), "undo:"),
            standard("Redo", key("z", SHIFT_CMD), "redo:"),
            Item::Separator,
            standard("Cut", key("x", CMD), "cut:"),
            standard("Copy", key("c", CMD), "copy:"),
            standard("Paste", key("v", CMD), "paste:"),
            standard("Delete", None, "delete:"),
            standard("Select All", key("a", CMD), "selectAll:"),
        ],
    },
    Menu {
        title: "View",
        role: Role::Plain,
        items: &[
            standard("Show Toolbar", None, "toggleToolbarShown:"),
            standard("Show Sidebar", key("s", CTRL_CMD), "toggleSidebar:"),
            Item::Separator,
            standard("Enter Full Screen", key("f", CTRL_CMD), "toggleFullScreen:"),
        ],
    },
    Menu {
        title: "Window",
        role: Role::Window,
        items: &[
            standard("Minimize", key("m", CMD), "performMiniaturize:"),
            standard("Zoom", None, "performZoom:"),
            Item::Separator,
            standard("Bring All to Front", None, "arrangeInFront:"),
        ],
    },
    Menu {
        title: "Help",
        role: Role::Help,
        items: &[standard("Grind Help", key("?", CMD), "showHelp:")],
    },
];

/// Every item in `menus` and their submenus, depth first.
#[cfg(test)]
pub fn every_item(menus: &'static [Menu]) -> Vec<&'static Item> {
    fn walk(menu: &'static Menu, out: &mut Vec<&'static Item>) {
        for item in menu.items {
            out.push(item);
            if let Item::Submenu { menu, .. } = item {
                walk(menu, out);
            }
        }
    }
    let mut out = Vec::new();
    for menu in menus {
        walk(menu, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The AppKit selectors this table may name — the responder chain's own vocabulary, spelled
    /// as `NSResponder`, `NSApplication`, `NSWindow`, `NSDocument` and `NSDocumentController`
    /// declare them. A selector outside this list is a typo until proved otherwise: a misspelt one
    /// is a grey item and nothing else, which is why it is a test.
    const KNOWN: &[&str] = &[
        "orderFrontStandardAboutPanel:",
        "hide:",
        "hideOtherApplications:",
        "unhideAllApplications:",
        "terminate:",
        "openDocument:",
        "clearRecentDocuments:",
        "performClose:",
        "saveDocument:",
        "saveDocumentAs:",
        "duplicateDocument:",
        "renameDocument:",
        "moveDocument:",
        "revertDocumentToSaved:",
        "browseDocumentVersions:",
        "undo:",
        "redo:",
        "cut:",
        "copy:",
        "paste:",
        "delete:",
        "selectAll:",
        "performFindPanelAction:",
        "orderFrontFontPanel:",
        "orderFrontColorPanel:",
        "toggleToolbarShown:",
        "runToolbarCustomizationPalette:",
        "toggleSidebar:",
        "toggleFullScreen:",
        "performMiniaturize:",
        "performZoom:",
        "arrangeInFront:",
        "showHelp:",
    ];

    /// The shortcuts the system keeps for itself, and which standard selector may carry each —
    /// `None` when no application item may take it at all.
    fn reserved() -> Vec<(Key, Option<&'static str>)> {
        let k = |key, mods| Key { key, mods };
        vec![
            (k(" ", CMD), None),
            (k("\t", CMD), None),
            (k("`", CMD), None),
            (k("h", CMD), Some("hide:")),
            (k("h", OPT_CMD), Some("hideOtherApplications:")),
            (k("m", CMD), Some("performMiniaturize:")),
            (k("q", CMD), Some("terminate:")),
            (k("q", CTRL_CMD), None),
            (k("\u{1b}", OPT_CMD), None),
            (k("3", SHIFT_CMD), None),
            (k("4", SHIFT_CMD), None),
            (k("5", SHIFT_CMD), None),
            (k(" ", CTRL_CMD), None),
            (k("f", CTRL_CMD), Some("toggleFullScreen:")),
        ]
    }

    fn entries() -> Vec<(&'static str, Option<Key>, Action)> {
        every_item(MENUS)
            .into_iter()
            .filter_map(|item| match item {
                Item::Entry { title, key, action } => Some((*title, *key, *action)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_standard_item_names_a_standard_selector() {
        for (title, _, action) in entries() {
            if let Action::Standard(selector) = action {
                assert!(KNOWN.contains(&selector), "{title}: {selector}");
                assert!(selector.ends_with(':'), "{title}: an action takes a sender");
            }
        }
    }

    #[test]
    fn every_command_is_in_exactly_one_menu() {
        let items = entries();
        for command in Command::ALL {
            let count = items
                .iter()
                .filter(|(_, _, action)| *action == Action::Command(command))
                .count();
            assert_eq!(count, 1, "{command:?}");
            assert_eq!(Command::from_tag(command.tag()), Some(command));
        }
        assert_eq!(Command::from_tag(-1), None);
    }

    #[test]
    fn no_two_items_share_a_key_equivalent() {
        let mut seen: HashMap<Key, &str> = HashMap::new();
        for (title, key, _) in entries() {
            if let Some(key) = key {
                assert!(key.mods.command, "{title}: a menu shortcut uses ⌘");
                assert_eq!(
                    key.key,
                    key.key.to_lowercase(),
                    "{title}: Shift is a modifier"
                );
                if let Some(other) = seen.insert(key, title) {
                    panic!("{title} and {other} both take {}", key.spelled());
                }
            }
        }
    }

    #[test]
    fn nothing_takes_a_shortcut_the_system_reserves() {
        let reserved = reserved();
        for (title, key, action) in entries() {
            let Some(key) = key else { continue };
            if let Some((_, owner)) = reserved.iter().find(|(r, _)| *r == key) {
                assert_eq!(
                    Some(action),
                    owner.map(Action::Standard),
                    "{title} takes {}, which the system keeps",
                    key.spelled()
                );
            }
        }
    }

    /// The shortcuts that are the platform's are the platform's.
    #[test]
    fn the_platforms_shortcuts_mean_what_they_mean_everywhere() {
        let key_of = |action: Action| {
            entries()
                .into_iter()
                .find(|(_, _, a)| *a == action)
                .and_then(|(_, key, _)| key)
                .map(|key| key.spelled())
        };
        for (action, spelled) in [
            (Action::Command(Command::NewSheet), "⌘N"),
            (Action::Standard("openDocument:"), "⌘O"),
            (Action::Standard("saveDocument:"), "⌘S"),
            (Action::Standard("duplicateDocument:"), "⇧⌘S"),
            (Action::Standard("performClose:"), "⌘W"),
            (Action::Standard("undo:"), "⌘Z"),
            (Action::Standard("redo:"), "⇧⌘Z"),
            (Action::Standard("copy:"), "⌘C"),
            (Action::Standard("toggleSidebar:"), "⌃⌘S"),
        ] {
            assert_eq!(key_of(action).as_deref(), Some(spelled), "{action:?}");
        }
    }

    /// An ellipsis says the item asks for more before it acts — and nothing else does.
    #[test]
    fn an_ellipsis_means_the_item_asks_for_more() {
        const ASKS: &[&str] = &[
            "openDocument:",
            "saveDocument:",
            "renameDocument:",
            "moveDocument:",
            "browseDocumentVersions:",
        ];
        for (title, _, action) in entries() {
            let asks = matches!(action, Action::Standard(selector) if ASKS.contains(&selector));
            assert_eq!(title.ends_with('…'), asks, "{title}");
            assert!(
                !title.ends_with("..."),
                "{title}: an ellipsis, not three dots"
            );
            assert!(title.starts_with(char::is_uppercase), "{title}");
        }
    }

    #[test]
    fn the_application_window_and_help_menus_are_where_appkit_expects_them() {
        let roles: Vec<Role> = MENUS.iter().map(|menu| menu.role).collect();
        assert_eq!(roles.first(), Some(&Role::Application));
        assert_eq!(roles.last(), Some(&Role::Help));
        assert_eq!(roles[roles.len() - 2], Role::Window);
        for role in [Role::Application, Role::Window, Role::Help] {
            assert_eq!(roles.iter().filter(|r| **r == role).count(), 1, "{role:?}");
        }
        assert_eq!(MENUS[0].title, APP_NAME);
    }

    /// `NSDocumentController` finds Open Recent by the one item it owns.
    #[test]
    fn open_recent_carries_the_item_the_document_controller_looks_for() {
        assert!(OPEN_RECENT.items.iter().any(|item| matches!(
            item,
            Item::Entry {
                action: Action::Standard("clearRecentDocuments:"),
                ..
            }
        )));
    }

    #[test]
    fn a_key_is_spelled_the_mac_way() {
        assert_eq!(
            Key {
                key: "s",
                mods: SHIFT_CMD
            }
            .spelled(),
            "⇧⌘S"
        );
        assert_eq!(
            Key {
                key: "f",
                mods: CTRL_CMD
            }
            .spelled(),
            "⌃⌘F"
        );
    }
}
