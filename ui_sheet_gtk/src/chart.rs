// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Drawing a chart's plot — bar, line or pie — from [`grind_sheet::ChartData`], at the
//! rectangle its own `draw:frame` occupies in widget space (`crate::geom::GridGeom::chart_rect`).
//!
//! A bar is a plain rectangle, the same `append_color` every other shape in [`crate::grid`]
//! draws with; a line or a pie slice needs an actual path, which is `gsk::PathBuilder` — GTK's
//! own vector drawing, already reachable through this shell's `v4_14` feature and one more
//! thing that means cairo is never pulled in for a chart this simple. No chart-level title, no
//! legend, no axis ticks: `doc/chart-format.md`'s own scope line, applied to the picture rather
//! than the file it comes from — an axis label is the one exception, since it is a different
//! element (`grind_sheet::chart::Chart::x_axis_label`/`y_axis_label`) from the chart-level
//! title/legend that stays out.
//!
//! Negative values are floored to zero rather than drawn the wrong way from a baseline that
//! would need its own zero line — every document this was built against (`ltwbw2026.*`, an
//! election's vote counts) is non-negative, and a chart of signed data is future work rather
//! than a regression here.
//!
//! [`mark_at`] shares the exact geometry [`draw`] paints from, so a click and a picture can
//! never disagree about which bar, slice or line a point belongs to — `grid.rs`'s
//! `chart_mark_hit` is the one caller.

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

/// Room reserved for an axis label, in widget pixels — one line of text, no font
/// customisation, the same minimalism `doc/chart-format.md` states for everything else this
/// drawer does.
const LABEL_SPACE: f64 = 18.0;

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

/// Draw one chart at `rect`, in widget space. `color` resolves a mark's colour —
/// `grind_sheet::chart::effective_color` converted to a swatch is what a caller fills it from,
/// so what is drawn here is the same colour the writer assigns on save.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    widget: &impl IsA<gtk::Widget>,
    snapshot: &gtk::Snapshot,
    rect: Rect,
    data: &ChartData,
    color: &MarkColor,
    background: gdk::RGBA,
    border: gdk::RGBA,
    foreground: gdk::RGBA,
    x_axis_label: Option<&str>,
    y_axis_label: Option<&str>,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    snapshot.append_color(&background, &bounds(rect));

    let plot = plot_rect(rect, x_axis_label, y_axis_label);
    if plot.w > 0.0 && plot.h > 0.0 {
        match data.kind {
            ChartKind::Bar => draw_bar(snapshot, plot, data, color),
            ChartKind::Line => draw_line(snapshot, plot, data, color),
            ChartKind::Pie => draw_pie(snapshot, plot, data, color),
        }
    }

    if let Some(text) = x_axis_label {
        draw_label(
            widget,
            snapshot,
            text,
            rect.x,
            rect.y + rect.h - LABEL_SPACE,
            rect.w,
            foreground,
            0.0,
        );
    }
    if let Some(text) = y_axis_label {
        draw_label(
            widget, snapshot, text, rect.x, rect.y, rect.h, foreground, -90.0,
        );
    }

    let outline = gsk::PathBuilder::new();
    outline.add_rect(&bounds(rect));
    let stroke = gsk::Stroke::builder(1.0).build();
    snapshot.append_stroke(&outline.to_path(), &stroke, &border);
}

/// The plot area within `rect` — inset from the frame's own border, and further inset for
/// whichever axis labels are present. Shared by [`draw`] and [`mark_at`] so a click and the
/// picture it is clicking on never disagree.
fn plot_rect(rect: Rect, x_axis_label: Option<&str>, y_axis_label: Option<&str>) -> Rect {
    let left = INSET
        + if y_axis_label.is_some() {
            LABEL_SPACE
        } else {
            0.0
        };
    let bottom = INSET
        + if x_axis_label.is_some() {
            LABEL_SPACE
        } else {
            0.0
        };
    Rect {
        x: rect.x + left,
        y: rect.y + INSET,
        w: (rect.w - left - INSET).max(0.0),
        h: (rect.h - bottom - INSET).max(0.0),
    }
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
    plot: Rect,
    layout: &BarLayout,
    data: &ChartData,
    series: usize,
    cat: usize,
) -> Option<Rect> {
    let max = max_value(data);
    let value = *data.series.get(series)?.1.get(cat)?;
    let h = (value.max(0.0) / max) * plot.h;
    let x = plot.x
        + cat as f64 * layout.group_w
        + layout.gap
        + series as f64 * (layout.bar_w + layout.gap);
    let y = plot.y + plot.h - h;
    Some(Rect {
        x,
        y,
        w: layout.bar_w,
        h,
    })
}

