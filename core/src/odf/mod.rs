// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading and writing OpenDocument — the part that is the same for every document type.
//! **\[GENERIC\]**
//!
//! `doc/ods-format.md` marks its own sections `[GENERIC]` or `[ODS]`, and this module is the
//! first set: §1 (packaging, both physical forms), §8 (the reading architecture) and §8.1
//! (name resolution). None of it knows what a cell or a paragraph is.
//!
//! What each document type adds is a `read` and a `write` of its own, driven by
//! [`context::parse`] over contexts it defines — `grind_sheet::odf` is the worked example, and
//! §10 is the note that said this would happen.

pub mod context;
pub mod names;
pub mod package;
pub mod xml;

use std::path::Path;

/// Which physical form a document takes (doc/ods-format.md §1).
///
/// Reading sniffs the form from the bytes, because an extension is a hint from a filesystem
/// rather than a fact about the data. Writing has to *choose* one, so this is the only place
/// the distinction is an input rather than an observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// A zip package — `.ods`, `.odt` (§1.1).
    Package,
    /// One flat XML file — `.fods`, `.fodt` (§1.2).
    Flat,
}

impl Form {
    /// The form a path's extension asks for, defaulting to the package form.
    ///
    /// A hint from a filesystem is the right input *here* and the wrong one for reading: a
    /// caller writing to `report.fodt` has said which form it wants, whereas a caller reading
    /// `report.xml` has said nothing at all.
    pub fn from_path(path: &Path) -> Form {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext)
                if ext.eq_ignore_ascii_case("fods")
                    || ext.eq_ignore_ascii_case("fodt")
                    || ext.eq_ignore_ascii_case("fodp")
                    || ext.eq_ignore_ascii_case("xml") =>
            {
                Form::Flat
            }
            _ => Form::Package,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flat_extensions_are_the_f_prefixed_ones_and_bare_xml() {
        for flat in ["a.fods", "a.fodt", "a.FODT", "a.xml"] {
            assert_eq!(Form::from_path(Path::new(flat)), Form::Flat, "{flat}");
        }
        for package in ["a.ods", "a.odt", "a", "a.zip"] {
            assert_eq!(
                Form::from_path(Path::new(package)),
                Form::Package,
                "{package}"
            );
        }
    }
}
