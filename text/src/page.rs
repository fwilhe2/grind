// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Breaking a document into pages — pure arithmetic over the flow (`doc/pdf-export.md`). Pages are
//! derived for an export or a preview and thrown away; the editor stays one continuous column.
//!
//! Built on [`flow::lay_out`], so the spacing rules exist once: the flow is walked top to bottom
//! and each page takes what fits, splitting a paragraph between lines and never a table row. The
//! unit is the caller's (the PDF uses points). Three rules:
//!
//! * *orphans*: a split paragraph leaves at least this many lines at the foot of the page, or
//!   moves whole;
//! * *widows*: and carries at least this many to the next;
//! * a heading is kept with what follows it.
//!
//! Space above a block at the top of a page is dropped, but for what its face keeps
//! ([`Faces::kept_above`]).
//!
//! A table row that does not fit splits between lines, each part drawn as a closed row
//! (`doc/odt-format.md` §5c fact 25) — unless it keeps together or is a heading row, which move
//! whole and overflow a page they are taller than.

use std::ops::Range;

use crate::flow::{self, CellBox, Spacing};
use crate::{App, BlockView, Faces};

/// How hard the cut tries to keep a paragraph together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    /// Fewest lines a split paragraph leaves at the foot of a page.
    pub orphans: usize,
    /// Fewest lines it carries to the head of the next.
    pub widows: usize,
}

impl Default for Rules {
    /// Two and two, the usual minimum and Writer's default. A real document passes its own
    /// (`grind_print` does, and none where it states none, `doc/odt-format.md` §5c).
    fn default() -> Self {
        Rules {
            orphans: 2,
            widows: 2,
        }
    }
}

/// What a document's paragraph style says about page breaks around a block (`fo:break-before`,
/// `fo:break-after`, `fo:keep-with-next`; rng:2073, rng:2110), asked through [`Faces::breaks`]; a
/// screen answers nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Breaks {
    /// `fo:break-before="page"`: the block starts a page.
    pub page_before: bool,
    /// `fo:break-after="page"`: the block ends one.
    pub page_after: bool,
    /// `fo:keep-with-next`, overriding the rule that keeps only a heading with what follows.
    pub keep_with_next: Option<bool>,
}

/// Some of one block's lines, placed on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// The block, by index.
    pub index: usize,
    /// Which of its laid-out lines are on this page — all of them, unless it was split.
    pub lines: Range<usize>,
    /// Distance from the top of the page's text area to the top of the first of those lines.
    pub top: f64,
    /// Distance from the text area's left edge — a list's indent, or a table cell's own.
    pub left: f64,
    /// The measure its lines were broken at.
    pub width: f64,
    /// Whether this is a table's heading row repeated at the top of a continuing page: drawn
    /// again, not part of the reading order.
    pub repeat: bool,
}

/// One page: what is on it, in the text area's coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub pieces: Vec<Piece>,
    /// The table cells on this page, for the rules drawn round them.
    pub cells: Vec<CellBox>,
    /// Which [`Slot`] the page is set on — `0` for a document with one kind of page.
    pub slot: usize,
    /// Set when this page starts a run of page numbers again: the number it is counted as.
    pub restart: Option<u32>,
}

/// A kind of page for cutting purposes: its text area's height, the kind of page after it — a
/// master page after its header and footer take their share (`grind-print`) — and which of
/// [`paginate_slots`]'s measures its text is set at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub height: f64,
    pub next: usize,
    pub measure: usize,
}

/// Where a block starts a page of its own kind: its [`Slot`], and the number it restarts counting
/// at, if any.
pub type Starts = std::collections::HashMap<usize, (usize, Option<u32>)>;

/// Break the document into pages with a text area `column` wide and `height` tall. `faces` and
/// `picture` are [`flow::lay_out`]'s, since the cut needs each block's lines measured the same
/// way.
pub fn paginate(
    app: &App,
    faces: &dyn Faces,
    column: f64,
    height: f64,
    spacing: &Spacing,
    rules: Rules,
    picture: &dyn Fn(&BlockView, f64) -> Option<f64>,
) -> Vec<Page> {
    paginate_first(
        app,
        faces,
        column,
        (height, height),
        spacing,
        rules,
        picture,
    )
}

/// [`paginate`] with the first page's text area `heights.0` tall and the rest `heights.1` — a
/// first page on its own master (`Document::first_page`).
pub fn paginate_first(
    app: &App,
    faces: &dyn Faces,
    column: f64,
    (first, height): (f64, f64),
    spacing: &Spacing,
    rules: Rules,
    picture: &dyn Fn(&BlockView, f64) -> Option<f64>,
) -> Vec<Page> {
    let slots = [
        Slot {
            height: first,
            next: 1,
            measure: 0,
        },
        Slot {
            height,
            next: 1,
            measure: 0,
        },
    ];
    paginate_slots(
        app,
        &[(faces, column)],
        &slots,
        (0, &Starts::new()),
        spacing,
        rules,
        picture,
    )
}

