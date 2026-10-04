// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Breaking a document into pages — pure arithmetic over the flow (`doc/pdf-export.md` P4).
//!
//! **Pages are derived, never stored.** This runs when somebody exports or previews, and its
//! answer is thrown away afterwards, the way `doc/view-modes.md`'s roles are: a stored page
//! would go stale on the next keystroke and a derived one cannot. The editor stays a
//! continuous column in every shell.
//!
//! **Built on [`flow::lay_out`], not beside it.** The flow already decides where every block
//! sits down an endless page — the gaps, a heading's space above, a table's grid — and those
//! rules must not exist twice. What this adds is the cut: the flow is walked top to bottom and
//! each page takes as much of it as fits, splitting a paragraph *between lines* and never a
//! table row. The unit is the caller's, as everywhere in layout; the PDF uses points.
//!
//! **Three rules, and only three**, each a typographer's rather than a guess about LibreOffice:
//!
//! * *orphans* — a paragraph split across pages leaves at least this many lines at the bottom
//!   of the first page, or it moves whole;
//! * *widows* — and carries at least this many to the top of the next;
//! * a **heading is kept with what follows it**, so no page ends on a heading.
//!
//! Space above a block that lands at the top of a page is dropped, since nothing above it is
//! there to be spaced from. A row taller than a whole page overflows it rather than being split
//! (ponytail: splitting a row by lines is `doc/pdf-export.md` §5's named case, and no document in
//! hand needs it yet).

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
    /// Two and two: the typographer's usual minimum, and what Writer writes on a new document's
    /// default paragraph style. A caller printing a real document passes that document's own
    /// instead — `grind_print` does, and none where it states none (`doc/odt-format.md` §5c).
    fn default() -> Self {
        Rules {
            orphans: 2,
            widows: 2,
        }
    }
}

/// What a document's own paragraph style says about where pages break around a block
/// (`fo:break-before`, `fo:break-after`, `fo:keep-with-next`; rng:2073, rng:2110) — asked of the
/// [`Faces`] through [`Faces::breaks`], so a screen, which answers nothing, is unaffected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Breaks {
    /// `fo:break-before="page"`: the block starts a page.
    pub page_before: bool,
    /// `fo:break-after="page"`: the block ends one.
    pub page_after: bool,
    /// `fo:keep-with-next`: `Some` when the style says, overriding the rule that keeps a
    /// heading, and only a heading, with what follows it.
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
}

/// One page: what is on it, in the text area's coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub pieces: Vec<Piece>,
    /// The table cells on this page, for the rules drawn round them.
    pub cells: Vec<CellBox>,
}

