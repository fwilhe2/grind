// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What opening a file means, the same in every shell: an ODF document is read as it is, and an
//! Excel workbook, a Word document, a CSV or a Markdown file is **imported** — converted to flat
//! ODF under an ODF name (`budget.xlsx` → `budget.fods`) and opened as a new, unsaved document
//! with no path, so nothing can write ODF over the file it came from.
//!
//! A workbook and a Word document are known by their bytes; CSV and Markdown, being plain text
//! with no signature, by their name, asked only after the bytes have said they are not ODF. The
//! filters are the `xlsx` and `docx` features: without one, its files are not recognised.

use grind_core::DocumentKind;
use grind_text::commonmark::Resolve;
use std::path::Path;

/// A file imported to flat ODF, for `App::open_bytes`.
#[derive(Debug, PartialEq, Eq)]
pub struct Imported {
    /// Which application opens it.
    pub kind: DocumentKind,
    /// The ODF name it goes by — the original's, with an ODF extension, directories kept.
    pub name: String,
    /// Flat ODF.
    pub odf: Vec<u8>,
    /// One sentence for a notice bar: what arrived, and what did not.
    pub summary: String,
}

/// Whether these bytes are a workbook this build imports.
pub fn is_workbook(bytes: &[u8]) -> bool {
    #[cfg(feature = "xlsx")]
    return grind_xlsx::sniff(bytes);
    #[cfg(not(feature = "xlsx"))]
    {
        let _ = bytes;
        false
    }
}

/// Whether these bytes are a Word document this build imports.
pub fn is_word(bytes: &[u8]) -> bool {
    #[cfg(feature = "docx")]
    return grind_docx::sniff(bytes);
    #[cfg(not(feature = "docx"))]
    {
        let _ = bytes;
        false
    }
}

/// Whether `name` is a CSV or TSV to import: bytes that are neither ODF nor a workbook, under a
/// delimited name (`grind_sheet::csv::is_delimited_name`).
pub fn is_delimited(name: &str, bytes: &[u8]) -> bool {
    !is_workbook(bytes)
        && grind_core::kind(bytes).is_none()
        && grind_sheet::csv::is_delimited_name(name)
}

/// Whether `name` is a Markdown file to import: bytes that are not ODF, under a Markdown name
/// (`grind_text::commonmark::is_markdown_name`).
pub fn is_markdown(name: &str, bytes: &[u8]) -> bool {
    grind_core::kind(bytes).is_none() && grind_text::commonmark::is_markdown_name(name)
}

/// Which application opens a file: an ODF document's own kind, or the kind its import makes —
/// `None` for bytes that are neither.
pub fn kind(name: &str, bytes: &[u8]) -> Option<DocumentKind> {
    grind_core::kind(bytes).or(if is_workbook(bytes) || is_delimited(name, bytes) {
        Some(DocumentKind::Spreadsheet)
    } else if is_word(bytes) || is_markdown(name, bytes) {
        Some(DocumentKind::Text)
    } else {
        None
    })
}

/// Import a file, or `None` when it is not one to import — ODF, or anything unknown, for the
/// caller to read as it is. `name` is what the result is named after, and is also how CSV and
/// Markdown are recognised; `resolve` finds a Markdown file's pictures
/// (`grind_text::commonmark::beside` for a file on disk, `nowhere` otherwise).
pub fn import(name: &str, bytes: &[u8], resolve: &Resolve<'_>) -> Option<Result<Imported, String>> {
    #[cfg(feature = "xlsx")]
    if grind_xlsx::sniff(bytes) {
        return Some(
            grind_xlsx::open(bytes)
                .map(|(odf, report)| Imported {
                    kind: DocumentKind::Spreadsheet,
                    name: grind_xlsx::suggested_name(name),
                    odf,
                    summary: report.summary(),
                })
                .map_err(|e| format!("{name}: {e}")),
        );
    }
    #[cfg(feature = "docx")]
    if grind_docx::sniff(bytes) {
        return Some(
            grind_docx::open(bytes)
                .map(|(odf, report)| Imported {
                    kind: DocumentKind::Text,
                    name: grind_docx::suggested_name(name),
                    odf,
                    summary: report.summary(),
                })
                .map_err(|e| format!("{name}: {e}")),
        );
    }
    if grind_core::kind(bytes).is_some() {
        return None;
    }
    if let Some(opened) = grind_sheet::csv::open(name, bytes) {
        return Some(opened.map(|o| Imported {
            kind: DocumentKind::Spreadsheet,
            name: o.name,
            odf: o.odf,
            summary: o.summary,
        }));
    }
    grind_text::commonmark::open(name, bytes, resolve).map(|opened| {
        opened.map(|o| Imported {
            kind: DocumentKind::Text,
            name: o.name,
            odf: o.odf,
            summary: o.summary,
        })
    })
}

