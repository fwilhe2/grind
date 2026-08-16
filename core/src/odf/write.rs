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
//! The output is deliberately minimal (§1.4): no `styles.xml`, no `meta.xml`, no
//! `settings.xml`, and no automatic styles, because there is nothing yet to put in them.
//! Styles arrive in phase 5 and land in the one `content` function.

use std::fmt::Write as _;
use std::io::{Cursor, Write as _};

use super::names::{OFFICE, TABLE, TEXT};
use crate::formula::date;
use crate::model::{CellValue, Document, NumberKind, Pos, Sheet};
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
    match form {
        Form::Flat => Ok(content(doc, form).into_bytes()),
        Form::Package => package(doc),
    }
}

/// The `content.xml` payload, which in the flat form is the whole document (§7.1–7.3).
fn content(doc: &Document, form: Form) -> String {
    let root = match form {
        Form::Flat => "office:document",
        Form::Package => "office:document-content",
    };

    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    // Declare only the namespaces this part actually uses (§1.4). `table:formula`'s `of:`
    // prefix is part of the *string*, not a namespace-resolved name (§4), so no `xmlns:of`.
    let _ = write!(
        out,
        "<{root} xmlns:office=\"{OFFICE}\" xmlns:table=\"{TABLE}\" xmlns:text=\"{TEXT}\" \
         office:version=\"{VERSION}\""
    );
    if form == Form::Flat {
        let _ = write!(out, " office:mimetype=\"{MIMETYPE}\"");
    }
    out.push_str(">\n <office:body>\n  <office:spreadsheet>\n");
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
    // §5.11. Before the tables, which is where the schema puts them — and every name is
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
    for sheet in &doc.sheets {
        table(&mut out, sheet, doc.null_date);
    }
    let _ = write!(out, "  </office:spreadsheet>\n </office:body>\n</{root}>\n");
    out
}

fn table(out: &mut String, sheet: &Sheet, null_date: i64) {
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
        write_row(out, sheet, row, cols, null_date);
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

fn write_row(out: &mut String, sheet: &Sheet, row: u32, cols: u32, null_date: i64) {
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
        // Only blank runs are compressed. Repeating a *valued* cell would need the formula
        // to repeat with it, and a formula is position-dependent — a correctness trap for
        // bytes nobody is short of.
        let repeat = if value.is_empty() && formula.is_none() {
            (col..=last)
                .take_while(|c| {
                    let p = Pos::new(row, *c);
                    sheet.get(p).is_empty() && sheet.formula(p).is_none()
                })
                .count() as u32
        } else {
            1
        };
        cell(out, &value, formula, sheet.kind(pos), null_date, repeat);
        col += repeat;
    }
    out.push_str("</table:table-row>\n");
}

fn cell(
    out: &mut String,
    value: &CellValue,
    formula: Option<&str>,
    kind: Option<NumberKind>,
    null_date: i64,
    repeat: u32,
) {
    let mut attrs = count(repeat, "columns");
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
            match kind {
                Some(NumberKind::Date) => {
                    let _ = write!(
                        out,
                        "<table:table-cell{attrs} office:value-type=\"date\" \
                         office:date-value=\"{}\"/>",
                        date::format_date(n, null_date)
                    );
                }
                Some(NumberKind::Time) => {
                    let _ = write!(
                        out,
                        "<table:table-cell{attrs} office:value-type=\"time\" \
                         office:time-value=\"{}\"/>",
                        date::format_time(n)
                    );
                }
                None => {
                    let _ = write!(
                        out,
                        "<table:table-cell{attrs} office:value-type=\"float\" \
                         office:value=\"{n}\"/>"
                    );
                }
            }
        }
        CellValue::Bool(b) => {
            let _ = write!(
                out,
                "<table:table-cell{attrs} office:value-type=\"boolean\" \
                 office:boolean-value=\"{b}\"/>"
            );
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
