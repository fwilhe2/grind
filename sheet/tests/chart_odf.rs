// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading `table:shapes`/`draw:frame`/`draw:object` — `doc/chart-format.md`'s two shapes,
//! both measured from a real `soffice` (26.2.5.2) rather than invented. The bar chart's own
//! nesting is `ltwbw2026.fods`/`.ods`'s exact structure; line and pie were built the same way
//! through a real LibreOffice's own UNO API and are reproduced here rather than vendored,
//! since the shape under test is the XML rather than the picture.

use grind_sheet::{ChartKind, read_bytes};

/// Wrap a `table:table` body in the smallest valid flat document that also declares the
/// namespaces a chart's own embedded document needs.
fn doc(body: &str) -> grind_sheet::Document {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  office:mimetype="application/vnd.oasis.opendocument.spreadsheet"
  office:version="1.4">
 <office:body><office:spreadsheet>{body}</office:spreadsheet></office:body>
</office:document>"#
    );
    read_bytes("test.fods", xml.as_bytes()).expect("fixture must parse")
}

/// A chart embedded inline, `chart:class` and a categories/series pair the only things that
/// vary — the shape every one of `ltwbw2026.fods`'s bar chart, and a `soffice`-built line and
/// pie chart, all three actually have.
fn inline_chart(class: &str) -> String {
    format!(
        r#"<table:table table:name="Sheet1">
  <table:shapes>
   <draw:frame draw:name="Frame1" svg:x="1cm" svg:y="2cm" svg:width="10cm" svg:height="8cm">
    <draw:object>
     <office:document office:mimetype="application/vnd.oasis.opendocument.chart" office:version="1.4">
      <office:body><office:chart>
       <chart:chart svg:width="10cm" svg:height="8cm" chart:class="{class}">
        <chart:plot-area>
         <chart:axis chart:dimension="x">
          <chart:categories table:cell-range-address="Sheet1.A2:Sheet1.A4"/>
         </chart:axis>
         <chart:axis chart:dimension="y"/>
         <chart:series chart:class="{class}"
                        chart:values-cell-range-address="Sheet1.B2:Sheet1.B4"
                        chart:label-cell-address="Sheet1.B1:Sheet1.B1">
          <chart:data-point chart:repeated="3"/>
         </chart:series>
        </chart:plot-area>
       </chart:chart>
      </office:chart></office:body>
     </office:document>
    </draw:object>
   </draw:frame>
  </table:shapes>
 </table:table>"#
    )
}

#[test]
fn a_bar_chart_reads_kind_position_and_ranges() {
    let d = doc(&inline_chart("chart:bar"));
    let charts = d.sheet(0).unwrap().charts();
    assert_eq!(charts.len(), 1);
    let chart = &charts[0];
    assert_eq!(chart.kind, ChartKind::Bar);
    assert_eq!(chart.categories.as_deref(), Some("Sheet1.A2:Sheet1.A4"));
    assert_eq!(chart.series.len(), 1);
    assert_eq!(chart.series[0].values, "Sheet1.B2:Sheet1.B4");
    assert_eq!(
        chart.series[0].label.as_deref(),
        Some("Sheet1.B1:Sheet1.B1")
    );
    assert_eq!((chart.x.as_str(), chart.y.as_str()), ("1cm", "2cm"));
    assert_eq!(
        (chart.width.as_str(), chart.height.as_str()),
        ("10cm", "8cm")
    );
}

#[test]
fn a_line_chart_reads_its_own_class() {
    let d = doc(&inline_chart("chart:line"));
    assert_eq!(d.sheet(0).unwrap().charts()[0].kind, ChartKind::Line);
}

#[test]
fn a_pie_chart_reads_circle_as_its_class() {
    // ODF's own name for a pie chart — `doc/chart-format.md` has the measurement.
    let d = doc(&inline_chart("chart:circle"));
    assert_eq!(d.sheet(0).unwrap().charts()[0].kind, ChartKind::Pie);
}

#[test]
fn an_unrecognised_chart_class_is_tolerated_as_no_chart_at_all() {
    let d = doc(&inline_chart("chart:stock"));
    assert_eq!(
        d.sheet(0).unwrap().charts().len(),
        0,
        "an unknown chart type is dropped, not an error — §9 tolerance"
    );
}

/// The package form's own shape: `draw:object` points at a separate part (`Object 1/
/// content.xml`, rooted at `office:document-content` rather than `office:document`) instead
/// of embedding one — measured from `ltwbw2026.ods`.
#[test]
fn a_package_form_chart_is_resolved_against_its_own_part() {
    let outer = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  office:version="1.4">
 <office:body><office:spreadsheet>
  <table:table table:name="Sheet1">
   <table:shapes>
    <draw:frame draw:name="Frame1" svg:x="1cm" svg:y="2cm" svg:width="10cm" svg:height="8cm">
     <draw:object xlink:href="./Object 1" xlink:type="simple" xlink:show="embed"
                   xlink:actuate="onLoad"/>
    </draw:frame>
   </table:shapes>
  </table:table>
 </office:spreadsheet></office:body>
</office:document-content>"#;

    let part = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0"
  xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
  office:version="1.4">
 <office:body><office:chart>
  <chart:chart svg:width="10cm" svg:height="8cm" chart:class="chart:bar">
   <chart:plot-area>
    <chart:axis chart:dimension="x">
     <chart:categories table:cell-range-address="Sheet1.A2:Sheet1.A4"/>
    </chart:axis>
    <chart:series chart:class="chart:bar"
                   chart:values-cell-range-address="Sheet1.B2:Sheet1.B4"/>
   </chart:plot-area>
  </chart:chart>
 </office:chart></office:body>
</office:document-content>"#;

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    std::io::Write::write_all(&mut zip, b"application/vnd.oasis.opendocument.spreadsheet").unwrap();
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("content.xml", deflated).unwrap();
    std::io::Write::write_all(&mut zip, outer.as_bytes()).unwrap();
    zip.start_file("Object 1/content.xml", deflated).unwrap();
    std::io::Write::write_all(&mut zip, part.as_bytes()).unwrap();
    let bytes = zip.finish().unwrap().into_inner();

    let d = read_bytes("test.ods", &bytes).expect("parses");
    let charts = d.sheet(0).unwrap().charts();
    assert_eq!(charts.len(), 1);
    assert_eq!(charts[0].kind, ChartKind::Bar);
    assert_eq!(charts[0].categories.as_deref(), Some("Sheet1.A2:Sheet1.A4"));
    assert_eq!(charts[0].series[0].values, "Sheet1.B2:Sheet1.B4");
}