/// [`import`] for a file on disk: named after its path, a Markdown file's pictures found beside
/// it.
pub fn import_file(path: &Path, bytes: &[u8]) -> Option<Result<Imported, String>> {
    import(
        &path.display().to_string(),
        bytes,
        &grind_text::commonmark::beside(path),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::commonmark::nowhere;

    fn flat_text() -> Vec<u8> {
        grind_text::write_bytes(&grind_text::Document::default(), grind_core::Form::Flat).unwrap()
    }

    #[cfg(feature = "xlsx")]
    #[test]
    fn a_workbook_is_known_by_its_bytes_and_imports_as_a_spreadsheet() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../xlsx/tests/data/sample.xlsx"
        ))
        .unwrap();
        assert!(is_workbook(&bytes));
        assert_eq!(kind("budget.xlsx", &bytes), Some(DocumentKind::Spreadsheet));
        let imported = import("budget.xlsx", &bytes, &nowhere).unwrap().unwrap();
        assert_eq!(imported.kind, DocumentKind::Spreadsheet);
        assert_eq!(imported.name, "budget.fods");
        assert_eq!(
            grind_core::kind(&imported.odf),
            Some(DocumentKind::Spreadsheet)
        );
        let app = grind_sheet::App::new();
        app.open_bytes(&imported.name, &imported.odf).unwrap();
        assert_eq!(
            app.get_viewport(0, 0..1, 0..1).unwrap().text(0, 0),
            Some("Region")
        );
    }

    #[cfg(feature = "docx")]
    #[test]
    fn a_word_document_is_known_by_its_bytes_and_imports_as_text() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docx/tests/data/sample.docx"
        ))
        .unwrap();
        assert!(is_word(&bytes));
        assert!(!is_workbook(&bytes), "a document is not a workbook");
        assert_eq!(kind("letter.docx", &bytes), Some(DocumentKind::Text));
        let imported = import("/somewhere/letter.docx", &bytes, &nowhere)
            .unwrap()
            .unwrap();
        assert_eq!(imported.kind, DocumentKind::Text);
        assert_eq!(imported.name, "/somewhere/letter.fodt");
        assert!(imported.summary.starts_with("Imported from Word"));
        let core = grind_text::App::new();
        core.open_bytes(&imported.name, &imported.odf).unwrap();
        assert!(core.outline().iter().any(|h| h.text == "Field Notes"));
    }

    #[test]
    fn a_csv_is_known_by_its_name_only_after_its_bytes() {
        assert!(is_delimited("prices.tsv", b"item\tprice\n"));
        assert!(!is_delimited("prices.txt", b"item\tprice\n"));
        assert!(
            !is_delimited("lying.csv", &flat_text()),
            "ODF bytes are ODF whatever the name"
        );
        let imported = import(
            "/somewhere/prices.csv",
            b"item,price\nbread,2.5\n",
            &nowhere,
        )
        .unwrap()
        .unwrap();
        assert_eq!(imported.kind, DocumentKind::Spreadsheet);
        assert_eq!(imported.name, "/somewhere/prices.fods");
        assert!(imported.summary.starts_with("Imported from CSV"));
        let app = grind_sheet::App::new();
        app.open_bytes(&imported.name, &imported.odf).unwrap();
        assert_eq!(
            app.value_text(0, grind_sheet::Pos::new(1, 1)).unwrap(),
            "2.5"
        );
    }

    #[test]
    fn markdown_imports_as_a_text_document() {
        assert!(is_markdown("notes.md", b"# Hi"));
        assert!(!is_markdown("notes.txt", b"# Hi"));
        assert_eq!(kind("notes.md", b"# Hi"), Some(DocumentKind::Text));
        let imported = import("/somewhere/notes.md", b"# Hi\n\ntext\n", &nowhere)
            .unwrap()
            .unwrap();
        assert_eq!(imported.kind, DocumentKind::Text);
        assert_eq!(imported.name, "/somewhere/notes.fodt");
        assert!(imported.summary.starts_with("Imported from Markdown"));
        let core = grind_text::App::new();
        core.open_bytes(&imported.name, &imported.odf).unwrap();
        assert_eq!(core.block_count(), 2);
    }

    #[test]
    fn odf_and_unknown_bytes_are_not_imported() {
        assert_eq!(import("note.fodt", &flat_text(), &nowhere), None);
        assert_eq!(import("lying.csv", &flat_text(), &nowhere), None);
        assert_eq!(import("notes.txt", b"plain", &nowhere), None);
        assert_eq!(kind("note.fodt", &flat_text()), Some(DocumentKind::Text));
        assert_eq!(kind("notes.txt", b"plain"), None);
    }
}
