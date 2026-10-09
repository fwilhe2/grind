// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a table looks like — its column widths, its cells' styles and which of its rows are its
//! heading — **read for showing and printing, never written** (`doc/pdf-export.md`).
//!
//! The model's table is its blocks and their cells' coordinates (`crate::model::Cell`), and that
//! is unchanged: this is a side table by table name, the way [`crate::Document::page`] is beside
//! the blocks. A save carries the file's own `table:style-name`s and `table:table-column`s out as
//! it always has (R6), so nothing here needs writing to survive.
//!
//! Sources: `style:table-column-properties`' `style:column-width` (rng:13627),
//! `style:table-cell-properties`' `fo:background-color`, `fo:border`/`fo:border-*`,
//! `fo:padding`/`fo:padding-*`, `style:border-line-width`/`style:border-line-width-*` and
//! `style:vertical-align` (rng:13710), and `table:table-header-rows` (rng:13931). Where a border
//! is drawn and how much room it takes is Writer's, measured: `doc/odt-format.md` §5c fact 18.

use std::collections::{BTreeSet, HashMap};

/// One cell's style: its fill, its four borders (top, right, bottom, left — ODF's three-part
/// `"0.5pt solid #000000"`, verbatim), its padding in millimetres in the same order, and its
/// vertical alignment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellLook {
    pub background: Option<String>,
    pub border: [Option<String>; 4],
    /// A `double` border's three widths in millimetres, side by side as `border` — ODF's
    /// `style:border-line-width`: the inner line, the gap and the outer line.
    pub line_widths: [Option<[f64; 3]>; 4],
    pub padding: [f64; 4],
    pub vertical_align: Option<String>,
}

/// One table's look.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableLook {
    /// Each column's width in millimetres, where its column style states one.
    pub columns: Vec<Option<f64>>,
    /// The rows inside `table:table-header-rows`, which repeat at the top of every page the
    /// table continues on.
    pub header_rows: BTreeSet<u32>,
    /// Each styled cell's look, by `(row, column)`.
    pub cells: HashMap<(u32, u32), CellLook>,
    /// How many rows the table has, which is where its bottom border goes.
    pub rows: u32,
}

/// The four sides of a cell, in [`CellLook::border`]'s order.
pub const TOP: usize = 0;
pub const RIGHT: usize = 1;
pub const BOTTOM: usize = 2;
pub const LEFT: usize = 3;

/// One side of a cell's border as Writer draws it, in points: the band it occupies and the lines
/// in that band, each `(offset, width)` measured **from the band's outside edge** — the edge away
/// from the cell's content.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub color: String,
    pub thickness: f64,
    pub lines: Vec<(f64, f64)>,
}

impl CellLook {
    /// `side`'s border, or `None` where it draws nothing (`none`, a width of zero, or a
    /// spelling that is not ODF's three parts).
    ///
    /// A `double` border is as thick as its three `border-line-width`s together, not its stated
    /// width, and its two lines go on the outside in an order that depends on the side: across
    /// the top and bottom the *outer* width is outermost, down the left and right the *inner*
    /// one is (§5c fact 18, measured with unequal widths). A `double` with no widths stated is
    /// three equal thirds.
    pub fn edge(&self, side: usize) -> Option<Edge> {
        let border = self.border.get(side)?.as_deref()?;
        let (width, style, color) = grind_core::style::border_parts(border)?;
        if width <= 0.0 || style == "none" || style == "hidden" {
            return None;
        }
        let pt = |mm: f64| mm * 72.0 / 25.4;
        let (thickness, lines) = match (style, self.line_widths.get(side).copied().flatten()) {
            ("double", Some([inner, gap, outer])) => {
                let (inner, gap, outer) = (pt(inner), pt(gap), pt(outer));
                let (first, second) = match side {
                    TOP | BOTTOM => (outer, inner),
                    _ => (inner, outer),
                };
                (
                    first + gap + second,
                    vec![(0.0, first), (first + gap, second)],
                )
            }
            ("double", None) => (
                width,
                vec![(0.0, width / 3.0), (width * 2.0 / 3.0, width / 3.0)],
            ),
            _ => (width, vec![(0.0, width)]),
        };
        Some(Edge {
            color: color.to_owned(),
            thickness,
            lines,
        })
    }
}

impl TableLook {
    /// The look of the cell at `row`, `column`, if its style says anything.
    pub fn cell(&self, row: u32, column: u32) -> Option<&CellLook> {
        self.cells.get(&(row, column))
    }

    /// How much room the border across the top of `row` takes, in points: the thicker of that
    /// row's own top border and the bottom border of the row above it — Writer draws one band
    /// between two rows, not two (§5c fact 18).
    pub fn band_above(&self, row: u32, column: u32) -> f64 {
        let thickness = |look: Option<&CellLook>, side| {
            look.and_then(|look| look.edge(side))
                .map_or(0.0, |edge| edge.thickness)
        };
        let own = thickness(self.cell(row, column), TOP);
        let above = row
            .checked_sub(1)
            .map_or(0.0, |up| thickness(self.cell(up, column), BOTTOM));
        own.max(above)
    }

    /// How much room the border under `row` takes inside it, in points: its bottom border's
    /// thickness on the table's last row, and nothing on any other, whose bottom border is the
    /// next row's [`band_above`](Self::band_above).
    pub fn band_below(&self, row: u32, column: u32) -> f64 {
        if row + 1 != self.rows {
            return 0.0;
        }
        self.cell(row, column)
            .and_then(|look| look.edge(BOTTOM))
            .map_or(0.0, |edge| edge.thickness)
    }
}
