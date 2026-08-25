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
    /// A user-assigned override for this whole series' colour — consulted for [`ChartKind::Line`]
    /// only, an ODF colour (`"#rrggbb"`). `None` means "use the default cycle" — see
    /// [`effective_color`].
    #[serde(default)]
    pub color: Option<String>,
    /// Per-point overrides, sparse by position within this series' own `values` range —
    /// consulted for [`ChartKind::Bar`] and [`ChartKind::Pie`] only. A missing index, or `None`
    /// at one, means "use the default cycle colour for this position" — see [`effective_color`].
    #[serde(default)]
    pub point_colors: Vec<Option<String>>,
}

/// One axis of a chart — everything this build carries about the x or the y one, which is
/// three things: a title, whether its tick labels are drawn, and whether it rules gridlines
/// across the plot. Each is a distinct element or attribute in ODF, cited on its own field.
///
/// This is deliberately *not* the chart's own title or its legend, both of which stay out
/// (`doc/chart-format.md`'s scope line) — an axis' own title is a different element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Axis {
    /// This axis' own `chart:title` (rng:422-434), plain text. A different element from
    /// `chart:chart`'s own title, which this build never reads or writes.
    pub label: Option<String>,
    /// Whether the tick labels along this axis are drawn — the categories on x, the value
    /// scale on y. `chart:display-label` (rng:10069), a `style:chart-properties` attribute on
    /// the axis' own style rather than an attribute of the axis element itself.
    pub tick_labels: bool,
    /// Whether this axis rules major gridlines across the plot —
    /// `chart:grid chart:class="major"` (rng:672-693), an element inside `chart:axis`.
    pub gridlines: bool,
}

impl Default for Axis {
    /// What a **new** axis is: tick labels on, no gridlines, no title. A product decision —
    /// a chart somebody just made should be readable without having to be told to be — and
    /// deliberately *not* what an axis a file says nothing about reads as, which is
    /// [`Axis::bare`].
    fn default() -> Self {
        Axis {
            label: None,
            tick_labels: true,
            gridlines: false,
        }
    }
}

impl Axis {
    /// An axis carrying nothing at all — no title, no tick labels, no gridlines.
    ///
    /// **This, not [`Axis::default`], is how an axis a file says nothing about reads.** The
    /// schema states no default for `chart:display-label` (rng:10069 is a bare optional
    /// boolean), so the oracle decides: LibreOffice draws no tick labels for an axis whose
    /// style omits it, measured in `doc/chart-format.md`. A chart LibreOffice writes always
    /// states the attribute either way, and so does this build's writer, so the case this
    /// default covers is a file neither of them wrote.
    pub fn bare() -> Self {
        Axis {
            label: None,
            tick_labels: false,
            gridlines: false,
        }
    }

    /// Whether this axis needs drawing at all, beyond the marks themselves.
    pub fn is_empty(&self) -> bool {
        self.label.is_none() && !self.tick_labels && !self.gridlines
    }
}

/// A chart, as this build models it — `doc/chart-format.md`'s scope line is the whole of what
/// is missing: no chart-level title, no subtitle, no legend, one categories range and one
/// values range per series. An axis' own title, tick labels and gridlines are in scope — see
/// [`Axis`].
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
    /// The category axis.
    #[serde(default)]
    pub x_axis: Axis,
    /// The value axis.
    #[serde(default)]
    pub y_axis: Axis,
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
            x_axis: Axis::default(),
            y_axis: Axis::default(),
        }
    }
}

