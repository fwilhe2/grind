// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading Excel workbooks — an import filter that produces an ODF document (`doc/xlsx-import.md`;
//! the measured facts are in `doc/xlsx-format.md`).
//!
//! - **One way in, never out.** Reading produces a `grind_sheet::Document` and everything
//!   downstream is ODF; no Excel vocabulary reaches `grind-sheet` (R1).
//! - **The XML form only**, Office 2007 onwards: not `.xls`, `.xlsb` or SpreadsheetML 2003.
//!   Transitional is the target, as every real producer writes it; Strict is recognised by a
//!   namespace table, not a fork.
//! - **Nothing is evaluated.** Excel's cached values are carried verbatim; recalculating is the
//!   user's decision.
//! - **The report is part of the output** ([`Report`]): what was dropped matters as much as what
//!   was carried.
//!
//! ```no_run
//! let bytes = std::fs::read("book.xlsx")?;
//! let (document, report) = grind_xlsx::import_bytes(&bytes)?;
//! println!("{} sheets, {} dropped kinds", report.sheets, report.dropped.len());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Values and the two date systems ([`dates`]), formulas ([`formula`]), number formats
//! ([`numfmt`]), the look ([`styles`], [`color`], [`theme`]) with track sizes, and the document
//! level — names, sheet names, autofilters, merges, rules, and the parts beside a sheet
//! ([`parts`]). What the model cannot hold is a named class, counted and never approximated. A
//! shell opens a workbook through [`open`], [`suggested_name`] and [`Report::summary`].

use std::path::Path;

use grind_sheet::model::Document;

pub mod address;
pub mod color;
pub mod controls;
pub mod dates;
pub mod formula;
pub mod numfmt;
pub mod parts;
pub mod report;
pub mod rules;
pub mod sheet;
pub mod strings;
pub mod styles;
pub mod tables;
pub mod theme;
pub mod workbook;
pub mod xml;

// The container layer, hoisted into `grind-ooxml` when the word processor's filter wanted it,
// and re-exported under the paths this crate always used.
pub use grind_ooxml::{mce, names, package};

pub use names::Flavour;
pub use report::{Dropped, Report};
pub use styles::Appearance;

pub use grind_ooxml::{Error, Result};

/// Whether these bytes are an OOXML spreadsheet, sniffed from content, never the name. An
/// **encrypted** workbook answers `false` (it is a CFB container); [`import_bytes`] tells the two
/// apart, which is the call to make when the answer decides what to tell a person.
pub fn sniff(bytes: &[u8]) -> bool {
    if !package::is_zip(bytes) {
        return false;
    }
    let Ok(mut package) = package::Package::open(bytes) else {
        return false;
    };
    let Ok(part) = workbook::find_part(&mut package, &mut names::Seen::default()) else {
        return false;
    };
    // A Word document's main part is found by the same relationship type, so the part has to
    // be asked what it is: a `<workbook>`, or something this filter does not read.
    use xml::Spreadsheet as _;
    package.part(&part).is_some_and(|bytes| {
        matches!(xml::Reader::new(&bytes).root(), Ok(Some((name, _))) if name.is("workbook"))
    })
}

