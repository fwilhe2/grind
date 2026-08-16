// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Serialising a document back to ODF. **[ODS]**, with `package` below it **[GENERIC]**.
//!
//! Templates from doc/ods-format.md §7; the package layout from §1.1/§1.3. The two forms
//! share one content writer and differ in exactly two places — the root element name and
//! whether `office:mimetype` sits on it (§7.3) — so there is no second serialiser to keep
//! in step.
//!
//! The output is deliberately minimal (§1.4): no `styles.xml`, no `meta.xml` and no
//! `settings.xml`, because there is nothing yet to put in them. `office:automatic-styles`
//! is written when — and only when — some cell carries a number format, and it holds
//! exactly the formats in use: §5.3's pooling rule is not an optimisation here, it is the
//! only construct ODF has for saying a cell looks a certain way.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::{Cursor, Write as _};
use std::sync::LazyLock;

use super::names::{FO, NUMBER, OFFICE, STYLE, TABLE, TEXT};
use crate::formula::date;
use crate::model::{CellValue, Document, NumberKind, Pos, Sheet};
use crate::numfmt::{self, Format, Kind, Part};
use crate::style::{CellStyle, EDGES};
use crate::{Error, Result};

/// The media type, byte for byte. Sniffed by readers at a fixed offset in the package
/// form (§1.1), so it is not somewhere to be creative.
pub const MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";

const VERSION: &str = "1.4";

/// Which physical form to write.
///
/// Reading sniffs the form from the bytes, because an extension is a hint rather than a
/// fact. Writing has to *choose* one, so this is the only place the distinction is an
/// input rather than an observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `.ods` — a zip package.
    Package,
    /// `.fods` — one flat XML file.
    Flat,
}

pub fn write(doc: &Document, form: Form) -> Result<Vec<u8>> {
    // R6 first: a document that came from a file and has only had cells edited goes back as
    // that file with those cells replaced. Everything else regenerates, which is always
    // correct and is what this did before splicing existed.
    if let Some(spliced) = splice(doc, form) {
        return Ok(spliced);
    }
    match form {
        Form::Flat => Ok(content(doc, form).into_bytes()),
        Form::Package => package(doc),
    }
}

/// The file this document was read from, with the edited cells put back in place.
///
/// `None` means "not applicable, regenerate" — never "failed". Every condition below is a
/// documented boundary of the trick rather than an error, and `odf::source` says why each
/// one is where it is.
fn splice(doc: &Document, form: Form) -> Option<Vec<u8>> {
    let source = doc.source.as_deref()?;
    // Saving as the other form is a conversion, not an edit.
    if source.form != form || !doc.edits.only_values {
        return None;
    }

    // Which elements have to be rewritten. Every edited cell must sit in one the file
    // actually spelled — one that does not means regenerating, because a document half in
    // its original bytes and half not would lose the other half silently.
    //
    // Keyed by element rather than by cell: several edited cells can share one repeated
    // element, and rewriting it once from the sheet covers all of them.
    let mut targets: BTreeMap<usize, (usize, u32, &super::source::Cell)> = BTreeMap::new();
    for (i, pos) in &doc.edits.cells {
        let at = source.covering(*i, pos.row, pos.col)?;
        doc.sheet(*i)?;
        targets.insert(at.range.start, (*i, pos.row, at));
    }

    // In file order, so the untouched stretches between are copied without seeking back.
    // Elements do not overlap by construction — they are siblings — but a corrupted span
    // would produce tangled bytes rather than an error, so refuse instead of trusting it.
    let mut patches = Vec::with_capacity(targets.len());
    for (i, row, at) in targets.into_values() {
        let sheet = doc.sheet(i)?;
        patches.push((at.range.clone(), rewrite(sheet, row, at, doc.null_date)));
    }
    if patches.windows(2).any(|w| w[0].0.end > w[1].0.start) {
        return None;
    }

    let mut out = Vec::with_capacity(source.bytes.len());
    let mut at = 0usize;
    for (range, text) in patches {
        out.extend_from_slice(source.bytes.get(at..range.start)?);
        out.extend_from_slice(text.as_bytes());
        at = range.end;
    }
    out.extend_from_slice(source.bytes.get(at..)?);
    Some(out)
}

