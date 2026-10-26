// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document model: values, addresses, sheets, and the document itself. Positions are 0-based;
//! `a1.rs` is the only conversion.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::grid::Column;
use crate::numfmt::Format;
use crate::style::CellStyle;

/// A cell's value. A formula's source is a side table on [`Sheet`], and an error is stored as its
/// name in text ([`crate::formula::eval::to_cell`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum CellValue {
    #[default]
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
}

impl CellValue {
    pub fn is_empty(&self) -> bool {
        matches!(self, CellValue::Empty)
    }
}

impl From<f64> for CellValue {
    fn from(n: f64) -> Self {
        CellValue::Number(n)
    }
}

impl From<&str> for CellValue {
    fn from(s: &str) -> Self {
        CellValue::Text(s.to_owned())
    }
}

impl From<bool> for CellValue {
    fn from(b: bool) -> Self {
        CellValue::Bool(b)
    }
}

/// The Number subtype a cell was written as — §4.3.2 Time and §4.3.3 Date — so the writer spells a
/// date back as a date rather than a bare serial. The value is an ordinary number either way, and
/// the display is [`Sheet::format`]'s (§5.2); a date with no format gets the ISO default on
/// writing, so LibreOffice does not invent a locale one.
///
/// ponytail: a date a formula computes has no kind, because the evaluator's `Value` has no Date
/// subtype, so `=DATE(2026;8;16)` shows its serial until formatted. Part 4 §4.3.3 puts the subtype
/// in the value model, where the fix belongs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumberKind {
    Date,
    Time,
}

/// A cell address within one sheet, 0-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Pos {
    pub row: u32,
    pub col: u32,
}

impl Pos {
    pub fn new(row: u32, col: u32) -> Self {
        Self { row, col }
    }
}

/// A `BTreeMap` keyed by [`Pos`], written as a list of pairs, since JSON (the session file) has
/// only string keys.
mod pairs {
    use super::Pos;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer, V: Serialize>(
        map: &BTreeMap<Pos, V>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, V: Deserialize<'de>>(
        d: D,
    ) -> Result<BTreeMap<Pos, V>, D::Error> {
        Ok(Vec::<(Pos, V)>::deserialize(d)?.into_iter().collect())
    }
}

/// Where a sheet came from and what has been done to it since, for a save that writes it back into
/// the table it was read from. Kept on the sheet, because deleting and undeleting a sheet moves it
/// and its touched rows must move with it. Over-reporting is safe (a touched cell is rewritten
/// from the model); under-reporting is not. Every `Provenance` is equal to every other, so sheet
/// equality is about content alone.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// Which `table:table` of the file the sheet was read from.
    pub table: Option<usize>,
    /// Cells whose value, formula, format or style has been written since.
    pub cells: BTreeSet<Pos>,
    /// Of those, the ones whose format or style was written, whose `table:style-name` a save
    /// replaces.
    pub looks: BTreeSet<Pos>,
    /// The ones whose value or formula was written. A cell only restyled keeps the file's own
    /// element (a hyperlink, rich text, an annotation) with a new style name.
    pub values: BTreeSet<Pos>,
    /// Rows whose height or hidden flag has changed since.
    pub rows: BTreeSet<u32>,
    /// Whether a column's width or hidden flag has changed since.
    pub columns: bool,
    /// Whether a chart has been added, removed or changed since.
    pub charts: bool,
    /// Whether the filter has changed since — which moves `table:visibility` on any row.
    pub filter: bool,
    /// Rows a merge was added to or taken from, written again whole.
    #[serde(default)]
    pub merges: BTreeSet<u32>,
    /// Whether a checkbox was added, changed or removed, which rewrites the sheet's
    /// `office:forms`.
    #[serde(default)]
    pub checkboxes: bool,
    /// Whether the conditional-format rules changed, which makes LibreOffice's
    /// `calcext:conditional-formats` copy stale.
    #[serde(default)]
    pub rules: bool,
}

