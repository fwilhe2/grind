// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ODS content model: one context per element we care about. **\[ODS\]**
//!
//! Structure per doc/ods-format.md §3. Everything not named here is handled by
//! `context::Ignore` without a line of code, which is the point of §8.
//!
//! Contexts never talk to each other. A child is told where it lives when it is created,
//! and all mutation flows through the shared [`Builder`] — so there is no child-to-parent
//! channel to get wrong, and no downcasting.

use std::collections::{BTreeSet, HashMap};

use crate::filter::Filter;
use crate::formula::date;
use crate::locale::Locale;
use crate::model::{CellValue, Document, NumberKind, Pos, Sheet};
use crate::numfmt::{self, Format, Kind, Map, Op, Part};
use crate::style::{CellStyle, EDGES};

use super::context::{Attrs, Context};
use super::names::{Name, Ns};

/// The sheet limits, which are also the clamp for every repeat count (§9).
use crate::{MAX_COLS, MAX_ROWS};

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

/// How long a run of equally sized columns or rows is still *layout* rather than the
/// sheet's background width. Shared with the setting side — see [`crate::MAX_TRACK_RUN`].
use crate::MAX_TRACK_RUN;

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
    row_cells: Vec<RowCell>,
    /// `table:number-rows-repeated` of the row being read. One element there stands for many
    /// rows, so none of its cells can be spliced — see [`crate::odf::source`].
    row_repeat: u32,
    /// Where each cell element of the row being read sits in the file (R6), in column order.
    /// Beside `row_cells` rather than inside it because there is one entry per *element*
    /// and `row_cells` has one per address.
    row_spans: Vec<super::source::Cell>,
    /// Text accumulated by the paragraph contexts under the current cell.
    text: String,
    /// Cells left in the materialisation budget. See [`MAX_MATERIALISED_CELLS`].
    budget: u64,

    // --- styles (§5) ---
    /// `number:*-style` by `style:name`.
    number_styles: HashMap<String, Format>,
    /// A `table-cell` `style:style`'s name to its `style:data-style-name` — the one link
    /// between "how it looks" and "how the number renders" (§5.1).
    cell_styles: HashMap<String, String>,
    /// The same styles' own properties: fonts, colours, borders, alignment.
    style_props: HashMap<String, CellStyle>,
    /// The parts of the `number:*-style` currently being read, and the text its literal
    /// pieces are collecting. Both live here rather than in the context, because a child
    /// element has no channel back to its parent — the parent takes these at its `end`.
    parts: Vec<Part>,
    style_text: String,
    /// `style:map` branches whose target has only been *named* so far — the style that owns
    /// the map, the condition, and the name it applies. A target may be declared after the
    /// style that points at it, so resolution waits until the section ends.
    pending_maps: Vec<(String, Op, String, String)>,
    /// `table:default-cell-style-name` per column of the current sheet, and for the current
    /// row. A cell's own `table:style-name` wins over the row's, which wins over the
    /// column's — the resolution order §5.1's indirection implies and the reason a
    /// whole-column format is not lost.
    col_styles: Vec<Option<String>>,
    row_style: Option<String>,
    /// Next column to be claimed by a `table:table-column` declaration.
    col_decl: u32,
    /// A `table-column` or `table-row` `style:style`'s name to the one property this model
    /// keeps of it: `style:column-width` or `style:row-height`, verbatim. One map for both
    /// families, because a `style:name` is unique across the document (§5.1).
    track_sizes: HashMap<String, String>,
    /// `style:row-height` of the row being read, applied when the row ends because
    /// `table:number-rows-repeated` decides how many rows it is for.
    row_size: Option<String>,

    // --- the autofilter (§9.4) ---
    /// The `table:database-range` being read and the sheet it covers, until its element
    /// ends. Here rather than in the context because its conditions are two levels down.
    filter: Option<(usize, Filter)>,
    /// The `table:filter-condition` being read: its field number and the values so far.
    filter_values: (u32, BTreeSet<String>),
}

