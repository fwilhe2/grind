// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ODS content model: one context per element we care about. **[ODS]**
//!
//! Structure per doc/ods-format.md §3. Everything not named here is handled by
//! `context::Ignore` without a line of code, which is the point of §8.
//!
//! Contexts never talk to each other. A child is told where it lives when it is created,
//! and all mutation flows through the shared [`Builder`] — so there is no child-to-parent
//! channel to get wrong, and no downcasting.

use crate::model::{CellValue, Document, Pos, Sheet};

use super::context::{Attrs, Context};
use super::names::{Name, Ns};

/// LibreOffice's sheet limits, and the clamp for every repeat count (§9).
const MAX_ROWS: u32 = 1_048_576;
const MAX_COLS: u32 = 16_384;

/// Ceiling on cells actually written, across the whole document.
///
/// §9 says to clamp repeat counts to the sheet limits, and that is necessary but not
/// sufficient: clamping bounds each *count* while the cost is their *product*. A row that
/// repeats 1 048 576 times holding a cell that repeats 16 384 times is entirely legal by
/// both clamps and asks for seventeen billion writes. So the budget bounds the work
/// itself. Only non-empty cells are counted — the trailing empty run that bounds a sheet's
/// extent is free — which puts the ceiling far above any real spreadsheet while still
/// making a hostile or generator-broken file terminate.
const MAX_MATERIALISED_CELLS: u64 = 4_000_000;

/// Everything the contexts share: the document under construction and the cursors into it.
pub struct Builder {
    pub doc: Document,
    sheet: usize,
    /// Next row to write in the current sheet.
    row: u32,
    /// Next column to write in the current row.
    col: u32,
    /// Cells of the row being read, held back until the row ends because
    /// `table:number-rows-repeated` may ask for the whole row again.
    row_cells: Vec<(u32, CellValue, Option<String>)>,
    /// Text accumulated by the paragraph contexts under the current cell.
    text: String,
    /// Cells left in the materialisation budget. See [`MAX_MATERIALISED_CELLS`].
    budget: u64,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            // Sheets come from the file. A document that declares none stays empty rather
            // than inheriting the default "Sheet1" a new document gets.
            doc: Document { sheets: Vec::new() },
            sheet: 0,
            row: 0,
            col: 0,
            row_cells: Vec::new(),
            text: String::new(),
            budget: MAX_MATERIALISED_CELLS,
        }
    }

    fn start_sheet(&mut self, name: String) {
        self.doc.sheets.push(Sheet::new(name));
        self.sheet = self.doc.sheets.len() - 1;
        self.row = 0;
    }

    /// Write the buffered row `repeat` times, then advance the row cursor.
    ///
    /// The empty case is the one that matters for speed *and* for not falling over: a
    /// trailing `<table:table-row table:number-rows-repeated="1048576">` holding one empty
    /// cell is how a sheet's extent is conventionally bounded (§3.3), and it is in most
    /// real files. With no cells buffered there is nothing to replay, so it costs one
    /// addition rather than a million iterations.
    fn finish_row(&mut self, repeat: u32) {
        if self.row_cells.is_empty() {
            self.row = self.row.saturating_add(repeat).min(MAX_ROWS);
            return;
        }
        let Some(sheet) = self.doc.sheets.get_mut(self.sheet) else {
            self.row_cells.clear();
            return;
        };

        // How many copies of this row we can afford. The cursor still advances by the full
        // repeat below, so cells after a truncated run keep their correct addresses — a
        // budget must never silently shift the rest of the sheet sideways.
        let per_row = self.row_cells.len() as u64;
        let affordable = (self.budget / per_row.max(1)).min(u64::from(repeat)) as u32;

        for r in 0..affordable {
            let row = self.row.saturating_add(r);
            if row >= MAX_ROWS {
                break;
            }
            for (col, value, formula) in &self.row_cells {
                let pos = Pos::new(row, *col);
                if !value.is_empty() {
                    sheet.set(pos, value.clone());
                }
                if let Some(f) = formula {
                    sheet.set_formula(pos, f.clone());
                }
            }
            self.budget = self.budget.saturating_sub(per_row);
        }

        self.row_cells.clear();
        self.row = self.row.saturating_add(repeat).min(MAX_ROWS);
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

type Ctx = Box<dyn Context<Builder>>;

/// The document root.
///
/// Accepts `office:document` (flat form) and `office:document-content` (the `content.xml`
/// of a package) identically — below the root the two are the same model (§1.2).
pub struct Root;

impl Context<Builder> for Root {
    fn start_child(&mut self, name: &Name, _a: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Office, "document" | "document-content") => Some(Box::new(Root)),
            (Ns::Office, "body") => Some(Box::new(Body)),
            _ => None,
        }
    }
}

