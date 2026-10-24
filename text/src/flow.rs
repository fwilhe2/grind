// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where each block sits down a page, and a table's blocks on its grid — pure arithmetic, above
//! line layout (which is `grind_core::layout`'s, through [`App::layout_block`]): each block's box,
//! where a cell's blocks go, which blocks are on screen, which one a click landed in, and how far
//! to scroll to keep the caret in view.
//!
//! A shell supplies its numbers ([`Spacing`], in its own unit and scale), its fonts ([`Faces`]),
//! and what a picture decodes to (a hook on [`lay_out`], since decoding needs an OS decoder). The
//! browser stacks blocks with the DOM and the terminal by whole rows, so neither calls this.

use std::collections::HashMap;
use std::ops::Range;

use crate::{App, BlockKind, BlockView, Cell, Faces, Table, Viewport};

/// How a shell sets its page, in its own unit and scaled to its screen — the decisions, as values;
/// the rules they feed are this module's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    /// From the document's top to the first block's text — a margin that scrolls with the text.
    pub top: f64,
    /// Under every block, and between two blocks in one table cell.
    pub gap: f64,
    /// Above a heading, collapsed against the gap the block before it already left.
    pub heading: f64,
    /// How far one level of a list indents its text.
    pub indent: f64,
    /// Between a table cell's rule and the text inside it.
    pub cell_pad: f64,
}

impl Spacing {
    /// How far a block's text is indented: one step per list level, nothing otherwise.
    pub fn indent_of(&self, kind: &BlockKind) -> f64 {
        match kind {
            BlockKind::ListItem { depth } => f64::from(*depth) * self.indent,
            _ => 0.0,
        }
    }

    /// The space above a block, before collapsing against the gap under the one above. A heading
    /// gets [`Spacing::heading`], as does a `Title` or `Subtitle` paragraph, which every shell
    /// sets in a heading's face.
    pub fn above(&self, kind: &BlockKind, style: Option<&str>) -> f64 {
        match (kind, style) {
            (BlockKind::Heading { .. }, _) | (_, Some("Title" | "Subtitle")) => self.heading,
            _ => 0.0,
        }
    }

    /// The measure a block outside a table is set at in a column `column` wide: the column less
    /// its indent, at least one unit. A shell's [`Faces`] must answer exactly this, or a list item
    /// breaks lines in one place and draws them in another; a block in a table is set at its
    /// cell's width ([`across`]).
    pub fn measure(&self, kind: &BlockKind, column: f64) -> f64 {
        (column - self.indent_of(kind)).max(1.0)
    }
}

/// The room a document's own paragraph style gives a block ([`Faces::spacing`]): space above and
/// below, added to its neighbours' (`doc/odt-format.md` §5c, fact 5), and its left indent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Space {
    pub above: f64,
    pub below: f64,
    pub left: f64,
}

/// One block's box in the flow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub index: usize,
    /// Distance from the top of the document to the top of this block's text.
    pub top: f64,
    /// The height of its lines — not counting the gap under it.
    pub height: f64,
    /// How far its text starts from the column's left edge: a list's indent, or a cell's left edge
    /// plus its padding.
    pub indent: f64,
    /// The measure its lines were broken at — carried because a block in a table is set at its
    /// cell's width. A picture is fitted to it, a click measured against it, and a caret operation
    /// asks the core with it.
    pub width: f64,
}

impl Slot {
    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }
}

/// One table cell's box, for drawing its rules: document coordinates, `left` from the column's
/// left edge as [`Slot::indent`] is. Derived from the blocks laid out in it; kept because a rule
/// is drawn once per cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellBox {
    pub top: f64,
    pub left: f64,
    pub width: f64,
    pub height: f64,
    /// The cell's first block, by index — how a painter finds its style ([`crate::table_look`]).
    pub first: usize,
}

impl CellBox {
    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }

    pub fn right_edge(&self) -> f64 {
        self.left + self.width
    }
}

/// Every block stacked, and a table's blocks on its grid. Built fresh whenever the document, the
/// width or the scale changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    slots: Vec<Slot>,
    cells: Vec<CellBox>,
    height: f64,
    /// The space under the last block, included in `height`, which the next block's space above
    /// collapses against ([`Flow::push`]) or adds to ([`Flow::push_spaced`]).
    pending: f64,
    /// The text column's width, which is what an ordinary block is measured at.
    measure: f64,
}

