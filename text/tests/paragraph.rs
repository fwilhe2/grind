// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A paragraph's style resolved for layout (`doc/pdf-export.md` P5): the block's own style, its
//! `style:parent-style-name` chain, and the default paragraph style under all of them — read, and
//! never written.

use std::path::Path;

use grind_text::{App, read_bytes};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  office:mimetype="application/vnd.oasis.opendocument.text""#;

fn app(styles: &str, body: &str) -> App {
    let bytes = format!(
        "<office:document {NS}>{styles}<office:body><office:text>{body}</office:text></office:body></office:document>"
    );
    let app = App::new();
    app.open_bytes("x.fodt", bytes.as_bytes()).unwrap();
    app
}

const CHAIN: &str = r#"
<office:font-face-decls><style:font-face style:name="LS" svg:font-family="'Liberation Sans'"/></office:font-face-decls>
<office:styles>
  <style:default-style style:family="paragraph">
    <style:paragraph-properties fo:widows="2" fo:orphans="2"/>
    <style:text-properties fo:font-size="12pt"/>
  </style:default-style>
  <style:style style:name="Standard" style:family="paragraph">
    <style:text-properties style:font-name="LS"/>
  </style:style>
  <style:style style:name="Heading" style:family="paragraph" style:parent-style-name="Standard">
    <style:paragraph-properties fo:margin-top="0.423cm" fo:margin-bottom="0.212cm" fo:keep-with-next="always"/>
    <style:text-properties fo:font-size="14pt"/>
  </style:style>
  <style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:parent-style-name="Heading">
    <style:text-properties fo:font-size="130%" fo:font-weight="bold"/>
  </style:style>
  <style:style style:name="Loop_A" style:family="paragraph" style:parent-style-name="Loop_B"/>
  <style:style style:name="Loop_B" style:family="paragraph" style:parent-style-name="Loop_A"/>
</office:styles>
<office:automatic-styles>
  <style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard">
    <style:paragraph-properties fo:text-align="center" fo:text-indent="1cm"/>
  </style:style>
</office:automatic-styles>"#;

#[test]
fn a_heading_takes_its_chain_and_a_percentage_is_of_its_parents_size() {
    let app = app(
        CHAIN,
        r#"<text:h text:style-name="Heading_20_1" text:outline-level="1">Title</text:h>"#,
    );
    let props = app.paragraph(0).unwrap().props;
    assert_eq!(
        props.font_size.as_deref(),
        Some("18.2pt"),
        "130% of Heading's 14pt"
    );
    assert_eq!(props.font_weight.as_deref(), Some("bold"));
    assert_eq!(
        props.font_family.as_deref(),
        Some("Liberation Sans"),
        "through font-face-decls"
    );
    assert_eq!(props.margin_top.as_deref(), Some("0.423cm"));
    assert_eq!(props.keep_with_next.as_deref(), Some("always"));
    assert_eq!(
        props.widows.as_deref(),
        Some("2"),
        "from the default style at the root"
    );
}

#[test]
fn an_automatic_style_layers_over_its_named_parent() {
    let app = app(CHAIN, r#"<text:p text:style-name="P1">centred</text:p>"#);
    let props = app.paragraph(0).unwrap().props;
    assert!(app.paragraph(0).unwrap().declared);
    assert_eq!(props.text_align.as_deref(), Some("center"));
    assert_eq!(props.text_indent.as_deref(), Some("1cm"));
    assert_eq!(props.font_family.as_deref(), Some("Liberation Sans"));
    assert_eq!(props.font_size.as_deref(), Some("12pt"));
}

#[test]
fn a_paragraph_naming_no_style_has_the_default_styles_props() {
    let app = app(CHAIN, "<text:p>plain</text:p>");
    let props = app.paragraph(0).unwrap().props;
    assert_eq!(props.font_size.as_deref(), Some("12pt"));
    assert_eq!(props.text_align, None);
}

#[test]
fn a_style_naming_nothing_declared_or_a_cycle_resolves_without_looping() {
    let app = app(
        CHAIN,
        r#"<text:p text:style-name="Nowhere">a</text:p><text:p text:style-name="Loop_A">b</text:p>"#,
    );
    assert_eq!(
        app.paragraph(0).unwrap().props.font_size.as_deref(),
        Some("12pt")
    );
    assert_eq!(
        app.paragraph(1).unwrap().props.font_size.as_deref(),
        Some("12pt")
    );
    assert!(app.paragraph(2).is_none(), "no third block");
}

#[test]
fn a_document_with_no_styles_resolves_to_nothing_stated() {
    let app = app("", "<text:p>a</text:p>");
    assert_eq!(app.paragraph(0).unwrap().props, Default::default());
    assert!(!app.paragraph(0).unwrap().declared);
}

#[test]
fn writers_own_title_is_centred_by_its_style() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/edited-default-paragraph-style.fodt");
    let doc = read_bytes("x.fodt", &std::fs::read(&path).unwrap()).unwrap();
    let title = doc
        .blocks
        .iter()
        .position(|b| b.style.as_deref() == Some("Title"))
        .unwrap();
    let app = App::new();
    app.open_bytes("x.fodt", &std::fs::read(&path).unwrap())
        .unwrap();
    let props = app.paragraph(title).unwrap().props;
    assert_eq!(props.text_align.as_deref(), Some("center"));
    assert_eq!(
        props.margin_top.as_deref(),
        Some("0.423cm"),
        "from Heading, its parent"
    );
}

/// `style:font-family-generic` on a font declaration says what kind of face a family is — what a
/// printer falls back on when the family itself is not installed.
#[test]
fn a_fonts_generic_family_is_read_from_its_declaration() {
    let app = app(
        r#"<office:font-face-decls>
             <style:font-face style:name="Adwaita Sans" svg:font-family="'Adwaita Sans'" style:font-family-generic="swiss"/>
             <style:font-face style:name="Odd" svg:font-family="Odd"/>
           </office:font-face-decls>"#,
        "<text:p>a</text:p>",
    );
    let generics = app.font_generics();
    assert_eq!(
        generics.get("Adwaita Sans").map(String::as_str),
        Some("swiss")
    );
    assert_eq!(generics.get("Odd"), None);
}