/// [`paginate`] over several kinds of page: the first on `slots[start]`, each next on the previous
/// one's `next`, and a block in `starts` beginning a new page on its own slot — how a series of
/// letters on one letterhead paginates.
///
/// `measures` are the widths text is set at, each with the faces that measure at it: a run of
/// pages opened by a block in `starts` is laid out at its slot's measure — a landscape section's
/// text as wide as its page.
pub fn paginate_slots(
    app: &App,
    measures: &[(&dyn Faces, f64)],
    slots: &[Slot],
    (start, starts): (usize, &Starts),
    spacing: &Spacing,
    rules: Rules,
    picture: &dyn Fn(&BlockView, f64) -> Option<f64>,
) -> Vec<Page> {
    // The flow from a top of zero; the page's own top margin is the page's business.
    let spacing = Spacing {
        top: 0.0,
        ..*spacing
    };
    let viewport = app.get_viewport(0..app.block_count());
    // The flow laid out at every measure; each unit is taken from the one its run of pages is set
    // at. A run opens on a page of its own, so no page mixes two.
    let laid: Vec<Vec<Unit>> = measures
        .iter()
        .map(|&(faces, column)| {
            let laid = flow::lay_out(app, faces, column, &spacing, picture);
            units(app, faces, &laid, &viewport, picture, spacing.cell_pad)
        })
        .collect();
    let measure_of = |slot: usize| slots.get(slot).map_or(0, |s| s.measure).min(laid.len() - 1);
    let mut measure = measure_of(start);
    // Each unit, and where its run's flow sits against the run before it — what an empty page's
    // origin moves by when a run starts on it.
    let units: Vec<(&Unit, f64)> = (0..laid[0].len())
        .map(|at| {
            let before = measure;
            if let Unit::Block { slot, .. } = &laid[0][at]
                && let Some(&(slot_of, _)) = starts.get(&slot.index)
            {
                measure = measure_of(slot_of);
            }
            (
                &laid[measure][at],
                laid[measure][at].top() - laid[before][at].top(),
            )
        })
        .collect();
    let mut cut = Cut {
        pages: vec![Page {
            slot: start,
            ..Page::default()
        }],
        origin: 0.0,
        slots,
        reserved: 0.0,
    };
    // A page break after the block before: the next unit starts a page wherever it is.
    let mut break_pending = false;
    let mut heading: Vec<(&str, usize)> = Vec::new();
    let mut heading_closed = false;
    for (at, &(unit, shift)) in units.iter().enumerate() {
        if std::mem::take(&mut break_pending) && !cut.fresh() {
            cut.turn(unit.broken_top());
        }
        match unit {
            Unit::Block {
                slot,
                lines,
                keep,
                breaks,
                notes,
                kept,
            } => {
                // A block opening a page on its own master: a new page unless this one is empty,
                // set on that master.
                if let Some(&(slot_of, restart)) = starts.get(&slot.index) {
                    match cut.fresh() {
                        true => cut.origin += shift,
                        false => cut.turn(unit.broken_top()),
                    }
                    cut.set_slot(slot_of, restart);
                }
                if breaks.page_before && !cut.fresh() {
                    cut.turn(unit.broken_top());
                }
                break_pending = breaks.page_after;
                if *keep
                    && !cut.fresh()
                    && let (Some(last), Some(next)) = (lines.last(), units.get(at + 1))
                    && next.0.lead(rules.orphans).max(last.bottom()) - cut.origin
                        > cut.page_height() + EPS
                {
                    cut.turn(lines[0].top - kept.0);
                }
                cut.block(slot, lines, rules, *notes, kept.0);
            }
            Unit::Row {
                slots,
                top,
                bottom,
                cells,
                table,
                header,
                kept,
            } => {
                // The table's heading rows, kept to repeat on every page it continues on; its
                // first non-heading row ends the list.
                if heading.first().is_some_and(|&(t, _)| t != table.as_str()) {
                    heading.clear();
                }
                if *header && !heading_closed {
                    heading.push((table.as_str(), at));
                } else {
                    heading_closed = !*header;
                }
                // A new page for the row from the flow's `at` on, its table's heading rows
                // repeated above it.
                let repeats =
                    !*header && heading.first().is_some_and(|&(t, _)| t == table.as_str());
                let turn = |cut: &mut Cut<'_>, at: f64| {
                    cut.turn(at);
                    if repeats {
                        let first = heading.first().map_or(at, |&(_, i)| units[i].0.top());
                        let mut tall = 0.0;
                        for &(_, i) in &heading {
                            if let Unit::Row {
                                slots,
                                cells,
                                bottom,
                                ..
                            } = units[i].0
                            {
                                let from = vec![0; slots.len()];
                                cut.place_row(slots, &from, None, cells, (first, None), true);
                                tall = bottom - first;
                            }
                        }
                        cut.origin = at - tall;
                    }
                };
                if *header || *kept {
                    if *bottom - cut.origin > cut.room() + EPS && !cut.fresh() {
                        turn(&mut cut, *top);
                    }
                    let from = vec![0; slots.len()];
                    cut.place_row(slots, &from, None, cells, (*top, None), false);
                    continue;
                }
                // Each block's first line not yet placed, and where the row's part on the
                // current page starts in the flow.
                let mut from = vec![0; slots.len()];
                let mut part = *top;
                let mut turned = false;
                loop {
                    let limit = cut.origin + cut.room();
                    if *bottom <= limit + EPS {
                        cut.place_row(slots, &from, None, cells, (part, None), false);
                        break;
                    }
                    let take: Vec<usize> = slots
                        .iter()
                        .zip(&from)
                        .map(|((_, lines), &start)| {
                            lines[start..]
                                .iter()
                                .take_while(|line| line.bottom() + spacing.cell_pad <= limit + EPS)
                                .count()
                        })
                        .collect();
                    if take.iter().all(|&n| n == 0) && (cut.fresh() || turned) {
                        // Not one line fits even on a page of its own: it overflows.
                        cut.place_row(slots, &from, None, cells, (part, None), false);
                        break;
                    }
                    if take.iter().any(|&n| n > 0) {
                        let end: Vec<usize> = from.iter().zip(&take).map(|(f, t)| f + t).collect();
                        let foot = slots
                            .iter()
                            .zip(from.iter().zip(&end))
                            .filter(|(_, (f, e))| e > f)
                            .map(|((_, lines), (_, &e))| lines[e - 1].bottom())
                            .fold(part, f64::max)
                            + spacing.cell_pad;
                        cut.place_row(slots, &from, Some(&end), cells, (part, Some(foot)), false);
                        from = end;
                    }
                    let rest = slots
                        .iter()
                        .zip(&from)
                        .filter_map(|((_, lines), &start)| lines.get(start).map(|line| line.top))
                        .fold(f64::INFINITY, f64::min);
                    if rest.is_infinite() {
                        // Every line placed; what is left is the row's own height, which a page
                        // break may end.
                        break;
                    }
                    part = match from.iter().all(|&start| start == 0) {
                        true => *top,
                        false => rest - spacing.cell_pad,
                    };
                    turn(&mut cut, part);
                    turned = true;
                }
            }
        }
    }
    cut.pages
}

