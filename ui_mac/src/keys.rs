// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a key means on the grid — **by selector, not by key code** (decision 7).
//!
//! A view hands `keyDown:` to `interpretKeyEvents:`, and AppKit answers with what the key
//! *means*: `insertText:` for text, or `doCommandBySelector:` with `moveLeft:`,
//! `moveToBeginningOfDocument:`, `deleteBackward:` and the rest of
//! `NSStandardKeyBindingResponding`. That table is the user's — it honours
//! `~/Library/KeyBindings/DefaultKeyBinding.dict` and the platform's own idea of what ⌘← and ⇧⌥↓
//! do — so this file is `ui_win32`'s virtual-key table in the Mac's vocabulary: a portable map
//! from a selector's name to a grid action, over `grind_sheet::nav`'s motions.
//!
//! The Mac spells a spreadsheet's keys like this, which is Excel for Mac's and Numbers' reading
//! of the platform's bindings:
//!
//! | Key | Selector | Here |
//! |---|---|---|
//! | ← → ↑ ↓ | `moveLeft:` … | one cell |
//! | ⌘← ⌘→ | `moveToLeftEndOfLine:` / `moveToBeginningOfLine:` … | the data's edge along the row |
//! | ⌘↑ ⌘↓ | `moveToBeginningOfDocument:` / `moveToEndOfDocument:` | the data's edge along the column |
//! | Home / End (fn←, fn→) | `scrollToBeginningOfDocument:` / `scrollToEndOfDocument:` | A1, and the last used cell |
//! | Page Up / Down | `scrollPageUp:` / `pageUp:` … | a screenful |
//! | ⇧ with any of those | `…AndModifySelection:` | the same, extending |
//! | Return, ⇧Return | `insertNewline:` … | down, or up |
//! | Tab, ⇧Tab | `insertTab:`, `insertBacktab:` | right, or left |
//! | Esc | `cancelOperation:` | the selection back to its active cell |
//!
//! Every selector `StandardKeyBinding.dict` binds is either answered here or named in
//! [`UNANSWERED`] with the reason; the runner holds that list to the system's own file.

use grind_sheet::nav::{Dir, Motion};

/// What a selector does on the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridAction {
    Move {
        motion: Motion,
        extend: bool,
    },
    /// The selection back to its active cell — Esc.
    Collapse,
    /// Empty the selected cells — Delete and Forward Delete. Editing, and so M4's.
    Clear,
}

const fn to(motion: Motion) -> GridAction {
    GridAction::Move {
        motion,
        extend: false,
    }
}

const fn extending(motion: Motion) -> GridAction {
    GridAction::Move {
        motion,
        extend: true,
    }
}

/// Every selector the grid answers, and what it does.
pub const GRID: &[(&str, GridAction)] = &[
    ("moveLeft:", to(Motion::By(Dir::Left))),
    ("moveRight:", to(Motion::By(Dir::Right))),
    ("moveUp:", to(Motion::By(Dir::Up))),
    ("moveDown:", to(Motion::By(Dir::Down))),
    (
        "moveLeftAndModifySelection:",
        extending(Motion::By(Dir::Left)),
    ),
    (
        "moveRightAndModifySelection:",
        extending(Motion::By(Dir::Right)),
    ),
    ("moveUpAndModifySelection:", extending(Motion::By(Dir::Up))),
    (
        "moveDownAndModifySelection:",
        extending(Motion::By(Dir::Down)),
    ),
    // ⌘-arrows: the edge of the data, the Mac spelling of a PC's Ctrl-arrows.
    ("moveToLeftEndOfLine:", to(Motion::Edge(Dir::Left))),
    ("moveToRightEndOfLine:", to(Motion::Edge(Dir::Right))),
    ("moveToBeginningOfLine:", to(Motion::Edge(Dir::Left))),
    ("moveToEndOfLine:", to(Motion::Edge(Dir::Right))),
    ("moveToBeginningOfDocument:", to(Motion::Edge(Dir::Up))),
    ("moveToEndOfDocument:", to(Motion::Edge(Dir::Down))),
    (
        "moveToLeftEndOfLineAndModifySelection:",
        extending(Motion::Edge(Dir::Left)),
    ),
    (
        "moveToRightEndOfLineAndModifySelection:",
        extending(Motion::Edge(Dir::Right)),
    ),
    (
        "moveToBeginningOfLineAndModifySelection:",
        extending(Motion::Edge(Dir::Left)),
    ),
    (
        "moveToEndOfLineAndModifySelection:",
        extending(Motion::Edge(Dir::Right)),
    ),
    (
        "moveToBeginningOfDocumentAndModifySelection:",
        extending(Motion::Edge(Dir::Up)),
    ),
    (
        "moveToEndOfDocumentAndModifySelection:",
        extending(Motion::Edge(Dir::Down)),
    ),
    // Home and End.
    ("scrollToBeginningOfDocument:", to(Motion::SheetStart)),
    ("scrollToEndOfDocument:", to(Motion::SheetEnd)),
    ("moveToBeginningOfParagraph:", to(Motion::RowStart)),
    ("moveToEndOfParagraph:", to(Motion::RowEnd)),
    // Page Up and Page Down, in both of the spellings the bindings use.
    ("scrollPageUp:", to(Motion::Page(Dir::Up))),
    ("scrollPageDown:", to(Motion::Page(Dir::Down))),
    ("pageUp:", to(Motion::Page(Dir::Up))),
    ("pageDown:", to(Motion::Page(Dir::Down))),
    (
        "pageUpAndModifySelection:",
        extending(Motion::Page(Dir::Up)),
    ),
    (
        "pageDownAndModifySelection:",
        extending(Motion::Page(Dir::Down)),
    ),
    // Return and Tab move on, as in every spreadsheet.
    ("insertNewline:", to(Motion::By(Dir::Down))),
    ("insertNewlineIgnoringFieldEditor:", to(Motion::By(Dir::Up))),
    ("insertTab:", to(Motion::By(Dir::Right))),
    ("insertBacktab:", to(Motion::By(Dir::Left))),
    ("cancelOperation:", GridAction::Collapse),
    ("deleteBackward:", GridAction::Clear),
    ("deleteForward:", GridAction::Clear),
];

