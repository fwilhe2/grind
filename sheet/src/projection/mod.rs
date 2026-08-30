// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The projection of a **spreadsheet** — `doc/dsl.md` layer 0. **\[ODS\]**
//!
//! A third physical form beside the package and the flat one: the same document, spelled as
//! plain text a person writes in any editor. The container is `grind_core::projection` (the
//! KDL syntax, the kind header, the token and span maps); what is here is the only part that
//! knows what a sheet is — this crate's node vocabulary, and the reader and writer over it.
//! §3.2 is why that is a module beside `odf/` rather than a crate of its own.
//!
//! **The bijection is the whole promise.** Read a projection, write it back, and nothing has
//! moved; project a document, read it back, and it is the same model. `sheet/tests/loop_f.rs`
//! is that claim held against every document the corpus has. The layer-1 generator — a script
//! that *computes* a document — deliberately does not round-trip and is not here (§1).
//!
//! ## The grammar
//!
//! ```kdl
//! grind spreadsheet
//!
//! // Comments survive an edit; see R6.
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
//! Four things about the shape, each of them a decision rather than an accident:
//!
//! * **`at`/`row` and `cell` are two spellings of the same state** (§3.4). The reader takes
//!   both; the writer emits a grid for the cells that are only values and a `cell` for the
//!   ones that carry more. Tolerance on the way in, strictness on the way out — this
//!   project's rule, applied to its own format. It is also why loop F compares *models* and
//!   never bytes.
//! * **A formula is stored verbatim**, `of:` prefix and all, because that is what the document
//!   holds and R1 says ODF's spelling is the product. As an authoring convenience the reader
//!   also takes a bare `"=SUM([.B2:.B4])"` in the value position and supplies the prefix —
//!   which is a *normalisation*, so a hand-written file and its re-projection differ by that
//!   one string and by nothing else.
//! * **A range is a bare identifier** (`B1:C1`), so a style or a format is stated once over the
//!   span it covers rather than once per cell — both how a person thinks and how the ODF
//!   writer pools them anyway.
//! * **A number format spells its parts** when it is not one of `numfmt::preset`'s (§3.8).
//!   `numfmt::Format` is an ordered sequence of `Part`s and not a format string, deliberately
//!   (`doc/ods-format.md` §5.2), so the projection spells the parts rather than inventing
//!   Excel's `#,##0.00`. `Format::is_preset` already draws exactly this line for the GTK
//!   format picker, and this is its second caller.
//!
//! ## What it does not carry yet
//!
//! Named gaps, in the sense loop F counts: a document using one of these projects with the
//! construct **dropped**, and the loop's excused list says so by name rather than the ratchet
//! quietly absorbing it.
//!
//! * **Charts** (`doc/chart-format.md`). Expressible, verbose, and nobody hand-writes one —
//!   §3.8 puts them in for bijectivity rather than for authoring, and they are not in yet.
//! * **The `office:settings` and style *definitions*** a `.fods` carries and R6 preserves.
//!   Converting *to* a projection drops what the projection has no node for, exactly as
//!   regenerating does today (§9). `grind lint` is the milestone that says which, by name,
//!   before it happens.

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