/// Slack for comparing positions that are sums of floats.
const EPS: f64 = 1e-9;

/// One line of a block, in the flow's coordinates.
#[derive(Clone, Copy, Debug)]
struct Line {
    top: f64,
    height: f64,
}

impl Line {
    fn bottom(&self) -> f64 {
        self.top + self.height
    }
}

/// What the cut walks: a block, which may split between lines, or a table row, which may not.
enum Unit {
    Block {
        slot: flow::Slot,
        lines: Vec<Line>,
        /// The room its footnotes take at the foot of the page it starts on.
        notes: f64,
        /// Whether a page may not end on it: a heading, unless its style says otherwise.
        keep: bool,
        breaks: Breaks,
        /// What stays of its space above when it starts a page: flowed onto one, after a break.
        kept: (f64, f64),
    },
    Row {
        /// Each block in the row and its lines.
        slots: Vec<(flow::Slot, Vec<Line>)>,
        top: f64,
        bottom: f64,
        cells: Vec<CellBox>,
        /// The table it is a row of, and whether it is one of that table's heading rows.
        table: String,
        header: bool,
        /// Whether it moves whole rather than splitting between lines.
        kept: bool,
    },
}

impl Unit {
    /// Where the unit starts in the flow: the top of its first line, or of its row.
    fn top(&self) -> f64 {
        match self {
            Unit::Block { slot, lines, .. } => lines.first().map_or(slot.top, |line| line.top),
            Unit::Row { top, .. } => *top,
        }
    }

    /// Where a page it starts after a page break begins in the flow: above it by what of its
    /// space above stays.
    fn broken_top(&self) -> f64 {
        match self {
            Unit::Block { kept, .. } => self.top() - kept.1,
            Unit::Row { .. } => self.top(),
        }
    }

