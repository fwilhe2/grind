// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Drawing a chart's plot — bar, line or pie — from [`grind_sheet::ChartData`], at the
//! rectangle its own `draw:frame` occupies in widget space (`crate::geom::GridGeom::chart_rect`).
//!
//! A bar is a plain rectangle, the same `append_color` every other shape in [`crate::grid`]
//! draws with; a line or a pie slice needs an actual path, which is `gsk::PathBuilder` — GTK's
//! own vector drawing, already reachable through this shell's `v4_14` feature and one more
//! thing that means cairo is never pulled in for a chart this simple. The chart's own title and
//! legend are drawn, and so is everything an axis carries — its title, its tick labels and its
//! gridlines ([`grind_sheet::ChartAxis`]). The marks follow the `dataviz` skill's specs: capped,
//! rounded bars with the surface between them, 2px round-jointed lines, a legend whose words
//! are ink beside a swatch of the colour. [`describe`] is the words a tooltip shows for a mark.
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

use grind_sheet::{
    Chart, ChartAxis, ChartData, ChartKind, ChartLegend as Legend, Ticks, axis_ticks, pie_slice_at,
    pie_slices,
};

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

/// The widest a bar is drawn, in widget pixels — a bar never fills its slot, and what is left
/// of the band is air, which is what makes a column of bars read as a chart rather than as a
/// block (the `dataviz` mark spec).
const BAR_MAX: f64 = 24.0;

/// The gap left between two bars of the same group — the surface showing through, rather than
/// an outline drawn round each.
const BAR_GAP: f64 = 2.0;

/// How round a bar's data end is; its baseline end stays square.
const BAR_ROUNDING: f32 = 4.0;

/// A legend swatch's side, the gap between it and its label, the gap between two entries in a
/// row, and the gap between the legend and whatever it sits beside.
const SWATCH: f64 = 10.0;
const SWATCH_GAP: f64 = 6.0;
const ENTRY_GAP: f64 = 14.0;
const LEGEND_GAP: f64 = 10.0;

/// The gap under the chart's own title.
const TITLE_GAP: f64 = 6.0;

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
            ChartKind::Pie => draw_pie(snapshot, plot, chart, data, paint.color),
        }
        draw_ticks(widget, snapshot, &layout, chart, data, &measure, paint);
    }

    if let (Some(text), Some(band)) = (&chart.title, layout.title) {
        draw_title(widget, snapshot, text, band, paint.foreground);
    }
    // Text in ink, identity in the swatch beside it: a light series colour is illegible as
    // text, and a legend that coloured its words would be one only some readers could read.
    for entry in &layout.legend {
        let swatch = gsk::RoundedRect::from_rect(bounds(entry.swatch), 2.0);
        snapshot.push_rounded_clip(&swatch);
        snapshot.append_color(
            &(paint.color)(entry.series, entry.point),
            &bounds(entry.swatch),
        );
        snapshot.pop();
        place(
            widget,
            snapshot,
            &entry.label,
            entry.text.0,
            entry.text.1,
            paint.foreground,
        );
    }

    let axes_frame = layout.axes;
    if let Some(text) = axes(chart, data).0.label {
        draw_label(
            widget,
            snapshot,
            &text,
            axes_frame.x,
            axes_frame.y + axes_frame.h - LABEL_SPACE,
            axes_frame.w,
            paint.foreground,
            0.0,
        );
    }
    if let Some(text) = axes(chart, data).1.label {
        draw_label(
            widget,
            snapshot,
            &text,
            axes_frame.x,
            axes_frame.y,
            axes_frame.h,
            paint.foreground,
            -90.0,
        );
    }

    let outline = gsk::PathBuilder::new();
    outline.add_rect(&bounds(rect));
    let stroke = gsk::Stroke::builder(1.0).build();
    snapshot.append_stroke(&outline.to_path(), &stroke, &paint.border);
}