/// Break the document into pages whose text area is `column` wide and `height` tall.
///
/// `faces` and `picture` are [`flow::lay_out`]'s, and the same answers must reach both, which is
/// why they are taken here rather than a finished flow: the cut needs each block's lines, and
/// those come from the same faces the flow was measured with.
pub fn paginate(
    app: &App,
    faces: &dyn Faces,
    column: f64,
    height: f64,
    spacing: &Spacing,
    rules: Rules,
    picture: &dyn Fn(&BlockView, f64) -> Option<f64>,
) -> Vec<Page> {
    // The flow's own idea of where everything sits, from a top of zero: a page has its own top
    // margin, which is the page's business rather than the flow's.
    let spacing = Spacing {
        top: 0.0,
        ..*spacing
    };
    let laid = flow::lay_out(app, faces, column, &spacing, picture);
    let viewport = app.get_viewport(0..app.block_count());
    let units = units(app, faces, &laid, &viewport, picture, spacing.cell_pad);
    let mut cut = Cut {
        pages: vec![Page::default()],
        origin: 0.0,
        height,
    };
    // A page break after the block before: the next unit starts a page wherever it is.
    let mut break_pending = false;
    for (at, unit) in units.iter().enumerate() {
        if std::mem::take(&mut break_pending) && !cut.fresh() {
            cut.turn(unit.top());
        }
        match unit {
            Unit::Block {
                slot,
                lines,
                keep,
                breaks,
            } => {
                if breaks.page_before && !cut.fresh() {
                    cut.turn(lines.first().map_or(slot.top, |line| line.top));
                }
                break_pending = breaks.page_after;
                if *keep
                    && !cut.fresh()
                    && let (Some(last), Some(next)) = (lines.last(), units.get(at + 1))
                    && next.lead(rules.orphans).max(last.bottom()) - cut.origin > height + EPS
                {
                    cut.turn(lines[0].top);
                }
                cut.block(slot, lines, rules);
            }
            Unit::Row {
                slots,
                top,
                bottom,
                cells,
            } => {
                if *bottom - cut.origin > height + EPS && !cut.fresh() {
                    cut.turn(*top);
                }
                let origin = cut.origin;
                let page = cut.page();
                for (slot, count) in slots {
                    page.pieces.push(Piece {
                        index: slot.index,
                        lines: 0..*count,
                        top: slot.top - origin,
                        left: slot.indent,
                        width: slot.width,
                    });
                }
                page.cells.extend(cells.iter().map(|cell| CellBox {
                    top: cell.top - origin,
                    ..*cell
                }));
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

/// What the cut walks: a block, which may split between its lines, or a table row, which may
/// not split at all.
enum Unit {
    Block {
        slot: flow::Slot,
        lines: Vec<Line>,
        /// Whether a page may not end on it: a heading, unless its style says otherwise.
        keep: bool,
        breaks: Breaks,
    },
    Row {
        /// Each block in the row and how many lines it has.
        slots: Vec<(flow::Slot, usize)>,
        top: f64,
        bottom: f64,
        cells: Vec<CellBox>,
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

    /// Where the part of this unit a heading above it must stay with ends: its first
    /// `orphans` lines, or the whole row.
    fn lead(&self, orphans: usize) -> f64 {
        match self {
            Unit::Block { lines, .. } => lines
                .get(orphans.clamp(1, lines.len().max(1)) - 1)
                .map_or(0.0, Line::bottom),
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
    // A block's lines in the flow's coordinates. A picture, or a block the core could not lay
    // out, is one line as tall as its slot — the flow already decided how tall that is.
    let lines_of = |slot: &flow::Slot, view: &BlockView| -> Vec<Line> {
        let whole = vec![Line {
            top: slot.top,
            height: slot.height,
        }];
        if picture(view, slot.width).is_some() {
            return whole;
        }
        let (width, metrics) = faces.of(slot.index, &view.kind, view.style.as_deref());
        match app.layout_block_indented(slot.index, width, metrics, faces.first_indent(slot.index))
        {
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
    // The table and row the last unit was, when it was a row — consecutive blocks in the same
    // row of the same table are one unit.
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
                        slots.push((*slot, lines.len()));
                        *t = t.min(top);
                        *b = b.max(bottom);
                    }
                    _ => {
                        row = Some(key);
                        out.push(Unit::Row {
                            slots: vec![(*slot, lines.len())],
                            top,
                            bottom,
                            cells: Vec::new(),
                        });
                    }
                }
            }
        }
    }
    // A row's cells are the boxes that start where it does, and a cell's box reaches the row's
    // full height, so the row is at least as tall as its tallest box.
    for unit in &mut out {
        if let Unit::Row {
            top, bottom, cells, ..
        } = unit
        {
            cells.extend(
                laid.cells()
                    .iter()
                    .filter(|cell| (cell.top - *top).abs() < EPS),
            );
            *bottom = cells.iter().map(CellBox::bottom).fold(*bottom, f64::max);
        }
    }
    out
}

/// The cut in progress: the pages so far, and where in the flow the current one begins.
struct Cut {
    pages: Vec<Page>,
    /// The flow's y at the top of the current page.
    origin: f64,
    height: f64,
}

impl Cut {
    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().expect("there is always a page")
    }

    /// Whether nothing is on the current page yet — the one case where something too big for a
    /// page goes on it anyway, because the next page would be no different.
    fn fresh(&self) -> bool {
        self.pages.last().is_none_or(|page| page.pieces.is_empty())
    }

    /// Start a new page whose top is the flow's `at`.
    fn turn(&mut self, at: f64) {
        self.pages.push(Page::default());
        self.origin = at;
    }

    /// Place a block, splitting it between lines as often as it takes.
    fn block(&mut self, slot: &flow::Slot, lines: &[Line], rules: Rules) {
        let mut start = 0;
        while start < lines.len() {
            let remaining = lines.len() - start;
            let fits = lines[start..]
                .iter()
                .take_while(|line| line.bottom() - self.origin <= self.height + EPS)
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
                });
                start += take;
            }
            if start < lines.len() {
                self.turn(lines[start].top);
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

    /// Under `Fixed` at a measure of 10, `"xxxxxxxx "` is one line, so a paragraph of `n` of
    /// them is `n` lines tall.
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
        // Four lines, then one more line fits of a three-line paragraph: an orphan, so the
        // whole paragraph moves.
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
        // Three lines, a gap, then three lines that fit only one line: the second paragraph
        // moves, and starts at the very top rather than one gap down.
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
    fn a_table_row_is_never_split_and_its_cells_go_with_it() {
        // A four-line paragraph, then a two-by-two table of one-line cells: the first row fits
        // as the fifth line, the second goes to the next page whole with its two cells.
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

    /// A face that answers [`Faces::breaks`] for some blocks and leaves the rest to the
    /// paginator's own rules.
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