impl Flow {
    /// An empty flow whose first block starts `top` below the document's top, across a column
    /// `measure` wide.
    pub fn new(top: f64, measure: f64) -> Self {
        Flow {
            slots: Vec::new(),
            cells: Vec::new(),
            height: top,
            pending: 0.0,
            measure,
        }
    }

    /// Add a block below the others with the space above and below it. `space_before` collapses
    /// against the gap already left, so a heading after a paragraph does not get both.
    pub fn push(&mut self, index: usize, height: f64, indent: f64, space_before: f64, gap: f64) {
        let top = match self.slots.is_empty() {
            // Nothing above the first block for its own space to sit under.
            true => self.height,
            false => self.height - self.pending + self.pending.max(space_before),
        };
        self.slots.push(Slot {
            index,
            top,
            height,
            indent,
            width: (self.measure - indent).max(1.0),
        });
        self.height = top + height + gap;
        self.pending = gap;
    }

    /// Add a block with a document style's space above and below it, added to the space before
    /// rather than collapsed, and above the first block too, as Writer does (`doc/odt-format.md`
    /// §5c, facts 5 and 6) — for a printed page. `width` is the measure its face set it at.
    pub fn push_spaced(
        &mut self,
        index: usize,
        height: f64,
        indent: f64,
        space: Space,
        width: f64,
    ) {
        let top = self.height + space.above;
        self.slots.push(Slot {
            index,
            top,
            height,
            indent: indent + space.left,
            width: width.max(1.0),
        });
        self.height = top + height + space.below;
        self.pending = space.below;
    }

    /// Put a block at an exact box rather than under the last one, as a table's cells are placed
    /// by coordinate. [`Flow::advance`] catches the running height up afterwards.
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

    /// Move the running height to `to`, of which the last `pending` is space under what was
    /// placed, for the next block to collapse against.
    pub fn advance(&mut self, to: f64, pending: f64) {
        self.height = self.height.max(to);
        self.pending = pending;
    }

    /// The measure an ordinary block is laid out at.
    pub fn measure(&self) -> f64 {
        self.measure
    }

    /// Every table cell's box, in document order.
    pub fn cells(&self) -> &[CellBox] {
        &self.cells
    }

    /// Every block's box, in document order.
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

    /// The slot for the block at `index`, looked up by block rather than by position in the list.
    pub fn slot(&self, index: usize) -> Option<&Slot> {
        match self.slots.binary_search_by_key(&index, |slot| slot.index) {
            Ok(at) => self.slots.get(at),
            Err(_) => self.slots.iter().find(|slot| slot.index == index),
        }
    }

    /// The blocks intersecting `top..bottom`, as a contiguous slice in document order, so a shell
    /// hands its ends to [`App::get_viewport`]. A scan from either end, not a binary search: in a
    /// table the second block of a tall cell sits below the first block beside it. The slice may
    /// include a just-off-screen block of the same table.
    pub fn visible(&self, top: f64, bottom: f64) -> &[Slot] {
        let first = self.slots.iter().position(|slot| slot.bottom() >= top);
        let last = self.slots.iter().rposition(|slot| slot.top < bottom);
        match (first, last) {
            (Some(first), Some(last)) if first <= last => &self.slots[first..=last],
            _ => &[],
        }
    }

    /// Which block a click at `(x, y)` landed in, `x` from the column's left edge — the nearest,
    /// never none, since a document has no outside. Vertical distance dominates and horizontal
    /// only breaks ties, which picks the column inside a table.
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

    /// Where the scroll must be for `target` (a line's top and height) to be on screen, moving as
    /// little as possible.
    pub fn follow(&self, scroll: f64, page: f64, target: (f64, f64)) -> f64 {
        let (top, height) = target;
        let scroll = match scroll {
            _ if top < scroll => top,
            s if top + height > s + page => top + height - page,
            s => s,
        };
        scroll.clamp(0.0, self.limit(page))
    }

    /// The furthest down the document the view may be scrolled, for a view `page` tall.
    pub fn limit(&self, page: f64) -> f64 {
        (self.height - page).max(0.0)
    }
}

/// Where a block in a table cell sits across the column, and how wide it is set. The caret needs
/// this too: a cell's lines are broken at its width, so [`across`] is computed once per reflow,
/// read by the shell's [`Faces`], and used by [`lay_out`] as well.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Across {
    /// From the column's left edge to where the text starts.
    pub left: f64,
    pub width: f64,
}

