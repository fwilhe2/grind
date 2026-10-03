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
    // Kept so a `draw:image`'s `xlink:href` can be resolved against the archive it came from —
    // a package stores a picture as its own part rather than inline `office:binary-data`.
    if package::is_package(bytes) {
        builder.set_package(bytes.to_vec());
    }
    // R6, and the rule above it: saving never makes an existing file worse. The source is
    // installed *before* parsing, because the block contexts record their spans into it as
    // they go. For a package its bytes are `content.xml` — the part a splice edits — and the
    // whole archive rides along, so a save keeps every other entry (`envelope::repackage`).
    let mut source = source::Source::new(form_of(bytes), content.clone());
    if package::is_package(bytes) {
        source.package = Some(bytes.to_vec());
    }
    builder.doc.source = Some(Box::new(source));
    // `styles.xml` first, so a named style defined there is already known when a paragraph in
    // `content.xml` references it. A part that will not parse costs the styles it carried and
    // not the document — §9 tolerance, one level up.
    if let Some(styles) = package::styles_xml(bytes) {
        let _ = context::parse(
            std::io::Cursor::new(styles),
            Box::new(read::Root),
            &mut builder,
        );
        // An automatic style belongs to the part that declares it (rng's `office:automatic-
        // styles` is per part), so nothing in `content.xml` can name one of `styles.xml`'s —
        // and a writer reusing one of those names for a run would point at nothing.
        builder.forget_automatic_styles();
    }
    context::parse(
        std::io::Cursor::new(content),
        Box::new(read::Root),
        &mut builder,
    )?;
    builder.publish_styles();
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