    /// Where the part of this unit a heading above it must stay with ends: its first `orphans`
    /// lines, a row's first line in each cell, or the whole of a row that does not split.
    fn lead(&self, orphans: usize) -> f64 {
        match self {
            Unit::Block { lines, .. } => lines
                .get(orphans.clamp(1, lines.len().max(1)) - 1)
                .map_or(0.0, Line::bottom),
            Unit::Row {
                bottom,
                header: false,
                kept: false,
                slots,
                ..
            } => slots
                .iter()
                .filter_map(|(_, lines)| lines.first().map(Line::bottom))
                .reduce(f64::max)
                .map_or(*bottom, |lead| lead.min(*bottom)),
            Unit::Row { bottom, .. } => *bottom,
        }
    }
}

/// The flow as units, in order: every stacked block with its lines, and every table row whole.
fn units(
    app: &App,
    faces: &dyn Faces,
    laid: &flow::Flow,
    viewport: &crate::Viewport,
    picture: &dyn Fn(&BlockView, f64) -> Option<f64>,
    pad: f64,
) -> Vec<Unit> {
    // A block's lines in flow coordinates; a picture or an unlaid-out block is one line as tall as
    // its slot.
    let lines_of = |slot: &flow::Slot, view: &BlockView| -> Vec<Line> {
        let whole = vec![Line {
            top: slot.top,
            height: slot.height,
        }];
        if picture(view, slot.width).is_some() {
            return whole;
        }
        let (width, metrics) = faces.of(slot.index, &view.kind, view.style.as_deref());
        match app.layout_block_tabbed(
            slot.index,
            width,
            metrics,
            faces.first_indent(slot.index),
            &faces.tabs(slot.index),
        ) {
            Ok(layout) if !layout.lines().is_empty() => layout
                .lines()
                .iter()
                .map(|line| Line {
                    top: slot.top + f64::from(line.top),
                    height: f64::from(line.height),
                })
                .collect(),
            _ => whole,
        }
    };

    let mut out: Vec<Unit> = Vec::new();
    // The table and row of the last unit, so consecutive blocks of one row are one unit.
    let mut row: Option<(String, u32)> = None;
    for slot in laid.slots() {
        let Some(view) = viewport.get(slot.index) else {
            continue;
        };
        let lines = lines_of(slot, view);
        match &view.cell {
            None => {
                row = None;
                let breaks = faces.breaks(slot.index).unwrap_or_default();
                let heading = matches!(view.kind, crate::BlockKind::Heading { .. });
                out.push(Unit::Block {
                    kept: faces.kept_above(slot.index),
                    notes: faces.footnote_room(slot.index),
                    slot: *slot,
                    keep: breaks.keep_with_next.unwrap_or(heading),
                    breaks,
                    lines,
                });
            }
            Some(cell) => {
                let key = (cell.table.clone(), cell.row);
                let top = slot.top - pad;
                let bottom = lines.last().map_or(slot.bottom(), Line::bottom) + pad;
                match (row.as_ref() == Some(&key), out.last_mut()) {
                    (
                        true,
                        Some(Unit::Row {
                            slots,
                            top: t,
                            bottom: b,
                            ..
                        }),
                    ) => {
                        slots.push((*slot, lines));
                        *t = t.min(top);
                        *b = b.max(bottom);
                    }
                    _ => {
                        let header = faces.header_row(&key.0, key.1);
                        out.push(Unit::Row {
                            slots: vec![(*slot, lines)],
                            top,
                            bottom,
                            cells: Vec::new(),
                            table: key.0.clone(),
                            header,
                            kept: faces.row_kept(&key.0, key.1),
                        });
                        row = Some(key);
                    }
                }
            }
        }
    }
    // A row is at least as tall as its tallest cell box; a cell is found by its first block, since
    // padding or centring puts its text anywhere in it.
    for unit in &mut out {
        if let Unit::Row {
            slots,
            top,
            bottom,
            cells,
            ..
        } = unit
        {
            cells.extend(
                laid.cells()
                    .iter()
                    .filter(|cell| slots.iter().any(|(slot, _)| slot.index == cell.first)),
            );
            *top = cells.iter().map(|cell| cell.top).fold(*top, f64::min);
            *bottom = cells.iter().map(CellBox::bottom).fold(*bottom, f64::max);
        }
    }
    out
}

/// The cut in progress: the pages so far, and where in the flow the current one begins.
struct Cut<'a> {
    pages: Vec<Page>,
    /// The flow's y at the top of the current page.
    origin: f64,
    /// The kinds of page there are: each page's text area, and what follows it.
    slots: &'a [Slot],
    /// What the footnotes cited on the current page take from the foot of it.
    reserved: f64,
}

