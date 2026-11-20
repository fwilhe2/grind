// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart in this window: [`grind_sheet::chart_paint`]'s marks as GSK nodes, measured and set in
//! the widget's own font. Where every mark goes, and what a point over it means, is the core's.

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::{gdk, graphene, gsk};

use grind_sheet::chart_paint::{self, Colours, Mark, Picture, Target};
use grind_sheet::{Chart, ChartData};

use crate::geom::Rect;
use crate::grid::SheetMetrics;
use crate::theme::{Palette, is_dark, rgb8, rgba};

/// The theme's colours as the core takes them.
pub fn colours(palette: &Palette) -> Colours {
    Colours {
        page: rgb8(palette.background),
        ink: rgb8(palette.foreground),
        grid: rgb8(palette.lines),
        accent: rgb8(palette.accent),
        dark: is_dark(palette),
    }
}

/// One chart laid out in `rect`, in widget space.
pub fn picture(
    widget: &impl IsA<gtk::Widget>,
    rect: Rect,
    chart: &Chart,
    data: &ChartData,
    palette: &Palette,
) -> Picture {
    let frame = chart_paint::Rect::new(rect.x, rect.y, rect.w, rect.h);
    chart_paint::picture(
        chart,
        data,
        frame,
        &colours(palette),
        &SheetMetrics::of(widget),
    )
}

/// Draw one chart at `rect`, in widget space.
pub fn draw(
    widget: &impl IsA<gtk::Widget>,
    snapshot: &gtk::Snapshot,
    rect: Rect,
    chart: &Chart,
    data: &ChartData,
    palette: &Palette,
) {
    let metrics = SheetMetrics::of(widget);
    let frame = chart_paint::Rect::new(rect.x, rect.y, rect.w, rect.h);
    for mark in chart_paint::draw(chart, data, frame, &colours(palette), &metrics) {
        put(widget, snapshot, &metrics, mark);
    }
}

/// The mark under `(x, y)` in words — a tooltip.
pub fn describe(
    widget: &impl IsA<gtk::Widget>,
    rect: Rect,
    chart: &Chart,
    data: &ChartData,
    palette: &Palette,
    (x, y): (f64, f64),
) -> Option<String> {
    picture(widget, rect, chart, data, palette).describe(x, y, data)
}

/// Which mark `(x, y)` hits, as the `(series, point)` its colour is resolved by.
pub fn mark_at(
    widget: &impl IsA<gtk::Widget>,
    rect: Rect,
    chart: &Chart,
    data: &ChartData,
    palette: &Palette,
    (x, y): (f64, f64),
) -> Option<(usize, Option<usize>)> {
    picture(widget, rect, chart, data, palette)
        .target_at(x, y, data)
        .map(Target::mark)
}

fn bounds(r: chart_paint::Rect) -> graphene::Rect {
    graphene::Rect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
}

fn path(points: &[(f64, f64)], close: bool) -> gsk::Path {
    let builder = gsk::PathBuilder::new();
    for (i, &(x, y)) in points.iter().enumerate() {
        match i {
            0 => builder.move_to(x as f32, y as f32),
            _ => builder.line_to(x as f32, y as f32),
        }
    }
    if close {
        builder.close();
    }
    builder.to_path()
}

fn put(
    widget: &impl IsA<gtk::Widget>,
    snapshot: &gtk::Snapshot,
    metrics: &SheetMetrics,
    mark: Mark,
) {
    match mark {
        Mark::Fill { rect, color } => snapshot.append_color(&rgba(color), &bounds(rect)),
        Mark::Path {
            points,
            fill,
            stroke,
        } => {
            if points.len() < 2 {
                return;
            }
            if let Some(color) = fill {
                snapshot.append_fill(&path(&points, true), gsk::FillRule::Winding, &rgba(color));
            }
            if let Some((color, width)) = stroke {
                let stroke = gsk::Stroke::builder(width as f32)
                    .line_join(gsk::LineJoin::Round)
                    .line_cap(gsk::LineCap::Round)
                    .build();
                snapshot.append_stroke(&path(&points, fill.is_some()), &stroke, &rgba(color));
            }
        }
        Mark::Text {
            x,
            top,
            text,
            style,
            color,
            clip,
            turn,
        } => {
            let layout = widget.create_pango_layout(Some(&text));
            layout.set_font_description(Some(&metrics.font(&style)));
            snapshot.push_clip(&bounds(clip));
            snapshot.save();
            snapshot.translate(&graphene::Point::new(x as f32, top as f32));
            // The core turns counter-clockwise; GSK turns clockwise.
            snapshot.rotate(-turn as f32);
            snapshot.append_layout(&layout, &rgba(color));
            snapshot.restore();
            snapshot.pop();
        }
    }
}

/// The colour a mark is drawn in, as the popover that recolours it shows it.
pub fn shown(chart: &Chart, series: usize, point: Option<usize>) -> gdk::RGBA {
    crate::theme::color(&grind_sheet::effective_color(chart, series, point))
        .unwrap_or(gdk::RGBA::BLACK)
}
