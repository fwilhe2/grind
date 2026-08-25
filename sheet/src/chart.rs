// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart: bar, line or pie, holding ranges rather than values — `doc/chart-format.md` is the
//! clean-room spec this model is built against, citing the schema by line.
//!
//! **A chart is embedded as its own ODF document** (`draw:frame`/`draw:object`, rng:5088/5539),
//! not a spreadsheet element in its own right — this model is the slice of that second
//! document this build reads and writes, not a second document type of its own. It holds
//! ranges rather than values on purpose: a chart tracks the cells it was built from, the way a
//! formula does, so editing the data it points at moves the chart without this build ever
//! being told to.

use serde::{Deserialize, Serialize};

use crate::{App, Result, a1};

/// Which of the three shapes this build knows — `doc/chart-format.md`'s scope line. Each is a
/// `chart:class` token, verified against a real `soffice` build rather than guessed (the schema
/// itself leaves the attribute a free `namespacedToken`, rng:487-489).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartKind {
    Bar,
    Line,
    Pie,
}

impl ChartKind {
    /// The `chart:class` this kind is spelled as.
    pub fn class(self) -> &'static str {
        match self {
            ChartKind::Bar => "chart:bar",
            ChartKind::Line => "chart:line",
            // ODF's own name for a pie chart — the one surprising token of the three,
            // `doc/chart-format.md` has the measurement.
            ChartKind::Pie => "chart:circle",
        }
    }

    /// The kind a `chart:class` names, tolerantly: anything this build does not draw is
    /// `None` rather than an error, the same §9 tolerance every other unrecognised value gets.
    pub fn from_class(class: &str) -> Option<Self> {
        match class {
            "chart:bar" => Some(ChartKind::Bar),
            "chart:line" => Some(ChartKind::Line),
            "chart:circle" => Some(ChartKind::Pie),
            _ => None,
        }
    }
}

/// One `chart:series` (rng:857) — a range of values, and optionally a range naming it
/// (`chart:label-cell-address`). Both are ODF range-address strings, kept verbatim the way
/// `col_widths` keeps `"2.258cm"`: resolved against the live sheet only when something reads
/// this chart's data, not parsed on the way in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub values: String,
    pub label: Option<String>,
}

/// A chart, as this build models it — `doc/chart-format.md`'s scope line is the whole of what
/// is missing: no title, no subtitle, no legend, one categories range and one values range per
/// series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    pub kind: ChartKind,
    /// `chart:categories`' own range (rng:454) — the axis labels, shared by every series.
    pub categories: Option<String>,
    pub series: Vec<Series>,
    /// `draw:frame`'s own `svg:x`/`svg:y`/`svg:width`/`svg:height` (rng:1722, rng:1778), ODF
    /// lengths kept verbatim like every other one in this codebase.
    pub x: String,
    pub y: String,
    pub width: String,
    pub height: String,
}

impl Chart {
    /// A new chart at a given position, with no series yet — `App::add_chart` fills them in.
    pub fn new(kind: ChartKind, x: String, y: String, width: String, height: String) -> Self {
        Chart {
            kind,
            categories: None,
            series: Vec::new(),
            x,
            y,
            width,
            height,
        }
    }
}

/// A user-typed range (`B3:B9`, or `Data.B3:B9` for another sheet), turned into the ODF
/// range-address string a chart stores (`Sheet1.B3:Sheet1.B9`) — `a1::parse`'s own grammar,
/// resolved against the live sheet the way a formula's own reference is, so a chart's ranges
/// are checked the same way anything else typed into this program is.
///
/// Defaults to `sheet` rather than to the first sheet: a chart *on* a sheet whose own ranges
/// name no sheet means that one, unlike [`a1::as_definition`]'s named-range rule, which a
/// formula anywhere in the document can reference and so has no "this sheet" to default to.
pub fn parse_range(app: &App, sheet: usize, addr: &str) -> Result<String> {
    let mut reference = a1::parse(addr)?;
    let name = app.sheet_name(sheet)?;
    if reference.start.sheet.is_none() {
        reference.start.sheet = Some(name.clone());
    }
    if let Some(end) = reference.end.as_mut()
        && end.sheet.is_none()
    {
        end.sheet = Some(name);
    }
    let (resolved, start, stop) = a1::resolve(app, &reference)?;
    let resolved_name = app.sheet_name(resolved)?;
    Ok(format!(
        "{}:{}",
        a1::format(Some(&resolved_name), start),
        a1::format(Some(&resolved_name), stop)
    ))
}