impl PartialEq for Provenance {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// One sheet: a sparse set of columns, so `used_cols` is the real extent rather than a high-water
/// mark. Serialisable because [`crate::Action::InsertSheet`] carries a whole one for undo, and the
/// undo stack can be saved (`--session`).
///
/// ponytail: that puts a deleted sheet's every cell into the session file; keep removed sheets in
/// a side arena if that ever matters.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub name: String,
    cols: Vec<Column>,
    /// Formula source text, kept verbatim beside the cached value, which is an ordinary
    /// [`CellValue`].
    #[serde(with = "pairs")]
    formulas: BTreeMap<Pos, String>,
    /// Which cells hold a date or a time rather than a plain number ([`NumberKind`]).
    #[serde(with = "pairs")]
    kinds: BTreeMap<Pos, NumberKind>,
    /// How each cell is displayed (`doc/ods-format.md` §5.2).
    ///
    /// ponytail: one [`Format`] clone per formatted cell rather than an index into a pool; pooling
    /// happens on write (§5.3). Intern here when a profile blames the clones.
    #[serde(with = "pairs")]
    formats: BTreeMap<Pos, Format>,
    /// How each cell looks (§5.1), with `formats`' ponytail.
    #[serde(with = "pairs")]
    styles: BTreeMap<Pos, CellStyle>,
    /// `style:column-width` per column as the document spelled it (`"2.258cm"`), parsed only by
    /// [`crate::style::length_mm`]. Sparse, and bounded on reading by `MAX_TRACK_RUN`: a long
    /// repeated run is the sheet's background width, not layout.
    #[serde(default)]
    col_widths: BTreeMap<u32, String>,
    /// `style:row-height` per row — the twin of `col_widths`, same rules.
    #[serde(default)]
    row_heights: BTreeMap<u32, String>,
    /// Columns hidden by hand (`table:visibility="collapse"`, §5.4); a column has no filter.
    #[serde(default)]
    hidden_cols: BTreeSet<u32>,
    /// Rows hidden by hand — kept apart from what the filter hides ([`Sheet::hidden_rows`]), so
    /// each reason stays known.
    #[serde(default)]
    manually_hidden_rows: BTreeSet<u32>,
    /// The sheet's autofilter (`table:database-range`, §9.4), one per sheet. The rows it hides are
    /// derived, not stored ([`crate::filter`]).
    #[serde(default)]
    filter: Option<crate::filter::Filter>,
    /// The sheet's charts (`table:shapes`, rng:15678) in document order (`doc/chart-format.md`).
    #[serde(default)]
    charts: Vec<crate::chart::Chart>,
    /// Merged ranges — `table:number-columns-spanned`/`table:number-rows-spanned` on the top-left
    /// cell (rng:16102), keyed by that cell ([`Span`]). A covered cell keeps its own value and
    /// style, as LibreOffice keeps them, and is simply not drawn. No two merges overlap:
    /// [`Sheet::set_merge`] is only told about one that does not.
    #[serde(default)]
    merges: Merges,
    /// Checkboxes, keyed by the cell each is drawn in (`doc/ods-format.md` §3.5) — the one form
    /// control this model carries ([`Checkbox`]).
    #[serde(default, with = "pairs")]
    checkboxes: BTreeMap<Pos, Checkbox>,
    /// Conditional-format rules in priority order: the first that holds at a cell is drawn
    /// (`doc/conditional-format.md`, [`crate::rule`]).
    #[serde(default)]
    rules: Vec<crate::rule::Rule>,
    /// Which `table:table` of the file this sheet was read from, so a regenerating save writes
    /// into that element and keeps what nobody touched (`odf::write`). Bookkeeping: never written,
    /// never projected, and ignored by equality ([`Provenance`]).
    #[serde(default)]
    pub(crate) origin: Provenance,
}

