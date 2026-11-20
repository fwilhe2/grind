// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart as a list of [`Mark`]s, so where every bar, point, slice and label goes is decided
//! once and tested on any host, and a shell only draws them. [`Picture`] keeps what was drawn
//! where, so a click and a tooltip ask the same geometry the marks were placed by. Scaled
//! against [`axis_ticks`], pies swept by [`pie_slices`], colours from [`effective_color`], and
//! text measured through the shell's `Metrics`. Coordinates are the caller's unit.

use grind_core::color::{self, Rgb};
use grind_core::layout::Metrics;
use grind_core::style::TextStyle;

use crate::{
    Chart, ChartAxis, ChartData, ChartKind, ChartLegend, Ticks, axis_ticks, effective_color,
    pie_slice_at, pie_slices,
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
    #[must_use]
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }

    #[must_use]
    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    #[must_use]
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    #[must_use]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
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
        style: Box<TextStyle>,
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
#[must_use]
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
#[must_use]
pub fn turned_origin(tick: f64, bottom: f64, gap: f64, w: f64, h: f64) -> (f64, f64) {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let anchor = (tick, bottom + gap + s * h / 2.0);
    (anchor.0 - s * (w + h / 2.0), anchor.1 + s * (w - h / 2.0))
}

/// How far the plot is inset from the frame.
const INSET: f64 = 6.0;
/// The share of a bar group's band left empty between groups.
const GROUP_GAP: f64 = 0.2;
/// The widest a bar is drawn: what is left of its band is air, which is what makes a column of
/// bars read as a chart rather than as a block.
const BAR_MAX: f64 = 24.0;
/// The gap between two bars of one group — the page showing through, not an outline.
const BAR_GAP: f64 = 2.0;
/// How round a bar's data end is; its baseline end stays square.
const BAR_ROUNDING: f64 = 4.0;
/// The band an axis title is set in.
const LABEL_SPACE: f64 = 18.0;
/// The gap between a tick label and the plot.
const TICK_GAP: f64 = 4.0;
/// The least gap between two level category names.
const TICK_CLEARANCE: f64 = 6.0;
/// The gap under the chart's title.
const TITLE_GAP: f64 = 6.0;
/// A legend swatch's side, the gap to its words, between two entries in a row, between two
/// rows, and between the legend and the plot.
const SWATCH: f64 = 10.0;
const SWATCH_GAP: f64 = 6.0;
const ENTRY_GAP: f64 = 14.0;
const ROW_GAP: f64 = 4.0;
const LEGEND_GAP: f64 = 10.0;
/// The most rows a top or bottom legend wraps to before it is left off.
const LEGEND_ROWS: usize = 3;
/// A line series' stroke.
const LINE_W: f64 = 2.0;
/// How close a point has to be to a line to be on it: a line has no area of its own.
const LINE_HIT: f64 = 4.0;
/// How many straight segments a whole turn of a pie's rim is drawn with.
const SEGMENTS_PER_TURN: f64 = 120.0;
/// How many straight segments a rounded corner is drawn with.
const CORNER_SEGMENTS: usize = 4;

/// Every text on a chart: the shell's own face, its title bold.
fn plain() -> TextStyle {
    TextStyle::default()
}

fn title_style() -> TextStyle {
    TextStyle {
        font_weight: Some("bold".into()),
        ..TextStyle::default()
    }
}

/// What a click or a pointer over a chart can mean: a legend entry, a bar, a line or a slice.
/// `series` and `point` mean what [`effective_color`] takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Legend { series: usize, point: Option<usize> },
    Bar { series: usize, point: usize },
    Line { series: usize },
    Slice { point: usize },
}

impl Target {
    /// The `(series, point)` whose colour this is.
    #[must_use]
    pub fn mark(self) -> (usize, Option<usize>) {
        match self {
            Target::Legend { series, point } => (series, point),
            Target::Bar { series, point } => (series, Some(point)),
            Target::Line { series } => (series, None),
            Target::Slice { point } => (0, Some(point)),
        }
    }
}

#[derive(Clone, Debug)]
enum Region {
    Rect(Rect),
    Line(Vec<(f64, f64)>),
    Pie { cx: f64, cy: f64, r: f64 },
}