/// One source element, re-emitted from the sheet's current contents.
///
/// Usually one cell in and one cell out. The interesting case is the repeated element: a
/// `table:number-columns-repeated="5"` covering five empty cells, with a value now written
/// into the middle one, comes back as *three* elements — the run before, the changed cell,
/// the run after — which is still a one-line diff, and is what keeps R6 true for the
/// overwhelmingly common case of writing into a cell that had no value.
///
/// Runs are re-formed by looking at the sheet, so a value written into a repeated run and
/// then cleared again collapses the run back. A formula never joins a run: it is
/// position-dependent, and repeating one would move it.
fn rewrite(sheet: &Sheet, row: u32, at: &super::source::Cell, null_date: i64) -> String {
    let mut out = String::new();
    let mut col = at.cols.start;
    while col < at.cols.end {
        let pos = Pos::new(row, col);
        let (value, formula, kind) = (sheet.get(pos), sheet.formula(pos), sheet.kind(pos));
        let repeat = match formula.is_some() {
            true => 1,
            false => (col..at.cols.end)
                .take_while(|c| {
                    let p = Pos::new(row, *c);
                    sheet.get(p) == value && sheet.formula(p).is_none() && sheet.kind(p) == kind
                })
                .count() as u32,
        };
        cell(
            &mut out,
            &value,
            formula,
            kind,
            (effective(sheet, pos), sheet.style(pos)),
            null_date,
            repeat,
            // The element's own unmanaged attributes, verbatim — its style name, its merge
            // spans. They applied to every column it covered, so every piece it splits into
            // keeps them.
            &at.keep,
        );
        col += repeat.max(1);
    }
    out
}

/// The `content.xml` payload, which in the flat form is the whole document (§7.1–7.3).
fn content(doc: &Document, form: Form) -> String {
    let root = match form {
        Form::Flat => "office:document",
        Form::Package => "office:document-content",
    };

    let pool = Pool::new(doc);
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    // Declare only the namespaces this part actually uses (§1.4). `table:formula`'s `of:`
    // prefix is part of the *string*, not a namespace-resolved name (§4), so no `xmlns:of`.
    let _ = write!(
        out,
        "<{root} xmlns:office=\"{OFFICE}\" xmlns:table=\"{TABLE}\" xmlns:text=\"{TEXT}\""
    );
    // The two style namespaces appear only in a document that has styles (§1.4).
    if !pool.is_empty() {
        let _ = write!(
            out,
            " xmlns:style=\"{STYLE}\" xmlns:number=\"{NUMBER}\" xmlns:fo=\"{FO}\""
        );
    }
    let _ = write!(out, " office:version=\"{VERSION}\"");
    if form == Form::Flat {
        let _ = write!(out, " office:mimetype=\"{MIMETYPE}\"");
    }
    out.push_str(">\n");
    // Before the body, which is where the schema puts it.
    if !pool.is_empty() {
        pool.write(&mut out);
    }
    out.push_str(" <office:body>\n  <office:spreadsheet>\n");
    // The epoch, and only when it is not the default — writing the default would be
    // correct but LibreOffice omits it, and matching that keeps our output diffable
    // against a file it wrote. First in the element, which is where the schema puts it.
    if doc.null_date != date::DEFAULT_NULL_DATE || doc.null_year != date::DEFAULT_NULL_YEAR {
        let year = match doc.null_year != date::DEFAULT_NULL_YEAR {
            true => format!(" table:null-year=\"{}\"", doc.null_year),
            false => String::new(),
        };
        let _ = writeln!(
            out,
            "   <table:calculation-settings{year}><table:null-date table:value-type=\"date\" \
             table:date-value=\"{}\"/></table:calculation-settings>",
            date::format_date(0.0, doc.null_date)
        );
    }
    for sheet in &doc.sheets {
        table(&mut out, sheet, doc.null_date, &pool);
    }
    // §5.11, and *after* the tables: the schema's `office-spreadsheet-content-epilogue`
    // (line 8263) is where `table-functions` sits, and `table:named-expressions` is its
    // first member. LibreOffice reads them in either position, so loop C cannot see this —
    // only the RELAX NG schema can, which is what `kb.rs` validates against. Every name is
    // written as `table:named-expression`, since the reader stores a named range as the
    // reference it stands for and the two forms are interchangeable on the way out.
    if !doc.names.is_empty() {
        out.push_str("   <table:named-expressions>\n");
        for (name, expression) in &doc.names {
            let _ = writeln!(
                out,
                "    <table:named-expression table:name=\"{}\" table:expression=\"{}\"/>",
                esc(name),
                esc(expression)
            );
        }
        out.push_str("   </table:named-expressions>\n");
    }
    let _ = write!(out, "  </office:spreadsheet>\n </office:body>\n</{root}>\n");
    out
}

