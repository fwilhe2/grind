// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A table's shape on the page — how many columns each has, and how wide the text in each cell
//! is set. Pure arithmetic, so it is tested on the host; `mod.rs` builds the elements.
//!
//! A table in the model is a **run of blocks carrying a `Cell`** (`grind_text::model::Cell`):
//! the body stays a flat sequence and a block says which table, row and column it sits in. This
//! pane used to ignore that and stack a cell's blocks like any other, so a table read as a run
//! of paragraphs with a tall blank gap where its empty cells were. It draws a grid now, with
//! `ui_text_gtk`'s answers to the two questions the model leaves open:
//!
//! - **Columns are equal shares of the measure**, since the model carries no column widths
//!   (`doc/text-core.md`: a table's own style is not read). Equal shares never overflow.
//! - **A merged cell is as wide as the columns it spans**, and as tall as the rows.
//!
//! The width matters beyond drawing. A cell's text is broken into lines at the cell's own width,
//! and a click and a Down-arrow have to ask the core with that same width, or the caret lands
//! where the ink is not — which is why [`measures`] is read by the `Faces` every motion goes
//! through, not only by the renderer.

use std::collections::HashMap;

use grind_text::model::Cell;

/// The space between a cell's rule and its text, and the rule's own width, in CSS pixels —
/// `ui_text_gtk::geom::CELL_PAD` and `RULE`, so a table is one shape in both windows.
pub const CELL_PAD: f64 = 6.0;
pub const RULE: f64 = 1.0;

/// How many columns each table has, by `table:name`: the furthest any cell of it reaches.
pub fn columns<'a>(cells: impl IntoIterator<Item = &'a Cell>) -> HashMap<String, u32> {
    let mut out: HashMap<String, u32> = HashMap::new();
    for cell in cells {
        let reach = cell.column + cell.columns_spanned.max(1);
        let entry = out.entry(cell.table.clone()).or_insert(1);
        *entry = (*entry).max(reach);
    }
    out
}

/// The width a cell's text is set at, in a flow `width` pixels wide: its share of the columns,
/// less the padding either side and the rule.
pub fn measure(cell: &Cell, columns: u32, width: f64) -> f64 {
    let share = width * f64::from(cell.columns_spanned.max(1)) / f64::from(columns.max(1));
    (share - 2.0 * CELL_PAD - RULE).max(1.0)
}

/// The measure of every block that is in a table cell, by block index — what the renderer lays
/// a cell's lines out at, and what [`super::Column`] answers a motion with.
pub fn measures(blocks: &[(usize, Option<&Cell>)], width: f64) -> HashMap<usize, f64> {
    let columns = columns(blocks.iter().filter_map(|(_, cell)| *cell));
    blocks
        .iter()
        .filter_map(|&(index, cell)| {
            let cell = cell?;
            let count = columns.get(&cell.table).copied().unwrap_or(1);
            Some((index, measure(cell, count, width)))
        })
        .collect()
}

/// The CSS grid placement of a cell: `grid-row`/`grid-column` with its spans, 1-based.
pub fn placement(cell: &Cell) -> String {
    format!(
        "grid-row:{} / span {};grid-column:{} / span {}",
        cell.row + 1,
        cell.rows_spanned.max(1),
        cell.column + 1,
        cell.columns_spanned.max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(table: &str, row: u32, column: u32) -> Cell {
        Cell::new(table, row, column)
    }

    #[test]
    fn a_table_is_as_wide_as_its_furthest_cell_reaches() {
        let mut merged = cell("B", 0, 1);
        merged.columns_spanned = 3;
        let cells = [cell("A", 0, 0), cell("A", 1, 2), cell("B", 0, 0), merged];
        let columns = columns(cells.iter());
        assert_eq!(columns["A"], 3);
        assert_eq!(columns["B"], 4, "a span reaches past its own column");
    }

    /// Equal shares, less the padding and the rule — the same arithmetic `ui_text_gtk`'s grid
    /// does, so a cell's lines break in the same places in both windows.
    #[test]
    fn a_cell_is_set_at_its_share_of_the_measure() {
        assert_eq!(measure(&cell("A", 0, 0), 3, 600.0), 200.0 - 12.0 - 1.0);
        let mut wide = cell("A", 0, 0);
        wide.columns_spanned = 2;
        assert_eq!(measure(&wide, 4, 600.0), 300.0 - 13.0);
        assert_eq!(measure(&cell("A", 0, 0), 50, 100.0), 1.0, "never nothing");
    }

    #[test]
    fn only_blocks_in_a_cell_get_a_measure_of_their_own() {
        let (a, b) = (cell("T", 0, 0), cell("T", 0, 1));
        let blocks = [(0, None), (1, Some(&a)), (2, Some(&b)), (3, None)];
        let measures = measures(&blocks, 400.0);
        assert_eq!(measures.len(), 2);
        assert_eq!(measures[&1], 200.0 - 13.0);
        assert!(!measures.contains_key(&0));
    }

    #[test]
    fn a_merged_cell_is_placed_across_what_it_spans() {
        let mut merged = cell("T", 1, 0);
        merged.rows_spanned = 2;
        merged.columns_spanned = 3;
        assert_eq!(
            placement(&merged),
            "grid-row:2 / span 2;grid-column:1 / span 3"
        );
    }
}
