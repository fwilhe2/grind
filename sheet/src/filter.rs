// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The autofilter: `table:database-range` and its `table:filter` (ODF 1.4 Part 3 §9.4).
//!
//! One filter per sheet, over one rectangle, keeping a set of values per column — the shape
//! `sheet/tests/data/samples/table.fods` uses and every dropdown offers: `table:filter-condition`
//! with `table:operator="="` and `table:filter-set-item` children. Any other condition (`<`,
//! `begins-with`, top-10) is dropped on reading rather than half-applied.
//!
//! Which rows are hidden is derived, never stored. A file says it twice — the conditions, and
//! `table:visibility="filter"` on each excluded row — so [`Filter::hides`] answers from the
//! conditions, the writer paints the attribute from that, and the reader ignores it.
//! `filter_matches_libreoffice` (`sheet/tests/filter.rs`) holds the computed rows to the ones
//! LibreOffice marked. Matching is exact, on display text: set membership needs no collation.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{Pos, Sheet};

/// The distinct values a field's column holds, in the order a dropdown lists them: each cell's
/// display text (what [`Filter::keep`] matches), including the ones currently hidden, the heading
/// excluded. The empty cell first, then numbers (dates and times among them) by value, then
/// everything else in code-point order — numbers by their text put `2,250.00 €` before `220.00 €`,
/// and text by anything but code points would need a collation. At most `limit` values.
pub fn offered(cells: &crate::Viewport, filter: &Filter, field: u32, limit: usize) -> Vec<String> {
    let col = filter.column(field);
    let mut seen = BTreeSet::new();
    let mut values: Vec<(u8, f64, String)> = Vec::new();
    for row in filter.first_data_row()..=filter.end.row {
        let Some(text) = cells.text(row, col) else {
            continue;
        };
        if !seen.insert(text.to_owned()) {
            continue;
        }
        let (class, number) = match cells.get(row, col) {
            _ if text.is_empty() => (0, 0.0),
            Some(crate::CellValue::Number(n)) => (1, *n),
            _ => (2, 0.0),
        };
        values.push((class, number, text.to_owned()));
    }
    values.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    values
        .into_iter()
        .map(|(_, _, text)| text)
        .take(limit)
        .collect()
}

/// An autofilter over a rectangle of one sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    /// `table:name`, kept as given; LibreOffice calls an unnamed autofilter
    /// `__Anonymous_Sheet_DB__0`.
    pub name: String,
    /// `table:target-range-address`, header row included, both ends inclusive.
    pub start: Pos,
    pub end: Pos,
    /// `table:contains-header`: [`Filter::start`]'s row is a heading and never hidden, even though
    /// its text is not among the values kept.
    pub contains_header: bool,
    /// `table:display-filter-buttons` — whether the dropdown buttons are drawn.
    pub buttons: bool,
    /// The values each field keeps, by field number from [`Filter::start`]'s column (§9.4's
    /// `table:field-number`). A field with no entry is unfiltered; the empty string keeps empty
    /// cells.
    pub keep: BTreeMap<u32, BTreeSet<String>>,
}

/// LibreOffice's name for an unnamed autofilter, which `grind sheet filter` writes too.
pub const ANONYMOUS: &str = "__Anonymous_Sheet_DB__0";

impl Filter {
    /// The filter Data ▸ Filter turns on over `start..=end`: the selection, or from a single cell
    /// the used table from there to the sheet's last used row and column (`used` is
    /// [`crate::App::used_extent`]). `Err` with the sentence to say when there is no heading with
    /// a row under it.
    pub fn over_selection(
        start: Pos,
        end: Pos,
        used: (u32, u32),
    ) -> std::result::Result<Self, &'static str> {
        let end = match start == end {
            true => Pos::new(used.0.saturating_sub(1), used.1.saturating_sub(1)),
            false => end,
        };
        match end.row > start.row && end.col >= start.col {
            true => Ok(Self::new(ANONYMOUS, start, end)),
            false => Err("Select the rows to filter, including their headings."),
        }
    }

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

    /// Whether this row is excluded: inside the range, with at least one field's value not among
    /// the ones it keeps (conditions are an and, §9.4's `table:filter-and`). Values are matched as
    /// shown in `locale`, so a German document's `1234,5` matches what a German LibreOffice saved.
    pub fn hides(
        &self,
        sheet: &Sheet,
        row: u32,
        null_date: i64,
        locale: Option<&crate::locale::Locale>,
    ) -> bool {
        if row < self.first_data_row() || row > self.end.row {
            return false;
        }
        self.keep.iter().any(|(field, keep)| {
            let col = self.column(*field);
            col <= self.end.col
                && !keep.contains(&crate::render_in(
                    sheet,
                    Pos::new(row, col),
                    null_date,
                    locale,
                ))
        })
    }

    /// Whether writing to this cell could change which rows are hidden. The splicing writer asks,
    /// because `table:visibility` is on the row.
    pub fn affects(&self, pos: Pos) -> bool {
        pos.row >= self.first_data_row()
            && pos.row <= self.end.row
            && pos.col >= self.start.col
            && self.keep.contains_key(&(pos.col - self.start.col))
    }

    /// Every row this filter hides, in order.
    pub fn hidden_rows(
        &self,
        sheet: &Sheet,
        null_date: i64,
        locale: Option<&crate::locale::Locale>,
    ) -> Vec<u32> {
        if self.keep.is_empty() {
            return Vec::new();
        }
        (self.first_data_row()..=self.end.row)
            .filter(|row| self.hides(sheet, *row, null_date, locale))
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
        assert_eq!(filter.hidden_rows(&sheet, 0, None), vec![3]);
        assert!(!filter.hides(&sheet, 0, 0, None), "the heading row");
        // Past the range: outside a filter's business entirely.
        assert!(!filter.hides(&sheet, 9, 0, None));
    }

    /// `kb/filter.fods` has dropdowns and nothing chosen, and must hide nothing.
    #[test]
    fn a_filter_with_no_conditions_hides_nothing() {
        let mut filter = filter(&[]);
        filter.keep.clear();
        assert!(filter.hidden_rows(&sheet(), 0, None).is_empty());
    }

    /// An empty cell is the value `""`, LibreOffice's "(empty)", so the sample's totals row
    /// survives its own filter.
    #[test]
    fn the_empty_string_keeps_empty_cells() {
        let mut sheet = sheet();
        sheet.set(Pos::new(2, 1), CellValue::Empty);
        assert_eq!(filter(&["Chair", ""]).hidden_rows(&sheet, 0, None), vec![3]);
        assert_eq!(filter(&["Chair"]).hidden_rows(&sheet, 0, None), vec![2, 3]);
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
        assert_eq!(filter.hidden_rows(&sheet, 0, None), vec![2]);
    }

    #[test]
    fn a_filter_over_one_cell_takes_the_used_table_and_over_one_row_nothing() {
        let (a1, c9) = (Pos::new(0, 0), Pos::new(8, 2));
        let whole = Filter::over_selection(a1, a1, (9, 3)).unwrap();
        assert_eq!((whole.start, whole.end), (a1, c9));
        assert_eq!(whole.name, ANONYMOUS);
        let picked = Filter::over_selection(a1, Pos::new(3, 1), (9, 3)).unwrap();
        assert_eq!(picked.end, Pos::new(3, 1), "the selection as it was made");
        assert!(Filter::over_selection(a1, Pos::new(0, 2), (9, 3)).is_err());
        assert!(
            Filter::over_selection(a1, a1, (1, 1)).is_err(),
            "a lone heading"
        );
    }
}
