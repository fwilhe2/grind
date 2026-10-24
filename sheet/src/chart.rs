// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart: bar, line or pie (`doc/chart-format.md`). In ODF a chart is an embedded document of
//! its own (`draw:frame`/`draw:object`, rng:5088/5539); this is the part of it that is read and
//! written. It holds ranges rather than values, so editing the data moves the chart.

use serde::{Deserialize, Serialize};

use crate::{App, CellValue, Pos, Result, a1};

/// The three shapes drawn (`doc/chart-format.md`), each a `chart:class` token checked against
/// `soffice` (the schema leaves the attribute free, rng:487-489).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartKind {
    Bar,
    Line,
    Pie,
}

impl ChartKind {
    /// Every kind, in the order a picker offers them.
    pub const ALL: [ChartKind; 3] = [ChartKind::Bar, ChartKind::Line, ChartKind::Pie];

    /// The kind's name as a control shows it — `Bar`, `Line`, `Pie`.
    pub fn name(self) -> &'static str {
        match self {
            ChartKind::Bar => "Bar",
            ChartKind::Line => "Line",
            ChartKind::Pie => "Pie",
        }
    }

    /// The `chart:class` this kind is spelled as.
    pub fn class(self) -> &'static str {
        match self {
            ChartKind::Bar => "chart:bar",
            ChartKind::Line => "chart:line",
            // ODF's name for a pie chart (`doc/chart-format.md`).
            ChartKind::Pie => "chart:circle",
        }
    }

    /// The kind a `chart:class` names; anything not drawn here is `None` (§9).
    pub fn from_class(class: &str) -> Option<Self> {
        match class {
            "chart:bar" => Some(ChartKind::Bar),
            "chart:line" => Some(ChartKind::Line),
            "chart:circle" => Some(ChartKind::Pie),
            _ => None,
        }
    }
}

/// One `chart:series` (rng:857): a range of values and optionally one naming it
/// (`chart:label-cell-address`), both address strings kept verbatim and resolved only when the
/// chart's data is read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub values: String,
    pub label: Option<String>,
    /// This series' colour (`"#rrggbb"`), for a bar or line; `None` is the default cycle
    /// ([`effective_color`]).
    #[serde(default)]
    pub color: Option<String>,
    /// Per-point overrides by position in the series — one bar, or one pie slice. A missing or
    /// `None` entry is no override ([`effective_color`]).
    #[serde(default)]
    pub point_colors: Vec<Option<String>>,
}

/// One axis of a chart: its title, whether its tick labels are drawn, and whether it rules
/// gridlines. The chart's own title and legend are [`Chart::title`] and [`Chart::legend`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Axis {
    /// This axis' own `chart:title` (rng:422-434), plain text.
    pub label: Option<String>,
    /// Whether the tick labels are drawn — categories on x, the value scale on y:
    /// `chart:display-label` (rng:10069), on the axis' style rather than the axis element.
    pub tick_labels: bool,
    /// Whether this axis rules major gridlines: `chart:grid chart:class="major"` (rng:672-693).
    pub gridlines: bool,
}

impl Default for Axis {
    /// A new axis: tick labels on, no gridlines, no title. An axis a file says nothing about is
    /// [`Axis::bare`] instead.
    fn default() -> Self {
        Axis {
            label: None,
            tick_labels: true,
            gridlines: false,
        }
    }
}

impl Axis {
    /// An axis carrying nothing: how an axis a file says nothing about reads. The schema gives
    /// `chart:display-label` no default (rng:10069), and LibreOffice draws no tick labels when it
    /// is absent (measured, `doc/chart-format.md`). Both writers always state it, so this covers
    /// files neither wrote.
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

/// Where a chart's legend sits — `chart:legend-position` (rng:700-707), in ODF's writing-direction
/// words: `End` is the right of a left-to-right chart. The corner positions (rng:718-725) read as
/// the edge they are on, and no position reads as `End`, as LibreOffice places one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Legend {
    Start,
    End,
    Top,
    Bottom,
}

impl Legend {
    /// Every position, in the order a picker offers them — the one a new chart gets first.
    pub const ALL: [Legend; 4] = [Legend::End, Legend::Bottom, Legend::Top, Legend::Start];

