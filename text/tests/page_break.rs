// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A page break made here (`doc/page-model.md`): Ctrl+Enter splits the paragraph and starts a
//! page with the second part, written as Writer writes it — an automatic paragraph style over the
//! paragraph's own with `fo:break-before="page"` (`doc/odt-format.md` §5c fact 58).

use grind_text::{App, BlockKind, Caret, Form};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#;

fn open(automatic: &str, body: &str) -> App {
    let bytes = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document {NS} office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text"><office:styles><style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph"/></office:styles><office:automatic-styles>{automatic}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document>"#
    );
    let app = App::new();
    app.open_bytes("a.fodt", bytes.as_bytes()).unwrap();
    app
}

fn reopened(app: &App) -> (App, String) {
    let bytes = app.save_bytes(Form::Flat).unwrap();
    let back = App::new();
    back.open_bytes("a.fodt", &bytes).unwrap();
    (back, String::from_utf8(bytes).unwrap())
}

fn texts(app: &App) -> Vec<String> {
    app.get_viewport(0..app.block_count())
        .iter()
        .map(|block| block.text.clone())
        .collect()
}

#[test]
fn ctrl_enter_splits_and_starts_a_page_with_the_second_part() {
    let app = open("", r#"<text:p text:style-name="Standard">onetwo</text:p>"#);
    app.break_page(Caret {
        block: 0,
        offset: 3,
    })
    .unwrap();
    assert_eq!(texts(&app), ["one", "two"]);
    assert!(!app.starts_page(0));
    assert!(app.starts_page(1));

    let (back, xml) = reopened(&app);
    assert_eq!(texts(&back), ["one", "two"]);
    assert!(back.starts_page(1), "{xml}");
    assert!(
        xml.contains(r#"style:parent-style-name="Standard"><style:paragraph-properties fo:break-before="page"/>"#),
        "{xml}"
    );

    // One undo step takes both the split and the break back.
    assert!(app.undo());
    assert_eq!(texts(&app), ["onetwo"]);
    assert!(!app.starts_page(0));
}

#[test]
fn a_break_keeps_what_the_paragraphs_own_automatic_style_says() {
    let app = open(
        r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="2cm"/></style:style>"#,
        r#"<text:p>one</text:p><text:p text:style-name="P1">two</text:p>"#,
    );
    app.set_page_break(1, true).unwrap();
    let (back, xml) = reopened(&app);
    assert!(back.starts_page(1), "{xml}");
    let props = back.paragraph(1).unwrap().props;
    assert_eq!(props.margin_left.as_deref(), Some("2cm"), "{xml}");
    assert_eq!(texts(&back), ["one", "two"]);
}

#[test]
fn writers_own_page_break_comes_out_with_its_style() {
    let app = open(
        r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:break-before="page"/></style:style>"#,
        r#"<text:p text:style-name="Standard">one</text:p><text:p text:style-name="P1">two</text:p>"#,
    );
    assert!(app.starts_page(1));
    app.set_page_break(1, false).unwrap();
    assert!(!app.starts_page(1));
    let (back, _) = reopened(&app);
    assert!(!back.starts_page(1));
    assert_eq!(
        back.get_viewport(1..2)
            .get(1)
            .and_then(|block| block.style.clone())
            .as_deref(),
        Some("Standard")
    );
}

#[test]
fn a_break_a_named_style_carries_is_the_styles_to_change() {
    let app = open(
        r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:break-before="page" fo:margin-left="1cm"/></style:style>"#,
        r#"<text:p>one</text:p><text:p text:style-name="P1">two</text:p>"#,
    );
    assert!(app.set_page_break(1, false).is_err());
    assert!(app.starts_page(1), "refused, so nothing changed");
}

#[test]
fn a_heading_broken_in_two_stays_a_heading_on_both_pages() {
    let app = open(
        "",
        r#"<text:h text:style-name="Heading_20_1" text:outline-level="1">headingtail</text:h>"#,
    );
    app.break_page(Caret {
        block: 0,
        offset: 7,
    })
    .unwrap();
    let (back, xml) = reopened(&app);
    let blocks = back.get_viewport(0..2);
    assert_eq!(
        blocks.get(1).map(|block| block.kind.clone()),
        Some(BlockKind::Heading { level: 1 }),
        "{xml}"
    );
    assert!(back.starts_page(1));
    assert!(
        xml.contains(r#"style:parent-style-name="Heading_20_1""#),
        "{xml}"
    );
}
