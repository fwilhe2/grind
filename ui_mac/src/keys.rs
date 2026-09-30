// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a key means on the grid and on the page — **by selector, not by key code** (decision 7).
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
//!
//! The page (M6) reads the same bindings as every Mac text view does — which is the point of
//! taking selectors: ⌥→ goes to the end of a word, ⌘← to the start of the visual line, ⌃A and ⌃E
//! to the ends of the paragraph, ⌃K kills to its end, and a `DefaultKeyBinding.dict` a user wrote
//! for TextEdit works here unchanged. [`PAGE`] and [`PAGE_UNANSWERED`] are that table.

use grind_sheet::nav::{Dir, Motion};

use crate::text::state::{Motion as Caret, Unit};

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

/// What a selector does on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageAction {
    Move {
        motion: Caret,
        extend: bool,
    },
    /// Esc: the selection collapsed to its caret.
    Collapse,
    Erase {
        unit: Unit,
        forward: bool,
    },
    /// Return: one block becomes two.
    Split,
    /// A line break inside the block — `text:line-break` — rather than a new block.
    LineBreak,
    /// Tab and ⇧Tab: a list nested or un-nested where there is one, a tab where there is not.
    Tab {
        back: bool,
    },
    /// ⌥Tab: a tab character, whatever the block is.
    LiteralTab,
    /// The view scrolled without the caret moving — fn↑, fn↓, Home and End on a Mac.
    Scroll(Scroll),
    /// ⌃L: the caret's line brought to the middle of the view.
    Center,
}

/// Where a [`PageAction::Scroll`] goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    PageUp,
    PageDown,
    Top,
    Bottom,
}

const fn moving(motion: Caret) -> PageAction {
    PageAction::Move {
        motion,
        extend: false,
    }
}

const fn selecting(motion: Caret) -> PageAction {
    PageAction::Move {
        motion,
        extend: true,
    }
}

const fn erasing(unit: Unit, forward: bool) -> PageAction {
    PageAction::Erase { unit, forward }
}

/// Every selector the page answers, and what it does.
pub const PAGE: &[(&str, PageAction)] = &[
    ("moveLeft:", moving(Caret::Char(-1))),
    ("moveRight:", moving(Caret::Char(1))),
    ("moveBackward:", moving(Caret::Char(-1))),
    ("moveForward:", moving(Caret::Char(1))),
    ("moveUp:", moving(Caret::Line(-1))),
    ("moveDown:", moving(Caret::Line(1))),
    ("moveWordLeft:", moving(Caret::Word(-1))),
    ("moveWordRight:", moving(Caret::Word(1))),
    ("moveWordBackward:", moving(Caret::Word(-1))),
    ("moveWordForward:", moving(Caret::Word(1))),
    ("moveToBeginningOfLine:", moving(Caret::LineStart)),
    ("moveToEndOfLine:", moving(Caret::LineEnd)),
    ("moveToLeftEndOfLine:", moving(Caret::LineStart)),
    ("moveToRightEndOfLine:", moving(Caret::LineEnd)),
    ("moveToBeginningOfParagraph:", moving(Caret::ParagraphStart)),
    ("moveToEndOfParagraph:", moving(Caret::ParagraphEnd)),
    ("moveToBeginningOfDocument:", moving(Caret::DocStart)),
    ("moveToEndOfDocument:", moving(Caret::DocEnd)),
    ("pageUp:", moving(Caret::Page(-1))),
    ("pageDown:", moving(Caret::Page(1))),
    ("moveLeftAndModifySelection:", selecting(Caret::Char(-1))),
    ("moveRightAndModifySelection:", selecting(Caret::Char(1))),
    (
        "moveBackwardAndModifySelection:",
        selecting(Caret::Char(-1)),
    ),
    ("moveForwardAndModifySelection:", selecting(Caret::Char(1))),
    ("moveUpAndModifySelection:", selecting(Caret::Line(-1))),
    ("moveDownAndModifySelection:", selecting(Caret::Line(1))),
    (
        "moveWordLeftAndModifySelection:",
        selecting(Caret::Word(-1)),
    ),
    (
        "moveWordRightAndModifySelection:",
        selecting(Caret::Word(1)),
    ),
    (
        "moveWordBackwardAndModifySelection:",
        selecting(Caret::Word(-1)),
    ),
    (
        "moveWordForwardAndModifySelection:",
        selecting(Caret::Word(1)),
    ),
    (
        "moveToBeginningOfLineAndModifySelection:",
        selecting(Caret::LineStart),
    ),
    (
        "moveToEndOfLineAndModifySelection:",
        selecting(Caret::LineEnd),
    ),
    (
        "moveToLeftEndOfLineAndModifySelection:",
        selecting(Caret::LineStart),
    ),
    (
        "moveToRightEndOfLineAndModifySelection:",
        selecting(Caret::LineEnd),
    ),
    (
        "moveToBeginningOfParagraphAndModifySelection:",
        selecting(Caret::ParagraphStart),
    ),
    (
        "moveToEndOfParagraphAndModifySelection:",
        selecting(Caret::ParagraphEnd),
    ),
    (
        "moveToBeginningOfDocumentAndModifySelection:",
        selecting(Caret::DocStart),
    ),
    (
        "moveToEndOfDocumentAndModifySelection:",
        selecting(Caret::DocEnd),
    ),
    ("pageUpAndModifySelection:", selecting(Caret::Page(-1))),
    ("pageDownAndModifySelection:", selecting(Caret::Page(1))),
    // ⇧⌥↑ and ⇧⌥↓, which only exist extending: a paragraph at a time, crossing into the next.
    (
        "moveParagraphBackwardAndModifySelection:",
        selecting(Caret::ParagraphBack),
    ),
    (
        "moveParagraphForwardAndModifySelection:",
        selecting(Caret::ParagraphOn),
    ),
    ("deleteBackward:", erasing(Unit::Char, false)),
    ("deleteForward:", erasing(Unit::Char, true)),
    (
        "deleteBackwardByDecomposingPreviousCharacter:",
        erasing(Unit::Char, false),
    ),
    ("deleteWordBackward:", erasing(Unit::Word, false)),
    ("deleteWordForward:", erasing(Unit::Word, true)),
    ("deleteToBeginningOfLine:", erasing(Unit::Line, false)),
    ("deleteToEndOfLine:", erasing(Unit::Line, true)),
    (
        "deleteToBeginningOfParagraph:",
        erasing(Unit::Paragraph, false),
    ),
    ("deleteToEndOfParagraph:", erasing(Unit::Paragraph, true)),
    ("insertNewline:", PageAction::Split),
    ("insertParagraphSeparator:", PageAction::Split),
    ("insertLineBreak:", PageAction::LineBreak),
    ("insertNewlineIgnoringFieldEditor:", PageAction::LineBreak),
    ("insertTab:", PageAction::Tab { back: false }),
    ("insertBacktab:", PageAction::Tab { back: true }),
    ("insertTabIgnoringFieldEditor:", PageAction::LiteralTab),
    ("cancelOperation:", PageAction::Collapse),
    ("scrollPageUp:", PageAction::Scroll(Scroll::PageUp)),
    ("scrollPageDown:", PageAction::Scroll(Scroll::PageDown)),
    (
        "scrollToBeginningOfDocument:",
        PageAction::Scroll(Scroll::Top),
    ),
    ("scrollToEndOfDocument:", PageAction::Scroll(Scroll::Bottom)),
    ("centerSelectionInVisibleArea:", PageAction::Center),
];

