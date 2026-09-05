// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where each block sits down the page, and where the page sits in the window — pure arithmetic.
//!
//! `sheet/geom.rs`'s counterpart for the text pane, and it exists for the same reason: the layout
//! decisions are the part most worth testing and the part hardest to test through a window, so
//! they live in a module that has never heard of one. **No Windows types at all.**
//!
//! **This is not line layout.** Breaking a paragraph into lines is `grind_core::layout`'s job and
//! reaches this shell through [`grind_text::App`] (`doc/text-layout.md`, Path C). What is left
//! here is the stacking above it: how tall each block's box is, where it starts, which ones are on
//! screen, and how far to scroll to keep the caret visible.
//!
//! ponytail: [`Flow`] is a second copy of `ui_text_gtk/src/geom.rs`'s, down to the collapsing
//! rule and the "nearest, never nothing" hit test. The two cannot share one today — the GTK
//! version lives in a crate that needs GTK to compile at all and this one must not — and the
//! upgrade path is `grind-text`, since stacking blocks with collapsed gaps is the word
//! processor's vocabulary rather than a toolkit's. **The trigger is a third copy**, or the first
//! time the two answer a scroll differently. `ui_web` is not one: it stacks blocks with the DOM.

use crate::sheet::geom::{Rect, scale};

/// Space either side of the text column, in pixels at 100%.
pub const MARGIN: f64 = 32.0;

/// The widest the text column is allowed to get.
///
/// A maximised window is far wider than a readable measure, and a word processor that sets prose
/// across 1800 pixels is unreadable in a way a spreadsheet never is. Roughly 80 characters at a
/// normal body size; the column is centred in whatever is left.
pub const MEASURE: f64 = 720.0;

/// How far one nesting level of a list indents its text.
pub const INDENT: f64 = 28.0;

/// The page's own top margin — space above the first block, which **scrolls with the text**
/// rather than framing it, so that a document scrolled to the bottom has no dead band at the top.
pub const TOP: f64 = 24.0;

/// Space under a block, and the extra a heading gets above it — the whole of this pane's
/// typography beyond the font itself.
pub const GAP: f64 = 10.0;
pub const HEADING_GAP: f64 = 18.0;

/// The status bar, the same height the grid's is so that the two panes' windows agree.
pub const STATUS_H: f64 = 22.0;

/// The notice bar, when there is a notice. Zero when there is not — see `win.rs`'s `banner_h`.
pub const BANNER_H: f64 = 26.0;

/// The text column inside a pane `width` pixels wide: where it starts and how wide it is.
///
/// Centred rather than left-aligned once the window is wider than [`MEASURE`], which keeps the
/// measure constant as a window grows instead of letting the lines stretch.
pub fn column(width: f64, dpi: u32) -> (f64, f64) {
    let margin = scale(MARGIN, dpi);
    let available = (width - 2.0 * margin).max(1.0);
    let text = available.min(scale(MEASURE, dpi));
    (margin + (available - text) / 2.0, text)
}

/// The window's furniture around the page: what is left for the document, and where.
///
/// The banner's *height* is the window's to set, and it is zero when there is no notice — so
/// every rectangle below it is one arithmetic expression whether or not it is showing, which is
/// the arrangement the grid already has.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Page {
    pub width: f64,
    pub height: f64,
    pub banner_h: f64,
    pub status_h: f64,
    pub dpi: u32,
    /// How far down the document the top of the body is, in pixels.
    pub scroll: f64,
}

impl Page {
    pub fn banner(&self) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: self.width,
            h: self.banner_h,
        }
    }

    /// The part of the window the document is drawn in.
    pub fn body(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.banner_h,
            w: self.width,
            h: (self.height - self.banner_h - self.status_h).max(0.0),
        }
    }

    pub fn status(&self) -> Rect {
        Rect {
            x: 0.0,
            y: (self.height - self.status_h).max(0.0),
            w: self.width,
            h: self.status_h,
        }
    }

    /// The text column, in window coordinates.
    pub fn text_column(&self) -> (f64, f64) {
        column(self.width, self.dpi)
    }
}

/// One block's box in the flow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub index: usize,
    /// Distance from the top of the document to the top of this block's text.
    pub top: f64,
    /// The height of its lines — not counting the gap under it.
    pub height: f64,
    /// How far its text is indented from the column's left edge.
    pub indent: f64,
}

impl Slot {
    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }
}

/// Every block, stacked. Built fresh whenever the document, the width or the DPI changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    slots: Vec<Slot>,
    height: f64,
}

impl Flow {
    /// An empty flow whose first block starts `top` below the document's top.
    pub fn new(top: f64) -> Self {
        Flow {
            slots: Vec::new(),
            height: top,
        }
    }

