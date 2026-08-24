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
    /// The form a path's extension asks for — **and the flat form when it asks for nothing.**
    ///
    /// A hint from a filesystem is the right input *here* and the wrong one for reading: a
    /// caller writing to `report.fodt` has said which form it wants, whereas a caller reading
    /// `report.xml` has said nothing at all.
    ///
    /// The default is the decision in `doc/flat-first.md`, and it is a product decision rather
    /// than a technical one: **in doubt, write the form that diffs.** `.ods` and `.odt` still
    /// mean the package, because naming one is not doubt — but a path with no extension, or one
    /// nothing here recognises, gets flat XML. A zip in a repository is one opaque blob per
    /// commit; the same document flat is a file review can read.
    pub fn from_path(path: &Path) -> Form {
        match path.extension().and_then(|e| e.to_str()) {
            // The package extensions, named exhaustively. Everything else — including no
            // extension at all — falls through to flat, which is the point.
            Some(ext)
                if ext.eq_ignore_ascii_case("ods")
                    || ext.eq_ignore_ascii_case("odt")
                    || ext.eq_ignore_ascii_case("odp")
                    || ext.eq_ignore_ascii_case("zip") =>
            {
                Form::Package
            }
            _ => Form::Flat,
        }
    }

    /// The extension a *new* document of this kind should be saved under.
    ///
    /// One place, so that a shell offering "Save As" and the CLI creating a file cannot disagree
    /// about what an unnamed document is called (`doc/flat-first.md`).
    pub fn extension(self, kind: crate::DocumentKind) -> &'static str {
        use crate::DocumentKind::{Presentation, Spreadsheet, Text};
        match (self, kind) {
            (Form::Flat, Spreadsheet) => "fods",
            (Form::Flat, Text) => "fodt",
            (Form::Flat, Presentation) => "fodp",
            (Form::Package, Spreadsheet) => "ods",
            (Form::Package, Text) => "odt",
            (Form::Package, Presentation) => "odp",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_extensions_are_named_and_everything_else_is_flat() {
        for package in ["a.ods", "a.odt", "a.odp", "a.ODS", "a.zip"] {
            assert_eq!(
                Form::from_path(Path::new(package)),
                Form::Package,
                "{package}"
            );
        }
        for flat in ["a.fods", "a.fodt", "a.FODT", "a.xml"] {
            assert_eq!(Form::from_path(Path::new(flat)), Form::Flat, "{flat}");
        }
    }

    /// `doc/flat-first.md`: naming `.ods` is a decision and gets the package; naming nothing is
    /// doubt, and doubt gets the form that diffs.
    #[test]
    fn a_path_that_asks_for_nothing_gets_the_flat_form() {
        for undecided in ["a", "report", "a.", "a.odss", "a.txt", "a.tar.gz"] {
            assert_eq!(
                Form::from_path(Path::new(undecided)),
                Form::Flat,
                "{undecided}"
            );
        }
    }

    #[test]
    fn every_form_and_kind_has_the_extension_a_new_document_takes() {
        use crate::DocumentKind::{Spreadsheet, Text};
        assert_eq!(Form::Flat.extension(Spreadsheet), "fods");
        assert_eq!(Form::Flat.extension(Text), "fodt");
        assert_eq!(Form::Package.extension(Spreadsheet), "ods");
        assert_eq!(Form::Package.extension(Text), "odt");
        // And the pair round-trips: an extension this hands out is one `from_path` reads back.
        for kind in [Spreadsheet, Text] {
            for form in [Form::Flat, Form::Package] {
                let name = format!("a.{}", form.extension(kind));
                assert_eq!(Form::from_path(Path::new(&name)), form, "{name}");
            }
        }
    }
}