impl Sheet {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            cols: Vec::new(),
            formulas: BTreeMap::new(),
            kinds: BTreeMap::new(),
            formats: BTreeMap::new(),
            styles: BTreeMap::new(),
            col_widths: BTreeMap::new(),
            row_heights: BTreeMap::new(),
            hidden_cols: BTreeSet::new(),
            manually_hidden_rows: BTreeSet::new(),
            filter: None,
            charts: Vec::new(),
            merges: Merges::default(),
            checkboxes: BTreeMap::new(),
            rules: Vec::new(),
            origin: Provenance::default(),
        }
    }

    /// The row a point `offset_mm` below the top of `row` falls in — where a control whose offset
    /// carries it past its anchor cell is drawn (`doc/ods-format.md` §3.5). A row with no height
    /// is [`DEFAULT_ROW_MM`] tall.
    pub fn row_holding(&self, mut row: u32, mut offset_mm: f64) -> u32 {
        let height = |row: u32| {
            self.row_height(row)
                .and_then(grind_core::style::length_mm)
                .filter(|mm| *mm > 0.0)
                .unwrap_or(DEFAULT_ROW_MM)
        };
        for _ in 0..1_000 {
            let h = height(row);
            if offset_mm < h || row + 1 >= crate::MAX_ROWS {
                break;
            }
            offset_mm -= h;
            row += 1;
        }
        row
    }

    /// The conditional-format rules, first-holding-wins order.
    pub fn rules(&self) -> &[crate::rule::Rule] {
        &self.rules
    }

    /// Replace the whole list, since reordering one rule changes what the others mean
    /// ([`crate::Action::SetRules`]).
    pub fn set_rules(&mut self, rules: Vec<crate::rule::Rule>) {
        self.rules = rules;
    }

    /// Every checkbox, by the cell it is drawn in, in address order.
    pub fn checkboxes(&self) -> impl Iterator<Item = (Pos, &Checkbox)> {
        self.checkboxes.iter().map(|(pos, c)| (*pos, c))
    }

    /// The checkbox drawn in `pos`, if there is one.
    pub fn checkbox(&self, pos: Pos) -> Option<&Checkbox> {
        self.checkboxes.get(&pos)
    }

    /// Put a checkbox in `pos`, replacing whatever one was there, or (with `None`) take it away.
    pub fn set_checkbox(&mut self, pos: Pos, checkbox: Option<Checkbox>) {
        match checkbox {
            Some(c) => self.checkboxes.insert(pos, c),
            None => self.checkboxes.remove(&pos),
        };
    }

    /// Every merged range, in address order of its top-left cell.
    pub fn merges(&self) -> impl Iterator<Item = (Pos, Span)> + '_ {
        self.merges
            .by_anchor
            .iter()
            .map(|(pos, span)| (*pos, *span))
    }

    /// The merge `pos` is part of, as its top-left or a covered cell, with that merge's top-left
    /// cell. Only anchors in the rows the tallest merge could reach down from are looked at.
    pub fn merge_at(&self, pos: Pos) -> Option<(Pos, Span)> {
        self.merges
            .reaching_down_to(pos.row, pos)
            .rev()
            .find(|(anchor, span)| span.contains(**anchor, pos))
            .map(|(anchor, span)| (*anchor, *span))
    }

    /// Whether `pos` is hidden under a merge: inside one, and not its top-left cell.
    pub fn covered(&self, pos: Pos) -> bool {
        self.merge_at(pos).is_some_and(|(anchor, _)| anchor != pos)
    }

    /// Merge the range starting at `anchor`, or with `None` remove the merge there. A one-cell
    /// span is stored as none. The caller has already removed any merge the new one overlaps
    /// ([`Sheet::merges_within`]).
    pub fn set_merge(&mut self, anchor: Pos, span: Option<Span>) {
        match span.filter(|span| !span.is_single()) {
            Some(span) => {
                self.merges.tallest = self.merges.tallest.max(span.rows);
                self.merges.by_anchor.insert(anchor, span)
            }
            None => self.merges.by_anchor.remove(&anchor),
        };
    }

    /// Every merge that overlaps the rectangle `start..=end`, by its top-left cell.
    pub fn merges_within(&self, start: Pos, end: Pos) -> Vec<(Pos, Span)> {
        self.merges
            .reaching_down_to(start.row, Pos::new(end.row, u32::MAX))
            .map(|(anchor, span)| (*anchor, *span))
            .filter(|(anchor, span)| {
                let last = span.end(*anchor);
                anchor.row <= end.row
                    && last.row >= start.row
                    && anchor.col <= end.col
                    && last.col >= start.col
            })
            .collect()
    }

    /// Every chart on this sheet, in document order.
    pub fn charts(&self) -> &[crate::chart::Chart] {
        &self.charts
    }

    /// Remove the chart at `index`, or `None` if there is none there.
    pub(crate) fn remove_chart(&mut self, index: usize) -> Option<crate::chart::Chart> {
        (index < self.charts.len()).then(|| self.charts.remove(index))
    }

    /// Insert `chart` back at `index`, undoing [`Self::remove_chart`].
    pub(crate) fn insert_chart(&mut self, index: usize, chart: crate::chart::Chart) {
        let index = index.min(self.charts.len());
        self.charts.insert(index, chart);
    }

    /// The chart at `index`, for [`Action::ReshapeChart`](crate::action::Action::ReshapeChart) to
    /// change its position and size in place.
    pub(crate) fn chart_mut(&mut self, index: usize) -> Option<&mut crate::chart::Chart> {
        self.charts.get_mut(index)
    }

    /// The sheet's autofilter, or `None` when it has none (§9.4).
    pub fn filter(&self) -> Option<&crate::filter::Filter> {
        self.filter.as_ref()
    }

    /// Set or, with `None`, remove it — replaced rather than merged.
    pub fn set_filter(&mut self, filter: Option<crate::filter::Filter>) {
        self.filter = filter;
    }

    /// Every row the filter hides, in order; empty without a filter. Matching is on what a cell
    /// displays, which needs the epoch and the document's locale.
    pub fn hidden_rows(
        &self,
        null_date: i64,
        locale: Option<&grind_core::locale::Locale>,
    ) -> Vec<u32> {
        self.filter
            .as_ref()
            .map(|f| f.hidden_rows(self, null_date, locale))
            .unwrap_or_default()
    }

    /// Whether one row is hidden by the filter, without building the whole list.
    pub fn row_hidden(
        &self,
        row: u32,
        null_date: i64,
        locale: Option<&grind_core::locale::Locale>,
    ) -> bool {
        self.filter
            .as_ref()
            .is_some_and(|f| f.hides(self, row, null_date, locale))
    }

    /// The column's width, or `None` when it is drawn at the shell's default.
    pub fn col_width(&self, col: u32) -> Option<&str> {
        self.col_widths.get(&col).map(String::as_str)
    }

    pub fn row_height(&self, row: u32) -> Option<&str> {
        self.row_heights.get(&row).map(String::as_str)
    }

    /// Set or, with `None`, clear a column's width; an empty string clears too.
    pub fn set_col_width(&mut self, col: u32, width: Option<String>) {
        match width {
            Some(w) => self.col_widths.insert(col, w),
            None => self.col_widths.remove(&col),
        };
    }

    pub fn set_row_height(&mut self, row: u32, height: Option<String>) {
        match height {
            Some(h) => self.row_heights.insert(row, h),
            None => self.row_heights.remove(&row),
        };
    }

    /// Every sized column, in order — what a renderer's prefix sums and the writer's pool
    /// both walk.
    pub fn col_widths(&self) -> impl Iterator<Item = (u32, &str)> {
        self.col_widths.iter().map(|(c, w)| (*c, w.as_str()))
    }

    pub fn row_heights(&self) -> impl Iterator<Item = (u32, &str)> {
        self.row_heights.iter().map(|(r, h)| (*r, h.as_str()))
    }

    /// Whether this column is hidden by hand.
    pub fn col_hidden(&self, col: u32) -> bool {
        self.hidden_cols.contains(&col)
    }

    /// Set or clear a column's manual hidden state.
    pub fn set_col_hidden(&mut self, col: u32, hidden: bool) {
        match hidden {
            true => self.hidden_cols.insert(col),
            false => self.hidden_cols.remove(&col),
        };
    }

    /// Every column hidden by hand, in order.
    pub fn hidden_cols(&self) -> impl Iterator<Item = u32> + '_ {
        self.hidden_cols.iter().copied()
    }

    /// Whether this row is hidden by hand, as distinct from [`Sheet::row_hidden`]'s filter answer.
    pub fn row_manually_hidden(&self, row: u32) -> bool {
        self.manually_hidden_rows.contains(&row)
    }

    /// Set or clear a row's manual hidden state.
    pub fn set_row_hidden(&mut self, row: u32, hidden: bool) {
        match hidden {
            true => self.manually_hidden_rows.insert(row),
            false => self.manually_hidden_rows.remove(&row),
        };
    }

    /// Every row hidden by hand, in order.
    pub fn manually_hidden_rows(&self) -> impl Iterator<Item = u32> + '_ {
        self.manually_hidden_rows.iter().copied()
    }

    pub fn formula(&self, pos: Pos) -> Option<&str> {
        self.formulas.get(&pos).map(String::as_str)
    }

    pub fn set_formula(&mut self, pos: Pos, formula: String) {
        self.formulas.insert(pos, formula);
    }

    /// Leave the cached value, drop the formula — the cell becomes an ordinary value cell.
    pub fn clear_formula(&mut self, pos: Pos) {
        self.formulas.remove(&pos);
    }

    pub fn formula_count(&self) -> usize {
        self.formulas.len()
    }

    /// Every formula in the sheet, in address order — what a full recalculation walks.
    pub fn formulas(&self) -> impl Iterator<Item = (Pos, &str)> {
        self.formulas.iter().map(|(pos, f)| (*pos, f.as_str()))
    }

    pub fn kind(&self, pos: Pos) -> Option<NumberKind> {
        self.kinds.get(&pos).copied()
    }

    pub fn set_kind(&mut self, pos: Pos, kind: NumberKind) {
        self.kinds.insert(pos, kind);
    }

    /// Every date or time cell, in address order.
    pub fn kinds(&self) -> impl Iterator<Item = (Pos, NumberKind)> {
        self.kinds.iter().map(|(pos, kind)| (*pos, *kind))
    }

    /// The cell's number format, or `None` when it is displayed as its plain value.
    pub fn format(&self, pos: Pos) -> Option<&Format> {
        self.formats.get(&pos)
    }

    pub fn set_format(&mut self, pos: Pos, format: Format) {
        self.formats.insert(pos, format);
    }

    /// Back to the plain spelling of the value — ODF's "General", which is no data style at all.
    pub fn clear_format(&mut self, pos: Pos) {
        self.formats.remove(&pos);
    }

    /// The cell's styling, or `None` when it is drawn plainly.
    pub fn style(&self, pos: Pos) -> Option<&CellStyle> {
        self.styles.get(&pos)
    }

    /// A style that sets nothing is stored as none, so "plain" has one spelling.
    pub fn set_style(&mut self, pos: Pos, style: CellStyle) {
        match style.is_plain() {
            true => self.styles.remove(&pos),
            false => self.styles.insert(pos, style),
        };
    }

    pub fn clear_style(&mut self, pos: Pos) {
        self.styles.remove(&pos);
    }

    /// Every styled cell, in address order.
    pub fn styles(&self) -> impl Iterator<Item = (Pos, &CellStyle)> {
        self.styles.iter().map(|(pos, s)| (*pos, s))
    }

    /// Every formatted cell, in address order — what the writer pools into styles.
    pub fn formats(&self) -> impl Iterator<Item = (Pos, &Format)> {
        self.formats.iter().map(|(pos, f)| (*pos, f))
    }

    pub fn get(&self, pos: Pos) -> CellValue {
        self.cols
            .get(pos.col as usize)
            .map_or(CellValue::Empty, |c| c.get(pos.row))
    }

    pub fn set(&mut self, pos: Pos, value: CellValue) {
        let col = pos.col as usize;
        // Clearing a cell in a column that does not exist writes nothing.
        if value.is_empty() && col >= self.cols.len() {
            return;
        }
        if col >= self.cols.len() {
            self.cols.resize_with(col + 1, Column::default);
        }
        self.cols[col].set(pos.row, value);
        while self.cols.last().is_some_and(Column::is_empty) {
            self.cols.pop();
        }
    }

    /// One past the last row holding anything — a value, a formula, a date kind, a number format
    /// or a cell style. `odf::write` emits exactly this rectangle, so a narrower answer would drop
    /// a style or a formula past the last value on save (a styled blank, or a hand-written `cell
    /// B5 "=SUM(…)"` with no cached value). Each side table is ordered by row, so its last key
    /// gives its last row.
    pub fn used_rows(&self) -> u32 {
        let values = self.cols.iter().map(Column::len).max().unwrap_or(0);
        let extra = self
            .side_tables()
            .filter_map(|last| last.map(|pos| pos.row + 1))
            .max()
            .unwrap_or(0);
        values.max(extra)
    }

    /// One past the last column holding anything — [`Sheet::used_rows`]'s twin, same reason.
    pub fn used_cols(&self) -> u32 {
        let extra = [
            self.formulas.keys().map(|pos| pos.col).max(),
            self.kinds.keys().map(|pos| pos.col).max(),
            self.formats.keys().map(|pos| pos.col).max(),
            self.styles.keys().map(|pos| pos.col).max(),
            self.merges()
                .map(|(anchor, span)| span.end(anchor).col)
                .max(),
        ]
        .into_iter()
        .flatten()
        .max()
        .map_or(0, |col| col + 1);
        (self.cols.len() as u32).max(extra)
    }

    /// One past the last row and column holding a value or a formula — what `Engine::area` bounds
    /// an open reference (`A:A`) by. Narrower than [`Sheet::used_rows`] because a styled blank
    /// contributes nothing to a function: `fdo67682-2.ods` has 94,868 of them past its data, and
    /// counting them took its recalculation from 1.8 to 8.3 seconds.
    pub fn cell_extent(&self) -> (u32, u32) {
        let values = self.cols.iter().map(Column::len).max().unwrap_or(0);
        let rows = self
            .formulas
            .keys()
            .next_back()
            .map_or(0, |pos| pos.row + 1)
            .max(values);
        let cols = self
            .formulas
            .keys()
            .map(|pos| pos.col + 1)
            .max()
            .unwrap_or(0)
            .max(self.cols.len() as u32);
        (rows, cols)
    }

    /// Every cell holding typed text — a text value and no formula — in reading order: what
    /// spelling checks (`doc/spelling.md`). One step per occupied cell.
    pub fn text_cells(&self) -> Vec<(Pos, String)> {
        let mut out = Vec::new();
        for (col, column) in self.cols.iter().enumerate() {
            for rows in column.occupied() {
                for row in rows {
                    let pos = Pos::new(row, col as u32);
                    if let CellValue::Text(text) = column.get(row)
                        && !self.formulas.contains_key(&pos)
                    {
                        out.push((pos, text));
                    }
                }
            }
        }
        out.sort_by_key(|(pos, _)| (pos.row, pos.col));
        out
    }

    /// Every row holding anything ([`Sheet::used_rows`]'s rule) as sorted, disjoint ranges. The
    /// writer asks this rather than every cell: a sheet with cells at `A1` and `XFD1048576` has a
    /// used rectangle of seventeen billion cells. One step per value block and per side-table
    /// entry.
    pub fn rows_carrying(&self) -> Vec<std::ops::Range<u32>> {
        let mut ranges: Vec<std::ops::Range<u32>> = self
            .cols
            .iter()
            .flat_map(Column::occupied)
            .chain(
                self.formulas
                    .keys()
                    .chain(self.kinds.keys())
                    .chain(self.formats.keys())
                    .chain(self.styles.keys())
                    .map(|pos| pos.row..pos.row + 1),
            )
            // A checkbox's cell is written to hold its control.
            .chain(self.checkboxes.keys().map(|pos| pos.row..pos.row + 1))
            // A merge's every row is written, covered cells and all.
            .chain(
                self.merges()
                    .map(|(anchor, span)| anchor.row..span.end(anchor).row + 1),
            )
            // A blank a rule covers is written to carry it, inside the rectangle the rest of the
            // sheet spans ([`Sheet::ruled`]).
            .chain(self.ruled_rows())
            .collect();
        ranges.sort_unstable_by_key(|range| range.start);
        let mut merged: Vec<std::ops::Range<u32>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        merged
    }

    /// Whether a rule covers `pos` and it lies inside the rectangle the sheet's content spans —
    /// the cells a save spells to carry a rule (`doc/conditional-format.md`).
    ///
    /// ponytail: a rule drawn on blank rows below everything else does not survive a save, which
    /// keeps `A1:A1048576` from becoming a million rows. The upgrade is a rule on a column's
    /// `table:default-cell-style-name`.
    pub fn ruled(&self, pos: Pos) -> bool {
        !self.rules.is_empty()
            && pos.row < self.used_rows()
            && pos.col < self.used_cols()
            && self.rules.iter().any(|rule| rule.covers(pos))
    }

    /// The rows [`Sheet::ruled`] can be true on, as ranges.
    fn ruled_rows(&self) -> Vec<std::ops::Range<u32>> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        let (rows, cols) = (self.used_rows(), self.used_cols());
        self.rules
            .iter()
            .flat_map(|rule| &rule.ranges)
            .filter(|(start, _)| start.col < cols && start.row < rows)
            .map(|(start, end)| start.row..(end.row + 1).min(rows))
            .collect()
    }

    /// The last `Pos` of each cell-keyed side table, which is its last row.
    fn side_tables(&self) -> impl Iterator<Item = Option<Pos>> {
        [
            self.formulas.keys().next_back().copied(),
            self.kinds.keys().next_back().copied(),
            self.formats.keys().next_back().copied(),
            self.styles.keys().next_back().copied(),
            self.merges()
                .map(|(anchor, span)| span.end(anchor))
                .max_by_key(|pos| pos.row),
            self.checkboxes.keys().next_back().copied(),
        ]
        .into_iter()
    }
}

