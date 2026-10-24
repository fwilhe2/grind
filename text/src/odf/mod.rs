// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading and writing ODF text documents. **\[ODT\]** The text content model, on `grind-core`'s
//! generic modules, re-exported below; `source` keeps the file a document came from for splicing.

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
    // Kept so a `draw:image`'s `xlink:href` resolves against the archive: a package stores a
    // picture as its own part.
    if package::is_package(bytes) {
        builder.set_package(bytes.to_vec());
    }
    // Installed before parsing, since the block contexts record their spans into it. For a package
    // its bytes are `content.xml`, and the archive rides along (`envelope::repackage`).
    let mut source = source::Source::new(form_of(bytes), content.clone());
    if package::is_package(bytes) {
        source.package = Some(bytes.to_vec());
    }
    builder.doc.source = Some(Box::new(source));
    // `styles.xml` first, so a named style defined there is known when a paragraph references it.
    // A part that will not parse costs its styles, not the document.
    if let Some(styles) = package::styles_xml(bytes) {
        let _ = context::parse(
            std::io::Cursor::new(styles),
            Box::new(read::Root),
            &mut builder,
        );
        // An automatic style belongs to the part declaring it, so nothing in `content.xml` can
        // name one of `styles.xml`'s, and reusing one of those names would point at nothing.
        builder.forget_automatic_styles();
    }
    context::parse(
        std::io::Cursor::new(content),
        Box::new(read::Root),
        &mut builder,
    )?;
    builder.publish_styles();
    builder.settle_paragraph_styles();
    builder.settle_page();
    builder.settle_table_looks();
    builder.doc.reindex_bookmarks();
    // Reading is not editing: a document just opened has no changes to splice.
    builder.doc.edits = source::Edits::default();
    Ok(builder.doc)
}

/// Serialise a document in the requested physical form.
fn form_of(bytes: &[u8]) -> Form {
    match package::is_package(bytes) {
        true => Form::Package,
        false => Form::Flat,
    }
}

pub fn write(doc: &Document, form: Form) -> Result<Vec<u8>> {
    write::write(doc, form)
}