    /// Add a block below the ones already in, with the space that goes above and below it.
    ///
    /// `space_before` is collapsed against the gap the previous block already left, the way every
    /// typesetting system collapses adjacent margins — otherwise a heading after a paragraph gets
    /// both gaps and floats.
    pub fn push(&mut self, index: usize, height: f64, indent: f64, space_before: f64, gap: f64) {
        let top = match self.slots.is_empty() {
            // Nothing above the first block for its own space to sit under.
            true => self.height,
            false => self.height + (space_before - gap).max(0.0),
        };
        self.slots.push(Slot {
            index,
            top,
            height,
            indent,
        });
        self.height = top + height + gap;
    }

    /// How tall the whole document is — the scrollbar's extent.
    pub fn height(&self) -> f64 {
        self.height
    }

    pub fn slot(&self, index: usize) -> Option<&Slot> {
        self.slots.get(index)
    }

    /// The blocks that intersect `top..bottom` — what a paint reads and nothing else.
    ///
    /// A contiguous slice because the flow is in document order, which is what lets a shell hand
    /// its ends straight to [`grind_text::App::get_viewport`] (architecture rule 1).
    pub fn visible(&self, top: f64, bottom: f64) -> &[Slot] {
        let first = self.slots.partition_point(|slot| slot.bottom() < top);
        let last = self.slots.partition_point(|slot| slot.top < bottom);
        &self.slots[first.min(last)..last]
    }

    /// Which block a click at `y` landed in.
    ///
    /// **Nearest, never nothing**: a click in the gap between two paragraphs, or below the last
    /// one, is a click in the closest block — a document has no "outside", and a caret that
    /// refuses to move because the pointer was two pixels low is a bug nobody can see the cause of.
    pub fn at_y(&self, y: f64) -> Option<usize> {
        let mut best: Option<(&Slot, f64)> = None;
        for slot in &self.slots {
            let distance = match y {
                y if y < slot.top => slot.top - y,
                y if y > slot.bottom() => y - slot.bottom(),
                _ => return Some(slot.index),
            };
            if best.is_none_or(|(_, d)| distance < d) {
                best = Some((slot, distance));
            }
        }
        best.map(|(slot, _)| slot.index)
    }

    /// Where the scroll has to be for `target` — one line's top and height — to be on screen.
    ///
    /// Moves the least it can: a caret already in view leaves the page exactly where it is, which
    /// is the difference between reading a document and having it jump under you.
    pub fn follow(&self, scroll: f64, page: f64, target: (f64, f64)) -> f64 {
        let (top, height) = target;
        let scroll = match scroll {
            _ if top < scroll => top,
            s if top + height > s + page => top + height - page,
            s => s,
        };
        scroll.clamp(0.0, self.limit(page))
    }

    /// The furthest down the document the view may be scrolled.
    pub fn limit(&self, page: f64) -> f64 {
        (self.height - page).max(0.0)
    }
}

/// Measure every block of a document and stack them.
///
/// **Portable on purpose, and this is the one place it matters most.** The whole of the pane's
/// vertical arithmetic is here, and the only thing it asks of Windows is the [`grind_text::Faces`]
/// it is handed — so on a machine with no Windows at all it can be handed
/// [`grind_text::Uniform`] over [`grind_text::Fixed`] and checked against the same answers
/// `grind text view --width` prints. That is the W5 exit criterion, and it is a test rather than
/// a claim (see this module's tests).
///
/// Each block's height is `Layout::height()` and nothing else: the pane never decides how tall a
/// paragraph is, it asks. A block the core cannot lay out at all takes no room rather than
/// stopping the document — R5's tolerance, carried up into the window.
pub fn flow_of(app: &grind_text::App, faces: &dyn grind_text::Faces, dpi: u32) -> Flow {
    let count = app.block_count();
    let viewport = app.get_viewport(0..count);
    let mut flow = Flow::new(scale(TOP, dpi));
    for index in 0..count {
        let Some(view) = viewport.get(index) else {
            continue;
        };
        // The measure and the metrics come from the same place the *caret* operations get them,
        // which is the whole point of asking through `grind_text::Faces`: a block laid out one
        // way for drawing and another for Down-arrow is a caret in the wrong place.
        let (width, metrics) = faces.of(index, &view.kind, view.style.as_deref());
        let height = app
            .layout_block(index, width, metrics)
            .map(|layout| f64::from(layout.height()))
            .unwrap_or(0.0);
        let (above, below) = spacing(&view.kind, dpi);
        flow.push(index, height, indent(&view.kind, dpi), above, below);
    }
    flow
}