/// The colours a chart this build **regenerates** assigns to its series (bar, line — one per
/// series) or its data points (pie — one per slice, since a pie has no axis to share a colour
/// down). [`grind_core::style::PALETTE`] minus the neutrals (`black`, `white`, `gray`,
/// `silver`), which read as "no data" rather than as a colour — the aesthetic this feature
/// exists for, `doc/chart-format.md` has LibreOffice's own defaults it replaces.
///
/// A chart this build only *read* keeps whatever colours the file already had; this table is
/// consulted only when this build's own writer is the one drawing the style.
pub const SERIES_COLORS: [&str; 12] = [
    "blue", "green", "orange", "red", "purple", "teal", "maroon", "olive", "fuchsia", "navy",
    "aqua", "yellow",
];

/// The `n`th colour a regenerated chart assigns, cycling past the end of the table rather than
/// running out — a pie with more slices than colours repeats rather than draws one with none.
pub fn series_color(n: usize) -> &'static str {
    let name = SERIES_COLORS[n % SERIES_COLORS.len()];
    crate::style::palette(name).expect("every name in SERIES_COLORS is in PALETTE")
}

/// A stored range, read back to the place it names — the reverse of [`parse_range`], used
/// whenever a chart's own ranges are resolved rather than typed: reading a chart back out to
/// list or draw it.
pub fn resolve_range(app: &App, addr: &str) -> Result<(usize, crate::Pos, crate::Pos)> {
    a1::resolve(app, &a1::parse_bracketed(&format!("[{addr}]"))?)
}

/// A chart's data, resolved against the live sheet — what a shell draws from, never the
/// ranges themselves. Categories are their displayed text; a series is its label (empty if it
/// named none) and its values, coerced the way an empty or textual cell coerces into a chart's
/// axis: a number as itself, anything else as `0.0` — a chart draws a bar of nothing rather
/// than refusing the whole picture over one cell that is not a number.
pub struct ChartData {
    pub kind: ChartKind,
    pub categories: Vec<String>,
    pub series: Vec<(String, Vec<f64>)>,
}

impl ChartData {
    /// Read straight from the live sheet — see [`crate::App::chart_data`], the one caller.
    pub fn read(app: &App, chart: &Chart) -> Result<Self> {
        let categories = match &chart.categories {
            Some(range) => strip(app, range)?,
            None => Vec::new(),
        };
        let mut series = Vec::with_capacity(chart.series.len());
        for s in &chart.series {
            let (sheet, start, stop) = resolve_range(app, &s.values)?;
            let viewport =
                app.get_viewport(sheet, start.row..stop.row + 1, start.col..stop.col + 1)?;
            let mut values = Vec::new();
            for row in start.row..=stop.row {
                for col in start.col..=stop.col {
                    values.push(match viewport.get(row, col) {
                        Some(crate::model::CellValue::Number(n)) => *n,
                        Some(crate::model::CellValue::Bool(true)) => 1.0,
                        _ => 0.0,
                    });
                }
            }
            let label = match &s.label {
                Some(range) => strip(app, range)?.into_iter().next().unwrap_or_default(),
                None => String::new(),
            };
            series.push((label, values));
        }
        Ok(ChartData {
            kind: chart.kind,
            categories,
            series,
        })
    }
}

/// A range's displayed text, one entry per cell in reading order.
fn strip(app: &App, range: &str) -> Result<Vec<String>> {
    let (sheet, start, stop) = resolve_range(app, range)?;
    let viewport = app.get_viewport(sheet, start.row..stop.row + 1, start.col..stop.col + 1)?;
    let mut out = Vec::new();
    for row in start.row..=stop.row {
        for col in start.col..=stop.col {
            out.push(viewport.text(row, col).unwrap_or_default().to_owned());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_own_class() {
        for kind in [ChartKind::Bar, ChartKind::Line, ChartKind::Pie] {
            assert_eq!(ChartKind::from_class(kind.class()), Some(kind));
        }
    }

    #[test]
    fn an_unrecognised_class_is_tolerated_as_nothing() {
        assert_eq!(ChartKind::from_class("chart:stock"), None);
    }

    #[test]
    fn the_colour_table_cycles_rather_than_panics_past_its_own_length() {
        let first = series_color(0);
        assert_eq!(series_color(SERIES_COLORS.len()), first);
    }
}