/// The format a date cell gets when the document gives it none.
///
/// Not decoration: LibreOffice *requires* a date cell to display through a date style and
/// invents one from its own locale when a file omits it, so a document written without one
/// comes back with `M/D/YY` bolted on and the round trip is no longer an identity. Writing
/// the ISO spelling instead makes the file say what it means, in the one form that reads
/// the same in every locale (§3.4 Note 2).
static DATE_DEFAULT: LazyLock<Format> =
    LazyLock::new(|| numfmt::preset(Kind::Date, 0, false, ""));

/// A date that carries a time is a DateTime (§4.3.4) and needs a style that shows both,
/// since a format cannot look at the value it is given.
static DATETIME_DEFAULT: LazyLock<Format> = LazyLock::new(numfmt::datetime_preset);

/// The same for a time cell — a 24-hour clock, for the same reason.
static TIME_DEFAULT: LazyLock<Format> =
    LazyLock::new(|| numfmt::preset(Kind::Time, 0, false, ""));

/// The format a cell is actually written with: its own, or the default its value type
/// demands.
fn effective(sheet: &Sheet, pos: Pos) -> Option<&Format> {
    if let Some(format) = sheet.format(pos) {
        return Some(format);
    }
    match sheet.kind(pos) {
        Some(NumberKind::Date) => match sheet.get(pos) {
            CellValue::Number(n) if n.fract() != 0.0 => Some(&DATETIME_DEFAULT),
            _ => Some(&DATE_DEFAULT),
        },
        Some(NumberKind::Time) => Some(&TIME_DEFAULT),
        None => None,
    }
}

/// How one cell is written: its number format and its styling, which travel together on a
/// single `style:style` and are therefore pooled together (§5.3). A cell with neither gets
/// no `table:style-name` at all.
type Look<'a> = (Option<&'a Format>, Option<&'a CellStyle>);

/// Every distinct format and every distinct look in the document, in first-seen order.
///
/// Two pools, because the file has two vocabularies: format *i* is a `number:*-style` named
/// `N{i}`, look *i* is a `table-cell` `style:style` named `ce{i}` that points at one and
/// carries the properties. Every cell that shares a look shares its name.
struct Pool<'a> {
    formats: Vec<&'a Format>,
    index: HashMap<&'a Format, usize>,
    looks: Vec<Look<'a>>,
    look_index: HashMap<Look<'a>, usize>,
}

impl<'a> Pool<'a> {
    fn new(doc: &'a Document) -> Self {
        let mut pool = Pool {
            formats: Vec::new(),
            index: HashMap::new(),
            looks: Vec::new(),
            look_index: HashMap::new(),
        };
        for sheet in &doc.sheets {
            let formatted = sheet.formats().map(|(pos, _)| pos);
            let dated = sheet.kinds().map(|(pos, _)| pos);
            let styled = sheet.styles().map(|(pos, _)| pos);
            for pos in formatted.chain(dated).chain(styled) {
                let look = (effective(sheet, pos), sheet.style(pos));
                if let Some(format) = look.0 {
                    pool.add(format);
                }
                if look.1.is_none() && look.0.is_none() {
                    continue;
                }
                if !pool.look_index.contains_key(&look) {
                    pool.look_index.insert(look, pool.looks.len());
                    pool.looks.push(look);
                }
            }
        }
        pool
    }

    /// Pool a format, and the target of every `style:map` it carries — a branch is a style
    /// in its own right in the file, referenced by name from the map (§5.1).
    fn add(&mut self, format: &'a Format) {
        // `insert` would *overwrite* the index of a format already pooled, quietly pointing
        // its first cells at a later style.
        if self.index.contains_key(format) {
            return;
        }
        self.index.insert(format, self.formats.len());
        self.formats.push(format);
        for map in &format.maps {
            self.add(&map.format);
        }
    }

