// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The page a document says it is set on (`doc/pdf-export.md` P2): read from the master page,
//! never written, and absent when the document states none.

use std::path::Path;

use grind_core::page::PageGeometry;
use grind_text::{Form, odf, read_bytes, read_file};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  office:mimetype="application/vnd.oasis.opendocument.text""#;

fn flat(styles: &str) -> Vec<u8> {
    format!(
        "<office:document {NS}>{styles}<office:body><office:text><text:p>Hi</text:p>\
         </office:text></office:body></office:document>"
    )
    .into_bytes()
}

fn data(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

#[test]
fn the_standard_master_page_gives_the_document_its_page() {
    let doc = read_file(&data("numbered-list.fodt")).unwrap();
    let page = doc.page.expect("Writer always writes a page layout");
    assert_eq!(page.iso_name().as_deref(), Some("A4"));
    assert!((page.left - 20.0).abs() < 1e-9);
}

#[test]
fn a_documents_own_us_letter_is_kept() {
    let doc = read_file(&data("picture.fodt")).unwrap();
    let page = doc.page.unwrap();
    assert!((page.width - 215.9).abs() < 1e-6);
    assert!((page.height - 279.4).abs() < 1e-6);
    assert!((page.top - 19.99996).abs() < 1e-3);
}

#[test]
fn a_document_that_states_no_page_has_none_and_prints_on_a4() {
    let doc = read_bytes("x.fodt", &flat("")).unwrap();
    assert_eq!(doc.page, None);
    assert_eq!(doc.page.unwrap_or_default(), PageGeometry::a4());
}

#[test]
fn the_master_page_named_standard_wins_over_the_first_one() {
    let doc = read_bytes(
        "x.fodt",
        &flat(
            r#"<office:automatic-styles>
              <style:page-layout style:name="small"><style:page-layout-properties fo:page-width="10cm" fo:page-height="10cm"/></style:page-layout>
              <style:page-layout style:name="big"><style:page-layout-properties fo:page-width="42cm" fo:page-height="59.4cm"/></style:page-layout>
            </office:automatic-styles>
            <office:master-styles>
              <style:master-page style:name="Other" style:page-layout-name="small"/>
              <style:master-page style:name="Standard" style:page-layout-name="big"/>
            </office:master-styles>"#,
        ),
    )
    .unwrap();
    assert_eq!(doc.page.unwrap().iso_name().as_deref(), Some("A2"));
}

#[test]
fn with_no_standard_the_first_master_page_is_the_page() {
    let doc = read_bytes(
        "x.fodt",
        &flat(
            r#"<office:master-styles>
              <style:master-page style:name="Only" style:page-layout-name="pl"/>
            </office:master-styles>
            <office:automatic-styles>
              <style:page-layout style:name="pl"><style:page-layout-properties fo:page-width="14.8cm" fo:page-height="21cm" fo:margin="1cm"/></style:page-layout>
            </office:automatic-styles>"#,
        ),
    )
    .unwrap();
    let page = doc.page.unwrap();
    assert_eq!(page.iso_name().as_deref(), Some("A5"));
    assert_eq!(page.text_width(), 128.0);
}

#[test]
fn a_master_page_naming_no_layout_that_exists_is_no_page() {
    let doc = read_bytes(
        "x.fodt",
        &flat(
            r#"<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="missing"/></office:master-styles>"#,
        ),
    )
    .unwrap();
    assert_eq!(doc.page, None);
}

#[test]
fn the_package_form_reads_its_page_from_styles_xml() {
    let doc = read_file(&data("picture.odt")).unwrap();
    assert!((doc.page.unwrap().width - 215.9).abs() < 1e-6);
}

#[test]
fn reading_the_page_changes_nothing_on_save() {
    for name in ["numbered-list.fodt", "picture.fodt"] {
        let bytes = std::fs::read(data(name)).unwrap();
        let doc = read_bytes(name, &bytes).unwrap();
        assert_eq!(odf::write(&doc, Form::Flat).unwrap(), bytes, "{name}");
    }
}
