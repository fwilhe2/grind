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

/// `doc/odt-format.md` §5c fact 3: widows and orphans are applied only where a document states
/// them, and Writer states them on its default paragraph style.
#[test]
fn the_default_paragraph_styles_widows_and_orphans_are_read() {
    let opened = |bytes: &[u8]| {
        let app = grind_text::App::new();
        app.open_bytes("x.fodt", bytes).unwrap();
        let stated = app.paragraph_defaults();
        (stated.widows, stated.orphans)
    };
    let file = |name: &str| std::fs::read(data(name)).unwrap();
    assert_eq!(
        opened(&file("edited-default-paragraph-style.fodt")),
        (Some(2), Some(2))
    );
    assert_eq!(opened(&file("numbered-list.fodt")), (None, None));
    let made = flat(
        r#"<office:styles><style:default-style style:family="paragraph"><style:paragraph-properties fo:widows="3" fo:orphans="nonsense"/></style:default-style>
           <style:default-style style:family="table"><style:paragraph-properties fo:widows="9"/></style:default-style></office:styles>"#,
    );
    assert_eq!(opened(&made), (Some(3), None));
}

const MARGINALS: &str = r#"<office:automatic-styles>
  <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/>
    <style:header-style><style:header-footer-properties fo:min-height="0.1cm" fo:margin-bottom="0.5cm"/></style:header-style>
    <style:footer-style><style:header-footer-properties fo:margin-top="0.4cm"/></style:footer-style>
  </style:page-layout>
</office:automatic-styles>
<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1">
  <style:header><text:p text:style-name="Header">The <text:span>head</text:span><text:tab/>line</text:p></style:header>
  <style:footer><text:p text:style-name="Footer">pg <text:page-number text:select-page="current">7</text:page-number> of <text:page-count>9</text:page-count></text:p></style:footer>
</style:master-page></office:master-styles>"#;

/// `doc/odt-format.md` §5c fact 9: the master page's header and footer, their paragraphs as text
/// and fields, and the room the page layout gives them — read, and never written.
#[test]
fn the_master_pages_header_and_footer_are_read() {
    use grind_text::marginal::Part;
    let doc = read_bytes("x.fodt", &flat(MARGINALS)).unwrap();
    let header = doc.header.expect("a header");
    assert_eq!(header.paragraphs.len(), 1);
    assert_eq!(header.paragraphs[0].style.as_deref(), Some("Header"));
    assert_eq!(
        header.paragraphs[0].parts,
        vec![Part::Text("The head\tline".into())]
    );
    assert!((header.min_height - 1.0).abs() < 1e-9 && (header.spacing - 5.0).abs() < 1e-9);
    let footer = doc.footer.expect("a footer");
    assert_eq!(
        footer.paragraphs[0].parts,
        vec![
            Part::Text("pg ".into()),
            Part::PageNumber,
            Part::Text(" of ".into()),
            Part::PageCount,
        ]
    );
    assert!((footer.spacing - 4.0).abs() < 1e-9 && footer.min_height == 0.0);
}

#[test]
fn a_document_with_no_header_or_footer_has_none_and_saves_unchanged() {
    let doc = read_bytes("x.fodt", &flat("")).unwrap();
    assert!(doc.header.is_none() && doc.footer.is_none());
    let bytes = flat(MARGINALS);
    let doc = read_bytes("x.fodt", &bytes).unwrap();
    assert_eq!(odf::write(&doc, Form::Flat).unwrap(), bytes);
}

/// A field's own text is its cached value, and it is what a reader of the page sees: Writer's
/// `Figure <text:sequence>1</text:sequence>.` reads "Figure 1.". Shown and never flattened: a
/// save that would turn the field into plain text is refused, never written.
#[test]
fn a_fields_cached_text_is_shown_and_never_flattened_on_save() {
    let bytes = flat(
        r#"<office:body><office:text><text:p>Figure <text:sequence text:name="Illustration" text:formula="ooow:Figure">1</text:sequence>. Caption.</text:p><text:p>Dated <text:date>4 October 2026</text:date> and <text:author-name>Max</text:author-name>.</text:p></office:text></office:body>"#,
    );
    // `flat` puts the body in itself; this document is its own body.
    let bytes = String::from_utf8(bytes).unwrap().replace("<office:body><office:text><text:p>Hi</text:p></office:text></office:body></office:document>", "</office:document>").into_bytes();
    let app = grind_text::App::new();
    app.open_bytes("f.fodt", &bytes).unwrap();
    let view = app.get_viewport(0..app.block_count());
    let texts: Vec<String> = view.iter().map(|b| b.text.clone()).collect();
    assert_eq!(
        texts,
        vec!["Figure 1. Caption.", "Dated 4 October 2026 and Max."]
    );
    assert_eq!(
        app.save_bytes(Form::Flat).unwrap(),
        bytes,
        "untouched, it saves as it was"
    );
    app.insert_text(
        grind_text::Caret {
            block: 0,
            offset: 0,
        },
        "A ",
    )
    .unwrap();
    let saved = app.save_bytes(Form::Flat);
    match saved {
        Err(grind_text::Error::WouldLose(_)) => {}
        Ok(bytes) => assert!(
            String::from_utf8_lossy(&bytes).contains("<text:sequence"),
            "the field survives"
        ),
        Err(other) => panic!("{other}"),
    }
}
