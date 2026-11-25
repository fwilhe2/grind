// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Edit ▸ Scratch Calculator…: the tape (`doc/calc.md`) as a notebook window. The rows are drawn
//! as [`Op`]s, so what goes where tests on any host; `tape_window.rs` puts one field over the row
//! with the caret and hands its keys to `grind_sheet::TapeEditor`. Lines are set in the
//! fixed-pitch face, which is what lets a run's place be its column times one advance.

use grind_core::color::Rgb;
use grind_sheet::formula::assist::Ink;
use grind_sheet::{Key, Notebook, Row, TapeEditor, TapeInk, TapeSpan};

use crate::metrics::{Family, Font};
use crate::ops::Op;
use crate::sheet::geom::Rect;

pub const WIDTH: f64 = 820.0;
pub const HEIGHT: f64 = 560.0;
/// The face's size, in points.
pub const SIZE: f64 = 13.0;
const PAD: f64 = 16.0;
const ROW_H: f64 = 28.0;
const HANDLE_W: f64 = 44.0;
const VALUE_W: f64 = 140.0;
/// Inside a row's well, before its text.
const INSET: f64 = 6.0;
const LINE_H: f64 = 20.0;
/// The buttons along the bottom.
const FOOTER_H: f64 = 52.0;

/// The system's colours, resolved when drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub ground: Rgb,
    pub ink: Rgb,
    pub muted: Rgb,
    pub divider: Rgb,
    pub accent: Rgb,
    pub red: Rgb,
    pub purple: Rgb,
    pub green: Rgb,
    pub orange: Rgb,
}

/// The fixed-pitch face lines are set in, and the band.
pub fn font() -> Font {
    Font {
        size: SIZE,
        bold: false,
        italic: false,
        family: Family::Mono,
        tabular: false,
    }
}

pub fn rows_area() -> Rect {
    Rect::new(
        PAD,
        PAD,
        WIDTH - PAD * 2.0,
        HEIGHT - FOOTER_H - LINE_H * 2.0 - PAD * 2.0,
    )
}

fn band() -> Rect {
    let rows = rows_area();
    Rect::new(PAD, rows.y + rows.h + PAD / 2.0, rows.w, LINE_H)
}

fn note() -> Rect {
    let band = band();
    Rect::new(PAD, band.y + LINE_H, band.w, LINE_H)
}

/// Where the buttons' row is, from the top.
pub fn footer() -> Rect {
    Rect::new(PAD, HEIGHT - FOOTER_H, WIDTH - PAD * 2.0, FOOTER_H)
}

pub fn visible() -> usize {
    (rows_area().h / ROW_H).floor() as usize
}

fn row(shown: usize) -> Rect {
    let rows = rows_area();
    Rect::new(rows.x, rows.y + ROW_H * shown as f64, rows.w, ROW_H)
}

/// A row's well: the text's place, and the field's on the caret's row.
pub fn well(shown: usize) -> Rect {
    let r = row(shown);
    Rect::new(
        r.x + HANDLE_W,
        r.y + 2.0,
        r.w - HANDLE_W - VALUE_W - PAD / 2.0,
        r.h - 4.0,
    )
}

/// Which row on screen a point is in.
pub fn hit(x: f64, y: f64) -> Option<usize> {
    let rows = rows_area();
    let inside = x >= rows.x && x < rows.x + rows.w && y >= rows.y;
    let shown = ((y - rows.y) / ROW_H).floor() as usize;
    (inside && shown < visible()).then_some(shown)
}

/// The first row shown, so that row `at` is on screen.
pub fn scrolled(top: usize, at: usize, visible: usize) -> usize {
    if at < top {
        at
    } else if at >= top + visible {
        at + 1 - visible
    } else {
        top
    }
}

