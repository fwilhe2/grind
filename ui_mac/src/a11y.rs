// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The accessibility floor's words (decision 10, M9) — portable.
//!
//! The page speaks `NSAccessibility`'s text-area protocol: its value is the document's text,
//! one line a block; its selected range is the selection or the caret in that text, in the
//! UTF-16 units an `NSRange` counts; its insertion line is the caret's block. A move on the grid
//! posts an announcement saying where the cursor is and what the cell shows. Each answer is
//! computed here from the core, so what VoiceOver is told cannot drift from what is drawn —
//! and the runner reads all of it back in-process through a drive's `a11y` step.
//!
//! ponytail: a line of the value is a block, not a laid-out line, so "the insertion point's
//! line" is a paragraph's number. `accesskit_macos` is the named upgrade (decision 10), and the
//! day a screen reader user asks for line-by-line reading is the trigger.

use std::ops::Range;

use grind_sheet::nav::Selection;
use grind_text::Caret;

use crate::text::state::Page;

/// The page's `accessibilityValue`: every block's text, a newline between each.
pub fn page_value(app: &grind_text::App) -> String {
    (0..app.block_count())
        .map(|block| app.input_text(block).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where `caret` is in [`page_value`], in UTF-16 units.
fn units_at(app: &grind_text::App, caret: Caret) -> usize {
    let mut units = 0;
    for block in 0..caret.block.min(app.block_count()) {
        units += app
            .input_text(block)
            .unwrap_or_default()
            .encode_utf16()
            .count()
            + 1;
    }
    let text = app.input_text(caret.block).unwrap_or_default();
    units
        + text
            .chars()
            .take(caret.offset)
            .map(char::len_utf16)
            .sum::<usize>()
}

/// The page's `accessibilitySelectedTextRange`: the selection, or the caret as an empty range.
pub fn page_range(app: &grind_text::App, page: &Page) -> Range<usize> {
    let (from, to) = page.selection().unwrap_or((page.caret, page.caret));
    units_at(app, from)..units_at(app, to)
}

/// The page's `accessibilityInsertionPointLineNumber`.
pub fn page_line(page: &Page) -> usize {
    page.caret.block
}

/// What a move on the grid announces: where the cursor is, as the name box says it, and what
/// the active cell shows — "empty" rather than silence, since silence sounds like nothing
/// happened.
pub fn grid_announcement(app: &grind_sheet::App, sheet: usize, selection: Selection) -> String {
    let place = grind_sheet::place::name_box(app, sheet, selection);
    let at = selection.active;
    let shown = app
        .get_viewport(sheet, at.row..at.row + 1, at.col..at.col + 1)
        .ok()
        .and_then(|view| view.text(at.row, at.col).map(str::to_owned))
        .filter(|text| !text.is_empty());
    match shown {
        Some(text) => format!("{place}, {text}"),
        None => format!("{place}, empty"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::{Pos, RecalcMode};
    use grind_text::BlockKind;

    #[test]
    fn the_page_is_its_blocks_a_line_each_and_the_range_counts_utf16() {
        let app = grind_text::App::new();
        app.set_text(0, "a😀b").unwrap();
        app.insert(1, BlockKind::Paragraph, "second").unwrap();
        assert_eq!(page_value(&app), "a😀b\nsecond");
        let mut page = Page::default();
        page.place(
            Caret {
                block: 0,
                offset: 2,
            },
            false,
        );
        assert_eq!(page_range(&app, &page), 3..3, "past the emoji's two units");
        page.place(
            Caret {
                block: 1,
                offset: 3,
            },
            true,
        );
        assert_eq!(page_range(&app, &page), 3..8, "across the newline");
        assert_eq!(page_line(&page), 1);
    }

    #[test]
    fn a_move_on_the_grid_says_where_and_what() {
        let app = grind_sheet::App::new();
        app.enter(0, Pos::new(1, 1), "42", RecalcMode::Document)
            .unwrap();
        let at = |row, col| Selection {
            anchor: Pos::new(row, col),
            active: Pos::new(row, col),
        };
        assert_eq!(grid_announcement(&app, 0, at(1, 1)), "B2, 42");
        assert_eq!(grid_announcement(&app, 0, at(0, 0)), "A1, empty");
    }
}
