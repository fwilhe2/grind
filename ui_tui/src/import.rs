// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Opening an Excel workbook (`doc/xlsx-import.md`, X6) — sniffed from the bytes, imported,
//! and opened as a new, **unsaved** ODF document with no path. `:w` then needs a name, so the
//! one command that could write ODF over the `.xlsx` it came from has nothing to write to
//! (`doc/not-doing.md` §1: one way in, never out).
//!
//! Compiled out with the `xlsx` feature; a build without it opens ODF only, as before X6.

use std::path::Path;

use grind_sheet::App;

/// What an import leaves the pane holding: the name it goes by, and the report's sentence.
#[derive(Debug, PartialEq, Eq)]
pub struct Imported {
    pub name: String,
    pub summary: String,
}

/// Whether `path` is a CSV or TSV opened as a document of its own: bytes that are neither a
/// workbook nor ODF, under a delimited name (`grind_sheet::csv::is_delimited_name` — plain text
/// has no signature, so its name is all there is).
pub fn is_delimited(path: &Path, bytes: &[u8]) -> bool {
    !is_workbook(bytes)
        && grind_core::kind(bytes).is_none()
        && grind_sheet::csv::is_delimited_name(&path.display().to_string())
}

/// Open the CSV at `path` into `app` as a new document — the same shape as a workbook: named
/// `data.fods`, with no path. Only called once [`is_delimited`] said yes.
pub fn open_delimited(app: &App, path: &Path, bytes: &[u8]) -> Result<Imported, String> {
    let file = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let opened = grind_sheet::csv::open(&file, bytes).ok_or("not a delimited file")??;
    app.open_bytes(&opened.name, &opened.odf)
        .map_err(|e| e.to_string())?;
    Ok(Imported {
        name: opened.name,
        summary: opened.summary,
    })
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

/// Import the workbook at `path` into `app`. Only called once [`is_workbook`] said yes.
pub fn open(app: &App, path: &Path, bytes: &[u8]) -> Result<Imported, String> {
    #[cfg(feature = "xlsx")]
    {
        let (odf, report) = grind_xlsx::open(bytes).map_err(|e| e.to_string())?;
        let name = grind_xlsx::suggested_name(&path.display().to_string());
        app.open_bytes(&name, &odf).map_err(|e| e.to_string())?;
        Ok(Imported {
            name,
            summary: report.summary(),
        })
    }
    #[cfg(not(feature = "xlsx"))]
    {
        let _ = (app, bytes);
        Err(format!(
            "{}: this build reads no Excel workbooks",
            path.display()
        ))
    }
}

#[cfg(all(test, feature = "xlsx"))]
mod tests {
    use super::*;

    #[test]
    fn a_workbook_imports_under_an_odf_name() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../xlsx/tests/data/sample.xlsx"
        ));
        let bytes = std::fs::read(path).unwrap();
        assert!(is_workbook(&bytes));
        let app = App::new();
        let imported = open(&app, path, &bytes).unwrap();
        assert!(imported.name.ends_with("sample.fods"), "{}", imported.name);
        let viewport = app.get_viewport(0, 0..1, 0..1).unwrap();
        assert_eq!(viewport.text(0, 0), Some("Region"));
    }
}

#[cfg(test)]
mod csv_tests {
    use super::*;

    #[test]
    fn a_csv_opens_as_an_unsaved_odf_document() {
        let path = Path::new("/somewhere/prices.csv");
        let bytes = b"item,price\nnut,0.5\n";
        assert!(is_delimited(path, bytes));
        let app = App::new();
        let imported = open_delimited(&app, path, bytes).unwrap();
        assert_eq!(imported.name, "prices.fods");
        assert!(imported.summary.starts_with("Imported from CSV"));
        assert!(!is_delimited(Path::new("notes.txt"), bytes));
    }
}