/// A cell's box across a column `column` wide: its left edge and width, in equal shares of the
/// columns (the answer that never overflows the measure). A merged cell takes every column it
/// spans.
pub fn cell_x(cell: &Cell, columns: u32, column: f64) -> (f64, f64) {
    let share = column / f64::from(columns.max(1));
    (
        f64::from(cell.column) * share,
        f64::from(cell.columns_spanned.max(1)) * share,
    )
}

/// [`cell_x`] for a table whose columns state widths (`widths`, `None` where a column states
/// none): unstated columns share what is left, and a table wider than the column is scaled to fit,
/// since paper has no sideways scroll.
pub fn cell_x_with(cell: &Cell, columns: u32, column: f64, widths: &[Option<f64>]) -> (f64, f64) {
    let count = columns.max(1) as usize;
    let stated: f64 = widths.iter().take(count).flatten().sum();
    let unstated = (0..count)
        .filter(|&i| widths.get(i).copied().flatten().is_none())
        .count();
    let share = match unstated {
        0 => 0.0,
        n => ((column - stated) / n as f64).max(0.0),
    };
    let each: Vec<f64> = (0..count)
        .map(|i| widths.get(i).copied().flatten().unwrap_or(share))
        .collect();
    let total: f64 = each.iter().sum();
    let scale = if total > column && total > 0.0 {
        column / total
    } else {
        1.0
    };
    let first = (cell.column as usize).min(count);
    let last = (first + cell.columns_spanned.max(1) as usize).min(count);
    let left: f64 = each[..first].iter().sum::<f64>() * scale;
    let width: f64 = each[first..last].iter().sum::<f64>() * scale;
    (left, width)
}

/// Where a cell sits across the column: by the table's widths when its face states them
/// ([`Faces::columns`]), else equal shares.
fn place_cell(
    faces: &dyn Faces,
    table: &str,
    cell: &Cell,
    columns: u32,
    column: f64,
) -> (f64, f64) {
    match faces.columns(table) {
        Some(widths) => cell_x_with(cell, columns, column, &widths),
        None => cell_x(cell, columns, column),
    }
}

/// A cell's padding — top, right, bottom, left — its face's for it, or the spacing's all round.
fn padding(faces: &dyn Faces, cell: &Cell, spacing: &Spacing) -> [f64; 4] {
    faces
        .cell_pad(&cell.table, cell.row, cell.column)
        .unwrap_or([spacing.cell_pad; 4])
}

/// Where one block sits in a cell starting at `left`, `width` wide: inside the padding, a list
/// item further in by its indent, so its bullet stays inside the cell.
fn in_cell(kind: &BlockKind, left: f64, width: f64, spacing: &Spacing, pad: [f64; 4]) -> Across {
    let indent = spacing.indent_of(kind);
    Across {
        left: left + pad[3] + indent,
        width: (width - pad[1] - pad[3] - indent).max(1.0),
    }
}

/// A [`Faces`] that answers nothing — every table in equal shares, padded by the spacing.
struct Plain;

impl Faces for Plain {
    fn of(&self, _: usize, _: &BlockKind, _: Option<&str>) -> (f32, &dyn crate::Metrics) {
        (1.0, &crate::Fixed)
    }
}

/// Every block in a table cell, by index, with where it sits across a column `column` wide; other
/// blocks use [`Spacing::measure`]. Built before the shell's `Faces`, since `Faces::of` runs under
/// `App`'s read lock and cannot ask which cell a block is in.
pub fn across(app: &App, column: f64, spacing: &Spacing) -> HashMap<usize, Across> {
    across_with(app, column, spacing, &Plain)
}

/// [`across`] with a table's own column widths and cell padding ([`Faces::columns`],
/// [`Faces::cell_pad`]), for a printed page.
pub fn across_with(
    app: &App,
    column: f64,
    spacing: &Spacing,
    looks: &dyn Faces,
) -> HashMap<usize, Across> {
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
            let (left, width) = place_cell(looks, &cell.table, cell, table.columns, column);
            let pad = padding(looks, cell, spacing);
            out.insert(at, in_cell(&view.kind, left, width, spacing, pad));
        }
        index = table.blocks.end.max(index + 1);
    }
    out
}

