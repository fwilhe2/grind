// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a LibreOffice file *means* by a style, where it says less than it means: a cell style
//! that inherits from a named one (`style:parent-style-name`), and a number style written
//! before ODF 1.3 gave decimals a minimum. Both were found by drawing LibreOffice's own
//! corpus beside LibreOffice's rendering of it (`tdf103734.ods`: every border inherited).

use grind_sheet::style::CellStyle;
use grind_sheet::{Document, Pos};

fn fods(body: &str, styles: &str) -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
 xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"
 office:version="1.2" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
{styles}
<office:body><office:spreadsheet><table:table table:name="S">
<table:table-row>{body}</table:table-row>
</table:table></office:spreadsheet></office:body></office:document>"##
    )
}

fn read(body: &str, styles: &str) -> Document {
    grind_sheet::read_bytes("t.fods", fods(body, styles).as_bytes()).expect("loads")
}

/// What the first `cols` cells display.
fn shown(body: &str, styles: &str, cols: u32) -> Vec<String> {
    let app = grind_sheet::App::new();
    app.open_bytes("t.fods", fods(body, styles).as_bytes())
        .expect("opens");
    let view = app.get_viewport(0, 0..1, 0..cols).expect("reads");
    (0..cols)
        .map(|c| view.text(0, c).unwrap_or_default().to_owned())
        .collect()
}

fn style(doc: &Document, col: u32) -> CellStyle {
    doc.sheet(0)
        .expect("one sheet")
        .style(Pos::new(0, col))
        .cloned()
        .unwrap_or_default()
}

const CELL: &str = r##"office:value-type="float" office:value="1""##;

/// A named style's parent is laid under it, an automatic style's named parent under that, and
/// the parents may be declared in any order — `Texte` after `Description`, as in the corpus.
#[test]
fn a_style_inherits_what_its_parents_set() {
    let styles = r##"<office:styles>
<style:style style:name="Child" style:family="table-cell" style:parent-style-name="Base">
 <style:text-properties fo:font-weight="bold"/></style:style>
<style:style style:name="Base" style:family="table-cell" style:parent-style-name="Default">
 <style:table-cell-properties fo:border="0.06pt solid #000000" fo:background-color="#ffcc99"/>
 <style:paragraph-properties fo:text-align="end"/></style:style>
<style:style style:name="Default" style:family="table-cell"/>
</office:styles>
<office:automatic-styles>
<style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Child">
 <style:table-cell-properties fo:background-color="transparent"/></style:style>
</office:automatic-styles>"##;
    let body = format!(
        r##"<table:table-cell table:style-name="ce1" {CELL}/><table:table-cell table:style-name="Child" {CELL}/>"##
    );
    let doc = read(&body, styles);

    let auto = style(&doc, 0);
    assert_eq!(auto.font_weight.as_deref(), Some("bold"), "from Child");
    assert_eq!(auto.align.as_deref(), Some("end"), "from Base, two up");
    assert_eq!(auto.uniform_border(), Some("0.06pt solid #000000"));
    assert_eq!(
        auto.background.as_deref(),
        Some("transparent"),
        "its own wins"
    );

    let named = style(&doc, 1);
    assert_eq!(named.background.as_deref(), Some("#ffcc99"));
    assert_eq!(named.font_weight.as_deref(), Some("bold"));
}