    /// The `chart:legend-position` token.
    pub fn token(self) -> &'static str {
        match self {
            Legend::Start => "start",
            Legend::End => "end",
            Legend::Top => "top",
            Legend::Bottom => "bottom",
        }
    }

    /// The position a `chart:legend-position` token names; unrecognised is `End` (§9).
    pub fn from_token(token: &str) -> Self {
        match token {
            "start" => Legend::Start,
            "top" | "top-start" | "top-end" => Legend::Top,
            "bottom" | "bottom-start" | "bottom-end" => Legend::Bottom,
            _ => Legend::End,
        }
    }
}

/// A chart, as modelled: no subtitle or footer, one categories range and one values range per
/// series (`doc/chart-format.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    pub kind: ChartKind,
    /// `chart:categories`' own range (rng:454) — the axis labels, shared by every series.
    pub categories: Option<String>,
    pub series: Vec<Series>,
    /// `draw:frame`'s `svg:x`/`svg:y`/`svg:width`/`svg:height` (rng:1722, rng:1778), verbatim.
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
    /// Which way a pie's slices run from twelve o'clock — the angle axis'
    /// `chart:reverse-direction` (rng:10064), `true` meaning clockwise; meaningless for a bar or
    /// line. A new chart is clockwise; a pie a file says nothing about reads counter-clockwise, as
    /// LibreOffice draws it (`doc/chart-format.md`, Direction). A serialised chart without the
    /// field reads as `true`.
    #[serde(default = "clockwise_by_default")]
    pub clockwise: bool,
    /// The chart's own `chart:title` (rng:466, rng:934), plain text.
    #[serde(default)]
    pub title: Option<String>,
    /// The chart's `chart:legend` (rng:475, rng:694), and where it sits; `None` is no legend.
    #[serde(default)]
    pub legend: Option<Legend>,
}

fn clockwise_by_default() -> bool {
    true
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
            clockwise: clockwise_by_default(),
            title: None,
            legend: None,
        }
    }
}

/// What a chart is, in the words a person types — what [`crate::App::add_chart`] and
/// [`crate::App::edit_chart`] take and [`guess`] produces. Ranges are typed ranges (`A2:A5`,
/// `Data.A2:A5`) resolved by [`parse_range`] against the chart's sheet; a stored range is one too,
/// so [`Spec::of`] can hand an existing chart to a dialog. A series is `RANGE[=LABEL]`. Position
/// is not here ([`crate::App::reshape_chart`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub kind: ChartKind,
    pub categories: Option<String>,
    pub series: Vec<(String, Option<String>)>,
    pub x_axis: Axis,
    pub y_axis: Axis,
    /// A pie's direction, kept whatever the kind so a pie turned into a bar and back keeps it.
    pub clockwise: bool,
    /// The chart's own title ([`Chart::title`]).
    pub title: Option<String>,
    /// Its legend ([`Chart::legend`]).
    pub legend: Option<Legend>,
}

impl Spec {
    /// A new chart of `kind` pointing at nothing yet — every other field a new chart's default.
    pub fn new(kind: ChartKind) -> Self {
        Spec {
            kind,
            categories: None,
            series: Vec::new(),
            x_axis: Axis::default(),
            y_axis: Axis::default(),
            clockwise: clockwise_by_default(),
            title: None,
            legend: None,
        }
    }

    /// The legend a chart gets when nobody said: one when there is more than one thing to tell
    /// apart (two series, or a pie's slices), none for a single bar or line series, which the
    /// title names.
    pub fn default_legend(&self) -> Option<Legend> {
        (self.kind == ChartKind::Pie || self.series.len() > 1).then_some(Legend::End)
    }

    /// An existing chart, as the spec that would make it — what an *Edit Chart* dialog opens on.
    pub fn of(chart: &Chart) -> Self {
        Spec {
            kind: chart.kind,
            categories: chart.categories.clone(),
            series: chart
                .series
                .iter()
                .map(|s| (s.values.clone(), s.label.clone()))
                .collect(),
            x_axis: chart.x_axis.clone(),
            y_axis: chart.y_axis.clone(),
            clockwise: chart.clockwise,
            title: chart.title.clone(),
            legend: chart.legend,
        }
    }
}

/// How a block of cells is read as a chart — the three questions [`guess`] answers from the
/// cells, and a person may overrule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// Each **row** of values is a series, rather than each column.
    pub by_rows: bool,
    /// The block's first row names things — the series, or the categories when `by_rows`.
    pub header_row: bool,
    /// The block's first column names things — the categories, or the series when `by_rows`.
    pub label_column: bool,
}