/// Selectors the standard bindings produce that the page deliberately does not answer, and why.
#[cfg_attr(not(test), allow(dead_code))]
pub const PAGE_UNANSWERED: &[(&str, &str)] = &[
    (
        "yank:",
        "⌃Y pastes the kill ring, and ⌃K here erases rather than kills: the pasteboard is the one clipboard this page has",
    ),
    (
        "transpose:",
        "⌃T swaps two characters; an edit with no verb in the core yet",
    ),
    (
        "complete:",
        "Esc's other binding; there is nothing to complete in prose",
    ),
    (
        "capitalizeWord:",
        "a case change is a replace over a span, which the core has only by search",
    ),
    ("uppercaseWord:", "likewise"),
    ("lowercaseWord:", "likewise"),
];

/// The page action a selector names, or `None` when the page does not answer it.
pub fn page_action(selector: &str) -> Option<PageAction> {
    PAGE.iter()
        .find(|(name, _)| *name == selector)
        .map(|(_, action)| *action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_page_reads_the_keys_every_mac_text_view_does() {
        assert_eq!(page_action("moveWordRight:"), Some(moving(Caret::Word(1))));
        assert_eq!(
            page_action("moveToLeftEndOfLineAndModifySelection:"),
            Some(selecting(Caret::LineStart))
        );
        assert_eq!(
            page_action("deleteToEndOfParagraph:"),
            Some(erasing(Unit::Paragraph, true)),
            "⌃K"
        );
        assert_eq!(page_action("insertNewline:"), Some(PageAction::Split));
        assert_eq!(page_action("noSuchSelector:"), None);
    }

    #[test]
    fn every_page_selector_is_answered_or_named_once() {
        let mut seen = HashSet::new();
        for name in PAGE
            .iter()
            .map(|(name, _)| *name)
            .chain(PAGE_UNANSWERED.iter().map(|(n, _)| *n))
        {
            assert!(name.ends_with(':'), "{name}: a selector takes a sender");
            assert!(seen.insert(name), "{name} is listed twice");
        }
        for (name, why) in PAGE_UNANSWERED {
            assert!(!why.is_empty(), "{name} needs its reason");
            assert_eq!(page_action(name), None);
        }
    }

    /// Shift never changes where a key goes on the page either, only whether the anchor stays.
    #[test]
    fn a_page_twin_goes_where_its_plain_selector_goes() {
        for (name, action) in PAGE {
            let Some(plain) = name.strip_suffix("AndModifySelection:") else {
                continue;
            };
            let PageAction::Move { motion, extend } = action else {
                panic!("{name} extends a motion");
            };
            assert!(extend, "{name}");
            if let Some(twin) = page_action(&format!("{plain}:")) {
                assert_eq!(twin, moving(*motion), "{name} and {plain}: disagree");
            }
        }
    }

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
