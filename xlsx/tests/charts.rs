// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A workbook's charts, in the shape a spreadsheet service exports a fan chart
//! (`doc/xlsx-format.md` §4.15) — rebuilt with invented data, since the file is not ours to vendor:
//! stacked bands of an area chart and a line over them, on a sheet of their own, plotting a
//! calculation sheet's columns, with no cached values.

use std::fmt::Write as _;
use std::io::{Cursor, Write};

use grind_sheet::{ChartKind, ChartLegend};
use grind_xlsx::Dropped;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";

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

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let mut body = String::new();
    for (id, kind, target) in entries {
        let _ = write!(
            body,
            r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"/>"#
        );
    }
    format!(r#"<Relationships xmlns="{PKG}">{body}</Relationships>"#)
}

/// One series: its name from a cell, its values from a column of `Calc`, its fill or line.
fn series(index: usize, column: &str, fill: &str) -> String {
    format!(
        r#"<c:ser><c:idx val="{index}"/><c:order val="{index}"/>
             <c:tx><c:strRef><c:f>Calc!${column}$1</c:f></c:strRef></c:tx>
             <c:spPr>{fill}</c:spPr>
             <c:cat><c:strRef><c:f>Calc!$A$2:$A$5</c:f></c:strRef></c:cat>
             <c:val><c:numRef><c:f>Calc!${column}$2:${column}$5</c:f><c:numCache/></c:numRef></c:val>
           </c:ser>"#
    )
}

const BAND: &str =
    r#"<a:solidFill><a:srgbClr val="000000"><a:alpha val="30000"/></a:srgbClr></a:solidFill>"#;
const LINE: &str = r#"<a:ln w="28575"><a:solidFill><a:srgbClr val="2C3A82"/></a:solidFill></a:ln>"#;

fn fan_chart() -> String {
    format!(
        r#"<c:chartSpace xmlns:c="{C}" xmlns:a="{A}" xmlns:r="{REL}"><c:chart><c:plotArea>
             <c:layout><c:manualLayout><c:x val="0.18"/></c:manualLayout></c:layout>
             <c:areaChart><c:grouping val="stacked"/>{}{}<c:axId val="7"/><c:axId val="8"/></c:areaChart>
             <c:lineChart>{}<c:axId val="7"/><c:axId val="8"/></c:lineChart>
             <c:catAx><c:axId val="7"/><c:delete val="0"/><c:axPos val="b"/>
               <c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t></a:t></a:r></a:p></c:rich></c:tx></c:title>
               <c:crossAx val="8"/></c:catAx>
             <c:valAx><c:axId val="8"/><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/>
               <c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>per month</a:t></a:r></a:p></c:rich></c:tx></c:title>
               <c:tickLblPos val="nextTo"/><c:crossAx val="7"/></c:valAx>
           </c:plotArea><c:legend><c:legendPos val="b"/></c:legend></c:chart></c:chartSpace>"#,
        series(0, "B", BAND),
        series(1, "C", BAND),
        series(2, "D", LINE).replace(
            "<c:tx><c:strRef><c:f>Calc!$D$1</c:f></c:strRef></c:tx>",
            "<c:tx><c:v>Median line</c:v></c:tx>"
        )
    )
}

/// A drawing anchoring `anchors`, each an `xdr:*Anchor` element.
fn drawing(anchors: &str) -> String {
    format!(
        r#"<xdr:wsDr xmlns:xdr="{XDR}" xmlns:a="{A}" xmlns:r="{REL}" xmlns:c="{C}">{anchors}</xdr:wsDr>"#
    )
}

fn chart_anchor(rel: &str, col: u32, row: u32, col_off: u64) -> String {
    format!(
        r#"<xdr:oneCellAnchor><xdr:from><xdr:col>{col}</xdr:col><xdr:colOff>{col_off}</xdr:colOff>
             <xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
             <xdr:ext cx="5760000" cy="3600000"/>
             <xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="1" name="Chart 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>
               <xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm>
               <a:graphic><a:graphicData uri="{C}"><c:chart r:id="{rel}"/></a:graphicData></a:graphic>
             </xdr:graphicFrame><xdr:clientData/></xdr:oneCellAnchor>"#
    )
}