/// What [`guess`] read: the block (one cell grows into its table), the [`Shape`] it read it as,
/// and the chart that makes.
#[derive(Clone, Debug, PartialEq)]
pub struct Guess {
    pub start: Pos,
    pub end: Pos,
    pub shape: Shape,
    pub spec: Spec,
}

/// A chart of a block of cells, guessed from what they hold, for every shell and `chart-add
/// --from`:
///
/// - One cell grows into the block around it — every row and column touching a non-empty cell.
/// - A first row of text over numbers names the series; a first column of text, dates or a
///   counting sequence (`2021, 2022, 2023`) names the categories.
/// - The values run in columns unless the block is wider than it is tall: twelve months across and
///   two rows of figures is two series of twelve.
/// - A line when the categories are dates or times, else a bar. Never a pie: the cells cannot say
///   whether parts make a whole.
///
/// `shape`, when given, replaces the answers about the block.
pub fn guess(
    sheet: &crate::Sheet,
    start: Pos,
    end: Pos,
    shape: Option<Shape>,
    null_date: i64,
) -> Guess {
    let (start, end) = match start == end {
        true => region(sheet, start),
        false => (start, end),
    };
    let shape = shape.unwrap_or_else(|| read_shape(sheet, start, end));
    let body = Pos::new(
        start.row + u32::from(shape.header_row),
        start.col + u32::from(shape.label_column),
    );
    let range = |from: Pos, to: Pos| match from == to {
        true => a1::format(None, from),
        false => format!("{}:{}", a1::format(None, from), a1::format(None, to)),
    };
    let mut spec = Spec::new(ChartKind::Bar);
    // Nothing to chart — the labels took the block, or nothing left is a number: an empty spec,
    // which a dialog can report.
    let figures = (body.row..=end.row).any(|row| {
        (body.col..=end.col).any(|col| {
            let pos = Pos::new(row, col);
            matches!(sheet.get(pos), CellValue::Number(_)) && crate::date_kind(sheet, pos).is_none()
        })
    });
    if body.row > end.row || body.col > end.col || !figures {
        return Guess {
            start,
            end,
            shape,
            spec,
        };
    }
    let category_cells: Vec<Pos> = match shape.by_rows {
        false => (body.row..=end.row)
            .map(|row| Pos::new(row, start.col))
            .collect(),
        true => (body.col..=end.col)
            .map(|col| Pos::new(start.row, col))
            .collect(),
    };
    let has_categories = match shape.by_rows {
        false => shape.label_column,
        true => shape.header_row,
    };
    if has_categories
        && let (Some(first), Some(last)) = (category_cells.first(), category_cells.last())
    {
        spec.categories = Some(range(*first, *last));
        if category_cells
            .iter()
            .any(|pos| crate::date_kind(sheet, *pos).is_some())
        {
            spec.kind = ChartKind::Line;
        }
    }
    match shape.by_rows {
        false => {
            for col in body.col..=end.col {
                let values = range(Pos::new(body.row, col), Pos::new(end.row, col));
                let label = shape
                    .header_row
                    .then(|| a1::format(None, Pos::new(start.row, col)));
                spec.series.push((values, label));
            }
        }
        true => {
            for row in body.row..=end.row {
                let values = range(Pos::new(row, body.col), Pos::new(row, end.col));
                let label = shape
                    .label_column
                    .then(|| a1::format(None, Pos::new(row, start.col)));
                spec.series.push((values, label));
            }
        }
    }
    // One series needs no legend, and its name is the chart's title.
    if let [(_, Some(label))] = spec.series.as_slice() {
        let pos = a1::parse(label).ok().and_then(|reference| {
            Some(Pos::new(
                reference.start.row?.index,
                reference.start.col?.index,
            ))
        });
        if let Some(pos) = pos {
            let text = crate::render(sheet, pos, null_date);
            spec.title = (!text.trim().is_empty()).then(|| text.trim().to_owned());
        }
    }
    spec.legend = spec.default_legend();
    Guess {
        start,
        end,
        shape,
        spec,
    }
}

