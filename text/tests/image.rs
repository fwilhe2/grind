// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `draw:frame` + `draw:image` (`doc/text-core.md`'s inline table, `doc/odt-format.md`'s
//! "An inserted image is a frame inside a frame") — round-tripped by this crate alone, no
//! LibreOffice required. The nested-frame shape this backs up was measured from a real
//! LibreOffice-authored `.fodt`; the fixture below reproduces it byte for byte rather than
//! inventing a simpler one, because that nesting is exactly what a real document has to
//! survive.

use grind_text::model::{Block, BlockKind, Document, Run};
use grind_text::{Form, odf};

/// Sixteen bytes is not a real JPEG, and does not need to be — nothing in the ODF schema or
/// this crate looks inside `office:binary-data`, only base64-decodes it. What must round-trip
/// is the bytes, not a decodable image.
const PIXELS: &[u8] = b"not-a-real-jpeg!";

/// The exact shape `doc/odt-format.md` measured from LibreOffice's own Insert Image: an outer
/// `draw:frame` (anchored `char`, width only) wrapping a `draw:text-box` wrapping a `text:p`
/// holding a second, inner frame (anchored `paragraph`, width *and* height) around the image.
fn nested_frame_fodt(base64: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  office:mimetype="application/vnd.oasis.opendocument.text">
  <office:body><office:text>
    <text:p text:style-name="P1">
     <draw:frame draw:name="Frame1" text:anchor-type="char" svg:width="13.229cm">
      <draw:text-box fo:min-height="13.229cm">
       <text:p text:style-name="Figure">
        <draw:frame draw:name="Image1" text:anchor-type="paragraph"
                    svg:width="13.229cm" svg:height="9.912cm">
         <draw:image draw:mime-type="image/jpeg">
          <office:binary-data>{base64}</office:binary-data>
         </draw:image>
        </draw:frame>
       </text:p>
      </draw:text-box>
     </draw:frame>
    </text:p>
  </office:text></office:body>
</office:document>"#
    )
}

fn image_run(doc: &Document) -> &Run {
    doc.blocks[0]
        .runs
        .iter()
        .find(|r| matches!(r, Run::Image { .. }))
        .expect("an image run")
}

#[test]
fn a_nested_frame_reads_as_one_image_sized_from_both_frames() {
    use base64::Engine as _;
    let base64 = base64::engine::general_purpose::STANDARD.encode(PIXELS);
    let bytes = nested_frame_fodt(&base64).into_bytes();
    let doc = odf::read(&bytes).expect("parses");

    assert_eq!(
        doc.blocks.len(),
        1,
        "the text-box's own paragraph is not a block"
    );
    let Run::Image {
        mime,
        data,
        width,
        height,
    } = image_run(&doc)
    else {
        unreachable!()
    };
    assert_eq!(mime, "image/jpeg");
    assert_eq!(data, PIXELS);
    // The outer frame's width wins; it had no height, so the inner frame's fills in.
    assert_eq!(width.as_deref(), Some("13.229cm"));
    assert_eq!(height.as_deref(), Some("9.912cm"));
}

#[test]
fn an_unedited_nested_frame_survives_byte_for_byte() {
    use base64::Engine as _;
    let base64 = base64::engine::general_purpose::STANDARD.encode(PIXELS);
    let bytes = nested_frame_fodt(&base64).into_bytes();
    let doc = odf::read(&bytes).expect("parses");

    // R6: nothing here was edited, so the splice puts the original element — nested frame,
    // text-box and all — straight back rather than regenerating the simpler shape.
    let out = odf::write(&doc, Form::Flat).expect("writes");
    assert_eq!(
        out, bytes,
        "an unedited image keeps its original XML exactly"
    );
}

#[test]
fn a_regenerated_document_writes_the_flat_shape_and_reads_it_back() {
    // No text-box, no nesting: R3's rule applied to a new element. Built directly rather
    // than read, so this is the *regenerate* path (nothing to splice against).
    let mut doc = Document::new();
    let id = doc.next_id();
    let mut block = Block::new(id, BlockKind::Paragraph);
    block.runs.push(Run::Image {
        mime: "image/png".to_owned(),
        data: PIXELS.to_vec(),
        width: Some("5cm".to_owned()),
        height: Some("5cm".to_owned()),
    });
    doc.blocks.push(block);

    for form in [Form::Flat, Form::Package] {
        let bytes = odf::write(&doc, form).expect("writes");
        let reread = odf::read(&bytes).expect("reads it back");
        let Run::Image {
            mime,
            data,
            width,
            height,
        } = image_run(&reread)
        else {
            unreachable!()
        };
        assert_eq!(mime, "image/png");
        assert_eq!(data, PIXELS);
        assert_eq!(width.as_deref(), Some("5cm"));
        assert_eq!(height.as_deref(), Some("5cm"));
    }
}

