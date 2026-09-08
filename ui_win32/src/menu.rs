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
//! spelling it — a cosmetic difference `win.rs`'s own `context_menu` and [`shortcuts`] (W7) do
//! not depend on, since both read the same `\t`-separated spelling this file already carries.

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
    /// Every function this build implements, as a list to pick from — `sheet/assist.rs`'s
    /// `function_lines`, which is `grind sheet functions --long`'s four columns in a dialog.
    /// Picking one puts `=NAME(` in the cell, so the list is a way of *writing* a formula and
    /// not only of reading about one.
    FunctionList,
    /// The active cell's formula in full — `formula::friendly::explain`: names spelled out,
    /// arguments labelled with the parameter they fill, one per line once a call stops fitting
    /// on one. Read-only, and it never parses back (R1: the document's formula is ODF's).
    ExplainFormula,
    /// Whether the formula bar shows that same reading in place of the text that would be typed
    /// back in — `friendly::explain_inline`, and the `fx` badge says which of the two is up.
    ToggleFriendly,
    SheetAdd,
    SheetRename,
    SheetDelete,
    SheetNext,
    SheetPrevious,
    /// Toggle bold/italic/underline across the selection — the text pane's, and left out of
    /// `Format` on the grid by [`applies_to`] rather than a no-op if it somehow arrives there
    /// anyway (a stale accelerator, say — `do_command`'s per-pane no-ops are the safety net).
    Bold,
    Italic,
    Underline,
    /// Toggle strikethrough — the fourth of `markdown::Emphasis`'s five, drawn on the strip now
    /// alongside Bold/Italic/Underline rather than left menu-and-key-only the way
    /// `grind-text-gtk`'s own bar still has it.
    Strike,
    /// Toggle the monospace family — what the `` `code` `` notation writes, and the strip's
    /// fifth toggle.
    Code,
    /// *Font* — `dialog::choose` over a curated list, `fo:font-family` verbatim.
    PickFamily,
    /// *Font Size* — `dialog::choose` over `grind_text::format::sizes`.
    PickSize,
    /// *Text Colour* — `dialog::choose` over `grind_core::style::PALETTE`.
    PickColor,
    /// *Highlight* — the same picker, writing `fo:background-color` instead.
    PickHighlight,
    /// Every property of the selection's formatting, off at once — the one-shot the toggles can
    /// only approximate, and the strip's own *Clear* button.
    ClearFormatting,
    /// The caret's block becomes a paragraph wearing the `Title` named style —
    /// `ui_text_gtk`'s own `win.title`, mirrored so the two windows both know how to apply and
    /// draw the same two names (`grind_text::NAMED_STYLES`).
    Title,
    Subtitle,
    /// The caret's block becomes a plain paragraph — `App::set_kind`, `BlockKind::Paragraph`.
    /// Left out of the grid's menus, the same way the three toggles above are.
    Paragraph,
    /// The caret's block becomes a heading at this level — `ui_text_gtk`'s own Ctrl+1/2/3, kept
    /// to the same three quick levels here so a key reaches the common case the way it does in
    /// that window. [`Command::BlockKindDialog`] is where the rest of the schema's uncapped
    /// `positiveInteger` (`doc/text-core.md`) and every list depth live instead of a key each.
    Heading1,
    Heading2,
    Heading3,
    /// The text pane's outline dialog — every heading, jump to any of them. Left out of the
    /// grid's menus.
    Outline,
    /// Every block kind this build authors, heading levels past 3 and list items included, as one
    /// dialog rather than a key per depth — the gap `doc/text-shell.md` names for every shell's
    /// own window ("no lists UI") and the first one to close it.
    BlockKindDialog,
    /// Insert a picture at the caret — a file dialog, then `App::insert_image`, into a paragraph
    /// of its own below the caret's block (an empty block is used as it stands). Decoded and
    /// drawn for real (`image.rs`, WIC), matching `ui_text_gtk`'s own Ctrl+Shift+I.
    InsertPicture,
    /// The document as its own projection (D9) — a modal list of its lines, `App::project`'s
    /// text-form, opened on whichever line the pane's own selection or caret projects to. Applies
    /// to both document types, the same as `App::project` reaching both.
    ShowSource,
    /// "Check Document" (D6) — every `App::lint` finding, worst first, each row a jump. Applies
    /// to both document types, the same as `App::lint` reaching both.
    CheckDocument,
    /// `doc/view-modes.md`'s role overlay, on or off. The grid's alone: the text pane has no
    /// `CellRole`.
    ToggleRoles,
    /// `doc/view-modes.md`'s name overlay, on or off — a defined name's range on the grid,
    /// `BlockView::marks`' bookmarks on the text pane (`:names`' equivalent there, since a
    /// bookmark contributes no characters of its own). Both document types.
    ToggleNames,
    /// W7's "key list" — every accelerator this shell answers, read straight off [`MENUS`]'
    /// own labels rather than a second table that could drift from them.
    Shortcuts,
    About,
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
        Command::FunctionList,
        Command::ExplainFormula,
        Command::ToggleFriendly,
        Command::SheetAdd,
        Command::SheetRename,
        Command::SheetDelete,
        Command::SheetNext,
        Command::SheetPrevious,
        Command::Bold,
        Command::Italic,
        Command::Underline,
        Command::Strike,
        Command::Code,
        Command::PickFamily,
        Command::PickSize,
        Command::PickColor,
        Command::PickHighlight,
        Command::ClearFormatting,
        Command::Title,
        Command::Subtitle,
        Command::Paragraph,
        Command::Heading1,
        Command::Heading2,
        Command::Heading3,
        Command::Outline,
        Command::BlockKindDialog,
        Command::InsertPicture,
        Command::ShowSource,
        Command::CheckDocument,
        Command::ToggleRoles,
        Command::ToggleNames,
        Command::Shortcuts,
        Command::About,
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
/// Seven menus and nothing that is not a verb. Format holds the text pane's toggles and block
/// kinds even now that W5b's drawn strip reaches the same four — those items *read and write* a
/// property of the selection, exactly what a strip is for, but a menu they can also start from
/// costs nothing and is where Ctrl+B/I/U were reachable first — and `win.rs`'s `build_menu`
/// leaves the whole menu out of the bar over the grid (`menu_has_items`), since `applies_to`
/// says every one of its items is the text pane's alone. View holds W6's three shared panes: the
/// source, the check, and the two overlays only the grid can draw. What is deliberately absent:
/// anything resembling a ribbon — `doc/sheet-shell.md`'s tab strip was removed for being one,
/// and the argument carries.
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
            Item::Verb {
                command: Command::Outline,
                label: "&Outline…\tCtrl+Shift+O",
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
        items: &[
            Item::Verb {
                command: Command::Recalculate,
                label: "&Recalculate\tF9",
            },
            Item::Separator,
            Item::Verb {
                command: Command::FunctionList,
                label: "&Function List…\tCtrl+Shift+F",
            },
            Item::Verb {
                command: Command::ExplainFormula,
                label: "&Explain Formula…\tCtrl+Shift+E",
            },
        ],
    },
    Menu {
        title: "F&ormat",
        items: &[
            Item::Verb {
                command: Command::Bold,
                label: "&Bold\tCtrl+B",
            },
            Item::Verb {
                command: Command::Italic,
                label: "&Italic\tCtrl+I",
            },
            Item::Verb {
                command: Command::Underline,
                label: "&Underline\tCtrl+U",
            },
            Item::Verb {
                command: Command::Strike,
                label: "S&trikethrough\tCtrl+Shift+X",
            },
            Item::Verb {
                command: Command::Code,
                label: "&Monospace\tCtrl+Shift+M",
            },
            Item::Separator,
            Item::Verb {
                command: Command::PickFamily,
                label: "&Font…",
            },
            Item::Verb {
                command: Command::PickSize,
                label: "Si&ze…",
            },
            Item::Verb {
                command: Command::PickColor,
                label: "Text &Colour…",
            },
            Item::Verb {
                command: Command::PickHighlight,
                label: "&Highlight…",
            },
            Item::Verb {
                command: Command::ClearFormatting,
                label: "C&lear Formatting",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Title,
                label: "Titl&e",
            },
            Item::Verb {
                command: Command::Subtitle,
                label: "&Subtitle",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Paragraph,
                label: "&Paragraph\tCtrl+0",
            },
            Item::Verb {
                command: Command::Heading1,
                label: "Heading &1\tCtrl+1",
            },
            Item::Verb {
                command: Command::Heading2,
                label: "Heading &2\tCtrl+2",
            },
            Item::Verb {
                command: Command::Heading3,
                label: "Heading &3\tCtrl+3",
            },
            Item::Separator,
            Item::Verb {
                command: Command::BlockKindDialog,
                label: "Block &Kind…\tCtrl+Shift+K",
            },
            Item::Verb {
                command: Command::InsertPicture,
                label: "I&nsert Picture…\tCtrl+Shift+I",
            },
        ],
    },
    Menu {
        title: "&View",
        items: &[
            Item::Verb {
                command: Command::ShowSource,
                label: "Show &Source\tCtrl+Shift+U",
            },
            Item::Verb {
                command: Command::CheckDocument,
                label: "&Check Document\tF8",
            },
            Item::Separator,
            Item::Verb {
                command: Command::ToggleRoles,
                label: "Cell R&oles",
            },
            Item::Verb {
                command: Command::ToggleNames,
                label: "&Names",
            },
            Item::Verb {
                command: Command::ToggleFriendly,
                label: "&Friendly Formulas",
            },
        ],
    },
    Menu {
        title: "&Help",
        items: &[
            Item::Verb {
                command: Command::Shortcuts,
                label: "&Keyboard Shortcuts",
            },
            Item::Verb {
                command: Command::About,
                label: "&About Grind",
            },
        ],
    },
];