/// A row's height when it states none: `0.452cm`, what LibreOffice writes for its default 10pt
/// font.
pub const DEFAULT_ROW_MM: f64 = 4.52;

/// A checkbox drawn in a cell: ODF's `form:checkbox`, cut down to a checklist (`doc/ods-format.md`
/// §3.5). Linked, its state is the linked cell's value — ticking writes `TRUE` — and `checked` is
/// its own state only when it is linked to nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkbox {
    /// `form:linked-cell` — the cell whose value it shows and sets.
    pub link: Option<Link>,
    /// `form:current-state`, for a checkbox with no linked cell.
    pub checked: bool,
    /// `form:name`.
    pub name: Option<String>,
    /// `form:label` — the text drawn beside the box.
    pub label: Option<String>,
}

/// A cell a checkbox is linked to: on its own sheet, or on another one named.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// The sheet, when it is not the checkbox's own.
    pub sheet: Option<String>,
    pub pos: Pos,
}

impl Link {
    /// An ODF cell address — `Checkliste.E5` — with the checkbox's own sheet folded away.
    pub fn parse(address: &str, own_sheet: &str) -> Option<Self> {
        let reference = crate::a1::parse_bracketed(&format!("[{address}]")).ok()?;
        if reference.end.is_some() {
            return None;
        }
        let cell = reference.start;
        let pos = Pos::new(cell.row?.index, cell.col?.index);
        let sheet = cell.sheet.filter(|name| name != own_sheet);
        Some(Self { sheet, pos })
    }