/// One cell of the buffered row. A struct rather than a tuple since the day it grew a
/// fourth member.
struct RowCell {
    col: u32,
    value: CellValue,
    formula: Option<String>,
    kind: Option<NumberKind>,
    format: Option<Format>,
    style: Option<CellStyle>,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            // Sheets come from the file. A document that declares none stays empty rather
            // than inheriting the default "Sheet1" a new document gets.
            doc: Document {
                sheets: Vec::new(),
                names: Default::default(),
                null_date: date::DEFAULT_NULL_DATE,
                null_year: date::DEFAULT_NULL_YEAR,
                source: None,
                edits: Default::default(),
            },
            sheet: 0,
            row: 0,
            col: 0,
            row_cells: Vec::new(),
            row_repeat: 1,
            row_spans: Vec::new(),
            text: String::new(),
            budget: MAX_MATERIALISED_CELLS,
            number_styles: HashMap::new(),
            cell_styles: HashMap::new(),
            style_props: HashMap::new(),
            parts: Vec::new(),
            style_text: String::new(),
            pending_maps: Vec::new(),
            col_styles: Vec::new(),
            row_style: None,
            col_decl: 0,
            track_sizes: HashMap::new(),
            row_size: None,
            filter: None,
            filter_values: Default::default(),
        }
    }

    fn start_sheet(&mut self, name: String) {
        self.doc.sheets.push(Sheet::new(name));
        self.sheet = self.doc.sheets.len() - 1;
        self.row = 0;
        self.col_styles.clear();
        self.col_decl = 0;
    }

    /// The number format a cell displays through, following §5.1's indirection: the cell
    /// style names a data style, and the data style is the format. A style that names none,
    /// or names one the document does not define, simply leaves the cell unformatted —
    /// tolerance again (§9), and the reason this returns an `Option` rather than a result.
    fn resolve_format(&self, cell_style: Option<&str>, col: u32) -> Option<Format> {
        let style = self.style_name(cell_style, col)?;
        let data_style = self.cell_styles.get(style)?;
        self.number_styles.get(data_style).cloned()
    }

    /// The cell's own styling, through the same chain — one lookup, two answers, because a
    /// `style:style` carries both.
    fn resolve_style(&self, cell_style: Option<&str>, col: u32) -> Option<CellStyle> {
        let style = self.style_name(cell_style, col)?;
        self.style_props.get(style).cloned()
    }

    /// Which `style:style` applies: the cell's own, else the row's default, else the
    /// column's. The column default is consulted last, and only for cells that exist, so a
    /// styled column costs entries for its cells rather than for its million rows.
    fn style_name<'a>(&'a self, cell_style: Option<&'a str>, col: u32) -> Option<&'a str> {
        cell_style
            .or(self.row_style.as_deref())
            .or_else(|| self.col_styles.get(col as usize)?.as_deref())
    }

    /// Attach every `style:map` collected in this section to the style that declared it.
    ///
    /// Deferred to the end of the section because a map may name a style declared after it,
    /// and resolved against a *snapshot* so a branch never gains branches of its own — one
    /// level is what LibreOffice writes and all that [`Format::render`] follows.
    fn resolve_maps(&mut self) {
        let targets = self.number_styles.clone();
        for (owner, op, value, target) in std::mem::take(&mut self.pending_maps) {
            let (Some(format), Some(owner)) =
                (targets.get(&target), self.number_styles.get_mut(&owner))
            else {
                continue; // A map naming a style the document does not define is dropped.
            };
            let mut format = format.clone();
            // A branch that names no locale of its own follows the style that points at it,
            // or a mapped negative would print with different separators from the positive.
            if format.locale.is_none() {
                format.locale.clone_from(&owner.locale);
            }
            owner.maps.push(Map { op, value, format });
        }
    }

    /// Claim `repeat` columns for a `table:table-column` declaration, recording the default
    /// cell style it gives them.
    fn declare_columns(&mut self, style: Option<String>, width: Option<&str>, repeat: u32) {
        let end = self.col_decl.saturating_add(repeat).min(MAX_COLS);
        // A column that names no default style still has to occupy its slots, or every
        // later declaration lands on the wrong column.
        if style.is_some() {
            self.col_styles.resize(end as usize, None);
            for slot in &mut self.col_styles[self.col_decl as usize..end as usize] {
                slot.clone_from(&style);
            }
        }
        if let Some(width) = width
            && repeat <= MAX_TRACK_RUN
            && let Some(sheet) = self.doc.sheets.get_mut(self.sheet)
        {
            for col in self.col_decl..end {
                sheet.set_col_width(col, Some(width.to_owned()));
            }
        }
        self.col_decl = end;
    }

    /// Write the buffered row `repeat` times, then advance the row cursor.
    ///
    /// The empty case is the one that matters for speed *and* for not falling over: a
    /// trailing `<table:table-row table:number-rows-repeated="1048576">` holding one empty
    /// cell is how a sheet's extent is conventionally bounded (§3.3), and it is in most
    /// real files. With no cells buffered there is nothing to replay, so it costs one
    /// addition rather than a million iterations.
    fn finish_row(&mut self, repeat: u32) {
        // R6: hand this row's cell elements to the source, but only when the row stands for
        // itself. A repeated row's one element covers many addresses, and splitting *that*
        // means emitting whole rows — which is no longer a small diff, which was the point.
        let spans = std::mem::take(&mut self.row_spans);
        if repeat == 1
            && !spans.is_empty()
            && let Some(source) = self.doc.source.as_mut()
        {
            source.rows.insert((self.sheet, self.row), spans);
        }

        // A height applies to the row whether or not it holds anything — an empty row a
        // document made tall is still tall.
        if let Some(height) = self.row_size.take()
            && repeat <= MAX_TRACK_RUN
            && let Some(sheet) = self.doc.sheets.get_mut(self.sheet)
        {
            for r in 0..repeat {
                let row = self.row.saturating_add(r);
                if row >= MAX_ROWS {
                    break;
                }
                sheet.set_row_height(row, Some(height.clone()));
            }
        }

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
            for cell in &self.row_cells {
                let pos = Pos::new(row, cell.col);
                if !cell.value.is_empty() {
                    sheet.set(pos, cell.value.clone());
                }
                if let Some(f) = &cell.formula {
                    sheet.set_formula(pos, f.clone());
                }
                if let Some(k) = cell.kind {
                    sheet.set_kind(pos, k);
                }
                if let Some(f) = &cell.format {
                    sheet.set_format(pos, f.clone());
                }
                if let Some(s) = &cell.style {
                    sheet.set_style(pos, s.clone());
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
            // `office:document-styles` is the root of a package's `styles.xml`, which is
            // parsed with this same context stack — the named styles a cell references may
            // live there rather than in `content.xml` (§1.1).
            (Ns::Office, "document" | "document-content" | "document-styles") => {
                Some(Box::new(Root))
            }
            (Ns::Office, "body") => Some(Box::new(Body)),
            // Both style sections hold the same elements and differ only in whether a human
            // named them (§5.1), so one context reads both.
            (Ns::Office, "automatic-styles" | "styles") => Some(Box::new(Styles)),
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
        if name.is(Ns::Table, "named-expressions") {
            return Some(Box::new(NamedExpressions));
        }
        if name.is(Ns::Table, "database-ranges") {
            return Some(Box::new(DatabaseRanges));
        }
        if name.is(Ns::Table, "calculation-settings") {
            // `table:null-year` is an attribute here; `table:null-date` is a child element.
            if let Some(year) = attrs.get(Ns::Table, "null-year")
                && let Ok(year) = year.trim().parse::<i64>()
            {
                b.doc.null_year = year;
            }
            return Some(Box::new(CalculationSettings));
        }
        if !name.is(Ns::Table, "table") {
            return None;
        }
        let sheet_name = attrs.get(Ns::Table, "name").unwrap_or("Sheet").to_owned();
        b.start_sheet(sheet_name);
        Some(Box::new(Table))
    }
}

/// `table:named-expressions` (§5.11), which appears both here and inside a `table:table`
/// for sheet-local names. Both land in the same map — see [`Document::names`].
struct NamedExpressions;

impl Context<Builder> for NamedExpressions {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let Some(key) = attrs.get(Ns::Table, "name") else {
            return Some(Box::new(super::context::Ignore));
        };
        // A named *range* carries a bare cell-range address; a named *expression* carries a
        // formula. Storing the range as the reference it stands for — brackets and all —
        // means the evaluator has one kind of thing to parse rather than two.
        let expression = if name.is(Ns::Table, "named-range") {
            attrs
                .get(Ns::Table, "cell-range-address")
                .map(|address| format!("[{address}]"))
        } else if name.is(Ns::Table, "named-expression") {
            attrs.get(Ns::Table, "expression").map(str::to_owned)
        } else {
            None
        };
        if let Some(expression) = expression {
            b.doc.names.insert(key.to_lowercase(), expression);
        }
        Some(Box::new(super::context::Ignore))
    }
}

