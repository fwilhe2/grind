// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A checklist workbook as an online spreadsheet exports one — the **shape** of a real file
//! somebody tried to open, rebuilt here with invented content, since the file itself is not
//! ours to vendor.
//!
//! What the shape is, read off that file's parts rather than remembered:
//!
//! * a heading in `B2`, merged across `B2:D2`, bold and centred — which, with the merge
//!   dropped, drew centred in `B2` alone and looked wrong;
//! * section headers merged across two columns, the **covered** cell carrying the border the
//!   merged area is drawn with;
//! * a note merged *down* one column across three item rows;
//! * the tick of each item as a boolean in a **hidden** column `E` — the exporter writes the
//!   checkbox's state there and no control at all — with one `expression` rule per block,
//!   `$E5=TRUE`, filling the item's row when it is ticked;
//! * a summary row merged across `B:D` whose formula counts the ticks, with two more rules on
//!   it, thresholds on that count;
//! * external hyperlinks on the item notes, a sheet with its grid lines off, and every row
//!   22.5 points tall by hand.
//!
//! The merges are carried. The conditional rules are counted and not carried: that is the
//! next piece of work (`doc/conditional-format.md`), and the count is what says so.

use std::io::{Cursor, Write};

use grind_sheet::{CellValue, Pos, Span};
use grind_xlsx::Dropped;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MCE: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

fn package(parts: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut out));
        let options: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in parts {
            zip.start_file(*name, options).expect("a zip entry");
            zip.write_all(content.as_bytes()).expect("writing an entry");
        }
        zip.finish().expect("a finished zip");
    }
    out
}

