// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Drawing a chart's plot — bar, line or pie — from [`grind_sheet::ChartData`], at the
//! rectangle its own `draw:frame` occupies in widget space (`crate::geom::GridGeom::chart_rect`).
//!
//! A bar is a plain rectangle, the same `append_color` every other shape in [`crate::grid`]
//! draws with; a line or a pie slice needs an actual path, which is `gsk::PathBuilder` — GTK's
//! own vector drawing, already reachable through this shell's `v4_14` feature and one more
//! thing that means cairo is never pulled in for a chart this simple. No chart-level title and
//! no legend: `doc/chart-format.md`'s own scope line, applied to the picture rather than the
//! file it comes from. What an *axis* carries is in scope and is drawn — its title, its tick
//! labels and its gridlines ([`grind_sheet::ChartAxis`]), each of which is a different element
//! from the chart-level title and legend that stay out.
//!
//! Negative values are floored to zero rather than drawn the wrong way from a baseline that
//! would need its own zero line — every document this was built against (`ltwbw2026.*`, an
//! election's vote counts) is non-negative, and a chart of signed data is future work rather
//! than a regression here.
//!
//! [`mark_at`] shares the exact geometry [`draw`] paints from, so a click and a picture can
//! never disagree about which bar, slice or line a point belongs to — `grid.rs`'s
//! `chart_hit` is the one caller. Because a tick label's own *width* moves the plot, both of
//! them take the same [`Measure`]: this shell's one contribution to a chart's layout is how
//! wide a piece of text is, exactly as it is for a page of text (`doc/text-layout.md`).

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::{gdk, graphene, gsk};

use grind_sheet::{Chart, ChartAxis, ChartData, ChartKind, Ticks, axis_ticks};

use crate::geom::Rect;

/// How far the plot is inset from the frame's own border, so a bar or a pie never touches
/// the line drawn around the chart.
const INSET: f64 = 6.0;

/// The gap between one category's group of bars and the next, as a fraction of the group's
/// own width.
const GROUP_GAP: f64 = 0.2;

/// Room reserved for an axis title, in widget pixels — one line of text, no font
/// customisation, the same minimalism `doc/chart-format.md` states for everything else this
/// drawer does.
const LABEL_SPACE: f64 = 18.0;

/// The gap between a tick label and the plot it is labelling.
const TICK_GAP: f64 = 4.0;

/// The gap between two x tick labels below which the second one is dropped rather than drawn
/// over the first — an axis of twenty categories in a chart four centimetres wide labels the
/// ones that fit and leaves the rest unlabelled, which reads as a scale where overlapping text
/// reads as a smudge.
const TICK_CLEARANCE: f64 = 6.0;

/// How close a point has to land to a line's own path, in widget pixels, to count as a hit —
/// a line has no area of its own, unlike a bar or a slice.
const LINE_HIT_DISTANCE: f64 = 4.0;

fn bounds(r: Rect) -> graphene::Rect {
    graphene::Rect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
}

/// A mark's colour, resolved by whoever is drawing or hit-testing — `series` and `point`
/// (`None` for a line, which has no per-point colour) mean exactly what
/// [`grind_sheet::chart::effective_color`] takes, so a caller can hand that straight in.
pub type MarkColor<'a> = dyn Fn(usize, Option<usize>) -> gdk::RGBA + 'a;

/// How wide and how tall a piece of text is, in widget pixels. The one thing a chart's layout
/// needs from the toolkit — a y tick label's own width is what decides where the plot starts,
/// so [`draw`] and [`mark_at`] have to measure identically or a click lands on the wrong bar.
pub type Measure<'a> = dyn Fn(&str) -> (f64, f64) + 'a;

/// A [`Measure`] backed by Pango, in the widget's own font. One `pango::Layout`, reused for
/// every string — building one per tick label would be the expensive way to the same answer.
pub fn measurer<W: IsA<gtk::Widget>>(widget: &W) -> impl Fn(&str) -> (f64, f64) + use<W> {
    let layout = widget.create_pango_layout(None);
    move |text: &str| {
        layout.set_text(text);
        let (w, h) = layout.pixel_size();
        (w as f64, h as f64)
    }
}