/// `table:calculation-settings`, for the one setting that changes what a stored number
/// *means*: `table:null-date`, the epoch (Part 4 §3.4 item 8).
///
/// The schema puts this element before the tables, so the epoch is always known by the
/// time a cell needs it. Everything else in here — iteration, case sensitivity, wildcards —
/// is left to `Ignore` until something depends on it.
struct CalculationSettings;

impl Context<Builder> for CalculationSettings {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Table, "null-date")
            && let Some(value) = attrs.get(Ns::Table, "date-value")
            // Parsed against 1970-01-01 rather than against the epoch being defined, which
            // would be circular. An unparseable date leaves the default in place (§9).
            && let Some(days) = date::parse_date(value, 0)
        {
            b.doc.null_date = days as i64;
        }
        Some(Box::new(super::context::Ignore))
    }
}

/// `table:database-ranges` (§9.4) — the autofilters, one per sheet at most.
///
/// The range is addressed by name (`Sheet1.A1:Sheet1.F12`), so it is parsed with
/// [`crate::a1`] and matched to a sheet here; the schema puts this element *after* the
/// tables, so every sheet it can name already exists.
struct DatabaseRanges;

impl Context<Builder> for DatabaseRanges {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Table, "database-range") {
            return None;
        }
        let address = attrs.get(Ns::Table, "target-range-address")?;
        let reference = crate::a1::parse(address).ok()?;
        let end = reference.end.clone().unwrap_or(reference.start.clone());
        // A whole column or row as a filter range is not a shape LibreOffice writes, and
        // guessing an extent for one would be inventing the filter's meaning.
        let (start, end) = (cell_pos(&reference.start)?, cell_pos(&end)?);
        let sheet = reference.start.sheet.as_deref()?;
        let sheet = b
            .doc
            .sheets
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(sheet))?;
        let mut filter = Filter::new(attrs.get(Ns::Table, "name").unwrap_or_default(), start, end);
        // Both default to true (§9.4).
        filter.contains_header = attrs.get(Ns::Table, "contains-header") != Some("false");
        filter.buttons = attrs.get(Ns::Table, "display-filter-buttons") != Some("false");
        b.filter = Some((sheet, filter));
        Some(Box::new(DatabaseRange))
    }
}

