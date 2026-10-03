// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The file a document came from, kept so that saving can edit it instead of replacing it.
//!
//! This is R6 (doc/plan.md): *writing must change as little of the XML as possible*.
//! Regenerating from the model is right for a document this program authored and wrong for
//! one it opened — the diff is the whole file, and what the model does not carry in the body
//! goes with it. Editing one number should be one line of `git diff`, the way it is in the
//! `.fods` repositories this format is good for.
//!
//! **Retain and splice, not a fuller model.** Carrying every unknown element as a shadow
//! tree would grow the model to the size of ODF, which is the trade this project exists not
//! to make. Instead the reader keeps the bytes it read and remembers where each cell's
//! element sat in them; the writer serialises the cells that changed and drops them into
//! those exact ranges. Every other byte — indentation included — is the file that came in.
//!
//! **A repeated cell is split rather than skipped.** `table:number-columns-repeated` is not
//! an edge case in files LibreOffice writes — a row of five empty cells is one element, and
//! so is the trailing run of sixteen thousand — so treating one as unspliceable would leave
//! R6 true only for cells that already had a value. Writing into one re-emits *that element*
//! as the run before, the changed cell, and the run after, which is still a one-element diff.
//! [`Cell::cols`] is what makes that possible: the element knows which addresses it stands
//! for.
//!
//! Three things are deliberately *not* here, each an honest boundary of the trick rather
//! than a corner cut:
//!
//! * **A repeated *row* is not split.** One element there stands for many rows rather than
//!   many columns, so splitting means emitting whole rows and the diff stops being small —
//!   which is the entire point. Rows are recorded only where `table:number-rows-repeated` is
//!   absent or 1.
//! * **Only values and formulas splice cell by cell.** Anything else — a format or a style, a
//!   row's height, a column's width, a chart, a name, a sheet added or removed — is spliced *a
//!   row at a time* into the file's own table instead (`odf::write`'s `plan`): untouched rows
//!   are their own bytes, a repeated row is split around the rows that changed, a restyled cell
//!   keeps its own element with a new style name, and the table's start tag, its columns and
//!   its charts are rewritten only when they changed. [`Table`] is where all of that sits;
//!   `model::Provenance`, on each sheet, is what changed.
//! * **A package splices its `content.xml`.** [`Source::bytes`] is that part and
//!   [`Source::package`] the archive around it, rebuilt from its own entries on save.
//!
//! Every part outside `office:body` is the file's own either way
//! (`grind_core::odf::envelope::merge`). **Saving never makes an existing file worse**: what a
//! save still cannot carry — a chart regenerated after a rename, a value typed into the covered
//! half of a merge — makes it an `Error::WouldLose` rather than a smaller file, and every save
//! is read back and compared with the document before it replaces anything.

use std::collections::HashMap;
use std::ops::Range;

use super::write::Form;

/// The bytes a document was read from, plus where its cells are in them.
#[derive(Clone, Debug)]
pub struct Source {
    /// Which physical form those bytes are. Splicing is refused for any other, so a
    /// `.fods` opened and saved as `.ods` regenerates rather than producing a zip full of
    /// flat XML.
    pub form: Form,
    /// The file exactly as it was read. For the flat form this is also `content.xml`, which
    /// is why the ranges below index straight into it.
    pub bytes: Vec<u8>,
    /// The cell elements of each `(sheet, row)`, in column order.
    ///
    /// Per row rather than per cell because of the repeated element: keying by address would
    /// mean sixteen thousand identical entries for one trailing `<table:table-cell
    /// table:number-columns-repeated="16384"/>`, which is in most real files and in every
    /// row of some. A row's elements are few, so finding the one covering a column is a scan
    /// over a short list.
    pub rows: HashMap<(usize, u32), Vec<Cell>>,
    /// The whole archive, when the document came from a package — [`Source::bytes`] is then
    /// its `content.xml`. Every other entry is written back from here on save
    /// (`grind_core::odf::envelope::repackage`).
    pub package: Option<Vec<u8>>,
    /// The document's locale as the file stated it, so a save knows whether the default cell
    /// style the file already has needs its language changed — and leaves it alone otherwise.
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
    /// `null-date` and `null-year` as the file stated them, so `table:calculation-settings` —
    /// which also says whether criteria take wildcards, whether search is case-sensitive and
    /// a dozen other things the evaluator assumes — goes back as the file's own unless the
    /// epoch changed.
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
    /// Every attribute of the original element that the writer does **not** produce itself,
    /// spelled exactly as the file spelled it, ready to drop into a start tag.
    ///
    /// The load-bearing detail of the whole splice, and the reason it is the *attributes*
    /// rather than just the style name. `table:style-name="ce7"` has to survive because that
    /// style lives in a part of the document nothing here models — our pool would emit `ce0`
    /// and point the cell at a style the file does not contain. But so does
    /// `table:number-columns-spanned`: writing a number into a merged cell and silently
    /// un-merging it is a worse bug than a large diff, and it is what re-deriving the whole
    /// start tag does. See [`kept_attributes`] for what is dropped and why.
    pub keep: String,
}

/// The attributes of a cell's start tag, minus the ones the writer produces from the model.
///
/// Verbatim, by slicing rather than by re-serialising: `Attrs` resolves prefixes to
/// namespaces, so rebuilding from it would spell a document's own attributes in *our*
/// prefixes and turn a one-element diff back into a whole-file one.
///
/// Two groups are dropped. The first is what the writer always emits — the value, its type,
/// the formula, the repeat count — which would otherwise appear twice and make the element
/// ill-formed. The second is what *describes* that value and is no longer true once it
/// changes: `office:currency`, and the `calcext:` mirror of the value type that LibreOffice
/// writes (R4 — allowed, but not something to carry forward onto a value it no longer
/// matches).
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
