// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The page's numbers, in points — where the text column sits and how blocks are spaced (M6).
//!
//! Everything about stacking blocks is `grind_text::flow`'s, shared with every window that sets
//! a page; this is only what this one is set *at*: its [`Spacing`] and its column. **No AppKit**,
//! so it is tested on any host.
//!
//! The page is the document view of an `NSScrollView` and is laid out in its own coordinates —
//! flipped, y down from the document's top — so scrolling is the scroll view's business and
//! nothing here has a scroll offset in it, which is the difference from the Windows pane's
//! `Page`.

use grind_text::flow::Spacing;

/// Space either side of the text column.
pub const MARGIN: f64 = 48.0;

/// The widest the text column gets — a readable measure, about eighty characters of the body;
/// wider windows centre it rather than stretch the lines.
pub const MEASURE: f64 = 640.0;

/// Above the first block, and under the last — the page's own margin, which scrolls with the
/// text.
pub const TOP: f64 = 36.0;

/// Under every block, and the extra a heading gets above it.
pub const GAP: f64 = 8.0;
pub const HEADING_GAP: f64 = 16.0;

/// How far one level of a list indents its text; its bullet hangs in that space.
pub const INDENT: f64 = 24.0;

/// Between a table cell's rule and its text, and the rule's own width.
pub const CELL_PAD: f64 = 6.0;
pub const RULE: f64 = 1.0;

/// How wide the caret is.
pub const CARET_W: f64 = 1.5;

/// The numbers `grind_text::flow` stacks this page with.
pub fn spacing() -> Spacing {
    Spacing {
        top: TOP,
        gap: GAP,
        heading: HEADING_GAP,
        indent: INDENT,
        cell_pad: CELL_PAD,
    }
}

/// The text column in a page `width` points wide: where it starts and how wide it is. Centred
/// once the page is wider than [`MEASURE`], which keeps the measure as a window grows.
pub fn column(width: f64) -> (f64, f64) {
    let available = (width - 2.0 * MARGIN).max(1.0);
    let text = available.min(MEASURE);
    (MARGIN + (available - text) / 2.0, text)
}

/// Where a list item's bullet is set, from the text's own left edge: most of the way back into
/// the indent it hangs in.
pub fn bullet_x() -> f64 {
    -INDENT * 0.7
}

/// How tall the page is for a document whose flow is `height` tall: the flow, a bottom margin
/// the size of the top one, and never less than the view it fills.
pub fn page_height(height: f64, visible: f64) -> f64 {
    (height + TOP).max(visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_is_centred_once_the_window_is_wider_than_the_measure() {
        assert_eq!(column(400.0), (MARGIN, 400.0 - 2.0 * MARGIN));
        let (x, w) = column(2.0 * MEASURE);
        assert_eq!(w, MEASURE);
        assert_eq!(x + w + x, 2.0 * MEASURE, "symmetrically");
        assert!(column(1.0).1 >= 1.0, "narrower than its margins");
    }

    #[test]
    fn a_bullet_hangs_inside_the_indent() {
        assert!(bullet_x() < 0.0 && bullet_x() > -INDENT);
    }

    #[test]
    fn the_page_fills_its_view_and_ends_with_a_margin() {
        assert_eq!(page_height(100.0, 600.0), 600.0);
        assert_eq!(page_height(1000.0, 600.0), 1000.0 + TOP);
    }
}