/// A `CellRef` as a position, when it names both axes.
fn cell_pos(cell: &crate::formula::lex::CellRef) -> Option<Pos> {
    Some(Pos::new(cell.row?.index, cell.col?.index))
}

struct DatabaseRange;

impl Context<Builder> for DatabaseRange {
    fn start_child(&mut self, name: &Name, _a: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        // `table:filter` and `table:filter-and` are both just wrappers around the
        // conditions this model keeps; `table:filter-or` is not, so it goes to `Ignore` and
        // the range keeps its buttons and no conditions.
        name.is(Ns::Table, "filter")
            .then(|| Box::new(FilterAnd) as Ctx)
    }

    fn end(&mut self, b: &mut Builder) {
        if let Some((sheet, filter)) = b.filter.take()
            && let Some(sheet) = b.doc.sheets.get_mut(sheet)
        {
            sheet.set_filter(Some(filter));
        }
    }
}

struct FilterAnd;

impl Context<Builder> for FilterAnd {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Table, "filter-and") => Some(Box::new(FilterAnd)),
            (Ns::Table, "filter-condition") => {
                // Only a set of values. Anything else — `<`, `begins-with`, top-10 — is
                // dropped rather than half-applied (see [`crate::filter`]).
                if attrs.get(Ns::Table, "operator") != Some("=") {
                    return None;
                }
                let field = attrs.get(Ns::Table, "field-number")?.parse().ok()?;
                b.filter_values = (
                    field,
                    // A condition with no `filter-set-item` children is a single value,
                    // and that is what `table:value` holds.
                    [attrs.get(Ns::Table, "value").unwrap_or_default().to_owned()].into(),
                );
                Some(Box::new(FilterCondition { items: false }))
            }
            _ => None,
        }
    }
}

struct FilterCondition {
    /// Whether any `filter-set-item` has been seen — the first one replaces the condition's
    /// own `table:value` rather than joining it.
    items: bool,
}

impl Context<Builder> for FilterCondition {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Table, "filter-set-item") {
            if !std::mem::replace(&mut self.items, true) {
                b.filter_values.1.clear();
            }
            b.filter_values
                .1
                .insert(attrs.get(Ns::Table, "value").unwrap_or_default().to_owned());
        }
        Some(Box::new(super::context::Ignore))
    }

    fn end(&mut self, b: &mut Builder) {
        let (field, values) = std::mem::take(&mut b.filter_values);
        if let Some((_, filter)) = b.filter.as_mut() {
            filter.keep.insert(field, values);
        }
    }
}

struct Table;

