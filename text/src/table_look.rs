// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a table looks like — its column widths, its cells' styles and its heading rows — read for
//! showing and printing, never written (`doc/pdf-export.md`): a side table by table name beside
//! the model's blocks, while a save carries the file's own table styles out (R6).
//!
//! Read from `style:table-properties`' width, margins and `table:align`,
//! `style:table-column-properties`' `style:column-width` (rng:13627),
//! `style:table-cell-properties`' `fo:background-color`, borders, padding,
//! `style:border-line-width*` and `style:vertical-align` (rng:13710), and
//! `table:table-header-rows` (rng:13931). Where a border is drawn and what room it takes is
//! Writer's, measured (`doc/odt-format.md` §5c fact 18).

use std::collections::{BTreeSet, HashMap};

/// One cell's style: its fill, four borders (top, right, bottom, left; ODF's three-part form
/// verbatim), padding in millimetres in the same order, and vertical alignment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellLook {
    pub background: Option<String>,
    pub border: [Option<String>; 4],
    /// A `double` border's three widths in millimetres per side — `style:border-line-width`: inner
    /// line, gap, outer line.
    pub line_widths: [Option<[f64; 3]>; 4],
    pub padding: [f64; 4],
    pub vertical_align: Option<String>,
}

/// One table's look.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableLook {
    /// Each column's width in millimetres, where its column style states one.
    pub columns: Vec<Option<f64>>,
    /// The rows inside `table:table-header-rows`, repeated at the top of every page the table
    /// continues on.
    pub header_rows: BTreeSet<u32>,
    /// Each styled cell's look, by `(row, column)`.
    pub cells: HashMap<(u32, u32), CellLook>,
    /// The rows whose style says `fo:keep-together="always"`: never split across pages
    /// (`doc/odt-format.md` §5c fact 25).
    pub kept_rows: BTreeSet<u32>,
    /// Each row's least height in millimetres, where its style states one. A stated exact height
    /// (`style:row-height`) is taken as a least one: text a row cannot hold is shown, not clipped.
    pub row_heights: HashMap<u32, f64>,
    /// How many rows the table has, which is where its bottom border goes.
    pub rows: u32,
    /// Where its table style places it across the page, if it has one.
    pub placement: Option<Placement>,
}

/// A table style's `style:table-properties`: `table:align`, `style:width` and the side margins,
/// lengths in millimetres.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    /// `table:align` as spelled; absent reads as `margins`.
    pub align: Option<String>,
    pub width: Option<f64>,
    pub margin_left: f64,
    pub margin_right: f64,
}

impl Placement {
    /// The table's left edge from the text area's and its width, in points, across a text area
    /// `column` points wide (`doc/odt-format.md` §5c fact 26). `None` for `margins` with nothing
    /// to say, which is the whole text area.
    pub fn span(&self, column: f64) -> Option<(f64, f64)> {
        let pt = grind_core::page::pt;
        let width = self.width.map(pt).filter(|w| *w > 0.0);
        match (self.align.as_deref(), width) {
            (Some("left"), Some(width)) => Some((pt(self.margin_left), width)),
            (Some("right"), Some(width)) => Some((column - width, width)),
            (Some("center"), Some(width)) => Some(((column - width) / 2.0, width)),
            _ if self.margin_left == 0.0 && self.margin_right == 0.0 => None,
            _ => {
                let left = pt(self.margin_left);
                Some((left, (column - left - pt(self.margin_right)).max(1.0)))
            }
        }
    }
}

/// The four sides of a cell, in [`CellLook::border`]'s order.
pub const TOP: usize = 0;
pub const RIGHT: usize = 1;
pub const BOTTOM: usize = 2;
pub const LEFT: usize = 3;

/// One side of a cell's border as Writer draws it, in points: the band it occupies and its lines,
/// each `(offset, width)` from the band's outside edge.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub color: String,
    pub thickness: f64,
    pub lines: Vec<(f64, f64)>,
}

impl CellLook {
    /// `side`'s border, or `None` where nothing is drawn (`none`, zero width, or not ODF's three
    /// parts). A `double` border is as thick as its three widths together; across the top and
    /// bottom the outer width is outermost, down the sides the inner one is (§5c fact 18,
    /// measured). A `double` with no widths is three equal thirds.
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

    /// The room the border across the top of `row` takes, in points: the thicker of its top border
    /// and the row above's bottom one — Writer draws one band between rows (§5c fact 18).
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

    /// The room the border under `row` takes inside it, in points: its bottom border on the
    /// table's last row, nothing elsewhere (the next row's [`band_above`](Self::band_above)).
    pub fn band_below(&self, row: u32, column: u32) -> f64 {
        if row + 1 != self.rows {
            return 0.0;
        }
        self.cell(row, column)
            .and_then(|look| look.edge(BOTTOM))
            .map_or(0.0, |edge| edge.thickness)
    }
}

#[cfg(test)]
mod tests {
    use super::Placement;

    /// §5c fact 26's tables, on its 17 cm text area: each one's left edge and width in cm.
    #[test]
    fn a_table_sits_where_writer_puts_it() {
        let cm = |pt: f64| (pt * 2.54 / 72.0 * 100.0).round() / 100.0;
        let area = grind_core::page::pt(170.0);
        let at = |align: Option<&str>, width: f64, left: f64, right: f64| {
            let placement = Placement {
                align: align.map(str::to_owned),
                width: Some(width * 10.0),
                margin_left: left * 10.0,
                margin_right: right * 10.0,
            };
            placement.span(area).map(|(x, w)| (cm(x), cm(w)))
        };
        assert_eq!(at(Some("left"), 10.0, 2.0, 0.0), Some((2.0, 10.0)));
        assert_eq!(at(Some("left"), 20.0, -2.0, 0.0), Some((-2.0, 20.0)));
        assert_eq!(at(Some("center"), 10.0, 3.0, 0.0), Some((3.5, 10.0)));
        assert_eq!(at(Some("center"), 20.0, 0.0, 0.0), Some((-1.5, 20.0)));
        assert_eq!(at(Some("right"), 10.0, 0.0, 2.0), Some((7.0, 10.0)));
        assert_eq!(at(Some("margins"), 10.0, 1.0, 3.0), Some((1.0, 13.0)));
        assert_eq!(at(None, 20.0, -1.0, 0.0), Some((-1.0, 18.0)));
        assert_eq!(at(None, 10.0, 0.0, 0.0), None, "the whole area");
    }
}
