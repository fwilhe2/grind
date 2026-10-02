// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The page's numbers and where its text column sits in the window, as pure arithmetic.
//! **No GTK types.**
//!
//! `ui_sheet_gtk/src/geom.rs`'s counterpart, and it exists for the same reason: a custom-drawn
//! widget's layout decisions are the part most worth testing and the part hardest to test
//! through a display, so they live in a module that has never heard of one.
//!
//! **This is neither line layout nor the stacking above it.** Breaking a paragraph into lines
//! is `grind_core::layout`'s job and reaches this shell through [`grind_text::App`]
//! (`doc/text-layout.md`, Path C); how tall each block's box is, where it starts, where a
//! table's blocks go, which ones are on screen and how far to scroll to keep the caret visible
//! is [`grind_text::flow`]'s, shared with every shell that draws a page. This file was one of
//! its two copies and `ui_win32` the other, until the macOS shell would have made a third
//! (`doc/macos-shell.md`, M1). What is left here is this window's own numbers, and where the
//! text column sits in it.

pub use grind_text::flow::{Across, Flow, Spacing};

/// Space either side of the text column, in pixels.
pub const MARGIN: f64 = 32.0;

/// The widest the text column is allowed to get.
///
/// A maximised window is far wider than a readable measure, and a word processor that sets
/// prose across 1800 pixels is unreadable in a way a spreadsheet never is. Roughly 80
/// characters at a normal body size; the column is centred in whatever is left.
pub const MEASURE: f64 = 720.0;

/// How far one nesting level of a list indents its text.
pub const INDENT: f64 = 28.0;

/// Space under a paragraph, and the extra a heading gets above it — the whole of this
/// shell's typography beyond the font itself.
pub const GAP: f64 = 10.0;
pub const HEADING_GAP: f64 = 18.0;

/// The text column inside a widget `width` pixels wide: how wide it is, and where it starts.
///
/// Centred rather than left-aligned once the window is wider than [`MEASURE`], which is what
/// keeps the measure constant as a window grows instead of letting the lines stretch.
pub fn column(width: f64) -> (f64, f64) {
    let available = (width - 2.0 * MARGIN).max(1.0);
    let text = available.min(MEASURE);
    (MARGIN + (available - text) / 2.0, text)
}

/// The space between a table's rule and the text inside it, and how thick that rule is.
pub const CELL_PAD: f64 = 6.0;
pub const RULE: f64 = 1.0;

/// This window's page — the numbers [`grind_text::flow`] stacks with. The page's top margin is
/// [`MARGIN`], the same space it leaves either side, and it is part of the flow rather than a
/// fixed band, so it scrolls away with the text the way the top of a page does.
pub const SPACING: Spacing = Spacing {
    top: MARGIN,
    gap: GAP,
    heading: HEADING_GAP,
    indent: INDENT,
    cell_pad: CELL_PAD,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_column_is_centred_once_the_window_is_wider_than_the_measure() {
        let (x, w) = column(400.0);
        assert_eq!((x, w), (MARGIN, 400.0 - 2.0 * MARGIN), "narrow: all of it");
        let (x, w) = column(2.0 * MEASURE);
        assert_eq!(w, MEASURE, "wide: the measure holds");
        assert!(x > MARGIN, "and what is left is split either side");
        assert_eq!(x + w + x, 2.0 * MEASURE, "symmetrically");
        // A window narrower than its own margins still has a column to draw in.
        assert!(column(1.0).1 >= 1.0);
    }
}
