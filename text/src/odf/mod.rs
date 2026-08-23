// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading and writing ODF **text documents**. **\[ODT\]**
//!
//! The generic half — packaging, namespace resolution, the element-context stack — lives in
//! `grind-core` and is re-exported below, so `read` reaches it as `super::context` and
//! `super::names` the way `grind_sheet::odf` does. What is left here is the only part that
//! knows what a paragraph is.
//!
//! R6's retain-and-splice is not here yet: this writer regenerates, which is always correct
//! and is what the spreadsheet did until phase 8.

pub mod read;
pub mod source;
pub mod write;

/// The generic modules, re-exported so this crate's reader reaches them by one path.
pub use grind_core::odf::{Form, context, names, package};

use crate::model::Document;
use grind_core::Result;

/// Read an ODF text document from bytes, in either the package (`.odt`) or flat (`.fodt`)
/// form — sniffed from the content, not from a file name.
pub fn read(bytes: &[u8]) -> Result<Document> {
    let content = package::content_xml(bytes)?;
    let mut builder = read::Builder::new();
    // R6: the flat form only, and installed *before* parsing, because the block contexts
    // record their spans into it as they go. A package is a zip and has no diff to preserve,
    // so it is read without one and always regenerates — see `odf::source`.
    if !package::is_package(bytes) {
        builder.doc.source = Some(Box::new(source::Source::new(Form::Flat, content.clone())));
    }
    // `styles.xml` first, so a named style defined there is already known when a paragraph in
    // `content.xml` references it. A part that will not parse costs the styles it carried and
    // not the document — §9 tolerance, one level up.
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
    builder.doc.reindex_bookmarks();
    // Reading is not editing: a document just opened has no changes to splice.
    builder.doc.edits = source::Edits::default();
    Ok(builder.doc)
}

/// Serialise a document in the requested physical form.
pub fn write(doc: &Document, form: Form) -> Result<Vec<u8>> {
    write::write(doc, form)
}