    /// ` table:style-name="ce3"`, or nothing when the cell has neither format nor styling.
    fn attr(&self, look: Look) -> String {
        match self.look_index.get(&look) {
            Some(i) => format!(" table:style-name=\"ce{i}\""),
            None => String::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.formats.is_empty() && self.looks.is_empty()
    }

    /// `office:automatic-styles`: each format once, and one cell style per format to point
    /// at it — the indirection is mandatory, there is no way to put a format on a cell
    /// directly (§5.3).
    fn write(&self, out: &mut String) {
        out.push_str(" <office:automatic-styles>\n");
        for (i, format) in self.formats.iter().enumerate() {
            let _ = writeln!(out, "  {}", data_style(format, i, self));
        }
        for (i, (format, style)) in self.looks.iter().enumerate() {
            let data = match format.and_then(|f| self.index.get(f)) {
                Some(n) => format!(" style:data-style-name=\"N{n}\""),
                None => String::new(),
            };
            let _ = write!(
                out,
                "  <style:style style:name=\"ce{i}\" style:family=\"table-cell\"{data}"
            );
            match style {
                Some(style) => {
                    let _ = writeln!(out, ">{}</style:style>", properties(style));
                }
                None => out.push_str("/>\n"),
            }
        }
        out.push_str(" </office:automatic-styles>\n");
    }
}

/// The property children of a cell style (§5.1), in the order the schema declares them.
///
/// Every value goes out exactly as it came in. LibreOffice will re-quantise a border width
/// and rewrite a font reference (§5.4) — that is its normalisation, not ours to anticipate.
fn properties(style: &CellStyle) -> String {
    let mut out = String::new();
    let attr = |out: &mut String, name: &str, value: &Option<String>| {
        if let Some(value) = value {
            let _ = write!(out, " {name}=\"{}\"", esc(value));
        }
    };

    let mut cell = String::new();
    attr(&mut cell, "fo:background-color", &style.background);
    attr(&mut cell, "style:vertical-align", &style.vertical_align);
    attr(&mut cell, "fo:wrap-option", &style.wrap);
    // The shorthand when every edge agrees, four attributes when they do not — the two
    // spell the same style, and emitting both would be a document contradicting itself.
    match style.uniform_border() {
        Some(border) => {
            let _ = write!(cell, " fo:border=\"{}\"", esc(border));
        }
        None => {
            for (i, edge) in EDGES.iter().enumerate() {
                attr(&mut cell, &format!("fo:border-{edge}"), &style.borders[i]);
            }
        }
    }
    if !cell.is_empty() {
        let _ = write!(out, "<style:table-cell-properties{cell}/>");
    }

    if let Some(align) = &style.align {
        let _ = write!(
            out,
            "<style:paragraph-properties fo:text-align=\"{}\"/>",
            esc(align)
        );
    }

    let mut text = String::new();
    attr(&mut text, "fo:font-weight", &style.font_weight);
    attr(&mut text, "fo:font-style", &style.font_style);
    attr(&mut text, "fo:font-size", &style.font_size);
    attr(&mut text, "fo:color", &style.color);
    if !text.is_empty() {
        let _ = write!(out, "<style:text-properties{text}/>");
    }
    out
}

/// One `number:*-style` (§5.2) — the exact inverse of what `read`'s `NumberStyle` builds.
fn data_style(format: &Format, i: usize, pool: &Pool) -> String {
    let element = match format.kind {
        Kind::Number => "number:number-style",
        Kind::Percentage => "number:percentage-style",
        Kind::Currency => "number:currency-style",
        Kind::Date => "number:date-style",
        Kind::Time => "number:time-style",
        Kind::Boolean => "number:boolean-style",
        Kind::Text => "number:text-style",
    };
    let locale = match &format.locale {
        Some(locale) if locale.country.is_empty() => {
            format!(" number:language=\"{}\"", esc(&locale.language))
        }
        Some(locale) => format!(
            " number:language=\"{}\" number:country=\"{}\"",
            esc(&locale.language),
            esc(&locale.country)
        ),
        None => String::new(),
    };
    let mut out = format!("<{element} style:name=\"N{i}\"{locale}>");
    for part in &format.parts {
        // `number:style="long"` is the spec's spelling of "padded"; short is the default and
        // is written by omission, which is what LibreOffice does too.
        let long = |long: &bool| match long {
            true => " number:style=\"long\"",
            false => "",
        };
        match part {
            Part::Text(text) => {
                let _ = write!(out, "<number:text>{}</number:text>", esc(text));
            }
            Part::Currency(symbol) => {
                let _ = write!(
                    out,
                    "<number:currency-symbol>{}</number:currency-symbol>",
                    esc(symbol)
                );
            }
            Part::Number {
                decimals,
                min_decimals,
                min_int,
                grouping,
            } => {
                let _ = write!(
                    out,
                    "<number:number number:decimal-places=\"{decimals}\" \
                     number:min-decimal-places=\"{min_decimals}\" \
                     number:min-integer-digits=\"{min_int}\"{}/>",
                    match grouping {
                        true => " number:grouping=\"true\"",
                        false => "",
                    }
                );
            }
            Part::Year { long: l } => {
                let _ = write!(out, "<number:year{}/>", long(l));
            }
            Part::Month { long: l, textual } => {
                let _ = write!(
                    out,
                    "<number:month{}{}/>",
                    long(l),
                    match textual {
                        true => " number:textual=\"true\"",
                        false => "",
                    }
                );
            }
            Part::Day { long: l } => {
                let _ = write!(out, "<number:day{}/>", long(l));
            }
            Part::DayOfWeek { long: l } => {
                let _ = write!(out, "<number:day-of-week{}/>", long(l));
            }
            Part::Hours { long: l } => {
                let _ = write!(out, "<number:hours{}/>", long(l));
            }
            Part::Minutes { long: l } => {
                let _ = write!(out, "<number:minutes{}/>", long(l));
            }
            Part::Seconds { long: l, decimals } => {
                let _ = write!(
                    out,
                    "<number:seconds{} number:decimal-places=\"{decimals}\"/>",
                    long(l)
                );
            }
            Part::AmPm => out.push_str("<number:am-pm/>"),
            Part::Boolean => out.push_str("<number:boolean/>"),
            Part::Content => out.push_str("<number:text-content/>"),
        }
    }
    // The branches last, which is where the schema puts them, and by the name the pool gave
    // the target — a map whose target is not pooled cannot happen, since `Pool::add` walks
    // them, but a missing one is skipped rather than written as a dangling reference.
    for map in &format.maps {
        let Some(target) = pool.index.get(&map.format) else {
            continue;
        };
        let _ = write!(
            out,
            "<style:map style:condition=\"{}\" style:apply-style-name=\"N{target}\"/>",
            esc(&format!("value(){}{}", map.op.spelling(), map.value))
        );
    }
    let _ = write!(out, "</{element}>");
    out
}

fn table(out: &mut String, sheet: &Sheet, null_date: i64, pool: &Pool) {
    let cols = sheet.used_cols().max(1);
    let rows = sheet.used_rows();

    let _ = writeln!(out, "   <table:table table:name=\"{}\">", esc(&sheet.name));
    // Both the column block and the row block are mandatory, even for an all-empty sheet
    // (§3.2), which is why everything here has a `.max(1)` behind it.
    let _ = writeln!(out, "    <table:table-column{}/>", count(cols, "columns"));

    if rows == 0 {
        out.push_str("    <table:table-row><table:table-cell/></table:table-row>\n");
    }

    let mut row = 0;
    while row < rows {
        // Interior blank rows collapse into one repeated row (§3.3) — the main file-size
        // lever, and the difference between a 20-row sheet and 20 rows plus a megabyte of
        // nothing after one stray edit at row 50 000.
        let blank = (row..rows)
            .take_while(|r| is_blank(sheet, *r, cols))
            .count() as u32;
        if blank > 0 {
            let _ = writeln!(
                out,
                "    <table:table-row{}><table:table-cell/></table:table-row>",
                count(blank, "rows")
            );
            row += blank;
            continue;
        }
        write_row(out, sheet, row, cols, null_date, pool);
        row += 1;
    }

    out.push_str("   </table:table>\n");
}

fn is_blank(sheet: &Sheet, row: u32, cols: u32) -> bool {
    (0..cols).all(|col| {
        let pos = Pos::new(row, col);
        sheet.get(pos).is_empty() && sheet.formula(pos).is_none()
    })
}

fn write_row(
    out: &mut String,
    sheet: &Sheet,
    row: u32,
    cols: u32,
    null_date: i64,
    pool: &Pool,
) {
    out.push_str("    <table:table-row>");
    // Trailing empty cells are simply not written: unmentioned is the same as empty
    // (§3.3), and the row is known non-blank so at least one cell survives.
    let last = (0..cols)
        .rposition(|col| {
            let pos = Pos::new(row, col);
            !sheet.get(pos).is_empty() || sheet.formula(pos).is_some()
        })
        .map_or(0, |c| c as u32);

    let mut col = 0;
    while col <= last {
        let pos = Pos::new(row, col);
        let value = sheet.get(pos);
        let formula = sheet.formula(pos);
        let look = (effective(sheet, pos), sheet.style(pos));
        // Only blank runs are compressed. Repeating a *valued* cell would need the formula
        // to repeat with it, and a formula is position-dependent — a correctness trap for
        // bytes nobody is short of. A run of blanks sharing one format compresses like any
        // other; one that does not share it stops the run, or the format would spread.
        let repeat = if value.is_empty() && formula.is_none() {
            (col..=last)
                .take_while(|c| {
                    let p = Pos::new(row, *c);
                    sheet.get(p).is_empty()
                        && sheet.formula(p).is_none()
                        && (effective(sheet, p), sheet.style(p)) == look
                })
                .count() as u32
        } else {
            1
        };
        cell(
            out,
            &value,
            formula,
            sheet.kind(pos),
            look,
            null_date,
            repeat,
            &pool.attr(look),
        );
        col += repeat;
    }
    out.push_str("</table:table-row>\n");
}

#[allow(clippy::too_many_arguments)]
fn cell(
    out: &mut String,
    value: &CellValue,
    formula: Option<&str>,
    kind: Option<NumberKind>,
    look: Look,
    null_date: i64,
    repeat: u32,
    style_attr: &str,
) {
    let format = look.0;
    let mut attrs = count(repeat, "columns");
    attrs.push_str(style_attr);
    if let Some(f) = formula {
        let _ = write!(attrs, " table:formula=\"{}\"", esc(f));
    }

    // The cached result travels with the formula, never instead of it: an omitted cached
    // value is schema-legal and renders blank until the next recalculation (§4).
    match value {
        // A bare cell is a valid, correctly typed empty cell (§3.4).
        CellValue::Empty => {
            let _ = write!(out, "<table:table-cell{attrs}/>");
        }
        CellValue::Number(n) => {
            // The reader maps a non-finite `office:value` to zero (§9); agreeing here keeps
            // read(write(d)) == d for every document the reader can produce, and xsd:double's
            // `INF`/`NaN` spellings are not worth the interop risk for a value no ODF
            // document should be carrying in the first place.
            let n = if n.is_finite() { *n } else { 0.0 };
            // A date and a time are Numbers (§4.3.3, §4.3.2) that were *written* as a
            // calendar date or a clock, and go back out the way they came in. The kind is
            // consulted only here, so a cell whose date was overwritten with text or a
            // boolean cannot carry a stale one out — no side table to keep in step.
            let typed = match kind {
                Some(NumberKind::Date) => format!(
                    "office:value-type=\"date\" office:date-value=\"{}\"",
                    date::format_date(n, null_date)
                ),
                Some(NumberKind::Time) => format!(
                    "office:value-type=\"time\" office:time-value=\"{}\"",
                    date::format_time(n)
                ),
                None => format!("office:value-type=\"float\" office:value=\"{n}\""),
            };
            display(out, &attrs, &typed, value, format, null_date);
        }
        CellValue::Bool(b) => {
            let typed = format!("office:value-type=\"boolean\" office:boolean-value=\"{b}\"");
            display(out, &attrs, &typed, value, format, null_date);
        }
        CellValue::Text(s) => {
            // Carried as paragraphs rather than `office:string-value`, which is what LO
            // itself writes and — the load-bearing reason — is the only form that survives
            // a newline. XML normalises a literal newline in an attribute value to a space,
            // so a multi-line string in `office:string-value` comes back mangled.
            let _ = write!(
                out,
                "<table:table-cell{attrs} office:value-type=\"string\">"
            );
            for line in s.split('\n') {
                if line.is_empty() {
                    out.push_str("<text:p/>");
                } else {
                    let _ = write!(out, "<text:p>{}</text:p>", paragraph(line));
                }
            }
            out.push_str("</table:table-cell>");
        }
    }
}

/// A typed cell, with the `text:p` its format renders when it has one.
///
/// The display text is redundant to us — the value and the style say everything — and is
/// written anyway because §7.2's template writes it and because a reader that does not
/// implement number formats shows *something* rather than a blank cell. It is never the
/// value: every branch here carries a typed `office:*-value` attribute beside it.
fn display(
    out: &mut String,
    attrs: &str,
    typed: &str,
    value: &CellValue,
    format: Option<&Format>,
    null_date: i64,
) {
    let Some(format) = format else {
        let _ = write!(out, "<table:table-cell{attrs} {typed}/>");
        return;
    };
    let _ = write!(
        out,
        "<table:table-cell{attrs} {typed}><text:p>{}</text:p></table:table-cell>",
        paragraph(&format.render(value, null_date))
    );
}

/// ` table:number-<axis>-repeated="n"`, or nothing at all when `n` is one.
fn count(n: u32, axis: &str) -> String {
    if n > 1 {
        format!(" table:number-{axis}-repeated=\"{n}\"")
    } else {
        String::new()
    }
}

/// One line of cell text as the body of a `text:p`.
///
/// Whitespace inside a `text:p` is **collapsed** by any conforming reader, which is the
/// entire reason `text:s` and `text:tab` exist. Writing `"a    b"` literally gets it back
/// as `"a b"`, and a leading or trailing space vanishes outright — so runs of spaces, and
/// any space at either end, are written as an explicit `text:s`. A single interior space
/// survives collapsing untouched and stays literal, which keeps ordinary prose readable in
/// the output.
fn paragraph(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        let plain = rest.find([' ', '\t']).unwrap_or(rest.len());
        if plain > 0 {
            out.push_str(&esc(&rest[..plain]));
            rest = &rest[plain..];
            continue;
        }
        if rest.starts_with('\t') {
            out.push_str("<text:tab/>");
            rest = &rest[1..];
            continue;
        }
        let spaces = rest.len() - rest.trim_start_matches(' ').len();
        let ends_the_line = spaces == rest.len();
        if spaces == 1 && !out.is_empty() && !ends_the_line {
            out.push(' ');
        } else {
            let _ = write!(out, "<text:s text:c=\"{spaces}\"/>");
        }
        rest = &rest[spaces..];
    }
    out
}

