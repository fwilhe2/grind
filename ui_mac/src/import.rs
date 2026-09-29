// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel workbooks and CSV: which bytes this shell imports rather than opens.
//!
//! Every shell has an `import.rs` of its own (`doc/xlsx-import.md`, X6), and this one starts as
//! only the two questions `main.rs` asks before there is a window. The rest — importing into a
//! new, **untitled** document under `grind_xlsx::suggested_name` — is M2's, and it is where
//! decision 5 makes autosave unable to write over a workbook: an untitled `NSDocument` has no
//! file to autosave into.

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

/// Whether `path` is a CSV or TSV this shell opens as a new document: bytes that are neither a
/// workbook nor ODF, under a delimited name (`grind_sheet::csv::is_delimited_name`). Plain text
/// has no signature, so its name is all there is — asked last, after the bytes have spoken.
pub fn is_delimited(path: &Path, bytes: &[u8]) -> bool {
    !is_workbook(bytes)
        && grind_core::kind(bytes).is_none()
        && grind_sheet::csv::is_delimited_name(&path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "xlsx")]
    #[test]
    fn a_workbook_is_recognised_by_its_bytes() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../xlsx/tests/data/sample.xlsx"
        ));
        assert!(is_workbook(&std::fs::read(path).unwrap()));
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
}