impl Context<Builder> for Table {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Table, "table-row") => {
                b.col = 0;
                b.row_cells.clear();
                b.row_style = attrs
                    .get(Ns::Table, "default-cell-style-name")
                    .map(str::to_owned);
                // §5.4: the row's own `style:style` is where its height lives.
                b.row_size = attrs
                    .get(Ns::Table, "style-name")
                    .and_then(|name| b.track_sizes.get(name))
                    .cloned();
                let repeat = attrs.count(Ns::Table, "number-rows-repeated", MAX_ROWS);
                b.row_repeat = repeat;
                Some(Box::new(Row { repeat }))
            }
            // Columns carry the default cell style for everything below them, which is how
            // a whole formatted column is written (§3.3).
            (Ns::Table, "table-column") => {
                let style = attrs
                    .get(Ns::Table, "default-cell-style-name")
                    .map(str::to_owned);
                let width = attrs
                    .get(Ns::Table, "style-name")
                    .and_then(|name| b.track_sizes.get(name))
                    .cloned();
                let repeat = attrs.count(Ns::Table, "number-columns-repeated", MAX_COLS);
                b.declare_columns(style, width.as_deref(), repeat);
                Some(Box::new(super::context::Ignore))
            }
            (Ns::Table, "table-column-group" | "table-header-columns" | "table-columns") => {
                Some(Box::new(Table))
            }
            // Row and column groups nest real rows inside a wrapper. Recursing with the
            // same context keeps the cursor continuous instead of losing the contents.
            (Ns::Table, "table-row-group" | "table-header-rows" | "table-rows") => {
                Some(Box::new(Table))
            }
            // Sheet-local names (§5.11).
            (Ns::Table, "named-expressions") => Some(Box::new(NamedExpressions)),
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

        // R6: where this element sits, so a later save can replace it in place. Recorded for
        // a repeated cell too — that element is split rather than skipped — but not inside a
        // repeated row, where one element stands for many rows. `Attrs::span` is the whole
        // element for the `<table:table-cell/>` form and the start tag for the other, which
        // `Cell::end` finishes.
        let span = (b.row_repeat == 1 && b.doc.source.is_some()).then(|| attrs.span());

        Some(Box::new(Cell {
            start,
            repeat,
            span,
            value_type: attrs
                .get_any(&[(Ns::Office, "value-type"), (Ns::Calcext, "value-type")])
                .map(str::to_owned),
            value: attrs.get(Ns::Office, "value").map(str::to_owned),
            string_value: attrs.get(Ns::Office, "string-value").map(str::to_owned),
            boolean_value: attrs.get(Ns::Office, "boolean-value").map(str::to_owned),
            date_value: attrs.get(Ns::Office, "date-value").map(str::to_owned),
            time_value: attrs.get(Ns::Office, "time-value").map(str::to_owned),
            formula: attrs.get(Ns::Table, "formula").map(str::to_owned),
            style: attrs.get(Ns::Table, "style-name").map(str::to_owned),
            cached_error: attrs.get(Ns::Calcext, "value-type") == Some("error"),
            saw_paragraph: false,
        }))
    }

    fn end(&mut self, b: &mut Builder) {
        b.finish_row(self.repeat);
    }
}

/// Where a cell element ends, given the span of its start tag.
///
/// `Attrs::span` covers the whole element for `<table:table-cell/>` — the common form, and
/// the one that arrives as a single `Event::Empty` — and only the start tag for a cell with
/// children, because `Context::end` is told nothing about position. Rather than widen that
/// trait for one caller, scan forward for the close tag.
///
/// A `table:table-cell` may legally contain a whole subtable, and therefore other cells; a
/// naive scan would then stop at the *inner* close tag and splice a fragment. So an opening
/// `<table:table-cell` before the close means this cell is simply not replaceable, which
/// costs a rare document its small diff and cannot cost it correctness.
fn cell_extent(bytes: &[u8], start_tag: std::ops::Range<usize>) -> Option<std::ops::Range<usize>> {
    const OPEN: &[u8] = b"<table:table-cell";
    const CLOSE: &[u8] = b"</table:table-cell>";
    if bytes.get(start_tag.end.checked_sub(2)?..start_tag.end)? == b"/>" {
        return Some(start_tag);
    }
    let rest = bytes.get(start_tag.end..)?;
    let close = rest
        .windows(CLOSE.len())
        .position(|w| w == CLOSE)
        .filter(|at| !rest[..*at].windows(OPEN.len()).any(|w| w == OPEN))?;
    Some(start_tag.start..start_tag.end + close + CLOSE.len())
}

