// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart as a list of [`Mark`]s, so where every bar, point, slice and label goes is tested on
//! any host and a shell only draws them. Scaled against [`axis_ticks`], pies swept by
//! [`pie_slices`], colours from [`effective_color`], and text measured through the grid's
//! `Metrics`. Coordinates are the caller's unit.

use grind_core::color::{self, Rgb};
use grind_core::layout::Metrics;
use grind_core::style::TextStyle;

use crate::{
    Chart, ChartAxis, ChartData, ChartKind, ChartLegend, Ticks, axis_ticks, effective_color,
    pie_slices,
};

/// A rectangle in the caller's unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
}

/// The shell's theme colours, which the document's colours are lifted against on a dark page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colours {
    /// The ground under the chart.
    pub page: Rgb,
    /// Titles and legend words.
    pub ink: Rgb,
    /// Hairlines — the card's edge and the gridlines.
    pub grid: Rgb,
    /// Tick labels, axis titles and the baseline.
    pub header_ink: Rgb,
    /// A mark whose colour names nothing this build can read.
    pub accent: Rgb,
    /// Whether the page is dark.
    pub dark: bool,
}

/// One thing to put down.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    Fill {
        rect: Rect,
        color: Rgb,
    },
    /// One line of text whose box starts at `(x, top)`, turned `turn` degrees counter-clockwise
    /// about that corner, drawn only inside `clip`.
    Text {
        x: f64,
        top: f64,
        text: String,
        style: TextStyle,
        color: Rgb,
        clip: Rect,
        turn: f64,
    },
    /// A polygon: filled when `fill` is set, stroked with `(colour, width)` when `stroke` is.
    Path {
        points: Vec<(f64, f64)>,
        fill: Option<Rgb>,
        stroke: Option<(Rgb, f64)>,
    },
}

/// How wide `text` is set in `style` — the last cumulative advance.
pub fn width(metrics: &dyn Metrics, text: &str, style: &TextStyle) -> f64 {
    let mut advances = Vec::new();
    metrics.advances(text, style, &mut advances);
    advances.last().copied().map_or(0.0, f64::from)
}

/// `text` whole if it fits in `room`, else its longest start that fits with "…" after it — so a
/// category's name stays under its own band — and `None` when not even one letter does.
pub fn shorten(text: &str, room: f64, width: impl Fn(&str) -> f64) -> Option<String> {
    if width(text) <= room {
        return Some(text.to_owned());
    }
    text.char_indices()
        .rev()
        .map(|(end, _)| text[..end].trim_end())
        .filter(|start| !start.is_empty())
        .map(|start| format!("{start}…"))
        .find(|cut| width(cut) <= room)
}

/// How a category axis sets its names: level, each within its own band, or turned
/// [`TURN_DEGREES`] when one is wider than its band — as LibreOffice turns them — rather than cut
/// to `b…`, `co…`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CategoryNames {
    /// Whether the names are turned.
    pub turned: bool,
    /// How wide a name may be before it is shortened ([`shorten`]).
    pub room: f64,
    /// How far below the plot the names reach, their gap from it included.
    pub depth: f64,
    /// Every how many'th name is drawn: turned names closer than a line apart would overlap,
    /// so some are left out, as LibreOffice leaves them out.
    pub every: usize,
}

/// The angle, counter-clockwise, a turned category name is set at.
pub const TURN_DEGREES: f64 = 45.0;
/// The most of a chart's height turned names may take.
const TURN_SHARE: f64 = 0.3;

/// How the names under a plot are set: `widest` the widest name, `band` one category's width,
/// `line` a line's height, `height` the chart's frame, `gap` the space between plot and names,
/// and `clearance` the least space between two level names.
pub fn category_names(
    widest: f64,
    band: f64,
    line: f64,
    height: f64,
    gap: f64,
    clearance: f64,
) -> CategoryNames {
    let level = (band - clearance).max(0.0);
    if widest <= level || band <= 0.0 {
        return CategoryNames {
            turned: false,
            room: level,
            depth: line + gap,
            every: 1,
        };
    }
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let room = widest.min((height * TURN_SHARE / s - line).max(0.0));
    CategoryNames {
        turned: true,
        room,
        depth: s * (room + line) + gap,
        every: (line / (band * s)).ceil().max(1.0) as usize,
    }
}

