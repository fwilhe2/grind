// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! An Excel workbook arriving in the page (`doc/xlsx-import.md`, X6): imported into flat ODF
//! before either pane sees it, and renamed so the download it becomes is an ODF document.
//!
//! A page can never write over the file it was given — saving is a download — so the rename is
//! the whole of "one way in, never out" here: `budget.xlsx` goes back as `budget.fods`, never
//! as an `.xlsx` with ODF inside. No browser API is touched, so it is tested on the host.

/// An imported workbook, ready for the spreadsheet pane.
#[derive(Debug)]
pub struct Imported {
    /// The name the document now goes by — and downloads as.
    pub name: String,
    /// Flat ODF, for `App::open_bytes`.
    pub odf: Vec<u8>,
    /// The report's sentence, for the page's message line.
    pub summary: String,
}

/// `Some` when `bytes` are a workbook, imported; `None` for anything else, which goes on to
/// `grind_core::kind` as before.
pub fn workbook(name: &str, bytes: &[u8]) -> Option<Result<Imported, String>> {
    #[cfg(feature = "xlsx")]
    if grind_xlsx::sniff(bytes) {
        return Some(
            grind_xlsx::open(bytes)
                .map(|(odf, report)| Imported {
                    name: grind_xlsx::suggested_name(name),
                    odf,
                    summary: report.summary(),
                })
                .map_err(|e| format!("{name}: {e}")),
        );
    }
    let _ = (name, bytes);
    None
}

#[cfg(all(test, feature = "xlsx"))]
mod tests {
    use super::*;

    #[test]
    fn a_workbook_becomes_flat_odf_under_an_odf_name() {
        let bytes = include_bytes!("../../xlsx/tests/data/sample.xlsx");
        let imported = workbook("sample.xlsx", bytes).expect("a workbook").unwrap();
        assert_eq!(imported.name, "sample.fods");
        assert_eq!(
            grind_core::kind(&imported.odf),
            Some(grind_core::DocumentKind::Spreadsheet)
        );
        assert!(imported.summary.starts_with("Imported from Excel"));
        assert!(workbook("plain.fods", b"<office:document/>").is_none());
    }
}
