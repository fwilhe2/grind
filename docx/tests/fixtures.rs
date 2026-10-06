// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hand-built Word documents for what the corpus does not reliably show — `grind_xlsx`'s
//! `fixtures.rs` for the other application, and for the same reason: every package here is
//! assembled from XML written out in full, so a reviewer can read the bytes under test.
//!
//! Each test states one claim about what a Word construct *becomes*, and checks it on the
//! document `grind_text` reads back — not on the ODF this filter wrote, because what matters is
//! what a shell shows and a save keeps.

use std::io::{Cursor, Write};

use grind_docx::{Dropped, Flavour};
use grind_text::{BlockKind, Document, Run};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const W_STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const R_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MCE: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";

/// Assemble a zip from `(part name, content)` pairs.
fn package(parts: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut out));
        let options: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in parts {
            zip.start_file(*name, options).expect("a zip entry");
            zip.write_all(content).expect("writing an entry");
        }
        zip.finish().expect("a finished zip");
    }
    out
}

/// A document whose body is `body`, with `parts` beside it — each `(name, rel type, xml)`
/// related from the main part as `rIdN` in order.
fn docx(body: &str, parts: &[(&str, &str, Vec<u8>)]) -> Vec<u8> {
    let rels: String = parts
        .iter()
        .enumerate()
        .filter(|(_, (_, kind, _))| !kind.is_empty())
        .map(|(i, (name, kind, _))| {
            let target = name.strip_prefix("word/").unwrap_or(name);
            format!(
                r#"<Relationship Id="rId{}" Type="{R}/{kind}" Target="{target}"/>"#,
                i + 1
            )
        })
        .collect();
    let mut all = vec![
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>"#
            )
            .into_bytes(),
        ),
        (
            "word/document.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W}" xmlns:r="{R}" xmlns:mc="{MCE}" xmlns:wp="{WP}" xmlns:a="{A}" xmlns:pic="{PIC}"><w:body>{body}</w:body></w:document>"#
            )
            .into_bytes(),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(r#"<Relationships xmlns="{PKG}">{rels}</Relationships>"#).into_bytes(),
        ),
    ];
    for (name, _, content) in parts {
        all.push((name, content.clone()));
    }
    package(&all)
}

fn styles(inner: &str) -> (&'static str, &'static str, Vec<u8>) {
    (
        "word/styles.xml",
        "styles",
        format!(r#"<w:styles xmlns:w="{W}">{inner}</w:styles>"#).into_bytes(),
    )
}

fn import(bytes: &[u8]) -> (Document, grind_docx::Report) {
    grind_docx::import_bytes(bytes).expect("an importable document")
}

fn texts(doc: &Document) -> Vec<String> {
    doc.blocks.iter().map(|b| b.text()).collect()
}

fn kinds(doc: &Document) -> Vec<BlockKind> {
    doc.blocks.iter().map(|b| b.kind.clone()).collect()
}

/// Every text run of the document, with the formatting it carries.
fn runs(
    doc: &Document,
) -> Vec<(
    String,
    grind_text::CharStyle,
    Option<String>,
    Option<String>,
)> {
    doc.blocks
        .iter()
        .flat_map(|b| b.runs.iter())
        .filter_map(|r| match r {
            Run::Text {
                text,
                style,
                props,
                href,
            } => Some((text.clone(), props.clone(), style.clone(), href.clone())),
            _ => None,
        })
        .collect()
}

fn p(text: &str) -> String {
    format!(r#"<w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

// ---- the package ----

#[test]
fn the_document_is_found_by_relationship_and_is_transitional() {
    let (doc, report) = import(&docx(&p("Hello"), &[]));
    assert_eq!(texts(&doc), ["Hello"]);
    assert_eq!(report.flavour, Flavour::Transitional);
    assert!(report.lossless(), "{:?}", report.dropped);
    assert!(grind_docx::sniff(&docx(&p("x"), &[])));
}

#[test]
fn a_strict_document_reads_by_the_same_table() {
    let bytes = package(&[
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{R_STRICT}/officeDocument" Target="word/document.xml"/></Relationships>"#
            )
            .into_bytes(),
        ),
        (
            "word/document.xml",
            format!(
                r#"<w:document xmlns:w="{W_STRICT}" w:conformance="strict"><w:body><w:p><w:r><w:t>Strict</w:t></w:r></w:p></w:body></w:document>"#
            )
            .into_bytes(),
        ),
    ]);
    let (doc, report) = import(&bytes);
    assert_eq!(texts(&doc), ["Strict"]);
    assert_eq!(report.flavour, Flavour::Strict);
}

#[test]
fn a_workbook_is_not_a_word_document_and_says_so() {
    let bytes = package(&[
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{R}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            )
            .into_bytes(),
        ),
        (
            "xl/workbook.xml",
            br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#
                .to_vec(),
        ),
    ]);
    assert!(!grind_docx::sniff(&bytes));
    let err = grind_docx::import_bytes(&bytes).expect_err("a workbook is not a document");
    assert!(matches!(err, grind_docx::Error::NotWordDocument), "{err}");
    assert!(grind_xlsx_free_sniff_is_not_needed());
}

