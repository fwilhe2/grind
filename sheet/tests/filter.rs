// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The autofilter against the document that asked for it: `samples/table.fods`, saved by
//! LibreOffice with one column filtered.
//!
//! `grind_sheet::filter` derives the hidden rows from the conditions rather than trusting
//! the `table:visibility="filter"` attributes in the file — so the file's attributes are
//! the oracle here, and this is the test that says the derivation agrees with LibreOffice.

use std::path::PathBuf;

fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/samples/table.fods")
}

/// The rows LibreOffice marked `table:visibility="filter"`, by counting `table:table-row`
/// elements — read out of the XML rather than named as literals, so this cannot drift if
/// the sample is re-saved.
fn marked_hidden(xml: &str) -> Vec<u32> {
    xml.split("<table:table-row")
        .skip(1)
        .enumerate()
        .filter(|(_, row)| {
            row.split('>')
                .next()
                .is_some_and(|tag| tag.contains("filter"))
        })
        .map(|(i, _)| i as u32)
        .collect()
}

#[test]
fn filter_matches_libreoffice() {
    let xml = std::fs::read_to_string(sample()).expect("sample");
    let doc = grind_sheet::read_file(&sample()).expect("loads");
    let sheet = doc.sheet(0).expect("one sheet");
    let filter = sheet.filter().expect("the sample has an autofilter");

    assert_eq!(filter.name, "__Anonymous_Sheet_DB__0");
    assert!(filter.contains_header, "the default when unwritten");
    assert!(filter.keep[&2].contains("Desk"), "a kept value");
    assert!(!filter.keep[&2].contains("Chair"), "a filtered-out value");

    let hidden = sheet.hidden_rows(doc.null_date);
    assert_eq!(
        hidden,
        marked_hidden(&xml),
        "derived vs. LibreOffice's marks"
    );
    assert!(!hidden.is_empty(), "the sample must actually hide rows");
}

/// Our own file says the same thing: the range, the values and the hidden rows all survive
/// being written and read back.
#[test]
fn a_filter_survives_our_own_round_trip() {
    let doc = grind_sheet::read_file(&sample()).expect("loads");
    let bytes = grind_sheet::write_bytes(&doc, grind_sheet::Form::Flat).expect("writes");
    let back = grind_sheet::read_bytes("out.fods", &bytes).expect("reads back");

    assert_eq!(
        back.sheet(0).expect("one sheet").filter(),
        doc.sheet(0).expect("one sheet").filter()
    );
    assert_eq!(
        back.sheet(0)
            .expect("one sheet")
            .hidden_rows(back.null_date),
        doc.sheet(0).expect("one sheet").hidden_rows(doc.null_date)
    );
    assert!(
        String::from_utf8_lossy(&bytes).contains("table:visibility=\"filter\""),
        "and the file says which rows they are"
    );
}
