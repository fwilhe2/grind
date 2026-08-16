// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading and writing OpenDocument.
//!
//! `package`, `names` and `context` are format-agnostic (doc/ods-format.md marks them
//! `[GENERIC]`); only `read` knows what a spreadsheet is. Text documents reuse the other
//! three unchanged (§10).

pub mod context;
pub mod names;
pub mod package;
pub mod read;
pub mod write;

pub use write::Form;

use crate::Result;
use crate::model::Document;

/// Read an ODF spreadsheet from bytes, in either the package (`.ods`) or flat (`.fods`)
/// form — sniffed from the content, not from a file name.
pub fn read(bytes: &[u8]) -> Result<Document> {
    let content = package::content_xml(bytes)?;
    let mut builder = read::Builder::new();
    // `styles.xml` first, so a named style defined there is already known when a cell in
    // `content.xml` references it (doc/ods-format.md §5.1). It holds no cells, so nothing
    // else about the document depends on the order. A part that will not parse costs the
    // styles it carried and not the document — §9 tolerance, one level up.
    if let Some(styles) = package::styles_xml(bytes) {
        let _ = context::parse(
            std::io::Cursor::new(styles),
            Box::new(read::Root),
            &mut builder,
        );
    }
    context::parse(
        std::io::Cursor::new(content),
        Box::new(read::Root),
        &mut builder,
    )?;
    Ok(builder.doc)
}

/// Serialise a document in the requested physical form.
pub fn write(doc: &Document, form: Form) -> Result<Vec<u8>> {
    write::write(doc, form)
}
