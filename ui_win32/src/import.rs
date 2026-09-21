// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Opening an Excel workbook (`doc/xlsx-import.md`, X6) or a CSV — the portable half, tested on
//! Linux like the rest of this crate's.
//!
//! A workbook is sniffed from its bytes, imported, and opened as a new **unsaved** spreadsheet
//! with no path: Save then runs Save As, seeded with the ODF name beside the workbook
//! (`budget.xlsx` → `budget.fods`), so the one command that could write ODF over the workbook it
//! came from has nowhere to write until somebody chooses (`doc/not-doing.md` §1).

use std::path::{Path, PathBuf};

/// What an import leaves the pane holding.
#[derive(Debug, PartialEq, Eq)]
pub struct Imported {
    /// Where Save As starts: the workbook's folder, under its ODF name.
    pub suggested: PathBuf,
    /// The report's sentence, for the notice bar.
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

/// Read `path` into `app`: as ODF, or — for a workbook — imported. `None` for an ODF document,
/// which keeps its path.
pub fn open(app: &grind_sheet::App, path: &Path) -> Result<Option<Imported>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    #[cfg(feature = "xlsx")]
    if grind_xlsx::sniff(&bytes) {
        let (odf, report) =
            grind_xlsx::open(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
        let suggested = PathBuf::from(grind_xlsx::suggested_name(&path.display().to_string()));
        app.open_bytes(&suggested.display().to_string(), &odf)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        return Ok(Some(Imported {
            suggested,
            summary: report.summary(),
        }));
    }
    // A CSV or TSV is the same shape as a workbook: a new, unsaved document under the ODF name
    // beside it (`grind_sheet::csv::open`). Asked only once the bytes have said they are not
    // ODF, since plain text has no signature and its name is all there is.
    if is_delimited(path, &bytes)
        && let Some(opened) = grind_sheet::csv::open(&path.display().to_string(), &bytes)
    {
        let opened = opened?;
        app.open_bytes(&opened.name, &opened.odf)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        return Ok(Some(Imported {
            suggested: PathBuf::from(opened.name),
            summary: opened.summary,
        }));
    }
    app.open_bytes(&path.display().to_string(), &bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(None)
}

/// Whether `path` is a CSV or TSV this shell opens as a new document: bytes that are neither a
/// workbook nor ODF, under a delimited name (`grind_sheet::csv::is_delimited_name`).
pub fn is_delimited(path: &Path, bytes: &[u8]) -> bool {
    !is_workbook(bytes)
        && grind_core::kind(bytes).is_none()
        && grind_sheet::csv::is_delimited_name(&path.display().to_string())
}

#[cfg(all(test, feature = "xlsx"))]
mod tests {
    use super::*;

    #[test]
    fn a_workbook_opens_unsaved_with_an_odf_name_beside_it() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../xlsx/tests/data/sample.xlsx"
        ));
        assert!(is_workbook(&std::fs::read(path).unwrap()));
        let app = grind_sheet::App::new();
        let imported = open(&app, path).unwrap().expect("a workbook");
        assert_eq!(imported.suggested, path.with_extension("fods"));
        assert!(imported.summary.starts_with("Imported from Excel"));
        let viewport = app.get_viewport(0, 0..1, 0..1).unwrap();
        assert_eq!(viewport.text(0, 0), Some("Region"));
    }
}

#[cfg(test)]
mod csv_tests {
    use super::*;

    #[test]
    fn a_csv_opens_unsaved_with_an_odf_name_beside_it() {
        let dir = std::env::temp_dir().join(format!("grind-win32-csv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prices.tsv");
        std::fs::write(&path, "item\tprice\nnut\t0.5\n").unwrap();
        let app = grind_sheet::App::new();
        let imported = open(&app, &path).unwrap().expect("a CSV is imported");
        assert_eq!(imported.suggested, path.with_extension("fods"));
        assert!(
            imported.summary.contains("tab-separated"),
            "{}",
            imported.summary
        );
        let viewport = app.get_viewport(0, 1..2, 0..2).unwrap();
        assert_eq!(viewport.text(1, 0), Some("nut"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