/// Everything [`draw`] paints a chart in that is not the data — the theme's colours, and the
/// resolved colour of a mark. Gathered into one value because a chart takes four of them and
/// an argument list that long is where a caller swaps two by accident.
pub struct Painter<'a> {
    pub background: gdk::RGBA,
    pub border: gdk::RGBA,
    pub foreground: gdk::RGBA,
    /// Gridlines and tick marks — the theme's own line colour, not the border's, so a
    /// gridline reads as behind the data rather than as part of the frame.
    pub grid: gdk::RGBA,
    pub color: &'a MarkColor<'a>,
}

/// Draw one chart at `rect`, in widget space. `paint.color` resolves a mark's colour —
/// `grind_sheet::chart::effective_color` converted to a swatch is what a caller fills it from,
/// so what is drawn here is the same colour the writer assigns on save.
pub fn draw(
    widget: &impl IsA<gtk::Widget>,
    snapshot: &gtk::Snapshot,
    rect: Rect,
    chart: &Chart,
    data: &ChartData,
    paint: &Painter,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    snapshot.append_color(&paint.background, &bounds(rect));

    let measure = measurer(widget);
    let layout = layout(rect, chart, data, &measure);
    let plot = layout.plot;
    if plot.w > 0.0 && plot.h > 0.0 {
        // Under the marks: a gridline a bar covers is a gridline behind the data, which is
        // the only place one belongs.
        draw_gridlines(snapshot, &layout, chart, data, paint.grid);
        match data.kind {
            ChartKind::Bar => draw_bar(snapshot, &layout, data, paint.color),
            ChartKind::Line => draw_line(snapshot, &layout, data, paint.color),
            ChartKind::Pie => draw_pie(snapshot, plot, data, paint.color),
        }
        draw_ticks(widget, snapshot, &layout, chart, data, &measure, paint);
    }

    if let Some(text) = &chart.x_axis.label {
        draw_label(
            widget,
            snapshot,
            text,
            rect.x,
            rect.y + rect.h - LABEL_SPACE,
            rect.w,
            paint.foreground,
            0.0,
        );
    }
    if let Some(text) = &chart.y_axis.label {
        draw_label(
            widget,
            snapshot,
            text,
            rect.x,
            rect.y,
            rect.h,
            paint.foreground,
            -90.0,
        );
    }

    let outline = gsk::PathBuilder::new();
    outline.add_rect(&bounds(rect));
    let stroke = gsk::Stroke::builder(1.0).build();
    snapshot.append_stroke(&outline.to_path(), &stroke, &paint.border);
}

/// A chart's frame, divided up: where the plot itself ends up once the axes have taken what
/// they need, and the scale the values are drawn against. Computed once and shared by
/// everything that draws or hit-tests, which is what keeps a click and the picture in step.
struct Layout {
    plot: Rect,
    /// The value axis' own scale — [`grind_sheet::axis_ticks`], so the top of the plot is a
    /// round number rather than whatever the largest bar happened to be.
    ticks: Ticks,
}

/// Which axes actually apply: a pie has neither, so it keeps the whole frame regardless of
/// what its axes carry (a pie chart in a file may well carry an axis title from whatever
/// wrote it — drawing one beside a circle would be inventing a meaning for it).
fn axes(chart: &Chart, data: &ChartData) -> (ChartAxis, ChartAxis) {
    match data.kind {
        ChartKind::Pie => (ChartAxis::bare(), ChartAxis::bare()),
        _ => (chart.x_axis.clone(), chart.y_axis.clone()),
    }
}

/// The plot area within `rect` and the scale it is drawn against — inset from the frame's own
/// border, and further inset by whatever the axes need: a title's fixed [`LABEL_SPACE`], the
/// widest y tick label, one line of x tick text.
fn layout(rect: Rect, chart: &Chart, data: &ChartData, measure: &Measure) -> Layout {
    let (x_axis, y_axis) = axes(chart, data);
    let ticks = axis_ticks(max_value(data));

    let (tick_w, tick_h) = ticks
        .values
        .iter()
        .map(|value| measure(&ticks.label(*value)))
        .fold((0.0_f64, 0.0_f64), |(w, h), (tw, th)| {
            (w.max(tw), h.max(th))
        });

    let mut left = INSET;
    if y_axis.label.is_some() {
        left += LABEL_SPACE;
        // The rotated title fills its whole band, so the tick labels beside it need a gap of
        // their own or the two touch. The x axis needs no equivalent: its title sits *under*
        // its tick labels, where the line spacing already separates them.
        if y_axis.tick_labels {
            left += TICK_GAP;
        }
    }
    if y_axis.tick_labels {
        left += tick_w + TICK_GAP;
    }
    let mut bottom = INSET;
    if x_axis.label.is_some() {
        bottom += LABEL_SPACE;
    }
    if x_axis.tick_labels {
        bottom += tick_h + TICK_GAP;
    }
    // The topmost y tick label is centred on the top of the plot, so half of it sits above —
    // room for that, or it is clipped by the frame.
    let top = INSET
        + match y_axis.tick_labels {
            true => tick_h / 2.0,
            false => 0.0,
        };
    Layout {
        plot: Rect {
            x: rect.x + left,
            y: rect.y + top,
            w: (rect.w - left - INSET).max(0.0),
            h: (rect.h - bottom - top).max(0.0),
        },
        ticks,
    }
}

