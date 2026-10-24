// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The projection of a spreadsheet — `doc/dsl.md` layer 0, a `.grind` file. **\[ODS\]**
//!
//! The same document as plain text: `grind_core::projection` is the container, and this is the
//! spreadsheet's node vocabulary with its reader and writer. Reading a projection and writing it
//! back moves nothing, and projecting a document and reading it back gives the same model
//! (`sheet/tests/loop_f.rs`). `doc/projection-sheet.md` is the full grammar.
//!
//! ```kdl
//! grind spreadsheet
//!
//! // Comments survive an edit.
//! null-date "1899-12-30"          // only when it is not the default
//! null-year 1930                  // likewise
//! name "tax_rate" "[$Sales.$B$1]" // table:named-expressions (§5.11)
//!
//! sheet Sales {
//!     col 2 width="2.258cm" hidden=#true
//!     row 3 height="0.45cm" hidden=#true
//!
//!     at A1 {
//!         row Region Q1 Q2
//!         row North  4200 4800
//!         row South  3100 3300
//!     }
//!     cell B5 15400 formula="of:=SUM([.B2:.B4])"
//!     cell A7 45123 date=#true
//!
//!     style  B1:C1 bold=#true background="#0074d9"
//!     format B2:C5 currency decimals=2 symbol="EUR"
//!
//!     filter "__Anonymous_Sheet_DB__0" A1:C4 header=#true {
//!         keep 0 North South
//!     }
//! }
//! ```
//!
//! * `at`/`row` and `cell` are two spellings of the same state (§3.4): the writer puts plain
//!   values in a grid and anything more in a `cell`, and the reader takes both — which is why loop
//!   F compares models, never bytes.
//! * A formula is stored verbatim, `of:` and all. The reader also takes a bare `"=SUM(…)"` in the
//!   value position and adds the prefix.
//! * A range is a bare identifier (`B1:C1`), so a style or format is stated once over its span.
//! * A number format that is not one of `numfmt::preset`'s spells its parts (§3.8), never an Excel
//!   format code.
//!
//! Charts are not projected (loop F's one named gap for spreadsheets), nor are `office:settings`
//! and style definitions: converting to a projection drops them.

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

/// The bytes a `.grind` is saved as: the file it came from with the edited cells put back when
/// possible (R6), a fresh projection otherwise. The one door out, so no caller saves a projection
/// without splicing.
pub fn save(doc: &Document) -> Vec<u8> {
    match write::splice(doc) {
        Some(text) => text.into_bytes(),
        None => project(doc).into_text().into_bytes(),
    }
}

/// Read a projection back into a document.
pub fn read(text: &str) -> Result<Document> {
    read::read(text)
}