/// `Plan`, the sheet the chart is on — column A 20mm wide, rows 18pt — and `Calc`, the columns it
/// plots, with a drawing that anchors nothing.
fn workbook(chart: &str, plan_drawing: &str) -> Vec<u8> {
    let mut calc_rows = String::from(
        r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Age</t></is></c><c r="B1" t="inlineStr"><is><t>Base</t></is></c><c r="C1" t="inlineStr"><is><t>Band</t></is></c><c r="D1" t="inlineStr"><is><t>Median</t></is></c></row>"#,
    );
    for r in 2..=5 {
        let _ = write!(
            calc_rows,
            r#"<row r="{r}"><c r="A{r}"><v>{}</v></c><c r="B{r}"><v>100</v></c><c r="C{r}"><v>{}</v></c><c r="D{r}"><v>150</v></c></row>"#,
            60 + r,
            10 * r
        );
    }
    let mut plan_rows = String::new();
    for r in 1..=3 {
        let _ = write!(
            plan_rows,
            r#"<row r="{r}" ht="18" customHeight="1"><c r="A{r}"><v>{r}</v></c></row>"#
        );
    }
    package(&[
        (
            "_rels/.rels",
            rels(&[("rId1", "officeDocument", "xl/workbook.xml")]),
        ),
        (
            "xl/workbook.xml",
            format!(
                r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets>
                     <sheet name="Plan" sheetId="1" r:id="rId1"/><sheet name="Calc" sheetId="2" r:id="rId2"/>
                   </sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            rels(&[
                ("rId1", "worksheet", "worksheets/sheet1.xml"),
                ("rId2", "worksheet", "worksheets/sheet2.xml"),
            ]),
        ),
        (
            "xl/worksheets/sheet1.xml",
            format!(
                r#"<worksheet xmlns="{MAIN}" xmlns:r="{REL}">
                     <cols><col min="1" max="1" width="10.79" customWidth="1"/></cols>
                     <sheetData>{plan_rows}</sheetData><drawing r:id="rId1"/></worksheet>"#
            ),
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            rels(&[("rId1", "drawing", "../drawings/drawing1.xml")]),
        ),
        (
            "xl/worksheets/sheet2.xml",
            format!(
                r#"<worksheet xmlns="{MAIN}" xmlns:r="{REL}"><sheetData>{calc_rows}</sheetData><drawing r:id="rId1"/></worksheet>"#
            ),
        ),
        (
            "xl/worksheets/_rels/sheet2.xml.rels",
            rels(&[("rId1", "drawing", "../drawings/drawing2.xml")]),
        ),
        ("xl/drawings/drawing1.xml", plan_drawing.to_owned()),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            rels(&[("rId1", "chart", "../charts/chart1.xml")]),
        ),
        ("xl/drawings/drawing2.xml", drawing("")),
        ("xl/charts/chart1.xml", chart.to_owned()),
    ])
}