/// `none`, `0` and a stated automatic colour are how a style takes back what its parent set,
/// though the model stores each as nothing at all.
#[test]
fn a_child_can_take_back_a_parents_border() {
    let styles = r##"<office:styles>
<style:style style:name="Boxed" style:family="table-cell">
 <style:table-cell-properties fo:border="0.06pt solid #000000" style:rotation-angle="90"/>
 <style:text-properties style:text-underline-style="solid" fo:color="#1f4e79"/></style:style>
</office:styles>
<office:automatic-styles>
<style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Boxed">
 <style:table-cell-properties fo:border="none" fo:border-top="0.06pt solid #000000" style:rotation-angle="0"/>
 <style:text-properties style:text-underline-style="none" style:use-window-font-color="true"/></style:style>
</office:automatic-styles>"##;
    let doc = read(
        &format!(r##"<table:table-cell table:style-name="ce1" {CELL}/>"##),
        styles,
    );
    let look = style(&doc, 0);
    let top = grind_sheet::style::EDGES
        .iter()
        .position(|edge| *edge == "top")
        .expect("an edge");
    for (i, edge) in look.borders.iter().enumerate() {
        let expected = (i == top).then_some("0.06pt solid #000000");
        assert_eq!(
            edge.as_deref(),
            expected,
            "edge {i}: top kept, the rest taken back"
        );
    }
    assert_eq!(look.rotation, None);
    assert_eq!(look.underline, None);
    assert_eq!(
        look.color, None,
        "the automatic colour, stated, hides the parent's navy"
    );
}

/// The data style comes from the nearest style in the chain that names one.
#[test]
fn a_style_inherits_its_parents_number_format() {
    let styles = r##"<office:styles>
<number:number-style style:name="N1"><number:number number:decimal-places="3" number:min-integer-digits="1"/></number:number-style>
<style:style style:name="Money" style:family="table-cell" style:data-style-name="N1"/>
</office:styles>
<office:automatic-styles>
<style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Money">
 <style:text-properties fo:font-weight="bold"/></style:style>
</office:automatic-styles>"##;
    let body = format!(r##"<table:table-cell table:style-name="ce1" {CELL}/>"##);
    assert_eq!(shown(&body, styles, 1), ["1.000"]);
}

/// ODF 1.3 added `number:min-decimal-places`; before it, `decimal-places="2"` meant every one
/// of them shown. LibreOffice 26.8 shows `1.00` and `1` for these two (`doc/ods-format.md`
/// §5.2).
#[test]
fn decimals_with_no_minimum_are_all_shown() {
    let styles = r##"<office:automatic-styles>
<number:number-style style:name="N1"><number:number number:decimal-places="2" number:min-integer-digits="1"/></number:number-style>
<number:number-style style:name="N2"><number:number number:decimal-places="2" number:min-decimal-places="0" number:min-integer-digits="1"/></number:number-style>
<style:style style:name="ce1" style:family="table-cell" style:data-style-name="N1"/>
<style:style style:name="ce2" style:family="table-cell" style:data-style-name="N2"/>
</office:automatic-styles>"##;
    let body = format!(
        r##"<table:table-cell table:style-name="ce1" {CELL}/><table:table-cell table:style-name="ce2" {CELL}/>"##
    );
    assert_eq!(shown(&body, styles, 2), ["1.00", "1"]);
}

/// `number:embedded-text` is read onto its number, shown among the digits as LibreOffice shows
/// it, and written back as it was read (`doc/ods-format.md` §5.2).
#[test]
fn embedded_text_is_read_shown_and_written() {
    let styles = r##"<office:automatic-styles>
<number:number-style style:name="N1"><number:number number:decimal-places="0" number:min-integer-digits="10"><number:embedded-text number:position="4">-</number:embedded-text><number:embedded-text number:position="7">) </number:embedded-text></number:number></number:number-style>
<style:style style:name="ce1" style:family="table-cell" style:data-style-name="N1"/>
</office:automatic-styles>"##;
    let body = r##"<table:table-cell table:style-name="ce1" office:value-type="float" office:value="5551234567"/>"##;
    assert_eq!(shown(body, styles, 1), ["555) 123-4567"]);
    let doc = read(body, styles);
    let bytes = grind_sheet::write_bytes(&doc, grind_sheet::Form::Flat).expect("writes");
    let back = grind_sheet::read_bytes("back.fods", &bytes).expect("reads back");
    let sheet = |d: &Document| d.sheet(0).expect("a sheet").format(Pos::new(0, 0)).cloned();
    assert_eq!(sheet(&back), sheet(&doc));
    assert!(
        String::from_utf8_lossy(&bytes)
            .contains(r#"<number:embedded-text number:position="7">) </number:embedded-text>"#)
    );
}