    /// The address as ODF writes it, always with its sheet: `Checkliste.E5`.
    pub fn address(&self, own_sheet: &str) -> String {
        let sheet = self.sheet.as_deref().unwrap_or(own_sheet);
        let mut cell = crate::a1::reference(Some(sheet), self.pos, self.pos).start;
        cell.sheet_absolute = false;
        cell.to_string()
    }
}

/// A sheet's merges by top-left cell, with the most rows any of them has spanned — an upper
/// bound, never lowered on removal — so a lookup reads only anchors close enough above to reach.
#[derive(Clone, Debug, Default)]
struct Merges {
    by_anchor: BTreeMap<Pos, Span>,
    tallest: u32,
}

impl Merges {
    /// The anchors from the first row a merge reaching `row` could start in, up to `last`.
    fn reaching_down_to(
        &self,
        row: u32,
        last: Pos,
    ) -> std::collections::btree_map::Range<'_, Pos, Span> {
        let first = row.saturating_sub(self.tallest.saturating_sub(1));
        self.by_anchor.range(Pos::new(first, 0)..=last)
    }
}

impl PartialEq for Merges {
    fn eq(&self, other: &Self) -> bool {
        self.by_anchor == other.by_anchor
    }
}

impl Serialize for Merges {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        pairs::serialize(&self.by_anchor, s)
    }
}

