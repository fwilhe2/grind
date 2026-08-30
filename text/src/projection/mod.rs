// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The projection of a **text document** — `doc/dsl.md` layer 0, milestone D2. **\[ODT\]**
//!
//! A third physical form beside the package and the flat one: the same document, spelled as
//! plain text a person writes in any editor. The container is `grind_core::projection` (the KDL
//! syntax, the kind header, the token and span maps); what is here is the only part that knows
//! what a paragraph is — this crate's node vocabulary, the inline notation, and the reader and
//! writer over them. §3.2 is why that is a module beside `odf/` rather than a crate of its own,
//! and `core/tests/generic.rs` is that rule made mechanical.
//!
//! ## The grammar
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
//! **The body is flat, and so is this.** `model.rs` records the schema fact the whole crate is
//! shaped by — a `text:h` does not contain the paragraphs under it (rng:16938) — and the
//! projection keeps that shape: one node per block, a heading's level and a list item's depth
//! as numbers rather than as nesting. `doc/dsl.md` §3.5 sketched a `list { li … }` wrapper; the
//! reader still takes one, because somebody hand-writing a list will type it, but the writer
//! never emits one. Tolerance on the way in, strictness on the way out.
//!
//! Three things fall straight out of KDL's string rules that would otherwise each be a
//! decision: interior spaces are `text:s` (KDL does not collapse whitespace and XML does), `\t`
//! is `text:tab`, and `\n` is `text:line-break` — visibly different from a new *block*, which
//! is a new node.
//!
//! ## What it does not carry yet
//!
//! **Images — the one named gap**, and loop F excludes them by name exactly as it excludes the
//! spreadsheet's charts. Not an oversight: `doc/dsl.md` §3.8 answers this one with a *sidecar
//! directory* holding the bytes beside the file, and D4 has since made the projection a
//! [`grind_core::Form`], which is reached through `write_bytes`/`read_bytes` — **bytes, no
//! path**. Rule 5 is that every `*_file` has a `*_bytes` twin, and `grind-web` has no
//! filesystem at all, so a form that only works when there is a directory to put things next to
//! is not a form this project can have. The sidecar is a design question D2 reopens rather than
//! one it implements, and it is written up in `doc/projection-text.md`.
//!
//! Also absent, and absent from the *model* rather than from the projection: paragraph-level
//! properties, style definitions, footnotes, fields, tables. `doc/text-core.md` is the scope
//! line, and `text/tests/projection_scope.rs` holds this vocabulary against it.

pub mod inline;
pub mod read;
pub mod write;

/// The generic container, re-exported so this crate's projection reader and writer reach it by
/// one path — the same ergonomics `odf::mod` provides for `context`, `names` and `package`.
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

/// The bytes a `.grind` is saved as — the file it came from with the edited blocks put back when
/// that is possible (R6, D5), and a fresh projection when it is not.
///
/// The one door out, so that no caller can save a projection *without* R6.
pub fn save(doc: &Document) -> Vec<u8> {
    match write::splice(doc) {
        Some(text) => text.into_bytes(),
        None => project(doc).into_text().into_bytes(),
    }
}
