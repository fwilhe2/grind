// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Data ▸ Scratch Calculator…: the tape (`doc/calc.md`) as a notebook in this shell's modal. The
//! rows are drawn, and one edit sits over the row with the caret; what a key does there is
//! `grind_sheet::TapeEditor`'s. Here is what is portable: where the rows go, which key is which,
//! and the colour of each kind of word. `dialog.rs` draws and pumps.

use grind_sheet::{Key, TapeInk};

use crate::sheet::geom::{Rect, scale};
use crate::theme::{Mode, Rgb, Theme};

/// One row, the handle's column and the value's, at 100%.
const ROW_H: f64 = 32.0;
const HANDLE_W: f64 = 44.0;
const VALUE_W: f64 = 132.0;
const GAP: f64 = 8.0;
/// The band of offers and the note under the rows, one line each.
const LINE_H: f64 = 22.0;

/// Where the parts of the body go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub rows: Rect,
    pub band: Rect,
    pub note: Rect,
    row_h: f64,
    handle_w: f64,
    value_w: f64,
    gap: f64,
}

impl Layout {
    pub fn of(body: Rect, dpi: u32) -> Self {
        let s = |v: f64| scale(v, dpi);
        let line = s(LINE_H);
        let rows_h = (body.h - line * 2.0 - s(GAP)).max(s(ROW_H));
        Self {
            rows: Rect { h: rows_h, ..body },
            band: Rect {
                y: body.y + rows_h + s(GAP),
                h: line,
                ..body
            },
            note: Rect {
                y: body.y + rows_h + s(GAP) + line,
                h: line,
                ..body
            },
            row_h: s(ROW_H),
            handle_w: s(HANDLE_W),
            value_w: s(VALUE_W),
            gap: s(GAP),
        }
    }

    /// How many rows fit.
    pub fn visible(&self) -> usize {
        ((self.rows.h / self.row_h).floor() as usize).max(1)
    }

    /// The `shown`th row on screen.
    pub fn row(&self, shown: usize) -> Rect {
        Rect {
            y: self.rows.y + self.row_h * shown as f64,
            h: self.row_h,
            ..self.rows
        }
    }

    pub fn handle(&self, row: Rect) -> Rect {
        Rect {
            w: self.handle_w,
            ..row
        }
    }

    /// The well the text is in: what the edit sits inside on the caret's row.
    pub fn text(&self, row: Rect) -> Rect {
        let inset = self.gap / 2.0;
        Rect {
            x: row.x + self.handle_w,
            y: row.y + inset / 2.0,
            w: (row.w - self.handle_w - self.value_w - self.gap).max(0.0),
            h: row.h - inset,
        }
    }

    pub fn value(&self, row: Rect) -> Rect {
        Rect {
            x: row.x + row.w - self.value_w,
            w: self.value_w,
            ..row
        }
    }

    /// Which row on screen a point is in.
    pub fn hit(&self, x: f64, y: f64) -> Option<usize> {
        self.rows
            .contains(x, y)
            .then(|| ((y - self.rows.y) / self.row_h).floor() as usize)
    }
}

/// The first row shown, so that row `at` is on screen: unchanged when it already is.
pub fn scrolled(top: usize, at: usize, visible: usize) -> usize {
    if at < top {
        at
    } else if at >= top + visible {
        at + 1 - visible
    } else {
        top
    }
}

/// A virtual key the editor answers. Shift+Tab is the dialog's, to go back to the buttons.
pub fn key(vk: u16, shift: bool) -> Option<Key> {
    Some(match vk {
        0x0d => Key::Enter,
        0x26 => Key::Up,
        0x28 => Key::Down,
        0x09 if !shift => Key::Tab,
        0x1b => Key::Escape,
        0x08 => Key::Backspace,
        _ => return None,
    })
}

/// The colour a kind of word is drawn in; `None` for the ordinary text's.
pub fn ink(ink: TapeInk, theme: &Theme) -> Option<Rgb> {
    let dark = theme.mode == Mode::Dark;
    let pick = |light: Rgb, night: Rgb| Some(if dark { night } else { light });
    match ink {
        TapeInk::Definition => None,
        TapeInk::Name | TapeInk::Handle => Some(theme.accent),
        TapeInk::Function => pick(Rgb(0x7c, 0x3a, 0xed), Rgb(0xc4, 0xa1, 0xff)),
        TapeInk::Number => pick(Rgb(0x04, 0x78, 0x57), Rgb(0x5f, 0xd3, 0xa5)),
        TapeInk::Text => pick(Rgb(0xb4, 0x53, 0x09), Rgb(0xf0, 0xb4, 0x6a)),
        TapeInk::Error | TapeInk::Unknown => Some(theme.misspelt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::of(
            Rect {
                x: 24.0,
                y: 60.0,
                w: 560.0,
                h: 420.0,
            },
            96,
        )
    }

    #[test]
    fn rows_fill_the_body_above_the_band_and_the_note() {
        let l = layout();
        assert_eq!(l.visible(), 11);
        assert_eq!(l.row(1).y, 92.0);
        assert!(l.note.y + l.note.h <= 480.0);
        assert!(l.band.y >= l.rows.y + l.rows.h);
        let row = l.row(0);
        let (text, value) = (l.text(row), l.value(row));
        assert!(
            text.x + text.w <= value.x,
            "the text stops before the value"
        );
        assert_eq!(l.hit(100.0, 93.0), Some(1));
        assert_eq!(l.hit(100.0, 10.0), None);
    }

    #[test]
    fn the_caret_row_is_scrolled_into_view() {
        assert_eq!(scrolled(0, 3, 10), 0);
        assert_eq!(scrolled(0, 12, 10), 3);
        assert_eq!(scrolled(5, 2, 10), 2);
    }

    #[test]
    fn keys_map_to_the_editor_and_shift_tab_is_the_dialogs() {
        assert_eq!(key(0x0d, false), Some(Key::Enter));
        assert_eq!(key(0x09, false), Some(Key::Tab));
        assert_eq!(key(0x09, true), None);
        assert_eq!(key(0x41, false), None);
    }

    #[test]
    fn a_name_is_the_accent_and_an_unknown_word_the_error_red() {
        let theme = Theme::of(Mode::Light);
        assert_eq!(ink(TapeInk::Name, &theme), Some(theme.accent));
        assert_eq!(ink(TapeInk::Unknown, &theme), Some(theme.misspelt));
        assert_eq!(ink(TapeInk::Definition, &theme), None);
    }
}
