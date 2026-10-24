// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The file a document came from, kept so that saving can edit it instead of replacing it (R6):
//! editing one number should be one line of `git diff`.
//!
//! The reader keeps the bytes and where each cell's element sat in them; the writer serialises the
//! cells that changed into those exact ranges, and every other byte is the file's own. A repeated
//! cell (`table:number-columns-repeated`) is split into the run before, the changed cell and the
//! run after ([`Cell::cols`]), still a one-element diff.
//!
//! * A repeated row is not split cell by cell; rows are recorded only where
//!   `table:number-rows-repeated` is absent or 1.
//! * Only values and formulas splice per cell. Formats, styles, track sizes, charts, names and
//!   sheets splice a row at a time into the file's own table (`odf::write`'s `plan`): untouched
//!   rows keep their bytes, a repeated row is split around the changed ones, and the table's start
//!   tag, columns and charts are rewritten only when they changed. [`Table`] holds the ranges;
//!   `model::Provenance` on each sheet says what changed.
//! * A package splices its `content.xml` ([`Source::bytes`]) and rebuilds the archive from its own
//!   entries ([`Source::package`]).
//!
//! Everything outside `office:body` is the file's own (`grind_core::odf::envelope::merge`). What a
//! save still cannot carry is an `Error::WouldLose`, and every save is read back and compared
//! before it replaces anything.

use std::collections::HashMap;
use std::ops::Range;

use super::write::Form;

/// The bytes a document was read from, plus where its cells are in them.
#[derive(Clone, Debug)]
pub struct Source {
    /// Which physical form those bytes are. Splicing is refused for any other, so a `.fods` saved
    /// as `.ods` regenerates.
    pub form: Form,
    /// The file exactly as it was read. For the flat form this is also `content.xml`, which
    /// is why the ranges below index straight into it.
    pub bytes: Vec<u8>,
    /// The cell elements of each `(sheet, row)`, in column order — per row, since keying by
    /// address would mean sixteen thousand entries for one trailing repeated cell.
    pub rows: HashMap<(usize, u32), Vec<Cell>>,
    /// The whole archive, when the document came from a package — [`Source::bytes`] is then
    /// its `content.xml`. Every other entry is written back from here on save
    /// (`grind_core::odf::envelope::repackage`).
    pub package: Option<Vec<u8>>,
    /// The document's locale as the file stated it, so a save changes the default cell style's
    /// language only when it changed.
    pub locale: Option<crate::locale::Locale>,
    /// The package directories holding the charts the model read (`Object 1`, …): what a
    /// regenerating save replaces, every other directory being kept.
    pub chart_parts: Vec<String>,
    /// Every `table:table` of the file, in file order — what a regenerating save splices a
    /// sheet back into (`model::Provenance::table` is the index here).
    pub tables: Vec<Table>,
    /// The children of `office:spreadsheet` that are not tables, each with what it is: the
    /// prelude ahead of the first table and the epilogue after the last, carried verbatim
    /// unless the model owns and changed them.
    pub parts: Vec<Part>,
    /// `null-date` and `null-year` as the file stated them, so `table:calculation-settings`
    /// (wildcards, case sensitivity and more) goes back as the file's own unless the epoch
    /// changed.
    pub epoch: (i64, i64),
    /// The content of `office:spreadsheet` — between its start tag and its end tag.
    pub body: Option<Range<usize>>,
}

/// One `table:table` of the file, and where everything a save may replace sits inside it.
#[derive(Clone, Debug, Default)]
pub struct Table {
    /// The whole element.
    pub range: Range<usize>,
    /// Its start tag.
    pub start: Range<usize>,
    /// The start tag's attributes other than `table:name`, verbatim — the table's style (and
    /// with it, its master page), print ranges, protection.
    pub keep: String,
    /// `table:shapes` — where the sheet's charts are.
    pub shapes: Option<Range<usize>>,
    /// `office:forms` — the sheet's form controls, its checkboxes among them.
    pub forms: Option<Range<usize>>,
    /// Each `form:checkbox` element in it, by its identifier.
    pub checkbox_elements: HashMap<String, Range<usize>>,
    /// Where a new checkbox goes: just before the end tag of the first `form:form`.
    pub form_end: Option<usize>,
    /// Every checkbox as it was read — the cell it was placed in, its identifier and what it
    /// said — so a save copies an unchanged one's own element and changes only the rest.
    pub checkboxes: std::collections::BTreeMap<crate::model::Pos, (String, crate::model::Checkbox)>,
    /// The column declarations: every `table:table-column` and column group directly in the
    /// table, in order.
    pub columns: Vec<Range<usize>>,
    /// Every row element, wherever it sits (inside a header-row or row group too), in order.
    pub rows: Vec<RowElement>,
    /// Sheet-local `table:named-expressions`, and the names in it.
    pub names: Option<(Range<usize>, Vec<String>)>,
    /// `table:default-cell-style-name` by column run, as declared — kept by a save that
    /// rewrites the column declarations, since the model does not carry it.
    pub column_defaults: Vec<(Range<u32>, Option<String>)>,
    /// LibreOffice's own copy of the sheet's conditional formats
    /// (`calcext:conditional-formats`, `doc/ods-format.md` §3.6) — stale once a save changes
    /// the rules, when the conditions the model reads are taken out of it.
    pub conditional_formats: Option<Range<usize>>,
    /// The package directories of this table's charts (`Object 1`, …) — dropped from the
    /// package only by a save that writes this table's charts again.
    pub charts: Vec<String>,
}