/// One chart drawn: its marks, the plot they were scaled into, and where each thing a pointer
/// can mean was put.
#[derive(Clone, Debug, Default)]
pub struct Picture {
    pub marks: Vec<Mark>,
    /// The rectangle the data was scaled into; empty when there was no room.
    pub plot: Option<Rect>,
    hits: Vec<(Region, Target)>,
    /// Each category's x, for a line's tooltip to name the nearest.
    categories: Vec<f64>,
    clockwise: bool,
}

impl Picture {
    /// What is under `(x, y)`: a legend entry before anything in the plot.
    #[must_use]
    pub fn target_at(&self, x: f64, y: f64, data: &ChartData) -> Option<Target> {
        let legend = self.hits.iter().find(|(region, target)| {
            matches!(target, Target::Legend { .. })
                && matches!(region, Region::Rect(r) if r.contains(x, y))
        });
        if let Some((_, target)) = legend {
            return Some(*target);
        }
        self.hits.iter().find_map(|(region, target)| {
            let hit = match region {
                Region::Rect(r) => r.contains(x, y),
                Region::Line(points) => points
                    .windows(2)
                    .any(|pair| distance_to_segment((x, y), pair[0], pair[1]) <= LINE_HIT),
                Region::Pie { cx, cy, r } => {
                    let (dx, dy) = (x - cx, y - cy);
                    (dx * dx + dy * dy).sqrt() <= *r
                        && pie_slice_at(data, self.clockwise, dy.atan2(dx))
                            .is_some_and(|point| *target == Target::Slice { point })
                }
            };
            hit.then_some(*target)
        })
    }

    /// The mark under `(x, y)` in words — `Sales · Feb: 20` for a bar or a line, `Feb: 20 (33%)`
    /// for a slice. A legend entry has none: its words are already on screen.
    #[must_use]
    pub fn describe(&self, x: f64, y: f64, data: &ChartData) -> Option<String> {
        let number = crate::formula::value::format_number;
        let series_name = |s: usize| match data.series.get(s) {
            Some((name, _)) if !name.is_empty() => name.clone(),
            _ => format!("Series {}", s + 1),
        };
        let category = |i: usize| match data.categories.get(i) {
            Some(name) if !name.is_empty() => name.clone(),
            _ => format!("{}", i + 1),
        };
        let at = |series: usize, cat: usize| -> Option<String> {
            let value = *data.series.get(series)?.1.get(cat)?;
            Some(format!(
                "{} · {}: {}",
                series_name(series),
                category(cat),
                number(value)
            ))
        };
        match self.target_at(x, y, data)? {
            Target::Legend { .. } => None,
            Target::Bar { series, point } => at(series, point),
            Target::Line { series } => {
                let (cat, _) = self
                    .categories
                    .iter()
                    .enumerate()
                    .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))?;
                at(series, cat)
            }
            Target::Slice { point } => {
                let values = &data.series.first()?.1;
                let value = *values.get(point)?;
                let total: f64 = values.iter().map(|v| v.max(0.0)).sum();
                Some(format!(
                    "{}: {} ({:.0}%)",
                    category(point),
                    number(value),
                    100.0 * value / total
                ))
            }
        }
    }
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
    picture(chart, data, frame, palette, metrics).marks
}

/// [`draw`], keeping where each bar, line, slice and legend entry went.
pub fn picture(
    chart: &Chart,
    data: &ChartData,
    frame: Rect,
    palette: &Colours,
    metrics: &dyn Metrics,
) -> Picture {
    let mut painter = Painter {
        chart,
        data,
        palette,
        metrics,
        clip: frame,
        out: Picture {
            clockwise: chart.clockwise,
            ..Picture::default()
        },
    };
    if frame.w > 0.0 && frame.h > 0.0 {
        painter.card(frame);
        let inner = painter.dress(frame);
        painter.plot(inner);
    }
    painter.out
}

