// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel workbooks and CSV: which bytes this shell imports rather than opens, and what importing
//! them hands the document.
//!
//! Every shell has an `import.rs` of its own (`doc/xlsx-import.md`, X6). This one answers in
//! **bytes**, because that is the shape `NSDocument` hands a document over in
//! (`readFromData:ofType:`), and `--render-to` reads a file the same way. A workbook or a CSV
//! comes back as flat ODF under its ODF name and marked [`Opened::untitled`]: the document that
//! opens it has no file URL, so there is nothing for Save — or autosave, decision 5 — to write
//! over the workbook with. One way in, never out, kept by construction.

use std::path::Path;

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

/// Whether these bytes are a Word document this build imports (`doc/docx-import.md`, DX6).
pub fn is_word(bytes: &[u8]) -> bool {
    #[cfg(feature = "docx")]
    return grind_docx::sniff(bytes);
    #[cfg(not(feature = "docx"))]
    {
        let _ = bytes;
        false
    }
}

/// Whether `path` is a CSV or TSV this shell opens as a new document: bytes that are neither a
/// workbook nor ODF, under a delimited name (`grind_sheet::csv::is_delimited_name`). Plain text
/// has no signature, so its name is all there is — asked last, after the bytes have spoken.
pub fn is_delimited(path: &Path, bytes: &[u8]) -> bool {
    !is_workbook(bytes)
        && grind_core::kind(bytes).is_none()
        && grind_sheet::csv::is_delimited_name(&path.display().to_string())
}

/// Whether `path` is a markdown file this shell opens as a text document of its own: bytes that
/// are not ODF, under a markdown name (`grind_text::commonmark::is_markdown_name`) — plain text
/// has no signature, so its name is all there is, asked after the bytes have spoken.
pub fn is_markdown(path: &Path, bytes: &[u8]) -> bool {
    grind_core::kind(bytes).is_none()
        && grind_text::commonmark::is_markdown_name(&path.display().to_string())
}

/// What a document opens: the bytes to hand `App::open_bytes`, the name it goes by, and whether
/// it came from somewhere it must never be saved back to.
#[derive(Debug, PartialEq, Eq)]
pub struct Opened {
    pub name: String,
    pub bytes: Vec<u8>,
    /// Imported: an untitled document, whose window says `summary` once.
    pub untitled: bool,
    /// The report's one sentence, the same in every shell (`grind_xlsx::Report::summary`, or
    /// the CSV's) — `None` for a document that was simply opened.
    pub summary: Option<String>,
}

/// Turn a file's bytes into what the document opens — a workbook or a CSV imported to flat ODF,
/// anything else passed through for the reader, whose own sniff decides the form.
pub fn open(name: &str, bytes: &[u8]) -> Result<Opened, String> {
    #[cfg(feature = "xlsx")]
    if grind_xlsx::sniff(bytes) {
        let (odf, report) = grind_xlsx::open(bytes).map_err(|error| format!("{name}: {error}"))?;
        return Ok(Opened {
            name: grind_xlsx::suggested_name(name),
            bytes: odf,
            untitled: true,
            summary: Some(report.summary()),
        });
    }
    #[cfg(feature = "docx")]
    if grind_docx::sniff(bytes) {
        let (odf, report) = grind_docx::open(bytes).map_err(|error| format!("{name}: {error}"))?;
        return Ok(Opened {
            name: grind_docx::suggested_name(name),
            bytes: odf,
            untitled: true,
            summary: Some(report.summary()),
        });
    }
    if is_delimited(Path::new(name), bytes)
        && let Some(opened) = grind_sheet::csv::open(name, bytes)
    {
        let opened = opened?;
        return Ok(Opened {
            name: opened.name,
            bytes: opened.odf,
            untitled: true,
            summary: Some(opened.summary),
        });
    }
    if is_markdown(Path::new(name), bytes)
        && let Some(opened) = grind_text::commonmark::open(
            name,
            bytes,
            // `name` is the file's path, so a picture beside it comes in too.
            &grind_text::commonmark::beside(Path::new(name)),
        )
    {
        let opened = opened?;
        return Ok(Opened {
            name: opened.name,
            bytes: opened.odf,
            untitled: true,
            summary: Some(opened.summary),
        });
    }
    Ok(Opened {
        name: name.to_owned(),
        bytes: bytes.to_vec(),
        untitled: false,
        summary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "xlsx")]
    #[test]
    fn a_workbook_is_recognised_by_its_bytes_and_opens_untitled() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../xlsx/tests/data/sample.xlsx"
        );
        let bytes = std::fs::read(path).unwrap();
        assert!(is_workbook(&bytes));
        let opened = open("budget.xlsx", &bytes).unwrap();
        assert!(opened.untitled);
        assert_eq!(opened.name, "budget.fods");
        assert_eq!(
            grind_core::kind(&opened.bytes),
            Some(grind_core::DocumentKind::Spreadsheet)
        );
        assert!(opened.summary.is_some());
    }

    #[cfg(feature = "docx")]
    #[test]
    fn a_word_document_is_recognised_by_its_bytes_and_opens_untitled() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docx/tests/data/sample.docx"
        );
        let bytes = std::fs::read(path).unwrap();
        assert!(is_word(&bytes));
        assert!(!is_workbook(&bytes));
        let opened = open("letter.docx", &bytes).unwrap();
        assert!(opened.untitled);
        assert_eq!(opened.name, "letter.fodt");
        assert_eq!(
            grind_core::kind(&opened.bytes),
            Some(grind_core::DocumentKind::Text)
        );
        assert_eq!(
            crate::document::sniff("letter.docx", &bytes),
            Ok(grind_core::DocumentKind::Text)
        );
    }

    #[test]
    fn a_delimited_file_is_recognised_by_its_name_only_after_its_bytes() {
        assert!(is_delimited(Path::new("prices.tsv"), b"item\tprice\n"));
        assert!(!is_delimited(Path::new("prices.txt"), b"item\tprice\n"));
        let odf = grind_text::write_bytes(&grind_text::Document::default(), grind_core::Form::Flat)
            .expect("a default document writes");
        assert!(
            !is_delimited(Path::new("lying.csv"), &odf),
            "ODF bytes are ODF whatever the name says"
        );
    }

    #[test]
    fn a_csv_opens_untitled_as_flat_odf() {
        let opened = open("prices.csv", b"item,price\nbread,2.5\n").unwrap();
        assert!(opened.untitled);
        assert_eq!(opened.name, "prices.fods");
        let app = grind_sheet::App::new();
        app.open_bytes(&opened.name, &opened.bytes).unwrap();
        assert_eq!(
            app.value_text(0, grind_sheet::Pos::new(1, 1)).unwrap(),
            "2.5"
        );
    }

    #[test]
    fn markdown_opens_untitled_as_a_flat_text_document() {
        let opened = open("notes.md", b"# Hi\n\ntext\n").unwrap();
        assert!(opened.untitled);
        assert_eq!(opened.name, "notes.fodt");
        assert_eq!(
            grind_core::kind(&opened.bytes),
            Some(grind_core::DocumentKind::Text)
        );
        assert!(!is_markdown(Path::new("notes.txt"), b"# Hi"));
    }

    #[test]
    fn an_odf_document_is_handed_on_as_it_is() {
        let odf = grind_text::write_bytes(&grind_text::Document::default(), grind_core::Form::Flat)
            .unwrap();
        let opened = open("note.fodt", &odf).unwrap();
        assert!(!opened.untitled);
        assert_eq!(opened.bytes, odf);
        assert_eq!(opened.summary, None);
    }
}
