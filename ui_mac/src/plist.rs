// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `Grind.app`'s Info.plist, generated from one table of types (M10) — portable, and printed by
//! `grind-mac --info-plist` for the bundle step to write.
//!
//! The types are the nine `ui_win32/src/assoc.rs` offers — the three ODF forms of each kind, and
//! the workbooks and delimited text this build imports — and the stance is that file's in Mac
//! terms: **offered, never taken**. Every type is at `LSHandlerRank` **Alternate**, so Grind is
//! in Open With and a double-click keeps going wherever the user sent it before — except
//! `.grind`, which nothing else opens, at **Owner**, and declared to conform to
//! `public.plain-text` so Quick Look previews a projection for free.
//!
//! An import is a **Viewer** role, because this build opens a workbook or a CSV and never writes
//! one back (`doc/xlsx-import.md`: one way in, never out); every ODF form is an **Editor**.

/// How strongly Grind claims a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rank {
    /// The one application that opens it.
    Owner,
    /// One of the applications that can; never the default by our own doing.
    Alternate,
}

/// Whether Grind writes a type back, or only reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Editor,
    Viewer,
}

/// Who declares a type's identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Declared {
    /// macOS itself.
    System,
    /// Another application may, and nothing is broken when none does — so this bundle imports
    /// the declaration, conforming to these.
    Imported(&'static [&'static str]),
    /// This bundle: nobody else has a name for it.
    Exported(&'static [&'static str]),
}

/// One type the bundle claims.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Type {
    pub name: &'static str,
    pub identifier: &'static str,
    pub extension: &'static str,
    pub mime: &'static str,
    pub rank: Rank,
    pub role: Role,
    pub declared: Declared,
}

pub const BUNDLE_ID: &str = "io.github.fwilhe2.Grind";
pub const BUNDLE_NAME: &str = "Grind";
pub const EXECUTABLE: &str = "grind-mac";
pub const ICON: &str = "Grind";
/// Decision 11's floor, as `LSMinimumSystemVersion` — `.cargo/config.toml`'s
/// `MACOSX_DEPLOYMENT_TARGET`, which a test holds this to.
pub const MINIMUM_SYSTEM: &str = "15.0";
/// The projection's identifier, which this bundle exports.
pub const PROJECTION: &str = "io.github.fwilhe2.grind.projection";

const XML: &[&str] = &["public.xml"];
const PLAIN: &[&str] = &["public.plain-text"];
const ZIP: &[&str] = &["public.zip-archive"];

pub const TYPES: &[Type] = &[
    Type {
        name: "OpenDocument Spreadsheet (flat)",
        identifier: "org.oasis-open.opendocument.spreadsheet-flat-xml",
        extension: "fods",
        mime: "application/vnd.oasis.opendocument.spreadsheet-flat-xml",
        rank: Rank::Alternate,
        role: Role::Editor,
        declared: Declared::Imported(XML),
    },
    Type {
        name: "OpenDocument Spreadsheet",
        identifier: "org.oasis-open.opendocument.spreadsheet",
        extension: "ods",
        mime: "application/vnd.oasis.opendocument.spreadsheet",
        rank: Rank::Alternate,
        role: Role::Editor,
        declared: Declared::System,
    },
    Type {
        name: "OpenDocument Text (flat)",
        identifier: "org.oasis-open.opendocument.text-flat-xml",
        extension: "fodt",
        mime: "application/vnd.oasis.opendocument.text-flat-xml",
        rank: Rank::Alternate,
        role: Role::Editor,
        declared: Declared::Imported(XML),
    },
    Type {
        name: "OpenDocument Text",
        identifier: "org.oasis-open.opendocument.text",
        extension: "odt",
        mime: "application/vnd.oasis.opendocument.text",
        rank: Rank::Alternate,
        role: Role::Editor,
        declared: Declared::System,
    },
    Type {
        name: "Grind Projection",
        identifier: PROJECTION,
        extension: "grind",
        mime: "text/x-grind",
        rank: Rank::Owner,
        role: Role::Editor,
        declared: Declared::Exported(PLAIN),
    },
    Type {
        name: "Excel Workbook",
        identifier: "org.openxmlformats.spreadsheetml.sheet",
        extension: "xlsx",
        mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        rank: Rank::Alternate,
        role: Role::Viewer,
        declared: Declared::System,
    },
    Type {
        name: "Excel Macro-Enabled Workbook",
        identifier: "com.microsoft.excel.sheet.macroenabled",
        extension: "xlsm",
        mime: "application/vnd.ms-excel.sheet.macroEnabled.12",
        rank: Rank::Alternate,
        role: Role::Viewer,
        declared: Declared::Imported(ZIP),
    },
    Type {
        name: "Comma-Separated Values",
        identifier: "public.comma-separated-values-text",
        extension: "csv",
        mime: "text/csv",
        rank: Rank::Alternate,
        role: Role::Viewer,
        declared: Declared::System,
    },
    Type {
        name: "Tab-Separated Values",
        identifier: "public.tab-separated-values-text",
        extension: "tsv",
        mime: "text/tab-separated-values",
        rank: Rank::Alternate,
        role: Role::Viewer,
        declared: Declared::System,
    },
];

