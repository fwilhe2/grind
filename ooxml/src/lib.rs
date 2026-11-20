// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The OOXML family's container layer, shared by the Excel and Word import filters: **OPC**
//! ([`package`] — the zip, relationships, part names and the caps a converter eating strangers'
//! files needs), **markup compatibility** (`mce`), the **tolerant walker** ([`xml`] — an
//! unrecognised element takes its subtree with it) and the **namespace and relationship tables**
//! ([`names`], both flavours). Cells, formulas, paragraphs and runs stay in each filter.

use std::fmt;

mod mce;
pub mod names;
pub mod package;
pub mod xml;

pub use names::Flavour;

pub type Result<T> = std::result::Result<T, Error>;

/// A *file* that cannot be read at all — the one error type of both filters. Anything short of
/// this is a report entry in the filter, since refusing a whole document over one construct helps
/// nobody.
#[derive(Debug)]
pub enum Error {
    /// The container would not open: not a zip, or one whose central directory is unusable.
    Package(String),
    /// Password-protected: the document is fine, we have no key. Distinct from [`Error::Package`]
    /// because "locked" and "not a document" are different sentences for a person, and a corpus
    /// loop must not count a locked file as a failure.
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
    #[must_use]
    pub fn is_encrypted(&self) -> bool {
        matches!(self, Error::Encrypted)
    }
}

/// Whether `docProps/core.xml` gives the document a title, subject, description, keywords or
/// category — what a person typed into File ▸ Properties. The author and dates are not counted:
/// every Office file carries them, and they say nothing the person chose to write down.
#[must_use]
pub fn has_core_properties(core: &[u8]) -> bool {
    let mut reader = xml::Reader::new(core);
    if !matches!(reader.root(), Ok(Some(_))) {
        return false;
    }
    let mut any = false;
    let _ = reader.children(|reader, name, _| {
        if matches!(
            name.local.as_str(),
            "title" | "subject" | "description" | "keywords" | "category"
        ) {
            any |= !reader.text()?.trim().is_empty();
            return Ok(xml::Handled::Yes);
        }
        Ok(xml::Handled::No)
    });
    any
}

#[cfg(test)]
mod core_properties_tests {
    use super::has_core_properties;

    const HEAD: &str = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">"#;

    #[test]
    fn a_title_counts_and_an_author_does_not() {
        let titled = format!("{HEAD}<dc:title>Budget</dc:title></cp:coreProperties>");
        let authored =
            format!("{HEAD}<dc:creator>A. Example</dc:creator><dc:title/></cp:coreProperties>");
        assert!(has_core_properties(titled.as_bytes()));
        assert!(!has_core_properties(authored.as_bytes()));
        assert!(!has_core_properties(b"not xml"));
    }
}
