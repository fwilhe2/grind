// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Drawing a chart's plot — bar, line or pie — from [`grind_sheet::ChartData`], at the
//! rectangle its own `draw:frame` occupies in widget space (`crate::geom::GridGeom::chart_rect`).
//!
//! A bar is a plain rectangle, the same `append_color` every other shape in [`crate::grid`]
//! draws with; a line or a pie slice needs an actual path, which is `gsk::PathBuilder` — GTK's
//! own vector drawing, already reachable through this shell's `v4_14` feature and one more
//! thing that means cairo is never pulled in for a chart this simple. No title, no legend, no
//! axis ticks: `doc/chart-format.md`'s own scope line, applied to the picture rather than the
//! file it comes from.
//!
//! Negative values are floored to zero rather than drawn the wrong way from a baseline that
//! would need its own zero line — every document this was built against (`ltwbw2026.*`, an
//! election's vote counts) is non-negative, and a chart of signed data is future work rather
//! than a regression here.

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::{gdk, graphene, gsk};

use grind_sheet::{ChartData, ChartKind};

use crate::geom::Rect;

/// How far the plot is inset from the frame's own border, so a bar or a pie never touches
/// the line drawn around the chart.
const INSET: f64 = 6.0;

/// The gap between one category's group of bars and the next, as a fraction of the group's
/// own width.
const GROUP_GAP: f64 = 0.2;

fn bounds(r: Rect) -> graphene::Rect {
    graphene::Rect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
}

/// Draw one chart at `rect`, in widget space. `colors` is one entry per series (bar, line)
/// or per category (pie) — [`grind_sheet::series_color`] is what a caller fills it from, so
/// what is drawn here is the same palette the writer assigns on save.
pub fn draw(
    snapshot: &gtk::Snapshot,
    rect: Rect,
    data: &ChartData,
    colors: &[gdk::RGBA],
    background: gdk::RGBA,
    border: gdk::RGBA,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    snapshot.append_color(&background, &bounds(rect));

    let plot = Rect {
        x: rect.x + INSET,
        y: rect.y + INSET,
        w: (rect.w - INSET * 2.0).max(0.0),
        h: (rect.h - INSET * 2.0).max(0.0),
    };
    if plot.w > 0.0 && plot.h > 0.0 {
        match data.kind {
            ChartKind::Bar => draw_bar(snapshot, plot, data, colors),
            ChartKind::Line => draw_line(snapshot, plot, data, colors),
            ChartKind::Pie => draw_pie(snapshot, plot, data, colors),
        }
    }

    let outline = gsk::PathBuilder::new();
    outline.add_rect(&bounds(rect));
    let stroke = gsk::Stroke::builder(1.0).build();
    snapshot.append_stroke(&outline.to_path(), &stroke, &border);
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

fn max_value(data: &ChartData) -> f64 {
    data.series
        .iter()
        .flat_map(|(_, values)| values.iter().copied())
        .fold(0.0_f64, f64::max)
        .max(1.0)
}

fn color_for(colors: &[gdk::RGBA], index: usize) -> gdk::RGBA {
    colors
        .get(index % colors.len().max(1))
        .copied()
        .unwrap_or(gdk::RGBA::BLACK)
}

fn draw_bar(snapshot: &gtk::Snapshot, plot: Rect, data: &ChartData, colors: &[gdk::RGBA]) {
    let categories = category_count(data);
    if categories == 0 {
        return;
    }
    let max = max_value(data);
    let series_count = data.series.len().max(1);
    let group_w = plot.w / categories as f64;
    let bar_w = (group_w * (1.0 - GROUP_GAP) / series_count as f64).max(1.0);
    let gap = (group_w - bar_w * series_count as f64) / (series_count as f64 + 1.0);
    for cat in 0..categories {
        for (s, (_, values)) in data.series.iter().enumerate() {
            let Some(&value) = values.get(cat) else {
                continue;
            };
            let h = (value.max(0.0) / max) * plot.h;
            let x = plot.x + cat as f64 * group_w + gap + s as f64 * (bar_w + gap);
            let y = plot.y + plot.h - h;
            snapshot.append_color(&color_for(colors, s), &bounds(Rect { x, y, w: bar_w, h }));
        }
    }
}

fn draw_line(snapshot: &gtk::Snapshot, plot: Rect, data: &ChartData, colors: &[gdk::RGBA]) {
    let categories = category_count(data);
    if categories < 2 {
        return;
    }
    let max = max_value(data);
    let step = plot.w / (categories - 1) as f64;
    for (s, (_, values)) in data.series.iter().enumerate() {
        if values.len() < 2 {
            continue;
        }
        let path = gsk::PathBuilder::new();
        for (i, &value) in values.iter().enumerate() {
            let x = (plot.x + i as f64 * step) as f32;
            let y = (plot.y + plot.h - (value.max(0.0) / max) * plot.h) as f32;
            match i {
                0 => path.move_to(x, y),
                _ => path.line_to(x, y),
            }
        }
        let stroke = gsk::Stroke::builder(2.0).build();
        snapshot.append_stroke(&path.to_path(), &stroke, &color_for(colors, s));
    }
}

/// One sample every this many degrees of arc — a slice is a straight-edged polygon rather
/// than a true arc (`gsk::PathBuilder` has no circular arc primitive; `conic_to` is a
/// rational Bezier and a fan of short segments is the simpler way to the same picture),
/// fine enough that the seam between segments is not visible.
const PIE_SAMPLES_PER_TURN: f64 = 96.0;

fn draw_pie(snapshot: &gtk::Snapshot, plot: Rect, data: &ChartData, colors: &[gdk::RGBA]) {
    let Some((_, values)) = data.series.first() else {
        return;
    };
    let total: f64 = values.iter().map(|v| v.max(0.0)).sum();
    if total <= 0.0 {
        return;
    }
    let cx = plot.x + plot.w / 2.0;
    let cy = plot.y + plot.h / 2.0;
    let radius = (plot.w.min(plot.h) / 2.0).max(1.0);
    let mut angle = -std::f64::consts::FRAC_PI_2; // 12 o'clock, reading order for the first slice.
    for (i, &value) in values.iter().enumerate() {
        let value = value.max(0.0);
        if value <= 0.0 {
            continue;
        }
        let sweep = (value / total) * std::f64::consts::TAU;
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
        snapshot.append_fill(
            &path.to_path(),
            gsk::FillRule::Winding,
            &color_for(colors, i),
        );
        angle += sweep;
    }
}
