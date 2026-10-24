// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The page's numbers and where its text column sits in the window, as pure arithmetic with **no
//! GTK types**. Line breaking is `grind_core::layout`'s and stacking the blocks — boxes, table
//! cells, what is on screen, scrolling to the caret — is [`grind_text::flow`]'s; what is left is
//! this window's own numbers.

pub use grind_text::flow::{Across, Flow, Spacing};

/// Space either side of the text column, in pixels.
pub const MARGIN: f64 = 32.0;

/// The widest the text column may get — roughly 80 characters at a normal body size, since prose
/// across 1800 pixels is unreadable; the column is centred in what is left.
pub const MEASURE: f64 = 720.0;

/// How far one nesting level of a list indents its text.
pub const INDENT: f64 = 28.0;

/// Space under a paragraph, and the extra a heading gets above it — the whole of this
/// shell's typography beyond the font itself.
pub const GAP: f64 = 10.0;
pub const HEADING_GAP: f64 = 18.0;

/// The text column inside a widget `width` pixels wide: how wide it is, and where it starts —
/// centred once the window is wider than [`MEASURE`].
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
