// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading and writing OpenDocument — the part that is the same for every document type: packaging
//! in both forms, the reading architecture and name resolution (`doc/ods-format.md` §1, §8, §8.1).
//! **\[GENERIC\]** Each document type adds its own `read` and `write`, driven by
//! [`context::parse`].

pub mod context;
pub mod envelope;
pub mod forms;
pub mod names;
pub mod package;
pub mod xml;

use std::path::Path;

/// Which physical form a document takes (`doc/ods-format.md` §1). Reading sniffs the form from the
/// bytes; writing has to choose one.
///
/// The projection (`doc/dsl.md` §9) is the third form and a peer of the other two: it is bijective
/// with the model, so every caller that asks "which form?" gets it for free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// A zip package — `.ods`, `.odt` (§1.1).
    Package,
    /// One flat XML file — `.fods`, `.fodt` (§1.2).
    Flat,
    /// The projection — `.grind`, KDL rather than XML (`doc/dsl.md`).
    Projection,
}

impl Form {
    /// The form a path's extension asks for, and the flat form when it asks for nothing: in doubt,
    /// write the form that diffs (`doc/flat-first.md`). `.ods` and `.odt` still mean the package.
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
            // The projection is named exhaustively too, and for the same reason: `.grind` is
            // a decision, not doubt.
            Some(ext) if ext.eq_ignore_ascii_case(crate::projection::EXTENSION) => Form::Projection,
            _ => Form::Flat,
        }
    }

    /// The extension a new document of this kind is saved under, so a shell's Save As and the CLI
    /// agree (`doc/flat-first.md`).
    pub fn extension(self, kind: crate::DocumentKind) -> &'static str {
        use crate::DocumentKind::{Presentation, Spreadsheet, Text};
        match (self, kind) {
            (Form::Flat, Spreadsheet) => "fods",
            (Form::Flat, Text) => "fodt",
            (Form::Flat, Presentation) => "fodp",
            (Form::Package, Spreadsheet) => "ods",
            (Form::Package, Text) => "odt",
            (Form::Package, Presentation) => "odp",
            // One extension for every kind, because the kind is *in the file* — its first
            // line is `grind spreadsheet` (`doc/dsl.md` §3.1). The ODF forms need six
            // extensions to say the same thing.
            (Form::Projection, _) => crate::projection::EXTENSION,
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
        for projection in ["a.grind", "a.GRIND"] {
            assert_eq!(
                Form::from_path(Path::new(projection)),
                Form::Projection,
                "{projection}"
            );
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
        // One extension for both kinds, because the projection says its kind in its first line.
        assert_eq!(Form::Projection.extension(Spreadsheet), "grind");
        assert_eq!(Form::Projection.extension(Text), "grind");
        // And the pair round-trips: an extension this hands out is one `from_path` reads back.
        for kind in [Spreadsheet, Text] {
            for form in [Form::Flat, Form::Package, Form::Projection] {
                let name = format!("a.{}", form.extension(kind));
                assert_eq!(Form::from_path(Path::new(&name)), form, "{name}");
            }
        }
    }
}