/// The two filters' sniffs must not both claim one file; the spreadsheet half of that is
/// `grind_xlsx`'s own test, and this one is the word processor's.
fn grind_xlsx_free_sniff_is_not_needed() -> bool {
    !grind_docx::sniff(b"PK\x03\x04 not really")
}

/// Markup compatibility, once, in the walker: the fallback is read and the choice is not.
#[test]
fn the_fallback_of_an_alternation_is_read() {
    let body = r#"<w:p><mc:AlternateContent><mc:Choice Requires="w14"><w:r><w:t>choice</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>fallback</w:t></w:r></mc:Fallback></mc:AlternateContent></w:p>"#;
    let (doc, _) = import(&docx(body, &[]));
    assert_eq!(texts(&doc), ["fallback"]);
}

// ---- paragraphs and text ----

#[test]
fn whitespace_tabs_and_breaks_arrive_as_odf_spells_them() {
    let body = r#"<w:p><w:r><w:t xml:space="preserve">  two  spaces</w:t><w:tab/><w:t>tabbed</w:t><w:br/><w:t>broken</w:t></w:r></w:p>"#;
    let (doc, _) = import(&docx(body, &[]));
    assert_eq!(texts(&doc), ["  two  spaces\ttabbed\nbroken"]);
}

#[test]
fn a_heading_is_a_stated_outline_level_and_its_style_is_named() {
    let s = styles(
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
           <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/>
             <w:pPr><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
           <w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/></w:style>"#,
    );
    let body = format!(
        r#"<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Report</w:t></w:r></w:p>
           <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Methods</w:t></w:r></w:p>{}"#,
        p("Body")
    );
    let (doc, report) = import(&docx(&body, &[s]));
    assert_eq!(
        kinds(&doc),
        [
            BlockKind::Paragraph,
            BlockKind::Heading { level: 2 },
            BlockKind::Paragraph
        ]
    );
    assert_eq!(
        doc.blocks[0].style.as_deref(),
        Some("Title"),
        "a shell draws Title by name"
    );
    assert_eq!(doc.blocks[1].style.as_deref(), Some("Heading_20_2"));
    assert_eq!(
        doc.blocks[2].style.as_deref(),
        Some("Normal"),
        "the default style"
    );
    // The named style is declared, with its parent and what it says.
    let heading = &doc.paragraph_styles["Heading_20_2"];
    assert_eq!(heading.parent.as_deref(), Some("Normal"));
    assert_eq!(heading.props.font_weight.as_deref(), Some("bold"));
    assert_eq!(heading.props.font_size.as_deref(), Some("16pt"));
    assert_eq!(report.headings, 1);
}

#[test]
fn direct_paragraph_formatting_is_an_automatic_style_under_the_named_one() {
    let s = styles(
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>"#,
    );
    let body = r#"<w:p><w:pPr><w:jc w:val="center"/><w:spacing w:before="240" w:after="120"/><w:ind w:left="720"/></w:pPr><w:r><w:t>Centred</w:t></w:r></w:p>"#;
    let (doc, _) = import(&docx(body, &[s]));
    let style = doc.blocks[0].style.clone().expect("a style");
    let resolved =
        grind_text::paragraph::resolve(&doc.paragraph_styles, &doc.default_paragraph, Some(&style));
    assert_eq!(resolved.text_align.as_deref(), Some("center"));
    assert_eq!(resolved.margin_top.as_deref(), Some("12pt"));
    assert_eq!(resolved.margin_bottom.as_deref(), Some("6pt"));
    assert_eq!(resolved.margin_left.as_deref(), Some("36pt"));
    assert_eq!(
        doc.paragraph_styles[&style].parent.as_deref(),
        Some("Normal")
    );
}