/// What a selector the field editor hands its delegate means to the tape.
pub fn key(selector: &str) -> Option<Key> {
    Some(match selector {
        "insertNewline:" => Key::Enter,
        "moveUp:" => Key::Up,
        "moveDown:" => Key::Down,
        "insertTab:" => Key::Tab,
        "cancelOperation:" => Key::Escape,
        "deleteBackward:" => Key::Backspace,
        _ => return None,
    })
}

pub fn ink(ink: TapeInk, palette: &Palette) -> Rgb {
    match ink {
        TapeInk::Definition => palette.ink,
        TapeInk::Name | TapeInk::Handle => palette.accent,
        TapeInk::Function => palette.purple,
        TapeInk::Number => palette.green,
        TapeInk::Text => palette.orange,
        TapeInk::Error | TapeInk::Unknown => palette.red,
    }
}

/// A run of `text` starting at column `column` of a line drawn from `x`.
fn run(x: f64, top: f64, column: usize, text: &str, color: Rgb, advance: f64, clip: Rect) -> Op {
    Op::Run {
        x: x + column as f64 * advance,
        top,
        text: text.to_owned(),
        font: font(),
        color,
        underline: false,
        strike: false,
        clip,
    }
}

/// `text`'s words in their colours, from `x`.
fn coloured(
    ops: &mut Vec<Op>,
    (x, top): (f64, f64),
    text: &str,
    spans: &[TapeSpan],
    palette: &Palette,
    advance: f64,
    clip: Rect,
) {
    let mut at = 0;
    let mut pieces: Vec<(usize, &str, Rgb)> = Vec::new();
    for span in spans {
        if let (Some(gap), Some(word)) =
            (text.get(at..span.range.start), text.get(span.range.clone()))
        {
            pieces.push((at, gap, palette.ink));
            pieces.push((span.range.start, word, ink(span.ink, palette)));
            at = span.range.end;
        }
    }
    pieces.push((at, text.get(at..).unwrap_or_default(), palette.ink));
    for (byte, piece, color) in pieces {
        if piece.is_empty() {
            continue;
        }
        let column = text.get(..byte).map_or(0, |head| head.chars().count());
        ops.push(run(x, top, column, piece, color, advance, clip));
    }
}

