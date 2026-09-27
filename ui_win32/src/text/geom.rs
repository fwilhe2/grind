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

use std::collections::HashMap;

use crate::sheet::geom::{Rect, scale};

/// Space either side of the text column, in pixels at 100%.
///
/// Wide enough since W10 to hold the **page** the column is set on: the card is [`PAGE_PAD`]
/// outside the text on each side, and this has to leave a strip of the window's own backdrop
/// showing either side of that or the page is not a page, it is the window.
pub const MARGIN: f64 = 48.0;

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
///
/// It is also the page card's top padding, which is why it grew in W10: a sheet of paper with
/// twenty-four pixels above the first line looks like a mistake, and forty like a margin.
pub const TOP: f64 = 40.0;

/// How far the page card extends past the text column on each side, and below the last block.
///
/// The card is the document's own surface — `Theme::background` — standing on the window's
/// backdrop, which is the whole of W10's answer for this pane: a word processor whose text sits
/// directly on the window chrome has no document in it, only some text.
pub const PAGE_PAD: f64 = 40.0;

/// Space under a block, and the extra a heading gets above it — the whole of this pane's
/// typography beyond the font itself.
pub const GAP: f64 = 10.0;
pub const HEADING_GAP: f64 = 18.0;

/// The gap between a picture and its caption — small, since the two read as one figure.
/// `ui_text_gtk`'s own `CAPTION_GAP`, at the same value.
pub const CAPTION_GAP: f64 = 4.0;

/// The status bar, the same height the grid's is so that the two panes' windows agree.
pub const STATUS_H: f64 = crate::sheet::draw::STATUS_H;

/// The notice bar, when there is a notice. Zero when there is not — see `win.rs`'s `banner_h`.
/// The grid's own height, and drawn the same way since W10: an inset card, not a stripe.
pub const BANNER_H: f64 = crate::sheet::draw::BANNER_H;

/// The format strip — decision 4's admission test applied to this pane: it reads and writes
/// `CharStyle`, so it goes under the menu rather than in it. Never zero, unlike the banner: this
/// bar has no "nothing to show" state, since the five toggles always mean something.
///
/// One Fluent control tall plus its surround, which is the grid's own strip height, so that the
/// two panes' chrome measures the same in the same window.
pub const STRIP_H: f64 = crate::sheet::draw::STRIP_H;

/// One toggle button's width on the strip — square, at Fluent's control height, which is what a
/// one-letter toggle wants and what lets the five of them read as one segmented group.
pub const BUTTON_W: f64 = crate::theme::space::CONTROL_H;

/// The Family picker's width — wide enough for a short font name and the chevron that says it
/// opens something; [`crate::sheet::draw::Align::Left`] elides anything longer.
pub const PICKER_W: f64 = 108.0;

/// The Size picker's, which needs room for `999pt` and a chevron and nothing else.
pub const SIZE_W: f64 = 72.0;

/// One colour swatch's width — square, like a toggle: a swatch draws a fill rather than a label
/// and needs no room for one.
pub const SWATCH_W: f64 = crate::theme::space::CONTROL_H;

/// *Clear Formatting*'s width — wider than a toggle, since "Clear" does not fit [`BUTTON_W`].
pub const CLEAR_W: f64 = 64.0;

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