/// The label a command shows in whichever menu names it — the one copy of that string, so a
/// context menu (W7) can put a verb next to its own spelling rather than a second one written
/// down beside it. Carries its `\t`-separated accelerator too, since a context menu benefits
/// from the reminder exactly as the menu bar's own does.
pub fn label_for(command: Command) -> Option<&'static str> {
    MENUS
        .iter()
        .flat_map(|menu| menu.items)
        .find_map(|item| match item {
            Item::Verb { command: c, label } if *c == command => Some(*label),
            _ => None,
        })
}

/// Every command that carries a `\t`-separated accelerator, as one line each — W7's "key list",
/// the answer to `gtk::ShortcutsWindow` this shell can build with no resources and no dialog
/// template: a plain read of the labels every menu item already has, so a shortcut shown here and
/// a shortcut shown on the bar can never say two different things about the same command.
///
/// Read out of [`MENUS`] in the bar's own order rather than [`Command::ALL`]'s, which is the
/// order a person tabbing through the menus meets them in — the more useful one for a reference
/// list — and the mnemonic's `&` is stripped, since a keyboard shortcuts window is not itself
/// navigated by one.
pub fn shortcuts() -> Vec<String> {
    MENUS
        .iter()
        .flat_map(|menu| menu.items)
        .filter_map(|item| match item {
            Item::Verb { label, .. } => label.split_once('\t'),
            Item::Separator => None,
        })
        .map(|(name, key)| format!("{} — {key}", name.replace('&', "")))
        .collect()
}

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
        (Key::Char('B'), true, false) => Some(Command::Bold),
        (Key::Char('I'), true, false) => Some(Command::Italic),
        (Key::Char('U'), true, false) => Some(Command::Underline),
        (Key::Char('X'), true, true) => Some(Command::Strike),
        (Key::Char('M'), true, true) => Some(Command::Code),
        (Key::Char('0'), true, false) => Some(Command::Paragraph),
        (Key::Char('1'), true, false) => Some(Command::Heading1),
        (Key::Char('2'), true, false) => Some(Command::Heading2),
        (Key::Char('3'), true, false) => Some(Command::Heading3),
        (Key::Char('O'), true, true) => Some(Command::Outline),
        (Key::Char('K'), true, true) => Some(Command::BlockKindDialog),
        (Key::Char('I'), true, true) => Some(Command::InsertPicture),
        (Key::Char('U'), true, true) => Some(Command::ShowSource),
        (Key::Char('F'), true, true) => Some(Command::FunctionList),
        (Key::Char('E'), true, true) => Some(Command::ExplainFormula),
        (Key::F8, false, false) => Some(Command::CheckDocument),
        _ => None,
    }
}

