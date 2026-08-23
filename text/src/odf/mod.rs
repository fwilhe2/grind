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
//! The writer is S5 and is not here yet; `doc/suite.md` has the order and the reason for it.

pub mod read;

/// The generic modules, re-exported so this crate's reader reaches them by one path.
pub use grind_core::odf::{Form, context, names, package};

use crate::model::Document;
use grind_core::Result;

/// Read an ODF text document from bytes, in either the package (`.odt`) or flat (`.fodt`)
/// form — sniffed from the content, not from a file name.
pub fn read(bytes: &[u8]) -> Result<Document> {
    let content = package::content_xml(bytes)?;
    let mut builder = read::Builder::new();
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
    Ok(builder.doc)
}