/// Which of the strip's controls a click landed on — [`Page::strip_hit`]'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripHit {
    /// One of the five toggles, by [`Page::strip_buttons`]'s own order.
    Toggle(usize),
    Family,
    Size,
    Color,
    Highlight,
    Clear,
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
    pub strip_h: f64,
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

    /// The format strip, under the banner and above the body — decision 4's growable menu bar
    /// holds a verb, this holds a property of the selection, and the two never trade places.
    pub fn strip(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.banner_h,
            w: self.width,
            h: self.strip_h,
        }
    }

    /// The part of the window the document is drawn in.
    pub fn body(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.banner_h + self.strip_h,
            w: self.width,
            h: (self.height - self.banner_h - self.strip_h - self.status_h).max(0.0),
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

    /// One control on the strip: `w` wide, at Fluent's control height, centred in the band.
    ///
    /// Every rectangle below is one of these, which is what stops the strip drifting back into
    /// full-height slabs the moment a control is added: a button is a *control standing on a
    /// band*, and the band's own height is not its business.
    fn strip_control(&self, x: f64, w: f64) -> Rect {
        let strip = self.strip();
        let h = scale(crate::theme::space::CONTROL_H, self.dpi).min(strip.h);
        Rect {
            x,
            y: strip.y + ((strip.h - h) / 2.0).max(0.0),
            w,
            h,
        }
    }

    /// Where the strip's controls begin — the same margin the grid's name box keeps, so the two
    /// panes' chrome lines up down the left of the window.
    fn strip_start(&self) -> f64 {
        scale(crate::theme::space::GROUP, self.dpi)
    }

    /// The gap between two controls that belong together, and between two groups of them.
    ///
    /// Three groups, and the grouping is the point (W10): the five emphasis toggles are one
    /// segmented control, the two pickers are the *shape* of the text, the two swatches are its
    /// *colour*, and Clear stands alone because it undoes all three. A row of nine evenly spaced
    /// buttons says none of that.
    fn strip_gaps(&self) -> (f64, f64) {
        (
            scale(crate::theme::space::GAP / 2.0, self.dpi),
            scale(crate::theme::space::GROUP, self.dpi),
        )
    }

    /// The strip's five toggle buttons, left to right: Bold, Italic, Underline, Strike, Code.
    pub fn strip_buttons(&self) -> [Rect; 5] {
        let (tight, _) = self.strip_gaps();
        let w = scale(BUTTON_W, self.dpi);
        let start = self.strip_start();
        std::array::from_fn(|i| self.strip_control(start + (w + tight) * i as f64, w))
    }

    /// *Font* — opens `dialog::choose` over the families this build knows.
    pub fn strip_family(&self) -> Rect {
        let (_, group) = self.strip_gaps();
        let last = self.strip_buttons()[4];
        self.strip_control(last.x + last.w + group, scale(PICKER_W, self.dpi))
    }

    /// *Size* — the ladder `grind_text::format::sizes` offers.
    pub fn strip_size(&self) -> Rect {
        let (tight, _) = self.strip_gaps();
        let family = self.strip_family();
        self.strip_control(family.x + family.w + tight, scale(SIZE_W, self.dpi))
    }

    /// The text colour swatch.
    pub fn strip_color(&self) -> Rect {
        let (_, group) = self.strip_gaps();
        let size = self.strip_size();
        self.strip_control(size.x + size.w + group, scale(SWATCH_W, self.dpi))
    }

    /// The highlight swatch.
    pub fn strip_highlight(&self) -> Rect {
        let (tight, _) = self.strip_gaps();
        let color = self.strip_color();
        self.strip_control(color.x + color.w + tight, scale(SWATCH_W, self.dpi))
    }

    /// *Clear Formatting* — the one-shot the toggles can only approximate.
    pub fn strip_clear(&self) -> Rect {
        let (_, group) = self.strip_gaps();
        let highlight = self.strip_highlight();
        self.strip_control(highlight.x + highlight.w + group, scale(CLEAR_W, self.dpi))
    }

    /// Where a separator goes between two groups: the middle of the gap before `next`, drawn at
    /// half a control's height. Fluent's `AppBarSeparator`, which is what says "these two things
    /// are not the same kind of thing" without a border round either of them.
    pub fn strip_separator(&self, next: Rect) -> Rect {
        let (_, group) = self.strip_gaps();
        let control = self.strip_control(0.0, 0.0);
        let h = control.h / 2.0;
        Rect {
            x: (next.x - group / 2.0).round(),
            y: control.y + (control.h - h) / 2.0,
            w: 1.0,
            h,
        }
    }

    /// Which control, if any, a click at `x, y` on the strip landed on.
    pub fn strip_hit(&self, x: f64, y: f64) -> Option<StripHit> {
        if let Some(index) = self
            .strip_buttons()
            .iter()
            .position(|rect| rect.contains(x, y))
        {
            return Some(StripHit::Toggle(index));
        }
        for (rect, hit) in [
            (self.strip_family(), StripHit::Family),
            (self.strip_size(), StripHit::Size),
            (self.strip_color(), StripHit::Color),
            (self.strip_highlight(), StripHit::Highlight),
            (self.strip_clear(), StripHit::Clear),
        ] {
            if rect.contains(x, y) {
                return Some(hit);
            }
        }
        None
    }

    /// The text column, in window coordinates.
    pub fn text_column(&self) -> (f64, f64) {
        column(self.width, self.dpi)
    }

    /// **The page** — the document's own surface, `height` pixels of content tall.
    ///
    /// The card the text is set on, in window coordinates and already scrolled: its top is the
    /// top of the document, so it moves off the screen as the document does, and its bottom is
    /// the last block plus one [`PAGE_PAD`]. Nothing is clipped here — the painter clips to the
    /// body, and a card whose top is a thousand pixels above the window is exactly what a
    /// document scrolled a thousand pixels down should have.
    ///
    /// This is decorative and the caret knows nothing about it: [`Page::text_column`] is
    /// unchanged, so where a line breaks and where a click lands are the same answers they were
    /// before there was a page to draw them on.
    pub fn page_card(&self, height: f64) -> Rect {
        let body = self.body();
        let (x, w) = self.text_column();
        let pad = scale(PAGE_PAD, self.dpi);
        Rect {
            x: x - pad,
            y: body.y - self.scroll,
            w: w + pad * 2.0,
            h: height + pad,
        }
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
    /// How far its text starts from the column's left edge: a list's indent for an ordinary
    /// block, and a table cell's own left edge plus its padding for a block in one.
    pub indent: f64,
    /// How wide the block was laid out — the measure its lines were broken at.
    ///
    /// Carried rather than derived from `indent`, because a block in a table is set at its
    /// *cell's* width and nothing about the block itself says so. A picture is fitted to it, and
    /// a click is measured against it.
    pub width: f64,
}

impl Slot {
    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }
}

/// One table cell's box — what the grid's rules are drawn round, in document coordinates with
/// `left` measured from the column's left edge, as [`Slot::indent`] is.
///
/// Presentation only, and derived: a cell *is* its blocks (`grind_text::model::Cell`), and this
/// is the rectangle they were laid out inside. Kept beside the slots because a rule is drawn once
/// per cell rather than once per block, and a cell may hold several.
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

    pub fn right_edge(&self) -> f64 {
        self.left + self.width
    }
}

