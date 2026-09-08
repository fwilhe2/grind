// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where each block sits down the page, as pure arithmetic. **No GTK types.**
//!
//! `ui_sheet_gtk/src/geom.rs`'s counterpart, and it exists for the same reason: a custom-drawn
//! widget's layout decisions are the part most worth testing and the part hardest to test
//! through a display, so they live in a module that has never heard of one.
//!
//! **This is not line layout.** Breaking a paragraph into lines is `grind_core::layout`'s job
//! and reaches this shell through [`grind_text::App`] (`doc/text-layout.md`, Path C). What is
//! left here is the stacking above it: how tall each block's box is, where it starts, which
//! ones are on screen, and how far to scroll to keep the caret visible. A second shell that
//! stacks blocks differently — a paginated one — replaces this file and nothing else.

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

/// One block's box in the flow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub index: usize,
    /// Distance from the top of the document to the top of this block's text.
    pub top: f64,
    /// The height of its lines — not counting the gap under it.
    pub height: f64,
    /// How far its text starts from the column's left edge. A list's indent for an ordinary
    /// block; a table cell's own left edge, plus the padding, for a block in one.
    pub indent: f64,
    /// How wide the block was laid out — the measure its lines were broken at.
    ///
    /// Carried rather than derived, because a block in a table is measured at its *cell's*
    /// width and nothing about the block itself says so. Every caret operation asks for it
    /// (`view.rs`'s `Column`), which is what makes Down-arrow inside a cell land where the ink
    /// is.
    pub width: f64,
}

impl Slot {
    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }
}

/// One table cell's box — what the grid's rules are drawn round.
///
/// Presentation only, and derived: a cell *is* its blocks (`grind_text::Cell`), and this is the
/// rectangle they were laid out inside. Kept beside the slots because a rule is drawn once per
/// cell rather than once per block, and a cell may hold several.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellBox {
    pub top: f64,
    pub left: f64,
    pub width: f64,
    pub height: f64,
}

impl CellBox {
    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }
}

/// Every block, stacked. Built fresh whenever the document or the width changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    slots: Vec<Slot>,
    cells: Vec<CellBox>,
    height: f64,
    /// The text column's width, which is what an ordinary block is measured at.
    measure: f64,
}

impl Flow {
    /// An empty flow whose first block starts `top` below the document's top — the page's
    /// own margin, which scrolls with the text rather than framing it — set across a column
    /// `measure` wide.
    pub fn new(top: f64, measure: f64) -> Self {
        Flow {
            slots: Vec::new(),
            cells: Vec::new(),
            height: top,
            measure,
        }
    }

