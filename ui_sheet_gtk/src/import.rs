// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Opening a file that may be an Excel workbook (`doc/xlsx-import.md`, X6) — with no GTK in
//! it, so the rule is testable with no display.
//!
//! The Open dialog imports **transparently**: a workbook is sniffed from its bytes, never its
//! name (`grind_xlsx::sniff`, as `grind_core::kind` is), imported, and opened as a **new,
//! unsaved ODF document** under the name `grind_xlsx::suggested_name` gives it — `budget.xlsx`
//! comes up as `budget.fods`, with no path. That last part is the one that matters: one way
//! in and never out (`doc/not-doing.md` §1), so Save on an imported workbook asks where to put
//! the ODF document rather than writing it over the `.xlsx` it came from.
//!
//! Compiled out with the `xlsx` feature, in which case every file is opened as ODF and a
//! workbook fails to open with the reader's own error — the same as before X6.

use std::path::{Path, PathBuf};

use grind_sheet::App;

/// What a successful open left the window holding.
#[derive(Debug, PartialEq, Eq)]
pub struct Opened {
    /// Where Save writes: the file's own path for an ODF document, `None` for an imported
    /// workbook, which has never been saved.
    pub path: Option<PathBuf>,
    /// Set when the file was a workbook: the name to show and offer, and the report's sentence.
    pub imported: Option<Imported>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Imported {
    pub name: String,
    pub summary: String,
}

/// Whether these bytes are a workbook this build can import.
pub fn is_workbook(bytes: &[u8]) -> bool {
    #[cfg(feature = "xlsx")]
    return grind_xlsx::sniff(bytes);
    #[cfg(not(feature = "xlsx"))]
    {
        let _ = bytes;
        false
    }
}

/// Open `bytes`, read from `path`, into `app`: as ODF, or as an imported workbook.
pub fn open(app: &App, path: &Path, bytes: &[u8]) -> Result<Opened, String> {
    #[cfg(feature = "xlsx")]
    if grind_xlsx::sniff(bytes) {
        let (odf, report) = grind_xlsx::open(bytes).map_err(|e| e.to_string())?;
        let name = grind_xlsx::suggested_name(&document_name(path));
        app.open_bytes(&name, &odf).map_err(|e| e.to_string())?;
        return Ok(Opened {
            path: None,
            imported: Some(Imported {
                name,
                summary: report.summary(),
            }),
        });
    }
    app.open_bytes(&path.display().to_string(), bytes)
        .map_err(|e| e.to_string())?;
    Ok(Opened {
        path: Some(path.to_owned()),
        imported: None,
    })
}

#[cfg(feature = "xlsx")]
fn document_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "Untitled".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(all(test, feature = "xlsx"))]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../xlsx/tests/data/sample.xlsx"
    );

    /// The exit criterion's first half, with no window: a workbook opens as an unsaved ODF
    /// document named after it, holding its cells.
    #[test]
    fn a_workbook_opens_as_an_unsaved_odf_document() {
        let app = App::new();
        let path = Path::new(SAMPLE);
        let bytes = std::fs::read(path).unwrap();
        assert!(is_workbook(&bytes));
        let opened = open(&app, path, &bytes).expect("it imports");
        assert_eq!(opened.path, None, "Save must not write over the workbook");
        let imported = opened.imported.expect("it was a workbook");
        assert_eq!(imported.name, "sample.fods");
        assert!(imported.summary.starts_with("Imported from Excel"));
        let viewport = app.get_viewport(0, 0..1, 0..1).unwrap();
        assert_eq!(viewport.text(0, 0), Some("Region"));
    }

    #[test]
    fn an_odf_document_opens_where_it_is() {
        let dir = std::env::temp_dir().join("grind-sheet-gtk-import");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plain.fods");
        let seed = App::new();
        seed.save_file(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(!is_workbook(&bytes));
        let opened = open(&App::new(), &path, &bytes).unwrap();
        assert_eq!(opened.path.as_deref(), Some(path.as_path()));
        assert_eq!(opened.imported, None);
    }
}