/// Every block, stacked — and a table's blocks placed on its grid. Built fresh whenever the
/// document, the width or the DPI changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    slots: Vec<Slot>,
    cells: Vec<CellBox>,
    height: f64,
    /// The text column's width, which is what an ordinary block is measured at.
    measure: f64,
}

impl Flow {
    /// An empty flow whose first block starts `top` below the document's top, set across a column
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
            width: (self.measure - indent).max(1.0),
        });
        self.height = top + height + gap;
    }

    /// Put a block at an exact box rather than under the last one — what a table needs, since its
    /// cells are placed by coordinate and not by what came before them.
    ///
    /// Deliberately not [`Flow::push`] with more arguments: stacking and placing are different
    /// operations, and a `push` that sometimes ignored the running height would be the kind of
    /// function whose callers each believe something different about it. [`Flow::advance`] is how
    /// the running height catches up afterwards. `ui_text_gtk/src/geom.rs` makes the same split.
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

    /// Every table cell's box, in document order.
    pub fn cells(&self) -> &[CellBox] {
        &self.cells
    }

    /// How tall the whole document is — the scrollbar's extent.
    pub fn height(&self) -> f64 {
        self.height
    }

    /// The slot for the block at `index` — looked up **by the block it is for**, not by its
    /// position in the list.
    ///
    /// The two used to be the same number and need not be any more: a table's blocks are placed
    /// cell by cell, which is the order its blocks are in for anything either reader produced but
    /// is not something this file should assume about a model built by hand.
    pub fn slot(&self, index: usize) -> Option<&Slot> {
        match self.slots.binary_search_by_key(&index, |slot| slot.index) {
            Ok(at) => self.slots.get(at),
            Err(_) => self.slots.iter().find(|slot| slot.index == index),
        }
    }

    /// The blocks that intersect `top..bottom` — what a paint reads and nothing else.
    ///
    /// A contiguous slice because the flow is in document order, which is what lets a shell hand
    /// its ends straight to [`grind_text::App::get_viewport`] (architecture rule 1). Found by a
    /// scan from either end rather than a binary search, because a table breaks the one property a
    /// binary search needs: the second block of a tall cell sits *below* the first block of the
    /// cell beside it, so the tops are no longer in order. The slice may therefore hold a block of
    /// the same table that is just off screen, which costs a layout and draws nothing.
    pub fn visible(&self, top: f64, bottom: f64) -> &[Slot] {
        let first = self.slots.iter().position(|slot| slot.bottom() >= top);
        let last = self.slots.iter().rposition(|slot| slot.top < bottom);
        match (first, last) {
            (Some(first), Some(last)) if first <= last => &self.slots[first..=last],
            _ => &[],
        }
    }

    /// Which block a click at `(x, y)` landed in, `x` measured from the column's left edge.
    ///
    /// **Nearest, never nothing**: a click in the gap between two paragraphs, or below the last
    /// one, is a click in the closest block — a document has no "outside", and a caret that
    /// refuses to move because the pointer was two pixels low is a bug nobody can see the cause of.
    ///
    /// Vertical distance dominates, and horizontal distance only settles a tie. That is the whole
    /// of what a table needs: the cells of one row share a band of the page, so the answer inside
    /// a table is "which column", and everywhere else there is exactly one block at a given height
    /// and `x` never gets a vote.
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

/// The space between a cell's rule and its text, and the rule's own width, in pixels at 100% —
/// `ui_text_gtk::geom::CELL_PAD` and `RULE`, so a table is one shape in every window.
pub const CELL_PAD: f64 = 6.0;
pub const RULE: f64 = 1.0;

/// Where a block in a table cell sits across the column, and how wide it is set — the one fact
/// about a table that the *caret* needs as well as the painter.
///
/// A cell's text is broken into lines at the cell's own width, so a click and a Down-arrow have
/// to ask the core with that same width or the caret lands where the ink is not. That is why
/// this is computed once per reflow and handed to `metrics::Faces`, which is what every motion
/// measures through — and why [`flow_of`] places a cell's blocks with the same function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Across {
    /// From the column's left edge to where the text starts.
    pub left: f64,
    pub width: f64,
}

/// A cell's own box across a column `column` wide: its left edge and its width.
///
/// **The columns are equal shares** — `ui_text_gtk`'s answer and the browser's, and a shell
/// decision with a reason: the model carries no column widths (`grind_text::model::Cell` — a
/// table's own style is not read), so there is nothing to honour, and equal shares is the answer
/// that never overflows the measure. A merged cell takes the width of every column it spans,
/// which is what makes it look merged rather than misaligned.
fn cell_x(cell: &grind_text::model::Cell, columns: u32, column: f64) -> (f64, f64) {
    let share = column / f64::from(columns.max(1));
    (
        f64::from(cell.column) * share,
        f64::from(cell.columns_spanned.max(1)) * share,
    )
}

