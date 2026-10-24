// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ODF plumbing shared by every document type. **\[GENERIC\]**
//!
//! No document type's vocabulary appears here (R8): the container, namespaces, the tolerant
//! reading architecture, style primitives, telling document types apart, line layout, the
//! projection container and what a diagnostic is. `grind-sheet` and `grind-text` each add a model,
//! a reader, a writer and an `App`. There is no trait over those `App`s yet; `observer.rs` says
//! why. `tests/generic.rs` checks R8.

use std::fmt;

pub mod atomic;
pub mod build_info;
pub mod color;
pub mod kind;
pub mod layout;
pub mod lint;
pub mod locale;
pub mod observer;
pub mod odf;
pub mod page;
pub mod projection;
pub mod search;
pub mod spell;
pub mod style;
pub mod third_party;
pub mod utf16;

pub use kind::{DocumentKind, kind};
pub use observer::Observer;
pub use odf::Form;

/// What can go wrong with an ODF document, whatever kind of document it is. Each document type's
/// own error enum wraps this one (R8).
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
    /// A projection (`doc/dsl.md` layer 0) that will not parse, or that does not declare its kind.
    Projection(String),
    /// A document whose kind this build has no reader for, or bytes that are not an ODF document
    /// at all — carrying what `kind` made of it, since the two want different words for a user.
    UnsupportedKind(Option<DocumentKind>),
    /// A save that would make the file worse, refused: the document on disk is left as it was.
    /// Carries what would have gone, spelled for a person (`2 × text:note`).
    WouldLose(Vec<String>),
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
            Error::WouldLose(what) => write!(
                f,
                "not saved: saving over this file would drop what this build cannot carry ({}) \
                 — undo the edit that changed it",
                what.join(", ")
            ),
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