struct Body;

impl Context<Builder> for Body {
    fn start_child(&mut self, name: &Name, _a: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        name.is(Ns::Office, "spreadsheet")
            .then(|| Box::new(Spreadsheet) as Ctx)
    }
}

struct Spreadsheet;

impl Context<Builder> for Spreadsheet {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Table, "table") {
            return None;
        }
        let sheet_name = attrs.get(Ns::Table, "name").unwrap_or("Sheet").to_owned();
        b.start_sheet(sheet_name);
        Some(Box::new(Table))
    }
}

struct Table;

impl Context<Builder> for Table {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Table, "table-row") => {
                b.col = 0;
                b.row_cells.clear();
                let repeat = attrs.count(Ns::Table, "number-rows-repeated", MAX_ROWS);
                Some(Box::new(Row { repeat }))
            }
            // Row and column groups nest real rows inside a wrapper. Recursing with the
            // same context keeps the cursor continuous instead of losing the contents.
            (Ns::Table, "table-row-group" | "table-header-rows" | "table-rows") => {
                Some(Box::new(Table))
            }
            _ => None,
        }
    }
}

struct Row {
    repeat: u32,
}

impl Context<Builder> for Row {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let covered = name.is(Ns::Table, "covered-table-cell");
        if !covered && !name.is(Ns::Table, "table-cell") {
            return None;
        }

        // Claim this cell's columns now, so the context does not need to report back.
        let start = b.col;
        let repeat = attrs.count(Ns::Table, "number-columns-repeated", MAX_COLS);
        b.col = b.col.saturating_add(repeat).min(MAX_COLS);
        b.text.clear();

        // A covered cell is the hidden half of a merge: it holds a grid position but no
        // value of its own.
        if covered {
            return Some(Box::new(super::context::Ignore));
        }

        Some(Box::new(Cell {
            start,
            repeat,
            value_type: attrs
                .get_any(&[(Ns::Office, "value-type"), (Ns::Calcext, "value-type")])
                .map(str::to_owned),
            value: attrs.get(Ns::Office, "value").map(str::to_owned),
            string_value: attrs.get(Ns::Office, "string-value").map(str::to_owned),
            boolean_value: attrs.get(Ns::Office, "boolean-value").map(str::to_owned),
            date_value: attrs.get(Ns::Office, "date-value").map(str::to_owned),
            time_value: attrs.get(Ns::Office, "time-value").map(str::to_owned),
            formula: attrs.get(Ns::Table, "formula").map(str::to_owned),
            cached_error: attrs.get(Ns::Calcext, "value-type") == Some("error"),
            saw_paragraph: false,
        }))
    }

    fn end(&mut self, b: &mut Builder) {
        b.finish_row(self.repeat);
    }
}

struct Cell {
    start: u32,
    repeat: u32,
    value_type: Option<String>,
    value: Option<String>,
    string_value: Option<String>,
    boolean_value: Option<String>,
    date_value: Option<String>,
    time_value: Option<String>,
    formula: Option<String>,
    /// `calcext:value-type="error"` — the formula's cached result is an error.
    cached_error: bool,
    /// Whether a `text:p` has already been seen, so the *second* one starts a new line.
    ///
    /// Counting paragraphs rather than testing whether the accumulated text is empty:
    /// otherwise `<text:p/><text:p/>` — a cell holding one blank line — collapses to the
    /// empty string, and every leading or doubled newline is silently eaten.
    saw_paragraph: bool,
}