/// Selectors the standard bindings produce that the grid **deliberately does not answer**, and
/// why — so a key that does nothing is a decision written down rather than an omission. Read by
/// the tests here and, on the runner, against the system's own `StandardKeyBinding.dict`.
#[cfg_attr(not(test), allow(dead_code))]
pub const UNANSWERED: &[(&str, &str)] = &[
    (
        "moveWordLeft:",
        "⌥← has no cell meaning; it belongs to the cell editor",
    ),
    ("moveWordRight:", "⌥→, likewise"),
    ("moveWordLeftAndModifySelection:", "⇧⌥←, likewise"),
    ("moveWordRightAndModifySelection:", "⇧⌥→, likewise"),
    (
        "moveParagraphForwardAndModifySelection:",
        "⇧⌥↓ — a paragraph is not a grid unit",
    ),
    ("moveParagraphBackwardAndModifySelection:", "⇧⌥↑, likewise"),
    ("deleteWordBackward:", "⌥⌫ is the cell editor's"),
    ("deleteWordForward:", "⌥⌦ is the cell editor's"),
    ("deleteToBeginningOfLine:", "⌘⌫ is the cell editor's"),
    ("deleteToEndOfParagraph:", "⌃K is the cell editor's"),
    ("yank:", "⌃Y is the cell editor's"),
    ("transpose:", "⌃T is the cell editor's"),
    (
        "centerSelectionInVisibleArea:",
        "⌃L; the grid scrolls to the active cell already",
    ),
    (
        "complete:",
        "Esc's other binding; completion is the formula editor's (M8)",
    ),
];

/// The grid action a selector names, or `None` when the grid does not answer it.
pub fn grid_action(selector: &str) -> Option<GridAction> {
    GRID.iter()
        .find(|(name, _)| *name == selector)
        .map(|(_, action)| *action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_arrows_move_and_shift_extends() {
        assert_eq!(grid_action("moveRight:"), Some(to(Motion::By(Dir::Right))));
        assert_eq!(
            grid_action("moveDownAndModifySelection:"),
            Some(extending(Motion::By(Dir::Down)))
        );
        assert_eq!(
            grid_action("moveToEndOfDocument:"),
            Some(to(Motion::Edge(Dir::Down)))
        );
        assert_eq!(
            grid_action("insertNewline:"),
            Some(to(Motion::By(Dir::Down)))
        );
        assert_eq!(grid_action("cancelOperation:"), Some(GridAction::Collapse));
        assert_eq!(grid_action("noSuchSelector:"), None);
    }

    /// A selector is answered or named as unanswered, never both, and never twice.
    #[test]
    fn every_selector_is_answered_or_named_once() {
        let mut seen = HashSet::new();
        for name in GRID
            .iter()
            .map(|(name, _)| *name)
            .chain(UNANSWERED.iter().map(|(n, _)| *n))
        {
            assert!(name.ends_with(':'), "{name}: a selector takes a sender");
            assert!(seen.insert(name), "{name} is listed twice");
        }
        for (name, why) in UNANSWERED {
            assert!(!why.is_empty(), "{name} needs its reason");
            assert_eq!(grid_action(name), None);
        }
    }

    /// Every extending selector is its plain one's twin, so Shift never changes where a key goes,
    /// only whether the anchor stays.
    #[test]
    fn a_modify_selection_twin_goes_where_its_plain_selector_goes() {
        for (name, action) in GRID {
            let Some(plain) = name.strip_suffix("AndModifySelection:") else {
                continue;
            };
            let GridAction::Move { motion, extend } = action else {
                panic!("{name} extends a motion");
            };
            assert!(extend, "{name}");
            assert_eq!(
                grid_action(&format!("{plain}:")),
                Some(to(*motion)),
                "{name} and {plain}: disagree"
            );
        }
    }
}