/// Where each category's own tick sits along the x axis, in widget space — the centre of a
/// bar's group, or a line's own point. What both the tick labels and the x gridlines are
/// placed at, so the two always line up.
fn category_ticks(layout: &Layout, data: &ChartData) -> Vec<f64> {
    let plot = layout.plot;
    let categories = category_count(data);
    if categories == 0 {
        return Vec::new();
    }
    match data.kind {
        // A line's first and last points sit *on* the plot's edges (`line_points`).
        ChartKind::Line if categories > 1 => {
            let step = plot.w / (categories - 1) as f64;
            (0..categories).map(|i| plot.x + i as f64 * step).collect()
        }
        _ => {
            let step = plot.w / categories as f64;
            (0..categories)
                .map(|i| plot.x + (i as f64 + 0.5) * step)
                .collect()
        }
    }
}

/// Gridlines, ruled before anything else is drawn. The y axis' run across at each value tick,
/// the x axis' run up at each category — both in the theme's line colour, hairline width.
fn draw_gridlines(
    snapshot: &gtk::Snapshot,
    layout: &Layout,
    chart: &Chart,
    data: &ChartData,
    color: gdk::RGBA,
) {
    let (x_axis, y_axis) = axes(chart, data);
    let plot = layout.plot;
    let path = gsk::PathBuilder::new();
    let mut any = false;
    if y_axis.gridlines {
        for value in &layout.ticks.values {
            let y = value_y(layout, *value);
            path.move_to(plot.x as f32, y as f32);
            path.line_to((plot.x + plot.w) as f32, y as f32);
            any = true;
        }
    }
    if x_axis.gridlines {
        for x in category_ticks(layout, data) {
            path.move_to(x as f32, plot.y as f32);
            path.line_to(x as f32, (plot.y + plot.h) as f32);
            any = true;
        }
    }
    if any {
        let stroke = gsk::Stroke::builder(1.0).build();
        snapshot.append_stroke(&path.to_path(), &stroke, &color);
    }
}

/// The tick labels themselves: the category names under the x axis, the value scale beside
/// the y one. An x label that would collide with the one before it is dropped
/// ([`TICK_CLEARANCE`]) rather than drawn over it.
fn draw_ticks(
    widget: &impl IsA<gtk::Widget>,
    snapshot: &gtk::Snapshot,
    layout: &Layout,
    chart: &Chart,
    data: &ChartData,
    measure: &Measure,
    paint: &Painter,
) {
    let (x_axis, y_axis) = axes(chart, data);
    let plot = layout.plot;
    if y_axis.tick_labels {
        for value in &layout.ticks.values {
            let text = layout.ticks.label(*value);
            let (w, h) = measure(&text);
            // Right-aligned against the plot's own left edge, centred on the tick.
            place(
                widget,
                snapshot,
                &text,
                plot.x - TICK_GAP - w,
                value_y(layout, *value) - h / 2.0,
                paint.foreground,
            );
        }
    }
    if x_axis.tick_labels {
        let mut drawn_to = f64::NEG_INFINITY;
        for (i, x) in category_ticks(layout, data).into_iter().enumerate() {
            let Some(text) = data.categories.get(i) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let (w, _) = measure(text);
            let left = x - w / 2.0;
            if left < drawn_to + TICK_CLEARANCE {
                continue;
            }
            drawn_to = left + w;
            place(
                widget,
                snapshot,
                text,
                left,
                plot.y + plot.h + TICK_GAP,
                paint.foreground,
            );
        }
    }
}