impl<'de> Deserialize<'de> for Merges {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let by_anchor: BTreeMap<Pos, Span> = pairs::deserialize(d)?;
        let tallest = by_anchor.values().map(|span| span.rows).max().unwrap_or(0);
        Ok(Merges { by_anchor, tallest })
    }
}

/// How far a merged range reaches from its top-left cell (rng:16102), each at least one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub cols: u32,
    pub rows: u32,
}

impl Span {
    /// The span from `start` to `end`, both inclusive, in either order.
    pub fn between(start: Pos, end: Pos) -> (Pos, Self) {
        let anchor = Pos::new(start.row.min(end.row), start.col.min(end.col));
        let span = Self {
            cols: start.col.abs_diff(end.col) + 1,
            rows: start.row.abs_diff(end.row) + 1,
        };
        (anchor, span)
    }

    /// One cell, which is no merge at all.
    pub fn is_single(&self) -> bool {
        self.cols <= 1 && self.rows <= 1
    }

    /// The bottom-right cell of the merge starting at `anchor`.
    pub fn end(&self, anchor: Pos) -> Pos {
        Pos::new(
            anchor.row.saturating_add(self.rows.max(1) - 1),
            anchor.col.saturating_add(self.cols.max(1) - 1),
        )
    }

    /// Whether the merge starting at `anchor` takes in `pos`.
    pub fn contains(&self, anchor: Pos, pos: Pos) -> bool {
        let end = self.end(anchor);
        (anchor.row..=end.row).contains(&pos.row) && (anchor.col..=end.col).contains(&pos.col)
    }
}