/// Measure every block and stack them, laying tables out as grids. A block's height is its
/// `Layout::height()` at the width `faces` gives — a cell's, from [`across`] — the same measure
/// and metrics caret operations use, so drawing and Down-arrow agree. A block the core cannot lay
/// out takes no room (R5).
///
/// `picture` measures a block that is a picture ([`crate::picture_of`]) from its decoded pixels
/// fitted to the width: `Some(height)` when the caller could decode it, `None` to use the text
/// height. A caller with no decoder passes `&|_, _| None`.
pub fn lay_out(
    app: &App,
    faces: &dyn Faces,
    column: f64,
    spacing: &Spacing,
    picture: &dyn Fn(&BlockView, f64) -> Option<f64>,
) -> Flow {
    let count = app.block_count();
    let viewport = app.get_viewport(0..count);
    let mut flow = Flow::new(spacing.top, column);
    let height_of = |index: usize, view: &BlockView| -> (f64, f64) {
        let (width, metrics) = faces.of(index, &view.kind, view.style.as_deref());
        let height = match picture(view, f64::from(width)) {
            Some(height) => height,
            None => app
                .layout_block_tabbed(
                    index,
                    width,
                    metrics,
                    faces.first_indent(index),
                    &faces.tabs(index),
                )
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
        // A table is laid out as a grid, its blocks placed rather than pushed, using
        // `App::table`'s fold.
        if view.cell.is_some()
            && let Some(table) = app.table(index)
        {
            lay_out_table(
                &mut flow, &viewport, &table, column, spacing, faces, &height_of,
            );
            index = table.blocks.end.max(index + 1);
            continue;
        }
        let (height, width) = height_of(index, view);
        let style = view.style.as_deref();
        let indent = spacing.indent_of(&view.kind);
        match faces.spacing(index) {
            Some(space) => flow.push_spaced(index, height, indent, space, width),
            None => flow.push(
                index,
                height,
                indent,
                spacing.above(&view.kind, style),
                spacing.gap,
            ),
        }
        index += 1;
    }
    flow
}

/// One cell of a table and the blocks in it, in the order they appear.
struct CellRun<'a> {
    cell: &'a Cell,
    blocks: Vec<usize>,
}

/// Every cell of a table, in the order its first block appears, built from the blocks.
fn cell_runs(viewport: &Viewport, blocks: Range<usize>) -> Vec<CellRun<'_>> {
    let mut out: Vec<CellRun> = Vec::new();
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

