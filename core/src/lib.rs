// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ODF plumbing shared by every document type. **\[GENERIC\]**
//!
//! This crate is `doc/suite.md`'s **R8** made a compilation unit: *no document type's
//! vocabulary appears in the shared crate.* Nothing here knows what a cell, a sheet, a
//! paragraph or a heading is. What it knows is the container (`odf::package`), the namespace
//! vocabulary (`odf::names`), the tolerant reading architecture (`odf::context`), the
//! styling primitives every family of style is built from (`style`), and how to tell one
//! document type from another before parsing it (`kind`).
//!
//! `doc/ods-format.md` marked these sections `[GENERIC]` from the beginning and §10 predicted
//! this split; extracting them was a move rather than a redesign. Each document type builds on
//! it with a crate of its own — `grind-sheet`, `grind-text` — that owns a model, a reader, a
//! writer and an `App`. What is *not* here is a trait over those `App`s; `observer.rs` says
//! why, and what has to exist before one can be written honestly.
//!
//! **R8 is checked, not promised**: this crate's `Cargo.toml` names no document-type crate, and
//! `tests/generic.rs` fails the build if body vocabulary appears in these sources.

use std::fmt;

pub mod build_info;
pub mod kind;
pub mod layout;
pub mod locale;
pub mod observer;
pub mod odf;
pub mod projection;
pub mod style;

pub use kind::{DocumentKind, kind};
pub use observer::Observer;
pub use odf::Form;

/// What can go wrong with an ODF document, whatever kind of document it is.
///
/// Deliberately short. Anything a *spreadsheet* or a *text document* can be wrong about — no
/// such sheet, a formula that will not parse, a range too large to format — belongs to that
/// document type's own error enum, which wraps this one (R8). `grind_sheet::Error::Odf` is the
/// worked example, and `?` does the conversion, so the split costs a `From` impl rather than a
/// rewrite of every call site.
#[derive(Debug)]
pub enum Error {
    /// The XML would not parse at all. Per doc/ods-format.md §8.2 this is the *structural*
    /// failure case — unrecognised content never reaches here, it is ignored instead.
    Xml(String),
    /// The zip container would not open, or holds no `content.xml`.
    Package(String),
    /// Password-protected. The document is fine; we have no key. Distinct from [`Error::Xml`]
    /// so callers can tell "cannot open" from "will not parse".
    Encrypted,
    /// A projection (`doc/dsl.md` layer 0) that will not parse, or that does not declare what
    /// kind of document it is. Distinct from [`Error::Xml`] because a projection is a *third*
    /// physical form beside the package and the flat one, and "line 12, column 4" is a
    /// different apology from "this zip has no content.xml".
    Projection(String),
    /// A document whose kind this build has no reader for — a presentation, say — or bytes
    /// that are not an ODF document at all. Carries what `kind` made of it, because "this is a
    /// presentation" and "this is not a document" want different words in front of a user.
    UnsupportedKind(Option<DocumentKind>),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Xml(e) => write!(f, "xml: {e}"),
            Error::Package(e) => write!(f, "package: {e}"),
            Error::Encrypted => write!(f, "password-protected document"),
            Error::Projection(e) => write!(f, "projection: {e}"),
            Error::UnsupportedKind(Some(kind)) => match kind.command() {
                Some(command) => write!(f, "that is a {}; try `grind {command}`", kind.label()),
                None => write!(f, "{}s are not something this build opens", kind.label()),
            },
            Error::UnsupportedKind(None) => write!(f, "not an OpenDocument file"),
            Error::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