/// `s` as XML character data.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn key(out: &mut String, indent: usize, name: &str) {
    out.push_str(&format!(
        "{}<key>{}</key>\n",
        "\t".repeat(indent),
        escape(name)
    ));
}

fn string(out: &mut String, indent: usize, value: &str) {
    out.push_str(&format!(
        "{}<string>{}</string>\n",
        "\t".repeat(indent),
        escape(value)
    ));
}

fn strings(out: &mut String, indent: usize, values: &[&str]) {
    let tabs = "\t".repeat(indent);
    out.push_str(&format!("{tabs}<array>\n"));
    for value in values {
        string(out, indent + 1, value);
    }
    out.push_str(&format!("{tabs}</array>\n"));
}

fn pair(out: &mut String, indent: usize, name: &str, value: &str) {
    key(out, indent, name);
    string(out, indent, value);
}

/// The declarations `UTImportedTypeDeclarations` or `UTExportedTypeDeclarations` holds.
fn declarations(out: &mut String, exported: bool) {
    let chosen: Vec<(&Type, &[&str])> = TYPES
        .iter()
        .filter_map(|ty| match (ty.declared, exported) {
            (Declared::Imported(conforms), false) | (Declared::Exported(conforms), true) => {
                Some((ty, conforms))
            }
            _ => None,
        })
        .collect();
    if chosen.is_empty() {
        return;
    }
    key(
        out,
        1,
        match exported {
            true => "UTExportedTypeDeclarations",
            false => "UTImportedTypeDeclarations",
        },
    );
    out.push_str("\t<array>\n");
    for (ty, conforms) in chosen {
        out.push_str("\t\t<dict>\n");
        pair(out, 3, "UTTypeIdentifier", ty.identifier);
        pair(out, 3, "UTTypeDescription", ty.name);
        key(out, 3, "UTTypeConformsTo");
        strings(out, 3, conforms);
        key(out, 3, "UTTypeTagSpecification");
        out.push_str("\t\t\t<dict>\n");
        key(out, 4, "public.filename-extension");
        strings(out, 4, &[ty.extension]);
        key(out, 4, "public.mime-type");
        strings(out, 4, &[ty.mime]);
        out.push_str("\t\t\t</dict>\n");
        out.push_str("\t\t</dict>\n");
    }
    out.push_str("\t</array>\n");
}

