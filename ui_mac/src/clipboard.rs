// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The pasteboard (M5): Copy, Cut, Paste and Delete over the grid — the page's (M6) are in
//! `page_view.rs`, over [`write()`] and [`read()`] — and the one file that touches
//! `NSPasteboard` — as `ui_win32/src/clipboard.rs` is the one that opens the Windows clipboard.
//!
//! What travels is `grind_sheet::clip`'s, the codec every shell's clipboard shares: each cell's
//! input text — the raw number, the formula in display syntax — tab-separated, a row a line. It
//! goes on the pasteboard twice, as plain text and as tab-separated text
//! (`NSPasteboardTypeTabularText`), so a text editor and a spreadsheet each take the flavour it
//! reads; Numbers and Excel read the rectangle back as a rectangle. A paste reads plain text, since
//! every source writes that, and goes in through `App::enter_range` — one ⌘Z for the whole
//! rectangle, the formulas turned back into ODF's syntax on the way.

use grind_sheet::{App, RecalcMode, clip};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSPasteboardTypeTabularText};
use objc2_foundation::NSString;

use crate::banner::Action;
use crate::grid_view::Pane;
use crate::notice;

impl Pane {
    /// Edit ▸ Copy: the selection on the pasteboard.
    pub fn copy(&self) {
        // A whole row or column is cut to what the sheet uses, or a copied column would be a
        // million lines (`nav::target`).
        let used = self.app.used_extent(self.sheet.get()).unwrap_or((0, 0));
        let (start, end) = grind_sheet::nav::target(self.selection.get(), used);
        let text = clip::rect_text(
            &self.app,
            self.sheet.get(),
            start,
            end,
            App::input_text,
            "\n",
        );
        write(&text, true);
    }

    /// Edit ▸ Copy Value: the selection as it is *shown* — a formula's formatted result rather
    /// than its source (`App::value_text`), for pasting into something that is not a spreadsheet.
    pub fn copy_value(&self) {
        let used = self.app.used_extent(self.sheet.get()).unwrap_or((0, 0));
        let (start, end) = grind_sheet::nav::target(self.selection.get(), used);
        let text = clip::rect_text(
            &self.app,
            self.sheet.get(),
            start,
            end,
            App::value_text,
            "\n",
        );
        write(&text, true);
    }

    /// Edit ▸ Cut: the selection on the pasteboard, then emptied — one ⌘Z brings it back.
    pub fn cut(&self) {
        self.copy();
        self.clear();
    }

    /// Edit ▸ Delete, and the Delete keys: the selected cells emptied, their formatting kept.
    pub fn clear(&self) {
        let (start, end) = self.selection.get().rect();
        if let Err(error) = self.app.clear_range(self.sheet.get(), start, end) {
            self.say(Some((&error.to_string(), None)));
        }
    }

    /// Edit ▸ Paste: the pasteboard's text as a rectangle from the active cell. Nothing happens
    /// when it holds no text — pasting a picture writes no garbage into a cell.
    pub fn paste(&self) {
        let Some(text) = read() else {
            return;
        };
        let rows = clip::parse_rows(&text);
        if rows.is_empty() {
            return;
        }
        let active = self.selection.get().active;
        match self
            .app
            .enter_range(self.sheet.get(), active, &rows, RecalcMode::Document)
        {
            Ok(outcome) => match outcome.recalc.filter(|recalc| recalc.spoiled > 0) {
                Some(recalc) => self.say(Some((
                    &notice::recalc_skipped(recalc.spoiled),
                    Some(Action::RecalculateAnyway),
                ))),
                None => self.say(None),
            },
            Err(error) => self.say(Some((&error.to_string(), None))),
        }
    }

    /// Whether Paste has anything to paste — what greys it when it has not.
    pub fn can_paste() -> bool {
        can_paste()
    }
}

/// `text` on the general pasteboard as plain text — and as tab-separated text too when it is a
/// rectangle of cells, so a spreadsheet reads it back as one.
pub fn write(text: &str, tabular: bool) {
    let text = NSString::from_str(text);
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    // SAFETY: both types are constants AppKit exports.
    unsafe {
        board.setString_forType(&text, NSPasteboardTypeString);
        if tabular {
            board.setString_forType(&text, NSPasteboardTypeTabularText);
        }
    }
}

/// The general pasteboard's plain text, when it holds some.
pub fn read() -> Option<String> {
    // SAFETY: the type is a constant AppKit exports.
    let text = unsafe { NSPasteboard::generalPasteboard().stringForType(NSPasteboardTypeString) };
    text.map(|text| text.to_string())
}

/// Whether Paste has anything to paste — what greys it when it has not.
pub fn can_paste() -> bool {
    read().is_some_and(|text| !text.is_empty())
}