/// Where a value sits vertically within the plot — the one place the value scale is applied,
/// so a gridline, a tick label and a bar's own top all agree.
fn value_y(layout: &Layout, value: f64) -> f64 {
    let plot = layout.plot;
    plot.y + plot.h - (value.max(0.0) / layout.ticks.max()) * plot.h
}

/// One string at a point, in the widget's own font — the unrotated, uncentred sibling of
/// [`draw_label`], which is what a tick label needs.
fn place(
    widget: &impl IsA<gtk::Widget>,
    snapshot: &gtk::Snapshot,
    text: &str,
    x: f64,
    y: f64,
    color: gdk::RGBA,
) {
    let layout = widget.create_pango_layout(Some(text));
    snapshot.save();
    snapshot.translate(&graphene::Point::new(x as f32, y as f32));
    snapshot.append_layout(&layout, &color);
    snapshot.restore();
}

/// One centred, single-line label — `angle` in degrees, `0.0` for the x axis (horizontal,
/// centred under `along`'s own width) and `-90.0` for the y axis (rotated, centred along the
/// frame's own height). No font or size a document controls: this build has no font
/// (`doc/not-doing.md`), the same reason a cell's own text is drawn in the widget's font.
#[allow(clippy::too_many_arguments)]
fn draw_label(
    widget: &impl IsA<gtk::Widget>,
    snapshot: &gtk::Snapshot,
    text: &str,
    x: f64,
    y: f64,
    along: f64,
    color: gdk::RGBA,
    angle: f64,
) {
    let layout = widget.create_pango_layout(Some(text));
    let (w, h) = layout.pixel_size();
    let (w, h) = (w as f64, h as f64);

    snapshot.save();
    if angle == 0.0 {
        snapshot.translate(&graphene::Point::new(
            (x + (along - w) / 2.0) as f32,
            (y + (LABEL_SPACE - h) / 2.0) as f32,
        ));
    } else {
        snapshot.translate(&graphene::Point::new(
            (x + (LABEL_SPACE + h) / 2.0) as f32,
            (y + (along + w) / 2.0) as f32,
        ));
        snapshot.rotate(angle as f32);
    }
    snapshot.append_layout(&layout, &color);
    snapshot.restore();
}

/// How many categories a chart's data actually has — the longer of the categories list and
/// any series' own values, since a chart tolerates the two disagreeing rather than refusing
/// to draw (`ChartData::read`'s own tolerance, carried into the picture).
fn category_count(data: &ChartData) -> usize {
    data.categories.len().max(
        data.series
            .iter()
            .map(|(_, values)| values.len())
            .max()
            .unwrap_or(0),
    )
}

/// The largest value plotted. Not the top of the axis — [`grind_sheet::axis_ticks`] rounds
/// this up to a tick, and [`Layout::ticks`] is what everything is actually scaled against.
fn max_value(data: &ChartData) -> f64 {
    data.series
        .iter()
        .flat_map(|(_, values)| values.iter().copied())
        .fold(0.0_f64, f64::max)
        .max(1.0)
}

/// One bar's own rectangle, in widget space — the geometry [`draw_bar`] paints and
/// [`bar_hit`] tests against, so the two can never disagree.
struct BarLayout {
    categories: usize,
    series_count: usize,
    group_w: f64,
    bar_w: f64,
    gap: f64,
}

fn bar_layout(plot: Rect, data: &ChartData) -> Option<BarLayout> {
    let categories = category_count(data);
    if categories == 0 {
        return None;
    }
    let series_count = data.series.len().max(1);
    let group_w = plot.w / categories as f64;
    let bar_w = (group_w * (1.0 - GROUP_GAP) / series_count as f64).max(1.0);
    let gap = (group_w - bar_w * series_count as f64) / (series_count as f64 + 1.0);
    Some(BarLayout {
        categories,
        series_count,
        group_w,
        bar_w,
        gap,
    })
}

fn bar_rect(
    layout: &Layout,
    bars: &BarLayout,
    data: &ChartData,
    series: usize,
    cat: usize,
) -> Option<Rect> {
    let plot = layout.plot;
    let value = *data.series.get(series)?.1.get(cat)?;
    let y = value_y(layout, value);
    let x = plot.x + cat as f64 * bars.group_w + bars.gap + series as f64 * (bars.bar_w + bars.gap);
    Some(Rect {
        x,
        y,
        w: bars.bar_w,
        h: plot.y + plot.h - y,
    })
}

