// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A page as data: everything decided, nothing drawn (`doc/pdf-export.md`, "The display list
//! is the seam").
//!
//! Two backends execute the same list — the PDF and the preview's raster — so the preview is the
//! PDF by construction rather than by agreement. It is `ui_mac/src/ops.rs`'s idea applied to
//! paper: decide everything portably, then hand it to whatever puts it down.
//!
//! Coordinates are **points from the page's top-left corner, y growing down**, the way every
//! layout in this suite measures. A backend whose y grows up (a PDF's user space) flips once.

use std::sync::Arc;

use crate::fonts::FaceId;
use crate::metrics::Glyph;

/// An opaque colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);

    /// `#rrggbb`, ODF's only spelling of a colour (`fo:color`). Anything else is `None`.
    pub fn parse(value: &str) -> Option<Rgb> {
        let hex = value.trim().strip_prefix('#')?;
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        Some(Rgb(channel(0)?, channel(2)?, channel(4)?))
    }
}

/// One thing to put on a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Shaped glyphs on a baseline starting at `(x, y)`. `text` is what they were shaped from,
    /// and each glyph's `text` range indexes into it.
    Text {
        x: f32,
        y: f32,
        face: FaceId,
        size: f32,
        glyphs: Vec<Glyph>,
        text: String,
        color: Rgb,
    },
    /// A filled rectangle: a highlight behind text.
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Rgb,
    },
    /// A straight stroke: an underline, a strike-through, a table's rule.
    Line {
        from: (f32, f32),
        to: (f32, f32),
        width: f32,
        color: Rgb,
    },
    /// A picture, scaled into its box.
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        mime: String,
        data: Arc<Vec<u8>>,
    },
}

/// One page, its size and what is on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub ops: Vec<Op>,
}

/// A heading, for the PDF's outline (its bookmarks pane).
#[derive(Clone, Debug, PartialEq)]
pub struct Heading {
    pub level: u32,
    pub title: String,
    /// 0-based.
    pub page: usize,
    /// The top of its first line, from the top of the page.
    pub y: f32,
}

/// A whole document, typeset.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub pages: Vec<Page>,
    pub outline: Vec<Heading>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_is_read_as_odf_spells_it_and_nothing_else_is() {
        assert_eq!(Rgb::parse("#1a2B3c"), Some(Rgb(0x1a, 0x2b, 0x3c)));
        assert_eq!(Rgb::parse("1a2b3c"), None);
        assert_eq!(Rgb::parse("#fff"), None);
        assert_eq!(Rgb::parse("red"), None);
        assert_eq!(Rgb::parse("#ééé"), None);
    }
}