/// The block around one cell: grown a row or column at a time while the line just outside it,
/// corners included, holds anything, never past what the sheet uses.
fn region(sheet: &crate::Sheet, at: Pos) -> (Pos, Pos) {
    let (rows, cols) = (sheet.used_rows(), sheet.used_cols());
    if rows == 0 || cols == 0 || at.row >= rows || at.col >= cols {
        return (at, at);
    }
    let filled = |row: u32, col: u32| !matches!(sheet.get(Pos::new(row, col)), CellValue::Empty);
    let (mut top, mut left, mut bottom, mut right) = (at.row, at.col, at.row, at.col);
    loop {
        let (l, r) = (left.saturating_sub(1), (right + 1).min(cols - 1));
        let (t, b) = (top.saturating_sub(1), (bottom + 1).min(rows - 1));
        let grow_up = top > 0 && (l..=r).any(|col| filled(top - 1, col));
        let grow_down = bottom + 1 < rows && (l..=r).any(|col| filled(bottom + 1, col));
        let grow_left = left > 0 && (t..=b).any(|row| filled(row, left - 1));
        let grow_right = right + 1 < cols && (t..=b).any(|row| filled(row, right + 1));
        if !(grow_up || grow_down || grow_left || grow_right) {
            return (Pos::new(top, left), Pos::new(bottom, right));
        }
        top -= u32::from(grow_up);
        bottom += u32::from(grow_down);
        left -= u32::from(grow_left);
        right += u32::from(grow_right);
    }
}

/// The three answers [`guess`] reads out of a block, when nobody gave them.
fn read_shape(sheet: &crate::Sheet, start: Pos, end: Pos) -> Shape {
    let value = |pos: Pos| sheet.get(pos);
    let is_text = |pos: Pos| matches!(value(pos), CellValue::Text(ref t) if !t.is_empty());
    let is_number = |pos: Pos| {
        matches!(value(pos), CellValue::Number(_)) && crate::date_kind(sheet, pos).is_none()
    };
    let cols = start.col..=end.col;
    let any_number_in = |rows: std::ops::RangeInclusive<u32>,
                         cols: std::ops::RangeInclusive<u32>| {
        rows.clone()
            .any(|row| cols.clone().any(|col| is_number(Pos::new(row, col))))
    };

    // A first row of text (nothing in it a number), over rows that hold numbers.
    let header_row = end.row > start.row
        && cols.clone().any(|col| is_text(Pos::new(start.row, col)))
        && !cols.clone().any(|col| is_number(Pos::new(start.row, col)))
        && any_number_in(start.row + 1..=end.row, cols.clone());
    let first_body_row = start.row + u32::from(header_row);

    // A first column of names: text or dates, or a counting sequence — beside columns that hold
    // numbers of their own.
    let column: Vec<Pos> = (first_body_row..=end.row)
        .map(|row| Pos::new(row, start.col))
        .collect();
    let names = column.iter().all(|pos| {
        is_text(*pos) || crate::date_kind(sheet, *pos).is_some() || value(*pos) == CellValue::Empty
    }) && column
        .iter()
        .any(|pos| !matches!(value(*pos), CellValue::Empty));
    let label_column = end.col > start.col
        && (names || counts(&column.iter().map(|pos| value(*pos)).collect::<Vec<_>>()))
        && any_number_in(first_body_row..=end.row, start.col + 1..=end.col);

    let body_rows = end.row - first_body_row + 1;
    let body_cols = end.col - (start.col + u32::from(label_column)) + 1;
    Shape {
        by_rows: body_cols > body_rows,
        header_row,
        label_column,
    }
}

/// Whether a run of cells is a counting sequence — at least three whole numbers with one constant
/// step (`2021, 2022, 2023`) — which labels rows rather than measuring anything.
fn counts(cells: &[CellValue]) -> bool {
    let numbers: Option<Vec<f64>> = cells
        .iter()
        .map(|cell| match cell {
            CellValue::Number(n) if n.fract() == 0.0 => Some(*n),
            _ => None,
        })
        .collect();
    let Some(numbers) = numbers else {
        return false;
    };
    if numbers.len() < 3 {
        return false;
    }
    let step = numbers[1] - numbers[0];
    step != 0.0 && numbers.windows(2).all(|pair| pair[1] - pair[0] == step)
}

/// The colour a mark gets — the one place the writer and every shell's painter resolve it. A bar
/// or line series is one colour ([`Series::color`], else the default cycle at its position), so a
/// legend can name it; a bar may override it per point when `point` is `Some`. A pie colours per
/// slice: an override, else the cycle at the slice's position; `point` must be `Some`.
pub fn effective_color(chart: &Chart, series: usize, point: Option<usize>) -> String {
    let s = &chart.series[series];
    let own = |point: usize| s.point_colors.get(point).cloned().flatten();
    match chart.kind {
        ChartKind::Pie => {
            let point = point.expect("a pie's marks are its slices");
            own(point).unwrap_or_else(|| series_color(point).to_owned())
        }
        ChartKind::Bar => point
            .and_then(own)
            .unwrap_or_else(|| series_own_color(chart, series)),
        ChartKind::Line => series_own_color(chart, series),
    }
}