/// A whole spreadsheet document.
#[derive(Debug, Clone)]
pub struct Document {
    pub sheets: Vec<Sheet>,
    /// `table:named-expressions` — a name to the formula text it stands for (§5.11), keyed
    /// lower-case since names match case-insensitively. A named range is stored as its reference
    /// (`[$Sheet1.$A$1]`), so one lookup serves both.
    ///
    /// ponytail: one flat map, so a sheet-local name is visible document-wide; two colliding
    /// sheet-local names are what this gets wrong.
    pub names: BTreeMap<String, String>,
    /// `HOST-NULL-DATE` (Part 4 §3.4 item 8): the epoch, as days from 1970-01-01, from
    /// `table:null-date`. Document-wide, as the spec says.
    pub null_date: i64,
    /// `HOST-NULL-YEAR` (Part 4 §3.4 item 7), from `table:null-year` — a corpus document sets
    /// 1919.
    pub null_year: i64,
    /// The document's own locale: `fo:language`/`fo:country` on the default cell style
    /// (`doc/ods-format.md` §5.2), or `None`. It decides the separators of every number shown
    /// without a locale of its own, and how typed numbers are read; a format naming a locale keeps
    /// it. LibreOffice uses the opening machine's locale instead (measured), so this is
    /// deliberately where the two differ.
    pub locale: Option<grind_core::locale::Locale>,
    /// The bytes this document was read from, if any (R6). Boxed: large and usually absent.
    pub source: Option<Box<crate::odf::source::Source>>,
    /// The projection this document was read from, if it was one (`doc/dsl.md` §3.1). A second
    /// slot because the two keep different things — XML elements beside
    /// [`crate::odf::source::Source`], an address-to-range map here — and a document has at most
    /// one.
    pub projection_source: Option<Box<grind_core::projection::Source>>,
    /// What has changed since. See [`Edits`].
    pub edits: Edits,
}