/// What draws one chart: its frame, its colours and the measure of its text.
struct Painter<'a> {
    chart: &'a Chart,
    data: &'a ChartData,
    palette: &'a Colours,
    metrics: &'a dyn Metrics,
    clip: Rect,
    out: Picture,
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
        self.out.marks.push(Mark::Text {
            x,
            top,
            text: text.to_owned(),
            style: Box::new(style),
            color,
            clip: self.clip,
            turn,
        });
    }

    fn fill(&mut self, rect: Rect, color: Rgb) {
        self.out.marks.push(Mark::Fill { rect, color });
    }

    fn shape(&mut self, points: Vec<(f64, f64)>, color: Rgb) {
        self.out.marks.push(Mark::Path {
            points,
            fill: Some(color),
            stroke: None,
        });
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

    /// The title and the legend take their bands first, and the plot gets what is left.
    fn dress(&mut self, frame: Rect) -> Rect {
        let mut rest = frame;
        if let Some(title) = self.chart.title.clone() {
            let style = title_style();
            let h = self.line_h(&style);
            if frame.h > 4.0 * h {
                let room = (frame.w - 2.0 * INSET).max(0.0);
                if let Some(title) = shorten(&title, room, |t| self.text_w(t, &style)) {
                    let w = self.text_w(&title, &style);
                    let x = frame.x + INSET + (room - w) / 2.0;
                    self.text(x, frame.y + INSET, &title, style, self.palette.ink);
                }
                rest.y += h + TITLE_GAP;
                rest.h -= h + TITLE_GAP;
            }
        }
        match self.chart.legend {
            Some(position) => self.legend(rest, position),
            None => rest,
        }
    }

    /// The legend at its edge of `rect`, and what is left — or no legend at all when it would
    /// take more than its share of a small chart.
    fn legend(&mut self, rect: Rect, position: ChartLegend) -> Rect {
        let items = legend_items(self.chart, self.data);
        if items.is_empty() {
            return rect;
        }
        let style = plain();
        let line = self.line_h(&style).max(SWATCH);
        let sized: Vec<(String, (usize, Option<usize>), f64)> = items
            .into_iter()
            .map(|(label, at)| {
                let w = SWATCH + SWATCH_GAP + self.text_w(&label, &style);
                (label, at, w)
            })
            .collect();
        let mut placed = Vec::new();
        let rest = match position {
            ChartLegend::Start | ChartLegend::End => {
                let width = sized.iter().map(|item| item.2).fold(0.0, f64::max);
                let height = sized.len() as f64 * (line + ROW_GAP) - ROW_GAP;
                if width + LEGEND_GAP > rect.w * 0.45 || height > rect.h - 2.0 * INSET {
                    return rect;
                }
                let x = match position {
                    ChartLegend::End => rect.right() - INSET - width,
                    _ => rect.x + INSET,
                };
                let mut y = rect.y + (rect.h - height) / 2.0;
                for item in sized {
                    placed.push((x, y, item));
                    y += line + ROW_GAP;
                }
                match position {
                    ChartLegend::End => Rect {
                        w: rect.w - width - LEGEND_GAP,
                        ..rect
                    },
                    _ => Rect {
                        x: rect.x + width + LEGEND_GAP,
                        w: rect.w - width - LEGEND_GAP,
                        ..rect
                    },
                }
            }
            ChartLegend::Top | ChartLegend::Bottom => {
                // Entries flow in rows, each centred, wrapping at the frame's own width.
                let room = rect.w - 2.0 * INSET;
                let mut rows: Vec<Vec<_>> = vec![Vec::new()];
                let mut used = 0.0;
                for item in sized {
                    let row = rows.last_mut().expect("never empty");
                    if !row.is_empty() && used + ENTRY_GAP + item.2 > room {
                        rows.push(Vec::new());
                        used = 0.0;
                    }
                    let row = rows.last_mut().expect("never empty");
                    used += if row.is_empty() {
                        item.2
                    } else {
                        ENTRY_GAP + item.2
                    };
                    row.push(item);
                }
                let height = rows.len() as f64 * (line + ROW_GAP) - ROW_GAP;
                if rows.len() > LEGEND_ROWS || height + LEGEND_GAP > rect.h * 0.35 {
                    return rect;
                }
                let mut y = match position {
                    ChartLegend::Top => rect.y + INSET,
                    _ => rect.bottom() - INSET - height,
                };
                for row in rows {
                    let width = row.iter().map(|item| item.2).sum::<f64>()
                        + ENTRY_GAP * row.len().saturating_sub(1) as f64;
                    let mut x = rect.x + (rect.w - width) / 2.0;
                    for item in row {
                        let step = item.2 + ENTRY_GAP;
                        placed.push((x, y, item));
                        x += step;
                    }
                    y += line + ROW_GAP;
                }
                match position {
                    ChartLegend::Top => Rect {
                        y: rect.y + height + LEGEND_GAP,
                        h: rect.h - height - LEGEND_GAP,
                        ..rect
                    },
                    _ => Rect {
                        h: rect.h - height - LEGEND_GAP,
                        ..rect
                    },
                }
            }
        };
        // Text in ink, identity in the key beside it: a light series colour is illegible as text.
        let ink = self.palette.ink;
        let text_h = self.line_h(&style);
        for (x, y, (label, (series, point), w)) in placed {
            let color = self.mark(series, point);
            let swatch = Rect::new(x, y + (line - SWATCH) / 2.0, SWATCH, SWATCH);
            if self.data.kind == ChartKind::Line {
                // A line is keyed by a stroke of itself rather than by a square of its colour.
                self.fill(
                    Rect::new(x, swatch.y + SWATCH / 2.0 - 1.0, SWATCH, 2.0),
                    color,
                );
            } else {
                self.shape(rounded(swatch, 2.0, 2.0), color);
            }
            self.text(
                x + SWATCH + SWATCH_GAP,
                y + (line - text_h) / 2.0,
                &label,
                style.clone(),
                ink,
            );
            self.out.hits.push((
                Region::Rect(Rect::new(x, y, w, line)),
                Target::Legend { series, point },
            ));
        }
        rest
    }

    /// The plot inside `frame`, once the axes have taken their room, and everything in it.
    fn plot(&mut self, frame: Rect) {
        let ticks = axis_ticks(max_value(self.data));
        let (x_axis, y_axis) = axes(self.chart, self.data);
        let style = plain();
        let tick_h = self.line_h(&style);
        let widest = ticks
            .values
            .iter()
            .map(|value| self.text_w(&self.data.tick_label(&ticks, *value), &style))
            .fold(0.0, f64::max);

        let mut left = INSET;
        if y_axis.label.is_some() {
            left += LABEL_SPACE;
            if y_axis.tick_labels {
                left += TICK_GAP;
            }
        }
        if y_axis.tick_labels {
            left += widest + TICK_GAP;
        }
        let mut bottom = INSET;
        if x_axis.label.is_some() {
            bottom += LABEL_SPACE;
        }
        let band = (frame.w - left - INSET).max(0.0) / category_count(self.data).max(1) as f64;
        let widest_name = self
            .data
            .categories
            .iter()
            .map(|name| self.text_w(name, &style))
            .fold(0.0, f64::max);
        let names = category_names(widest_name, band, tick_h, frame.h, TICK_GAP, TICK_CLEARANCE);
        if x_axis.tick_labels {
            bottom += names.depth;
        }
        // The topmost value label is centred on the plot's top, so half of it sits above.
        let top = INSET
            + if y_axis.tick_labels {
                tick_h / 2.0
            } else {
                0.0
            };
        let plot = Rect::new(
            frame.x + left,
            frame.y + top,
            (frame.w - left - INSET).max(0.0),
            (frame.h - top - bottom).max(0.0),
        );
        if plot.w <= 0.0 || plot.h <= 0.0 {
            return;
        }
        self.out.plot = Some(plot);
        self.out.categories = category_x(self.data, plot);
        if self.data.kind != ChartKind::Pie {
            self.gridlines(&x_axis, &y_axis, &ticks, plot);
        }
        match self.data.kind {
            ChartKind::Bar => self.bars(&ticks, plot),
            ChartKind::Line => self.lines(&ticks, plot),
            ChartKind::Pie => self.pie(plot),
        }
        let ink = self.palette.ink;
        if y_axis.tick_labels {
            for value in &ticks.values {
                let label = self.data.tick_label(&ticks, *value);
                let x = plot.x - TICK_GAP - self.text_w(&label, &style);
                let y = value_y(*value, &ticks, plot) - tick_h / 2.0;
                self.text(x, y, &label, style.clone(), ink);
            }
        }
        if x_axis.tick_labels {
            for (i, x) in self.out.categories.clone().into_iter().enumerate() {
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
                if names.turned {
                    let (x, top) = turned_origin(x, plot.bottom(), TICK_GAP, w, tick_h);
                    self.turned(x, top, &label, style.clone(), ink, TURN_DEGREES);
                } else {
                    self.text(
                        x - w / 2.0,
                        plot.bottom() + TICK_GAP,
                        &label,
                        style.clone(),
                        ink,
                    );
                }
            }
        }
        // Each axis title centred in its band: the category axis' under the frame, the value
        // axis' turned along its left edge, reading upwards.
        if let Some(label) = &x_axis.label {
            let w = self.text_w(label, &style);
            let x = frame.x + (frame.w - w) / 2.0;
            let top = frame.bottom() - LABEL_SPACE + (LABEL_SPACE - tick_h) / 2.0;
            self.text(x, top, label, style.clone(), ink);
        }
        if let Some(label) = &y_axis.label {
            let w = self.text_w(label, &style);
            let x = frame.x + (LABEL_SPACE - tick_h) / 2.0;
            let top = frame.y + f64::midpoint(frame.h, w);
            self.turned(x, top, label, style, ink, 90.0);
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
            for x in self.out.categories.clone() {
                self.fill(Rect::new(x - 0.5, plot.y, 1.0, plot.h), rule);
            }
        }
    }

    /// Each bar grows from the baseline, square there and rounded at its data end; a group's bars
    /// are at most [`BAR_MAX`] wide and [`BAR_GAP`] apart, centred in the group's band.
    fn bars(&mut self, ticks: &Ticks, plot: Rect) {
        let count = category_count(self.data);
        let series = self.data.series.len();
        if count == 0 || series == 0 {
            return;
        }
        let group = plot.w / count as f64;
        let gaps = BAR_GAP * (series - 1) as f64;
        let w = ((group * (1.0 - GROUP_GAP) - gaps) / series as f64).clamp(1.0, BAR_MAX);
        let lead = (group - (w * series as f64 + gaps)) / 2.0;
        for cat in 0..count {
            for s in 0..series {
                let Some(value) = self.data.series[s].1.get(cat).copied() else {
                    continue;
                };
                let y = value_y(value, ticks, plot);
                let x = plot.x + cat as f64 * group + lead + s as f64 * (w + BAR_GAP);
                let bar = Rect::new(x, y, w, plot.bottom() - y);
                self.out.hits.push((
                    Region::Rect(bar),
                    Target::Bar {
                        series: s,
                        point: cat,
                    },
                ));
                if bar.h > 0.0 {
                    let color = self.mark(s, Some(cat));
                    self.shape(rounded(bar, BAR_ROUNDING, 0.0), color);
                }
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
            self.out
                .hits
                .push((Region::Line(points.clone()), Target::Line { series: s }));
            self.out.marks.push(Mark::Path {
                points,
                fill: None,
                stroke: Some((color, LINE_W)),
            });
        }
    }

    /// Every slice as [`pie_slices`] sweeps it, a fan of segments round its rim.
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
            self.out.hits.push((
                Region::Pie { cx, cy, r },
                Target::Slice { point: slice.point },
            ));
            self.shape(points, color);
        }
    }
}