/// Escape for both text and attribute values, dropping characters XML cannot represent.
///
/// The drop is not paranoia: values arrive from documents we did not write, and a control
/// character that is legal in a Rust `String` has no encoding in XML 1.0 at all — emitting
/// one produces a file no parser will read back, including ours.
fn esc(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| matches!(*c, '\t' | '\n' | '\r' | ' '..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..))
        .collect();
    quick_xml::escape::escape(&clean).into_owned()
}

/// The zip package (§1.1). Only the three entries that are actually required (§1.4).
fn package(doc: &Document) -> Result<Vec<u8>> {
    let zip = |e: zip::result::ZipError| Error::Package(e.to_string());
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));

    // `mimetype` must be first, stored uncompressed, raw bytes, no trailing newline —
    // readers sniff it at a fixed offset before parsing any XML (§1.1).
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("mimetype", stored).map_err(zip)?;
    w.write_all(MIMETYPE.as_bytes())?;

    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    w.start_file("META-INF/manifest.xml", deflated)
        .map_err(zip)?;
    w.write_all(manifest().as_bytes())?;

    w.start_file("content.xml", deflated).map_err(zip)?;
    w.write_all(content(doc, Form::Package).as_bytes())?;

    Ok(w.finish().map_err(zip)?.into_inner())
}