/// What has changed since the document was read, which decides whether saving can splice (R6).
/// Filled in by [`Document::apply`], the only way a document changes.
///
/// ponytail: an undone edit stays in `cells`, so saving rewrites that cell in our spelling:
/// correct, one element of diff instead of none.
#[derive(Clone, Debug)]
pub struct Edits {
    /// Every `(sheet, position)` written to.
    pub cells: std::collections::BTreeSet<(usize, Pos)>,
    /// Whether every edit so far was a value or a formula; a format or a style needs a new
    /// `style:style`, so false is sticky and the save regenerates.
    pub only_values: bool,
    /// Whether a named expression was added, changed or removed, so a regenerating save writes
    /// `table:named-expressions`.
    pub names: bool,
}

impl Default for Edits {
    fn default() -> Self {
        Self {
            cells: Default::default(),
            only_values: true,
            names: false,
        }
    }
}

/// One empty sheet, for a new document. A reader builds its sheets from the file, and a file
/// declaring none stays empty.
impl Default for Document {
    fn default() -> Self {
        Self {
            sheets: vec![Sheet::new("Sheet1")],
            names: BTreeMap::new(),
            null_date: crate::formula::date::DEFAULT_NULL_DATE,
            null_year: crate::formula::date::DEFAULT_NULL_YEAR,
            locale: None,
            source: None,
            projection_source: None,
            edits: Edits::default(),
        }
    }
}

impl Document {
    /// Resolve a named expression (§5.11), case-insensitively.
    pub fn name(&self, name: &str) -> Option<&str> {
        self.names.get(&name.to_lowercase()).map(String::as_str)
    }

    pub fn sheet(&self, i: usize) -> Option<&Sheet> {
        self.sheets.get(i)
    }

    pub fn sheet_mut(&mut self, i: usize) -> Option<&mut Sheet> {
        self.sheets.get_mut(i)
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn a_link_reads_and_writes_libreoffices_spelling() {
        let own = Link::parse("Checkliste.E5", "Checkliste").unwrap();
        assert_eq!(
            own,
            Link {
                sheet: None,
                pos: Pos::new(4, 4)
            }
        );
        assert_eq!(own.address("Checkliste"), "Checkliste.E5");
        let other = Link::parse("'Q3 Actuals'.B2", "Checkliste").unwrap();
        assert_eq!(other.sheet.as_deref(), Some("Q3 Actuals"));
        assert_eq!(other.address("Checkliste"), "'Q3 Actuals'.B2");
        assert_eq!(Link::parse("Checkliste.E5:E6", "x"), None);
    }
}
