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

use grind_sheet::{App, CellValue, Filter, Pos};

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

    let hidden = sheet.hidden_rows(doc.null_date, doc.locale.as_ref());
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
            .hidden_rows(back.null_date, back.locale.as_ref()),
        doc.sheet(0)
            .expect("one sheet")
            .hidden_rows(doc.null_date, doc.locale.as_ref())
    );
    assert!(
        String::from_utf8_lossy(&bytes).contains("table:visibility=\"filter\""),
        "and the file says which rows they are"
    );
}

/// What a dropdown lists: every distinct value, the hidden ones included, numbers by value —
/// `2,250` after `220`, which ordering by display text got backwards — and text in the model's
/// own order, since ordering it any other way is a collation decision (`doc/not-doing.md`).
#[test]
fn a_dropdown_offers_numbers_by_value_and_text_in_the_models_order() {
    let app = App::new();
    let cells = [
        CellValue::Text("Amount".into()),
        CellValue::Number(2250.0),
        CellValue::Number(220.0),
        CellValue::Text("b".into()),
        CellValue::Number(220.0),
        CellValue::Text("B".into()),
        CellValue::Empty,
        CellValue::Number(-3.5),
    ];
    for (row, value) in cells.iter().enumerate() {
        app.set_cell(0, Pos::new(row as u32, 0), value.clone())
            .expect("sets");
    }
    let filter = Filter::new("f", Pos::new(0, 0), Pos::new(7, 0));
    let viewport = app.get_viewport(0, 0..8, 0..1).expect("reads");
    assert_eq!(
        grind_sheet::filter::offered(&viewport, &filter, 0, 500),
        ["", "-3.5", "220", "2250", "B", "b"]
    );
    assert_eq!(
        grind_sheet::filter::offered(&viewport, &filter, 0, 2),
        ["", "-3.5"]
    );
}

/// A filter by **colour** (`loext:data-type`, `doc/ods-format.md` §6) is not a filter on the
/// colour's spelling as a value. Read as one, it hid every data row of LibreOffice's own
/// `autofilter-colors.ods` — rows the file leaves visible — since no cell displays `#e8f2a1`.
#[test]
fn a_filter_by_colour_hides_nothing_it_cannot_judge() {
    let fods = r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0"
 office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
<office:body><office:spreadsheet>
<table:table table:name="S"><table:table-column/>
<table:table-row><table:table-cell office:value-type="string"><text:p>Head</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell office:value-type="float" office:value="2"><text:p>2</text:p></table:table-cell></table:table-row>
</table:table>
<table:database-ranges><table:database-range table:name="__Anonymous_Sheet_DB__0"
 table:target-range-address="S.A1:S.A3" table:display-filter-buttons="true">
<table:filter><table:filter-and>
<table:filter-condition loext:data-type="background-color" table:value="#e8f2a1" table:operator="=" table:field-number="0"/>
</table:filter-and></table:filter></table:database-range></table:database-ranges>
</office:spreadsheet></office:body></office:document>"##;
    let doc = grind_sheet::read_bytes("colours.fods", fods.as_bytes()).expect("loads");
    let sheet = doc.sheet(0).expect("one sheet");
    assert!(
        sheet.filter().is_some(),
        "the range and its buttons are kept"
    );
    assert_eq!(
        sheet.hidden_rows(doc.null_date, doc.locale.as_ref()),
        Vec::<u32>::new()
    );
}