/// How much space goes above a block of this kind, and below it.
///
/// A heading gets more room above it than a paragraph does, and nothing else in this pane's
/// typography is decided anywhere but here.
pub fn spacing(kind: &grind_text::BlockKind, dpi: u32) -> (f64, f64) {
    let gap = scale(GAP, dpi);
    match kind {
        grind_text::BlockKind::Heading { .. } => (scale(HEADING_GAP, dpi), gap),
        _ => (0.0, gap),
    }
}

/// How far a list item's text is indented — one step per nesting level, and nothing for
/// everything else.
pub fn indent(kind: &grind_text::BlockKind, dpi: u32) -> f64 {
    match kind {
        grind_text::BlockKind::ListItem { depth } => f64::from(*depth) * scale(INDENT, dpi),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::BlockKind;

    /// Three paragraphs of one line each, 10 tall, with a gap of 10 under each.
    fn flow() -> Flow {
        let mut flow = Flow::default();
        for index in 0..3 {
            flow.push(index, 10.0, 0.0, 0.0, 10.0);
        }
        flow
    }

    #[test]
    fn blocks_stack_with_the_gap_between_them() {
        let flow = flow();
        assert_eq!(flow.slot(0).unwrap().top, 0.0);
        assert_eq!(flow.slot(1).unwrap().top, 20.0);
        assert_eq!(flow.slot(2).unwrap().top, 40.0);
        assert_eq!(flow.height(), 60.0);
    }

    /// Adjacent space collapses, or a heading after a paragraph would carry both gaps.
    #[test]
    fn the_space_above_a_heading_collapses_against_the_gap_below_the_paragraph() {
        let mut flow = Flow::default();
        flow.push(0, 10.0, 0.0, 0.0, 10.0);
        flow.push(1, 20.0, 0.0, 18.0, 10.0);
        assert_eq!(flow.slot(1).unwrap().top, 28.0, "20 + (18 - 10)");

        // And the first block never floats: nothing is above it for its space to sit under.
        let mut alone = Flow::new(30.0);
        alone.push(0, 20.0, 0.0, 18.0, 10.0);
        assert_eq!(alone.slot(0).unwrap().top, 30.0);
    }

    #[test]
    fn only_the_blocks_on_screen_are_visible() {
        let flow = flow();
        let indices: Vec<usize> = flow.visible(15.0, 35.0).iter().map(|s| s.index).collect();
        assert_eq!(indices, vec![1], "20..30 is the only one inside 15..35");
        assert_eq!(flow.visible(0.0, 100.0).len(), 3);
        assert!(flow.visible(500.0, 600.0).is_empty());
        assert_eq!(flow.visible(5.0, 6.0)[0].index, 0, "half on screen is on");
    }

    /// A click has to land somewhere: the gaps between blocks belong to the nearest one.
    #[test]
    fn a_click_in_a_gap_lands_in_the_nearest_block() {
        let flow = flow();
        assert_eq!(flow.at_y(5.0), Some(0));
        assert_eq!(flow.at_y(12.0), Some(0), "just under the first");
        assert_eq!(flow.at_y(18.0), Some(1), "just over the second");
        assert_eq!(flow.at_y(-40.0), Some(0), "above the document");
        assert_eq!(flow.at_y(4000.0), Some(2), "below it");
        assert_eq!(
            Flow::default().at_y(0.0),
            None,
            "an empty document has none"
        );
    }

    #[test]
    fn following_the_caret_moves_the_least_it_can() {
        let flow = flow();
        assert_eq!(flow.follow(0.0, 30.0, (20.0, 10.0)), 0.0, "already in view");
        assert_eq!(flow.follow(0.0, 25.0, (40.0, 10.0)), 25.0, "below the fold");
        assert_eq!(flow.follow(30.0, 25.0, (0.0, 10.0)), 0.0, "above it");
        assert_eq!(
            flow.follow(0.0, 500.0, (40.0, 10.0)),
            0.0,
            "never past the end"
        );
    }

    #[test]
    fn the_text_column_is_centred_once_the_window_is_wider_than_the_measure() {
        let (x, w) = column(400.0, 96);
        assert_eq!((x, w), (MARGIN, 400.0 - 2.0 * MARGIN), "narrow: all of it");
        let (x, w) = column(2.0 * MEASURE, 96);
        assert_eq!(w, MEASURE, "wide: the measure holds");
        assert!(x > MARGIN, "and what is left is split either side");
        assert_eq!(x + w + x, 2.0 * MEASURE, "symmetrically");
        assert!(
            column(1.0, 96).1 >= 1.0,
            "a window narrower than its margins"
        );
    }

    /// Everything measured is rebuilt from the constants at this monitor's scaling rather than
    /// scaled from the last answer, which is the same rule the grid follows.
    #[test]
    fn the_column_scales_with_the_monitor() {
        let (_, wide) = column(4000.0, 192);
        assert_eq!(wide, 2.0 * MEASURE, "the measure is a physical size");
    }

    /// The three bands are contiguous and add up to the window, banner or no banner.
    #[test]
    fn the_page_bands_tile_the_window() {
        let page = Page {
            width: 800.0,
            height: 600.0,
            banner_h: 0.0,
            status_h: STATUS_H,
            dpi: 96,
            scroll: 0.0,
        };
        assert_eq!(page.body().y, 0.0);
        assert_eq!(page.body().h + page.status().h, 600.0);
        let with_notice = Page {
            banner_h: BANNER_H,
            ..page
        };
        assert_eq!(with_notice.body().y, BANNER_H);
        assert_eq!(
            with_notice.banner().h + with_notice.body().h + with_notice.status().h,
            600.0
        );
    }

    /// A document, three blocks, one of them long enough to wrap at the width below.
    fn document() -> grind_text::App {
        let app = grind_text::App::new();
        app.insert(0, BlockKind::Heading { level: 1 }, "Quarterly report")
            .unwrap();
        app.insert(
            1,
            BlockKind::Paragraph,
            "The quick brown fox jumps over the lazy dog, and then does it again, and again, \
             until there is quite certainly more than one line of it.",
        )
        .unwrap();
        app.insert(2, BlockKind::ListItem { depth: 1 }, "One indented item")
            .unwrap();
        app
    }

    /// **W5's exit criterion, as a test that needs no Windows.**
    ///
    /// The pane's stacking and `grind text view --width` must break the same text in the same
    /// places when both are given [`grind_text::Fixed`] — which they do because neither of them
    /// breaks anything: line layout is `grind_core::layout`'s, and this asserts that the pane
    /// really does take its heights from there rather than measuring anything itself
    /// (`doc/text-layout.md`, Path C).
    #[test]
    fn the_pane_breaks_lines_where_the_cli_does() {
        let app = document();
        let width = 30.0f32;
        let faces = grind_text::Uniform::new(width, &grind_text::Fixed);
        let flow = flow_of(&app, &faces, 96);

        assert_eq!(flow.slots.len(), app.block_count());
        for index in 0..app.block_count() {
            // What `grind text view --width 30` measures, in the same unit and through the same
            // call the CLI makes.
            let expected = app.layout_block(index, width, &grind_text::Fixed).unwrap();
            let slot = flow.slot(index).unwrap();
            assert_eq!(
                slot.height,
                f64::from(expected.height()),
                "block {index} is as tall as its lines and no taller"
            );
        }
        // And the paragraph really is the one that wrapped, or the test would be asserting
        // nothing at all.
        let lines = app.layout_block(1, width, &grind_text::Fixed).unwrap();
        assert!(lines.lines().len() > 1, "the fixture has to wrap");
    }

    /// Blocks come out in document order, none overlapping the next, and the document is as tall
    /// as the last one's bottom plus its gap.
    #[test]
    fn a_measured_document_stacks_in_order() {
        let flow = flow_of(
            &document(),
            &grind_text::Uniform::new(30.0, &grind_text::Fixed),
            96,
        );
        let tops: Vec<f64> = flow.slots.iter().map(|slot| slot.top).collect();
        assert!(tops.windows(2).all(|w| w[1] > w[0]), "{tops:?}");
        assert_eq!(flow.slot(0).unwrap().top, TOP, "the page's own top margin");
        assert!(flow.height() > flow.slot(2).unwrap().bottom());
        // The list item is the only one indented, and by exactly one step.
        assert_eq!(flow.slot(1).unwrap().indent, 0.0);
        assert_eq!(flow.slot(2).unwrap().indent, INDENT);
    }

    /// An empty document has nothing to stack and must not be a special case anywhere above.
    #[test]
    fn an_empty_document_measures_to_its_own_margin() {
        let flow = flow_of(
            &grind_text::App::new(),
            &grind_text::Uniform::new(30.0, &grind_text::Fixed),
            96,
        );
        assert_eq!(flow.at_y(0.0), None);
        assert_eq!(flow.limit(500.0), 0.0, "nothing to scroll");
    }

    #[test]
    fn a_heading_gets_more_room_above_it_than_a_paragraph() {
        let (above, below) = spacing(&BlockKind::Heading { level: 1 }, 96);
        assert_eq!((above, below), (HEADING_GAP, GAP));
        assert_eq!(spacing(&BlockKind::Paragraph, 96), (0.0, GAP));
    }

    #[test]
    fn a_list_item_indents_once_per_level() {
        assert_eq!(indent(&BlockKind::ListItem { depth: 1 }, 96), INDENT);
        assert_eq!(indent(&BlockKind::ListItem { depth: 3 }, 96), 3.0 * INDENT);
        assert_eq!(indent(&BlockKind::Paragraph, 96), 0.0);
    }
}