/// Where one block sits inside a cell whose box starts at `left` and is `width` wide: inside the
/// padding, and a list item further in by its own indent, so that its bullet has somewhere to go
/// that is still inside the cell.
fn in_cell(kind: &grind_text::BlockKind, left: f64, width: f64, dpi: u32) -> Across {
    let pad = scale(CELL_PAD, dpi);
    let indent = indent(kind, dpi);
    Across {
        left: left + pad + indent,
        width: (width - 2.0 * pad - indent).max(1.0),
    }
}

/// Every block that is in a table cell, by block index, with where it sits across a column
/// `column` wide. Blocks outside a table are not in the map: they are set at the column less
/// their indent, which `metrics::Faces` already knows.
pub fn across(app: &grind_text::App, column: f64, dpi: u32) -> HashMap<usize, Across> {
    let count = app.block_count();
    let viewport = app.get_viewport(0..count);
    let mut out = HashMap::new();
    let mut index = 0;
    while index < count {
        let Some(table) = viewport
            .get(index)
            .filter(|view| view.cell.is_some())
            .and_then(|_| app.table(index))
        else {
            index += 1;
            continue;
        };
        for at in table.blocks.clone() {
            let Some(view) = viewport.get(at) else {
                continue;
            };
            let Some(cell) = view.cell.as_ref() else {
                continue;
            };
            let (left, width) = cell_x(cell, table.columns, column);
            out.insert(at, in_cell(&view.kind, left, width, dpi));
        }
        index = table.blocks.end.max(index + 1);
    }
    out
}

/// One cell of a table and the blocks in it, in the order they appear.
struct CellRun<'a> {
    cell: &'a grind_text::model::Cell,
    blocks: Vec<usize>,
}

/// Every cell of a table, in the order its first block appears.
fn cell_runs<'a>(
    viewport: &'a grind_text::Viewport,
    blocks: std::ops::Range<usize>,
) -> Vec<CellRun<'a>> {
    let mut out: Vec<CellRun<'a>> = Vec::new();
    for index in blocks {
        let Some(cell) = viewport.get(index).and_then(|view| view.cell.as_ref()) else {
            continue;
        };
        match out
            .iter_mut()
            .find(|run| run.cell.row == cell.row && run.cell.column == cell.column)
        {
            Some(run) => run.blocks.push(index),
            None => out.push(CellRun {
                cell,
                blocks: vec![index],
            }),
        }
    }
    out
}

/// Measure every block of a document and stack them, laying a table out as a grid.
///
/// **Portable on purpose, and this is the one place it matters most.** The whole of the pane's
/// vertical arithmetic is here, and the only thing it asks of Windows is the [`grind_text::Faces`]
/// it is handed — so on a machine with no Windows at all it can be handed
/// [`grind_text::Uniform`] over [`grind_text::Fixed`] and checked against the same answers
/// `grind text view --width` prints. That is the W5 exit criterion, and it is a test rather than
/// a claim (see this module's tests).
///
/// Each block's height is `Layout::height()` and nothing else: the pane never decides how tall a
/// paragraph is, it asks, at the width the `Faces` says — which for a block in a table is its
/// cell's, from the [`across`] map the window built alongside this flow. A block the core cannot
/// lay out at all takes no room rather than stopping the document — R5's tolerance, carried up
/// into the window.
///
/// `picture` is the one exception, and it is a hook rather than a special case here: a block
/// that is a picture (`grind_text::picture_of`) is not measured as text at all — its height
/// comes from the image's own decoded pixels, scaled to fit the column — and decoding needs an
/// OS decoder this module must not depend on (it has no Windows types at all, W5's own rule).
/// `picture(view, width)` answers `Some(height)` when `view` is a picture this caller could
/// decode and size, `None` otherwise, in which case the ordinary text height is used — so a
/// document with no pictures, or a Linux caller with no decoder, passes `&|_, _| None` and gets
/// exactly the old behaviour.
pub fn flow_of(
    app: &grind_text::App,
    faces: &dyn grind_text::Faces,
    column: f64,
    dpi: u32,
    picture: &dyn Fn(&grind_text::BlockView, f64) -> Option<f64>,
) -> Flow {
    let count = app.block_count();
    let viewport = app.get_viewport(0..count);
    let mut flow = Flow::new(scale(TOP, dpi), column);
    // The measure and the metrics come from the same place the *caret* operations get them,
    // which is the whole point of asking through `grind_text::Faces`: a block laid out one way
    // for drawing and another for Down-arrow is a caret in the wrong place.
    let height_of = |index: usize, view: &grind_text::BlockView| -> (f64, f64) {
        let (width, metrics) = faces.of(index, &view.kind, view.style.as_deref());
        let height = match picture(view, f64::from(width)) {
            Some(height) => height,
            None => app
                .layout_block(index, width, metrics)
                .map(|layout| f64::from(layout.height()))
                .unwrap_or(0.0),
        };
        (height, f64::from(width))
    };
    let mut index = 0;
    while index < count {
        let Some(view) = viewport.get(index) else {
            index += 1;
            continue;
        };
        // A table is laid out as a grid rather than stacked, and its blocks are placed rather than
        // pushed. `App::table` is the same fold the writer and the projection use, asked of the
        // core so that four callers cannot answer it four ways.
        if view.cell.is_some()
            && let Some(table) = app.table(index)
        {
            lay_out_table(&mut flow, &viewport, &table, column, dpi, &height_of);
            index = table.blocks.end.max(index + 1);
            continue;
        }
        let (height, _) = height_of(index, view);
        let (above, below) = spacing(&view.kind, dpi);
        flow.push(index, height, indent(&view.kind, dpi), above, below);
        index += 1;
    }
    flow
}