fn draw_bar(snapshot: &gtk::Snapshot, layout: &Layout, data: &ChartData, color: &MarkColor) {
    let Some(bars) = bar_layout(layout.plot, data) else {
        return;
    };
    for cat in 0..bars.categories {
        for s in 0..bars.series_count.min(data.series.len()) {
            let Some(rect) = bar_rect(layout, &bars, data, s, cat) else {
                continue;
            };
            snapshot.append_color(&color(s, Some(cat)), &bounds(rect));
        }
    }
}

/// Which bar, if any, `(x, y)` lands in.
fn bar_hit(layout: &Layout, data: &ChartData, x: f64, y: f64) -> Option<(usize, usize)> {
    let bars = bar_layout(layout.plot, data)?;
    for cat in 0..bars.categories {
        for s in 0..bars.series_count.min(data.series.len()) {
            if let Some(rect) = bar_rect(layout, &bars, data, s, cat)
                && rect.contains(x, y)
            {
                return Some((s, cat));
            }
        }
    }
    None
}

/// One line series' own points, in widget space — shared the same way [`BarLayout`] is.
fn line_points(layout: &Layout, data: &ChartData, series: usize) -> Option<Vec<(f64, f64)>> {
    let plot = layout.plot;
    let categories = category_count(data);
    if categories < 2 {
        return None;
    }
    let step = plot.w / (categories - 1) as f64;
    let (_, values) = data.series.get(series)?;
    if values.len() < 2 {
        return None;
    }
    Some(
        values
            .iter()
            .enumerate()
            .map(|(i, &value)| (plot.x + i as f64 * step, value_y(layout, value)))
            .collect(),
    )
}

fn draw_line(snapshot: &gtk::Snapshot, layout: &Layout, data: &ChartData, color: &MarkColor) {
    for s in 0..data.series.len() {
        let Some(points) = line_points(layout, data, s) else {
            continue;
        };
        let path = gsk::PathBuilder::new();
        for (i, &(x, y)) in points.iter().enumerate() {
            match i {
                0 => path.move_to(x as f32, y as f32),
                _ => path.line_to(x as f32, y as f32),
            }
        }
        let stroke = gsk::Stroke::builder(2.0).build();
        snapshot.append_stroke(&path.to_path(), &stroke, &color(s, None));
    }
}

