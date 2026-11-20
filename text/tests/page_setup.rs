// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Setting the page (`doc/page-model.md`): paper, orientation and margins written over the
//! master page's own layout and nothing else, as Writer writes them (`doc/odt-format.md` §5c
//! fact 59) — or, in a document stating no page, a layout and a master page added.

use grind_core::page::PageGeometry;
use grind_text::{App, BlockKind, Form};

/// A5 landscape with 1.5 cm margins, in millimetres.
fn a5_landscape() -> PageGeometry {
    PageGeometry {
        width: 210.0,
        height: 148.0,
        top: 15.0,
        bottom: 15.0,
        left: 15.0,
        right: 15.0,
    }
}

fn close(a: PageGeometry, b: PageGeometry) -> bool {
    let near = |x: f64, y: f64| (x - y).abs() < 0.01;
    near(a.width, b.width)
        && near(a.height, b.height)
        && near(a.top, b.top)
        && near(a.bottom, b.bottom)
        && near(a.left, b.left)
        && near(a.right, b.right)
}

fn reopened(app: &App, form: Form) -> (App, Vec<u8>) {
    let bytes = app.save_bytes(form).unwrap();
    let back = App::new();
    let name = if form == Form::Package {
        "a.odt"
    } else {
        "a.fodt"
    };
    back.open_bytes(name, &bytes).unwrap();
    (back, bytes)
}

/// The layout Writer wrote for a new document in this session's locale (fact 59's "before").
const WRITER: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text"><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" style:num-format="1" style:print-orientation="portrait" fo:margin-top="0.7874in" fo:margin-bottom="0.7874in" fo:margin-left="0.7874in" fo:margin-right="0.7874in" style:writing-mode="lr-tb" style:footnote-max-height="0in"><style:footnote-sep style:width="0.0071in"/></style:page-layout-properties><style:header-style/><style:footer-style/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles><office:body><office:text><text:p>one</text:p></office:text></office:body></office:document>"#;

#[test]
fn a_writer_page_is_set_in_place_and_keeps_what_else_it_says() {
    let app = App::new();
    app.open_bytes("a.fodt", WRITER.as_bytes()).unwrap();
    app.set_page(a5_landscape()).unwrap();
    let (back, bytes) = reopened(&app, Form::Flat);
    assert!(close(back.page().unwrap(), a5_landscape()));
    let xml = String::from_utf8(bytes).unwrap();
    assert!(
        xml.contains(r#"style:print-orientation="landscape""#),
        "{xml}"
    );
    for kept in [
        r#"style:num-format="1""#,
        r#"style:writing-mode="lr-tb""#,
        r#"<style:footnote-sep style:width="0.0071in"/>"#,
        r#"<style:page-layout style:name="pm1">"#,
        r#"<style:master-page style:name="Standard" style:page-layout-name="pm1"/>"#,
    ] {
        assert!(xml.contains(kept), "{kept} went: {xml}");
    }
    assert!(!xml.contains("8.5in"), "{xml}");
}

#[test]
fn a_document_stating_no_page_gains_one_in_either_form() {
    for form in [Form::Flat, Form::Package] {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "one").unwrap();
        app.set_page(a5_landscape()).unwrap();
        let (back, _) = reopened(&app, form);
        assert!(close(back.page().unwrap(), a5_landscape()), "{form:?}");

        // And again, now that the file it came from has a page of its own.
        let mut taller = a5_landscape();
        std::mem::swap(&mut taller.width, &mut taller.height);
        back.set_page(taller).unwrap();
        let (again, _) = reopened(&back, form);
        assert!(close(again.page().unwrap(), taller), "{form:?}");
    }
}

#[test]
fn a_package_from_a_flat_writer_document_is_set_in_its_styles_part() {
    let flat = App::new();
    flat.open_bytes("a.fodt", WRITER.as_bytes()).unwrap();
    let package = flat.save_bytes(Form::Package).unwrap();
    let app = App::new();
    app.open_bytes("a.odt", &package).unwrap();
    app.set_page(a5_landscape()).unwrap();
    let (back, _) = reopened(&app, Form::Package);
    assert!(close(back.page().unwrap(), a5_landscape()));
}

#[test]
fn setting_the_page_undoes_in_one_step() {
    let app = App::new();
    app.open_bytes("a.fodt", WRITER.as_bytes()).unwrap();
    let before = app.page();
    app.set_page(a5_landscape()).unwrap();
    assert!(app.undo());
    assert_eq!(app.page(), before);
}