fn draw_bar(snapshot: &gtk::Snapshot, plot: Rect, data: &ChartData, color: &MarkColor) {
    let Some(layout) = bar_layout(plot, data) else {
        return;
    };
    for cat in 0..layout.categories {
        for s in 0..layout.series_count.min(data.series.len()) {
            let Some(rect) = bar_rect(plot, &layout, data, s, cat) else {
                continue;
            };
            snapshot.append_color(&color(s, Some(cat)), &bounds(rect));
        }
    }
}

/// Which bar, if any, `(x, y)` lands in.
fn bar_hit(plot: Rect, data: &ChartData, x: f64, y: f64) -> Option<(usize, usize)> {
    let layout = bar_layout(plot, data)?;
    for cat in 0..layout.categories {
        for s in 0..layout.series_count.min(data.series.len()) {
            if let Some(rect) = bar_rect(plot, &layout, data, s, cat)
                && rect.contains(x, y)
            {
                return Some((s, cat));
            }
        }
    }
    None
}

/// One line series' own points, in widget space — shared the same way [`BarLayout`] is.
fn line_points(plot: Rect, data: &ChartData, series: usize) -> Option<Vec<(f64, f64)>> {
    let categories = category_count(data);
    if categories < 2 {
        return None;
    }
    let max = max_value(data);
    let step = plot.w / (categories - 1) as f64;
    let (_, values) = data.series.get(series)?;
    if values.len() < 2 {
        return None;
    }
    Some(
        values
            .iter()
            .enumerate()
            .map(|(i, &value)| {
                (
                    plot.x + i as f64 * step,
                    plot.y + plot.h - (value.max(0.0) / max) * plot.h,
                )
            })
            .collect(),
    )
}

fn draw_line(snapshot: &gtk::Snapshot, plot: Rect, data: &ChartData, color: &MarkColor) {
    for s in 0..data.series.len() {
        let Some(points) = line_points(plot, data, s) else {
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
fn line_hit(plot: Rect, data: &ChartData, x: f64, y: f64) -> Option<usize> {
    for s in 0..data.series.len() {
        let Some(points) = line_points(plot, data, s) else {
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
/// `rect` is the chart's own frame, exactly what [`draw`] was given.
pub fn mark_at(
    rect: Rect,
    data: &ChartData,
    x: f64,
    y: f64,
    x_axis_label: Option<&str>,
    y_axis_label: Option<&str>,
) -> Option<(usize, Option<usize>)> {
    let plot = plot_rect(rect, x_axis_label, y_axis_label);
    match data.kind {
        ChartKind::Bar => bar_hit(plot, data, x, y).map(|(s, p)| (s, Some(p))),
        ChartKind::Line => line_hit(plot, data, x, y).map(|s| (s, None)),
        ChartKind::Pie => pie_hit(plot, data, x, y).map(|p| (0, Some(p))),
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

    #[test]
    fn a_click_on_one_bar_names_its_series_and_point_not_the_other_bar() {
        let data = ChartData {
            kind: ChartKind::Bar,
            categories: vec!["a".into(), "b".into()],
            series: vec![("Votes".into(), vec![100.0, 80.0])],
        };
        // The first bar is the left half of the plot, near its bottom (a full-height bar).
        let hit = mark_at(FRAME, &data, 30.0, 90.0, None, None);
        assert_eq!(hit, Some((0, Some(0))));
        // Well outside the plot area hits nothing.
        assert_eq!(mark_at(FRAME, &data, 500.0, 500.0, None, None), None);
    }

    #[test]
    fn a_click_on_a_line_names_its_series_and_no_point() {
        let data = ChartData {
            kind: ChartKind::Line,
            categories: vec!["a".into(), "b".into()],
            series: vec![("Votes".into(), vec![100.0, 0.0])],
        };
        // At the left edge, the line sits at the top of the plot (the max value).
        let plot = plot_rect(FRAME, None, None);
        let hit = mark_at(FRAME, &data, plot.x, plot.y, None, None);
        assert_eq!(hit, Some((0, None)));
    }

    #[test]
    fn a_click_on_a_pie_slice_names_its_point_with_series_zero() {
        let data = ChartData {
            kind: ChartKind::Pie,
            categories: vec!["a".into(), "b".into()],
            series: vec![("Votes".into(), vec![1.0, 1.0])],
        };
        let plot = plot_rect(FRAME, None, None);
        let cx = plot.x + plot.w / 2.0;
        let cy = plot.y + plot.h / 2.0;
        // Just above centre is the first half (12 o'clock going clockwise).
        let hit = mark_at(FRAME, &data, cx, cy - 5.0, None, None);
        assert_eq!(hit, Some((0, Some(0))));
    }

    #[test]
    fn an_axis_label_shrinks_the_plot_area_the_same_way_drawing_and_hit_testing_agree() {
        let with_labels = plot_rect(FRAME, Some("x"), Some("y"));
        let without = plot_rect(FRAME, None, None);
        assert!(with_labels.w < without.w);
        assert!(with_labels.h < without.h);
    }
}
