// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ODF spreadsheet core.
//!
//! Phase 0: the harness exists, the code does not. `read_file` is the entry point
//! Loop A (`tests/corpus_read.rs`) drives; it is expected to fail on every corpus
//! file until Phase 2 lands.
//!
//! See `doc/ods-format.md` for the format spec this implements, and the plan in
//! `doc/plan.md` for phase order.

use std::fmt;
use std::path::Path;

pub mod formula;
pub mod numfmt;
pub mod odf;

#[derive(Debug)]
pub enum Error {
    /// Not built yet. Carries the phase that will remove it.
    Unimplemented(&'static str),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unimplemented(what) => write!(f, "unimplemented: {what}"),
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

/// A loaded spreadsheet. Grows a real model in Phase 1.
#[derive(Debug, Default)]
pub struct Document {}

/// Read an `.ods` (package) or `.fods` (flat) document.
pub fn read_file(_path: &Path) -> Result<Document> {
    Err(Error::Unimplemented("odf read — phase 2"))
}

/// Read a document from bytes. Paired with `read_file` from the start because the
/// browser has no filesystem, and this is not retrofittable later.
pub fn read_bytes(_name: &str, _bytes: &[u8]) -> Result<Document> {
    Err(Error::Unimplemented("odf read — phase 2"))
}