impl Table {
    /// The default cell style the file declared for `col`.
    pub fn column_default(&self, col: u32) -> Option<&str> {
        self.column_defaults
            .iter()
            .find(|(cols, _)| cols.contains(&col))
            .and_then(|(_, style)| style.as_deref())
    }
}

/// One `table:table-row` of the file.
#[derive(Clone, Debug)]
pub struct RowElement {
    /// The whole element.
    pub range: Range<usize>,
    /// Its start tag.
    pub start: Range<usize>,
    /// The first row it stands for, and how many (`table:number-rows-repeated`).
    pub first: u32,
    pub repeat: u32,
}

/// What a non-table child of `office:spreadsheet` is, as far as a save cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    CalculationSettings,
    NamedExpressions,
    DatabaseRanges,
    /// Anything else — content validations, label ranges, a data pilot, a vendor's own.
    Other,
}

/// One non-table child of `office:spreadsheet`.
#[derive(Clone, Debug)]
pub struct Part {
    pub kind: PartKind,
    pub range: Range<usize>,
    /// Whether it came after the last table.
    pub after_tables: bool,
}

/// One cell element of the source file.
#[derive(Clone, Debug)]
pub struct Cell {
    /// The element's extent in [`Source::bytes`], start tag through end tag.
    pub range: Range<usize>,
    /// The columns it stands for — one column, or the run a
    /// `table:number-columns-repeated` covers.
    pub cols: Range<u32>,
    /// Every attribute of the original element the writer does not produce itself, spelled as the
    /// file spelled it. `table:style-name="ce7"` must survive because the style lives in a part
    /// nothing here models, and `table:number-columns-spanned` because writing a number must not
    /// un-merge a cell. See [`kept_attributes`].
    pub keep: String,
}

/// The attributes of a cell's start tag minus the ones the writer produces from the model, sliced
/// verbatim rather than re-serialised (which would respell them in our prefixes).
///
/// Dropped: what the writer always emits (value, type, formula, repeat count), and what describes
/// a value that is no longer true once it changes (`office:currency`, LibreOffice's `calcext:`
/// mirror of the value type).
pub fn kept_attributes(start_tag: &[u8]) -> String {
    attributes(start_tag, &DROP)
}

/// What [`kept_attributes`] drops: what the writer always emits, and what describes a value
/// that is no longer the cell's.
pub const DROP: [&str; 10] = [
    "office:value-type",
    "office:value",
    "office:date-value",
    "office:time-value",
    "office:boolean-value",
    "office:string-value",
    "office:currency",
    "table:formula",
    "table:number-columns-repeated",
    "calcext:value-type",
];

/// Every attribute of a start tag except those named in `drop`, spelled as the file spelled
/// them.
pub fn attributes(start_tag: &[u8], drop: &[&str]) -> String {
    let Ok(tag) = std::str::from_utf8(start_tag) else {
        return String::new();
    };
    // Past `<table:table-cell`, and stopping before the `/>` or `>` that closes it.
    let Some(body) = tag.find(char::is_whitespace).map(|i| &tag[i..]) else {
        return String::new();
    };
    let body = body.trim_end_matches('>').trim_end_matches('/');

    let mut out = String::new();
    let mut rest = body;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim();
        let after = &rest[eq + 1..];
        // An attribute value is quoted, and cannot contain its own quote character — so the
        // next one of the same kind ends it, `>` and `<` inside notwithstanding.
        let Some(quote) = after.chars().find(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let Some(open) = after.find(quote) else { break };
        let Some(len) = after[open + 1..].find(quote) else {
            break;
        };
        let end = open + 1 + len + 1;
        if !name.is_empty() && !drop.contains(&name) {
            out.push(' ');
            out.push_str(name);
            out.push('=');
            out.push_str(&after[open..end]);
        }
        rest = &after[end..];
    }
    out
}

impl Source {
    pub fn new(form: Form, bytes: Vec<u8>) -> Self {
        Self {
            form,
            bytes,
            rows: HashMap::new(),
            package: None,
            locale: None,
            chart_parts: Vec::new(),
            tables: Vec::new(),
            parts: Vec::new(),
            epoch: (0, 0),
            body: None,
        }
    }

    /// The element covering `col` of this row, if the file spelled one.
    pub fn covering(&self, sheet: usize, row: u32, col: u32) -> Option<&Cell> {
        self.rows
            .get(&(sheet, row))?
            .iter()
            .find(|c| c.cols.contains(&col))
    }
}