struct Cell {
    start: u32,
    repeat: u32,
    /// The start tag's extent, when this cell is replaceable. See [`cell_extent`].
    span: Option<std::ops::Range<usize>>,
    value_type: Option<String>,
    value: Option<String>,
    string_value: Option<String>,
    boolean_value: Option<String>,
    date_value: Option<String>,
    time_value: Option<String>,
    formula: Option<String>,
    /// `table:style-name` — resolved to a number format at the cell's end, when the styles
    /// section has certainly been read (the schema puts it before the body).
    style: Option<String>,
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
    ///
    /// The second half of the answer is the cell's [`NumberKind`], which is `Some` exactly
    /// when a date or a time was successfully converted — see the `date`/`time` arms.
    fn resolve(&self, text: &str, null_date: i64) -> (CellValue, Option<NumberKind>) {
        let number = |raw: &Option<String>| -> CellValue {
            raw.as_deref()
                .or(Some(text))
                .and_then(|s| s.trim().parse::<f64>().ok())
                .filter(|f| f.is_finite())
                .map_or(CellValue::Number(0.0), CellValue::Number)
        };

        let value = match self.value_type.as_deref() {
            Some("float" | "percentage" | "currency") => number(&self.value),
            Some("boolean") => {
                let raw = self.boolean_value.as_deref().unwrap_or(text).trim();
                CellValue::Bool(raw.eq_ignore_ascii_case("true") || raw == "1")
            }
            // A date and a time *are* Numbers — Part 4 §4.3.3 and §4.3.2 — counted from
            // `null_date`, so this is a conversion into the model's one numeric type and
            // not a second one. The kind rides alongside purely so the writer can spell the
            // cell back out as the date it was.
            //
            // Unparseable keeps the ISO text, which is the §9 rule: a value we cannot read
            // costs its cell a type, never the document. A `date` cell whose serial we do
            // have is never text again, so date arithmetic works on real files.
            Some("date") => {
                let parsed = self
                    .date_value
                    .as_deref()
                    .and_then(|s| date::parse_date(s, null_date));
                match parsed {
                    Some(serial) => return (CellValue::Number(serial), Some(NumberKind::Date)),
                    None => CellValue::Text(self.date_value.as_deref().unwrap_or(text).to_owned()),
                }
            }
            Some("time") => {
                let parsed = self.time_value.as_deref().and_then(date::parse_time);
                match parsed {
                    Some(fraction) => {
                        return (CellValue::Number(fraction), Some(NumberKind::Time));
                    }
                    None => CellValue::Text(self.time_value.as_deref().unwrap_or(text).to_owned()),
                }
            }
            Some("string" | "error") => {
                // Part 4 §4.6 stores an error result as a string — but LO writes the *empty*
                // string into `office:string-value` and leaves the error name in the display
                // text, flagging the cell with `calcext:value-type="error"`
                // (doc/ods-format.md §6). Trusting the attribute there turns every cached
                // error into an empty cell, so for those the paragraph is the value.
                if self.cached_error {
                    return (CellValue::Text(text.to_owned()), None);
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
        };
        (value, None)
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
        let (value, kind) = self.resolve(&text, b.doc.null_date);

        // Phase 4 parses this. Until then it is carried verbatim so a re-save does not
        // silently drop every formula in the document (§9).
        let formula = self.formula.clone();

        // R6: the element's full extent, resolved here because only now is it closed. Every
        // cell element the row has is recorded, empty ones included — writing a value into a
        // cell the file spells `<table:table-cell/>` is the case R6 is named for.
        if let Some((range, keep)) =
            self.span
                .take()
                .zip(b.doc.source.as_ref())
                .and_then(|(tag, source)| {
                    let keep = super::source::kept_attributes(source.bytes.get(tag.clone())?);
                    Some((cell_extent(&source.bytes, tag)?, keep))
                })
        {
            b.row_spans.push(super::source::Cell {
                range,
                cols: self.start..self.start.saturating_add(self.repeat).min(MAX_COLS),
                keep,
            });
        }

        if value.is_empty() && formula.is_none() {
            return; // Nothing to write; the columns were claimed at start.
        }
        // §5.1's indirection, resolved once per cell rather than once per repeat.
        let format = b.resolve_format(self.style.as_deref(), self.start);
        let style = b.resolve_style(self.style.as_deref(), self.start);
        for i in 0..self.repeat {
            let col = self.start.saturating_add(i);
            if col >= MAX_COLS {
                break;
            }
            b.row_cells.push(RowCell {
                col,
                value: value.clone(),
                formula: formula.clone(),
                kind,
                format: format.clone(),
                style: style.clone(),
            });
        }
    }
}

// --- styles (§5) -----------------------------------------------------------------------

/// `office:automatic-styles` and `office:styles`.
///
/// Three things in here are read: a `number:*-style`, which *is* a format; a `table-cell`
/// `style:style`, which carries both the link to a format and the cell's own look; and a
/// `table-column`/`table-row` one, of which only the size is kept (§5.4). Everything else a
/// style carries falls down the ignore path and is dropped rather than half-kept.
struct Styles;

impl Context<Builder> for Styles {
    fn end(&mut self, b: &mut Builder) {
        b.resolve_maps();
    }

    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "style") {
            let family = attrs.get(Ns::Style, "family").unwrap_or_default();
            let Some(name) = attrs.get(Ns::Style, "name").map(str::to_owned) else {
                return Some(Box::new(super::context::Ignore));
            };
            if let "table-column" | "table-row" = family {
                return Some(Box::new(TrackStyleProps { name }));
            }
            // Anything else — a `text` style inside a paragraph, a `table` style — has
            // nothing this model holds.
            if family != "table-cell" {
                return Some(Box::new(super::context::Ignore));
            }
            // ponytail: `style:parent-style-name` is not followed, so a cell style that
            // inherits its data style from a parent rather than naming one loses its format.
            // LibreOffice's own automatic styles always name it directly, which is why this
            // has not bitten; walk the parent chain when a file shows up that needs it.
            if let Some(data) = attrs.get(Ns::Style, "data-style-name") {
                b.cell_styles.insert(name.clone(), data.to_owned());
            }
            b.style_props.insert(name.clone(), CellStyle::default());
            return Some(Box::new(CellStyleProps { name }));
        }
        if name.ns != Ns::Number {
            return None;
        }
        let kind = match name.local.as_str() {
            "number-style" => Kind::Number,
            "percentage-style" => Kind::Percentage,
            "currency-style" => Kind::Currency,
            "date-style" => Kind::Date,
            "time-style" => Kind::Time,
            "boolean-style" => Kind::Boolean,
            "text-style" => Kind::Text,
            _ => return None,
        };
        b.parts.clear();
        // §5.2: the locale sits on the style, and decides the decimal and grouping
        // characters every `number:number` beneath it prints with.
        let locale = attrs.get(Ns::Number, "language").map(|language| {
            Locale::new(
                language,
                attrs.get(Ns::Number, "country").unwrap_or_default(),
            )
        });
        Some(Box::new(NumberStyle {
            name: attrs.get(Ns::Style, "name").unwrap_or_default().to_owned(),
            kind,
            locale,
        }))
    }
}