impl Cut<'_> {
    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().expect("there is always a page")
    }

    /// Whether nothing is on the current page yet — when something too big goes on anyway, since
    /// the next page would be no different.
    fn fresh(&self) -> bool {
        self.pages.last().is_none_or(|page| page.pieces.is_empty())
    }

    /// Start a new page whose top is the flow's `at`.
    fn turn(&mut self, at: f64) {
        let next = self.slots[self.pages.last().map_or(0, |page| page.slot)].next;
        self.pages.push(Page {
            slot: next,
            ..Page::default()
        });
        self.origin = at;
        self.reserved = 0.0;
    }

    /// Set the current page, which has nothing on it, on another kind of page.
    fn set_slot(&mut self, slot: usize, restart: Option<u32>) {
        let page = self.page();
        page.slot = slot;
        page.restart = restart;
    }

    /// How tall the current page's text area is.
    fn page_height(&self) -> f64 {
        self.slots[self.pages.last().map_or(0, |page| page.slot)].height
    }

    /// How tall the body of the current page may be, its footnotes' room taken out.
    fn room(&self) -> f64 {
        self.page_height() - self.reserved
    }

    /// Place a row's blocks — each one's lines from `from` to `to`, or to its last — and its
    /// cells, cut to the part of the row from `part.0` in the flow to `part.1` or each cell's
    /// own bottom. A repeated heading (`repeat`) is placed from its own first row's top; a row
    /// in place from the page's origin.
    fn place_row(
        &mut self,
        slots: &[(flow::Slot, Vec<Line>)],
        from: &[usize],
        to: Option<&[usize]>,
        cells: &[CellBox],
        (top, foot): (f64, Option<f64>),
        repeat: bool,
    ) {
        let origin = if repeat { top } else { self.origin };
        let page = self.page();
        for (at, (slot, lines)) in slots.iter().enumerate() {
            let start = from.get(at).copied().unwrap_or(0);
            let end = to.and_then(|to| to.get(at).copied()).unwrap_or(lines.len());
            if start >= end {
                continue;
            }
            let first = match start {
                0 => slot.top,
                _ => lines[start].top,
            };
            page.pieces.push(Piece {
                index: slot.index,
                lines: start..end,
                top: first - origin,
                left: slot.indent,
                width: slot.width,
                repeat,
            });
        }
        page.cells.extend(cells.iter().map(|cell| {
            let from = cell.top.max(top);
            let to = foot.unwrap_or_else(|| cell.bottom());
            CellBox {
                top: from - origin,
                height: (to - from).max(0.0),
                ..*cell
            }
        }));
    }

    /// Place a block, splitting it between lines as often as it takes.
    /// Cut a block's lines into pages; a page it moves to whole starts `kept` above its first.
    fn block(&mut self, slot: &flow::Slot, lines: &[Line], rules: Rules, notes: f64, kept: f64) {
        let mut start = 0;
        while start < lines.len() {
            let remaining = lines.len() - start;
            // A block's footnotes go on its first line's page, shortening that page for its lines
            // too.
            let extra = if start == 0 { notes } else { 0.0 };
            let room = self.room() - extra;
            let fits = lines[start..]
                .iter()
                .take_while(|line| line.bottom() - self.origin <= room + EPS)
                .count();
            let mut take = fits;
            if take < remaining {
                if remaining - take < rules.widows {
                    take = remaining.saturating_sub(rules.widows);
                }
                if take < rules.orphans {
                    take = 0;
                }
                if take == 0 && self.fresh() {
                    take = fits.clamp(1, remaining);
                }
            }
            if take > 0 {
                let top = lines[start].top - self.origin;
                self.page().pieces.push(Piece {
                    index: slot.index,
                    lines: start..start + take,
                    top,
                    left: slot.indent,
                    width: slot.width,
                    repeat: false,
                });
                if start == 0 {
                    self.reserved += extra;
                }
                start += take;
            }
            if start < lines.len() {
                self.turn(lines[start].top - if start == 0 { kept } else { 0.0 });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockKind, Caret, Fixed, Uniform};
    use std::collections::HashMap;

    /// Every gap zero, so a line is exactly one unit and every number below is a line count.
    const TIGHT: Spacing = Spacing {
        top: 0.0,
        gap: 0.0,
        heading: 0.0,
        indent: 2.0,
        cell_pad: 0.0,
    };

    /// At `Fixed` and a measure of 10, `"xxxxxxxx "` is one line, so `n` of them are `n` lines.
    fn lines(n: usize) -> String {
        "xxxxxxxx ".repeat(n)
    }

    /// A document of these paragraphs, in order, and nothing else.
    fn doc(blocks: &[(BlockKind, String)]) -> App {
        let app = App::new();
        for (at, (kind, text)) in blocks.iter().enumerate() {
            app.insert(at, kind.clone(), text).unwrap();
        }
        // `App::new()`'s own empty paragraph is last, and is no part of these tests.
        app.delete(blocks.len()..blocks.len() + 1).unwrap();
        app
    }

    fn para(n: usize) -> (BlockKind, String) {
        (BlockKind::Paragraph, lines(n))
    }

    fn cut(app: &App, height: f64, spacing: &Spacing) -> Vec<Page> {
        let faces = Uniform::new(10.0, &Fixed);
        paginate(
            app,
            &faces,
            10.0,
            height,
            spacing,
            Rules::default(),
            &|_, _| None,
        )
    }

    /// Each page as `(block, lines)` pairs — what almost every assertion is about.
    fn shape(pages: &[Page]) -> Vec<Vec<(usize, Range<usize>)>> {
        pages
            .iter()
            .map(|page| {
                page.pieces
                    .iter()
                    .map(|p| (p.index, p.lines.clone()))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_short_document_is_one_page_stacked_as_the_flow_stacks_it() {
        let app = doc(&[para(1), para(2), para(1)]);
        let pages = cut(&app, 50.0, &TIGHT);
        assert_eq!(shape(&pages), vec![vec![(0, 0..1), (1, 0..2), (2, 0..1)]]);
        let tops: Vec<f64> = pages[0].pieces.iter().map(|p| p.top).collect();
        assert_eq!(tops, vec![0.0, 1.0, 3.0]);
        assert_eq!(pages[0].pieces[0].width, 10.0);
    }

    #[test]
    fn a_document_with_no_blocks_is_one_empty_page() {
        let app = doc(&[]);
        assert_eq!(app.block_count(), 0);
        assert_eq!(cut(&app, 5.0, &TIGHT), vec![Page::default()]);
    }

    #[test]
    fn a_long_paragraph_continues_at_the_top_of_the_next_page() {
        let app = doc(&[para(7)]);
        let pages = cut(&app, 5.0, &TIGHT);
        assert_eq!(shape(&pages), vec![vec![(0, 0..5)], vec![(0, 5..7)]]);
        assert_eq!(pages[1].pieces[0].top, 0.0, "the rest starts at the top");
    }

    #[test]
    fn a_single_last_line_is_not_left_alone_at_the_top_of_a_page() {
        // Five lines fit; the sixth would be a widow, so the fifth goes over with it.
        let app = doc(&[para(6)]);
        assert_eq!(
            shape(&cut(&app, 5.0, &TIGHT)),
            vec![vec![(0, 0..4)], vec![(0, 4..6)]]
        );
    }

    #[test]
    fn a_single_first_line_is_not_left_alone_at_the_foot_of_a_page() {
        // Four lines, then one line of a three-line paragraph would fit: an orphan, so the
        // paragraph moves whole.
        let app = doc(&[para(4), para(3)]);
        assert_eq!(
            shape(&cut(&app, 5.0, &TIGHT)),
            vec![vec![(0, 0..4)], vec![(1, 0..3)]]
        );
    }

    #[test]
    fn a_heading_is_kept_with_what_follows_it() {
        // The heading would fit as the fifth line, and nothing of its paragraph after it.
        let heading = (BlockKind::Heading { level: 1 }, "Title".to_owned());
        let app = doc(&[para(4), heading, para(3)]);
        assert_eq!(
            shape(&cut(&app, 5.0, &TIGHT)),
            vec![vec![(0, 0..4)], vec![(1, 0..1), (2, 0..3)]]
        );
    }

    #[test]
    fn space_above_a_block_is_dropped_at_the_top_of_a_page() {
        let spaced = Spacing { gap: 1.0, ..TIGHT };
        // Three lines, a gap, then room for one line of three: the second paragraph moves,
        // starting at the very top.
        let app = doc(&[para(3), para(3)]);
        let pages = cut(&app, 5.0, &spaced);
        assert_eq!(shape(&pages), vec![vec![(0, 0..3)], vec![(1, 0..3)]]);
        assert_eq!(pages[1].pieces[0].top, 0.0);
    }

    #[test]
    fn a_page_shorter_than_a_line_still_takes_one_so_the_cut_ends() {
        let app = doc(&[para(3)]);
        let pages = cut(&app, 0.5, &TIGHT);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[2].pieces[0].lines, 2..3);
    }

    #[test]
    fn a_list_item_keeps_its_indent_on_the_page() {
        let item = (BlockKind::ListItem { depth: 1 }, "item".to_owned());
        let app = doc(&[item]);
        let pages = cut(&app, 5.0, &TIGHT);
        assert_eq!(pages[0].pieces[0].left, 2.0);
    }

    #[test]
    fn a_row_of_one_line_cells_moves_whole_with_its_cells() {
        // Four lines, then a two-by-two table of one-line cells: the first row fits, the second
        // moves whole.
        let app = doc(&[para(4)]);
        app.insert_table(1, 2, 2, Some("T".into())).unwrap();
        for index in 1..5 {
            app.insert_text(
                Caret {
                    block: index,
                    offset: 0,
                },
                "c",
            )
            .unwrap();
        }
        let pages = cut(&app, 5.0, &TIGHT);
        assert_eq!(
            shape(&pages),
            vec![
                vec![(0, 0..4), (1, 0..1), (2, 0..1)],
                vec![(3, 0..1), (4, 0..1)]
            ]
        );
        assert_eq!(pages[0].cells.len(), 2);
        assert_eq!(pages[1].cells.len(), 2);
        assert_eq!(
            pages[1].cells[0].top, 0.0,
            "a row carried over starts at the top"
        );
        assert_eq!(pages[1].pieces[0].top, 0.0);
    }

    /// A one-row table after `before` lines: a cell of four lines beside one of one.
    fn tall_row(before: usize) -> App {
        let app = doc(&[para(before)]);
        app.insert_table(1, 1, 2, Some("T".into())).unwrap();
        for (block, text) in [(1, lines(4)), (2, "e".to_owned())] {
            app.insert_text(Caret { block, offset: 0 }, &text).unwrap();
        }
        app
    }

    /// `doc/odt-format.md` §5c fact 25: a row that does not fit splits between lines, though it
    /// would fit whole on the next page, and each part's cells close under it.
    #[test]
    fn a_row_that_does_not_fit_splits_between_lines() {
        let pages = cut(&tall_row(3), 5.0, &TIGHT);
        assert_eq!(
            shape(&pages),
            vec![vec![(0, 0..3), (1, 0..2), (2, 0..1)], vec![(1, 2..4)]]
        );
        let boxes = |page: &Page| -> Vec<(f64, f64)> {
            page.cells.iter().map(|c| (c.top, c.height)).collect()
        };
        assert_eq!(boxes(&pages[0]), vec![(3.0, 2.0), (3.0, 2.0)]);
        // The cell whose lines all fitted is drawn empty beside the rest, as Writer draws it.
        assert_eq!(boxes(&pages[1]), vec![(0.0, 2.0), (0.0, 2.0)]);
        assert_eq!(pages[1].pieces[0].top, 0.0);
    }

    /// A face whose every table row keeps together.
    struct Kept;

    impl Faces for Kept {
        fn of(&self, _: usize, _: &BlockKind, _: Option<&str>) -> (f32, &dyn crate::Metrics) {
            (10.0, &Fixed)
        }
        fn row_kept(&self, _table: &str, _row: u32) -> bool {
            true
        }
    }

    #[test]
    fn a_row_that_keeps_together_moves_whole() {
        let pages = paginate(
            &tall_row(3),
            &Kept,
            10.0,
            5.0,
            &TIGHT,
            Rules::default(),
            &|_, _| None,
        );
        assert_eq!(
            shape(&pages),
            vec![vec![(0, 0..3)], vec![(1, 0..4), (2, 0..1)]]
        );
    }

    /// A face that answers [`Faces::breaks`] for some blocks.
    struct Breaking(HashMap<usize, Breaks>);

    impl Faces for Breaking {
        fn of(&self, _: usize, _: &BlockKind, _: Option<&str>) -> (f32, &dyn crate::Metrics) {
            (10.0, &Fixed)
        }
        fn breaks(&self, index: usize) -> Option<Breaks> {
            self.0.get(&index).copied()
        }
    }

    fn broken(app: &App, height: f64, breaks: &[(usize, Breaks)]) -> Vec<Page> {
        let faces = Breaking(breaks.iter().copied().collect());
        paginate(
            app,
            &faces,
            10.0,
            height,
            &TIGHT,
            Rules::default(),
            &|_, _| None,
        )
    }

    #[test]
    fn a_page_break_before_or_after_a_block_starts_a_new_page() {
        let app = doc(&[para(1), para(1), para(1)]);
        let before = Breaks {
            page_before: true,
            ..Breaks::default()
        };
        assert_eq!(
            shape(&broken(&app, 50.0, &[(1, before)])),
            vec![vec![(0, 0..1)], vec![(1, 0..1), (2, 0..1)]]
        );
        let after = Breaks {
            page_after: true,
            ..Breaks::default()
        };
        assert_eq!(
            shape(&broken(&app, 50.0, &[(1, after)])),
            vec![vec![(0, 0..1), (1, 0..1)], vec![(2, 0..1)]]
        );
        // A break before the very first block has nothing to break away from.
        assert_eq!(shape(&broken(&app, 50.0, &[(0, before)])).len(), 1);
    }

    #[test]
    fn a_style_keeping_a_paragraph_with_the_next_is_honoured_and_one_not_keeping_a_heading_is_too()
    {
        // A paragraph that keeps with the next behaves as a heading does…
        let app = doc(&[para(4), para(1), para(3)]);
        let keep = Breaks {
            keep_with_next: Some(true),
            ..Breaks::default()
        };
        assert_eq!(
            shape(&broken(&app, 5.0, &[(1, keep)])),
            vec![vec![(0, 0..4)], vec![(1, 0..1), (2, 0..3)]]
        );
        // …and a heading whose style says it does not, stays on the page it fits on.
        let heading = (BlockKind::Heading { level: 1 }, "Title".to_owned());
        let app = doc(&[para(4), heading, para(3)]);
        let loose = Breaks {
            keep_with_next: Some(false),
            ..Breaks::default()
        };
        assert_eq!(
            shape(&broken(&app, 5.0, &[(1, loose)])),
            vec![vec![(0, 0..4), (1, 0..1)], vec![(2, 0..3)]]
        );
    }

    /// A face that marks row 0 of every table as its heading.
    struct Headed;

    impl Faces for Headed {
        fn of(&self, _: usize, _: &BlockKind, _: Option<&str>) -> (f32, &dyn crate::Metrics) {
            (10.0, &Fixed)
        }
        fn header_row(&self, _table: &str, row: u32) -> bool {
            row == 0
        }
    }

    #[test]
    fn a_tables_heading_rows_repeat_at_the_top_of_every_page_it_continues_on() {
        let app = App::new();
        app.insert_table(0, 7, 1, Some("T".into())).unwrap();
        let count = app.block_count();
        for index in 0..count {
            app.insert_text(
                Caret {
                    block: index,
                    offset: 0,
                },
                "c",
            )
            .unwrap();
        }
        let pages = paginate(
            &app,
            &Headed,
            10.0,
            3.0,
            &TIGHT,
            Rules::default(),
            &|_, _| None,
        );
        let shaped: Vec<Vec<(usize, bool)>> = pages
            .iter()
            .map(|page| {
                page.pieces
                    .iter()
                    .filter(|p| p.index < 7)
                    .map(|p| (p.index, p.repeat))
                    .collect()
            })
            .collect();
        // Three rows a page: the heading and two rows, then the heading again on every page.
        assert_eq!(shaped[0], vec![(0, false), (1, false), (2, false)]);
        assert_eq!(shaped[1], vec![(0, true), (3, false), (4, false)]);
        assert_eq!(shaped[2], vec![(0, true), (5, false), (6, false)]);
        assert_eq!(pages[1].pieces[0].top, 0.0, "the heading at the top");
        assert_eq!(pages[1].pieces[1].top, 1.0, "the next row under it");
    }

    /// A face whose block 1 cites a footnote two units tall.
    struct Footnoted;

    impl Faces for Footnoted {
        fn of(&self, _: usize, _: &BlockKind, _: Option<&str>) -> (f32, &dyn crate::Metrics) {
            (10.0, &Fixed)
        }
        fn footnote_room(&self, index: usize) -> f64 {
            if index == 1 { 2.0 } else { 0.0 }
        }
    }

    #[test]
    fn a_footnote_takes_its_room_from_the_page_its_citation_starts_on() {
        // Five units a page: block 0 two lines, block 1 one line citing a two-unit note (2 + 1 + 2
        // = 5), and block 2's two lines then do not fit.
        let app = doc(&[para(2), para(1), para(2)]);
        let pages = paginate(
            &app,
            &Footnoted,
            10.0,
            5.0,
            &TIGHT,
            Rules::default(),
            &|_, _| None,
        );
        assert_eq!(
            shape(&pages),
            vec![vec![(0, 0..2), (1, 0..1)], vec![(2, 0..2)]]
        );
        // Without the note, all five lines share one page.
        assert_eq!(cut(&app, 5.0, &TIGHT).len(), 1);
    }

    #[test]
    fn a_picture_is_one_unit_that_moves_whole() {
        let app = doc(&[para(4), para(1)]);
        let pages = {
            let faces = Uniform::new(10.0, &Fixed);
            // Block 1 is a "picture" three units tall: it cannot fit under four lines.
            let picture = |view: &BlockView, _: f64| (view.text == lines(1)).then_some(3.0);
            paginate(&app, &faces, 10.0, 5.0, &TIGHT, Rules::default(), &picture)
        };
        assert_eq!(shape(&pages), vec![vec![(0, 0..4)], vec![(1, 0..1)]]);
    }
}