/// A bar or line series' own colour: its override, else the default cycle at its position.
fn series_own_color(chart: &Chart, series: usize) -> String {
    chart.series[series]
        .color
        .clone()
        .unwrap_or_else(|| series_color(series).to_owned())
}

/// A typed range (`B3:B9`, `Data.B3:B9`) as the address string a chart stores
/// (`Sheet1.B3:Sheet1.B9`), through `a1::parse` and resolved like a formula's reference. An
/// unqualified range means `sheet`, the chart's own.
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

/// One stored range with a sheet renamed (`doc/dsl.md` §6.5): read as [`resolve_range`] reads it,
/// the sheet replaced by `formula::rename`, printed as [`parse_range`] prints. `None` when nothing
/// changed, when it will not parse, or when an end lacks an axis.
pub(crate) fn rename_sheet_in_range(addr: &str, from: &str, to: &str) -> Option<String> {
    let reference = a1::parse_bracketed(&format!("[{addr}]")).ok()?;
    let renamed = crate::formula::rename::rename_in_reference(&reference, from, to);
    if renamed == reference {
        return None;
    }
    let end = renamed.end.clone().unwrap_or_else(|| renamed.start.clone());
    let spell = |cell: &crate::formula::lex::CellRef| {
        let pos = crate::Pos::new(cell.row?.index, cell.col?.index);
        Some(a1::format(cell.sheet.as_deref(), pos))
    };
    let (start, stop) = (spell(&renamed.start)?, spell(&end)?);
    // A one-cell label address stays one cell.
    Some(match renamed.end.is_some() {
        true => format!("{start}:{stop}"),
        false => start,
    })
}

/// The colours a regenerated chart assigns: one per series for a bar or line, one per slice for a
/// pie. [`grind_core::style::PALETTE`] without its neutrals, which read as "no data". A chart that
/// was only read keeps its file's colours.
pub const SERIES_COLORS: [&str; 12] = [
    "blue", "green", "orange", "red", "purple", "teal", "maroon", "olive", "fuchsia", "navy",
    "aqua", "yellow",
];

/// The `n`th colour a regenerated chart assigns, cycling past the end of the table.
pub fn series_color(n: usize) -> &'static str {
    let name = SERIES_COLORS[n % SERIES_COLORS.len()];
    crate::style::palette(name).expect("every name in SERIES_COLORS is in PALETTE")
}

/// The value axis' scale: where its ticks sit and how to spell one, computed from the data here so
/// every shell draws the same axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Ticks {
    /// The distance between two ticks — 1, 2 or 5 times a power of ten.
    pub step: f64,
    /// Every tick from `0.0` up to and including [`Ticks::max`], in order.
    pub values: Vec<f64>,
}

impl Ticks {
    /// The top of the axis: the last tick, at or above the largest value. A plot is scaled against
    /// this, so the top gridline meets the top of the plot.
    pub fn max(&self) -> f64 {
        self.values.last().copied().unwrap_or(1.0)
    }

    /// One tick spelled for an axis, rounded to the decimals [`Ticks::step`] needs (`0.3`, not
    /// `0.30000000000000004`).
    pub fn label(&self, value: f64) -> String {
        let decimals = (-self.step.log10().floor()).clamp(0.0, 6.0) as usize;
        format!("{value:.decimals$}")
    }
}

/// The ticks of a value axis from zero to `max`: a step of 1, 2 or 5 times a power of ten, the
/// smallest keeping the count near `TICK_TARGET`.
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

/// How many intervals [`axis_ticks`] aims for.
const TICK_TARGET: f64 = 5.0;

/// One pie slice's arc in screen radians — 0 at three o'clock, positive clockwise on a y-down
/// surface. `sweep` is signed by the pie's direction. `point` is the slice's position in the
/// series, for its colour and category, which differs from the slice's own index since a
/// non-positive value takes no slice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    pub point: usize,
    pub start: f64,
    pub sweep: f64,
}

/// Where a pie's first slice starts, in either direction: twelve o'clock (`doc/chart-format.md`,
/// Direction).
const TWELVE: f64 = -std::f64::consts::FRAC_PI_2;