/// Where a name `w` wide and `h` tall goes, turned [`TURN_DEGREES`] about its box's top-left
/// corner, so that it ends under `tick` and its top corner is `gap` below `bottom`: the top-left
/// of its box before turning.
pub fn turned_origin(tick: f64, bottom: f64, gap: f64, w: f64, h: f64) -> (f64, f64) {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let anchor = (tick, bottom + gap + s * h / 2.0);
    (anchor.0 - s * (w + h / 2.0), anchor.1 + s * (w - h / 2.0))
}

/// How far the plot is inset from the frame.
const INSET: f64 = 8.0;
/// The share of a bar group's band left empty between groups.
const GROUP_GAP: f64 = 0.2;
/// The gap between a tick label and the plot, and between an axis title and its tick labels.
const TICK_GAP: f64 = 4.0;
/// A legend swatch's side, the gap to its words, between two entries in a row, and between the
/// legend and the plot.
const SWATCH: f64 = 10.0;
const SWATCH_GAP: f64 = 6.0;
const ENTRY_GAP: f64 = 14.0;
const LEGEND_GAP: f64 = 10.0;
/// A line series' stroke, in points.
const LINE_W: f64 = 2.0;
/// How many straight segments a whole turn of a pie's rim is drawn with.
const SEGMENTS_PER_TURN: f64 = 120.0;

/// Tick labels, the legend and the axis titles: the grid's face, a little smaller.
fn small() -> TextStyle {
    TextStyle {
        font_size: Some("9pt".into()),
        ..TextStyle::default()
    }
}

/// The chart's own title: the same face, larger and bold.
fn title_style() -> TextStyle {
    TextStyle {
        font_size: Some("12pt".into()),
        font_weight: Some("bold".into()),
        ..TextStyle::default()
    }
}

/// What draws one chart: its frame, its colours and the measure of its text.
struct Painter<'a> {
    chart: &'a Chart,
    data: &'a ChartData,
    palette: &'a Colours,
    metrics: &'a dyn Metrics,
    clip: Rect,
    ops: Vec<Mark>,
}

/// One chart in `frame`: a card on the page, then its title and legend, then the plot in what is
/// left — gridlines, marks, tick labels and axis titles, in that order.
pub fn draw(
    chart: &Chart,
    data: &ChartData,
    frame: Rect,
    palette: &Colours,
    metrics: &dyn Metrics,
) -> Vec<Mark> {
    let mut painter = Painter {
        chart,
        data,
        palette,
        metrics,
        clip: frame,
        ops: Vec::new(),
    };
    painter.card(frame);
    let inner = painter.dress(frame);
    painter.plot(inner);
    painter.ops
}

