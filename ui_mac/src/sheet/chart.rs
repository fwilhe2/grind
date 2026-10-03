// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart, as a list of [`Op`]s — portable, so where every bar, point, slice and label goes is
//! tested on Linux and the Mac only puts it down.
//!
//! The fourth renderer of `doc/chart-format.md`'s shapes, after the GTK shell's `snapshot()`, the
//! browser's SVG and the writer's own `chart:` XML. It draws from [`ChartData`] and [`Chart`] as
//! those do, scales the plot against [`axis_ticks`] rather than the tallest bar — so a chart has
//! the same axis here as in either other window — sweeps a pie with [`pie_slices`], so it runs the
//! same way round, and colours every mark with [`effective_color`]. Unlike the browser's, text is
//! *measured*, through the same `Metrics` the grid's cells are, so a legend or a tick label is
//! placed on its real width.
//!
//! **Read-only.** A chart is a picture on this grid: nothing here is hit-tested, and adding or
//! editing one is the CLI's and the GNOME window's (`doc/feature-matrix.md`).

use grind_core::layout::Metrics;
use grind_sheet::{App, Chart, ChartData, ChartKind, ChartLegend};

use super::geom::{PT_PER_MM, Rect};
use super::paint::Palette;
use crate::ops::Op;

/// Where a chart's frame is on its sheet, in points from the corner of A1 — `draw:frame`'s own
/// position and size, or `None` for a length this build cannot read, which is a chart it does
/// not draw rather than one drawn in the wrong place.
pub fn frame_of(chart: &Chart) -> Option<Rect> {
    let pt = |length: &str| grind_sheet::style::length_mm(length).map(|mm| mm * PT_PER_MM);
    let rect = Rect::new(
        pt(&chart.x)?,
        pt(&chart.y)?,
        pt(&chart.width)?,
        pt(&chart.height)?,
    );
    (rect.w > 0.0 && rect.h > 0.0).then_some(rect)
}

/// A chart being dragged: which, and how far it has moved, in points.
pub type Moving = Option<(usize, f64, f64)>;

/// Every chart on `sheet` that meets `view`, drawn in the sheet's own coordinates — the space
/// [`super::paint::cells`] draws in, so a chart floats over the cells under it. The one being
/// dragged, if one is, is drawn where the drag has it.
pub fn charts(
    app: &App,
    sheet: usize,
    view: &Rect,
    palette: &Palette,
    metrics: &dyn Metrics,
    moving: Moving,
) -> Vec<Op> {
    let mut ops = Vec::new();
    for (index, chart) in app.charts(sheet).unwrap_or_default().iter().enumerate() {
        let Some(mut frame) = frame_of(chart) else {
            continue;
        };
        if let Some((_, dx, dy)) = moving.filter(|(at, ..)| *at == index) {
            frame = frame.offset(dx, dy);
        }
        if frame.intersection(view).is_empty() {
            continue;
        }
        let Ok(data) = app.chart_data(sheet, index) else {
            continue;
        };
        ops.extend(draw(chart, &data, frame, palette, metrics));
    }
    ops
}

/// The topmost chart on `sheet` whose frame holds `(x, y)` — the last drawn, since a later chart
/// covers an earlier one — by its index, for a context menu to act on.
pub fn chart_at(app: &App, sheet: usize, x: f64, y: f64) -> Option<usize> {
    let charts = app.charts(sheet).ok()?;
    charts.iter().enumerate().rev().find_map(|(index, chart)| {
        let frame = frame_of(chart)?;
        (x >= frame.x && x < frame.right() && y >= frame.y && y < frame.bottom()).then_some(index)
    })
}

/// Where a chart dragged by `(dx, dy)` lands, as the two ODF lengths `App::reshape_chart` takes
/// for its corner — never above or left of A1, where nothing can show it.
pub fn moved_to(frame: Rect, dx: f64, dy: f64) -> (String, String) {
    let length = |pt: f64| grind_sheet::style::mm_length(pt.max(0.0) / PT_PER_MM);
    (length(frame.x + dx), length(frame.y + dy))
}

/// One change a chart's context menu makes.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Kind(ChartKind),
    Legend(Option<ChartLegend>),
    Title(Option<String>),
}

