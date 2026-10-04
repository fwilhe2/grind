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
//! `fo:padding`/`fo:padding-*` and `style:vertical-align` (rng:13710), and
//! `table:table-header-rows` (rng:13931).

use std::collections::{BTreeSet, HashMap};

/// One cell's style: its fill, its four borders (top, right, bottom, left — ODF's three-part
/// `"0.5pt solid #000000"`, verbatim), its padding in millimetres in the same order, and its
/// vertical alignment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellLook {
    pub background: Option<String>,
    pub border: [Option<String>; 4],
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
}

impl TableLook {
    /// The look of the cell at `row`, `column`, if its style says anything.
    pub fn cell(&self, row: u32, column: u32) -> Option<&CellLook> {
        self.cells.get(&(row, column))
    }
}
