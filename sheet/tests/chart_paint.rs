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

/// Every category keeps a name, centred under its own band and shortened to fit it, so no name
/// is skipped and the one that shows is never under the neighbouring group.
#[test]
fn every_category_is_named_under_its_own_bars_and_shortened_to_fit() {
    let (chart, data) = budget();
    let marks = chart_paint::draw(
        &chart,
        &data,
        Rect::new(0.0, 0.0, 330.0, 200.0),
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
        assert!(right - left <= step, "{text} is wider than its band");
        assert!(
            data.categories[i].starts_with(text.trim_end_matches('…')),
            "{text}"
        );
    }
    assert!(names.iter().any(|(_, _, t)| t.ends_with('…')), "{names:?}");
    assert!(
        names.iter().any(|(_, _, t)| t == "Leisure"),
        "a name that fits stays whole"
    );
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
