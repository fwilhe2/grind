// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading and writing ODF spreadsheets. **\[ODS\]** The ODS content model (§3), formulas (§4) and
//! the spreadsheet half of styles (§5), on `grind-core`'s generic modules, re-exported below.

pub mod read;
pub mod source;
pub mod write;

/// `grind-core`'s generic modules, under the paths this crate's reader and writer use.
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
    // Installed before parsing, because the cell contexts record their spans into it. For a
    // package its bytes are `content.xml`, and the whole archive rides along
    // (`envelope::repackage`).
    let mut source = source::Source::new(form_of(bytes), content.clone());
    if package::is_package(bytes) {
        source.package = Some(bytes.to_vec());
    }
    builder.doc.source = Some(Box::new(source));
    // `styles.xml` first, so a named style defined there is known when a cell references it
    // (`doc/ods-format.md` §5.1). A part that will not parse costs its styles, not the document.
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
    let locale = builder.doc.locale.clone();
    let epoch = (builder.doc.null_date, builder.doc.null_year);
    let chart_parts = std::mem::take(&mut builder.chart_parts);
    if let Some(source) = builder.doc.source.as_deref_mut() {
        source.locale = locale;
        source.chart_parts = chart_parts;
        source.epoch = epoch;
    }
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