/// The chart's own title, centred in its band and set bold — the one piece of text on a chart
/// that is a heading rather than a label.
fn draw_title(
    widget: &impl IsA<gtk::Widget>,
    snapshot: &gtk::Snapshot,
    text: &str,
    band: Rect,
    color: gdk::RGBA,
) {
    let layout = widget.create_pango_layout(Some(text));
    let attributes = gtk::pango::AttrList::new();
    attributes.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold));
    layout.set_attributes(Some(&attributes));
    layout.set_width((band.w * f64::from(gtk::pango::SCALE)) as i32);
    layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
    layout.set_alignment(gtk::pango::Alignment::Center);
    snapshot.save();
    snapshot.translate(&graphene::Point::new(band.x as f32, band.y as f32));
    snapshot.append_layout(&layout, &color);
    snapshot.restore();
}

/// A chart's frame, divided up: its own title's band, its legend, the frame the axes are laid
/// out in once those have taken their room, the plot inside that, and the scale the values are
/// drawn against. Computed once and shared by everything that draws or hit-tests, which is what
/// keeps a click and the picture in step.
struct Layout {
    plot: Rect,
    /// Where the axes and their titles go — the chart's frame, less its title and its legend.
    axes: Rect,
    /// The value axis' own scale — [`grind_sheet::axis_ticks`], so the top of the plot is a
    /// round number rather than whatever the largest bar happened to be.
    ticks: Ticks,
    /// The band the chart's own title is drawn in, when it has one and there is room.
    title: Option<Rect>,
    /// Every legend entry, placed — empty when the chart has no legend, or no room for one.
    legend: Vec<LegendEntry>,
}

/// One entry of a legend: its swatch, where its label starts, and which mark it names — the
/// same `(series, point)` pair [`mark_at`] answers, so a click on it can colour what it names.
struct LegendEntry {
    swatch: Rect,
    text: (f64, f64),
    label: String,
    series: usize,
    point: Option<usize>,
}

/// What a legend names: one entry per series for a bar or a line, one per slice for a pie,
/// each with the `(series, point)` its colour is resolved by. A series with no name of its own
/// is named by its position, since a swatch with no words beside it names nothing.
fn legend_items(chart: &Chart, data: &ChartData) -> Vec<(String, usize, Option<usize>)> {
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
                (name, 0, Some(slice.point))
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
                (name, i, None)
            })
            .collect(),
    }
}