/// What `chart` is with `change` made — the spec `App::edit_chart` takes, everything else kept.
pub fn changed(chart: &Chart, change: Change) -> grind_sheet::ChartSpec {
    let mut spec = grind_sheet::ChartSpec::of(chart);
    match change {
        Change::Kind(kind) => spec.kind = kind,
        Change::Legend(legend) => spec.legend = legend,
        Change::Title(title) => spec.title = title.filter(|title| !title.trim().is_empty()),
    }
    spec
}

/// One chart in `frame`: `grind_sheet::chart_paint`'s marks, which every window that draws a chart
/// shares, turned into this shell's [`Op`]s.
pub fn draw(
    chart: &Chart,
    data: &ChartData,
    frame: Rect,
    palette: &Palette,
    metrics: &dyn Metrics,
) -> Vec<Op> {
    use grind_sheet::chart_paint::{self as paint, Colours, Mark};
    let rect = |r: paint::Rect| Rect::new(r.x, r.y, r.w, r.h);
    let colours = Colours {
        page: palette.page,
        ink: palette.ink,
        grid: palette.grid,
        header_ink: palette.header_ink,
        accent: palette.accent,
        dark: palette.dark,
    };
    let frame = paint::Rect::new(frame.x, frame.y, frame.w, frame.h);
    paint::draw(chart, data, frame, &colours, metrics)
        .into_iter()
        .map(|mark| match mark {
            Mark::Fill { rect: r, color } => Op::Fill {
                rect: rect(r),
                color,
            },
            Mark::Text {
                x,
                top,
                text,
                style,
                color,
                clip,
            } => Op::Text {
                x,
                top,
                text,
                style,
                color,
                clip: rect(clip),
            },
            Mark::Path {
                points,
                fill,
                stroke,
            } => Op::Path {
                points,
                fill,
                stroke,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::color;
    use grind_core::layout::Fixed;
    use grind_sheet::{Pos, RecalcMode, effective_color};

    /// A sheet with a small table and one chart over it, of `kind`.
    fn sheet_with(kind: &str) -> App {
        let app = App::new();
        for (row, (name, value)) in [("North", "3"), ("South", "5"), ("East", "2")]
            .iter()
            .enumerate()
        {
            app.enter(0, Pos::new(row as u32, 0), name, RecalcMode::Document)
                .unwrap();
            app.enter(0, Pos::new(row as u32, 1), value, RecalcMode::Document)
                .unwrap();
        }
        let kind = match kind {
            "bar" => ChartKind::Bar,
            "line" => ChartKind::Line,
            _ => ChartKind::Pie,
        };
        let spec = grind_sheet::ChartSpec {
            categories: Some("A1:A3".into()),
            series: vec![("B1:B3".into(), None)],
            ..grind_sheet::ChartSpec::new(kind)
        };
        app.add_chart(0, &spec, "1cm", "1cm", "10cm", "6cm")
            .unwrap();
        app
    }

    fn marks(ops: &[Op]) -> (usize, usize) {
        let paths = ops
            .iter()
            .filter(|op| matches!(op, Op::Path { .. }))
            .count();
        let fills = ops
            .iter()
            .filter(|op| matches!(op, Op::Fill { .. }))
            .count();
        (paths, fills)
    }

    #[test]
    fn a_frame_is_read_from_its_lengths() {
        let chart = Chart::new(
            ChartKind::Bar,
            "1in".into(),
            "0cm".into(),
            "2in".into(),
            "1in".into(),
        );
        assert_eq!(frame_of(&chart), Some(Rect::new(72.0, 0.0, 144.0, 72.0)));
        let broken = Chart {
            width: "wide".into(),
            ..chart
        };
        assert_eq!(
            frame_of(&broken),
            None,
            "a length nothing reads is not drawn"
        );
    }

    #[test]
    fn a_bar_chart_draws_a_bar_per_value_standing_on_one_baseline() {
        let app = sheet_with("bar");
        let chart = &app.charts(0).unwrap()[0];
        let data = app.chart_data(0, 0).unwrap();
        let frame = Rect::new(0.0, 0.0, 300.0, 200.0);
        let ops = draw(chart, &data, frame, &Palette::LIGHT, &Fixed);
        let colour = color::parse(&effective_color(chart, 0, Some(0))).unwrap();
        let bars: Vec<Rect> = ops
            .iter()
            .filter_map(|op| match op {
                Op::Fill { rect, color } if *color == colour => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 3, "{bars:?}");
        let floor = bars[0].bottom();
        assert!(bars.iter().all(|bar| (bar.bottom() - floor).abs() < 1e-9));
        assert!(bars[1].h > bars[0].h && bars[0].h > bars[2].h, "5 > 3 > 2");
        assert!(
            bars.iter().all(|bar| bar.x >= frame.x
                && bar.y >= frame.y
                && bar.right() <= frame.right()
                && bar.bottom() <= frame.bottom()),
            "{bars:?} in {frame:?}"
        );
    }

    #[test]
    fn a_line_is_one_path_and_a_pie_one_per_slice() {
        let app = sheet_with("line");
        let data = app.chart_data(0, 0).unwrap();
        let chart = &app.charts(0).unwrap()[0];
        let frame = Rect::new(10.0, 10.0, 300.0, 200.0);
        let (paths, _) = marks(&draw(chart, &data, frame, &Palette::LIGHT, &Fixed));
        assert_eq!(paths, 1);

        let app = sheet_with("pie");
        let data = app.chart_data(0, 0).unwrap();
        let chart = &app.charts(0).unwrap()[0];
        let ops = draw(chart, &data, frame, &Palette::LIGHT, &Fixed);
        let slices: Vec<&Vec<(f64, f64)>> = ops
            .iter()
            .filter_map(|op| match op {
                Op::Path { points, fill, .. } if fill.is_some() => Some(points),
                _ => None,
            })
            .collect();
        assert_eq!(slices.len(), 3);
        for points in slices {
            for (x, y) in points {
                assert!(frame.x <= *x && *x <= frame.right() && frame.y <= *y);
                assert!(*y <= frame.bottom());
            }
        }
    }

    #[test]
    fn charts_off_the_view_are_not_drawn() {
        let app = sheet_with("bar");
        let frame = frame_of(&app.charts(0).unwrap()[0]).unwrap();
        let near = charts(&app, 0, &frame, &Palette::LIGHT, &Fixed, None);
        assert!(!near.is_empty());
        let far = Rect::new(frame.right() + 1000.0, 0.0, 100.0, 100.0);
        assert!(charts(&app, 0, &far, &Palette::LIGHT, &Fixed, None).is_empty());
    }

    #[test]
    fn the_topmost_chart_under_a_point_is_found_and_changed() {
        let app = sheet_with("bar");
        let frame = frame_of(&app.charts(0).unwrap()[0]).unwrap();
        assert_eq!(chart_at(&app, 0, frame.x + 5.0, frame.y + 5.0), Some(0));
        assert_eq!(chart_at(&app, 0, frame.x - 5.0, frame.y + 5.0), None);
        let chart = &app.charts(0).unwrap()[0];
        let pie = changed(chart, Change::Kind(ChartKind::Pie));
        assert_eq!(pie.kind, ChartKind::Pie);
        assert_eq!(
            pie.series,
            grind_sheet::ChartSpec::of(chart).series,
            "the rest kept"
        );
        assert_eq!(changed(chart, Change::Title(Some("  ".into()))).title, None);
        app.edit_chart(
            0,
            0,
            &changed(chart, Change::Legend(Some(ChartLegend::Top))),
        )
        .unwrap();
        assert_eq!(app.charts(0).unwrap()[0].legend, Some(ChartLegend::Top));
    }

    #[test]
    fn a_dragged_chart_is_drawn_where_the_drag_has_it_and_lands_there() {
        let app = sheet_with("bar");
        let frame = frame_of(&app.charts(0).unwrap()[0]).unwrap();
        let view = Rect::new(0.0, 0.0, 2000.0, 2000.0);
        let ground = |moving| {
            charts(&app, 0, &view, &Palette::LIGHT, &Fixed, moving)
                .into_iter()
                .find_map(|op| match op {
                    Op::Fill { rect, .. } => Some(rect),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(ground(None), frame);
        assert_eq!(ground(Some((0, 30.0, 40.0))), frame.offset(30.0, 40.0));
        let (x, y) = moved_to(frame, 72.0 - frame.x, -1000.0);
        assert!((grind_sheet::style::length_mm(&x).unwrap() - 25.4).abs() < 0.01);
        assert_eq!(
            grind_sheet::style::length_mm(&y),
            Some(0.0),
            "never above row 1"
        );
    }
}