/// §1.3. Lists the package root and every part actually present — a part in the zip but
/// missing here is rejected or ignored by LO's package layer, and the converse is worse.
fn manifest() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <manifest:manifest \
         xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" \
         manifest:version=\"{VERSION}\">\n\
         \x20<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"{VERSION}\" \
         manifest:media-type=\"{MIMETYPE}\"/>\n\
         \x20<manifest:file-entry manifest:full-path=\"content.xml\" \
         manifest:media-type=\"text/xml\"/>\n\
         </manifest:manifest>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(doc: &Document) -> String {
        content(doc, Form::Flat)
    }

    #[test]
    fn an_empty_sheet_still_has_a_column_and_a_row() {
        let xml = flat(&Document::default());
        // §3.2: both blocks are `oneOrMore` in the grammar. Omitting either is invalid even
        // though there is nothing to say.
        assert!(xml.contains("<table:table-column/>"), "{xml}");
        assert!(
            xml.contains("<table:table-row><table:table-cell/></table:table-row>"),
            "{xml}"
        );
    }

    #[test]
    fn blank_rows_and_cells_collapse_into_repeats() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 0), CellValue::Number(1.0));
        sheet.set(Pos::new(500, 3), CellValue::Number(2.0));

        let xml = flat(&doc);
        // 499 blank rows between them, and 3 blank cells before the second value.
        assert!(xml.contains("table:number-rows-repeated=\"499\""), "{xml}");
        assert!(xml.contains("table:number-columns-repeated=\"3\""), "{xml}");
        // A 501-row sheet must not cost 501 row elements.
        assert!(xml.matches("<table:table-row").count() < 10, "{xml}");
    }

    #[test]
    fn trailing_blank_cells_are_not_written_at_all() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 9), CellValue::Number(1.0));
        sheet.set(Pos::new(1, 0), CellValue::Number(2.0));

        // Row 1 stops after its one cell rather than padding out to the sheet's width;
        // unmentioned is the same as empty (§3.3).
        let xml = flat(&doc);
        let row1 = xml
            .lines()
            .find(|l| l.contains("office:value=\"2\""))
            .unwrap();
        assert_eq!(row1.matches("<table:table-cell").count(), 1, "{row1}");
    }

    #[test]
    fn a_formula_keeps_its_cached_value() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 0), CellValue::Number(30.0));
        sheet.set_formula(Pos::new(0, 0), "of:=SUM([.B1:.C1])".into());

        let xml = flat(&doc);
        // Both, always: an omitted cached value renders blank until recalculation (§4).
        assert!(
            xml.contains("table:formula=\"of:=SUM([.B1:.C1])\""),
            "{xml}"
        );
        assert!(xml.contains("office:value=\"30\""), "{xml}");
    }

    #[test]
    fn characters_xml_cannot_carry_are_dropped_rather_than_emitted() {
        // A vertical tab is legal in a Rust String and has no representation in XML 1.0.
        // Writing it produces a file nothing can read back, including us.
        assert_eq!(esc("a\u{b}b"), "ab");
        assert_eq!(esc("<&\">"), "&lt;&amp;&quot;&gt;");
        assert_eq!(esc("keep\tthese\n"), "keep\tthese\n");
    }

    #[test]
    fn space_runs_are_written_explicitly_because_readers_collapse_them() {
        // A single interior space survives collapsing and stays readable.
        assert_eq!(paragraph("a b"), "a b");
        // Everything else would be eaten: runs, and either end of the line.
        assert_eq!(paragraph("a    b"), "a<text:s text:c=\"4\"/>b");
        assert_eq!(paragraph(" a"), "<text:s text:c=\"1\"/>a");
        assert_eq!(paragraph("a "), "a<text:s text:c=\"1\"/>");
        assert_eq!(paragraph("a\tb"), "a<text:tab/>b");
        assert_eq!(paragraph("<&"), "&lt;&amp;");
    }

    fn money() -> Format {
        let mut f = Format::new(Kind::Currency);
        f.push(Part::Number {
            decimals: 2,
            min_decimals: 2,
            min_int: 1,
            grouping: true,
        });
        f.push(Part::Currency(" \u{20ac}".into()));
        f
    }

    /// §5.3: one style per distinct format, however many cells wear it. Two cells sharing a
    /// format must share a name — pooling that silently reindexes is worse than none, since
    /// the second cell then displays through the *other* format in the document.
    #[test]
    fn identical_formats_pool_into_one_style_and_different_ones_do_not() {
        let mut doc = Document::default();
        let mut percent = Format::new(Kind::Percentage);
        percent.push(Part::Number {
            decimals: 1,
            min_decimals: 1,
            min_int: 1,
            grouping: false,
        });
        let sheet = doc.sheet_mut(0).unwrap();
        for row in 0..3 {
            sheet.set(Pos::new(row, 0), CellValue::Number(1.0));
        }
        sheet.set_format(Pos::new(0, 0), money());
        sheet.set_format(Pos::new(1, 0), percent);
        sheet.set_format(Pos::new(2, 0), money());

        let xml = flat(&doc);
        assert_eq!(xml.matches("<style:style ").count(), 2, "{xml}");
        assert_eq!(xml.matches("table:style-name=\"ce0\"").count(), 2, "{xml}");
        assert_eq!(xml.matches("table:style-name=\"ce1\"").count(), 1, "{xml}");
        // The link from cell to format is the only construct ODF has (§5.3).
        assert!(
            xml.contains("<style:style style:name=\"ce0\" style:family=\"table-cell\" \
                          style:data-style-name=\"N0\"/>"),
            "{xml}"
        );
    }

    /// A formatted cell carries its display text, and the value beside it is untouched —
    /// the format is display only (§5.2).
    #[test]
    fn a_formatted_cell_carries_display_text_next_to_its_real_value() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 0), CellValue::Number(1234.5));
        sheet.set_format(Pos::new(0, 0), money());

        let xml = flat(&doc);
        assert!(xml.contains("office:value=\"1234.5\""), "{xml}");
        assert!(xml.contains("<text:p>1,234.50 \u{20ac}</text:p>"), "{xml}");
    }

    /// A document with no formats must not gain a style section or its namespaces (§1.4).
    #[test]
    fn a_document_without_formats_declares_no_style_namespaces() {
        let xml = flat(&Document::default());
        assert!(!xml.contains("automatic-styles"), "{xml}");
        assert!(!xml.contains("xmlns:number"), "{xml}");
    }

    #[test]
    fn the_package_starts_with_an_uncompressed_mimetype_entry() {
        let bytes = package(&Document::default()).unwrap();
        // Readers sniff this at a fixed offset without unzipping anything (§1.1): local
        // header is 30 bytes, then the name, then the raw media type. Compression method
        // (offset 8) must be 0 = stored, and the extra-field length (offset 28) zero.
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(
            &bytes[8..10],
            &[0, 0],
            "mimetype must be stored, not deflated"
        );
        assert_eq!(
            &bytes[28..30],
            &[0, 0],
            "mimetype entry must carry no extra field"
        );
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..38 + MIMETYPE.len()], MIMETYPE.as_bytes());
    }
}