/// The colour a mark actually gets: a bar, a pie slice, or a line — the single place the
/// writer and every shell's painter both resolve this, so they can never disagree.
///
/// `point` is `Some` for [`ChartKind::Bar`] and [`ChartKind::Pie`] (one colour per bar or per
/// slice, an override in [`Series::point_colors`] else the default cycle at that position,
/// resetting per series — a pie's own rule, applied to bar too); `None` for
/// [`ChartKind::Line`] (one colour per line, [`Series::color`] else the default cycle at the
/// series' own position).
pub fn effective_color(chart: &Chart, series: usize, point: Option<usize>) -> String {
    let s = &chart.series[series];
    match chart.kind {
        ChartKind::Bar | ChartKind::Pie => {
            let point = point.expect("Bar and Pie marks are per-point");
            s.point_colors
                .get(point)
                .cloned()
                .flatten()
                .unwrap_or_else(|| series_color(point).to_owned())
        }
        ChartKind::Line => s
            .color
            .clone()
            .unwrap_or_else(|| series_color(series).to_owned()),
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

/// The value axis' own scale: where its ticks sit, and how to spell one. Computed from the
/// data rather than stored, and computed **here** rather than in a shell, so that a chart
/// drawn by two different shells is drawn against the same axis — the same reason line layout
/// lives in `grind-core` (`doc/text-layout.md`, Path C) rather than in each window that draws
/// text.
#[derive(Clone, Debug, PartialEq)]
pub struct Ticks {
    /// The distance between two ticks — 1, 2 or 5 times a power of ten.
    pub step: f64,
    /// Every tick from `0.0` up to and including [`Ticks::max`], in order.
    pub values: Vec<f64>,
}

impl Ticks {
    /// The top of the axis: the last tick, which is at or above the largest value plotted.
    /// **This, not the data's own maximum, is what a plot is scaled against** — an axis
    /// rounded up to a tick is what makes a gridline meet the top of the plot instead of
    /// floating just below it.
    pub fn max(&self) -> f64 {
        self.values.last().copied().unwrap_or(1.0)
    }

    /// One tick, spelled for an axis. Rounded to the decimals [`Ticks::step`] actually needs,
    /// so a step of `0.1` reads `0.3` rather than the `0.30000000000000004` a binary float
    /// would otherwise print.
    pub fn label(&self, value: f64) -> String {
        let decimals = (-self.step.log10().floor()).clamp(0.0, 6.0) as usize;
        format!("{value:.decimals$}")
    }
}

/// The ticks a value axis running from zero to `max` gets — a step of 1, 2 or 5 times a power
/// of ten, chosen as the smallest that keeps the count near `TICK_TARGET`. The "nice
/// numbers" rule, which is a presentation decision this build makes once rather than one each
/// shell makes differently.
pub fn axis_ticks(max: f64) -> Ticks {
    if !max.is_finite() || max <= 0.0 {
        return Ticks {
            step: 1.0,
            values: vec![0.0, 1.0],
        };
    }
    let rough = max / TICK_TARGET;
    let magnitude = 10f64.powf(rough.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|multiple| multiple * magnitude)
        .find(|step| *step >= rough)
        // Unreachable in exact arithmetic (`10 * magnitude > rough` by construction), and a
        // rounding error at the boundary is worth a wider axis rather than a panic.
        .unwrap_or(magnitude * 10.0);
    let count = (max / step).ceil().max(1.0) as usize;
    Ticks {
        step,
        // Multiplied rather than accumulated: adding `step` to itself `count` times drifts.
        values: (0..=count).map(|i| i as f64 * step).collect(),
    }
}

/// How many intervals [`axis_ticks`] aims for. Five is the count that reads as a scale without
/// becoming a ladder — the number of gridlines a reader can count without counting.
const TICK_TARGET: f64 = 5.0;

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

    fn series(values: &str) -> Series {
        Series {
            values: values.to_owned(),
            label: None,
            color: None,
            point_colors: Vec::new(),
        }
    }

    #[test]
    fn a_bar_colours_per_point_by_default_like_a_pie() {
        let mut chart = Chart::new(
            ChartKind::Bar,
            "0cm".into(),
            "0cm".into(),
            "1cm".into(),
            "1cm".into(),
        );
        chart.series = vec![series("B1:B3")];
        assert_ne!(
            effective_color(&chart, 0, Some(0)),
            effective_color(&chart, 0, Some(1))
        );
        assert_eq!(effective_color(&chart, 0, Some(0)), series_color(0));
        assert_eq!(effective_color(&chart, 0, Some(1)), series_color(1));
    }

    #[test]
    fn a_line_colours_per_series_by_default() {
        let mut chart = Chart::new(
            ChartKind::Line,
            "0cm".into(),
            "0cm".into(),
            "1cm".into(),
            "1cm".into(),
        );
        chart.series = vec![series("B1:B3"), series("C1:C3")];
        assert_eq!(effective_color(&chart, 0, None), series_color(0));
        assert_eq!(effective_color(&chart, 1, None), series_color(1));
    }

    #[test]
    fn a_point_override_beats_the_default_cycle() {
        let mut chart = Chart::new(
            ChartKind::Pie,
            "0cm".into(),
            "0cm".into(),
            "1cm".into(),
            "1cm".into(),
        );
        let mut s = series("B1:B3");
        s.point_colors = vec![None, Some("#123456".to_owned())];
        chart.series = vec![s];
        assert_eq!(effective_color(&chart, 0, Some(0)), series_color(0));
        assert_eq!(effective_color(&chart, 0, Some(1)), "#123456");
    }

    #[test]
    fn ticks_run_from_zero_to_at_least_the_largest_value() {
        for max in [1.0, 7.0, 99.0, 100.0, 1234.0, 0.37] {
            let ticks = axis_ticks(max);
            assert_eq!(ticks.values.first(), Some(&0.0), "max {max}");
            assert!(ticks.max() >= max, "max {max} ticked to {}", ticks.max());
            // Near enough to the target that the axis reads as a scale rather than a ladder
            // or a pair of endpoints.
            assert!(
                (3..=11).contains(&ticks.values.len()),
                "max {max} gave {} ticks",
                ticks.values.len()
            );
        }
    }

    #[test]
    fn a_tick_is_spelled_with_the_decimals_its_own_step_needs() {
        assert_eq!(axis_ticks(1000.0).label(400.0), "400");
        let tenths = axis_ticks(0.5);
        assert_eq!(tenths.label(tenths.values[3]), "0.3");
    }

    #[test]
    fn an_axis_of_nothing_still_has_a_scale_rather_than_dividing_by_zero() {
        let ticks = axis_ticks(0.0);
        assert!(ticks.max() > 0.0);
        assert_eq!(axis_ticks(f64::NAN).max(), 1.0);
    }

    #[test]
    fn a_series_colour_override_beats_the_default_cycle_for_a_line() {
        let mut chart = Chart::new(
            ChartKind::Line,
            "0cm".into(),
            "0cm".into(),
            "1cm".into(),
            "1cm".into(),
        );
        let mut s = series("B1:B3");
        s.color = Some("#abcdef".to_owned());
        chart.series = vec![s];
        assert_eq!(effective_color(&chart, 0, None), "#abcdef");
    }
}
