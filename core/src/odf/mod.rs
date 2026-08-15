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

use crate::model::Document;
use crate::Result;

/// Read an ODF spreadsheet from bytes, in either the package (`.ods`) or flat (`.fods`)
/// form — sniffed from the content, not from a file name.
pub fn read(bytes: &[u8]) -> Result<Document> {
    let content = package::content_xml(bytes)?;
    let mut builder = read::Builder::new();
    context::parse(
        std::io::Cursor::new(content),
        Box::new(read::Root),
        &mut builder,
    )?;
    Ok(builder.doc)
}