/// One table, as a grid: equal columns across the measure, each row as tall as its tallest cell,
/// each cell's blocks stacked inside it — `ui_text_gtk`'s `lay_out_table`, so a table is one
/// shape in both desktop windows.
///
/// Two passes, because a cell cannot be positioned until its row's height is known and a row's
/// height is the tallest cell in it. A cell that spans rows contributes its share to each row it
/// covers — the honest approximation, since which of the covered rows should grow is a question
/// only a full table layout answers.
fn lay_out_table(
    flow: &mut Flow,
    viewport: &grind_text::Viewport,
    table: &grind_text::Table,
    column: f64,
    dpi: u32,
    height_of: &dyn Fn(usize, &grind_text::BlockView) -> (f64, f64),
) {
    let pad = scale(CELL_PAD, dpi);
    let gap = scale(GAP, dpi);
    // The same gap above a table a paragraph would leave, collapsed the same way: the running
    // height already ends one gap below whatever came before.
    let top = flow.height();
    let rows = table.rows.max(1) as usize;
    let cells = cell_runs(viewport, table.blocks.clone());

    // Every block's height and measure once, since both passes want them.
    let measured: HashMap<usize, (f64, f64)> = cells
        .iter()
        .flat_map(|run| run.blocks.iter().copied())
        .filter_map(|index| Some((index, height_of(index, viewport.get(index)?))))
        .collect();
    let content = |run: &CellRun| -> f64 {
        let stacked: f64 = run
            .blocks
            .iter()
            .map(|index| measured.get(index).map_or(0.0, |m| m.0) + gap)
            .sum();
        (stacked - gap).max(0.0) + 2.0 * pad
    };

    let mut heights = vec![2.0 * pad; rows];
    for run in &cells {
        let over = run.cell.rows_spanned.max(1) as usize;
        let first = run.cell.row as usize;
        let share = content(run) / over as f64;
        for height in heights.iter_mut().skip(first).take(over) {
            *height = height.max(share);
        }
    }
    let tops: Vec<f64> = heights
        .iter()
        .scan(top, |at, height| {
            let here = *at;
            *at += height;
            Some(here)
        })
        .collect();

    for run in &cells {
        let row = run.cell.row as usize;
        let Some(&cell_top) = tops.get(row) else {
            continue;
        };
        let over = run.cell.rows_spanned.max(1) as usize;
        let height: f64 = heights.iter().skip(row).take(over).sum();
        let (left, width) = cell_x(run.cell, table.columns, column);
        flow.cell(CellBox {
            top: cell_top,
            left,
            width,
            height,
        });
        let mut at = cell_top + pad;
        for &index in &run.blocks {
            let Some(view) = viewport.get(index) else {
                continue;
            };
            let place = in_cell(&view.kind, left, width, dpi);
            // The width the lines were really broken at, which is the `Faces`' answer — the same
            // as `place.width` in a window, since the window's `Faces` reads [`across`].
            let (block_height, measure) =
                measured.get(&index).copied().unwrap_or((0.0, place.width));
            flow.place(index, at, block_height, place.left, measure);
            at += block_height + gap;
        }
    }
    flow.advance(top + heights.iter().sum::<f64>() + gap);
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

    /// `flow_of`'s picture hook, answering "not a picture" for every block — what a caller with
    /// no image decoder passes, and what every test here that is not about pictures wants.
    fn no_pictures(_: &grind_text::BlockView, _: f64) -> Option<f64> {
        None
    }

    /// An ordinary window on an ordinary document: 800 by 600 at 100%, no notice up.
    fn page() -> Page {
        Page {
            width: 800.0,
            height: 600.0,
            banner_h: 0.0,
            strip_h: STRIP_H,
            status_h: STATUS_H,
            dpi: 96,
            scroll: 0.0,
        }
    }

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
        assert_eq!(flow.visible(5.0, 6.0)[0].index, 0, "half on screen is on");
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

    /// The page is the document's own surface: wider than the text on it, starting where the
    /// document starts, and moving with the scroll rather than framing the window.
    #[test]
    fn the_page_is_a_surface_the_text_stands_on() {
        let page = page();
        let (x, w) = page.text_column();
        let card = page.page_card(1000.0);
        assert!(card.x < x, "the page is wider than the text on it");
        assert_eq!(card.x, x - PAGE_PAD);
        assert_eq!(card.x + card.w, x + w + PAGE_PAD);
        assert_eq!(card.y, page.body().y, "unscrolled, it starts at the body");
        assert!(card.h > 1000.0, "and there is room under the last line");
        // Scrolled, it moves with the document — a page is a thing in the document, not a frame
        // round the window.
        let scrolled = Page {
            scroll: 300.0,
            ..page
        };
        assert_eq!(scrolled.page_card(1000.0).y, page.body().y - 300.0);
        // And there is always window left either side of it, or it would not read as a page.
        for width in [400.0, 800.0, 2400.0] {
            let card = Page { width, ..page }.page_card(100.0);
            assert!(card.x > 0.0, "{width}: the page touches the window's edge");
            assert!(card.x + card.w < width, "{width}");
        }
    }

    /// The four bands are contiguous and add up to the window, banner or no banner.
    #[test]
    fn the_page_bands_tile_the_window() {
        let page = page();
        assert_eq!(page.body().y, STRIP_H);
        assert_eq!(page.strip().h + page.body().h + page.status().h, 600.0);
        let with_notice = Page {
            banner_h: BANNER_H,
            ..page
        };
        assert_eq!(with_notice.body().y, BANNER_H + STRIP_H);
        assert_eq!(
            with_notice.banner().h
                + with_notice.strip().h
                + with_notice.body().h
                + with_notice.status().h,
            600.0
        );
    }

    /// The five toggles sit side by side on the strip, in order, none overlapping — a segmented
    /// group with the tight gap between its members and the window's own margin before the first.
    #[test]
    fn the_strip_buttons_tile_left_to_right_and_dont_overlap() {
        let page = page();
        let buttons = page.strip_buttons();
        assert_eq!(buttons[0].x, crate::theme::space::GROUP);
        for pair in buttons.windows(2) {
            assert!(pair[0].x + pair[0].w < pair[1].x, "they overlap");
            assert!(
                pair[1].x - (pair[0].x + pair[0].w) <= crate::theme::space::GAP,
                "a segmented group, not a scattering"
            );
        }
        // A control standing on the band rather than filling it, which is what makes the strip
        // read as a surface with things on it.
        assert_eq!(buttons[0].h, crate::theme::space::CONTROL_H);
        assert!(buttons[0].y > page.strip().y);
        assert_eq!(
            page.strip_hit(buttons[0].x + 1.0, buttons[0].y + 1.0),
            Some(StripHit::Toggle(0))
        );
        assert_eq!(
            page.strip_hit(buttons[2].x + 1.0, buttons[2].y + 1.0),
            Some(StripHit::Toggle(2))
        );
        assert_eq!(
            page.strip_hit(0.0, page.body().y + 5.0),
            None,
            "below the strip"
        );
        // The window's own margin is not a button: a click there is a click on the band.
        assert_eq!(page.strip_hit(1.0, buttons[0].y + 1.0), None);
    }

    /// The buttons scale with the monitor, the same rule every other measurement here follows.
    #[test]
    fn the_strip_buttons_scale_with_the_monitor() {
        let page = Page { dpi: 192, ..page() };
        assert_eq!(page.strip_buttons()[0].w, 2.0 * BUTTON_W);
        assert_eq!(page.strip_buttons()[0].x, 2.0 * crate::theme::space::GROUP);
    }

    /// The four pickers follow the five toggles in order, none overlapping, each its own
    /// [`StripHit`] — and the gap between two *groups* is wider than the gap inside one, which is
    /// the whole of how the strip says which controls belong together.
    #[test]
    fn the_pickers_continue_where_the_toggles_end_and_dont_overlap() {
        let page = page();
        let toggles = page.strip_buttons();
        let rects = [
            page.strip_family(),
            page.strip_size(),
            page.strip_color(),
            page.strip_highlight(),
            page.strip_clear(),
        ];
        let after = |a: Rect, b: Rect| b.x - (a.x + a.w);
        let tight = after(toggles[0], toggles[1]);
        assert!(after(toggles[4], rects[0]) > tight, "toggles | pickers");
        assert_eq!(after(rects[0], rects[1]), tight, "family and size pair up");
        assert!(after(rects[1], rects[2]) > tight, "pickers | colour");
        assert_eq!(after(rects[2], rects[3]), tight, "the two swatches pair up");
        assert!(after(rects[3], rects[4]) > tight, "colour | clear");
        for pair in rects.windows(2) {
            assert!(pair[0].x + pair[0].w < pair[1].x, "no overlap");
        }
        // The separator sits in a group gap and touches neither side of it.
        let rule = page.strip_separator(rects[0]);
        assert!(rule.x > toggles[4].x + toggles[4].w && rule.x < rects[0].x);
        assert!(rule.h < toggles[0].h, "half a control tall");
        assert_eq!(
            page.strip_hit(rects[0].x + 1.0, rects[0].y + 1.0),
            Some(StripHit::Family)
        );
        assert_eq!(
            page.strip_hit(rects[1].x + 1.0, rects[1].y + 1.0),
            Some(StripHit::Size)
        );
        assert_eq!(
            page.strip_hit(rects[2].x + 1.0, rects[2].y + 1.0),
            Some(StripHit::Color)
        );
        assert_eq!(
            page.strip_hit(rects[3].x + 1.0, rects[3].y + 1.0),
            Some(StripHit::Highlight)
        );
        assert_eq!(
            page.strip_hit(rects[4].x + 1.0, rects[4].y + 1.0),
            Some(StripHit::Clear)
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
        let flow = flow_of(&app, &faces, 30.0, 96, &no_pictures);

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

    /// The picture hook's answer wins over the text layout entirely — a decoded image is never
    /// also measured as if its placeholder character were a line of text.
    #[test]
    fn a_picture_is_measured_by_the_hook_and_not_as_text() {
        let app = document();
        let faces = grind_text::Uniform::new(30.0, &grind_text::Fixed);
        let flow = flow_of(&app, &faces, 30.0, 96, &|_, _| Some(123.0));
        assert!(
            flow.slots.iter().all(|slot| slot.height == 123.0),
            "every block answered by the hook, not by its own text"
        );
    }

    /// Blocks come out in document order, none overlapping the next, and the document is as tall
    /// as the last one's bottom plus its gap.
    #[test]
    fn a_measured_document_stacks_in_order() {
        let flow = flow_of(
            &document(),
            &grind_text::Uniform::new(30.0, &grind_text::Fixed),
            30.0,
            96,
            &no_pictures,
        );
        let tops: Vec<f64> = flow.slots.iter().map(|slot| slot.top).collect();
        assert!(tops.windows(2).all(|w| w[1] > w[0]), "{tops:?}");
        assert_eq!(flow.slot(0).unwrap().top, TOP, "the page's own top margin");
        assert!(flow.height() > flow.slot(2).unwrap().bottom());
        // The list item is the only one indented, and by exactly one step.
        assert_eq!(flow.slot(1).unwrap().indent, 0.0);
        assert_eq!(flow.slot(2).unwrap().indent, INDENT);
    }

    /// A document with **no blocks at all** has nothing to stack and must not be a special case
    /// anywhere above.
    ///
    /// Written out of `Document::empty` and read back, because `App::new()` is no longer such a
    /// document: a *new* one is one empty paragraph, so that a caret has somewhere to be. A file
    /// with an empty `office:text` still is one, and this is that case.
    #[test]
    fn a_document_with_no_blocks_measures_to_its_own_margin() {
        let app = grind_text::App::new();
        let bytes = grind_text::write_bytes(&grind_text::Document::empty(), grind_core::Form::Flat)
            .expect("writes");
        app.open_bytes("empty.fodt", &bytes).expect("reads");
        assert_eq!(app.block_count(), 0, "the file really has none");

        let flow = flow_of(
            &app,
            &grind_text::Uniform::new(30.0, &grind_text::Fixed),
            30.0,
            96,
            &no_pictures,
        );
        assert_eq!(flow.at(0.0, 0.0), None);
        assert_eq!(flow.limit(500.0), 0.0, "nothing to scroll");
    }

    /// And the document this shell's **New Text Document** actually opens: one empty paragraph,
    /// with a slot for it, because the first keystroke lands in a block or it fails.
    #[test]
    fn a_new_document_has_one_block_to_type_into() {
        let app = grind_text::App::new();
        assert_eq!(app.block_count(), 1);
        let flow = flow_of(
            &app,
            &grind_text::Uniform::new(30.0, &grind_text::Fixed),
            30.0,
            96,
            &no_pictures,
        );
        let slot = flow.slot(0).expect("the paragraph has a place on the page");
        assert_eq!(slot.top, TOP, "the page's own top margin");
        assert!(slot.height > 0.0, "an empty paragraph is still a line tall");
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

    /// The `Faces` the window builds, minus the fonts: a block in a table is set at its cell's
    /// width from the [`across`] map, everything else at the column less its indent.
    struct InCells {
        column: f32,
        across: HashMap<usize, Across>,
    }

    impl grind_text::Faces for InCells {
        fn of(
            &self,
            index: usize,
            kind: &BlockKind,
            _style: Option<&str>,
        ) -> (f32, &dyn grind_core::layout::Metrics) {
            let width = match self.across.get(&index) {
                Some(across) => across.width as f32,
                None => self.column - indent(kind, 96) as f32,
            };
            (width.max(1.0), &grind_text::Fixed)
        }
    }

    const COLUMN: f64 = 300.0;

    /// A paragraph, an empty two-by-three table, and a paragraph — `Insert Table…`'s own result,
    /// between two blocks of text.
    fn with_a_table() -> grind_text::App {
        let app = grind_text::App::new();
        app.insert(0, BlockKind::Paragraph, "Before").unwrap();
        app.insert_table(1, 2, 3, Some("Prices".into())).unwrap();
        // `App::new()`'s own empty paragraph is last; give it some text so it is the "after".
        let last = app.block_count() - 1;
        app.insert_text(
            grind_text::Caret {
                block: last,
                offset: 0,
            },
            "After",
        )
        .unwrap();
        app
    }

    fn laid_out(app: &grind_text::App) -> Flow {
        let faces = InCells {
            column: COLUMN as f32,
            across: across(app, COLUMN, 96),
        };
        flow_of(app, &faces, COLUMN, 96, &no_pictures)
    }

    /// **The bug this replaced**: an empty table's six cells stacked as six paragraphs, a tall
    /// blank gap with nothing round it. Now a row's cells share a top, the columns are equal
    /// shares of the measure, and the table is two rows tall rather than six blocks.
    #[test]
    fn a_table_is_laid_out_as_a_grid_rather_than_stacked() {
        let app = with_a_table();
        assert_eq!(app.block_count(), 8, "before, six cells, after");
        let flow = laid_out(&app);

        let cells: Vec<&Slot> = (1..7).map(|i| flow.slot(i).unwrap()).collect();
        let share = COLUMN / 3.0;
        for (at, slot) in cells.iter().enumerate() {
            let (row, column) = (at / 3, at % 3);
            assert_eq!(slot.top, cells[row * 3].top, "a row shares one top");
            assert_eq!(slot.indent, column as f64 * share + CELL_PAD, "cell {at}");
            assert_eq!(slot.width, share - 2.0 * CELL_PAD, "set inside its padding");
        }
        assert!(
            cells[3].top > cells[0].bottom(),
            "the second row is under the first"
        );

        assert_eq!(flow.cells().len(), 6, "one rule per cell");
        let boxes = flow.cells();
        assert_eq!(boxes[1].left, share);
        assert_eq!(
            boxes[0].bottom(),
            boxes[3].top,
            "rows abut: one rule between them"
        );
        assert_eq!(boxes[0].right_edge(), boxes[1].left, "and so do columns");

        let before = flow.slot(0).unwrap();
        let after = flow.slot(7).unwrap();
        assert!(
            boxes[0].top >= before.bottom(),
            "the table starts under the paragraph"
        );
        assert!(
            after.top >= boxes[5].bottom(),
            "and the next one under the table"
        );
        assert_eq!(after.indent, 0.0, "back to the full column");
        assert_eq!(after.width, COLUMN);
        // Two rows of one line each plus their padding, where six stacked blocks were six lines
        // and five gaps.
        let table_h = boxes[5].bottom() - boxes[0].top;
        assert_eq!(table_h, 2.0 * (1.0 + 2.0 * CELL_PAD));
    }

    /// The width the caret measures a cell at and the width it was laid out at are one answer —
    /// the property that keeps a Down-arrow inside a cell landing where the ink is.
    #[test]
    fn the_caret_and_the_page_agree_on_a_cells_measure() {
        let app = with_a_table();
        let flow = laid_out(&app);
        let map = across(&app, COLUMN, 96);
        assert_eq!(map.len(), 6, "only the blocks in the table");
        for (index, across) in &map {
            let slot = flow.slot(*index).unwrap();
            assert_eq!((slot.indent, slot.width), (across.left, across.width));
        }
        assert!(!map.contains_key(&0) && !map.contains_key(&7));
    }

    /// Inside a table the click's `x` decides the cell; outside one it never gets a vote.
    #[test]
    fn a_click_in_a_table_lands_in_the_cell_it_is_over() {
        let flow = laid_out(&with_a_table());
        let row = flow.slot(1).unwrap().top + 0.5;
        let share = COLUMN / 3.0;
        assert_eq!(flow.at(10.0, row), Some(1));
        assert_eq!(flow.at(share + 10.0, row), Some(2));
        assert_eq!(flow.at(2.0 * share + 10.0, row), Some(3));
        assert_eq!(
            flow.at(COLUMN + 500.0, row),
            Some(3),
            "past the edge: the nearest"
        );
        let second = flow.slot(4).unwrap().top + 0.5;
        assert_eq!(flow.at(share + 10.0, second), Some(5));
    }

    /// A cell holding two paragraphs makes its whole row taller, and its neighbours' text stays
    /// at the row's top rather than sliding down beside the second paragraph.
    #[test]
    fn a_cell_with_two_blocks_grows_its_row() {
        let app = with_a_table();
        let flat = laid_out(&app).cells()[0];
        app.split_block(grind_text::Caret {
            block: 1,
            offset: 0,
        })
        .unwrap();
        let flow = laid_out(&app);
        let (first, second, beside) = (
            flow.slot(1).unwrap(),
            flow.slot(2).unwrap(),
            flow.slot(3).unwrap(),
        );
        assert_eq!(second.indent, first.indent, "the same cell");
        assert!(second.top > first.bottom());
        assert_eq!(
            beside.top, first.top,
            "the next cell starts at the row's top"
        );
        assert!(flow.cells()[0].height > flat.height);
        assert_eq!(
            flow.cells()[0].height,
            flow.cells()[1].height,
            "one row, one height"
        );

        // And the tops are out of order now — the case `visible` scans rather than searches for.
        let seen: Vec<usize> = flow
            .visible(second.top, second.bottom())
            .iter()
            .map(|slot| slot.index)
            .collect();
        assert!(seen.contains(&2), "{seen:?}");
    }

    /// A merged cell is as wide as the columns it spans — the one table shape only a file can
    /// produce today, so it is asked of the arithmetic directly.
    #[test]
    fn a_merged_cell_spans_the_columns_it_covers() {
        let mut merged = grind_text::model::Cell::new("T", 0, 1);
        merged.columns_spanned = 2;
        assert_eq!(cell_x(&merged, 4, 400.0), (100.0, 200.0));
        let plain = grind_text::model::Cell::new("T", 0, 0);
        assert_eq!(
            cell_x(&plain, 0, 400.0),
            (0.0, 400.0),
            "never a division by zero"
        );
    }

    /// A list item in a cell keeps its indent inside the cell, so its bullet is drawn in the cell
    /// and not over the rule or in the neighbour.
    #[test]
    fn a_list_item_in_a_cell_is_indented_inside_it() {
        let item = in_cell(&BlockKind::ListItem { depth: 1 }, 100.0, 100.0, 96);
        assert_eq!(item.left, 100.0 + CELL_PAD + INDENT);
        assert_eq!(item.width, 100.0 - 2.0 * CELL_PAD - INDENT);
    }
}
