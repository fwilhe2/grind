// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading Excel workbooks — an import filter that produces an ODF document.
//!
//! `doc/xlsx-import.md` is the plan and is normative; `doc/xlsx-format.md` holds the facts
//! about the format that had to be measured. The short version:
//!
//! - **One way in, never out.** Writing `.xlsx` is `doc/not-doing.md` §1 and stays there.
//!   Reading is a one-way translation at the edge that produces a `grind_sheet::Document`,
//!   and everything downstream is ODF. No Excel vocabulary reaches `grind-sheet` (R1).
//! - **The XML form only** — Office 2007 onwards. Not `.xls`, not `.xlsb`, not
//!   SpreadsheetML 2003. **Transitional** is the target, because that is what every real
//!   producer writes; Strict is recognised by a namespace table rather than by a fork.
//! - **Nothing is evaluated.** Excel's cached values are authoritative and are carried
//!   verbatim. Recalculating an imported document is the user's decision, exactly as it is
//!   for any other document.
//! - **The report is part of the output.** What a conversion dropped is as important as what
//!   it carried; see [`Report`].
//!
//! ```no_run
//! let bytes = std::fs::read("book.xlsx")?;
//! let (document, report) = grind_xlsx::import_bytes(&bytes)?;
//! println!("{} sheets, {} dropped kinds", report.sheets, report.dropped.len());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! **State of the build: milestone X2.** The seam (X0) — the package, the flavour and markup
//! compatibility machinery, the workbook's sheet list — every cell's **value** (X1): shared and
//! inline strings, all seven cell types, and the two date systems with the 1900 leap-year
//! rule — and every cell's **formula** (X2), translated into OpenFormula by [`formula`], with
//! shared groups resolved through the core's own `formula::shift`. A number format is read only
//! far enough to know whether a number is a date (X3), and a cell carries no style yet (X4).

use std::fmt;
use std::path::Path;

use grind_sheet::model::Document;

pub mod address;
pub mod dates;
pub mod formula;
pub mod mce;
pub mod names;
pub mod numfmt;
pub mod package;
pub mod report;
pub mod sheet;
pub mod strings;
pub mod styles;
pub mod workbook;
pub mod xml;

pub use names::Flavour;
pub use report::{Dropped, Report};

pub type Result<T> = std::result::Result<T, Error>;

/// A *file* that cannot be read at all.
///
/// Everything else is a [`Report`] entry, because a conversion that refuses a whole document
/// over one unsupported chart is a conversion nobody can use. Same split `odf/read.rs`
/// already makes between `Error::Xml` and silent tolerance.
#[derive(Debug)]
pub enum Error {
    /// The container would not open: not a zip, or one whose central directory is unusable.
    Package(String),
    /// Password-protected. The workbook is fine; we have no key. Distinct from
    /// [`Error::Package`] because "this is locked" and "this is not a spreadsheet" are
    /// different sentences to put in front of a person — and because loop A′ must not count
    /// a locked file as a tolerance failure.
    Encrypted,
    /// The XML would not parse at all — the *structural* failure case. Unrecognised content
    /// never reaches here; it is ignored instead (`xml.rs`).
    Xml(String),
    /// A readable package that is not a spreadsheet: no workbook part to be found.
    NotSpreadsheet,
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Package(e) => write!(f, "package: {e}"),
            Error::Encrypted => write!(f, "password-protected workbook"),
            Error::Xml(e) => write!(f, "xml: {e}"),
            Error::NotSpreadsheet => write!(f, "not an Excel workbook: no workbook part"),
            Error::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl Error {
    /// The one outcome loop A′ accepts beside success, for the same reason loop A accepts it.
    pub fn is_encrypted(&self) -> bool {
        matches!(self, Error::Encrypted)
    }
}

/// Whether these bytes are an OOXML spreadsheet.
///
/// Sniffed from content, never from the file name, as `grind_core::kind` is: an extension is
/// a hint from a filesystem rather than a fact about the data.
///
/// An **encrypted** workbook answers `false` here — it is a CFB container, and nothing in it
/// can be read without a key. [`import_bytes`] tells the two apart properly, which is the
/// call to make when the answer decides what to say to a person.
pub fn sniff(bytes: &[u8]) -> bool {
    if !package::is_zip(bytes) {
        return false;
    }
    let Ok(mut package) = package::Package::open(bytes) else {
        return false;
    };
    workbook::find_part(&mut package, &mut names::Seen::default()).is_ok()
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

    // A macro-enabled workbook imports its data like any other. The macro is a separate part
    // and is **never executed** — counted, and that is all.
    if package.part("xl/vbaProject.bin").is_some() {
        report.drop_one(Dropped::Macro);
    }

    let mut document = workbook::document(&book, &mut report);

    // Styles before sheets, as `odf/read.rs` reads `styles.xml` before `content.xml`: a serial
    // is not a date until its format says so. Both parts are optional, and a workbook without
    // them reads every cell as a plain value.
    let styles = book
        .styles_part
        .as_deref()
        .and_then(|part| package.part(part))
        .map(|bytes| styles::read(&bytes))
        .unwrap_or_default();
    let strings = book
        .strings_part
        .as_deref()
        .and_then(|part| package.part(part))
        .map(|bytes| strings::read(&bytes))
        .unwrap_or_default();
    let context = sheet::Context {
        strings: &strings,
        styles: &styles,
        date_1904: book.date_1904,
        null_date: document.null_date,
    };
    // `document` has exactly one sheet per entry — or one invented `Sheet1` for a workbook
    // with none, which has no part — so the indices agree.
    for (index, (entry, target)) in book
        .sheets
        .iter()
        .zip(document.sheets.iter_mut())
        .enumerate()
    {
        let Some(bytes) = entry.part.as_deref().and_then(|part| package.part(part)) else {
            continue;
        };
        sheet::read(&bytes, &context, index, target, &mut report, &mut seen);
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

/// The filesystem twin. `import_bytes` is the real function — rule 5, and the browser has no
/// filesystem.
pub fn import_file(path: &Path) -> Result<(Document, Report)> {
    import_bytes(&std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

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
