// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The autofilter: `table:database-range` and its `table:filter` (ODF 1.4 Part 3 §9.4).
//!
//! One filter per sheet, over one rectangle, keeping a **set of values** per column — the
//! shape `core/tests/data/samples/table.fods` uses and the shape every autofilter dropdown
//! offers. `table:filter-condition` with `table:operator="="` and a list of
//! `table:filter-set-item` children *is* that set; a condition spelled any other way (`<`,
//! `begins-with`, top-10) is read and dropped rather than half-applied, because a filter
//! that quietly keeps the wrong rows is worse than one that is visibly absent.
//!
//! **Which rows are hidden is derived, never stored.** A filtered document says so twice —
//! the conditions here, and `table:visibility="filter"` on each row they exclude — and two
//! copies of one fact is how they come to disagree. So [`Filter::hides`] answers from the
//! conditions and the cells, the writer paints the attribute from that answer, and the
//! reader ignores the attribute it reads. `filter_matches_libreoffice` in
//! `core/tests/filter.rs` is that decision held to the sample: the rows this computes must
//! be exactly the rows LibreOffice marked.
//!
//! Matching is on the cell's **display text**, the string the dropdown listed, compared
//! exactly. Not a collation: `doc/not-doing.md` gates sorting on a collation decision this
//! project has not made, and set membership is the half of "sort and filter" that does not
//! need one.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{Pos, Sheet};

/// An autofilter over a rectangle of one sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    /// `table:name`. LibreOffice calls an unnamed autofilter `__Anonymous_Sheet_DB__0`, and
    /// the name is kept as it was given: it is an identifier in the file, not a label.
    pub name: String,
    /// `table:target-range-address`, header row included, both ends inclusive.
    pub start: Pos,
    pub end: Pos,
    /// `table:contains-header`. When set, [`Filter::start`]'s row is a heading rather than
    /// data and is never hidden — which is what the sample shows: `Product` is not one of
    /// the values its own filter keeps, and LibreOffice leaves that row visible anyway.
    pub contains_header: bool,
    /// `table:display-filter-buttons` — whether the dropdown buttons are drawn. Carried so a
    /// round trip does not silently turn them off; nothing in this build draws one yet.
    pub buttons: bool,
    /// The values each field keeps, by field number: 0 is [`Filter::start`]'s column, 1 the
    /// one after it (§9.4's `table:field-number`). A field with no entry is unfiltered, and
    /// an entry holding the empty string keeps empty cells.
    pub keep: BTreeMap<u32, BTreeSet<String>>,
}

impl Filter {
    /// A filter over `start..=end` that hides nothing yet.
    pub fn new(name: impl Into<String>, start: Pos, end: Pos) -> Self {
        Self {
            name: name.into(),
            start,
            end,
            contains_header: true,
            buttons: true,
            keep: BTreeMap::new(),
        }
    }

    /// The sheet column a field number names.
    pub fn column(&self, field: u32) -> u32 {
        self.start.col.saturating_add(field)
    }

    /// The first row the filter may hide — past the heading, if there is one.
    pub fn first_data_row(&self) -> u32 {
        match self.contains_header {
            true => self.start.row.saturating_add(1),
            false => self.start.row,
        }
    }

    /// Whether this row is excluded: it is inside the range, and at least one field's value
    /// is not among the ones that field keeps. Several conditions are an **and** (§9.4's
    /// `table:filter-and`), so one failing field is enough.
    pub fn hides(&self, sheet: &Sheet, row: u32, null_date: i64) -> bool {
        if row < self.first_data_row() || row > self.end.row {
            return false;
        }
        self.keep.iter().any(|(field, keep)| {
            let col = self.column(*field);
            col <= self.end.col
                && !keep.contains(&crate::render(sheet, Pos::new(row, col), null_date))
        })
    }

    /// Whether writing to this cell could change which rows are hidden — it sits in a row
    /// the filter judges, in a column it judges on. The diffable writer (R6) asks, because
    /// `table:visibility` lives on the row rather than in the cell it would splice.
    pub fn affects(&self, pos: Pos) -> bool {
        pos.row >= self.first_data_row()
            && pos.row <= self.end.row
            && pos.col >= self.start.col
            && self.keep.contains_key(&(pos.col - self.start.col))
    }

    /// Every row this filter hides, in order — what the writer paints
    /// `table:visibility="filter"` onto and what a shell leaves undrawn.
    pub fn hidden_rows(&self, sheet: &Sheet, null_date: i64) -> Vec<u32> {
        if self.keep.is_empty() {
            return Vec::new();
        }
        (self.first_data_row()..=self.end.row)
            .filter(|row| self.hides(sheet, *row, null_date))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellValue;

    fn sheet() -> Sheet {
        let mut sheet = Sheet::new("Sheet1");
        for (row, product) in ["Product", "Chair", "Desk", "Lamp"].iter().enumerate() {
            sheet.set(
                Pos::new(row as u32, 1),
                CellValue::Text((*product).to_owned()),
            );
        }
        sheet
    }

    fn filter(keep: &[&str]) -> Filter {
        let mut filter = Filter::new("f", Pos::new(0, 1), Pos::new(3, 1));
        filter
            .keep
            .insert(0, keep.iter().map(|s| (*s).to_owned()).collect());
        filter
    }

    /// The rule, in one: a value in the set stays, one that is not goes, and the heading is
    /// never judged even though its own text is not in the set.
    #[test]
    fn a_row_survives_when_every_field_is_one_of_the_kept_values() {
        let (sheet, filter) = (sheet(), filter(&["Chair", "Desk"]));
        assert_eq!(filter.hidden_rows(&sheet, 0), vec![3]);
        assert!(!filter.hides(&sheet, 0, 0), "the heading row");
        // Past the range: outside a filter's business entirely.
        assert!(!filter.hides(&sheet, 9, 0));
    }

    /// A filter with no conditions is a range with dropdown buttons and nothing chosen —
    /// `kb/filter.fods` is exactly that, and it must hide nothing at all.
    #[test]
    fn a_filter_with_no_conditions_hides_nothing() {
        let mut filter = filter(&[]);
        filter.keep.clear();
        assert!(filter.hidden_rows(&sheet(), 0).is_empty());
    }

    /// An empty cell is a value like any other, spelled `""` — which is how LibreOffice
    /// writes "(empty)" in the dropdown, and why the sample's totals row survives its own
    /// filter.
    #[test]
    fn the_empty_string_keeps_empty_cells() {
        let mut sheet = sheet();
        sheet.set(Pos::new(2, 1), CellValue::Empty);
        assert_eq!(filter(&["Chair", ""]).hidden_rows(&sheet, 0), vec![3]);
        assert_eq!(filter(&["Chair"]).hidden_rows(&sheet, 0), vec![2, 3]);
    }

    /// Two fields are an `and`: a row has to satisfy both to stay.
    #[test]
    fn every_condition_has_to_hold() {
        let mut sheet = sheet();
        for row in 1..4u32 {
            sheet.set(Pos::new(row, 2), CellValue::Text("Office".to_owned()));
        }
        sheet.set(Pos::new(2, 2), CellValue::Text("Home".to_owned()));
        let mut filter = filter(&["Chair", "Desk", "Lamp"]);
        filter.end.col = 2;
        filter.keep.insert(1, ["Office".to_owned()].into());
        assert_eq!(filter.hidden_rows(&sheet, 0), vec![2]);
    }
}
