// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A worksheet's DrawingML drawing (`doc/xlsx-format.md` §4.15): every chart it anchors that the
//! model can hold is read into a [`Chart`] where it sits; a chart it cannot hold is counted as
//! `Dropped::Chart`, and a drawing anchoring anything else — a picture, a shape — as
//! `Dropped::Drawing`, once per drawing part. A chart's values are always its cells': the cache
//! beside each reference is never read.

use grind_sheet::chart::{Axis, Chart, ChartKind, Legend, Series};
use grind_sheet::formula::parse::Expr;
use grind_sheet::{Pos, Sheet};

use crate::names::{Ns, RelType, Seen};
use crate::package::Package;
use crate::report::{Dropped, Report};
use crate::xml::{Handled, Name, Reader};

/// DrawingML's unit: 36,000 to the millimetre, 914,400 to the inch.
const EMU_PER_MM: f64 = 36_000.0;

/// A column no `<col>` sizes is Excel's 64 pixels wide (§4.15).
const DEFAULT_COL_MM: f64 = 64.0 * 25.4 / 96.0;

/// Read every chart the worksheet `sheet_part`'s drawings anchor into `sheet`, which has its
/// tracks sized already, so a chart lands over the cells it was anchored to.
pub fn read(
    package: &mut Package,
    sheet_part: &str,
    sheet: &mut Sheet,
    renames: &[(String, String)],
    report: &mut Report,
    seen: &mut Seen,
) {
    for rel in package.rels(sheet_part, seen) {
        if rel.external || rel.kind != RelType::Drawing {
            continue;
        }
        let Some(bytes) = package.part(&rel.target) else {
            continue;
        };
        // A drawing whose every shape is a form control's DrawingML twin is not a drawing anybody
        // made: the controls are read from VML (`controls.rs`).
        if only_control_twins(&bytes) {
            continue;
        }
        let charts = package.rels(&rel.target, seen);
        let mut other = false;
        for anchor in anchors(&bytes) {
            match anchor.content {
                Content::Chart(id) => {
                    let read = charts
                        .iter()
                        .find(|r| r.id == id && r.kind == RelType::Chart && !r.external)
                        .and_then(|r| package.part(&r.target))
                        .and_then(|bytes| chart(&bytes, &sheet.name, renames));
                    match read {
                        Some(mut chart) => {
                            place(&mut chart, &anchor.place, sheet);
                            let index = sheet.charts().len();
                            sheet.insert_chart(index, chart);
                        }
                        None => report.drop_one(Dropped::Chart),
                    }
                }
                Content::Other => other = true,
                Content::Nothing => {}
            }
        }
        if other {
            report.drop_one(Dropped::Drawing);
        }
    }
}

/// Whether every anchored shape in a DrawingML drawing is the twin of a VML form control —
/// marked by an `a14:compatExt` naming the VML shape (`doc/xlsx-format.md` §4.11). Counted on
/// the bytes: one anchor element per shape, one `compatExt` per twin, and a drawing with no
/// anchors at all is still a drawing.
fn only_control_twins(bytes: &[u8]) -> bool {
    let count = |needle: &[u8]| bytes.windows(needle.len()).filter(|w| *w == needle).count();
    let anchors = count(b"twoCellAnchor") + count(b"oneCellAnchor") + count(b"absoluteAnchor");
    // Each anchor element's name appears twice, in its start tag and its end tag.
    let anchors = anchors / 2;
    anchors > 0 && count(b"compatExt") >= anchors
}

/// A cell and an offset into it, in EMU — an anchor's `xdr:from` or `xdr:to`.
#[derive(Clone, Copy, Debug, Default)]
struct Corner {
    col: u32,
    col_off: f64,
    row: u32,
    row_off: f64,
}

/// Where an anchor puts its shape, as each of the three anchors says it.
#[derive(Debug)]
enum Place {
    /// `xdr:twoCellAnchor`: from one corner to another.
    Cells(Corner, Corner),
    /// `xdr:oneCellAnchor`: from a corner, `cx` × `cy` EMU.
    Sized(Corner, f64, f64),
    /// `xdr:absoluteAnchor`: from the sheet's corner, all in EMU.
    Absolute(f64, f64, f64, f64),
}

#[derive(Debug)]
enum Content {
    /// A chart, by the drawing's relationship id.
    Chart(String),
    /// A picture, a shape, a connector, a group.
    Other,
    Nothing,
}