#[test]
fn a_fan_chart_is_read_where_it_is_anchored() {
    let bytes = workbook(&fan_chart(), &drawing(&chart_anchor("rId1", 1, 2, 360_000)));
    let (document, report) = grind_xlsx::import_bytes(&bytes).unwrap();
    let charts = document.sheets[0].charts();
    assert_eq!(charts.len(), 1, "{:?}", report.dropped);
    let chart = &charts[0];
    assert_eq!(chart.kind, ChartKind::Area);
    assert!(chart.stacked);
    assert_eq!(chart.categories.as_deref(), Some("Calc.A2:Calc.A5"));
    let values: Vec<_> = chart.series.iter().map(|s| s.values.as_str()).collect();
    assert_eq!(
        values,
        ["Calc.B2:Calc.B5", "Calc.C2:Calc.C5", "Calc.D2:Calc.D5"]
    );
    assert_eq!(chart.series[0].label.as_deref(), Some("Calc.B1"));
    // A name given as text rather than a cell.
    assert_eq!(chart.series[2].label, None);
    assert_eq!(chart.series[2].name.as_deref(), Some("Median line"));
    let kinds: Vec<_> = chart.series.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [None, None, Some(ChartKind::Line)]);
    // 30% black over white, and the line's own colour.
    assert_eq!(chart.series[0].color.as_deref(), Some("#b3b3b3"));
    assert_eq!(chart.series[2].color.as_deref(), Some("#2c3a82"));
    assert_eq!(chart.legend, Some(ChartLegend::Bottom));
    assert!(chart.y_axis.gridlines && chart.y_axis.tick_labels);
    assert_eq!(chart.y_axis.label.as_deref(), Some("per month"));
    // An empty title is no title.
    assert_eq!(chart.x_axis.label, None);
    // Column A is 10.79 characters of 7 pixels, plus a 10mm offset into B; two 18pt rows down.
    let mm = |length: &str| grind_sheet::style::length_mm(length).unwrap();
    assert!(
        (mm(&chart.x) - (10.79 * 7.0 * 25.4 / 96.0 + 10.0)).abs() < 0.01,
        "{}",
        chart.x
    );
    assert!(
        (mm(&chart.y) - 2.0 * 18.0 * 25.4 / 72.0).abs() < 0.01,
        "{}",
        chart.y
    );
    assert!((mm(&chart.width) - 160.0).abs() < 0.01, "{}", chart.width);
    assert!((mm(&chart.height) - 100.0).abs() < 0.01, "{}", chart.height);
    // Nothing lost: the chart was read, and the other drawing held nothing.
    assert_eq!(
        report.dropped.get(&Dropped::Chart),
        None,
        "{:?}",
        report.dropped
    );
    assert_eq!(
        report.dropped.get(&Dropped::Drawing),
        None,
        "{:?}",
        report.dropped
    );
}

/// A kind not built yet, and a picture, are counted as before — each as what it is.
#[test]
fn what_is_not_a_chart_this_build_draws_is_counted() {
    let doughnut = format!(
        r#"<c:chartSpace xmlns:c="{C}" xmlns:a="{A}"><c:chart><c:plotArea>
             <c:doughnutChart>{}</c:doughnutChart></c:plotArea></c:chart></c:chartSpace>"#,
        series(0, "B", "")
    );
    let picture = r#"<xdr:oneCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff>
        <xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="1" cy="1"/>
        <xdr:pic/><xdr:clientData/></xdr:oneCellAnchor>"#;
    let bytes = workbook(
        &doughnut,
        &drawing(&format!("{}{picture}", chart_anchor("rId1", 1, 2, 0))),
    );
    let (document, report) = grind_xlsx::import_bytes(&bytes).unwrap();
    assert_eq!(document.sheets[0].charts(), []);
    assert_eq!(
        report.dropped.get(&Dropped::Chart),
        Some(&1),
        "{:?}",
        report.dropped
    );
    assert_eq!(
        report.dropped.get(&Dropped::Drawing),
        Some(&1),
        "{:?}",
        report.dropped
    );
}

/// What the import wrote is what a reader reads back, and LibreOffice too (loop C covers the
/// model; this covers the import's own values).
#[test]
fn an_imported_chart_survives_its_own_writer() {
    let bytes = workbook(&fan_chart(), &drawing(&chart_anchor("rId1", 1, 2, 0)));
    let (odf, _) = grind_xlsx::open(&bytes).unwrap();
    let back = grind_sheet::read_bytes("fan.fods", &odf).unwrap();
    let chart = &back.sheets[0].charts()[0];
    assert!(chart.stacked);
    assert_eq!(chart.series[2].kind, Some(ChartKind::Line));
}

/// The legend names a series by its text when no cell does.
#[test]
fn a_series_named_by_text_is_named_so_in_the_legend() {
    let bytes = workbook(&fan_chart(), &drawing(&chart_anchor("rId1", 1, 2, 0)));
    let (odf, _) = grind_xlsx::open(&bytes).unwrap();
    let app = grind_sheet::App::new();
    app.open_bytes("fan.fods", &odf).unwrap();
    let data = app.chart_data(0, 0).unwrap();
    let names: Vec<_> = data.series.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["Base", "Band", "Median line"]);
}