/// The property children of a `table-cell` `style:style` (§5.1).
///
/// Each one is written straight into the style being built rather than accumulated and
/// handed back, for the usual reason: a child has no channel to its parent, and the
/// [`Builder`] is the channel.
struct CellStyleProps {
    name: String,
}

impl Context<Builder> for CellStyleProps {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.ns != Ns::Style {
            return None;
        }
        let style = b.style_props.get_mut(&self.name)?;
        // Most of a style is `fo:`, but a few properties are `style:` — `vertical-align` is
        // the one here. Both spellings are asked for by meaning, never by prefix (§8.1).
        let fo = |local: &str| {
            attrs
                .get_any(&[(Ns::Fo, local), (Ns::Style, local)])
                .map(str::to_owned)
        };
        match name.local.as_str() {
            "table-cell-properties" => {
                style.background = fo("background-color");
                style.vertical_align = fo("vertical-align");
                style.wrap = fo("wrap-option");
                // The shorthand first, then each edge, so a document that writes both — and
                // LibreOffice does — ends with the specific one winning.
                style.set_border(fo("border"));
                for (i, edge) in EDGES.iter().enumerate() {
                    if let Some(value) = fo(&format!("border-{edge}")) {
                        style.borders[i] = Some(value);
                    }
                }
                // ODF spells "explicitly no border" as `none`; the model spells it as an
                // absent attribute, and keeping both would make two equal styles unequal.
                for edge in &mut style.borders {
                    if edge.as_deref() == Some("none") {
                        *edge = None;
                    }
                }
            }
            "text-properties" => {
                style.font_weight = fo("font-weight");
                style.font_style = fo("font-style");
                style.font_size = fo("font-size");
                style.color = fo("color");
            }
            "paragraph-properties" => style.align = fo("text-align"),
            _ => return None,
        }
        Some(Box::new(super::context::Ignore))
    }

    fn end(&mut self, b: &mut Builder) {
        // A style that sets nothing is no style: keeping it would have every cell in a
        // LibreOffice document carrying an empty one.
        if b.style_props
            .get(&self.name)
            .is_some_and(CellStyle::is_plain)
        {
            b.style_props.remove(&self.name);
        }
    }
}