/// A pie's slices, in order, from its first series' values; a non-positive value takes no slice.
/// Computed here so every shell draws the same pie. Empty when there is nothing to draw.
pub fn pie_slices(data: &ChartData, clockwise: bool) -> Vec<Slice> {
    let Some((_, values)) = data.series.first() else {
        return Vec::new();
    };
    let total: f64 = values.iter().map(|v| v.max(0.0)).sum();
    // `max` already dropped every NaN, so what is left to refuse is nothing, or infinity.
    if total <= 0.0 || !total.is_finite() {
        return Vec::new();
    }
    let direction = match clockwise {
        true => 1.0,
        false => -1.0,
    };
    let mut start = TWELVE;
    let mut slices = Vec::new();
    for (point, &value) in values.iter().enumerate() {
        if value.is_nan() || value <= 0.0 {
            continue;
        }
        let sweep = direction * (value / total) * std::f64::consts::TAU;
        slices.push(Slice {
            point,
            start,
            sweep,
        });
        start += sweep;
    }
    slices
}

/// Which slice a screen angle (`atan2(dy, dx)` from the centre) falls in, sharing [`pie_slices`]'
/// arithmetic. `None` for a pie without slices; whether the point is inside the circle is the
/// caller's question.
pub fn pie_slice_at(data: &ChartData, clockwise: bool, angle: f64) -> Option<usize> {
    let slices = pie_slices(data, clockwise);
    // How far round from twelve o'clock the point is, travelling the way this pie runs.
    let travelled = match clockwise {
        true => angle - TWELVE,
        false => TWELVE - angle,
    }
    .rem_euclid(std::f64::consts::TAU);
    let mut reached = 0.0;
    for slice in &slices {
        reached += slice.sweep.abs();
        if travelled < reached {
            return Some(slice.point);
        }
    }
    // Rounding at the very end of the last slice lands just past the total.
    slices.last().map(|slice| slice.point)
}

/// A stored range read back to the place it names — the reverse of [`parse_range`].
pub fn resolve_range(app: &App, addr: &str) -> Result<(usize, crate::Pos, crate::Pos)> {
    a1::resolve(app, &a1::parse_bracketed(&format!("[{addr}]"))?)
}

