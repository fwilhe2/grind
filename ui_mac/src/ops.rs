// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A frame as a list of things to draw, for either document — the vocabulary `sheet/paint.rs`
//! and `text/paint.rs` speak and `render.rs` executes.
//!
//! Portable, with no CoreGraphics in it: whatever decides *what* is drawn where is tested on
//! Linux, and the Mac half only puts it down (decision 9: one drawing path, two callers).

use std::rc::Rc;

use grind_core::color::Rgb;
use grind_core::style::TextStyle;

use crate::metrics::Font;
use crate::sheet::geom::Rect;

/// How strongly an [`Op::Wash`] tints what is under it — enough to read as selected, and little
/// enough that a document's own red is still red. `ui_win32`'s wash, as an alpha rather than a
/// blend, since CoreGraphics composites where GDI could not.
pub const WASH: f64 = 0.22;

/// One thing to draw.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A filled rectangle — a ground, a fill the document chose, a rule, a caret.
    Fill { rect: Rect, color: Rgb },
    /// A translucent tint over what is already down, at [`WASH`] — the selection.
    Wash { rect: Rect, color: Rgb },
    /// One line of a **cell's** text whose box starts at `(x, top)`, set in `style` as the grid
    /// sets it, drawn only inside `clip`. The renderer finds the baseline from the font it
    /// resolves `style` to — the same resolution `metrics::CoreText` measured with.
    Text {
        x: f64,
        top: f64,
        text: String,
        style: TextStyle,
        color: Rgb,
        clip: Rect,
    },
    /// One piece of a line on the **page**, whose box starts at `(x, top)`, set in `font` — the
    /// face its block and its run resolved to, which is exactly what measured it
    /// (`text/face.rs`). Underline and strike are drawn with it, from that font's own metrics.
    Run {
        x: f64,
        top: f64,
        text: String,
        font: Font,
        color: Rgb,
        underline: bool,
        strike: bool,
        clip: Rect,
    },
    /// A picture's bytes — a PNG, a JPEG, whatever the document holds — drawn to fill `rect`,
    /// decoded by the renderer, which on a Mac is `NSImage`. Shared rather than copied, since a
    /// frame may draw one picture in several places and every paint makes a frame.
    Image { rect: Rect, data: Rc<[u8]> },
}

impl Op {
    /// The same thing drawn `(dx, dy)` further on, with nothing cut away — a view whose frame
    /// starts after a header band draws the band's frame shifted by it.
    pub fn shifted(self, dx: f64, dy: f64) -> Op {
        match self {
            Op::Fill { rect, color } => Op::Fill {
                rect: rect.offset(dx, dy),
                color,
            },
            Op::Wash { rect, color } => Op::Wash {
                rect: rect.offset(dx, dy),
                color,
            },
            Op::Text {
                x,
                top,
                text,
                style,
                color,
                clip,
            } => Op::Text {
                x: x + dx,
                top: top + dy,
                text,
                style,
                color,
                clip: clip.offset(dx, dy),
            },
            Op::Run {
                x,
                top,
                text,
                font,
                color,
                underline,
                strike,
                clip,
            } => Op::Run {
                x: x + dx,
                top: top + dy,
                text,
                font,
                color,
                underline,
                strike,
                clip: clip.offset(dx, dy),
            },
            Op::Image { rect, data } => Op::Image {
                rect: rect.offset(dx, dy),
                data,
            },
        }
    }

    /// The same thing drawn `(dx, dy)` further on, kept inside `clip` — how one part of a frame
    /// (a header band, the cells) is placed in the whole.
    pub fn placed(self, dx: f64, dy: f64, clip: &Rect) -> Option<Op> {
        match self {
            Op::Fill { rect, color } => {
                let rect = rect.offset(dx, dy).intersection(clip);
                (!rect.is_empty()).then_some(Op::Fill { rect, color })
            }
            Op::Wash { rect, color } => {
                let rect = rect.offset(dx, dy).intersection(clip);
                (!rect.is_empty()).then_some(Op::Wash { rect, color })
            }
            Op::Text {
                x,
                top,
                text,
                style,
                color,
                clip: own,
            } => {
                let clip = own.offset(dx, dy).intersection(clip);
                (!clip.is_empty()).then_some(Op::Text {
                    x: x + dx,
                    top: top + dy,
                    text,
                    style,
                    color,
                    clip,
                })
            }
            Op::Run {
                x,
                top,
                text,
                font,
                color,
                underline,
                strike,
                clip: own,
            } => {
                let clip = own.offset(dx, dy).intersection(clip);
                (!clip.is_empty()).then_some(Op::Run {
                    x: x + dx,
                    top: top + dy,
                    text,
                    font,
                    color,
                    underline,
                    strike,
                    clip,
                })
            }
            // A picture is drawn whole or not at all: cutting one would need the renderer to
            // clip, and a picture straddling a band's edge is not a case any frame has.
            Op::Image { rect, data } => {
                let rect = rect.offset(dx, dy);
                (!rect.intersection(clip).is_empty()).then_some(Op::Image { rect, data })
            }
        }
    }
}
