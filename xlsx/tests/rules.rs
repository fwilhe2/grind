// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel's conditional formats as the model's one rule type (`xlsx/src/rules.rs`), over the
//! workbook `doc/xlsx-format.md` §4.12 measured LibreOffice's conversion of — four `dxf`s and
//! six `cfRule`s, in XML a reviewer can read here.

use std::io::{Cursor, Write};

use grind_sheet::Pos;
use grind_sheet::rule::Rule;
use grind_sheet::style::CellStyle;
use grind_xlsx::Dropped;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn package(parts: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut out));
        let options: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        for (name, content) in parts {
            zip.start_file(*name, options).expect("a zip entry");
            zip.write_all(content.as_bytes()).expect("writing an entry");
        }
        zip.finish().expect("a finished zip");
    }
    out
}

fn workbook(rules: &str, dxfs: &str) -> Vec<u8> {
    let rows: String = (1..=5)
        .map(|r| {
            let cells: String = ["A", "B", "C", "D"]
                .iter()
                .enumerate()
                .map(|(i, c)| format!(r#"<c r="{c}{r}"><v>{}</v></c>"#, r * (i + 1)))
                .collect();
            format!(r#"<row r="{r}">{cells}</row>"#)
        })
        .collect();
    package(&[
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/workbook.xml",
            format!(
                r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                r#"<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{REL}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{REL}/styles" Target="styles.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/styles.xml",
            format!(
                r#"<styleSheet xmlns="{MAIN}"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellXfs><dxfs>{dxfs}</dxfs></styleSheet>"#
            ),
        ),
        (
            "xl/worksheets/sheet1.xml",
            format!(
                r#"<worksheet xmlns="{MAIN}"><sheetData>{rows}</sheetData>{rules}</worksheet>"#
            ),
        ),
    ])
}

/// §4.12's four `dxf`s: a `patternFill` with no `patternType`, a solid fill whose `fgColor` and
/// `bgColor` differ, a bold coloured font, and `<b val="0"/>` with one border edge.
const DXFS: &str = r#"
    <dxf><fill><patternFill><bgColor rgb="FFFF0000"/></patternFill></fill></dxf>
    <dxf><fill><patternFill patternType="solid"><fgColor rgb="FF00FF00"/><bgColor rgb="FF0000FF"/></patternFill></fill></dxf>
    <dxf><font><b/><color rgb="FF00FF00"/></font></dxf>
    <dxf><font><b val="0"/><u/></font><border><left style="thin"><color rgb="FFFF0000"/></left></border><numFmt numFmtId="200" formatCode="0.0"/></dxf>"#;

const RULES: &str = r#"
    <conditionalFormatting sqref="C3:C4 A1:A2"><cfRule type="expression" dxfId="0" priority="1"><formula>B1&gt;2</formula></cfRule></conditionalFormatting>
    <conditionalFormatting sqref="B1:B5"><cfRule type="cellIs" dxfId="1" priority="2" operator="between"><formula>3</formula><formula>$D$2</formula></cfRule></conditionalFormatting>
    <conditionalFormatting sqref="D1:D5">
      <cfRule type="cellIs" dxfId="2" priority="4" operator="notEqual"><formula>"x"</formula></cfRule>
      <cfRule type="containsText" dxfId="3" priority="3" operator="containsText" text="a"><formula>NOT(ISERROR(SEARCH("a",D1)))</formula></cfRule>
      <cfRule type="top10" dxfId="3" priority="5" rank="2"/>
      <cfRule type="colorScale" priority="6"><colorScale><cfvo type="min"/><cfvo type="max"/><color rgb="FFFF0000"/><color rgb="FF00FF00"/></colorScale></cfRule>
    </conditionalFormatting>"#;

fn fill(colour: &str) -> CellStyle {
    CellStyle {
        background: Some(colour.into()),
        ..CellStyle::default()
    }
}

#[test]
fn every_rule_with_a_formula_is_carried_in_priority_order() {
    let (document, report) = grind_xlsx::import_bytes(&workbook(RULES, DXFS)).expect("imports");
    let p = Pos::new;
    let rules = document.sheets[0].rules();
    assert_eq!(
        rules,
        [
            // Written from the top-left of the whole `sqref`, A1, not from C3.
            Rule {
                ranges: vec![(p(2, 2), p(3, 2)), (p(0, 0), p(1, 0))],
                base: p(0, 0),
                condition: "[.B1]>2".into(),
                style: fill("#ff0000"),
            },
            // A dxf's solid fill is its bgColor.
            Rule::over(
                p(0, 1),
                p(4, 1),
                "AND([.B1]>=3;[.B1]<=[.$D$2])",
                fill("#0000ff")
            ),
            // Priority 3 before priority 4, whatever order the file listed them in.
            Rule::over(
                p(0, 3),
                p(4, 3),
                "NOT(ISERROR(SEARCH(\"a\";[.D1])))",
                CellStyle {
                    font_weight: Some("normal".into()),
                    underline: Some("solid".into()),
                    borders: [Some("0.74pt solid #ff0000".into()), None, None, None],
                    ..CellStyle::default()
                },
            ),
            Rule::over(
                p(0, 3),
                p(4, 3),
                "[.D1]<>\"x\"",
                CellStyle {
                    font_weight: Some("bold".into()),
                    color: Some("#00ff00".into()),
                    ..CellStyle::default()
                },
            ),
        ]
    );
    assert_eq!(report.rules, 4);
    // The top-ten rule and the colour scale are a second rule engine each.
    assert_eq!(report.dropped.get(&Dropped::ConditionalFormat), Some(&2));
    // What the fourth dxf could not say, once per rule.
    let lost: Vec<&str> = report.appearance_lost.keys().map(|a| a.label()).collect();
    assert!(
        lost.contains(&"number format applied by a conditional rule (per rule)"),
        "{lost:?}"
    );
    assert!(!report.lossless());
}

#[test]
fn an_imported_rule_draws_once_the_document_is_written() {
    let (document, _) = grind_xlsx::import_bytes(&workbook(RULES, DXFS)).expect("imports");
    let bytes = grind_sheet::write_bytes(&document, grind_sheet::Form::Flat).expect("writes");
    let app = grind_sheet::App::new();
    app.open_bytes("rules.fods", &bytes).unwrap();
    let background = |row: u32, col: u32| {
        let view = app.get_viewport(0, row..row + 1, col..col + 1).unwrap();
        view.style(row, col).and_then(|s| s.background.clone())
    };
    // B2 = 4 holds `B1>2` written from A1 — A2 and C3:C4 share the rule.
    assert_eq!(background(1, 0).as_deref(), Some("#ff0000"));
    assert_eq!(background(0, 0), None, "B1 = 2 is not > 2");
    assert_eq!(
        background(2, 2).as_deref(),
        Some("#ff0000"),
        "C3 reads D3 = 12"
    );
    // B1:B5 hold 2, 4, 6, 8, 10, and between 3 and D2 = 8.
    let between: Vec<bool> = (0..5).map(|r| background(r, 1).is_some()).collect();
    assert_eq!(between, [false, true, true, true, false]);
}

#[test]
fn a_rule_with_no_dxf_or_one_the_model_cannot_draw_is_counted() {
    let rules = r#"<conditionalFormatting sqref="A1"><cfRule type="expression" priority="1"><formula>TRUE</formula></cfRule><cfRule type="expression" dxfId="0" priority="2"><formula>TRUE</formula></cfRule></conditionalFormatting>"#;
    let dxfs = r#"<dxf><font><vertAlign val="superscript"/></font></dxf>"#;
    let (document, report) = grind_xlsx::import_bytes(&workbook(rules, dxfs)).expect("imports");
    assert!(document.sheets[0].rules().is_empty());
    assert_eq!(report.dropped.get(&Dropped::ConditionalFormat), Some(&2));
}