/// One table as a grid: columns across the measure, each row as tall as its tallest cell, each
/// cell's blocks stacked inside. Two passes, since a cell's position needs its row's height. A
/// cell spanning rows contributes a share to each.
fn lay_out_table(
    flow: &mut Flow,
    viewport: &Viewport,
    table: &Table,
    column: f64,
    spacing: &Spacing,
    faces: &dyn Faces,
    height_of: &dyn Fn(usize, &BlockView) -> (f64, f64),
) {
    let gap = spacing.gap;
    // The same gap above a table a paragraph would leave: the running height already ends one gap
    // below what came before.
    let top = flow.height();
    let rows = table.rows.max(1) as usize;
    let cells = cell_runs(viewport, table.blocks.clone());

    // Every block's height and measure once, since both passes want them.
    let measured: HashMap<usize, (f64, f64)> = cells
        .iter()
        .flat_map(|run| run.blocks.iter().copied())
        .filter_map(|index| Some((index, height_of(index, viewport.get(index)?))))
        .collect();
    // Space above and below a block in a cell: its face's own spacing when stated, added as Writer
    // adds it, above the first and below the last block too (`doc/odt-format.md` §5c fact 18);
    // otherwise the shell's gap.
    let room = |index: usize| -> (f64, f64) {
        faces
            .spacing(index)
            .map_or((0.0, gap), |space| (space.above, space.below))
    };
    // The text's own height in a cell, and the cell's with its padding.
    let text = |run: &CellRun| -> f64 {
        let stacked: f64 = run
            .blocks
            .iter()
            .map(|&index| {
                let (above, below) = room(index);
                above + measured.get(&index).map_or(0.0, |m| m.0) + below
            })
            .sum();
        let trailing = match run.blocks.last() {
            Some(&last) if faces.spacing(last).is_none() => gap,
            _ => 0.0,
        };
        (stacked - trailing).max(0.0)
    };
    let content = |run: &CellRun| -> f64 {
        let pad = padding(faces, run.cell, spacing);
        text(run) + pad[0] + pad[2]
    };

    let mut heights = vec![2.0 * spacing.cell_pad; rows];
    // A row with only padded cells is as tall as their padding, not the spacing's.
    for run in &cells {
        let pad = padding(faces, run.cell, spacing);
        if let Some(height) = heights.get_mut(run.cell.row as usize) {
            *height = height.min(pad[0] + pad[2]);
        }
    }
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
        let (left, width) = place_cell(faces, &run.cell.table, run.cell, table.columns, column);
        let pad = padding(faces, run.cell, spacing);
        flow.cell(CellBox {
            top: cell_top,
            left,
            width,
            height,
            first: run.blocks.first().copied().unwrap_or(0),
        });
        // A cell whose face centres it vertically puts its text halfway down the room left.
        let slack = (height - text(run) - pad[0] - pad[2]).max(0.0);
        let centred = faces.cell_centred(&run.cell.table, run.cell.row, run.cell.column);
        let mut at = cell_top + pad[0] + if centred { slack / 2.0 } else { 0.0 };
        for &index in &run.blocks {
            let Some(view) = viewport.get(index) else {
                continue;
            };
            let place = in_cell(&view.kind, left, width, spacing, pad);
            // The width the lines were broken at — the `Faces`' answer.
            let (block_height, measure) =
                measured.get(&index).copied().unwrap_or((0.0, place.width));
            let (above, below) = room(index);
            let shift = faces.spacing(index).map_or(0.0, |space| space.left);
            flow.place(index, at + above, block_height, place.left + shift, measure);
            at += above + block_height + below;
        }
    }
    // A screen leaves its gap under a table; a face spacing blocks as a document does leaves none,
    // as Writer does.
    let after = match table
        .blocks
        .clone()
        .next()
        .map(|first| faces.spacing(first))
    {
        Some(Some(_)) => 0.0,
        _ => gap,
    };
    flow.advance(top + heights.iter().sum::<f64>() + after, after);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Caret, Fixed, Metrics, Uniform};

    /// The numbers the Windows pane uses at 100%, which are as good as any.
    const SPACING: Spacing = Spacing {
        top: 40.0,
        gap: 10.0,
        heading: 18.0,
        indent: 28.0,
        cell_pad: 6.0,
    };

    /// `lay_out`'s picture hook answering "not a picture" for every block.
    fn no_pictures(_: &BlockView, _: f64) -> Option<f64> {
        None
    }

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
        // 40 to the last block's top, 10 of text, and the gap under it.
        assert_eq!(flow.height(), 60.0);
    }

    /// Adjacent space collapses, or a heading after a paragraph would carry both gaps.
    #[test]
    fn the_space_above_a_heading_collapses_against_the_gap_below_the_paragraph() {
        let mut flow = Flow::new(0.0, 100.0);
        flow.push(0, 10.0, 0.0, 0.0, 10.0);
        flow.push(1, 20.0, 0.0, 18.0, 10.0);
        assert_eq!(flow.slot(1).unwrap().top, 28.0, "20 + (18 - 10)");

        // The first block starts exactly at the page's top margin.
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
    fn a_heading_and_a_title_get_more_room_above_them_than_a_paragraph() {
        let heading = BlockKind::Heading { level: 1 };
        assert_eq!(SPACING.above(&heading, None), SPACING.heading);
        assert_eq!(SPACING.above(&BlockKind::Paragraph, None), 0.0);
        for display in ["Title", "Subtitle"] {
            assert_eq!(
                SPACING.above(&BlockKind::Paragraph, Some(display)),
                SPACING.heading,
                "{display}: a paragraph by kind, a heading by face"
            );
        }
        assert_eq!(SPACING.above(&BlockKind::Paragraph, Some("Quote")), 0.0);
    }

    #[test]
    fn a_list_item_indents_once_per_level_and_is_measured_narrower() {
        let item = |depth| BlockKind::ListItem { depth };
        assert_eq!(SPACING.indent_of(&item(1)), 28.0);
        assert_eq!(SPACING.indent_of(&item(3)), 84.0);
        assert_eq!(SPACING.indent_of(&BlockKind::Paragraph), 0.0);
        assert_eq!(SPACING.measure(&item(1), 300.0), 272.0);
        assert_eq!(SPACING.measure(&BlockKind::Paragraph, 300.0), 300.0);
        assert_eq!(
            SPACING.measure(&item(9), 100.0),
            1.0,
            "never less than a unit"
        );
    }

    /// A document, three blocks, one of them long enough to wrap at the width below.
    fn document() -> App {
        let app = App::new();
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

    /// The `Faces` a shell builds, without fonts: a block in a table at its cell's width from
    /// [`across`], others at [`Spacing::measure`].
    struct InCells {
        column: f64,
        across: HashMap<usize, Across>,
    }

    impl Faces for InCells {
        fn of(&self, index: usize, kind: &BlockKind, _: Option<&str>) -> (f32, &dyn Metrics) {
            let width = match self.across.get(&index) {
                Some(across) => across.width,
                None => SPACING.measure(kind, self.column),
            };
            (width as f32, &Fixed)
        }
    }

    /// Every block one unit tall at `Fixed`, with paper spacing: two above and three below, except
    /// block 1, which keeps the screen's collapsing.
    struct Spaced;

    impl Faces for Spaced {
        fn of(&self, _: usize, _: &BlockKind, _: Option<&str>) -> (f32, &dyn Metrics) {
            (100.0, &Fixed)
        }
        fn spacing(&self, index: usize) -> Option<Space> {
            (index != 1).then_some(Space {
                above: 2.0,
                below: 3.0,
                left: 0.0,
            })
        }
    }

    #[test]
    fn spacing_a_face_states_adds_and_is_applied_above_the_first_block() {
        let app = App::new();
        for (at, text) in ["a", "b", "c"].iter().enumerate() {
            app.insert(at, BlockKind::Paragraph, text).unwrap();
        }
        app.delete(3..4).unwrap();
        let tight = Spacing {
            top: 10.0,
            gap: 1.0,
            heading: 0.0,
            indent: 0.0,
            cell_pad: 0.0,
        };
        let flow = lay_out(&app, &Spaced, 100.0, &tight, &no_pictures);
        let tops: Vec<f64> = flow.slots().iter().map(|s| s.top).collect();
        // Block 0: the top plus its two above. Block 1 states nothing, so it sits under block 0's
        // three below, collapsed with the screen's gap. Block 2 adds its two above to block 1's
        // gap (`doc/odt-format.md` §5c, fact 5).
        assert_eq!(tops, vec![12.0, 16.0, 20.0]);
        assert_eq!(
            flow.height(),
            24.0,
            "block 2's own three below are the flow's last"
        );
    }

    /// A face with a table's own column widths and padding.
    struct Widths;

    impl Faces for Widths {
        fn of(&self, _: usize, _: &BlockKind, _: Option<&str>) -> (f32, &dyn Metrics) {
            (100.0, &Fixed)
        }
        fn columns(&self, _table: &str) -> Option<Vec<Option<f64>>> {
            Some(vec![Some(60.0), None, Some(90.0)])
        }
        fn cell_pad(&self, _table: &str, row: u32, _column: u32) -> Option<[f64; 4]> {
            (row == 0).then_some([3.0, 1.0, 3.0, 2.0])
        }
    }

    #[test]
    fn a_tables_own_widths_and_padding_place_its_cells() {
        let app = App::new();
        app.insert_table(0, 2, 3, Some("W".into())).unwrap();
        let tight = Spacing {
            top: 0.0,
            gap: 0.0,
            heading: 0.0,
            indent: 0.0,
            cell_pad: 1.0,
        };
        let across = across_with(&app, 300.0, &tight, &Widths);
        // 60 and 90 stated, the rest of 300 for the one that states nothing.
        assert_eq!(
            (across[&0].width, across[&1].width, across[&2].width),
            (60.0 - 3.0, 150.0 - 3.0, 90.0 - 3.0)
        );
        assert_eq!(across[&0].left, 2.0, "the cell's own left padding");
        let flow = lay_out(&app, &Widths, 300.0, &tight, &no_pictures);
        let first = flow.slot(0).unwrap();
        assert_eq!(first.top, 3.0, "the cell's own top padding");
        let cells = flow.cells();
        assert_eq!(
            (cells[0].left, cells[0].width, cells[0].first),
            (0.0, 60.0, 0)
        );
        assert_eq!(
            cells[0].height,
            1.0 + 3.0 + 3.0,
            "a line and the row's padding"
        );
        assert_eq!(cells[1].left, 60.0);
    }

    fn laid_out(app: &App, column: f64) -> Flow {
        let faces = InCells {
            column,
            across: across(app, column, &SPACING),
        };
        lay_out(app, &faces, column, &SPACING, &no_pictures)
    }

    /// The stacking takes its heights from the core, so with [`Fixed`] it breaks lines where
    /// `grind text view --width` does.
    #[test]
    fn the_page_breaks_lines_where_the_cli_does() {
        let app = document();
        let flow = laid_out(&app, 30.0);

        assert_eq!(flow.slots().len(), app.block_count());
        for slot in flow.slots() {
            // What `grind text view --width` measures, through the same call the CLI makes.
            let expected = app
                .layout_block(slot.index, slot.width as f32, &Fixed)
                .unwrap();
            assert_eq!(
                slot.height,
                f64::from(expected.height()),
                "block {} is as tall as its lines and no taller",
                slot.index
            );
        }
        // And the paragraph really wrapped, or the test asserts nothing.
        let lines = app.layout_block(1, 30.0, &Fixed).unwrap();
        assert!(lines.lines().len() > 1, "the fixture has to wrap");
    }

    /// A decoded picture is never also measured as its placeholder's line of text.
    #[test]
    fn a_picture_is_measured_by_the_hook_and_not_as_text() {
        let app = document();
        let faces = Uniform::new(30.0, &Fixed);
        let flow = lay_out(&app, &faces, 30.0, &SPACING, &|_, _| Some(123.0));
        assert!(
            flow.slots().iter().all(|slot| slot.height == 123.0),
            "every block answered by the hook, not by its own text"
        );
    }

    /// Blocks come out in document order, none overlapping the next, and the document is as
    /// tall as the last one's bottom plus its gap.
    #[test]
    fn a_measured_document_stacks_in_order() {
        let flow = laid_out(&document(), 30.0);
        let tops: Vec<f64> = flow.slots().iter().map(|slot| slot.top).collect();
        assert!(tops.windows(2).all(|w| w[1] > w[0]), "{tops:?}");
        assert_eq!(flow.slot(0).unwrap().top, SPACING.top, "the page's margin");
        assert!(flow.height() > flow.slot(2).unwrap().bottom());
        // The list item is the only one indented, and by exactly one step.
        assert_eq!(flow.slot(1).unwrap().indent, 0.0);
        assert_eq!(flow.slot(2).unwrap().indent, SPACING.indent);
    }

    /// A document with no blocks at all has nothing to stack. Read from `Document::empty`, since a
    /// new `App` has one empty paragraph.
    #[test]
    fn a_document_with_no_blocks_measures_to_its_own_margin() {
        let app = App::new();
        let bytes = crate::write_bytes(&crate::Document::empty(), crate::Form::Flat).unwrap();
        app.open_bytes("empty.fodt", &bytes).unwrap();
        assert_eq!(app.block_count(), 0, "the file really has none");

        let flow = laid_out(&app, 30.0);
        assert!(flow.is_empty());
        assert_eq!(flow.at(0.0, 0.0), None);
        assert_eq!(flow.limit(500.0), 0.0, "nothing to scroll");
    }

    /// A new document has one empty paragraph with a slot, so the first keystroke lands somewhere.
    #[test]
    fn a_new_document_has_one_block_to_type_into() {
        let app = App::new();
        assert_eq!(app.block_count(), 1);
        let flow = laid_out(&app, 30.0);
        let slot = flow.slot(0).expect("the paragraph has a place on the page");
        assert_eq!(slot.top, SPACING.top, "the page's own top margin");
        assert!(slot.height > 0.0, "an empty paragraph is still a line tall");
    }

    const COLUMN: f64 = 300.0;

    /// A paragraph, an empty two-by-three table, and a paragraph — `Insert Table…`'s result.
    fn with_a_table() -> App {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "Before").unwrap();
        app.insert_table(1, 2, 3, Some("Prices".into())).unwrap();
        // `App::new()`'s own empty paragraph is last; give it some text so it is the "after".
        let last = app.block_count() - 1;
        app.insert_text(
            Caret {
                block: last,
                offset: 0,
            },
            "After",
        )
        .unwrap();
        app
    }

    /// An empty table's six cells form a grid: a row's cells share a top, the columns are equal
    /// shares, and it is two rows tall.
    #[test]
    fn a_table_is_laid_out_as_a_grid_rather_than_stacked() {
        let app = with_a_table();
        assert_eq!(app.block_count(), 8, "before, six cells, after");
        let flow = laid_out(&app, COLUMN);
        let pad = SPACING.cell_pad;

        let cells: Vec<&Slot> = (1..7).map(|i| flow.slot(i).unwrap()).collect();
        let share = COLUMN / 3.0;
        for (at, slot) in cells.iter().enumerate() {
            let (row, column) = (at / 3, at % 3);
            assert_eq!(slot.top, cells[row * 3].top, "a row shares one top");
            assert_eq!(slot.indent, column as f64 * share + pad, "cell {at}");
            assert_eq!(slot.width, share - 2.0 * pad, "set inside its padding");
        }
        assert!(
            cells[3].top > cells[0].bottom(),
            "the second row is under the first"
        );

        assert_eq!(flow.cells().len(), 6, "one rule per cell");
        let boxes = flow.cells();
        assert_eq!(boxes[1].left, share);
        assert_eq!(boxes[0].bottom(), boxes[3].top, "rows abut: one rule");
        assert_eq!(boxes[0].right_edge(), boxes[1].left, "and so do columns");

        let before = flow.slot(0).unwrap();
        let after = flow.slot(7).unwrap();
        assert!(boxes[0].top >= before.bottom(), "under the paragraph");
        assert!(after.top >= boxes[5].bottom(), "and the next one under it");
        assert_eq!(after.indent, 0.0, "back to the full column");
        assert_eq!(after.width, COLUMN);
        // Two rows of one line each plus padding.
        let table_h = boxes[5].bottom() - boxes[0].top;
        assert_eq!(table_h, 2.0 * (1.0 + 2.0 * pad));
    }

    /// A table is one gap from what comes before it and one from what follows.
    #[test]
    fn a_table_sits_one_gap_from_its_neighbours() {
        let flow = laid_out(&with_a_table(), COLUMN);
        let (before, after) = (flow.slot(0).unwrap(), flow.slot(7).unwrap());
        let boxes = flow.cells();
        assert_eq!(boxes[0].top - before.bottom(), SPACING.gap, "above");
        assert_eq!(after.top - boxes[5].bottom(), SPACING.gap, "below");
    }

    /// The width the caret measures a cell at is the width it was laid out at.
    #[test]
    fn the_caret_and_the_page_agree_on_a_cells_measure() {
        let app = with_a_table();
        let flow = laid_out(&app, COLUMN);
        let map = across(&app, COLUMN, &SPACING);
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
        let flow = laid_out(&with_a_table(), COLUMN);
        let row = flow.slot(1).unwrap().top + 0.5;
        let share = COLUMN / 3.0;
        assert_eq!(flow.at(10.0, row), Some(1));
        assert_eq!(flow.at(share + 10.0, row), Some(2));
        assert_eq!(flow.at(2.0 * share + 10.0, row), Some(3));
        assert_eq!(flow.at(COLUMN + 500.0, row), Some(3), "past the edge");
        let second = flow.slot(4).unwrap().top + 0.5;
        assert_eq!(flow.at(share + 10.0, second), Some(5));
    }

    /// A cell with two paragraphs makes its row taller, and its neighbours' text stays at the
    /// row's top.
    #[test]
    fn a_cell_with_two_blocks_grows_its_row() {
        let app = with_a_table();
        let flat = laid_out(&app, COLUMN).cells()[0];
        app.split_block(Caret {
            block: 1,
            offset: 0,
        })
        .unwrap();
        let flow = laid_out(&app, COLUMN);
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

    /// A merged cell is as wide as the columns it spans.
    #[test]
    fn a_merged_cell_spans_the_columns_it_covers() {
        let mut merged = Cell::new("T", 0, 1);
        merged.columns_spanned = 2;
        assert_eq!(cell_x(&merged, 4, 400.0), (100.0, 200.0));
        let plain = Cell::new("T", 0, 0);
        assert_eq!(cell_x(&plain, 0, 400.0), (0.0, 400.0), "never / 0");
    }

    /// A list item in a cell keeps its indent inside the cell.
    #[test]
    fn a_list_item_in_a_cell_is_indented_inside_it() {
        let item = in_cell(
            &BlockKind::ListItem { depth: 1 },
            100.0,
            100.0,
            &SPACING,
            [SPACING.cell_pad; 4],
        );
        let (pad, indent) = (SPACING.cell_pad, SPACING.indent);
        assert_eq!(item.left, 100.0 + pad + indent);
        assert_eq!(item.width, 100.0 - 2.0 * pad - indent);
    }
}