/// A chart's data, resolved against the sheet — what a shell draws from. Categories are their
/// displayed text; a series is its label (empty if none) and its values, a non-number counting as
/// `0.0`.
#[derive(Clone, Debug, PartialEq)]
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

    /// `doc/chart-format.md`, Colour: two series side by side are two colours, and every bar of
    /// one is the same colour.
    #[test]
    fn a_bar_colours_per_series_so_two_series_are_two_colours() {
        let mut chart = Chart::new(
            ChartKind::Bar,
            "0cm".into(),
            "0cm".into(),
            "1cm".into(),
            "1cm".into(),
        );
        chart.series = vec![series("B1:B3"), series("C1:C3")];
        assert_eq!(
            effective_color(&chart, 0, Some(0)),
            effective_color(&chart, 0, Some(2)),
            "every bar of one series is one colour"
        );
        assert_ne!(
            effective_color(&chart, 0, Some(0)),
            effective_color(&chart, 1, Some(0)),
            "two series in the same group are told apart"
        );
        assert_eq!(effective_color(&chart, 1, None), series_color(1));
        // A bar picked by hand still wins over its series, and only that bar.
        chart.series[0].point_colors = vec![None, Some("#123456".to_owned())];
        assert_eq!(effective_color(&chart, 0, Some(1)), "#123456");
        assert_eq!(effective_color(&chart, 0, Some(0)), series_color(0));
        // A whole series picked by hand colours every bar in it.
        chart.series[1].color = Some("#abcdef".to_owned());
        assert_eq!(effective_color(&chart, 1, Some(2)), "#abcdef");
    }

    /// A sheet from rows of cells, `""` being empty and anything that parses a number.
    fn table(rows: &[&[&str]]) -> crate::Sheet {
        let mut sheet = crate::Sheet::new("S");
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let value = match (cell.is_empty(), cell.parse::<f64>()) {
                    (true, _) => continue,
                    (false, Ok(n)) => CellValue::Number(n),
                    (false, Err(_)) => CellValue::Text((*cell).to_owned()),
                };
                sheet.set(Pos::new(r as u32, c as u32), value);
            }
        }
        sheet
    }

    fn series_of(spec: &Spec) -> Vec<(&str, Option<&str>)> {
        spec.series
            .iter()
            .map(|(values, label)| (values.as_str(), label.as_deref()))
            .collect()
    }

    const DOWN: &[&[&str]] = &[
        &["Month", "Sales", "Costs"],
        &["Jan", "10", "4"],
        &["Feb", "20", "6"],
        &["Mar", "30", "9"],
        &["Apr", "40", "12"],
    ];

    const ACROSS: &[&[&str]] = &[
        &["", "Jan", "Feb", "Mar", "Apr", "May"],
        &["Sales", "10", "20", "30", "40", "50"],
        &["Costs", "4", "6", "9", "12", "15"],
    ];

    /// The shape every spreadsheet's first chart has: a column of names, a row of headings,
    /// and a column of figures per series.
    #[test]
    fn a_table_down_the_page_is_a_series_per_column() {
        let guessed = guess(&table(DOWN), Pos::new(0, 0), Pos::new(4, 2), None, 0);
        assert_eq!(
            guessed.shape,
            Shape {
                by_rows: false,
                header_row: true,
                label_column: true
            }
        );
        assert_eq!(guessed.spec.kind, ChartKind::Bar);
        assert_eq!(guessed.spec.categories.as_deref(), Some("A2:A5"));
        assert_eq!(
            series_of(&guessed.spec),
            [("B2:B5", Some("B1")), ("C2:C5", Some("C1"))]
        );
        // Two series are told apart by a legend, and no one of them is the title.
        assert_eq!(guessed.spec.legend, Some(Legend::End));
        assert_eq!(guessed.spec.title, None);
    }

    /// Months across the top are the categories, and each row of figures is a series.
    #[test]
    fn a_table_across_the_page_is_a_series_per_row() {
        let guessed = guess(&table(ACROSS), Pos::new(0, 0), Pos::new(2, 5), None, 0);
        assert!(guessed.shape.by_rows);
        assert_eq!(guessed.spec.categories.as_deref(), Some("B1:F1"));
        assert_eq!(
            series_of(&guessed.spec),
            [("B2:F2", Some("A2")), ("B3:F3", Some("A3"))]
        );
    }

    /// Clicking anywhere in a table means the table.
    #[test]
    fn one_cell_grows_into_the_table_around_it() {
        let mut rows: Vec<&[&str]> = ACROSS.to_vec();
        rows.push(&[]);
        rows.push(&["", "", "", "", "", "", "", "unrelated"]);
        let guessed = guess(&table(&rows), Pos::new(1, 3), Pos::new(1, 3), None, 0);
        assert_eq!(
            (guessed.start, guessed.end),
            (Pos::new(0, 0), Pos::new(2, 5))
        );
    }

    #[test]
    fn dates_along_the_categories_make_a_line() {
        let mut sheet = table(DOWN);
        for row in 1..=4 {
            sheet.set(
                Pos::new(row, 0),
                CellValue::Number(46000.0 + f64::from(row)),
            );
            sheet.set_kind(Pos::new(row, 0), crate::model::NumberKind::Date);
        }
        let guessed = guess(&sheet, Pos::new(0, 0), Pos::new(4, 2), None, 0);
        assert!(guessed.shape.label_column, "a date column names its rows");
        assert_eq!(guessed.spec.kind, ChartKind::Line);
        assert_eq!(guessed.spec.series.len(), 2);
    }

    /// A column that counts labels the rows beside it — a year is not a measurement.
    #[test]
    fn a_counting_column_labels_its_rows_rather_than_being_charted() {
        let rows: &[&[&str]] = &[
            &["Year", "Revenue"],
            &["2021", "5"],
            &["2022", "7"],
            &["2023", "6"],
        ];
        let guessed = guess(&table(rows), Pos::new(0, 0), Pos::new(3, 1), None, 0);
        assert!(guessed.shape.label_column);
        assert_eq!(guessed.spec.categories.as_deref(), Some("A2:A4"));
        assert_eq!(series_of(&guessed.spec), [("B2:B4", Some("B1"))]);
        // One series is what the chart is of: its name is the title, and it needs no legend.
        assert_eq!(guessed.spec.title.as_deref(), Some("Revenue"));
        assert_eq!(guessed.spec.legend, None);
        // Figures that do not count are figures.
        let rows: &[&[&str]] = &[&["A", "B"], &["3", "5"], &["1", "7"], &["8", "6"]];
        let guessed = guess(&table(rows), Pos::new(0, 0), Pos::new(3, 1), None, 0);
        assert!(!guessed.shape.label_column);
        assert_eq!(guessed.spec.series.len(), 2);
    }

    #[test]
    fn a_bare_column_of_figures_is_one_series_with_nothing_naming_it() {
        let rows: &[&[&str]] = &[&["3"], &["1"], &["4"]];
        let guessed = guess(&table(rows), Pos::new(0, 0), Pos::new(2, 0), None, 0);
        assert_eq!(guessed.spec.categories, None);
        assert_eq!(series_of(&guessed.spec), [("A1:A3", None)]);
    }

    /// A person overruling the guess gets exactly what they said, and the rest follows.
    #[test]
    fn a_shape_given_is_a_shape_obeyed() {
        let columns = Shape {
            by_rows: false,
            header_row: true,
            label_column: true,
        };
        let guessed = guess(
            &table(ACROSS),
            Pos::new(0, 0),
            Pos::new(2, 5),
            Some(columns),
            0,
        );
        assert_eq!(guessed.shape, columns);
        assert_eq!(guessed.spec.categories.as_deref(), Some("A2:A3"));
        assert_eq!(guessed.spec.series.len(), 5);
        assert_eq!(guessed.spec.series[0], ("B2:B3".into(), Some("B1".into())));
    }

    #[test]
    fn a_block_with_no_figures_in_it_charts_nothing() {
        let rows: &[&[&str]] = &[&["a", "b"], &["c", "d"]];
        let guessed = guess(&table(rows), Pos::new(0, 0), Pos::new(1, 1), None, 0);
        assert!(guessed.spec.series.is_empty());
        assert_eq!(guessed.spec.categories, None);
    }

    fn pie_of(values: &[f64]) -> ChartData {
        ChartData {
            kind: ChartKind::Pie,
            categories: Vec::new(),
            series: vec![("".into(), values.to_vec())],
        }
    }

    /// Both directions start at twelve o'clock and cover the whole turn; only the sign of the
    /// sweep differs, which is the whole of what `chart:reverse-direction` changes.
    #[test]
    fn a_pie_runs_from_twelve_either_way_round() {
        let data = pie_of(&[1.0, 3.0]);
        for clockwise in [true, false] {
            let slices = pie_slices(&data, clockwise);
            assert_eq!(slices.len(), 2);
            assert_eq!(slices[0].start, TWELVE);
            let turned: f64 = slices.iter().map(|s| s.sweep.abs()).sum();
            assert!((turned - std::f64::consts::TAU).abs() < 1e-9);
            assert_eq!(slices[0].sweep > 0.0, clockwise);
            assert!((slices[1].start - (TWELVE + slices[0].sweep)).abs() < 1e-12);
        }
    }

    /// A point just clockwise of twelve is in the first slice of a clockwise pie and the last of a
    /// counter-clockwise one.
    #[test]
    fn a_hit_just_right_of_twelve_depends_on_which_way_the_pie_runs() {
        let data = pie_of(&[1.0, 1.0, 1.0, 1.0]);
        let just_right_of_twelve = TWELVE + 0.1;
        let just_left_of_twelve = TWELVE - 0.1;
        assert_eq!(pie_slice_at(&data, true, just_right_of_twelve), Some(0));
        assert_eq!(pie_slice_at(&data, false, just_right_of_twelve), Some(3));
        assert_eq!(pie_slice_at(&data, false, just_left_of_twelve), Some(0));
        assert_eq!(pie_slice_at(&data, true, just_left_of_twelve), Some(3));
        // Six o'clock is the boundary between the second and third quarters either way; just
        // past it, travelling each way, is the third slice.
        let six = std::f64::consts::FRAC_PI_2;
        assert_eq!(pie_slice_at(&data, true, six + 0.1), Some(2));
        assert_eq!(pie_slice_at(&data, false, six - 0.1), Some(2));
    }

    /// A value that takes no slice is skipped, but the slices after it keep their own positions
    /// — a colour and a category name are looked up by position, not by slice.
    #[test]
    fn a_zero_takes_no_slice_and_moves_nobody_elses_position() {
        let slices = pie_slices(&pie_of(&[2.0, 0.0, -1.0, 2.0]), true);
        let points: Vec<usize> = slices.iter().map(|s| s.point).collect();
        assert_eq!(points, [0, 3]);
        assert!(pie_slices(&pie_of(&[0.0, 0.0]), true).is_empty());
        assert!(pie_slices(&pie_of(&[f64::NAN]), true).is_empty());
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