    /// Add a block below the ones already in, with the space that goes above and below it.
    ///
    /// `space_before` is collapsed against the gap the previous block already left, the way
    /// every typesetting system collapses adjacent margins — otherwise a heading after a
    /// paragraph gets both gaps and floats.
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
            width: (self.measure - indent).max(1.0),
        });
        self.height = top + height + gap;
    }

    /// Put a block at an exact box rather than under the last one — what a table needs, since
    /// its cells are placed by coordinate and not by what came before them.
    ///
    /// Deliberately not [`Flow::push`] with more arguments: stacking and placing are different
    /// operations, and a `push` that sometimes ignored the running height would be the kind of
    /// function whose callers each believe something different about it. [`Flow::advance`] is
    /// how the running height catches up afterwards.
    pub fn place(&mut self, index: usize, top: f64, height: f64, left: f64, width: f64) {
        self.slots.push(Slot {
            index,
            top,
            height,
            indent: left,
            width: width.max(1.0),
        });
    }

    /// Record a cell's box, for the rule drawn round it.
    pub fn cell(&mut self, cell: CellBox) {
        self.cells.push(cell);
    }

    /// Move the running height to `to` — where the next stacked block starts.
    pub fn advance(&mut self, to: f64) {
        self.height = self.height.max(to);
    }

    /// The measure an ordinary block is laid out at.
    pub fn measure(&self) -> f64 {
        self.measure
    }

    /// Every table cell's box, in document order.
    pub fn cells(&self) -> &[CellBox] {
        &self.cells
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// How tall the whole document is — the scrollbar's extent.
    pub fn height(&self) -> f64 {
        self.height
    }

    /// The slot for the block at `index` — looked up **by the block it is for**, not by its
    /// position in the list.
    ///
    /// The two used to be the same number and are not any more: a table's cells are placed in
    /// row-major order, which is the order its blocks are in for anything either reader
    /// produced but is not something this file should assume about a model built by hand.
    pub fn slot(&self, index: usize) -> Option<&Slot> {
        match self.slots.binary_search_by_key(&index, |slot| slot.index) {
            Ok(at) => self.slots.get(at),
            Err(_) => self.slots.iter().find(|slot| slot.index == index),
        }
    }

    /// The blocks that intersect `top..bottom` — what a paint reads and nothing else.
    ///
    /// A contiguous slice because the flow is in document order, which is what lets a shell
    /// hand its ends straight to [`grind_text::App::get_viewport`] (doc/plan.md rule 1).
    pub fn visible(&self, top: f64, bottom: f64) -> &[Slot] {
        let first = self.slots.partition_point(|slot| slot.bottom() < top);
        let last = self.slots.partition_point(|slot| slot.top < bottom);
        &self.slots[first.min(last)..last]
    }

    /// Which block a click at `(x, y)` landed in, `x` measured from the column's left edge.
    ///
    /// **Nearest, never nothing**: a click in the gap between two paragraphs, or below the
    /// last one, is a click in the closest block — a document has no "outside", and a caret
    /// that refuses to move because the pointer was two pixels low is a bug the user cannot
    /// see the cause of.
    ///
    /// Vertical distance dominates, and horizontal distance only settles a tie. That is the
    /// whole of what a table needs: the cells of one row share a band of the page, so the
    /// answer inside a table is "which column", and everywhere else there is exactly one block
    /// at a given height and `x` never gets a vote.
    pub fn at(&self, x: f64, y: f64) -> Option<usize> {
        let mut best: Option<(&Slot, (f64, f64))> = None;
        for slot in &self.slots {
            let vertical = match y {
                y if y < slot.top => slot.top - y,
                y if y > slot.bottom() => y - slot.bottom(),
                _ => 0.0,
            };
            let right = slot.indent + slot.width;
            let horizontal = match x {
                x if x < slot.indent => slot.indent - x,
                x if x > right => x - right,
                _ => 0.0,
            };
            let distance = (vertical, horizontal);
            if best.is_none_or(|(_, d)| distance < d) {
                best = Some((slot, distance));
            }
        }
        best.map(|(slot, _)| slot.index)
    }

    /// Where the scroll has to be for `target` — one line's top and height — to be on screen.
    ///
    /// Moves the least it can: a caret already in view leaves the page exactly where it is,
    /// which is the difference between reading a document and having it jump under you.
    pub fn follow(&self, scroll: f64, page: f64, target: (f64, f64)) -> f64 {
        let (top, height) = target;
        let limit = (self.height - page).max(0.0);
        let scroll = match scroll {
            _ if top < scroll => top,
            s if top + height > s + page => top + height - page,
            s => s,
        };
        scroll.clamp(0.0, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three paragraphs of one line each, 10 tall, with a gap of 10 under each.
    fn flow() -> Flow {
        let mut flow = Flow::new(0.0, 100.0);
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
        // 40 to the last block's top, 10 of text, and the gap under it — a document ends a
        // line short of its own bottom margin otherwise.
        assert_eq!(flow.height(), 60.0);
    }

    /// Adjacent space collapses, or a heading after a paragraph would carry both gaps.
    #[test]
    fn the_space_above_a_heading_collapses_against_the_gap_below_the_paragraph() {
        let mut flow = Flow::new(0.0, 100.0);
        flow.push(0, 10.0, 0.0, 0.0, 10.0);
        flow.push(1, 20.0, 0.0, 18.0, 10.0);
        assert_eq!(flow.slot(1).unwrap().top, 28.0, "20 + (18 - 10)");

        // And the first block never floats: nothing is above it for its space to sit under,
        // so it starts exactly at the page's own top margin.
        let mut alone = Flow::new(30.0, 100.0);
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
        // A block half on screen is on screen.
        assert_eq!(flow.visible(5.0, 6.0)[0].index, 0);
    }

    /// A click has to land somewhere: the gaps between blocks belong to the nearest one.
    #[test]
    fn a_click_in_a_gap_lands_in_the_nearest_block() {
        let flow = flow();
        assert_eq!(flow.at(0.0, 5.0), Some(0));
        assert_eq!(flow.at(0.0, 12.0), Some(0), "just under the first");
        assert_eq!(flow.at(0.0, 18.0), Some(1), "just over the second");
        assert_eq!(flow.at(0.0, -40.0), Some(0), "above the document");
        assert_eq!(flow.at(0.0, 4000.0), Some(2), "below it");
        assert_eq!(
            Flow::default().at(0.0, 0.0),
            None,
            "an empty document has none"
        );
    }

    #[test]
    fn following_the_caret_moves_the_least_it_can() {
        let flow = flow();
        // Already in view: nothing moves.
        assert_eq!(flow.follow(0.0, 30.0, (20.0, 10.0)), 0.0);
        // Below the fold: scrolled just far enough that its bottom is the page's.
        assert_eq!(flow.follow(0.0, 25.0, (40.0, 10.0)), 25.0);
        // Above it: the line's top becomes the page's.
        assert_eq!(flow.follow(30.0, 25.0, (0.0, 10.0)), 0.0);
        // Never past the end of the document.
        assert_eq!(flow.follow(0.0, 500.0, (40.0, 10.0)), 0.0);
    }

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