/// A frame with no image inside it — `draw:object`, a plain shape, an empty caption someone
/// left behind — contributes nothing rather than an image with no bytes. Silent, the way any
/// other unmodelled element is (§8's default-ignore), and the paragraph around it still
/// survives whole through R6 if it is never edited.
#[test]
fn a_frame_with_no_image_inside_it_is_simply_not_a_run() {
    let bytes = br#"<?xml version="1.0"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  office:mimetype="application/vnd.oasis.opendocument.text">
  <office:body><office:text>
    <text:p><draw:frame draw:name="Empty"/></text:p>
  </office:text></office:body>
</office:document>"#;
    let doc = odf::read(bytes).expect("parses");
    assert_eq!(doc.blocks.len(), 1);
    assert!(doc.blocks[0].runs.is_empty());
}

/// [`nested_frame_fodt`] plus the caption LibreOffice's own Insert Image dialog writes after
/// the inner frame, inside the same `text:box`-nested paragraph: plain text, a `text:sequence`
/// field (rng:8655) standing for the figure number, and more plain text.
fn captioned_frame_fodt(base64: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
  office:mimetype="application/vnd.oasis.opendocument.text">
  <office:body><office:text><text:p text:style-name="P1"><draw:frame draw:name="Frame1" text:anchor-type="char" svg:width="13.229cm"><draw:text-box fo:min-height="13.229cm"><text:p text:style-name="Figure"><draw:frame draw:name="Image1" text:anchor-type="paragraph" svg:width="13.229cm" svg:height="9.912cm"><draw:image draw:mime-type="image/jpeg"><office:binary-data>{base64}</office:binary-data></draw:image></draw:frame>Figure <text:sequence text:ref-name="refFigure0" text:name="Figure" text:formula="ooow:Figure+1" style:num-format="1">1</text:sequence>: a photograph.</text:p></draw:text-box></draw:frame></text:p></office:text></office:body>
</office:document>"#
    )
}

/// The caption text lands as a second run, *after* the image — `doc/text-shell.md`'s "Images"
/// row and the real `Earthrise.fodt` this shape was measured from both show a picture that is
/// captioned rather than bare, and dropping that text silently (as an earlier build did) is
/// the bug this pins.
#[test]
fn a_captioned_frame_reads_the_caption_as_a_run_after_the_image() {
    use base64::Engine as _;
    let base64 = base64::engine::general_purpose::STANDARD.encode(PIXELS);
    let bytes = captioned_frame_fodt(&base64).into_bytes();
    let doc = odf::read(&bytes).expect("parses");

    assert_eq!(doc.blocks.len(), 1);
    let runs = &doc.blocks[0].runs;
    assert_eq!(runs.len(), 2, "the image, then its caption");
    assert!(matches!(runs[0], Run::Image { .. }));
    assert_eq!(
        runs[1].text(),
        "Figure 1: a photograph.",
        "the sequence field's own text (\"1\") survives alongside the plain text around it"
    );
}

/// An unedited captioned frame still splices byte for byte — the caption run does not turn
/// into a reason to regenerate any more than the image run already wasn't one.
#[test]
fn an_unedited_captioned_frame_survives_byte_for_byte() {
    use base64::Engine as _;
    let base64 = base64::engine::general_purpose::STANDARD.encode(PIXELS);
    let bytes = captioned_frame_fodt(&base64).into_bytes();
    let doc = odf::read(&bytes).expect("parses");
    let out = odf::write(&doc, Form::Flat).expect("writes");
    assert_eq!(out, bytes);
}

/// The package form's other choice for a picture (rng:1621's `common-draw-data-attlist`): a
/// separate part in the zip, referenced by `xlink:href` rather than embedded as base64 — what
/// LibreOffice actually writes into a `.odt`, as opposed to the `.fodt` fixtures above. Built as
/// a real zip rather than vendoring one, since the shape under test is entirely the packaging.
#[test]
fn a_package_form_image_is_resolved_against_its_own_part() {
    let content = br#"<?xml version="1.0"?>
<office:document-content
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  office:version="1.4">
  <office:body><office:text>
    <text:p><draw:frame draw:name="Image1">
      <draw:image xlink:href="Pictures/pic.jpg" xlink:type="simple" xlink:show="embed"
                  xlink:actuate="onLoad" draw:mime-type="image/jpeg"/>
    </draw:frame></text:p>
  </office:text></office:body>
</office:document-content>"#;

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).expect("mimetype entry");
    std::io::Write::write_all(&mut zip, b"application/vnd.oasis.opendocument.text")
        .expect("mimetype bytes");
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("content.xml", deflated)
        .expect("content.xml entry");
    std::io::Write::write_all(&mut zip, content).expect("content.xml bytes");
    zip.start_file("Pictures/pic.jpg", deflated)
        .expect("picture entry");
    std::io::Write::write_all(&mut zip, PIXELS).expect("picture bytes");
    let bytes = zip.finish().expect("finishes").into_inner();

    let doc = odf::read(&bytes).expect("parses");
    let Run::Image { mime, data, .. } = image_run(&doc) else {
        unreachable!()
    };
    assert_eq!(mime, "image/jpeg");
    assert_eq!(
        data, PIXELS,
        "the referenced part's own bytes, not empty ones"
    );
}
