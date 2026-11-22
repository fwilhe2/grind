// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where `chart_paint` puts a bar chart's category names, and what a point on a chart hits.

use grind_core::layout::Metrics;
use grind_core::style::TextStyle;
use grind_sheet::chart::Series;
use grind_sheet::chart_paint::{self, Colours, Mark, Picture, Rect, Target};
use grind_sheet::{Chart, ChartData, ChartKind};

/// Six units a letter, twelve a line: near enough a 9pt face in pixels.
struct Letters;

impl Metrics for Letters {
    fn advances(&self, text: &str, _style: &TextStyle, out: &mut Vec<f32>) {
        out.extend((1..=text.chars().count()).map(|n| n as f32 * 6.0));
    }

    fn line_height(&self, _style: &TextStyle) -> f32 {
        12.0
    }
}

fn colours() -> Colours {
    Colours {
        page: (255, 255, 255),
        ink: (0, 0, 0),
        grid: (200, 200, 200),
        accent: (0, 0, 255),
        dark: false,
    }
}

fn budget() -> (Chart, ChartData) {
    let mut chart = Chart::new(
        ChartKind::Bar,
        "0cm".into(),
        "0cm".into(),
        "10cm".into(),
        "6cm".into(),
    );
    chart.series = ["B2:B7", "C2:C7"]
        .map(|values| Series {
            values: values.into(),
            label: None,
            color: None,
            point_colors: Vec::new(),
            name: None,
            kind: None,
        })
        .into();
    let names = [
        "Housing",
        "Groceries",
        "Transport",
        "Utilities",
        "Leisure",
        "Savings",
    ];
    let data = ChartData {
        kind: ChartKind::Bar,
        categories: names.iter().map(|n| (*n).to_owned()).collect(),
        series: vec![
            (
                "Budgeted".into(),
                vec![2250.0, 500.0, 220.0, 260.0, 80.0, 400.0],
            ),
            (
                "Actual".into(),
                vec![1825.0, 612.0, 180.0, 238.0, 0.0, 400.0],
            ),
        ],
        value_format: None,
        kinds: Vec::new(),
        stacked: false,
    };
    (chart, data)
}

fn draw(width: f64, height: f64) -> (Picture, ChartData) {
    let (chart, data) = budget();
    let picture = chart_paint::picture(
        &chart,
        &data,
        Rect::new(0.0, 0.0, width, height),
        &colours(),
        &Letters,
    );
    (picture, data)
}

/// The plot's run along the baseline, and every text on the lowest line: the category names.
fn names_and_plot(picture: &Picture) -> (Vec<(f64, f64, String)>, f64, f64) {
    let plot = picture.plot.expect("a plot");
    let (x, w) = (plot.x, plot.w);
    let texts: Vec<(f64, f64, &str)> = picture
        .marks
        .iter()
        .filter_map(|mark| match mark {
            Mark::Text { x, top, text, .. } => Some((*x, *top, text.as_str())),
            _ => None,
        })
        .collect();
    let lowest = texts.iter().map(|t| t.1).fold(f64::NEG_INFINITY, f64::max);
    let names = texts
        .into_iter()
        .filter(|t| t.1 == lowest)
        .map(|(left, _, text)| {
            (
                left,
                left + chart_paint::width(&Letters, text, &TextStyle::default()),
                text.to_owned(),
            )
        })
        .collect();
    (names, x, w)
}

/// Names that fit their bands stay level, whole and centred under their own bars.
#[test]
fn names_that_fit_stay_level_under_their_own_bars() {
    let (picture, data) = draw(600.0, 200.0);
    let (names, plot_x, plot_w) = names_and_plot(&picture);
    assert_eq!(names.len(), 6, "{names:?}");
    let step = plot_w / 6.0;
    for (i, (left, right, text)) in names.iter().enumerate() {
        let centre = plot_x + (i as f64 + 0.5) * step;
        assert!(
            ((left + right) / 2.0 - centre).abs() < 0.01,
            "{text} is off its band"
        );
        assert_eq!(text, &data.categories[i]);
    }
    assert!(
        picture
            .marks
            .iter()
            .all(|mark| !matches!(mark, Mark::Text { turn, .. } if *turn != 0.0))
    );
}