impl Painter<'_> {
    fn text_w(&self, text: &str, style: &TextStyle) -> f64 {
        width(self.metrics, text, style)
    }

    fn line_h(&self, style: &TextStyle) -> f64 {
        f64::from(self.metrics.line_height(style))
    }

    fn text(&mut self, x: f64, top: f64, text: &str, style: TextStyle, color: Rgb) {
        self.turned(x, top, text, style, color, 0.0);
    }

    fn turned(&mut self, x: f64, top: f64, text: &str, style: TextStyle, color: Rgb, turn: f64) {
        self.ops.push(Mark::Text {
            x,
            top,
            text: text.to_owned(),
            style,
            color,
            clip: self.clip,
            turn,
        });
    }

    fn fill(&mut self, rect: Rect, color: Rgb) {
        self.ops.push(Mark::Fill { rect, color });
    }

    /// A mark's colour: the document's ([`effective_color`]) made legible on a dark page, or the
    /// accent when it names nothing readable.
    fn mark(&self, series: usize, point: Option<usize>) -> Rgb {
        let own = color::parse(&effective_color(self.chart, series, point));
        match own {
            Some(own) if self.palette.dark => {
                color::document_ink(Some(own), None, self.palette.page, own, true)
            }
            Some(own) => own,
            None => self.palette.accent,
        }
    }

    /// The chart's ground and a hairline round it, so it reads as a thing over the cells.
    fn card(&mut self, frame: Rect) {
        let (page, edge) = (self.palette.page, self.palette.grid);
        self.fill(frame, page);
        for side in [
            Rect::new(frame.x, frame.y, frame.w, 1.0),
            Rect::new(frame.x, frame.bottom() - 1.0, frame.w, 1.0),
            Rect::new(frame.x, frame.y, 1.0, frame.h),
            Rect::new(frame.right() - 1.0, frame.y, 1.0, frame.h),
        ] {
            self.fill(side, edge);
        }
    }

    /// The title and the legend take their bands first, and the plot gets what is left. A legend
    /// that would take more than its share of a small chart is left off.
    fn dress(&mut self, frame: Rect) -> Rect {
        let mut rest = frame;
        if let Some(title) = self.chart.title.clone() {
            let style = title_style();
            let h = self.line_h(&style);
            if frame.h > 4.0 * h {
                let w = self.text_w(&title, &style);
                let x = frame.x + ((frame.w - w) / 2.0).max(INSET);
                self.text(x, frame.y + INSET, &title, style, self.palette.ink);
                rest.y += h + INSET;
                rest.h -= h + INSET;
            }
        }
        let Some(position) = self.chart.legend else {
            return rest;
        };
        let items = legend_items(self.chart, self.data);
        if items.is_empty() {
            return rest;
        }
        let style = small();
        let line = self.line_h(&style).max(SWATCH) + 4.0;
        let widths: Vec<f64> = items
            .iter()
            .map(|(label, _)| SWATCH + SWATCH_GAP + self.text_w(label, &style))
            .collect();
        let ink = self.palette.ink;
        let entry = |painter: &mut Self, x: f64, y: f64, label: &str, at: usize| {
            let (series, point) = items[at].1;
            let color = painter.mark(series, point);
            painter.fill(
                Rect::new(x, y + (line - 4.0 - SWATCH) / 2.0, SWATCH, SWATCH),
                color,
            );
            painter.text(x + SWATCH + SWATCH_GAP, y, label, style.clone(), ink);
        };
        match position {
            ChartLegend::Start | ChartLegend::End => {
                let w = widths.iter().copied().fold(0.0, f64::max);
                let h = items.len() as f64 * line;
                if w + LEGEND_GAP > rest.w * 0.45 || h > rest.h {
                    return rest;
                }
                let x = match position {
                    ChartLegend::End => rest.right() - INSET - w,
                    _ => rest.x + INSET,
                };
                let mut y = rest.y + (rest.h - h) / 2.0;
                for (at, (label, _)) in items.iter().enumerate() {
                    entry(self, x, y, label, at);
                    y += line;
                }
                if position == ChartLegend::Start {
                    rest.x += w + LEGEND_GAP;
                }
                rest.w -= w + LEGEND_GAP;
            }
            ChartLegend::Top | ChartLegend::Bottom => {
                let total = widths.iter().sum::<f64>() + ENTRY_GAP * (items.len() - 1) as f64;
                // One row, centred; a legend too wide for it is left off rather than wrapped.
                if total > rest.w - 2.0 * INSET || line + LEGEND_GAP > rest.h * 0.35 {
                    return rest;
                }
                let y = match position {
                    ChartLegend::Top => rest.y + INSET,
                    _ => rest.bottom() - INSET - line,
                };
                let mut x = rest.x + (rest.w - total) / 2.0;
                for (at, ((label, _), w)) in items.iter().zip(&widths).enumerate() {
                    entry(self, x, y, label, at);
                    x += w + ENTRY_GAP;
                }
                if position == ChartLegend::Top {
                    rest.y += line + LEGEND_GAP;
                }
                rest.h -= line + LEGEND_GAP;
            }
        }
        rest
    }

    /// The plot inside `frame`, once the axes have taken their room, and everything in it.
    fn plot(&mut self, frame: Rect) {
        let ticks = axis_ticks(max_value(self.data));
        let (x_axis, y_axis) = axes(self.chart, self.data);
        let style = small();
        let tick_h = self.line_h(&style);
        let widest = ticks
            .values
            .iter()
            .map(|value| self.text_w(&self.data.tick_label(&ticks, *value), &style))
            .fold(0.0, f64::max);

        let mut left = INSET;
        let mut top = INSET;
        if y_axis.label.is_some() {
            // ponytail: the value axis' title sits above the axis, level, rather than turned along
            // it — `Mark::Text` is level only.
            top += tick_h + TICK_GAP;
        }
        if y_axis.tick_labels {
            left += widest + TICK_GAP;
            top += tick_h / 2.0;
        }
        let mut bottom = INSET;
        if x_axis.label.is_some() {
            bottom += tick_h + TICK_GAP;
        }
        let band = (frame.w - left - INSET).max(0.0) / category_count(self.data).max(1) as f64;
        let widest_name = self
            .data
            .categories
            .iter()
            .map(|name| self.text_w(name, &style))
            .fold(0.0, f64::max);
        let names = category_names(widest_name, band, tick_h, frame.h, TICK_GAP, TICK_GAP);
        if x_axis.tick_labels {
            bottom += names.depth;
        }
        let plot = Rect::new(
            frame.x + left,
            frame.y + top,
            (frame.w - left - INSET).max(0.0),
            (frame.h - top - bottom).max(0.0),
        );
        if plot.w <= 0.0 || plot.h <= 0.0 {
            return;
        }
        if self.data.kind != ChartKind::Pie {
            self.gridlines(&x_axis, &y_axis, &ticks, plot);
        }
        match self.data.kind {
            ChartKind::Bar => self.bars(&ticks, plot),
            ChartKind::Line => self.lines(&ticks, plot),
            ChartKind::Pie => self.pie(plot),
        }
        let ink = self.palette.header_ink;
        if y_axis.tick_labels {
            for value in &ticks.values {
                let label = self.data.tick_label(&ticks, *value);
                let x = plot.x - TICK_GAP - self.text_w(&label, &style);
                let y = value_y(*value, &ticks, plot) - tick_h / 2.0;
                self.text(x, y, &label, style.clone(), ink);
            }
        }
        if x_axis.tick_labels {
            for (i, x) in category_x(self.data, plot).into_iter().enumerate() {
                if i % names.every != 0 {
                    continue;
                }
                let Some(label) = self.data.categories.get(i).filter(|t| !t.is_empty()) else {
                    continue;
                };
                let Some(label) = shorten(label, names.room, |t| self.text_w(t, &style)) else {
                    continue;
                };
                let w = self.text_w(&label, &style);
                match names.turned {
                    true => {
                        let (x, top) = turned_origin(x, plot.bottom(), TICK_GAP, w, tick_h);
                        self.turned(x, top, &label, style.clone(), ink, TURN_DEGREES);
                    }
                    false => self.text(
                        x - w / 2.0,
                        plot.bottom() + TICK_GAP,
                        &label,
                        style.clone(),
                        ink,
                    ),
                }
            }
        }
        if let Some(label) = &x_axis.label {
            let w = self.text_w(label, &style);
            let x = plot.x + (plot.w - w) / 2.0;
            self.text(
                x,
                frame.bottom() - INSET - tick_h,
                label,
                style.clone(),
                ink,
            );
        }
        if let Some(label) = &y_axis.label {
            self.text(frame.x + INSET, frame.y + INSET, label, style, ink);
        }
    }

    fn gridlines(&mut self, x_axis: &ChartAxis, y_axis: &ChartAxis, ticks: &Ticks, plot: Rect) {
        let rule = self.palette.grid;
        if y_axis.gridlines {
            for value in &ticks.values {
                let y = value_y(*value, ticks, plot);
                self.fill(Rect::new(plot.x, y - 0.5, plot.w, 1.0), rule);
            }
        }
        if x_axis.gridlines {
            for x in category_x(self.data, plot) {
                self.fill(Rect::new(x - 0.5, plot.y, 1.0, plot.h), rule);
            }
        }
        // The baseline every bar stands on, in the axis' own ink.
        let base = self.palette.header_ink;
        self.fill(Rect::new(plot.x, plot.bottom() - 0.5, plot.w, 1.0), base);
    }

    fn bars(&mut self, ticks: &Ticks, plot: Rect) {
        let count = category_count(self.data);
        if count == 0 {
            return;
        }
        let series = self.data.series.len().max(1);
        let group = plot.w / count as f64;
        let w = (group * (1.0 - GROUP_GAP) / series as f64).max(1.0);
        let gap = (group - w * series as f64) / (series as f64 + 1.0);
        for cat in 0..count {
            for s in 0..self.data.series.len() {
                let Some(value) = self.data.series[s].1.get(cat).copied() else {
                    continue;
                };
                let y = value_y(value, ticks, plot);
                let x = plot.x + cat as f64 * group + gap + s as f64 * (w + gap);
                let color = self.mark(s, Some(cat));
                self.fill(Rect::new(x, y, w, plot.bottom() - y), color);
            }
        }
    }

    fn lines(&mut self, ticks: &Ticks, plot: Rect) {
        let count = category_count(self.data);
        if count < 2 {
            return;
        }
        let step = plot.w / count as f64;
        for s in 0..self.data.series.len() {
            let values = &self.data.series[s].1;
            if values.len() < 2 {
                continue;
            }
            let points: Vec<(f64, f64)> = values
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    (
                        plot.x + (i as f64 + 0.5) * step,
                        value_y(*value, ticks, plot),
                    )
                })
                .collect();
            let color = self.mark(s, None);
            self.ops.push(Mark::Path {
                points,
                fill: None,
                stroke: Some((color, LINE_W)),
            });
        }
    }

    /// Every slice as [`pie_slices`] sweeps it, a fan of segments round its rim, with a line of
    /// page ground between slices so same-coloured neighbours part.
    fn pie(&mut self, plot: Rect) {
        let (cx, cy) = (plot.x + plot.w / 2.0, plot.y + plot.h / 2.0);
        let r = (plot.w.min(plot.h) / 2.0).max(1.0);
        for slice in pie_slices(self.data, self.chart.clockwise) {
            let steps = ((slice.sweep.abs() / std::f64::consts::TAU) * SEGMENTS_PER_TURN)
                .ceil()
                .max(1.0) as usize;
            let mut points = vec![(cx, cy)];
            points.extend((0..=steps).map(|i| {
                let angle = slice.start + slice.sweep * i as f64 / steps as f64;
                (cx + r * angle.cos(), cy + r * angle.sin())
            }));
            let color = self.mark(0, Some(slice.point));
            self.ops.push(Mark::Path {
                points,
                fill: Some(color),
                stroke: Some((self.palette.page, 1.0)),
            });
        }
    }
}

