// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading and writing ODF **spreadsheets**. **\[ODS\]**
//!
//! The generic half — packaging, namespace resolution, the element-context stack — lives in
//! `grind-core` and is re-exported below, so `read` and `write` reach it as `super::context`
//! and `super::names` exactly as they did when it was one crate. What is left here is the only
//! part that knows what a sheet is: the ODS content model (§3), formulas (§4), and the
//! spreadsheet half of styles (§5).
//!
//! `grind-text` builds on the same three generic modules with contexts of its own, which is
//! what §10 said would happen.

pub mod read;
pub mod source;
pub mod write;

/// The generic modules, re-exported so this crate's readers and writers reach them by the
/// paths they always used. The crate boundary is the real seam — `grind-core` compiles with no
/// knowledge of spreadsheets and R8's test enforces it — and these aliases are ergonomics on
/// this side of it, not a hole in it.
pub use grind_core::odf::{Form, context, names, package};

use crate::Result;
use crate::model::Document;

/// Read an ODF spreadsheet from bytes, in either the package (`.ods`) or flat (`.fods`)
/// form — sniffed from the content, not from a file name.
pub fn read(bytes: &[u8]) -> Result<Document> {
    let content = package::content_xml(bytes)?;
    let mut builder = read::Builder::new();
    // Kept so a chart's `xlink:href` can be resolved against the archive it came from — a
    // package stores one as its own part rather than inline (`doc/chart-format.md`).
    if package::is_package(bytes) {
        builder.set_package(bytes.to_vec());
    }
    // R6: the flat form only, and installed *before* parsing, because the cell contexts
    // record their spans into it as they go. A package is a zip and has no diff to preserve,
    // so it is read without one and always regenerates — see `odf::source`.
    if !package::is_package(bytes) {
        builder.doc.source = Some(Box::new(source::Source::new(Form::Flat, content.clone())));
    }
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