/// Names wider than their bands are turned rather than cut, each whole and ending under its own
/// bars.
#[test]
fn names_too_wide_for_their_bands_are_turned_whole() {
    let (picture, data) = draw(330.0, 200.0);
    let (_, plot_x, plot_w) = names_and_plot(&picture);
    let turned: Vec<(f64, &str)> = picture
        .marks
        .iter()
        .filter_map(|mark| match mark {
            Mark::Text { x, text, turn, .. } if *turn == chart_paint::TURN_DEGREES => {
                Some((*x, text.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(turned.len(), 6, "{turned:?}");
    let step = plot_w / 6.0;
    let s = std::f64::consts::FRAC_1_SQRT_2;
    for (i, (x, text)) in turned.iter().enumerate() {
        assert_eq!(*text, data.categories[i]);
        let w = chart_paint::width(&Letters, text, &TextStyle::default());
        let end = x + s * (w + 12.0 / 2.0);
        let centre = plot_x + (i as f64 + 0.5) * step;
        assert!(
            (end - centre).abs() < 0.01,
            "{text} ends at {end}, not {centre}"
        );
    }
}

/// Turned names closer than a line apart would overlap, so some are left out; their room is
/// capped, so a very long name cannot take the plot.
#[test]
fn turned_names_keep_a_line_apart_and_a_share_of_the_height() {
    let names = chart_paint::category_names(300.0, 10.0, 12.0, 200.0, 4.0, 4.0);
    assert!(names.turned);
    assert_eq!(names.every, 2, "10 apart turned is 7 across; a line is 12");
    assert!(names.depth <= 200.0 * 0.3 + 4.0 + 0.01, "{names:?}");
    let level = chart_paint::category_names(30.0, 50.0, 12.0, 200.0, 4.0, 4.0);
    assert_eq!((level.turned, level.every, level.depth), (false, 1, 16.0));
}

#[test]
fn shorten_keeps_what_fits_and_gives_up_on_nothing() {
    let width = |t: &str| t.chars().count() as f64;
    assert_eq!(
        chart_paint::shorten("Housing", 7.0, width).as_deref(),
        Some("Housing")
    );
    assert_eq!(
        chart_paint::shorten("Housing", 4.0, width).as_deref(),
        Some("Hou…")
    );
    assert_eq!(
        chart_paint::shorten("Ab cd", 4.0, width).as_deref(),
        Some("Ab…")
    );
    assert_eq!(chart_paint::shorten("Housing", 1.0, width), None);
}

/// A point on a bar is that bar, and says what it is; a point between groups is nothing.
#[test]
fn a_bar_is_hit_where_it_is_drawn_and_named() {
    let (picture, data) = draw(600.0, 200.0);
    let plot = picture.plot.expect("a plot");
    let band = plot.w / 6.0;
    // Housing's Budgeted bar: the first of the first group, the tallest bar on the chart.
    let (x, y) = (plot.x + band / 2.0 - 2.0, plot.bottom() - 2.0);
    assert_eq!(
        picture.target_at(x, y, &data),
        Some(Target::Bar {
            series: 0,
            point: 0
        })
    );
    assert_eq!(
        picture.describe(x, y, &data).as_deref(),
        Some("Budgeted · Housing: 2250")
    );
    assert_eq!(
        picture.target_at(plot.x + 1.0, plot.bottom() - 2.0, &data),
        None
    );
}

/// No bar is wider than its maximum, and each is drawn as a shape — rounded at its data end.
#[test]
fn bars_are_narrow_shapes_however_wide_the_chart() {
    let (picture, _) = draw(1200.0, 200.0);
    let bars: Vec<f64> = picture
        .marks
        .iter()
        .filter_map(|mark| match mark {
            Mark::Path {
                points,
                fill: Some(_),
                stroke: None,
            } if points.len() > 4 => {
                let xs = points.iter().map(|p| p.0);
                Some(xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        bars.len(),
        11,
        "twelve bars, and a zero draws nothing: {bars:?}"
    );
    assert!(bars.iter().all(|w| *w <= 24.0 + 1e-9), "{bars:?}");
}

/// A legend entry names the series it is the colour of, before anything drawn under it.
#[test]
fn a_legend_entry_is_its_series() {
    let (mut chart, data) = budget();
    chart.legend = Some(grind_sheet::ChartLegend::Bottom);
    let picture = chart_paint::picture(
        &chart,
        &data,
        Rect::new(0.0, 0.0, 600.0, 200.0),
        &colours(),
        &Letters,
    );
    let actual = picture
        .marks
        .iter()
        .find_map(|mark| match mark {
            Mark::Text { x, top, text, .. } if text == "Actual" => Some((*x, *top)),
            _ => None,
        })
        .expect("the legend names Actual");
    assert_eq!(
        picture.target_at(actual.0 + 3.0, actual.1 + 3.0, &data),
        Some(Target::Legend {
            series: 1,
            point: None
        })
    );
    assert_eq!(
        picture.describe(actual.0 + 3.0, actual.1 + 3.0, &data),
        None
    );
}

/// A point on a line is that series, named at the category nearest it; a slice is its point.
#[test]
fn a_line_and_a_slice_are_hit_and_named() {
    let (mut chart, mut data) = budget();
    chart.kind = ChartKind::Line;
    data.kind = ChartKind::Line;
    let frame = Rect::new(0.0, 0.0, 600.0, 200.0);
    let picture = chart_paint::picture(&chart, &data, frame, &colours(), &Letters);
    let line = picture
        .marks
        .iter()
        .find_map(|mark| match mark {
            Mark::Path {
                points, fill: None, ..
            } => Some(points.clone()),
            _ => None,
        })
        .expect("a line");
    let (x, y) = line[2];
    assert_eq!(
        picture.target_at(x, y + 1.0, &data),
        Some(Target::Line { series: 0 })
    );
    assert_eq!(
        picture.describe(x, y + 1.0, &data).as_deref(),
        Some("Budgeted · Transport: 220")
    );

    chart.kind = ChartKind::Pie;
    data.kind = ChartKind::Pie;
    let picture = chart_paint::picture(&chart, &data, frame, &colours(), &Letters);
    let plot = picture.plot.expect("a plot");
    // Twelve o'clock, a little way in: the first slice, Housing's.
    let (cx, cy) = (plot.x + plot.w / 2.0, plot.y + plot.h / 2.0);
    let target = picture.target_at(cx + 1.0, cy - plot.h / 4.0, &data);
    assert_eq!(target, Some(Target::Slice { point: 0 }));
    assert_eq!(target.map(Target::mark), Some((0, Some(0))));
    assert_eq!(
        picture.target_at(plot.x + 1.0, plot.y + 1.0, &data),
        None,
        "outside the circle"
    );
}

/// A stacked area chart's bands sit on one another and the axis reaches the top of the stack; a
/// line among its series is drawn at its own values, over the bands — as LibreOffice draws one
/// (`doc/chart-format.md`, Area).
#[test]
fn a_stacked_area_stacks_its_bands_and_leaves_a_line_at_its_own_values() {
    let mut chart = Chart::new(
        ChartKind::Area,
        "0cm".into(),
        "0cm".into(),
        "10cm".into(),
        "6cm".into(),
    );
    chart.stacked = true;
    let series = |kind| Series {
        values: "B1:B3".into(),
        label: None,
        color: None,
        point_colors: Vec::new(),
        name: None,
        kind,
    };
    chart.series = vec![series(None), series(None), series(Some(ChartKind::Line))];
    let data = ChartData {
        kind: ChartKind::Area,
        categories: vec!["1".into(), "2".into(), "3".into()],
        series: vec![
            ("Base".into(), vec![10.0; 3]),
            ("Band".into(), vec![10.0; 3]),
            ("Median".into(), vec![15.0; 3]),
        ],
        value_format: None,
        kinds: vec![ChartKind::Area, ChartKind::Area, ChartKind::Line],
        stacked: true,
    };
    let picture = chart_paint::picture(
        &chart,
        &data,
        Rect::new(0.0, 0.0, 400.0, 300.0),
        &colours(),
        &Letters,
    );
    let plot = picture.plot.expect("a plot");
    // The stack's top, 20, is the axis' top.
    let y = |value: f64| plot.bottom() - value / 20.0 * plot.h;
    let paths: Vec<_> = picture
        .marks
        .iter()
        .filter_map(|mark| match mark {
            Mark::Path {
                points,
                fill,
                stroke,
            } => Some((points.clone(), fill.is_some(), stroke.is_some())),
            _ => None,
        })
        .collect();
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let top = |points: &[(f64, f64)]| points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let bottom = |points: &[(f64, f64)]| points.iter().map(|p| p.1).fold(0.0, f64::max);
    let bands: Vec<_> = paths.iter().filter(|p| p.1).collect();
    assert_eq!(bands.len(), 2, "{paths:?}");
    assert!(close(top(&bands[0].0), y(10.0)) && close(bottom(&bands[0].0), y(0.0)));
    assert!(close(top(&bands[1].0), y(20.0)) && close(bottom(&bands[1].0), y(10.0)));
    let line = paths.iter().find(|p| !p.1 && p.2).expect("the line");
    assert!(line.0.iter().all(|p| close(p.1, y(15.0))), "{line:?}");
    // Drawn after the bands, so it is on top of them.
    let order: Vec<bool> = paths.iter().map(|p| p.1).collect();
    assert_eq!(order.last(), Some(&false));
    // A band is hit where it is drawn, and a pointer over it names the series.
    let (mx, my) = (plot.x + plot.w / 2.0, y(17.0));
    assert_eq!(
        picture.target_at(mx, my, &data),
        Some(Target::Area { series: 1 })
    );
}
