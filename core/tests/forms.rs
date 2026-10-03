// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind_core::odf::forms` — a document moved between ODF's two forms, held to carrying every
//! element of it and to refusing, by name, what the other form has no place for. Fixtures are
//! written out here rather than vendored so a reviewer can read every part of them.

use std::io::{Cursor, Write};

use grind_core::Error;
use grind_core::odf::{Form, envelope, forms::convert, package};

const NS: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
    xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
    xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
    xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" \
    xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" \
    xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" \
    xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\" \
    xmlns:xlink=\"http://www.w3.org/1999/xlink\"";

/// A flat text document with every part a flat document has, a footnote the body owns and
/// nothing models, a picture, and a chart embedded inline.
fn flat() -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document {NS} \
         office:version=\"1.4\" office:mimetype=\"application/vnd.oasis.opendocument.text\">\
         <office:meta><meta:generator>hand</meta:generator></office:meta>\
         <office:settings><config:config-item-set config:name=\"view\"/></office:settings>\
         <office:font-face-decls><style:font-face style:name=\"Serif\" \
         svg:font-family=\"Serif\"/></office:font-face-decls>\
         <office:styles><style:style style:name=\"Standard\" style:family=\"paragraph\"/>\
         </office:styles>\
         <office:automatic-styles><style:page-layout style:name=\"pm1\"/>\
         <style:style style:name=\"P1\" style:family=\"paragraph\"/></office:automatic-styles>\
         <office:master-styles><style:master-page style:name=\"Standard\" \
         style:page-layout-name=\"pm1\"/></office:master-styles>\
         <office:body><office:text><text:p text:style-name=\"P1\">Note\
         <text:note text:id=\"n1\"><text:note-citation>1</text:note-citation>\
         <text:note-body><text:p>body</text:p></text:note-body></text:note></text:p>\
         <text:p><draw:frame><draw:image><office:binary-data>iVBORw==</office:binary-data>\
         </draw:image></draw:frame><draw:frame><draw:object>\
         <office:document office:version=\"1.4\" \
         office:mimetype=\"application/vnd.oasis.opendocument.chart\">\
         <office:body><office:chart/></office:body></office:document>\
         </draw:object></draw:frame></text:p>\
         </office:text></office:body></office:document>"
    )
    .into_bytes()
}

fn package(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        w.start_file(*name, options).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn content(body: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NS} \
         office:version=\"1.4\"><office:body><office:text>{body}</office:text></office:body>\
         </office:document-content>"
    )
    .into_bytes()
}

const TEXT: &[u8] = b"application/vnd.oasis.opendocument.text";

#[test]
fn a_flat_document_becomes_a_package_and_comes_back_with_every_element() {
    let original = flat();
    let package = convert(&original, Form::Package).expect("converts");
    let names = package::entry_names(&package);
    for name in [
        "mimetype",
        "META-INF/manifest.xml",
        "content.xml",
        "styles.xml",
        "meta.xml",
        "settings.xml",
        "Object 1/content.xml",
    ] {
        assert!(names.iter().any(|n| n == name), "no {name} in {names:?}");
    }
    let styles = String::from_utf8(package::part(&package, "styles.xml").unwrap()).unwrap();
    assert!(styles.contains("<office:master-styles>"), "{styles}");
    let content = String::from_utf8(package::content_xml(&package).unwrap()).unwrap();
    assert!(content.contains("xlink:href=\"./Object 1\""), "{content}");
    assert!(content.contains("<text:note "), "{content}");
    let manifest =
        String::from_utf8(package::part(&package, "META-INF/manifest.xml").unwrap()).unwrap();
    assert!(
        manifest.contains(
            "manifest:full-path=\"Object 1/\" \
             manifest:media-type=\"application/vnd.oasis.opendocument.chart\""
        ),
        "{manifest}"
    );

    let back = convert(&package, Form::Flat).expect("converts back");
    assert_eq!(
        envelope::document_vocabulary(&back),
        envelope::document_vocabulary(&original),
        "{}",
        String::from_utf8_lossy(&back)
    );
}

#[test]
fn a_picture_in_a_package_is_inlined_into_the_flat_file() {
    let picture = b"\x89PNG not really";
    let bytes = package(&[
        ("mimetype", TEXT),
        (
            "content.xml",
            &content(
                "<text:p><draw:frame><draw:image xlink:href=\"Pictures/a.png\" \
                 xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>\
                 </draw:frame></text:p>",
            ),
        ),
        ("Pictures/a.png", picture),
        ("Thumbnails/thumbnail.png", b"cache"),
    ]);
    let flat = String::from_utf8(convert(&bytes, Form::Flat).expect("converts")).unwrap();
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(picture);
    assert!(
        flat.contains(&format!(
            "<draw:image><office:binary-data>{encoded}</office:binary-data></draw:image>"
        )),
        "{flat}"
    );
    assert!(!flat.contains("xlink:href"), "{flat}");
    assert!(
        flat.contains("office:mimetype=\"application/vnd.oasis.opendocument.text\""),
        "{flat}"
    );
}

#[test]
fn what_a_flat_file_has_no_place_for_is_refused_by_name() {
    let bytes = package(&[
        ("mimetype", TEXT),
        ("content.xml", &content("<text:p/>")),
        ("Basic/Standard/Module1.xml", b"<macro/>"),
    ]);
    match convert(&bytes, Form::Flat) {
        Err(Error::WouldLose(what)) => {
            assert!(
                what[0].starts_with("Basic/Standard/Module1.xml"),
                "{what:?}"
            )
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn two_parts_declaring_one_font_keep_the_fuller_declaration_or_refuse() {
    let styles = |font: &str| {
        format!(
            "<office:document-styles {NS} office:version=\"1.4\"><office:font-face-decls>\
             {font}</office:font-face-decls></office:document-styles>"
        )
        .into_bytes()
    };
    let with_fonts = |content_font: &str, styles_font: &str| {
        let content = format!(
            "<office:document-content {NS} office:version=\"1.4\"><office:font-face-decls>\
             {content_font}</office:font-face-decls><office:body><office:text/></office:body>\
             </office:document-content>"
        );
        package(&[
            ("mimetype", TEXT),
            ("content.xml", content.as_bytes()),
            ("styles.xml", &styles(styles_font)),
        ])
    };
    // The same font, one spelling escaped, the other saying more about it.
    let fuller = "<style:font-face style:name=\"Sans\" svg:font-family=\"'Sans'\" \
                  style:font-pitch=\"variable\"/>";
    let flat = convert(
        &with_fonts(
            "<style:font-face style:name=\"Sans\" svg:font-family=\"&apos;Sans&apos;\"/>",
            fuller,
        ),
        Form::Flat,
    )
    .expect("one font");
    let flat = String::from_utf8(flat).unwrap();
    assert_eq!(flat.matches("<style:font-face ").count(), 1, "{flat}");
    assert!(flat.contains("style:font-pitch=\"variable\""), "{flat}");

    let clash = with_fonts(
        "<style:font-face style:name=\"Sans\" svg:font-family=\"A\"/>",
        "<style:font-face style:name=\"Sans\" svg:font-family=\"B\"/>",
    );
    assert!(matches!(
        convert(&clash, Form::Flat),
        Err(Error::WouldLose(_))
    ));
}

#[test]
fn a_document_already_in_the_form_asked_for_is_its_own_bytes() {
    let original = flat();
    assert_eq!(convert(&original, Form::Flat).unwrap(), original);
}
