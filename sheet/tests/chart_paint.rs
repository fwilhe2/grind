// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where `chart_paint::draw` puts a bar chart's category names.

use grind_core::color::Rgb;
use grind_core::layout::Metrics;
use grind_core::style::TextStyle;
use grind_sheet::chart::Series;
use grind_sheet::chart_paint::{self, Colours, Mark, Rect};
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

const AXIS: Rgb = (1, 2, 3);

fn colours() -> Colours {
    Colours {
        page: (255, 255, 255),
        ink: (0, 0, 0),
        grid: (200, 200, 200),
        header_ink: AXIS,
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
    };
    (chart, data)
}

/// The plot's run along the baseline, and every text on the lowest line: the category names.
fn names_and_plot(marks: &[Mark]) -> (Vec<(f64, f64, String)>, f64, f64) {
    let (x, w) = marks
        .iter()
        .find_map(|mark| match mark {
            Mark::Fill { rect, color } if *color == AXIS && rect.h == 1.0 => Some((rect.x, rect.w)),
            _ => None,
        })
        .expect("a baseline");
    let texts: Vec<(f64, f64, &str)> = marks
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
    let (chart, data) = budget();
    let marks = chart_paint::draw(
        &chart,
        &data,
        Rect::new(0.0, 0.0, 600.0, 200.0),
        &colours(),
        &Letters,
    );
    let (names, plot_x, plot_w) = names_and_plot(&marks);
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
        marks
            .iter()
            .all(|mark| !matches!(mark, Mark::Text { turn, .. } if *turn != 0.0))
    );
}

/// Names wider than their bands are turned rather than cut, each whole and ending under its own
/// bars.
#[test]
fn names_too_wide_for_their_bands_are_turned_whole() {
    let (chart, data) = budget();
    let marks = chart_paint::draw(
        &chart,
        &data,
        Rect::new(0.0, 0.0, 330.0, 200.0),
        &colours(),
        &Letters,
    );
    let (_, plot_x, plot_w) = names_and_plot(&marks);
    let turned: Vec<(f64, &str)> = marks
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