/// The whole Info.plist, for a bundle at `version`.
pub fn info_plist(version: &str) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    pair(&mut out, 1, "CFBundleIdentifier", BUNDLE_ID);
    pair(&mut out, 1, "CFBundleName", BUNDLE_NAME);
    pair(&mut out, 1, "CFBundleDisplayName", BUNDLE_NAME);
    pair(&mut out, 1, "CFBundleExecutable", EXECUTABLE);
    pair(&mut out, 1, "CFBundleIconFile", ICON);
    pair(&mut out, 1, "CFBundlePackageType", "APPL");
    pair(&mut out, 1, "CFBundleShortVersionString", version);
    pair(&mut out, 1, "CFBundleVersion", version);
    pair(&mut out, 1, "CFBundleInfoDictionaryVersion", "6.0");
    pair(&mut out, 1, "LSMinimumSystemVersion", MINIMUM_SYSTEM);
    pair(
        &mut out,
        1,
        "LSApplicationCategoryType",
        "public.app-category.productivity",
    );
    pair(&mut out, 1, "NSPrincipalClass", "NSApplication");
    key(&mut out, 1, "NSHighResolutionCapable");
    out.push_str("\t<true/>\n");
    key(&mut out, 1, "NSSupportsAutomaticTermination");
    out.push_str("\t<false/>\n");
    key(&mut out, 1, "CFBundleDocumentTypes");
    out.push_str("\t<array>\n");
    for ty in TYPES {
        out.push_str("\t\t<dict>\n");
        pair(&mut out, 3, "CFBundleTypeName", ty.name);
        pair(
            &mut out,
            3,
            "CFBundleTypeRole",
            match ty.role {
                Role::Editor => "Editor",
                Role::Viewer => "Viewer",
            },
        );
        pair(
            &mut out,
            3,
            "LSHandlerRank",
            match ty.rank {
                Rank::Owner => "Owner",
                Rank::Alternate => "Alternate",
            },
        );
        key(&mut out, 3, "LSItemContentTypes");
        strings(&mut out, 3, &[ty.identifier]);
        pair(&mut out, 3, "NSDocumentClass", "GrindDocument");
        out.push_str("\t\t</dict>\n");
    }
    out.push_str("\t</array>\n");
    declarations(&mut out, false);
    declarations(&mut out, true);
    out.push_str("</dict>\n</plist>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::Form;
    use std::path::Path;

    /// Every extension declared is one this build opens: an ODF form `Form::from_path` reads as
    /// the form it is, or an import `import.rs` recognises by name.
    #[test]
    fn every_extension_is_one_this_build_opens() {
        for ty in TYPES {
            let name = format!("doc.{}", ty.extension);
            let path = Path::new(&name);
            let expected = match ty.extension {
                "ods" | "odt" => Some(Form::Package),
                "fods" | "fodt" => Some(Form::Flat),
                "grind" => Some(Form::Projection),
                _ => None,
            };
            match expected {
                Some(form) => assert_eq!(Form::from_path(path), form, "{name}"),
                None => assert_eq!(ty.role, Role::Viewer, "{name}: an import, never written"),
            }
        }
    }

    /// Offered, never taken: only the projection, which nothing else opens, is owned.
    #[test]
    fn only_the_projection_is_owned() {
        for ty in TYPES {
            assert_eq!(
                ty.rank == Rank::Owner,
                ty.extension == "grind",
                "{}",
                ty.extension
            );
        }
        let grind = TYPES.iter().find(|ty| ty.extension == "grind").unwrap();
        assert_eq!(grind.declared, Declared::Exported(&["public.plain-text"]));
    }

    #[test]
    fn identifiers_and_extensions_are_unique() {
        let mut ids: Vec<&str> = TYPES.iter().map(|ty| ty.identifier).collect();
        let mut exts: Vec<&str> = TYPES.iter().map(|ty| ty.extension).collect();
        ids.sort_unstable();
        ids.dedup();
        exts.sort_unstable();
        exts.dedup();
        assert_eq!(ids.len(), TYPES.len());
        assert_eq!(exts.len(), TYPES.len());
    }

    /// The document controller's own type names are among the declared ones.
    #[test]
    fn the_document_types_are_declared() {
        for name in [crate::document::SHEET, crate::document::TEXT] {
            assert!(TYPES.iter().any(|ty| ty.identifier == name), "{name}");
        }
    }

    #[test]
    fn the_floor_is_the_deployment_target() {
        let config = include_str!("../../.cargo/config.toml");
        assert!(
            config.contains(&format!("MACOSX_DEPLOYMENT_TARGET = \"{MINIMUM_SYSTEM}\"")),
            "LSMinimumSystemVersion and the deployment target disagree"
        );
    }

    /// Well-formed enough for `plutil -lint`, which the runner runs: balanced, escaped, and every
    /// type in it once.
    #[test]
    fn the_plist_is_balanced_and_names_every_type() {
        let plist = info_plist("1.2.3");
        for tag in ["dict", "array", "plist"] {
            assert_eq!(
                plist.matches(&format!("<{tag}")).count(),
                plist.matches(&format!("</{tag}>")).count(),
                "{tag}"
            );
        }
        for ty in TYPES {
            assert!(plist.contains(ty.identifier), "{}", ty.identifier);
        }
        assert!(plist.contains("<string>1.2.3</string>"));
        assert!(plist.contains("<key>UTExportedTypeDeclarations</key>"));
        assert!(plist.contains("<key>UTImportedTypeDeclarations</key>"));
        assert_eq!(escape("a<b&c"), "a&lt;b&amp;c");
    }
}