/// Whether this verb means anything for a document of this kind — the half of "the menus are
/// finished" that can be answered with no window at all. `win.rs`'s `build_menu` is the caller,
/// through [`items_for`]/[`menu_has_items`]: a `false` here is why `Recalculate` is missing from
/// the bar over a text document rather than shown and unclickable. State *within* a kind — Undo
/// with nothing to undo, Paste with an empty clipboard — is a separate question this does not
/// answer, and W7 still owes it. A command silent about both kinds would be a command with
/// nowhere to act, which is a different bug from the one this answers — that one is
/// `Command::ALL` matching `MENUS`, checked below.
///
/// `Presentation` answers `false` to everything: this shell opens neither kind of pane for one
/// (`opened` refuses the document before a `Pane` exists), so there is nothing it could mean.
pub fn applies_to(command: Command, kind: grind_core::DocumentKind) -> bool {
    use grind_core::DocumentKind::{Presentation, Spreadsheet, Text};
    match command {
        Command::Recalculate
        | Command::SheetAdd
        | Command::SheetRename
        | Command::SheetDelete
        | Command::SheetNext
        | Command::SheetPrevious
        // The three formula verbs are the grid's for the plainest of reasons: a text document
        // has no formulas, so a function list in one would offer things it cannot hold.
        | Command::FunctionList
        | Command::ExplainFormula
        | Command::ToggleFriendly
        // `doc/view-modes.md`'s role overlay is `CellRole`, the grid's own vocabulary; the text
        // pane has no per-character role.
        | Command::ToggleRoles => matches!(kind, Spreadsheet),
        Command::Bold
        | Command::Italic
        | Command::Underline
        | Command::Strike
        | Command::Code
        | Command::PickFamily
        | Command::PickSize
        | Command::PickColor
        | Command::PickHighlight
        | Command::ClearFormatting
        | Command::Title
        | Command::Subtitle
        | Command::Paragraph
        | Command::Heading1
        | Command::Heading2
        | Command::Heading3
        | Command::Outline
        | Command::BlockKindDialog
        | Command::InsertPicture => matches!(kind, Text),
        Command::New
        | Command::Open
        | Command::Save
        | Command::SaveAs
        | Command::Exit
        | Command::Undo
        | Command::Redo
        | Command::Cut
        | Command::Copy
        | Command::Paste
        | Command::ClearCells
        | Command::GoTo
        // `App::project` and `App::lint` both reach either document type, so the two W6 verbs
        // that are not overlays follow the universal group rather than either specific one — and
        // so does the name overlay, since a bookmark is the text pane's own name anchor.
        | Command::ShowSource
        | Command::CheckDocument
        | Command::ToggleNames
        | Command::Shortcuts
        | Command::About => !matches!(kind, Presentation),
    }
}