/// Read an Excel workbook.
///
/// Never evaluates, never fetches anything, and never fails on a construct it cannot carry:
/// what it cannot carry is counted in the [`Report`].
pub fn import_bytes(bytes: &[u8]) -> Result<(Document, Report)> {
    let mut report = Report::default();
    let mut seen = names::Seen::default();
    let mut package = package::Package::open(bytes)?;

    let part = workbook::find_part(&mut package, &mut seen)?;
    let source = package.part(&part).ok_or(Error::NotSpreadsheet)?;
    let mut book = workbook::read(&source, &mut report)?;
    workbook::resolve_parts(&mut book, &mut package, &part, &mut seen);

    // A macro-enabled workbook imports its data like any other; the macro is counted and **never
    // executed**.
    if package.part("xl/vbaProject.bin").is_some() {
        report.drop_one(Dropped::Macro);
    }

    let mut document = workbook::document(&book, &mut report);
    let names = workbook::names(&book, &mut document, &mut report);

    // Styles before sheets: a serial is not a date until its format says so. Both parts are
    // optional.
    let theme = book
        .theme_part
        .as_deref()
        .and_then(|part| package.part(part))
        .map(|bytes| theme::read(&bytes))
        .unwrap_or_default();
    let styles = book
        .styles_part
        .as_deref()
        .and_then(|part| package.part(part))
        .map(|bytes| styles::read(&bytes, &theme))
        .unwrap_or_default();
    let size = styles.default_size();
    document.default_font_size =
        (size != grind_sheet::look::DEFAULT_FONT_PT).then(|| format!("{size}pt"));
    let strings = book
        .strings_part
        .as_deref()
        .and_then(|part| package.part(part))
        .map(|bytes| strings::read(&bytes))
        .unwrap_or_default();
    let renames = report.renamed.clone();
    let sheet_names: Vec<String> = book.sheets.iter().map(|s| s.name.clone()).collect();
    let mut tables = tables::Tables::default();
    for (index, entry) in book.sheets.iter().enumerate() {
        let Some(part) = entry.part.as_deref() else {
            continue;
        };
        for rel in package.rels(part, &mut seen) {
            if rel.kind != names::RelType::Table || rel.external {
                continue;
            }
            if let Some(table) = package
                .part(&rel.target)
                .and_then(|bytes| tables::read(&bytes, index))
            {
                tables.list.push(table);
            }
        }
    }
    let context = sheet::Context {
        strings: &strings,
        styles: &styles,
        date_1904: book.date_1904,
        null_date: document.null_date,
        names: &names,
        renames: &renames,
        tables: &tables,
        sheet_names: &sheet_names,
    };
    // `document` has exactly one sheet per entry — or one invented `Sheet1` for a workbook
    // with none, which has no part — so the indices agree.
    for (index, (entry, target)) in book
        .sheets
        .iter()
        .zip(document.sheets.iter_mut())
        .enumerate()
    {
        let Some(part) = entry.part.as_deref() else {
            continue;
        };
        let Some(bytes) = package.part(part) else {
            continue;
        };
        sheet::read(&bytes, &context, index, target, &mut report, &mut seen);
        controls::read(&mut package, part, target, &mut report, &mut seen);
        parts::count(&mut package, part, &mut report, &mut seen);
    }
    // The flavour is whatever the *whole* read saw, not only what the workbook part did: a
    // Strict relationship type in `_rels/.rels` is evidence before any part is opened.
    if seen.strict {
        report.flavour = match report.flavour {
            Flavour::Transitional => Flavour::Mixed,
            other => other,
        };
    }
    Ok((document, report))
}

/// What a **shell** opens: the workbook imported and written as flat ODF, ready for
/// `App::open_bytes` under [`suggested_name`], with the report beside it. Bytes because `App`'s
/// only door is `open_bytes`; writing then reading is the identity loop A′ asserts, and costs
/// about a second and a half for half a million cells in a release build. Flat is the default form
/// (`doc/flat-first.md`).
pub fn open(bytes: &[u8]) -> Result<(Vec<u8>, Report)> {
    let (document, report) = import_bytes(bytes)?;
    let odf = grind_sheet::write_bytes(&document, grind_sheet::Form::Flat)
        .map_err(|e| Error::Xml(format!("the imported document would not write: {e}")))?;
    Ok((odf, report))
}

/// The name an imported workbook goes by until it is saved: `budget.xlsx` becomes `budget.fods`.
///
/// **A shell must never save back to the workbook's own path**: writing ODF into a file called
/// `.xlsx` is the one way this filter could destroy a user's data. This is the name to offer in a
/// save dialog; the path to save to is `None` until the user picks one.
pub fn suggested_name(workbook: &str) -> String {
    let stem = match workbook.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.contains(['/', '\\']) => stem,
        _ => workbook,
    };
    format!("{stem}.fods")
}

/// The filesystem twin. `import_bytes` is the real function — rule 5, and the browser has no
/// filesystem.
pub fn import_file(path: &Path) -> Result<(Document, Report)> {
    import_bytes(&std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_imported_workbook_is_offered_under_an_odf_name() {
        assert_eq!(suggested_name("budget.xlsx"), "budget.fods");
        assert_eq!(
            suggested_name("/home/a/Q3 plan.xlsm"),
            "/home/a/Q3 plan.fods"
        );
        assert_eq!(suggested_name("no-extension"), "no-extension.fods");
        assert_eq!(suggested_name("dir.v2/book"), "dir.v2/book.fods");
    }

    #[test]
    fn nothing_that_is_not_a_package_sniffs_as_one() {
        assert!(!sniff(b""));
        assert!(!sniff(b"<?xml version=\"1.0\"?><office:document/>"));
        assert!(!sniff(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]));
    }

    #[test]
    fn a_locked_workbook_says_so_rather_than_claiming_to_be_junk() {
        let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        bytes.extend_from_slice(&[0u8; 512]);
        let err = import_bytes(&bytes).expect_err("a locked workbook is not importable");
        assert!(err.is_encrypted(), "got {err}");
    }
}