#[test]
fn run_formatting_is_carried_character_by_character() {
    let body = r#"<w:p>
        <w:r><w:t xml:space="preserve">plain </w:t></w:r>
        <w:r><w:rPr><w:b/><w:i/></w:rPr><w:t xml:space="preserve">bold italic </w:t></w:r>
        <w:r><w:rPr><w:u w:val="single"/><w:strike/><w:color w:val="C00000"/><w:sz w:val="28"/><w:rFonts w:ascii="Arial"/><w:highlight w:val="yellow"/></w:rPr><w:t>dressed</w:t></w:r>
      </w:p>"#;
    let (doc, _) = import(&docx(body, &[]));
    let got = runs(&doc);
    assert_eq!(got[0].0, "plain ");
    assert!(got[0].1.is_plain());
    assert_eq!(got[1].1.font_weight.as_deref(), Some("bold"));
    assert_eq!(got[1].1.font_style.as_deref(), Some("italic"));
    let dressed = &got[2].1;
    assert_eq!(dressed.underline.as_deref(), Some("solid"));
    assert_eq!(dressed.line_through.as_deref(), Some("solid"));
    assert_eq!(dressed.color.as_deref(), Some("#c00000"));
    assert_eq!(dressed.font_size.as_deref(), Some("14pt"));
    assert_eq!(dressed.font_family.as_deref(), Some("Arial"));
    assert_eq!(dressed.background.as_deref(), Some("#ffff00"));
}

#[test]
fn a_character_style_stays_a_name() {
    let s = styles(
        r#"<w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/></w:rPr></w:style>"#,
    );
    let body = r#"<w:p><w:r><w:rPr><w:rStyle w:val="Strong"/></w:rPr><w:t>named</w:t></w:r></w:p>"#;
    let (doc, _) = import(&docx(body, &[s]));
    let got = runs(&doc);
    assert_eq!(got[0].2.as_deref(), Some("Strong"));
    assert!(
        got[0].1.is_plain(),
        "a named style's look stays behind its name"
    );
}