#[derive(Debug)]
struct Anchor {
    place: Place,
    content: Content,
}

/// Every anchor in a drawing part. A part that will not parse anchors nothing.
fn anchors(bytes: &[u8]) -> Vec<Anchor> {
    let mut reader = Reader::new(bytes);
    let mut out = Vec::new();
    if !matches!(reader.root(), Ok(Some((ref root, _))) if root.in_ns(Ns::SpreadsheetDrawing, "wsDr"))
    {
        return out;
    }
    let _ = reader.children(|reader, name, _| {
        if name.ns != Ns::SpreadsheetDrawing
            || !matches!(
                name.local.as_str(),
                "oneCellAnchor" | "twoCellAnchor" | "absoluteAnchor"
            )
        {
            return Ok(Handled::No);
        }
        let (mut from, mut to) = (Corner::default(), Corner::default());
        let (mut size, mut at) = ((0.0, 0.0), (0.0, 0.0));
        let mut content = Content::Nothing;
        reader.children(|reader, child, attrs| {
            let emu = |local: &str| {
                attrs
                    .plain(local)
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            match (child.ns, child.local.as_str()) {
                (Ns::SpreadsheetDrawing, "from") => from = corner(reader)?,
                (Ns::SpreadsheetDrawing, "to") => to = corner(reader)?,
                (Ns::SpreadsheetDrawing, "ext") => size = (emu("cx"), emu("cy")),
                (Ns::SpreadsheetDrawing, "pos") => at = (emu("x"), emu("y")),
                (Ns::SpreadsheetDrawing, "graphicFrame") => {
                    content = match chart_id(reader)? {
                        Some(id) => Content::Chart(id),
                        None => Content::Other,
                    };
                }
                (Ns::SpreadsheetDrawing, "sp" | "pic" | "grpSp" | "cxnSp" | "contentPart") => {
                    content = Content::Other;
                }
                _ => return Ok(Handled::No),
            }
            Ok(Handled::Yes)
        })?;
        let place = match name.local.as_str() {
            "twoCellAnchor" => Place::Cells(from, to),
            "oneCellAnchor" => Place::Sized(from, size.0, size.1),
            _ => Place::Absolute(at.0, at.1, size.0, size.1),
        };
        out.push(Anchor { place, content });
        Ok(Handled::Yes)
    });
    out
}

/// `xdr:from` or `xdr:to`: `xdr:col`, `xdr:colOff`, `xdr:row`, `xdr:rowOff`, columns and rows
/// 0-based.
fn corner(reader: &mut Reader<'_>) -> grind_ooxml::Result<Corner> {
    let mut corner = Corner::default();
    reader.children(|reader, name, _| {
        if name.ns != Ns::SpreadsheetDrawing {
            return Ok(Handled::No);
        }
        let text = reader.text()?;
        let number = text.trim().parse::<f64>().unwrap_or(0.0);
        match name.local.as_str() {
            "col" => corner.col = number as u32,
            "colOff" => corner.col_off = number,
            "row" => corner.row = number as u32,
            "rowOff" => corner.row_off = number,
            _ => {}
        }
        Ok(Handled::Yes)
    })?;
    Ok(corner)
}

/// The relationship id of the `c:chart` inside a graphic frame, if what it frames is a chart.
fn chart_id(reader: &mut Reader<'_>) -> grind_ooxml::Result<Option<String>> {
    let mut id = None;
    descend(reader, &mut |name, attrs| {
        if name.in_ns(Ns::Chart, "chart") {
            id = attrs.get(Ns::Relationships, "id").map(str::to_owned);
        }
    })?;
    Ok(id)
}

/// Hand every element below the current one to `visit`, depth first.
fn descend(
    reader: &mut Reader<'_>,
    visit: &mut dyn FnMut(&Name, &crate::xml::Attrs),
) -> grind_ooxml::Result<()> {
    reader.children(|reader, name, attrs| {
        visit(name, attrs);
        descend(reader, visit)?;
        Ok(Handled::Yes)
    })
}

/// Where the anchor puts the chart: the cells' sizes up to its corner, plus the offset into it.
fn place(chart: &mut Chart, place: &Place, sheet: &Sheet) {
    let x = |corner: &Corner| {
        (0..corner.col)
            .map(|col| {
                sheet
                    .col_width(col)
                    .and_then(grind_sheet::style::length_mm)
                    .unwrap_or(DEFAULT_COL_MM)
            })
            .sum::<f64>()
            + corner.col_off / EMU_PER_MM
    };
    let y = |corner: &Corner| {
        (0..corner.row)
            .map(|row| {
                sheet
                    .row_height(row)
                    .and_then(grind_sheet::style::length_mm)
                    .unwrap_or(grind_sheet::model::DEFAULT_ROW_MM)
            })
            .sum::<f64>()
            + corner.row_off / EMU_PER_MM
    };
    let (left, top, width, height) = match place {
        Place::Cells(from, to) => (x(from), y(from), x(to) - x(from), y(to) - y(from)),
        Place::Sized(from, cx, cy) => (x(from), y(from), cx / EMU_PER_MM, cy / EMU_PER_MM),
        Place::Absolute(ax, ay, cx, cy) => (
            ax / EMU_PER_MM,
            ay / EMU_PER_MM,
            cx / EMU_PER_MM,
            cy / EMU_PER_MM,
        ),
    };
    let mm = grind_sheet::style::mm_length;
    (chart.x, chart.y, chart.width, chart.height) =
        (mm(left), mm(top), mm(width.max(0.0)), mm(height.max(0.0)));
}

/// One chart type element of a plot area — `c:areaChart` — and its series.
struct Group {
    kind: ChartKind,
    stacked: bool,
    series: Vec<RawSeries>,
}

/// A `c:ser` as the part spells it: references in the workbook's formula syntax.
#[derive(Default)]
struct RawSeries {
    name: Option<String>,
    /// A name given as text, `c:tx/c:v`, rather than a cell.
    text: Option<String>,
    categories: Option<String>,
    values: Option<String>,
    fill: Option<String>,
    line: Option<String>,
}

/// Everything a chart part says that the model holds.
#[derive(Default)]
struct Found {
    title: Option<String>,
    legend: Option<Legend>,
    groups: Vec<Group>,
    x_axis: Option<Axis>,
    y_axis: Option<Axis>,
    value_axes: usize,
    /// Something the model cannot draw as it is meant: a kind not built, percent stacking, a
    /// second value axis.
    refused: bool,
}

/// A chart part as a [`Chart`] on the sheet named `own`, or `None` when the model cannot hold it.
fn chart(bytes: &[u8], own: &str, renames: &[(String, String)]) -> Option<Chart> {
    let mut reader = Reader::new(bytes);
    if !matches!(reader.root(), Ok(Some((ref root, _))) if root.in_ns(Ns::Chart, "chartSpace")) {
        return None;
    }
    let mut found = Found::default();
    reader
        .children(|reader, name, _| {
            if !name.in_ns(Ns::Chart, "chart") {
                return Ok(Handled::No);
            }
            reader.children(|reader, name, _| {
                match (name.ns, name.local.as_str()) {
                    (Ns::Chart, "title") => found.title = rich_text(reader)?,
                    (Ns::Chart, "legend") => {
                        let mut position = Legend::End;
                        reader.children(|_, name, attrs| {
                            if name.in_ns(Ns::Chart, "legendPos") {
                                position = match attrs.plain("val") {
                                    Some("b") => Legend::Bottom,
                                    Some("t") => Legend::Top,
                                    Some("l") => Legend::Start,
                                    _ => Legend::End,
                                };
                            }
                            Ok(Handled::No)
                        })?;
                        found.legend = Some(position);
                    }
                    (Ns::Chart, "plotArea") => plot_area(reader, &mut found)?,
                    _ => return Ok(Handled::No),
                }
                Ok(Handled::Yes)
            })?;
            Ok(Handled::Yes)
        })
        .ok()?;
    build(found, own, renames)
}

fn plot_area(reader: &mut Reader<'_>, found: &mut Found) -> grind_ooxml::Result<()> {
    reader.children(|reader, name, _| {
        if name.ns != Ns::Chart {
            return Ok(Handled::No);
        }
        let kind = match name.local.as_str() {
            "barChart" => ChartKind::Bar,
            "lineChart" => ChartKind::Line,
            "pieChart" => ChartKind::Pie,
            "areaChart" => ChartKind::Area,
            "catAx" | "dateAx" => {
                found.x_axis = Some(axis(reader)?);
                return Ok(Handled::Yes);
            }
            "valAx" => {
                found.value_axes += 1;
                let axis = axis(reader)?;
                found.y_axis.get_or_insert(axis);
                return Ok(Handled::Yes);
            }
            other if other.ends_with("Chart") || other == "serAx" => {
                found.refused = true;
                return Ok(Handled::No);
            }
            _ => return Ok(Handled::No),
        };
        let mut group = Group {
            kind,
            stacked: false,
            series: Vec::new(),
        };
        reader.children(|reader, name, attrs| {
            match (name.ns, name.local.as_str()) {
                (Ns::Chart, "grouping") => match attrs.plain("val") {
                    Some("stacked") => group.stacked = true,
                    Some("percentStacked") => found.refused = true,
                    _ => {}
                },
                (Ns::Chart, "ser") => group.series.push(series(reader)?),
                _ => return Ok(Handled::No),
            }
            Ok(Handled::Yes)
        })?;
        found.groups.push(group);
        Ok(Handled::Yes)
    })
}

fn series(reader: &mut Reader<'_>) -> grind_ooxml::Result<RawSeries> {
    let mut raw = RawSeries::default();
    reader.children(|reader, name, _| {
        match (name.ns, name.local.as_str()) {
            (Ns::Chart, "tx") => (raw.name, raw.text) = source(reader)?,
            (Ns::Chart, "cat") => raw.categories = source(reader)?.0,
            (Ns::Chart, "val") => raw.values = source(reader)?.0,
            (Ns::Chart, "spPr") => (raw.fill, raw.line) = colours(reader)?,
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    Ok(raw)
}

/// Where a series' name or data comes from: the `c:f` inside a `c:strRef`, `c:numRef` or
/// `c:multiLvlStrRef`, and the literal text of a `c:v`.
fn source(reader: &mut Reader<'_>) -> grind_ooxml::Result<(Option<String>, Option<String>)> {
    let (mut formula, mut text) = (None, None);
    reader.children(|reader, name, _| {
        if name.in_ns(Ns::Chart, "v") {
            text = Some(reader.text()?);
            return Ok(Handled::Yes);
        }
        if name.ns != Ns::Chart || !name.local.ends_with("Ref") {
            return Ok(Handled::No);
        }
        reader.children(|reader, name, _| {
            if name.in_ns(Ns::Chart, "f") {
                formula = Some(reader.text()?.trim().to_owned());
                return Ok(Handled::Yes);
            }
            Ok(Handled::No)
        })?;
        Ok(Handled::Yes)
    })?;
    Ok((formula, text))
}

/// A shape's fill and its outline's colour, each `a:solidFill`'s `a:srgbClr`, seen through its
/// `a:alpha` over white (thousandths of a percent).
fn colours(reader: &mut Reader<'_>) -> grind_ooxml::Result<(Option<String>, Option<String>)> {
    let (mut fill, mut line) = (None, None);
    reader.children(|reader, name, _| {
        match (name.ns, name.local.as_str()) {
            (Ns::Drawing, "solidFill") => fill = solid(reader)?,
            (Ns::Drawing, "ln") => {
                reader.children(|reader, name, _| {
                    if name.in_ns(Ns::Drawing, "solidFill") {
                        line = solid(reader)?;
                        return Ok(Handled::Yes);
                    }
                    Ok(Handled::No)
                })?;
            }
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    Ok((fill, line))
}

fn solid(reader: &mut Reader<'_>) -> grind_ooxml::Result<Option<String>> {
    let mut color = None;
    reader.children(|reader, name, attrs| {
        if !name.in_ns(Ns::Drawing, "srgbClr") {
            return Ok(Handled::No);
        }
        let hex = attrs
            .plain("val")
            .filter(|v| v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()))
            .map(|v| format!("#{}", v.to_ascii_lowercase()));
        let mut alpha = None;
        reader.children(|_, name, attrs| {
            if name.in_ns(Ns::Drawing, "alpha") {
                alpha = attrs
                    .plain("val")
                    .and_then(|v| v.trim().parse::<f64>().ok());
            }
            Ok(Handled::No)
        })?;
        color = match (hex, alpha) {
            (Some(hex), Some(alpha)) => grind_sheet::chart::over_white(&hex, alpha / 100_000.0),
            (hex, _) => hex,
        };
        Ok(Handled::Yes)
    })?;
    Ok(color)
}

/// A category or value axis: hidden by `c:delete`, its labels by `c:tickLblPos val="none"`, its
/// gridlines by `c:majorGridlines`, and its title's text.
fn axis(reader: &mut Reader<'_>) -> grind_ooxml::Result<Axis> {
    let mut axis = Axis {
        tick_labels: true,
        ..Axis::bare()
    };
    reader.children(|reader, name, attrs| {
        match (name.ns, name.local.as_str()) {
            (Ns::Chart, "delete") => {
                if matches!(attrs.plain("val"), Some("1" | "true")) {
                    axis.tick_labels = false;
                }
            }
            (Ns::Chart, "tickLblPos") => {
                if attrs.plain("val") == Some("none") {
                    axis.tick_labels = false;
                }
            }
            (Ns::Chart, "majorGridlines") => axis.gridlines = true,
            (Ns::Chart, "title") => {
                axis.label = rich_text(reader)?;
                return Ok(Handled::Yes);
            }
            _ => {}
        }
        Ok(Handled::No)
    })?;
    Ok(axis)
}

/// A title's text: every `a:t` below it, joined; nothing at all is no title.
fn rich_text(reader: &mut Reader<'_>) -> grind_ooxml::Result<Option<String>> {
    let mut text = String::new();
    collect_text(reader, &mut text)?;
    let text = text.trim();
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

fn collect_text(reader: &mut Reader<'_>, out: &mut String) -> grind_ooxml::Result<()> {
    reader.children(|reader, name, _| {
        if name.in_ns(Ns::Drawing, "t") {
            out.push_str(&reader.text()?);
        } else {
            collect_text(reader, out)?;
        }
        Ok(Handled::Yes)
    })
}

/// The chart the parts make, when the model can hold it: one value axis, a pie alone, stacking
/// only in the first chart type — the one whose series the model stacks (`doc/chart-format.md`,
/// Area) — and every series' values in cells.
fn build(found: Found, own: &str, renames: &[(String, String)]) -> Option<Chart> {
    let main = found.groups.first()?;
    let pie_among_others =
        found.groups.len() > 1 && found.groups.iter().any(|g| g.kind == ChartKind::Pie);
    if found.refused
        || found.value_axes > 1
        || pie_among_others
        || found.groups.iter().skip(1).any(|g| g.stacked)
    {
        return None;
    }
    let mut chart = Chart::new(
        main.kind,
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    chart.stacked = main.stacked;
    chart.title = found.title;
    chart.legend = found.legend;
    chart.x_axis = found.x_axis.unwrap_or_else(Axis::bare);
    chart.y_axis = found.y_axis.unwrap_or_else(Axis::bare);
    for group in &found.groups {
        for raw in &group.series {
            let values = address(raw.values.as_deref()?, own, renames)?;
            if chart.categories.is_none() {
                chart.categories = raw
                    .categories
                    .as_deref()
                    .and_then(|f| address(f, own, renames));
            }
            let color = match group.kind {
                ChartKind::Line => raw.line.clone(),
                _ => raw.fill.clone(),
            };
            chart.series.push(Series {
                values,
                label: raw.name.as_deref().and_then(|f| address(f, own, renames)),
                color,
                point_colors: Vec::new(),
                name: raw.text.clone(),
                kind: (group.kind != main.kind).then_some(group.kind),
            });
        }
    }
    (!chart.series.is_empty()).then_some(chart)
}

/// A chart's reference (`Calc!$B$2:$B$5`) as the address a chart stores (`Calc.B2:Calc.B5`),
/// following the sheets the import renamed. `None` for anything but one cell or one rectangle.
fn address(formula: &str, own: &str, renames: &[(String, String)]) -> Option<String> {
    let expr = crate::formula::translate_in(formula, None).ok()?;
    let Expr::Ref(reference) = crate::workbook::renamed(&expr, renames) else {
        return None;
    };
    if reference.source.is_some() {
        return None;
    }
    let sheet = reference
        .start
        .sheet
        .clone()
        .unwrap_or_else(|| own.to_owned());
    let spell = |cell: &grind_sheet::formula::lex::CellRef| {
        let pos = Pos::new(cell.row?.index, cell.col?.index);
        Some(grind_sheet::a1::format(
            Some(cell.sheet.as_deref().unwrap_or(&sheet)),
            pos,
        ))
    };
    let start = spell(&reference.start)?;
    Some(match &reference.end {
        Some(end) => format!("{start}:{}", spell(end)?),
        None => start,
    })
}