/// The window's frame under its controls: each row's handle, its words coloured (the caret's row
/// left to the field over it) and its value, then the offers and the note. `advance` is the
/// fixed-pitch face's width of one character.
pub fn frame(editor: &TapeEditor, top: usize, palette: &Palette, advance: f64) -> Vec<Op> {
    let mut ops = vec![Op::Fill {
        rect: Rect::new(0.0, 0.0, WIDTH, HEIGHT),
        color: palette.ground,
    }];
    let text_top = |r: Rect| r.y + (r.h - LINE_H) / 2.0 + 2.0;
    let lines = editor.tape().lines();
    for (shown, at) in editor.rows().iter().skip(top).take(visible()).enumerate() {
        let r = row(shown);
        ops.push(Op::Fill {
            rect: Rect::new(r.x, r.y + r.h - 1.0, r.w, 1.0),
            color: palette.divider,
        });
        let label = Notebook::label(*at, editor.tape());
        ops.push(run(r.x, text_top(r), 0, &label, palette.muted, advance, r));
        if *at != editor.row() {
            let w = well(shown);
            let (text, spans) = editor.shown(*at);
            coloured(
                &mut ops,
                (w.x + INSET, text_top(r)),
                &text,
                &spans,
                palette,
                advance,
                w,
            );
        }
        if let Row::Line(n) = at
            && let Some(line) = lines.get(*n)
        {
            let width = line.value.chars().count() as f64 * advance;
            let color = if line.error { palette.red } else { palette.ink };
            ops.push(run(
                r.x + r.w - width,
                text_top(r),
                0,
                &line.value,
                color,
                advance,
                r,
            ));
        }
    }

    let band_rect = band();
    let pieces = editor.band();
    if pieces.is_empty() {
        let hint = "Return: next line  \u{b7}  \u{2191}\u{2193}: move  \u{b7}  Tab: complete  \u{b7}  \u{2318}Z: undo";
        ops.push(run(
            band_rect.x,
            band_rect.y,
            0,
            hint,
            palette.muted,
            advance,
            band_rect,
        ));
    } else {
        let mut column = 0;
        for piece in pieces {
            let color = match piece.ink {
                Ink::Plain => palette.ink,
                Ink::Muted => palette.muted,
                Ink::Strong => palette.accent,
            };
            ops.push(run(
                band_rect.x,
                band_rect.y,
                column,
                &piece.text,
                color,
                advance,
                band_rect,
            ));
            column += piece.text.chars().count();
        }
    }
    let note_rect = note();
    let (said, color) = if editor.tape().is_empty() && editor.note().is_empty() {
        (
            "A formula without its =. #2 reads line 2; vat = 0.19 names a line.",
            palette.muted,
        )
    } else {
        (editor.note(), palette.red)
    };
    if !said.is_empty() {
        ops.push(run(
            note_rect.x,
            note_rect.y,
            0,
            said,
            color,
            advance,
            note_rect,
        ));
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::Tape;

    const PALETTE: Palette = Palette {
        ground: (255, 255, 255),
        ink: (0, 0, 0),
        muted: (128, 128, 128),
        divider: (230, 230, 230),
        accent: (0, 0, 255),
        red: (255, 0, 0),
        purple: (128, 0, 128),
        green: (0, 128, 0),
        orange: (255, 128, 0),
    };

    fn runs(ops: &[Op]) -> Vec<(String, Rgb, f64)> {
        ops.iter()
            .filter_map(|op| match op {
                Op::Run { text, color, x, .. } => Some((text.clone(), *color, *x)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_line_is_drawn_with_its_handle_its_coloured_words_and_its_value() {
        let mut tape = Tape::new(None);
        tape.push("foo = 3").unwrap();
        tape.push("foo * 2").unwrap();
        let editor = TapeEditor::new(tape);
        let drawn = runs(&frame(&editor, 0, &PALETTE, 8.0));
        let at = |word: &str| drawn.iter().find(|(t, _, _)| t == word).cloned();
        assert_eq!(at("#2").map(|r| r.1), Some(PALETTE.muted));
        let name = drawn
            .iter()
            .filter(|(t, _, _)| t == "foo")
            .map(|r| r.1)
            .collect::<Vec<_>>();
        assert_eq!(name, [PALETTE.ink, PALETTE.accent], "defined, then read");
        // Right-aligned: the value ends at the row's right edge, a character per advance.
        let six = at("6").expect("the second line's value");
        assert_eq!(six.2, rows_area().x + rows_area().w - 8.0);
        // The caret's row is the field's: the blank row after the last draws only its handle.
        assert!(at("#3").is_some());
    }

    #[test]
    fn a_word_after_others_is_placed_by_its_column() {
        let mut tape = Tape::new(None);
        tape.push("1 + 2").unwrap();
        let mut editor = TapeEditor::new(tape);
        editor.typed("x", 1);
        let drawn = runs(&frame(&editor, 0, &PALETTE, 10.0));
        let two = drawn.iter().find(|(t, _, _)| t == "2").expect("the 2");
        assert_eq!(two.2, well(0).x + INSET + 2.0 * 10.0);
        assert_eq!(two.1, PALETTE.green);
    }

    #[test]
    fn rows_fit_above_the_band_and_a_click_finds_its_row() {
        assert!(visible() >= 10);
        assert!(note().y + LINE_H <= footer().y);
        assert_eq!(hit(100.0, rows_area().y + ROW_H * 1.5), Some(1));
        assert_eq!(hit(100.0, 2.0), None);
        assert_eq!(scrolled(0, 14, 10), 5);
    }

    #[test]
    fn the_field_editors_selectors_are_the_tapes_keys() {
        assert_eq!(key("insertNewline:"), Some(Key::Enter));
        assert_eq!(key("deleteBackward:"), Some(Key::Backspace));
        assert_eq!(key("moveLeft:"), None);
    }
}