#[test]
fn a_superscript_survives_into_the_file_a_save_writes() {
    let body = r#"<w:p><w:r><w:t>E=mc</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:t>2</w:t></w:r></w:p>"#;
    let (odf, _) = grind_docx::convert(&docx(body, &[])).unwrap();
    let odf = String::from_utf8(odf).unwrap();
    assert!(odf.contains(r#"style:text-position="super 58%""#), "{odf}");
    // …and an untouched save of what was read is those bytes.
    let doc = grind_text::read_bytes("x.fodt", odf.as_bytes()).unwrap();
    assert_eq!(
        grind_text::write_bytes(&doc, grind_text::Form::Flat).unwrap(),
        odf.into_bytes()
    );
}

// ---- links, bookmarks, fields ----

#[test]
fn hyperlinks_external_and_internal() {
    let body = r##"<w:p><w:bookmarkStart w:id="0" w:name="intro"/><w:r><w:t>Intro</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>
        <w:p><w:hyperlink r:id="rId1"><w:r><w:t>site</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> and </w:t></w:r><w:hyperlink w:anchor="intro"><w:r><w:t>back</w:t></w:r></w:hyperlink></w:p>"##;
    let bytes = package(&[
        (
            "_rels/.rels",
            format!(r#"<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>"#).into_bytes(),
        ),
        (
            "word/document.xml",
            format!(r#"<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}</w:body></w:document>"#).into_bytes(),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(r#"<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{R}/hyperlink" Target="https://example.invalid/" TargetMode="External"/></Relationships>"#).into_bytes(),
        ),
    ]);
    let (doc, _) = import(&bytes);
    let got = runs(&doc);
    let href = |t: &str| got.iter().find(|r| r.0 == t).and_then(|r| r.3.clone());
    assert_eq!(href("site").as_deref(), Some("https://example.invalid/"));
    assert_eq!(href("back").as_deref(), Some("#intro"));
    assert_eq!(href(" and "), None);
    assert!(doc.bookmarks.contains_key("intro"));
}

#[test]
fn a_hidden_bookmark_is_kept_only_where_something_points_at_it() {
    let body = r##"<w:p><w:bookmarkStart w:id="0" w:name="_GoBack"/><w:bookmarkStart w:id="1" w:name="_Toc1"/><w:r><w:t>Head</w:t></w:r></w:p>
        <w:p><w:hyperlink w:anchor="_Toc1"><w:r><w:t>see</w:t></w:r></w:hyperlink></w:p>"##;
    let (doc, _) = import(&docx(body, &[]));
    assert!(doc.bookmarks.contains_key("_Toc1"));
    assert!(!doc.bookmarks.contains_key("_GoBack"));
}

#[test]
fn a_field_shows_its_result_and_a_page_number_is_a_field() {
    let body = r#"<w:p>
        <w:r><w:t xml:space="preserve">Dated </w:t></w:r>
        <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> DATE \@ "d MMMM yyyy" </w:instrText></w:r>
        <w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>5 October 2026</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
      </w:p>
      <w:p><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>7</w:t></w:r></w:fldSimple></w:p>"#;
    let bytes = docx(body, &[]);
    let (doc, report) = import(&bytes);
    assert_eq!(texts(&doc)[0], "Dated 5 October 2026");
    assert_eq!(
        report.dropped.get(&Dropped::Field),
        Some(&1),
        "DATE no longer updates"
    );
    let (odf, _) = grind_docx::convert(&bytes).unwrap();
    let odf = String::from_utf8(odf).unwrap();
    assert!(odf.contains("<text:page-number"), "PAGE is a page number");
    assert!(!odf.contains(">7<"), "and its cached text is not");
}

#[test]
fn a_field_instruction_spanning_paragraphs_does_not_leave_them_behind() {
    let body = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> IF "a" = "" "" "shown</w:instrText></w:r></w:p>
        <w:p><w:r><w:instrText>" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>shown</w:t></w:r></w:p>
        <w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    let (doc, _) = import(&docx(body, &[]));
    assert_eq!(texts(&doc), ["shown", ""]);
}

#[test]
fn a_tracked_insertion_is_kept_and_a_deletion_is_not() {
    let body = r#"<w:p><w:r><w:t xml:space="preserve">keep </w:t></w:r>
        <w:ins w:id="1" w:author="a"><w:r><w:t xml:space="preserve">added </w:t></w:r></w:ins>
        <w:del w:id="2" w:author="a"><w:r><w:delText>gone </w:delText></w:r></w:del>
        <w:r><w:t>end</w:t></w:r></w:p>"#;
    let (doc, report) = import(&docx(body, &[]));
    assert_eq!(texts(&doc), ["keep added end"]);
    assert_eq!(report.dropped.get(&Dropped::TrackedChange), Some(&2));
}

#[test]
fn a_content_control_is_read_through() {
    let body = r#"<w:sdt><w:sdtPr><w:alias w:val="Block"/></w:sdtPr><w:sdtContent>
        <w:p><w:sdt><w:sdtPr><w:dropDownList/></w:sdtPr><w:sdtContent><w:r><w:t>Choice</w:t></w:r></w:sdtContent></w:sdt></w:p>
      </w:sdtContent></w:sdt>"#;
    let (doc, report) = import(&docx(body, &[]));
    assert_eq!(texts(&doc), ["Choice"]);
    assert_eq!(
        report.dropped.get(&Dropped::ContentControl),
        Some(&1),
        "only the drop-down"
    );
}

// ---- lists ----

fn numbering() -> (&'static str, &'static str, Vec<u8>) {
    (
        "word/numbering.xml",
        "numbering",
        format!(
            r#"<w:numbering xmlns:w="{W}">
                 <w:abstractNum w:abstractNumId="0">
                   <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl>
                   <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2)"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl>
                 </w:abstractNum>
                 <w:abstractNum w:abstractNumId="1">
                   <w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="&#xF0B7;"/><w:rPr><w:rFonts w:ascii="Symbol"/></w:rPr></w:lvl>
                 </w:abstractNum>
                 <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
                 <w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
               </w:numbering>"#
        )
        .into_bytes(),
    )
}

fn item(num: u32, level: u32, text: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{num}"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
    )
}

#[test]
fn numbered_items_are_list_items_at_their_depth() {
    let body = [
        item(1, 0, "one"),
        item(1, 1, "one a"),
        item(1, 0, "two"),
        p("between"),
        item(1, 0, "three"),
        item(2, 0, "bullet"),
    ]
    .concat();
    let bytes = docx(&body, &[numbering()]);
    let (doc, report) = import(&bytes);
    assert_eq!(
        kinds(&doc),
        [
            BlockKind::ListItem { depth: 1 },
            BlockKind::ListItem { depth: 2 },
            BlockKind::ListItem { depth: 1 },
            BlockKind::Paragraph,
            BlockKind::ListItem { depth: 1 },
            BlockKind::ListItem { depth: 1 },
        ]
    );
    assert_eq!(report.list_items, 5);
    // The file says *how* each is numbered, and that "three" continues from "two".
    let (odf, _) = grind_docx::convert(&bytes).unwrap();
    let odf = String::from_utf8(odf).unwrap();
    assert!(
        odf.contains(r#"style:num-format="1" style:num-suffix=".""#),
        "{odf}"
    );
    assert!(odf.contains(r#"style:num-format="a" style:num-suffix=")""#));
    assert!(
        odf.contains(r#"text:bullet-char="•""#),
        "Symbol's bullet, as a bullet"
    );
    assert!(odf.contains(r#"text:continue-list="list1""#));
}

#[test]
fn numbering_from_a_style_makes_a_list_item_too() {
    let s = styles(
        r#"<w:style w:type="paragraph" w:styleId="ListBullet"><w:name w:val="List Bullet"/>
             <w:pPr><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr></w:style>"#,
    );
    let body =
        r#"<w:p><w:pPr><w:pStyle w:val="ListBullet"/></w:pPr><w:r><w:t>styled</w:t></w:r></w:p>"#;
    let (doc, _) = import(&docx(body, &[s, numbering()]));
    assert_eq!(kinds(&doc), [BlockKind::ListItem { depth: 1 }]);
}

// ---- tables ----

#[test]
fn a_table_with_a_horizontal_and_a_vertical_merge() {
    let body = r#"<w:tbl><w:tblPr><w:tblBorders><w:insideH w:val="single" w:sz="4"/></w:tblBorders></w:tblPr>
        <w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>
        <w:tr><w:trPr><w:tblHeader/></w:trPr>
          <w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>wide</w:t></w:r></w:p></w:tc>
          <w:tc><w:tcPr><w:vMerge w:val="restart"/><w:shd w:val="clear" w:fill="FFFF00"/></w:tcPr><w:p><w:r><w:t>tall</w:t></w:r></w:p></w:tc></w:tr>
        <w:tr>
          <w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc>
          <w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p><w:p><w:r><w:t>b2</w:t></w:r></w:p></w:tc>
          <w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr>
      </w:tbl>"#;
    let (doc, report) = import(&docx(body, &[]));
    assert_eq!(report.tables, 1);
    // The table, and the paragraph every document ends in after one.
    assert!(
        doc.blocks
            .last()
            .is_some_and(|b| b.cell.is_none() && b.is_empty())
    );
    let cells: Vec<_> = doc
        .blocks
        .iter()
        .filter_map(|b| {
            let c = b.cell.as_ref()?;
            Some((b.text(), c.row, c.column, c.columns_spanned, c.rows_spanned))
        })
        .collect();
    assert_eq!(
        cells,
        [
            ("wide".into(), 0, 0, 2, 1),
            ("tall".into(), 0, 2, 1, 2),
            ("a".into(), 1, 0, 1, 1),
            ("b".into(), 1, 1, 1, 1),
            ("b2".into(), 1, 1, 1, 1),
        ]
    );
    let look = &doc.table_looks["Table1"];
    assert_eq!(look.columns.len(), 3);
    assert!(look.header_rows.contains(&0), "a repeated header row");
    assert_eq!(
        look.cell(0, 2).and_then(|c| c.background.as_deref()),
        Some("#ffff00")
    );
}

// ---- pages, marginals, notes, pictures ----

#[test]
fn the_first_sections_page_and_its_header_and_footer() {
    let header = (
        "word/header1.xml",
        "header",
        format!(r#"<w:hdr xmlns:w="{W}"><w:p><w:r><w:t>Letterhead</w:t></w:r></w:p></w:hdr>"#)
            .into_bytes(),
    );
    let footer = (
        "word/footer1.xml",
        "footer",
        format!(
            r#"<w:ftr xmlns:w="{W}"><w:p><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>PAGE</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:ftr>"#
        )
        .into_bytes(),
    );
    let body = format!(
        r#"{}<w:sectPr><w:headerReference w:type="default" r:id="rId1"/><w:footerReference w:type="default" r:id="rId2"/>
             <w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1134" w:bottom="1134" w:left="1417" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>"#,
        p("Body")
    );
    let (doc, _) = import(&docx(&body, &[header, footer]));
    let page = doc.page.expect("a page");
    assert!((page.width - 210.0).abs() < 0.1, "A4 wide: {}", page.width);
    assert!(
        (page.height - 297.0).abs() < 0.1,
        "A4 high: {}",
        page.height
    );
    let header = doc.header.expect("a header");
    assert_eq!(header.paragraphs[0].text(1, 1), "Letterhead");
    let footer = doc.footer.expect("a footer");
    assert_eq!(
        footer.paragraphs[0].text(3, 9),
        "Page 3",
        "the page's own number"
    );
}

#[test]
fn a_footnote_is_set_where_it_is_cited() {
    let notes = (
        "word/footnotes.xml",
        "footnotes",
        format!(
            r#"<w:footnotes xmlns:w="{W}">
                 <w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>
                 <w:footnote w:id="1"><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> The source.</w:t></w:r></w:p></w:footnote>
               </w:footnotes>"#
        )
        .into_bytes(),
    );
    let body = r#"<w:p><w:r><w:t>Claim</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r><w:r><w:t>.</w:t></w:r></w:p>"#;
    let (doc, report) = import(&docx(body, &[notes]));
    assert_eq!(report.notes, 1);
    assert_eq!(doc.notes.len(), 1);
    let note = &doc.notes[0];
    assert_eq!(note.citation, "1");
    assert_eq!(note.offset, 5);
    assert_eq!(
        note.paragraphs[0].text(1, 1),
        "The source.",
        "no separator, no leading space"
    );
}

#[test]
fn a_page_break_starts_a_paragraph_on_a_new_page() {
    let body = r#"<w:p><w:r><w:t>before</w:t><w:br w:type="page"/><w:t>after</w:t></w:r></w:p>"#;
    let (doc, _) = import(&docx(body, &[]));
    assert_eq!(texts(&doc), ["before", "after"]);
    let style = doc.blocks[1].style.clone();
    let resolved = grind_text::paragraph::resolve(
        &doc.paragraph_styles,
        &doc.default_paragraph,
        style.as_deref(),
    );
    assert_eq!(resolved.break_before.as_deref(), Some("page"));
}

/// A one-pixel PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[test]
fn an_inline_picture_is_a_picture_at_its_size() {
    let body = r#"<w:p><w:r><w:t xml:space="preserve">See </w:t></w:r><w:r><w:drawing><wp:inline><wp:extent cx="914400" cy="457200"/>
        <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rId1"/></pic:blipFill></pic:pic></a:graphicData></a:graphic>
      </wp:inline></w:drawing></w:r></w:p>"#;
    let media = ("word/media/image1.png", "image", PNG.to_vec());
    let (doc, report) = import(&docx(body, &[media]));
    assert_eq!(report.images, 1);
    let image = doc.blocks[0]
        .runs
        .iter()
        .find_map(|r| match r {
            Run::Image {
                mime,
                data,
                width,
                height,
                anchor,
            } => Some((
                mime.clone(),
                data.clone(),
                width.clone(),
                height.clone(),
                anchor.clone(),
            )),
            _ => None,
        })
        .expect("a picture");
    assert_eq!(image.0, "image/png");
    assert_eq!(image.1, PNG);
    assert_eq!(image.2.as_deref(), Some("72pt"));
    assert_eq!(image.3.as_deref(), Some("36pt"));
    assert_eq!(image.4.as_deref(), Some("as-char"));
}

/// A text box's story follows the paragraph that anchors it — from the VML fallback Word
/// writes beside the DrawingML choice, read once.
#[test]
fn a_text_boxs_text_follows_its_anchor() {
    let body = r#"<w:p><w:r><w:t>Anchor</w:t></w:r><w:r><mc:AlternateContent>
        <mc:Choice Requires="wps"><w:drawing><wp:anchor/></w:drawing></mc:Choice>
        <mc:Fallback><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" style="position:absolute;width:100pt;height:50pt"><v:textbox><w:txbxContent>
          <w:p><w:r><w:t>Boxed</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback>
      </mc:AlternateContent></w:r></w:p><w:p><w:r><w:t>After</w:t></w:r></w:p>"#;
    let (doc, report) = import(&docx(body, &[]));
    assert_eq!(texts(&doc), ["Anchor", "Boxed", "After"]);
    assert_eq!(report.dropped.get(&Dropped::TextBox), Some(&1));
    assert!(!report.dropped.contains_key(&Dropped::Drawing));
}

#[test]
fn a_shape_with_no_picture_is_counted_not_invented() {
    let body = r#"<w:p><w:r><w:drawing><wp:anchor><wp:extent cx="1" cy="1"/><a:graphic><a:graphicData uri="x"/></a:graphic></wp:anchor></w:drawing></w:r></w:p>"#;
    let (doc, report) = import(&docx(body, &[]));
    assert_eq!(texts(&doc), [""]);
    assert_eq!(report.dropped.get(&Dropped::Drawing), Some(&1));
}

// ---- tolerance ----

#[test]
fn damage_part_way_through_keeps_what_came_before() {
    let bytes = package(&[
        (
            "_rels/.rels",
            format!(r#"<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>"#).into_bytes(),
        ),
        (
            "word/document.xml",
            format!(r#"<w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>kept</w:t></w:r></w:p><w:p><w:r><w:t>broken</w:x></w:r></w:p></w:body></w:document>"#).into_bytes(),
        ),
    ]);
    let (doc, report) = import(&bytes);
    assert_eq!(texts(&doc), ["kept"]);
    assert_eq!(report.dropped.get(&Dropped::Damaged), Some(&1));
}

#[test]
fn a_relationship_out_of_the_package_reaches_nothing() {
    let body = r#"<w:p><w:r><w:drawing><wp:inline><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rId1"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let media = ("../../etc/passwd", "image", b"root".to_vec());
    let (doc, report) = import(&docx(body, &[media]));
    assert_eq!(texts(&doc), [""]);
    assert_eq!(report.images, 0);
}

/// Numbered headings — `1 Introduction`, `1.1 Scope` — are Word's list numbering on a heading
/// style; they become ODF's outline numbering, which numbers headings by level, and a heading
/// Word left unnumbered at a numbered level stays so.
#[test]
fn numbered_headings_become_the_outline_numbering() {
    let s = styles(
        r#"<w:style w:type="paragraph" w:styleId="H1"><w:name w:val="heading 1"/>
             <w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr><w:outlineLvl w:val="0"/></w:pPr></w:style>
           <w:style w:type="paragraph" w:styleId="H2"><w:name w:val="heading 2"/>
             <w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr><w:outlineLvl w:val="1"/></w:pPr></w:style>"#,
    );
    let body = r#"<w:p><w:pPr><w:pStyle w:val="H1"/></w:pPr><w:r><w:t>Introduction</w:t></w:r></w:p>
        <w:p><w:pPr><w:pStyle w:val="H2"/></w:pPr><w:r><w:t>Scope</w:t></w:r></w:p>
        <w:p><w:pPr><w:pStyle w:val="H1"/><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>Appendix</w:t></w:r></w:p>"#;
    let bytes = docx(body, &[s, numbering()]);
    let (doc, report) = import(&bytes);
    assert_eq!(
        kinds(&doc),
        [
            BlockKind::Heading { level: 1 },
            BlockKind::Heading { level: 2 },
            BlockKind::Heading { level: 1 }
        ]
    );
    assert!(
        !report.dropped.contains_key(&Dropped::HeadingNumber),
        "{:?}",
        report.dropped
    );
    let (odf, _) = grind_docx::convert(&bytes).unwrap();
    let odf = String::from_utf8(odf).unwrap();
    assert!(odf.contains(r#"<text:outline-level-style text:level="1" style:num-format="1" style:num-suffix="."/>"#), "{odf}");
    assert!(
        odf.contains(r#"text:outline-level="1" text:is-list-header="true">Appendix"#),
        "{odf}"
    );
}

/// A later section on a page of its own — a landscape table in a portrait report — is a master
/// page of its own, which its first paragraph starts; the document's page stays the first one.
#[test]
fn a_landscape_section_is_a_master_page_of_its_own() {
    let body = r#"<w:p><w:r><w:t>Portrait</w:t></w:r></w:p>
        <w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:pPr><w:r><w:t>end of one</w:t></w:r></w:p>
        <w:p><w:r><w:t>Landscape</w:t></w:r></w:p>
        <w:sectPr><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>"#;
    let bytes = docx(body, &[]);
    let (doc, report) = import(&bytes);
    assert!(report.lossless(), "{:?}", report.dropped);
    assert_eq!(texts(&doc), ["Portrait", "end of one", "Landscape"]);
    let page = doc.page.expect("a page");
    assert!(
        page.width < page.height,
        "the document's page is the first section's"
    );
    let (odf, _) = grind_docx::convert(&bytes).unwrap();
    let odf = String::from_utf8(odf).unwrap();
    assert!(
        odf.contains(r#"<style:master-page style:name="Section2" style:page-layout-name="pm2">"#),
        "{odf}"
    );
    assert!(odf.contains(r#"style:print-orientation="landscape""#));
    assert!(odf.contains(r#"style:master-page-name="Section2""#));
}

/// Even pages' own footer, under `w:evenAndOddHeaders`, is ODF's left-page footer — and without
/// the setting Word shows no even variant, so none is written and nothing is lost.
#[test]
fn an_even_page_footer_is_a_left_page_footer() {
    let footer = |name: &'static str, text: &str| {
        (
            name,
            "footer",
            format!(r#"<w:ftr xmlns:w="{W}"><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:ftr>"#)
                .into_bytes(),
        )
    };
    let body = r#"<w:p><w:r><w:t>one</w:t></w:r></w:p><w:sectPr><w:footerReference w:type="default" r:id="rId1"/><w:footerReference w:type="even" r:id="rId2"/></w:sectPr>"#;
    let settings = |even: bool| {
        (
            "word/settings.xml",
            "settings",
            format!(
                r#"<w:settings xmlns:w="{W}">{}</w:settings>"#,
                if even { "<w:evenAndOddHeaders/>" } else { "" }
            )
            .into_bytes(),
        )
    };
    for even in [true, false] {
        let bytes = docx(
            body,
            &[
                footer("word/footer1.xml", "ODD"),
                footer("word/footer2.xml", "EVEN"),
                settings(even),
            ],
        );
        let (_, report) = import(&bytes);
        assert!(report.lossless(), "{:?}", report.dropped);
        let odf = String::from_utf8(grind_docx::convert(&bytes).unwrap().0).unwrap();
        assert_eq!(odf.contains("<style:footer-left>"), even, "{odf}");
        assert!(odf.contains("<style:footer>"));
    }
}

/// A comment is an `office:annotation` — its author, its date, its paragraphs — over the range
/// it covers, and the commented text is text like any other.
#[test]
fn a_comment_is_an_annotation_over_its_range() {
    let comments = (
        "word/comments.xml",
        "comments",
        format!(
            r#"<w:comments xmlns:w="{W}"><w:comment w:id="7" w:author="Ada" w:date="2026-10-06T09:30:00Z">
                 <w:p><w:r><w:t>Check this figure.</w:t></w:r></w:p></w:comment></w:comments>"#
        )
        .into_bytes(),
    );
    let body = r#"<w:p><w:r><w:t xml:space="preserve">Revenue was </w:t></w:r><w:commentRangeStart w:id="7"/>
        <w:r><w:t>40%</w:t></w:r><w:commentRangeEnd w:id="7"/><w:r><w:commentReference w:id="7"/></w:r>
        <w:r><w:t xml:space="preserve"> higher.</w:t></w:r></w:p>"#;
    let bytes = docx(body, &[comments]);
    let (doc, report) = import(&bytes);
    assert!(report.lossless(), "{:?}", report.dropped);
    assert_eq!(report.comments, 1);
    assert_eq!(texts(&doc), ["Revenue was 40% higher."]);
    let odf = String::from_utf8(grind_docx::convert(&bytes).unwrap().0).unwrap();
    assert!(
        odf.contains(r#"<office:annotation office:name="comment7"><dc:creator>Ada</dc:creator><dc:date>2026-10-06T09:30:00Z</dc:date><text:p>Check this figure.</text:p></office:annotation>40%<office:annotation-end office:name="comment7"/>"#),
        "{odf}"
    );
}