/// What a legend names and whose colour it shows: one series each for a bar or line, one slice
/// each for a pie.
fn legend_items(chart: &Chart, data: &ChartData) -> Vec<(String, (usize, Option<usize>))> {
    match data.kind {
        ChartKind::Pie => pie_slices(data, chart.clockwise)
            .into_iter()
            .map(|slice| {
                let name = data
                    .categories
                    .get(slice.point)
                    .filter(|name| !name.is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("{}", slice.point + 1));
                (name, (0, Some(slice.point)))
            })
            .collect(),
        ChartKind::Bar | ChartKind::Line => data
            .series
            .iter()
            .enumerate()
            .map(|(i, (name, _))| {
                let name = match name.is_empty() {
                    true => format!("Series {}", i + 1),
                    false => name.clone(),
                };
                (name, (i, None))
            })
            .collect(),
    }
}

/// A pie has no axes, so axis settings cost its circle nothing.
fn axes(chart: &Chart, data: &ChartData) -> (ChartAxis, ChartAxis) {
    match data.kind {
        ChartKind::Pie => (ChartAxis::bare(), ChartAxis::bare()),
        _ => (chart.x_axis.clone(), chart.y_axis.clone()),
    }
}

fn max_value(data: &ChartData) -> f64 {
    data.series
        .iter()
        .flat_map(|(_, values)| values.iter().copied())
        .fold(0.0_f64, f64::max)
        .max(1.0)
}

fn category_count(data: &ChartData) -> usize {
    data.categories.len().max(
        data.series
            .iter()
            .map(|(_, values)| values.len())
            .max()
            .unwrap_or(0),
    )
}

/// Where a value sits vertically — the one place the scale is applied.
fn value_y(value: f64, ticks: &Ticks, plot: Rect) -> f64 {
    plot.bottom() - (value.max(0.0) / ticks.max()) * plot.h
}

/// Each category's tick, at the middle of its band, so the first name stays off the value axis.
fn category_x(data: &ChartData, plot: Rect) -> Vec<f64> {
    let count = category_count(data);
    if count == 0 {
        return Vec::new();
    }
    let step = plot.w / count as f64;
    (0..count)
        .map(|i| plot.x + (i as f64 + 0.5) * step)
        .collect()
}