/// `rect` as a polygon with its top corners rounded by `top` and its bottom ones by `bottom`,
/// each no more than half the rectangle allows.
fn rounded(rect: Rect, top: f64, bottom: f64) -> Vec<(f64, f64)> {
    let limit = (rect.w / 2.0).min(rect.h / 2.0).max(0.0);
    let (top, bottom) = (top.min(limit), bottom.min(limit));
    let mut points = Vec::new();
    // Clockwise on screen from the top-left corner's arc: each corner a quarter turn about its
    // own centre, or the corner itself when its radius is zero.
    let corners = [
        (rect.x + top, rect.y + top, top, std::f64::consts::PI),
        (
            rect.right() - top,
            rect.y + top,
            top,
            1.5 * std::f64::consts::PI,
        ),
        (rect.right() - bottom, rect.bottom() - bottom, bottom, 0.0),
        (
            rect.x + bottom,
            rect.bottom() - bottom,
            bottom,
            0.5 * std::f64::consts::PI,
        ),
    ];
    for (cx, cy, r, from) in corners {
        if r <= 0.0 {
            points.push((cx, cy));
            continue;
        }
        for i in 0..=CORNER_SEGMENTS {
            let angle = from + std::f64::consts::FRAC_PI_2 * i as f64 / CORNER_SEGMENTS as f64;
            points.push((cx + r * angle.cos(), cy + r * angle.sin()));
        }
    }
    points
}

fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len_sq = dx * dx + dy * dy;
    let t = if len_sq <= f64::EPSILON {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len_sq).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// What a legend names and whose colour it shows: one series each for a bar or line, one slice
/// each for a pie. A series with no name of its own is named by its position.
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
                let name = if name.is_empty() {
                    format!("Series {}", i + 1)
                } else {
                    name.clone()
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

/// The longer of the categories and any series' values: a chart tolerates the two disagreeing.
fn category_count(data: &ChartData) -> usize {
    data.categories.len().max(
        data.series
            .iter()
            .map(|(_, values)| values.len())
            .max()
            .unwrap_or(0),
    )
}

/// Where a value sits vertically — the one place the scale is applied. Negative values are
/// floored to zero.
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