impl Cell {
    /// Resolve the cell's value from its attributes and its display text.
    ///
    /// Tolerance per §9: every failure degrades to a safe default and is scoped to this
    /// cell. A malformed number is 0, not a rejected document; an absent `office:value` for
    /// a numeric type falls back to the paragraph text; and a cell with no value-type at
    /// all but some text is a string, which is the ODF rule that display text *is* the
    /// value when no explicit one is given.
    fn resolve(&self, text: &str) -> CellValue {
        let number = |raw: &Option<String>| -> CellValue {
            raw.as_deref()
                .or(Some(text))
                .and_then(|s| s.trim().parse::<f64>().ok())
                .filter(|f| f.is_finite())
                .map_or(CellValue::Number(0.0), CellValue::Number)
        };

        match self.value_type.as_deref() {
            Some("float" | "percentage" | "currency") => number(&self.value),
            Some("boolean") => {
                let raw = self.boolean_value.as_deref().unwrap_or(text).trim();
                CellValue::Bool(raw.eq_ignore_ascii_case("true") || raw == "1")
            }
            // ponytail: dates and times are kept as their ISO text. Converting to a serial
            // number needs `table:calculation-settings/table:null-date`, which arrives with
            // the evaluator in phase 4 — converting now against an assumed epoch would bake
            // in exactly the Excel-shaped guess this project exists to avoid. The original
            // string is preserved meanwhile, so nothing is lost.
            Some("date") => self
                .date_value
                .clone()
                .map_or(CellValue::Text(text.to_owned()), CellValue::Text),
            Some("time") => self
                .time_value
                .clone()
                .map_or(CellValue::Text(text.to_owned()), CellValue::Text),
            Some("string" | "error") => {
                // Part 4 §4.6 stores an error result as a string — but LO writes the *empty*
                // string into `office:string-value` and leaves the error name in the display
                // text, flagging the cell with `calcext:value-type="error"`
                // (doc/ods-format.md §6). Trusting the attribute there turns every cached
                // error into an empty cell, so for those the paragraph is the value.
                if self.cached_error {
                    return CellValue::Text(text.to_owned());
                }
                CellValue::Text(self.string_value.clone().unwrap_or_else(|| text.to_owned()))
            }
            // An unknown value-type is not a reason to lose the cell: keep what is visible.
            Some(_) | None => {
                if let Some(s) = &self.string_value {
                    CellValue::Text(s.clone())
                } else if text.is_empty() {
                    CellValue::Empty
                } else {
                    CellValue::Text(text.to_owned())
                }
            }
        }
    }
}

impl Context<Builder> for Cell {
    fn start_child(&mut self, name: &Name, _a: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Text, "p") {
            return None;
        }
        // A cell may hold several paragraphs; they are separate lines, not one run-on
        // string.
        if self.saw_paragraph {
            b.text.push('\n');
        }
        self.saw_paragraph = true;
        Some(Box::new(Paragraph))
    }

    fn end(&mut self, b: &mut Builder) {
        let text = std::mem::take(&mut b.text);
        let value = self.resolve(&text);

        // Phase 4 parses this. Until then it is carried verbatim so a re-save does not
        // silently drop every formula in the document (§9).
        let formula = self.formula.clone();

        if value.is_empty() && formula.is_none() {
            return; // Nothing to write; the columns were claimed at start.
        }
        for i in 0..self.repeat {
            let col = self.start.saturating_add(i);
            if col >= MAX_COLS {
                break;
            }
            b.row_cells.push((col, value.clone(), formula.clone()));
        }
    }
}

/// `text:p` and the runs beneath it — the only contexts that collect character data.
struct Paragraph;

impl Context<Builder> for Paragraph {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            // A span is styled text; its characters still belong to the cell.
            (Ns::Text, "span" | "a") => Some(Box::new(Paragraph)),
            // `text:s` carries a *count*: ODF collapses runs of whitespace in `text:p`, so
            // "a    b" is written as one literal space plus `<text:s text:c="3"/>`.
            // Ignoring the count silently turns every multi-space string into a
            // single-space one.
            (Ns::Text, "s") => {
                let n = attrs.count(Ns::Text, "c", MAX_COLS);
                for _ in 0..n {
                    b.text.push(' ');
                }
                Some(Box::new(super::context::Ignore))
            }
            (Ns::Text, "tab") => {
                b.text.push('\t');
                Some(Box::new(super::context::Ignore))
            }
            (Ns::Text, "line-break") => {
                b.text.push('\n');
                Some(Box::new(super::context::Ignore))
            }
            _ => None,
        }
    }

    fn text(&mut self, text: &str, b: &mut Builder) {
        b.text.push_str(text);
    }
}