/// Which line, if any, passes within [`LINE_HIT_DISTANCE`] of `(x, y)`.
fn line_hit(layout: &Layout, data: &ChartData, x: f64, y: f64) -> Option<usize> {
    for s in 0..data.series.len() {
        let Some(points) = line_points(layout, data, s) else {
            continue;
        };
        for pair in points.windows(2) {
            if distance_to_segment((x, y), pair[0], pair[1]) <= LINE_HIT_DISTANCE {
                return Some(s);
            }
        }
    }
    None
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

/// One sample every this many degrees of arc — a slice is a straight-edged polygon rather
/// than a true arc (`gsk::PathBuilder` has no circular arc primitive; `conic_to` is a
/// rational Bezier and a fan of short segments is the simpler way to the same picture),
/// fine enough that the seam between segments is not visible.
const PIE_SAMPLES_PER_TURN: f64 = 96.0;

/// One pie slice's own start and sweep angle, radians from 12 o'clock — shared by
/// [`draw_pie`] and [`pie_hit`].
fn pie_slices(data: &ChartData) -> Option<Vec<(usize, f64, f64)>> {
    let (_, values) = data.series.first()?;
    let total: f64 = values.iter().map(|v| v.max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let mut angle = -std::f64::consts::FRAC_PI_2;
    let mut slices = Vec::new();
    for (i, &value) in values.iter().enumerate() {
        let value = value.max(0.0);
        if value <= 0.0 {
            continue;
        }
        let sweep = (value / total) * std::f64::consts::TAU;
        slices.push((i, angle, sweep));
        angle += sweep;
    }
    Some(slices)
}

fn draw_pie(snapshot: &gtk::Snapshot, plot: Rect, data: &ChartData, color: &MarkColor) {
    let Some(slices) = pie_slices(data) else {
        return;
    };
    let cx = plot.x + plot.w / 2.0;
    let cy = plot.y + plot.h / 2.0;
    let radius = (plot.w.min(plot.h) / 2.0).max(1.0);
    for (i, angle, sweep) in slices {
        let steps =
            ((sweep / (std::f64::consts::TAU / PIE_SAMPLES_PER_TURN)).ceil() as usize).max(1);
        let path = gsk::PathBuilder::new();
        path.move_to(cx as f32, cy as f32);
        for step in 0..=steps {
            let a = angle + sweep * (step as f64 / steps as f64);
            path.line_to(
                (cx + radius * a.cos()) as f32,
                (cy + radius * a.sin()) as f32,
            );
        }
        path.close();
        snapshot.append_fill(&path.to_path(), gsk::FillRule::Winding, &color(0, Some(i)));
    }
}

/// Which pie slice, if any, `(x, y)` lands in.
fn pie_hit(plot: Rect, data: &ChartData, x: f64, y: f64) -> Option<usize> {
    let slices = pie_slices(data)?;
    let cx = plot.x + plot.w / 2.0;
    let cy = plot.y + plot.h / 2.0;
    let radius = (plot.w.min(plot.h) / 2.0).max(1.0);
    let (dx, dy) = (x - cx, y - cy);
    if (dx * dx + dy * dy).sqrt() > radius {
        return None;
    }
    let mut a = dy.atan2(dx) + std::f64::consts::FRAC_PI_2;
    if a < 0.0 {
        a += std::f64::consts::TAU;
    }
    for (i, angle, sweep) in slices {
        let mut start = angle + std::f64::consts::FRAC_PI_2;
        if start < 0.0 {
            start += std::f64::consts::TAU;
        }
        let end = start + sweep;
        if a >= start && a < end || (end > std::f64::consts::TAU && a < end - std::f64::consts::TAU)
        {
            return Some(i);
        }
    }
    None
}

/// Which mark, if any, a point in widget space hits — `(series, point)`, `point` always
/// `Some` for `Bar`/`Pie` (`Pie`'s own `series` is always `0`) and always `None` for `Line`.
/// `rect` is the chart's own frame, exactly what [`draw`] was given, and `measure` must be the
/// same one: a y tick label's width moves the plot, so a caller measuring differently would
/// hit-test against a plot the user is not looking at.
pub fn mark_at(
    rect: Rect,
    chart: &Chart,
    data: &ChartData,
    x: f64,
    y: f64,
    measure: &Measure,
) -> Option<(usize, Option<usize>)> {
    let layout = layout(rect, chart, data, measure);
    match data.kind {
        ChartKind::Bar => bar_hit(&layout, data, x, y).map(|(s, p)| (s, Some(p))),
        ChartKind::Line => line_hit(&layout, data, x, y).map(|s| (s, None)),
        ChartKind::Pie => pie_hit(layout.plot, data, x, y).map(|p| (0, Some(p))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 200.0,
        h: 100.0,
    };

    /// A stand-in for Pango, so every one of these runs with no display: seven pixels a
    /// character, fourteen tall. Nothing here asserts a *pixel*, only that one layout is
    /// bigger or smaller than another, so the numbers only have to be plausible.
    fn measure(text: &str) -> (f64, f64) {
        (text.chars().count() as f64 * 7.0, 14.0)
    }

    /// A chart whose axes carry nothing — the frame is the plot, which is what the hit-test
    /// cases below are positioned against.
    fn bare(kind: ChartKind) -> Chart {
        let mut chart = Chart::new(kind, "0cm".into(), "0cm".into(), "5cm".into(), "3cm".into());
        chart.x_axis = ChartAxis::bare();
        chart.y_axis = ChartAxis::bare();
        chart
    }

    #[test]
    fn a_click_on_one_bar_names_its_series_and_point_not_the_other_bar() {
        let chart = bare(ChartKind::Bar);
        let data = ChartData {
            kind: ChartKind::Bar,
            categories: vec!["a".into(), "b".into()],
            series: vec![("Votes".into(), vec![100.0, 80.0])],
        };
        // The first bar is the left half of the plot, near its bottom (a full-height bar).
        let hit = mark_at(FRAME, &chart, &data, 30.0, 90.0, &measure);
        assert_eq!(hit, Some((0, Some(0))));
        // Well outside the plot area hits nothing.
        assert_eq!(mark_at(FRAME, &chart, &data, 500.0, 500.0, &measure), None);
    }

    #[test]
    fn a_click_on_a_line_names_its_series_and_no_point() {
        let chart = bare(ChartKind::Line);
        let data = ChartData {
            kind: ChartKind::Line,
            categories: vec!["a".into(), "b".into()],
            series: vec![("Votes".into(), vec![100.0, 0.0])],
        };
        // At the left edge, the line sits at the top of the plot (the max value, which is
        // also a tick — `axis_ticks(100)` tops out at exactly 100).
        let plot = layout(FRAME, &chart, &data, &measure).plot;
        let hit = mark_at(FRAME, &chart, &data, plot.x, plot.y, &measure);
        assert_eq!(hit, Some((0, None)));
    }

    #[test]
    fn a_click_on_a_pie_slice_names_its_point_with_series_zero() {
        let chart = bare(ChartKind::Pie);
        let data = ChartData {
            kind: ChartKind::Pie,
            categories: vec!["a".into(), "b".into()],
            series: vec![("Votes".into(), vec![1.0, 1.0])],
        };
        let plot = layout(FRAME, &chart, &data, &measure).plot;
        let cx = plot.x + plot.w / 2.0;
        let cy = plot.y + plot.h / 2.0;
        // Just above centre is the first half (12 o'clock going clockwise).
        let hit = mark_at(FRAME, &chart, &data, cx, cy - 5.0, &measure);
        assert_eq!(hit, Some((0, Some(0))));
    }

    fn data(kind: ChartKind) -> ChartData {
        ChartData {
            kind,
            categories: vec!["a".into(), "b".into()],
            series: vec![("Votes".into(), vec![100.0, 80.0])],
        }
    }

    #[test]
    fn an_axis_title_shrinks_the_plot_area_the_same_way_drawing_and_hit_testing_agree() {
        let data = data(ChartKind::Bar);
        let mut chart = bare(ChartKind::Bar);
        let without = layout(FRAME, &chart, &data, &measure).plot;
        chart.x_axis.label = Some("x".into());
        chart.y_axis.label = Some("y".into());
        let with_labels = layout(FRAME, &chart, &data, &measure).plot;
        assert!(with_labels.w < without.w);
        assert!(with_labels.h < without.h);
    }

    /// Tick labels take room too, and take it from the same two edges — the y axis' by the
    /// width of the widest number on it, which is why the layout has to measure text at all.
    #[test]
    fn tick_labels_take_their_room_out_of_the_plot() {
        let data = data(ChartKind::Bar);
        let bare = layout(FRAME, &bare(ChartKind::Bar), &data, &measure).plot;

        let mut chart = bare_with_ticks();
        let ticked = layout(FRAME, &chart, &data, &measure).plot;
        assert!(ticked.w < bare.w, "the y scale takes width");
        assert!(ticked.h < bare.h, "the categories take height");

        // Turning only one of them on takes only that edge.
        chart.x_axis.tick_labels = false;
        let only_y = layout(FRAME, &chart, &data, &measure).plot;
        assert!(only_y.w < bare.w);
        assert!(only_y.h > ticked.h);
    }

    fn bare_with_ticks() -> Chart {
        let mut chart = bare(ChartKind::Bar);
        chart.x_axis.tick_labels = true;
        chart.y_axis.tick_labels = true;
        chart
    }

    /// A pie has no axes, so nothing an axis carries costs it any of its own circle.
    #[test]
    fn a_pie_keeps_its_whole_frame_whatever_its_axes_say() {
        let data = data(ChartKind::Pie);
        let plain = layout(FRAME, &bare(ChartKind::Pie), &data, &measure).plot;
        let mut chart = bare_with_ticks();
        chart.kind = ChartKind::Pie;
        chart.x_axis.label = Some("Party".into());
        chart.y_axis.gridlines = true;
        assert_eq!(layout(FRAME, &chart, &data, &measure).plot, plain);
    }

    /// The plot is scaled to the top *tick*, not to the tallest bar — that is what puts the
    /// topmost gridline on the plot's own top edge instead of somewhere below it.
    #[test]
    fn the_plot_is_scaled_to_the_top_tick_rather_than_to_the_largest_value() {
        let chart = bare(ChartKind::Bar);
        let data = ChartData {
            kind: ChartKind::Bar,
            categories: vec!["a".into()],
            // 90 ticks up to 100, so the bar is nine tenths of the plot rather than all of it.
            series: vec![("Votes".into(), vec![90.0])],
        };
        let laid = layout(FRAME, &chart, &data, &measure);
        assert_eq!(laid.ticks.max(), 100.0);
        let top = value_y(&laid, 90.0);
        assert!(top > laid.plot.y, "the tallest bar stops short of the top");
        assert_eq!(value_y(&laid, 100.0), laid.plot.y);
    }
}