/// The legend laid out at its edge of `rect`, and what is left for everything else — or no
/// legend at all when it would take more than its share of a small chart, since a chart
/// squeezed to a sliver beside a legend is worse than one without.
fn place_legend(
    rect: Rect,
    position: Legend,
    items: Vec<(String, usize, Option<usize>)>,
    measure: &Measure,
) -> (Vec<LegendEntry>, Rect) {
    if items.is_empty() {
        return (Vec::new(), rect);
    }
    let sized: Vec<_> = items
        .into_iter()
        .map(|(label, series, point)| {
            let (w, h) = measure(&label);
            (label, series, point, w, h.max(SWATCH))
        })
        .collect();
    let line = sized.iter().map(|item| item.4).fold(0.0, f64::max);
    let entry =
        |x: f64, y: f64, (label, series, point, _, h): (String, usize, Option<usize>, f64, f64)| {
            LegendEntry {
                swatch: Rect {
                    x,
                    y: y + (h - SWATCH) / 2.0,
                    w: SWATCH,
                    h: SWATCH,
                },
                text: (x + SWATCH + SWATCH_GAP, y),
                label,
                series,
                point,
            }
        };
    match position {
        Legend::Start | Legend::End => {
            let width = SWATCH + SWATCH_GAP + sized.iter().map(|item| item.3).fold(0.0, f64::max);
            let height = sized.len() as f64 * (line + 4.0) - 4.0;
            if width + LEGEND_GAP > rect.w * 0.45 || height > rect.h - 2.0 * INSET {
                return (Vec::new(), rect);
            }
            let x = match position {
                Legend::End => rect.x + rect.w - INSET - width,
                _ => rect.x + INSET,
            };
            let mut y = rect.y + (rect.h - height) / 2.0;
            let mut entries = Vec::new();
            for item in sized {
                entries.push(entry(x, y, item));
                y += line + 4.0;
            }
            let rest = match position {
                Legend::End => Rect {
                    w: rect.w - width - LEGEND_GAP,
                    ..rect
                },
                _ => Rect {
                    x: rect.x + width + LEGEND_GAP,
                    w: rect.w - width - LEGEND_GAP,
                    ..rect
                },
            };
            (entries, rest)
        }
        Legend::Top | Legend::Bottom => {
            // Entries flow in rows, each row centred, wrapping at the frame's own width.
            let room = rect.w - 2.0 * INSET;
            let mut rows: Vec<Vec<_>> = vec![Vec::new()];
            let mut used = 0.0;
            for item in sized {
                let width = SWATCH + SWATCH_GAP + item.3;
                let row = rows.last_mut().expect("never empty");
                if !row.is_empty() && used + ENTRY_GAP + width > room {
                    rows.push(Vec::new());
                    used = 0.0;
                }
                let row = rows.last_mut().expect("never empty");
                used += if row.is_empty() {
                    width
                } else {
                    ENTRY_GAP + width
                };
                row.push(item);
            }
            let height = rows.len() as f64 * (line + 4.0) - 4.0;
            if rows.len() > 3 || height + LEGEND_GAP > rect.h * 0.35 {
                return (Vec::new(), rect);
            }
            let mut y = match position {
                Legend::Top => rect.y + INSET,
                _ => rect.y + rect.h - INSET - height,
            };
            let mut entries = Vec::new();
            for row in rows {
                let width: f64 = row
                    .iter()
                    .map(|item| SWATCH + SWATCH_GAP + item.3)
                    .sum::<f64>()
                    + ENTRY_GAP * (row.len().saturating_sub(1)) as f64;
                let mut x = rect.x + (rect.w - width) / 2.0;
                for item in row {
                    let step = SWATCH + SWATCH_GAP + item.3 + ENTRY_GAP;
                    entries.push(entry(x, y, item));
                    x += step;
                }
                y += line + 4.0;
            }
            let rest = match position {
                Legend::Top => Rect {
                    y: rect.y + height + LEGEND_GAP,
                    h: rect.h - height - LEGEND_GAP,
                    ..rect
                },
                _ => Rect {
                    h: rect.h - height - LEGEND_GAP,
                    ..rect
                },
            };
            (entries, rest)
        }
    }
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

/// The frame divided up ([`Layout`]): the chart's own title takes a band off the top, the
/// legend takes its edge ([`place_legend`]), and the plot is what is left once the axes have
/// taken what they need there — a title's fixed [`LABEL_SPACE`], the widest y tick label, one
/// line of x tick text.
fn layout(rect: Rect, chart: &Chart, data: &ChartData, measure: &Measure) -> Layout {
    let (x_axis, y_axis) = axes(chart, data);
    let ticks = axis_ticks(max_value(data));

    let mut frame = rect;
    let mut title = None;
    if let Some(text) = &chart.title {
        let (_, h) = measure(text);
        if rect.h > 4.0 * h {
            title = Some(Rect {
                x: rect.x + INSET,
                y: rect.y + INSET,
                w: (rect.w - 2.0 * INSET).max(0.0),
                h,
            });
            frame.y += h + TITLE_GAP;
            frame.h -= h + TITLE_GAP;
        }
    }
    let (legend, frame) = match chart.legend {
        Some(position) => place_legend(frame, position, legend_items(chart, data), measure),
        None => (Vec::new(), frame),
    };
    let rect = frame;

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
        axes: rect,
        ticks,
        title,
        legend,
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
///
/// A group's bars are [`BAR_MAX`] wide at most and [`BAR_GAP`] apart, centred in the group's
/// band; what is left of the band is air between groups.
struct BarLayout {
    categories: usize,
    series_count: usize,
    group_w: f64,
    bar_w: f64,
    /// Where the first bar of a group starts, measured from the start of its band.
    lead: f64,
}

fn bar_layout(plot: Rect, data: &ChartData) -> Option<BarLayout> {
    let categories = category_count(data);
    if categories == 0 {
        return None;
    }
    let series_count = data.series.len().max(1);
    let group_w = plot.w / categories as f64;
    let gaps = BAR_GAP * (series_count - 1) as f64;
    let bar_w = ((group_w * (1.0 - GROUP_GAP) - gaps) / series_count as f64).clamp(1.0, BAR_MAX);
    let lead = (group_w - (bar_w * series_count as f64 + gaps)) / 2.0;
    Some(BarLayout {
        categories,
        series_count,
        group_w,
        bar_w,
        lead,
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
    let x = plot.x + cat as f64 * bars.group_w + bars.lead + series as f64 * (bars.bar_w + BAR_GAP);
    Some(Rect {
        x,
        y,
        w: bars.bar_w,
        h: plot.y + plot.h - y,
    })
}

/// Each bar grows from the baseline, square there and rounded at its data end.
fn draw_bar(snapshot: &gtk::Snapshot, layout: &Layout, data: &ChartData, color: &MarkColor) {
    let Some(bars) = bar_layout(layout.plot, data) else {
        return;
    };
    for cat in 0..bars.categories {
        for s in 0..bars.series_count.min(data.series.len()) {
            let Some(rect) = bar_rect(layout, &bars, data, s, cat) else {
                continue;
            };
            if rect.h <= 0.0 {
                continue;
            }
            let round = BAR_ROUNDING.min(rect.w as f32 / 2.0).min(rect.h as f32);
            let corner = graphene::Size::new(round, round);
            let square = graphene::Size::zero();
            let shape = gsk::RoundedRect::new(bounds(rect), corner, corner, square, square);
            snapshot.push_rounded_clip(&shape);
            snapshot.append_color(&color(s, Some(cat)), &bounds(rect));
            snapshot.pop();
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
        let stroke = gsk::Stroke::builder(2.0)
            .line_join(gsk::LineJoin::Round)
            .line_cap(gsk::LineCap::Round)
            .build();
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

/// A pie's centre and radius within its plot — shared by [`draw_pie`] and [`pie_hit`].
fn pie_circle(plot: Rect) -> (f64, f64, f64) {
    (
        plot.x + plot.w / 2.0,
        plot.y + plot.h / 2.0,
        (plot.w.min(plot.h) / 2.0).max(1.0),
    )
}

/// Every slice as [`grind_sheet::pie_slices`] sweeps it — the core's arithmetic, so this
/// window and the browser cannot run the same pie two ways round (`doc/chart-format.md`,
/// Direction). `sweep` is signed, and stepping `start + sweep × t` draws either direction.
fn draw_pie(
    snapshot: &gtk::Snapshot,
    plot: Rect,
    chart: &Chart,
    data: &ChartData,
    color: &MarkColor,
) {
    let (cx, cy, radius) = pie_circle(plot);
    for slice in pie_slices(data, chart.clockwise) {
        let steps = ((slice.sweep.abs() / (std::f64::consts::TAU / PIE_SAMPLES_PER_TURN)).ceil()
            as usize)
            .max(1);
        let path = gsk::PathBuilder::new();
        path.move_to(cx as f32, cy as f32);
        for step in 0..=steps {
            let a = slice.start + slice.sweep * (step as f64 / steps as f64);
            path.line_to(
                (cx + radius * a.cos()) as f32,
                (cy + radius * a.sin()) as f32,
            );
        }
        path.close();
        snapshot.append_fill(
            &path.to_path(),
            gsk::FillRule::Winding,
            &color(0, Some(slice.point)),
        );
    }
}

/// Which pie slice, if any, `(x, y)` lands in — [`grind_sheet::pie_slice_at`] once the point
/// is known to be inside the circle.
fn pie_hit(plot: Rect, chart: &Chart, data: &ChartData, x: f64, y: f64) -> Option<usize> {
    let (cx, cy, radius) = pie_circle(plot);
    let (dx, dy) = (x - cx, y - cy);
    if (dx * dx + dy * dy).sqrt() > radius {
        return None;
    }
    pie_slice_at(data, chart.clockwise, dy.atan2(dx))
}

/// What the mark under `(x, y)` *is*, in words — a tooltip's text: `Sales · Feb: 20` for a bar
/// or a line, `Feb: 20 (33%)` for a pie slice. The same hit-test [`mark_at`] answers, so the
/// words are always about the mark under the pointer.
///
/// A line has no area of its own, so hovering one names the category nearest the pointer along
/// it; a legend entry is its own words already and gets no tooltip.
pub fn describe(
    rect: Rect,
    chart: &Chart,
    data: &ChartData,
    x: f64,
    y: f64,
    measure: &Measure,
) -> Option<String> {
    let layout = layout(rect, chart, data, measure);
    if legend_hit(&layout, x, y, measure).is_some() {
        return None;
    }
    let number = |n: f64| grind_sheet::formula::value::format_number(n);
    let series_name = |s: usize| match data.series.get(s) {
        Some((name, _)) if !name.is_empty() => name.clone(),
        _ => format!("Series {}", s + 1),
    };
    let category = |i: usize| match data.categories.get(i) {
        Some(name) if !name.is_empty() => name.clone(),
        _ => format!("{}", i + 1),
    };
    match data.kind {
        ChartKind::Bar => {
            let (s, cat) = bar_hit(&layout, data, x, y)?;
            let value = *data.series.get(s)?.1.get(cat)?;
            Some(format!(
                "{} · {}: {}",
                series_name(s),
                category(cat),
                number(value)
            ))
        }
        ChartKind::Line => {
            let s = line_hit(&layout, data, x, y)?;
            let ticks = category_ticks(&layout, data);
            let (cat, _) = ticks
                .iter()
                .enumerate()
                .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))?;
            let value = *data.series.get(s)?.1.get(cat)?;
            Some(format!(
                "{} · {}: {}",
                series_name(s),
                category(cat),
                number(value)
            ))
        }
        ChartKind::Pie => {
            let point = pie_hit(layout.plot, chart, data, x, y)?;
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

/// The legend entry, if any, whose swatch or words `(x, y)` lands on.
fn legend_hit<'a>(
    layout: &'a Layout,
    x: f64,
    y: f64,
    measure: &Measure,
) -> Option<&'a LegendEntry> {
    layout.legend.iter().find(|entry| {
        let (w, h) = measure(&entry.label);
        Rect {
            x: entry.swatch.x,
            y: entry.text.1.min(entry.swatch.y),
            w: entry.text.0 - entry.swatch.x + w,
            h: h.max(SWATCH),
        }
        .contains(x, y)
    })
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
    // A legend entry names the mark it is the colour of, swatch and words alike.
    if let Some(entry) = legend_hit(&layout, x, y, measure) {
        return Some((entry.series, entry.point));
    }
    match data.kind {
        ChartKind::Bar => bar_hit(&layout, data, x, y).map(|(s, p)| (s, Some(p))),
        ChartKind::Line => line_hit(&layout, data, x, y).map(|s| (s, None)),
        ChartKind::Pie => pie_hit(layout.plot, chart, data, x, y).map(|p| (0, Some(p))),
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
        // The middle of the first bar, near its bottom (a full-height bar).
        let laid = layout(FRAME, &chart, &data, &measure);
        let bars = bar_layout(laid.plot, &data).unwrap();
        let first = bar_rect(&laid, &bars, &data, 0, 0).unwrap();
        let hit = mark_at(
            FRAME,
            &chart,
            &data,
            first.x + first.w / 2.0,
            90.0,
            &measure,
        );
        assert_eq!(hit, Some((0, Some(0))));
        // The band either side of a capped bar is air, not the bar.
        assert_eq!(
            mark_at(FRAME, &chart, &data, first.x - 2.0, 90.0, &measure),
            None
        );
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

    fn two_series() -> ChartData {
        ChartData {
            kind: ChartKind::Bar,
            categories: vec!["Jan".into(), "Feb".into()],
            series: vec![
                ("Sales".into(), vec![100.0, 80.0]),
                ("Costs".into(), vec![40.0, 30.0]),
            ],
        }
    }

    /// The `dataviz` mark spec: a bar never fills its slot, and two bars of one group are
    /// separated by the surface rather than touching.
    #[test]
    fn bars_are_capped_and_two_of_a_group_stand_a_gap_apart() {
        let wide = Rect { w: 900.0, ..FRAME };
        let data = two_series();
        let laid = layout(wide, &bare(ChartKind::Bar), &data, &measure);
        let bars = bar_layout(laid.plot, &data).unwrap();
        let a = bar_rect(&laid, &bars, &data, 0, 0).unwrap();
        let b = bar_rect(&laid, &bars, &data, 1, 0).unwrap();
        assert_eq!(a.w, BAR_MAX);
        assert!((b.x - (a.x + a.w) - BAR_GAP).abs() < 1e-9);
    }

    /// A legend takes its edge out of the plot, and a click on an entry names what it is the
    /// colour of — the series for a bar, the slice for a pie.
    #[test]
    fn a_legend_takes_its_edge_and_its_entries_name_their_marks() {
        let data = two_series();
        let mut chart = bare(ChartKind::Bar);
        let without = layout(FRAME, &chart, &data, &measure).plot;
        for (position, narrower) in [(Legend::End, true), (Legend::Bottom, false)] {
            chart.legend = Some(position);
            let laid = layout(FRAME, &chart, &data, &measure);
            assert_eq!(laid.legend.len(), 2, "{position:?}");
            match narrower {
                true => assert!(laid.plot.w < without.w && laid.plot.h == without.h),
                false => assert!(laid.plot.h < without.h),
            }
            let costs = &laid.legend[1];
            let hit = mark_at(
                FRAME,
                &chart,
                &data,
                costs.swatch.x + 1.0,
                costs.swatch.y + 1.0,
                &measure,
            );
            assert_eq!(hit, Some((1, None)), "{position:?}");
        }
        let pie = ChartData {
            kind: ChartKind::Pie,
            ..two_series()
        };
        chart.kind = ChartKind::Pie;
        chart.legend = Some(Legend::End);
        let laid = layout(FRAME, &chart, &pie, &measure);
        let names: Vec<&str> = laid.legend.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(names, ["Jan", "Feb"], "a pie's legend names its slices");
        assert_eq!((laid.legend[1].series, laid.legend[1].point), (0, Some(1)));
    }

    /// A legend that would crowd a small chart out is left off rather than squeezing the plot
    /// to a sliver.
    #[test]
    fn a_legend_too_big_for_its_chart_is_left_off() {
        let tiny = Rect {
            w: 60.0,
            h: 40.0,
            ..FRAME
        };
        let mut chart = bare(ChartKind::Bar);
        chart.legend = Some(Legend::End);
        assert!(
            layout(tiny, &chart, &two_series(), &measure)
                .legend
                .is_empty()
        );
    }

    #[test]
    fn a_title_takes_a_band_off_the_top() {
        let data = two_series();
        let mut chart = bare(ChartKind::Bar);
        let without = layout(FRAME, &chart, &data, &measure).plot;
        chart.title = Some("Quarter".into());
        let laid = layout(FRAME, &chart, &data, &measure);
        let band = laid.title.expect("room for a title");
        assert!(band.y < laid.plot.y && laid.plot.h < without.h);
    }

    /// The tooltip names the mark under the pointer, in the reader's words rather than the
    /// file's.
    #[test]
    fn a_tooltip_names_the_series_the_category_and_the_value() {
        let data = two_series();
        let chart = bare(ChartKind::Bar);
        let laid = layout(FRAME, &chart, &data, &measure);
        let bars = bar_layout(laid.plot, &data).unwrap();
        let costs_feb = bar_rect(&laid, &bars, &data, 1, 1).unwrap();
        let said = describe(
            FRAME,
            &chart,
            &data,
            costs_feb.x + costs_feb.w / 2.0,
            costs_feb.y + costs_feb.h - 1.0,
            &measure,
        );
        assert_eq!(said.as_deref(), Some("Costs · Feb: 30"));
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
