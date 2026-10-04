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

/// What a piece of content *is*, for a tagged PDF's structure: part of a block's text, the
/// label in front of a list item, or decoration a reader should skip (a table's rule, a
/// highlight behind a word).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Content(usize),
    Label(usize),
    Decoration,
}

/// One block's place in the document's structure.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Paragraph {
        block: usize,
    },
    Heading {
        block: usize,
        level: u32,
        title: String,
    },
    ListItem {
        block: usize,
        depth: u32,
    },
    Figure {
        block: usize,
        alt: Option<String>,
    },
    /// A block in a table cell. `table` is the table's name, which is what tells one table from
    /// the next in a flat sequence of blocks.
    Cell {
        block: usize,
        table: String,
        row: u32,
        column: u32,
    },
}

impl Element {
    pub fn block(&self) -> usize {
        match self {
            Element::Paragraph { block }
            | Element::Heading { block, .. }
            | Element::ListItem { block, .. }
            | Element::Figure { block, .. }
            | Element::Cell { block, .. } => *block,
        }
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
        mark: Mark,
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
        mark: Mark,
    },
}

/// One page, its size and what is on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub ops: Vec<Op>,
    /// The first character on the page, and one past the last — `None` on a page with no text
    /// at all. A page that ends where the next begins is the cut `grind text pages` prints.
    pub start: Option<grind_text::Caret>,
    pub end: Option<grind_text::Caret>,
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
    /// Every block that printed, in document order, as what it is.
    pub structure: Vec<Element>,
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
