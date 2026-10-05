// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The OOXML family's container layer, shared by the two import filters.
//!
//! `grind-xlsx` (`doc/xlsx-import.md`) built all of this first, for Excel. When `grind-docx`
//! (`doc/docx-import.md`) arrived it wanted the same four things and none of them were about
//! spreadsheets: **OPC** ([`package`] — the zip, relationships, part names and the caps a
//! converter eating strangers' files needs), **markup compatibility** ([`mce`]), the **tolerant
//! walker** ([`xml`] — an unrecognised element takes its subtree with it) and the **namespace and
//! relationship tables** ([`names`], both flavours). So they were hoisted rather than copied,
//! the way `grind_sheet::formula::assist` came out of a shell the day a second one asked.
//!
//! What stays in each filter is its own vocabulary: cells and formulas in one, paragraphs and
//! runs in the other. Nothing here knows either.

use std::fmt;

pub mod mce;
pub mod names;
pub mod package;
pub mod xml;

pub use names::Flavour;

pub type Result<T> = std::result::Result<T, Error>;

/// A *file* that cannot be read at all — the one error type of both filters.
///
/// Everything short of this is a report entry in whichever filter is reading, because a
/// conversion that refuses a whole document over one unsupported construct is a conversion
/// nobody can use.
#[derive(Debug)]
pub enum Error {
    /// The container would not open: not a zip, or one whose central directory is unusable.
    Package(String),
    /// Password-protected. The document is fine; we have no key. Distinct from
    /// [`Error::Package`] because "this is locked" and "this is not a document" are different
    /// sentences to put in front of a person — and because a corpus loop must not count a
    /// locked file as a tolerance failure.
    Encrypted,
    /// The XML would not parse at all — the *structural* failure case. Unrecognised content
    /// never reaches here; it is ignored instead (`xml.rs`).
    Xml(String),
    /// A readable package that is not a spreadsheet: no workbook part to be found.
    NotSpreadsheet,
    /// A readable package that is not a word-processing document: no document part.
    NotWordDocument,
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Package(e) => write!(f, "package: {e}"),
            Error::Encrypted => write!(f, "password-protected document"),
            Error::Xml(e) => write!(f, "xml: {e}"),
            Error::NotSpreadsheet => write!(f, "not an Excel workbook: no workbook part"),
            Error::NotWordDocument => write!(f, "not a Word document: no document part"),
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
    /// The one outcome a corpus loop accepts beside success, for the same reason loop A
    /// accepts it.
    pub fn is_encrypted(&self) -> bool {
        matches!(self, Error::Encrypted)
    }
}