/// The property child of a `table-column` or `table-row` `style:style` (§5.4).
///
/// One property each is kept — `style:column-width`, `style:row-height` — verbatim, for the
/// reason every other ODF length is kept verbatim (`style.rs`).
/// `style:use-optimal-column-width`/`-row-height` is deliberately dropped: it says the size
/// was *derived* from the content, and re-deriving it means measuring text, which the core
/// cannot do. The explicit size travels beside it in every file that sets it, so the layout
/// survives either way.
struct TrackStyleProps {
    name: String,
}

impl Context<Builder> for TrackStyleProps {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let size = match (name.ns, name.local.as_str()) {
            (Ns::Style, "table-column-properties") => attrs.get(Ns::Style, "column-width"),
            (Ns::Style, "table-row-properties") => attrs.get(Ns::Style, "row-height"),
            _ => return None,
        };
        if let Some(size) = size {
            b.track_sizes.insert(self.name.clone(), size.to_owned());
        }
        Some(Box::new(super::context::Ignore))
    }
}

/// One `number:*-style`: an ordered sequence of pieces (§5.2).
struct NumberStyle {
    name: String,
    kind: Kind,
    locale: Option<Locale>,
}

impl Context<Builder> for NumberStyle {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        // The conditional branch of a two-branch format (§5.1), and the only `style:`
        // element inside a data style.
        if name.is(Ns::Style, "map") {
            if let (Some(condition), Some(target)) = (
                attrs.get(Ns::Style, "condition"),
                attrs.get(Ns::Style, "apply-style-name"),
            ) && let Some((op, value)) = numfmt::parse_condition(condition)
            {
                b.pending_maps
                    .push((self.name.clone(), op, value, target.to_owned()));
            }
            return Some(Box::new(super::context::Ignore));
        }
        if name.ns != Ns::Number {
            return None;
        }
        let long = attrs.get(Ns::Number, "style") == Some("long");
        // The two pieces whose content is character data need a context of their own; every
        // other piece is entirely described by its attributes and is pushed right here.
        let part = match name.local.as_str() {
            "text" => return Some(Box::new(StyleText { currency: false })),
            "currency-symbol" => return Some(Box::new(StyleText { currency: true })),
            "number" => Part::Number {
                decimals: digits(attrs, "decimal-places", 2),
                min_decimals: digits(attrs, "min-decimal-places", 0),
                min_int: digits(attrs, "min-integer-digits", 1),
                grouping: attrs.get(Ns::Number, "grouping") == Some("true"),
            },
            "year" => Part::Year { long },
            "month" => Part::Month {
                long,
                textual: attrs.get(Ns::Number, "textual") == Some("true"),
            },
            "day" => Part::Day { long },
            "day-of-week" => Part::DayOfWeek { long },
            "hours" => Part::Hours { long },
            "minutes" => Part::Minutes { long },
            "seconds" => Part::Seconds {
                long,
                decimals: digits(attrs, "decimal-places", 0),
            },
            "am-pm" => Part::AmPm,
            "boolean" => Part::Boolean,
            "text-content" => Part::Content,
            _ => return None,
        };
        b.parts.push(part);
        Some(Box::new(super::context::Ignore))
    }

    fn end(&mut self, b: &mut Builder) {
        let format = Format {
            kind: self.kind,
            parts: std::mem::take(&mut b.parts),
            locale: self.locale.take(),
            maps: Vec::new(),
        };
        // The name is *not* taken: a `style:map` collected under this style refers to it by
        // name, and resolution happens after the section ends.
        b.number_styles.insert(self.name.clone(), format);
    }
}

/// A digit-count attribute. Not [`Attrs::count`], which clamps to at least one: zero
/// decimal places is both legal and common.
fn digits(attrs: &Attrs, local: &str, default: u8) -> u8 {
    attrs
        .get(Ns::Number, local)
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(default)
}

/// `number:text` and `number:currency-symbol` — the two pieces that *are* their content.
struct StyleText {
    currency: bool,
}

impl Context<Builder> for StyleText {
    fn text(&mut self, text: &str, b: &mut Builder) {
        b.style_text.push_str(text);
    }

    fn end(&mut self, b: &mut Builder) {
        let text = std::mem::take(&mut b.style_text);
        // An empty `number:text` says nothing, and keeping it would make a format's identity
        // depend on whether the writer spelled the element `<number:text/>` or left it out —
        // which is exactly the difference that stops a format round-tripping.
        if text.is_empty() && !self.currency {
            return;
        }
        b.parts.push(match self.currency {
            true => Part::Currency(text),
            false => Part::Text(text),
        });
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
