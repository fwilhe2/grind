// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The projection of a text document — `doc/dsl.md` layer 0. **\[ODT\]**
//!
//! The same document as plain text: `grind_core::projection` is the container, and this is the
//! text node vocabulary, the inline notation, and their reader and writer.
//! `doc/projection-text.md` is the full grammar.
//!
//! ```kdl
//! grind text
//!
//! h 1 "Field Notes"
//! p "Written entirely from a shell, which is **rather** the point."
//! h 2 "Addresses"
//! p "{#intro}p12 is a position. §2.1 survives edits above it."
//! li 1 "by position"
//! li 2 "invalidated by an insert above"
//! p #"a literal ** stays literal, and    four spaces survive"#
//! p "name\tvalue\nsecond line" style="Preformatted Text"
//! ```
//!
//! The body is flat, and so is this: one node per block, a heading's level and a list item's depth
//! as numbers (rng:16938). The reader also takes a `list { li … }` wrapper, which a person writing
//! by hand will type; the writer never emits one. Interior spaces, `\t` and `\n` are KDL's own
//! string rules: `text:s`, `text:tab`, `text:line-break`.
//!
//! Images are not projected — loop F's one named gap, as charts are for spreadsheets — because
//! their bytes would need a sidecar directory, and a form must work as bytes alone (rule 5;
//! `doc/projection-text.md`). Footnotes and fields are spelled as the text they show.

pub mod inline;
pub mod read;
pub mod write;

/// `grind_core::projection`'s container types, under this crate's path.
pub use grind_core::projection::{Anchor, Emitter, Projection, Token, TokenKind};

use crate::Result;
use crate::model::Document;

/// Project a document: the text, and the token and span maps beside it.
pub fn project(doc: &Document) -> Projection {
    write::project(doc)
}

/// Read a projection back into a document.
pub fn read(text: &str) -> Result<Document> {
    read::read(text)
}

/// The bytes a `.grind` is saved as: the file it came from with edited blocks put back when
/// possible (R6), a fresh projection otherwise. The one door out.
pub fn save(doc: &Document) -> Vec<u8> {
    match write::splice(doc) {
        Some(text) => text.into_bytes(),
        None => project(doc).into_text().into_bytes(),
    }
}