/// One menu's items for a document of this kind — a verb this pane has no answer for is
/// **omitted**, not greyed, and a separator left with nothing either side of it (because
/// everything around it was dropped) goes with it.
///
/// This replaced greying: a menu bar with every verb visible and half of them unclickable read
/// as a text pane that still thought it was a grid, since the sheet's own six verbs (`Sheet`'s
/// whole menu, `Recalculate`, `Cell Roles`) so outnumbered the universal ones that the &View
/// and &Sheet menus looked identical open on either pane. Two ends now: `Format` on the grid and
/// `Sheet`/`Data` on the text pane can end up with nothing in them at all, which is
/// [`menu_has_items`]'s question, asked before a menu is put in the bar at all.
pub fn items_for(menu: &Menu, kind: grind_core::DocumentKind) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::with_capacity(menu.items.len());
    for item in menu.items {
        match item {
            Item::Verb { command, .. } if !applies_to(*command, kind) => continue,
            Item::Separator if matches!(items.last(), None | Some(Item::Separator)) => continue,
            other => items.push(*other),
        }
    }
    if matches!(items.last(), Some(Item::Separator)) {
        items.pop();
    }
    items
}

/// Whether a menu has anything left to show for a document of this kind — a menu whose every
/// verb [`items_for`] dropped contributes nothing to the bar rather than an empty popup with
/// only its title.
pub fn menu_has_items(menu: &Menu, kind: grind_core::DocumentKind) -> bool {
    items_for(menu, kind)
        .iter()
        .any(|item| matches!(item, Item::Verb { .. }))
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
            (Key::Char('B'), ctrl, Command::Bold),
            (Key::Char('I'), ctrl, Command::Italic),
            (Key::Char('U'), ctrl, Command::Underline),
            (Key::Char('0'), ctrl, Command::Paragraph),
            (Key::Char('1'), ctrl, Command::Heading1),
            (Key::Char('2'), ctrl, Command::Heading2),
            (Key::Char('3'), ctrl, Command::Heading3),
            (Key::Char('O'), ctrl_shift, Command::Outline),
            (Key::Char('K'), ctrl_shift, Command::BlockKindDialog),
            (Key::Char('U'), ctrl_shift, Command::ShowSource),
            (Key::Char('F'), ctrl_shift, Command::FunctionList),
            (Key::Char('E'), ctrl_shift, Command::ExplainFormula),
            (Key::F8, Mods::default(), Command::CheckDocument),
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

    /// A verb that means nothing anywhere would be dead weight in the menu; every one must apply
    /// to at least the sheet or the text pane, since those are the only two kinds this shell ever
    /// shows a `Pane` for.
    #[test]
    fn every_command_applies_to_at_least_one_document_kind() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in Command::ALL {
            assert!(
                applies_to(*command, Spreadsheet) || applies_to(*command, Text),
                "{command:?} applies to neither document type"
            );
        }
    }

    /// Nothing applies to a presentation: this shell never holds a `Pane` for one, so a menu
    /// item that thought otherwise would be untestable by construction.
    #[test]
    fn nothing_applies_to_a_presentation() {
        for command in Command::ALL {
            assert!(!applies_to(
                *command,
                grind_core::DocumentKind::Presentation
            ));
        }
    }

    /// The Format menu's toggles, pickers and Clear, plus the outline dialog, are the text
    /// pane's and only the text pane's — the sheet has no `char_style` and no headings to grey
    /// them into meaning.
    #[test]
    fn formatting_and_the_outline_are_the_text_panes_alone() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in [
            Command::Bold,
            Command::Italic,
            Command::Underline,
            Command::Strike,
            Command::Code,
            Command::PickFamily,
            Command::PickSize,
            Command::PickColor,
            Command::PickHighlight,
            Command::ClearFormatting,
            Command::Title,
            Command::Subtitle,
            Command::Paragraph,
            Command::Heading1,
            Command::Heading2,
            Command::Heading3,
            Command::Outline,
            Command::BlockKindDialog,
            Command::InsertPicture,
        ] {
            assert!(applies_to(command, Text), "{command:?}");
            assert!(!applies_to(command, Spreadsheet), "{command:?}");
        }
    }

    /// Recalculate, the four sheet verbs and the three formula verbs are the grid's alone — the
    /// text pane has no sheets and no formulas.
    #[test]
    fn sheet_verbs_are_the_grids_alone() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in [
            Command::Recalculate,
            Command::FunctionList,
            Command::ExplainFormula,
            Command::ToggleFriendly,
            Command::SheetAdd,
            Command::SheetRename,
            Command::SheetDelete,
            Command::SheetNext,
            Command::SheetPrevious,
        ] {
            assert!(applies_to(command, Spreadsheet), "{command:?}");
            assert!(!applies_to(command, Text), "{command:?}");
        }
    }

    /// The role overlay is the grid's alone — the text pane has no per-character `CellRole`.
    #[test]
    fn the_role_overlay_is_the_grids_alone() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        assert!(applies_to(Command::ToggleRoles, Spreadsheet));
        assert!(!applies_to(Command::ToggleRoles, Text));
    }

    /// The name overlay reaches both: a defined name on the grid, a bookmark on the text pane.
    #[test]
    fn the_name_overlay_reaches_both_document_types() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        assert!(applies_to(Command::ToggleNames, Spreadsheet));
        assert!(applies_to(Command::ToggleNames, Text));
    }

    /// The source and the check are W6's two verbs that are not overlays, and `App::project` /
    /// `App::lint` reach both document types, so both panes get them.
    #[test]
    fn the_source_and_the_check_reach_both_document_types() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in [Command::ShowSource, Command::CheckDocument] {
            assert!(applies_to(command, Spreadsheet), "{command:?}");
            assert!(applies_to(command, Text), "{command:?}");
        }
    }

    /// Every command in a menu has a label a context menu (W7) can borrow, and it is the same
    /// string the bar itself shows — there is no second copy for `label_for` to disagree with.
    #[test]
    fn every_menu_command_has_a_label_and_it_is_the_bars_own() {
        for menu in MENUS {
            for item in menu.items {
                let Item::Verb { command, label } = item else {
                    continue;
                };
                assert_eq!(label_for(*command), Some(*label), "{command:?}");
            }
        }
    }

    /// A command in no menu — there are none, `every_command_is_reachable_from_exactly_one_menu_item`
    /// already says so — would have no label; this is the same answer from the other function.
    #[test]
    fn every_command_has_a_label() {
        for command in Command::ALL {
            assert!(label_for(*command).is_some(), "{command:?}");
        }
    }

    /// The shortcuts list is one line per command that carries an accelerator, name and key both
    /// present and the mnemonic's `&` gone — a keyboard shortcuts window is not itself navigated
    /// by one.
    #[test]
    fn the_shortcuts_list_names_every_accelerator_and_drops_the_mnemonic() {
        let rows = shortcuts();
        assert!(!rows.is_empty());
        for row in &rows {
            assert!(!row.contains('&'), "{row}");
            assert!(row.contains(" — "), "{row}");
        }
        assert!(
            rows.iter()
                .any(|row| row.contains("Save") && row.contains("Ctrl+S"))
        );
        // A command with no accelerator — `SheetAdd` has none — contributes no row at all,
        // rather than one with an empty key.
        assert!(
            !rows.iter().any(|row| row.starts_with("Add")),
            "a menu item with no accelerator is not a shortcut"
        );
    }

    fn menu(title: &str) -> &'static Menu {
        MENUS
            .iter()
            .find(|menu| menu.title == title)
            .unwrap_or_else(|| panic!("no menu titled {title}"))
    }

    /// `Sheet` and `Data` are the grid's alone — every one of their verbs answers only to
    /// `Spreadsheet` — so a text document leaves both out of the bar rather than showing them
    /// with nothing clickable in them.
    #[test]
    fn sheet_and_data_vanish_on_the_text_pane() {
        use grind_core::DocumentKind::Text;
        assert!(!menu_has_items(menu("&Sheet"), Text));
        assert!(!menu_has_items(menu("&Data"), Text));
        assert!(items_for(menu("&Sheet"), Text).is_empty());
        assert!(items_for(menu("&Data"), Text).is_empty());
    }

    /// `Format` is the text pane's alone, for the same reason in reverse.
    #[test]
    fn format_vanishes_on_the_grid() {
        use grind_core::DocumentKind::Spreadsheet;
        assert!(!menu_has_items(menu("F&ormat"), Spreadsheet));
        assert!(items_for(menu("F&ormat"), Spreadsheet).is_empty());
    }

    /// `Edit` mixes universal verbs with `Outline`, which is the text pane's alone — so the
    /// menu survives on the grid, minus that one item, rather than vanishing or keeping a verb
    /// with nothing to do.
    #[test]
    fn edit_loses_only_outline_on_the_grid() {
        use grind_core::DocumentKind::Spreadsheet;
        assert!(menu_has_items(menu("&Edit"), Spreadsheet));
        let items = items_for(menu("&Edit"), Spreadsheet);
        assert!(!items.iter().any(|item| matches!(
            item,
            Item::Verb {
                command: Command::Outline,
                ..
            }
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            Item::Verb {
                command: Command::GoTo,
                ..
            }
        )));
    }

    /// `View` mixes a grid-only toggle (`Cell Roles`) with three universal verbs, so it survives
    /// on the text pane minus that one item — the same shape `Edit` has on the grid.
    #[test]
    fn view_loses_only_cell_roles_on_the_text_pane() {
        use grind_core::DocumentKind::Text;
        assert!(menu_has_items(menu("&View"), Text));
        let items = items_for(menu("&View"), Text);
        assert!(!items.iter().any(|item| matches!(
            item,
            Item::Verb {
                command: Command::ToggleRoles,
                ..
            }
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            Item::Verb {
                command: Command::ToggleNames,
                ..
            }
        )));
    }

    /// Never a leading, trailing, or doubled separator — the visible cost of filtering a menu's
    /// items by hand, and the reason `items_for` cleans them up rather than leaving them for
    /// `AppendMenuW` to draw as dead space.
    #[test]
    fn filtering_never_leaves_a_stray_separator() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for menu in MENUS {
            for kind in [Spreadsheet, Text] {
                let items = items_for(menu, kind);
                assert_ne!(
                    items.first(),
                    Some(&Item::Separator),
                    "{}: {kind:?}",
                    menu.title
                );
                assert_ne!(
                    items.last(),
                    Some(&Item::Separator),
                    "{}: {kind:?}",
                    menu.title
                );
                assert!(
                    !items
                        .windows(2)
                        .any(|pair| pair == [Item::Separator, Item::Separator]),
                    "{}: {kind:?} has two separators in a row",
                    menu.title
                );
            }
        }
    }

    /// A menu that keeps every one of its items for a kind is unchanged, order included — the
    /// baseline `filtering_never_leaves_a_stray_separator` and the vanish/survive tests above
    /// all lean on.
    #[test]
    fn a_menu_with_nothing_to_drop_is_returned_whole() {
        use grind_core::DocumentKind::Text;
        assert_eq!(items_for(menu("&File"), Text), menu("&File").items.to_vec());
    }
}
