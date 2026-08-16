// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document model: values, addresses, sheets, and the document itself.
//!
//! Positions are **0-based** everywhere in the core. Only the CLI is 1-based, and
//! it converts in exactly one place (doc/plan.md, phase 6).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::grid::Column;
use crate::numfmt::Format;
use crate::style::CellStyle;

/// A cell's value.
///
/// Deliberately has no formula variant: a formula's *value* is one of these, and the
/// formula text is a separate concern that arrives with the reader in phase 2. No
/// error variant yet either — the OpenFormula error set is normative and belongs
/// with the evaluator in phase 4 (doc/small-group.md), not invented here.
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

/// The Number subtype a cell was written as — §4.3.2 Time and §4.3.3 Date.
///
/// Both are Numbers and live in the grid as one, which is what makes date arithmetic work
/// at all. This records only how the cell *spelled* that number, so the writer can put it
/// back the way it found it instead of turning every date in a document into a bare serial.
///
/// This is the value *type*, not the display: how the cell is spelled out is
/// [`Sheet::format`]'s business (§5.2), and the two are independent — a date with no format
/// is a real cell, and the writer gives it the ISO default so LibreOffice does not invent a
/// locale one.
///
/// ponytail: a date that a *formula* computes has no kind, because the evaluator's `Value`
/// carries no Date subtype — so `=DATE(2026;8;16)` displays as its serial until the cell is
/// given a format. Part 4 §4.3.3 makes the subtype part of the value model, which is where
/// the fix belongs.
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

/// One sheet: a sparse set of columns.
///
/// Columns past the last used one simply do not exist, and trailing empty columns are
/// dropped on write, so `used_cols` is always the real extent rather than a high-water
/// mark.
#[derive(Debug, Default, Clone)]
pub struct Sheet {
    pub name: String,
    cols: Vec<Column>,
    /// Formula source text, kept verbatim beside the cached value.
    ///
    /// A side table rather than a cell variant, because a formula's *value* is an ordinary
    /// [`CellValue`] and only its source is extra. Phase 4 replaces the string with a
    /// parsed AST; until then carrying it means a re-save does not silently drop every
    /// formula in the document.
    formulas: BTreeMap<Pos, String>,
    /// Which cells hold a date or a time rather than a plain number — see [`NumberKind`].
    /// A side table for the same reason `formulas` is one: the *value* is an ordinary
    /// [`CellValue::Number`] and only its spelling is extra.
    kinds: BTreeMap<Pos, NumberKind>,
    /// How each cell is *displayed* (doc/ods-format.md §5.2). A third side table, and the
    /// same argument: a number format changes nothing about the value.
    ///
    /// ponytail: one [`Format`] per formatted cell rather than an index into a document-wide
    /// pool, so a column formatted top to bottom holds a thousand equal clones. Pooling
    /// happens on write, where it is required anyway (§5.3); intern here when a profile
    /// blames the clones.
    formats: BTreeMap<Pos, Format>,
    /// How each cell *looks* (§5.1) — the other half of its `style:style`. A fourth side
    /// table, and the same ponytail as `formats`: one clone per styled cell, pooled only on
    /// the way out.
    styles: BTreeMap<Pos, CellStyle>,
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
        }
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

    /// Back to the plain spelling of the value — ODF's "General", which is the absence of a
    /// data style rather than a style of its own.
    pub fn clear_format(&mut self, pos: Pos) {
        self.formats.remove(&pos);
    }

    /// The cell's styling, or `None` when it is drawn plainly.
    pub fn style(&self, pos: Pos) -> Option<&CellStyle> {
        self.styles.get(&pos)
    }

    /// A style that sets nothing is stored as no style at all, so "plain" has one spelling
    /// and the writer never emits an empty `style:style`.
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

    /// One past the last row holding a value, across all columns.
    pub fn used_rows(&self) -> u32 {
        self.cols.iter().map(Column::len).max().unwrap_or(0)
    }

    /// One past the last column holding a value.
    pub fn used_cols(&self) -> u32 {
        self.cols.len() as u32
    }
}

/// A whole spreadsheet document.
#[derive(Debug, Clone)]
pub struct Document {
    pub sheets: Vec<Sheet>,
    /// `table:named-expressions` — a name to the formula text it stands for (§5.11).
    ///
    /// Keyed lower-case because §5.11 makes names case-*consistent*: they match
    /// case-insensitively and may not differ only in case. The value is always a formula
    /// expression, so a named **range** is stored as the reference it is — `[$Sheet1.$A$1]`
    /// rather than the bare address ODF writes — and one lookup path serves both forms.
    ///
    /// ponytail: one flat map, so a sheet-local name is visible document-wide. §5.11 only
    /// *requires* global names, and a document with two sheet-local names that collide is
    /// the case this gets wrong. Split the map by sheet when one turns up.
    pub names: BTreeMap<String, String>,
    /// `HOST-NULL-DATE` (Part 4 §3.4 item 8): the epoch every serial date is counted from,
    /// as days from 1970-01-01. Set by `table:calculation-settings/table:null-date`.
    ///
    /// A document-wide setting rather than a per-sheet one because the spec makes it one —
    /// two sheets of the same document cannot disagree about what the number 0 means.
    pub null_date: i64,
    /// `HOST-NULL-YEAR` (Part 4 §3.4 item 7): "each two-digit year value is interpreted as
    /// a year that equals or follows this year". Set by `table:null-year`, which a document
    /// in the corpus really does put at 1919 — hence a setting rather than a constant.
    pub null_year: i64,
    /// The bytes this document was read from, when it was read from any (doc/plan.md R6).
    ///
    /// `None` for a document built in memory, which is every document `sheet new` makes —
    /// there is no file to preserve, so the writer generates one. Boxed because it is the
    /// only large-and-usually-absent field on a `Document`, and an `Option<Box<_>>` keeps
    /// the common case a pointer.
    pub source: Option<Box<crate::odf::source::Source>>,
    /// What has changed since. See [`Edits`].
    pub edits: Edits,
}

/// What has been changed since the document was read, which is what decides whether saving
/// can splice (doc/plan.md R6) or has to regenerate.
///
/// Filled in by [`Document::apply`], which is the only way a document changes — that is what
/// makes tracking this one insert rather than a diffing pass.
///
/// ponytail: an undone edit stays in `cells`, so undoing a change and saving rewrites that
/// cell in our spelling rather than restoring the file's. The result is correct and the diff
/// is one element instead of none. Comparing against the value read would fix it and needs a
/// snapshot of the document to compare *to*; not worth a second copy of the grid.
#[derive(Clone, Debug)]
pub struct Edits {
    /// Every `(sheet, position)` written to.
    pub cells: std::collections::BTreeSet<(usize, Pos)>,
    /// Whether every edit so far was a value or a formula.
    ///
    /// A number format or a cell style needs a `style:style` that the original file does not
    /// contain, so it cannot be spliced into it — see `odf::source`. False is sticky: once a
    /// document has had a format changed, saving it regenerates.
    pub only_values: bool,
}

impl Default for Edits {
    fn default() -> Self {
        Self {
            cells: Default::default(),
            only_values: true,
        }
    }
}

/// One empty sheet — what a *user* gets. A reader builds its sheets from the file instead,
/// and a document that declares none stays empty.
impl Default for Document {
    fn default() -> Self {
        Self {
            sheets: vec![Sheet::new("Sheet1")],
            names: BTreeMap::new(),
            null_date: crate::formula::date::DEFAULT_NULL_DATE,
            null_year: crate::formula::date::DEFAULT_NULL_YEAR,
            source: None,
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