/// The workbook. Style ids, as `cellXfs` below numbers them:
///
/// | id | used for |
/// |---|---|
/// | 1 | the sheet's plain cells, vertically centred |
/// | 2 | the heading: bold 16pt, centred both ways |
/// | 3 | a section header's top-left cell: white bold on a solid fill, bordered |
/// | 4 | a section header's covered cell: the border only |
/// | 5 | an item's tick column `B` and text `C`: filled, bordered |
/// | 6 | an item's note `D`: underlined, filled, bordered |
/// | 7 | the summary: bold, centred |
fn checklist() -> Vec<u8> {
    let strings = [
        "Moving house: a checklist",   // 0
        "Before the move",             // 1
        "Notes",                       // 2
        "Give notice on the old flat", // 3
        "Template letter",             // 4
        "Book the van",                // 5
        "Compare prices",              // 6
        "Pack the kitchen last",       // 7
        "Labels: one colour per room", // 8
        "After the move",              // 9
        "Register the new address",    // 10
        "Update the bank's records",   // 11
        "Read the meters",             // 12
        "Photograph each one",         // 13
    ];
    let shared = format!(
        r#"<sst xmlns="{MAIN}" count="{n}" uniqueCount="{n}">{items}</sst>"#,
        n = strings.len(),
        items = strings
            .iter()
            .map(|s| format!("<si><t>{s}</t></si>"))
            .collect::<String>()
    );

    let styles = format!(
        r#"<styleSheet xmlns="{MAIN}">
  <fonts count="5">
    <font><sz val="11.0"/><name val="Calibri"/></font>
    <font><sz val="11.0"/><color rgb="FF222222"/><name val="Montserrat"/></font>
    <font><b/><sz val="16.0"/><color rgb="FF222222"/><name val="Montserrat"/></font>
    <font><b/><sz val="11.0"/><color rgb="FFFFFFFF"/><name val="Montserrat"/></font>
    <font><u/><sz val="11.0"/><color rgb="FF1155CC"/><name val="Montserrat"/></font>
  </fonts>
  <fills count="4">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="lightGray"/></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FF2E4057"/><bgColor rgb="FF2E4057"/></patternFill></fill>
    <fill><patternFill patternType="solid"><fgColor rgb="FFF2F2F2"/><bgColor rgb="FFF2F2F2"/></patternFill></fill>
  </fills>
  <borders count="2">
    <border/>
    <border>
      <left style="thin"><color rgb="FFFFFFFF"/></left><right style="thin"><color rgb="FFFFFFFF"/></right>
      <top style="thin"><color rgb="FFFFFFFF"/></top><bottom style="thin"><color rgb="FFFFFFFF"/></bottom>
    </border>
  </borders>
  <cellStyleXfs count="1"><xf borderId="0" fillId="0" fontId="0" numFmtId="0"/></cellStyleXfs>
  <cellXfs count="8">
    <xf borderId="0" fillId="0" fontId="0" numFmtId="0" xfId="0"/>
    <xf borderId="0" fillId="0" fontId="1" numFmtId="0" xfId="0" applyAlignment="1" applyFont="1"><alignment vertical="center"/></xf>
    <xf borderId="0" fillId="0" fontId="2" numFmtId="0" xfId="0" applyAlignment="1" applyFont="1"><alignment horizontal="center" vertical="center"/></xf>
    <xf borderId="1" fillId="2" fontId="3" numFmtId="0" xfId="0" applyAlignment="1" applyBorder="1" applyFill="1" applyFont="1"><alignment horizontal="center" vertical="center"/></xf>
    <xf borderId="1" fillId="0" fontId="1" numFmtId="0" xfId="0" applyBorder="1"/>
    <xf borderId="1" fillId="3" fontId="1" numFmtId="0" xfId="0" applyAlignment="1" applyBorder="1" applyFill="1" applyFont="1"><alignment vertical="center"/></xf>
    <xf borderId="1" fillId="3" fontId="4" numFmtId="0" xfId="0" applyAlignment="1" applyBorder="1" applyFill="1" applyFont="1"><alignment vertical="center" wrapText="1"/></xf>
    <xf borderId="0" fillId="0" fontId="2" numFmtId="0" xfId="0" applyAlignment="1" applyFont="1"><alignment horizontal="center" vertical="center"/></xf>
  </cellXfs>
  <cellStyles count="1"><cellStyle xfId="0" name="Normal" builtinId="0"/></cellStyles>
  <dxfs count="3">
    <dxf><fill><patternFill patternType="solid"><fgColor rgb="FF97E8CA"/><bgColor rgb="FF97E8CA"/></patternFill></fill></dxf>
    <dxf><font><color rgb="FFFFFFFF"/></font><fill><patternFill patternType="solid"><fgColor rgb="FF2E7D32"/><bgColor rgb="FF2E7D32"/></patternFill></fill></dxf>
    <dxf><font><color rgb="FF222222"/></font><fill><patternFill patternType="solid"><fgColor rgb="FFFFD54F"/><bgColor rgb="FFFFD54F"/></patternFill></fill></dxf>
  </dxfs>
</styleSheet>"#
    );

    // One row: `(row, [(col, style, content)])`, content `s:N` for a shared string, `b:0`/`b:1`
    // for a boolean, `f:…` for a formula, and nothing for a styled blank.
    let row = |r: u32, cells: &[(&str, u32, &str)]| {
        let cells: String = cells
            .iter()
            .map(|(col, s, content)| {
                let at = format!("{col}{r}");
                match content.split_once(':') {
                    Some(("s", n)) => format!(r#"<c r="{at}" s="{s}" t="s"><v>{n}</v></c>"#),
                    Some(("b", v)) => format!(r#"<c r="{at}" s="{s}" t="b"><v>{v}</v></c>"#),
                    Some(("f", f)) => {
                        format!(r#"<c r="{at}" s="{s}" t="str"><f>{f}</f><v></v></c>"#)
                    }
                    _ => format!(r#"<c r="{at}" s="{s}"/>"#),
                }
            })
            .collect();
        format!(r#"<row r="{r}" ht="22.5" customHeight="1">{cells}</row>"#)
    };
    let item = |r: u32, text: &str, note: &str, ticked: bool| {
        let mut cells = vec![("A", 1, ""), ("B", 5, ""), ("C", 5, text)];
        if !note.is_empty() {
            cells.push(("D", 6, note));
        } else {
            cells.push(("D", 6, ""));
        }
        cells.push(("E", 1, if ticked { "b:1" } else { "b:0" }));
        row(r, &cells)
    };
    let summary = "IF(COUNTIF(E:E,TRUE)&gt;0,&quot;Done: &quot;&amp;ROUND(100*COUNTIF(E:E,TRUE)/COUNTA(E:E),0)&amp;&quot;%&quot;,&quot;&quot;)";
    let rows = [
        row(1, &[("A", 1, ""), ("B", 1, "")]),
        row(2, &[("A", 1, ""), ("B", 2, "s:0")]),
        row(3, &[("A", 1, "")]),
        row(
            4,
            &[("A", 1, ""), ("B", 3, "s:1"), ("C", 4, ""), ("D", 3, "s:2")],
        ),
        item(5, "s:3", "s:4", true),
        item(6, "s:5", "s:6", false),
        item(7, "s:7", "s:8", false),
        item(8, "s:12", "", true),
        item(9, "s:11", "", false),
        row(10, &[("A", 1, "")]),
        row(
            11,
            &[("A", 1, ""), ("B", 3, "s:9"), ("C", 4, ""), ("D", 3, "s:2")],
        ),
        item(12, "s:10", "s:13", false),
        row(
            13,
            &[
                ("A", 1, ""),
                ("B", 7, &format!("f:{summary}")),
                ("C", 7, ""),
                ("D", 7, ""),
            ],
        ),
    ]
    .concat();

    let sheet = format!(
        r#"<worksheet xmlns="{MAIN}" xmlns:r="{REL}" xmlns:mc="{MCE}"
            xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac">
  <sheetPr><pageSetUpPr/></sheetPr>
  <sheetViews><sheetView showGridLines="0" workbookViewId="0"/></sheetViews>
  <sheetFormatPr customHeight="1" defaultColWidth="14.43" defaultRowHeight="15.0"/>
  <cols>
    <col customWidth="1" min="1" max="1" width="11.43"/>
    <col customWidth="1" min="2" max="2" width="3.14"/>
    <col customWidth="1" min="3" max="3" width="70.43"/>
    <col customWidth="1" min="4" max="4" width="36.43"/>
    <col customWidth="1" hidden="1" min="5" max="5" width="11.43"/>
  </cols>
  <sheetData>{rows}</sheetData>
  <mergeCells count="5">
    <mergeCell ref="B2:D2"/><mergeCell ref="B4:C4"/><mergeCell ref="D7:D9"/>
    <mergeCell ref="B11:C11"/><mergeCell ref="B13:D13"/>
  </mergeCells>
  <conditionalFormatting sqref="B5:C9 D5:D6"><cfRule type="expression" dxfId="0" priority="1"><formula>$E5=TRUE</formula></cfRule></conditionalFormatting>
  <conditionalFormatting sqref="B12:D12"><cfRule type="expression" dxfId="0" priority="2"><formula>$E12=TRUE</formula></cfRule></conditionalFormatting>
  <conditionalFormatting sqref="B13"><cfRule type="expression" dxfId="1" priority="3"><formula>COUNTIF(E:E,TRUE)/COUNTA(E:E)&gt;=0.75</formula></cfRule></conditionalFormatting>
  <conditionalFormatting sqref="B13"><cfRule type="expression" dxfId="2" priority="4"><formula>COUNTIF(E:E,TRUE)/COUNTA(E:E)&gt;=0.33</formula></cfRule></conditionalFormatting>
  <hyperlinks><hyperlink r:id="rId1" ref="D5"/><hyperlink r:id="rId2" ref="D6"/></hyperlinks>
  <printOptions/>
  <pageMargins bottom="0.787401575" footer="0.0" header="0.0" left="0.7" right="0.7" top="0.787401575"/>
  <pageSetup orientation="landscape"/>
</worksheet>"#
    );

    let sheet_rels = format!(
        r#"<Relationships xmlns="{PKG}">
  <Relationship Id="rId1" Type="{REL}/hyperlink" Target="https://example.invalid/letter" TargetMode="External"/>
  <Relationship Id="rId2" Type="{REL}/hyperlink" Target="https://example.invalid/vans" TargetMode="External"/>
</Relationships>"#
    );

    package(&[
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default ContentType="application/xml" Extension="xml"/>
  <Default ContentType="application/vnd.openxmlformats-package.relationships+xml" Extension="rels"/>
  <Override ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml" PartName="/xl/workbook.xml"/>
  <Override ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml" PartName="/xl/worksheets/sheet1.xml"/>
  <Override ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml" PartName="/xl/sharedStrings.xml"/>
  <Override ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml" PartName="/xl/styles.xml"/>
</Types>"#
                .to_owned(),
        ),
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/workbook.xml",
            format!(
                r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><workbookPr/><sheets><sheet state="visible" name="Checklist" sheetId="1" r:id="rId3"/></sheets><definedNames/><calcPr/></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                r#"<Relationships xmlns="{PKG}">
  <Relationship Id="rId1" Type="{REL}/styles" Target="styles.xml"/>
  <Relationship Id="rId2" Type="{REL}/sharedStrings" Target="sharedStrings.xml"/>
  <Relationship Id="rId3" Type="{REL}/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#
            ),
        ),
        ("xl/sharedStrings.xml", shared),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", sheet),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels),
    ])
}

fn p(row: u32, col: u32) -> Pos {
    Pos::new(row, col)
}

#[test]
fn the_heading_and_the_sections_are_merged() {
    let (document, report) = grind_xlsx::import_bytes(&checklist()).expect("it imports");
    let sheet = &document.sheets[0];
    let merges: Vec<(Pos, Span)> = sheet.merges().collect();
    let span = |cols, rows| Span { cols, rows };
    assert_eq!(
        merges,
        [
            (p(1, 1), span(3, 1)),  // B2:D2, the heading
            (p(3, 1), span(2, 1)),  // B4:C4
            (p(6, 3), span(1, 3)),  // D7:D9, a note down three items
            (p(10, 1), span(2, 1)), // B11:C11
            (p(12, 1), span(3, 1)), // B13:D13, the summary
        ]
    );
    assert_eq!(report.dropped.get(&Dropped::MergedCells), None);

    // The heading is drawn from its top-left cell, centred across the whole merge.
    assert_eq!(
        sheet.get(p(1, 1)),
        CellValue::Text("Moving house: a checklist".into())
    );
    let heading = sheet.style(p(1, 1)).expect("the heading is styled");
    assert_eq!(heading.align.as_deref(), Some("center"));
    assert_eq!(heading.font_weight.as_deref(), Some("bold"));

    // A covered cell keeps its own look — the border on the merged header's right edge.
    assert!(sheet.covered(p(3, 2)));
    assert!(
        sheet.style(p(3, 2)).is_some_and(|s| s.borders[1].is_some()),
        "{:?}",
        sheet.style(p(3, 2))
    );
}

#[test]
fn the_ticks_are_booleans_in_a_hidden_column() {
    let (document, _) = grind_xlsx::import_bytes(&checklist()).expect("it imports");
    let sheet = &document.sheets[0];
    assert!(sheet.col_hidden(4), "column E is hidden");
    let ticks: Vec<CellValue> = [4, 5, 6, 7, 8, 11]
        .iter()
        .map(|row| sheet.get(p(*row, 4)))
        .collect();
    use CellValue::Bool;
    assert_eq!(
        ticks,
        [
            Bool(true),
            Bool(false),
            Bool(false),
            Bool(true),
            Bool(false),
            Bool(false)
        ]
    );
    // The summary counts them: a formula the filter translates, in the merge's top-left cell.
    assert!(
        sheet
            .formula(p(12, 1))
            .is_some_and(|f| f.contains("COUNTIF"))
    );
}

/// The rules that tick a row green, and the two thresholds on the summary: counted, one per
/// `<cfRule>`, until the model has the rule type that carries them.
#[test]
fn the_conditional_rules_are_counted_not_carried() {
    let (_, report) = grind_xlsx::import_bytes(&checklist()).expect("it imports");
    assert_eq!(report.dropped.get(&Dropped::ConditionalFormat), Some(&4));
}

/// What a shell does with it: write the flat file and read it back, merges and all.
#[test]
fn the_merges_survive_the_conversion() {
    let (document, _) = grind_xlsx::import_bytes(&checklist()).expect("it imports");
    let bytes = grind_sheet::write_bytes(&document, grind_sheet::Form::Flat).expect("writes");
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        text.contains("table:number-columns-spanned=\"3\""),
        "the heading's span is written"
    );
    let back = grind_sheet::read_bytes("checklist.fods", &bytes).expect("reads back");
    assert_eq!(
        back.sheets[0].merges().collect::<Vec<_>>(),
        document.sheets[0].merges().collect::<Vec<_>>()
    );
    assert_eq!(
        back.sheets[0].style(p(3, 2)),
        document.sheets[0].style(p(3, 2))
    );
}
