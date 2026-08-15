// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document model: values, addresses, sheets, and the document itself.
//!
//! Positions are **0-based** everywhere in the core. Only the CLI is 1-based, and
//! it converts in exactly one place (doc/plan.md, phase 6).

use std::collections::BTreeMap;

use crate::grid::Column;

/// A cell's value.
///
/// Deliberately has no formula variant: a formula's *value* is one of these, and the
/// formula text is a separate concern that arrives with the reader in phase 2. No
/// error variant yet either — the OpenFormula error set is normative and belongs
/// with the evaluator in phase 4 (doc/small-group.md), not invented here.
#[derive(Clone, Debug, Default, PartialEq)]
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

/// A cell address within one sheet, 0-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
}

impl Sheet {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            cols: Vec::new(),
            formulas: BTreeMap::new(),
        }
    }

    pub fn formula(&self, pos: Pos) -> Option<&str> {
        self.formulas.get(&pos).map(String::as_str)
    }

    pub fn set_formula(&mut self, pos: Pos, formula: String) {
        self.formulas.insert(pos, formula);
    }

    pub fn formula_count(&self) -> usize {
        self.formulas.len()
    }

    /// Every formula in the sheet, in address order — what a full recalculation walks.
    pub fn formulas(&self) -> impl Iterator<Item = (Pos, &str)> {
        self.formulas.iter().map(|(pos, f)| (*pos, f.as_str()))
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
}

/// One empty sheet — what a *user* gets. A reader builds its sheets from the file instead,
/// and a document that declares none stays empty.
impl Default for Document {
    fn default() -> Self {
        Self {
            sheets: vec![Sheet::new("Sheet1")],
            names: BTreeMap::new(),
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