/// A shape that is not a chart at all — a plain picture, say — is simply not a chart, the same
/// way an image with no `draw:image` inside it is not a run in the word processor.
#[test]
fn a_frame_with_no_chart_inside_it_is_simply_not_a_chart() {
    let d = doc(r#"<table:table table:name="Sheet1">
  <table:shapes>
   <draw:frame draw:name="Frame1" svg:x="1cm" svg:y="1cm" svg:width="5cm" svg:height="5cm"/>
  </table:shapes>
 </table:table>"#);
    assert_eq!(d.sheet(0).unwrap().charts().len(), 0);
}

/// The shape LibreOffice writes for an Excel fan chart: an area chart whose plot area is stacked,
/// with a line among its series under its own `chart:class`, and every band 30% opaque.
#[test]
fn a_stacked_area_chart_reads_its_stacking_and_each_series_own_class() {
    let d = doc(r##"<table:table table:name="Sheet1">
  <table:shapes>
   <draw:frame svg:x="0cm" svg:y="0cm" svg:width="16cm" svg:height="10cm">
    <draw:object>
     <office:document office:mimetype="application/vnd.oasis.opendocument.chart" office:version="1.4">
      <office:automatic-styles>
       <style:style style:name="ch3" style:family="chart">
        <style:chart-properties chart:interpolation="cubic-spline" chart:stacked="true"/>
       </style:style>
       <style:style style:name="ch7" style:family="chart">
        <style:graphic-properties draw:stroke="none" draw:fill-color="#000000" draw:opacity="30%"/>
       </style:style>
       <style:style style:name="ch8" style:family="chart">
        <style:chart-properties chart:symbol-type="none"/>
        <style:graphic-properties svg:stroke-color="#2c3a82" draw:fill-color="#2c3a82"/>
       </style:style>
      </office:automatic-styles>
      <office:body><office:chart>
       <chart:chart svg:width="16cm" svg:height="10cm" chart:class="chart:area">
        <chart:plot-area chart:style-name="ch3">
         <chart:axis chart:dimension="x">
          <chart:categories table:cell-range-address="Sheet1.A2:Sheet1.A4"/>
         </chart:axis>
         <chart:axis chart:dimension="y"/>
         <chart:series chart:style-name="ch7" chart:values-cell-range-address="Sheet1.B2:Sheet1.B4" chart:class="chart:area"/>
         <chart:series chart:style-name="ch7" chart:values-cell-range-address="Sheet1.C2:Sheet1.C4" chart:class="chart:area"/>
         <chart:series chart:style-name="ch8" chart:values-cell-range-address="Sheet1.D2:Sheet1.D4" chart:class="chart:line"/>
        </chart:plot-area>
       </chart:chart>
      </office:chart></office:body>
     </office:document>
    </draw:object>
   </draw:frame>
  </table:shapes>
 </table:table>"##);
    let chart = &d.sheet(0).unwrap().charts()[0];
    assert_eq!(chart.kind, ChartKind::Area);
    assert!(chart.stacked);
    let kinds: Vec<_> = chart.series.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [None, None, Some(ChartKind::Line)]);
    // 30% black over the chart's white: the colour a reader sees, carried as one.
    assert_eq!(chart.series[0].color.as_deref(), Some("#b3b3b3"));
    assert_eq!(chart.series[2].color.as_deref(), Some("#2c3a82"));
}

#[test]
fn a_plot_area_that_says_nothing_is_not_stacked() {
    let d = doc(&inline_chart("chart:area"));
    let chart = &d.sheet(0).unwrap().charts()[0];
    assert_eq!(chart.kind, ChartKind::Area);
    assert!(!chart.stacked);
    assert_eq!(chart.series[0].kind, None);
}

/// A series named by text no cell holds, as LibreOffice writes one: `loext:label-string`, a
/// quoted string.
#[test]
fn a_series_named_by_text_reads_its_name() {
    let d = doc(&inline_chart("chart:line").replace(
        r#"chart:label-cell-address="Sheet1.B1:Sheet1.B1""#,
        r#"xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0" loext:label-string="&quot;Median&quot;""#,
    ));
    let series = &d.sheet(0).unwrap().charts()[0].series[0];
    assert_eq!(series.label, None);
    assert_eq!(series.name.as_deref(), Some("Median"));
}

/// LibreOffice writes `chart:label-cell-address=""` (or blanks) for a series it has no cell for:
/// no address is no label.
#[test]
fn a_blank_label_address_is_no_label() {
    for blank in ["", "  "] {
        let d = doc(&inline_chart("chart:bar").replace(
            r#"chart:label-cell-address="Sheet1.B1:Sheet1.B1""#,
            &format!(r#"chart:label-cell-address="{blank}""#),
        ));
        assert_eq!(
            d.sheet(0).unwrap().charts()[0].series[0].label,
            None,
            "{blank:?}"
        );
    }
}
