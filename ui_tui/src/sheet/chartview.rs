// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart **drawn in characters** — what `:charts` shows, so the terminal can see what it just
//! inserted rather than take the other windows' word for it (`doc/feature-matrix.md` §6).
//!
//! No geometry of the chart's own is used: the frame's position and size are ODF lengths for a
//! page and mean nothing in a terminal. What is drawn is what the chart *says* — its title, its
//! legend (where it asked for one), and its data: a bar chart as one block bar per value, a line as
//! a sparkline per series, a pie as each slice's share. Pure, so every line is a test.

use grind_sheet::chart::{Chart, ChartData, ChartKind, Legend};

const BAR: char = '█';
const STEPS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn name(data: &ChartData, i: usize) -> String {
    match data.series.get(i) {
        Some((label, _)) if !label.is_empty() => label.clone(),
        _ => format!("Series {}", i + 1),
    }
}

fn category(data: &ChartData, i: usize) -> String {
    data.categories
        .get(i)
        .filter(|c| !c.is_empty())
        .cloned()
        .unwrap_or_else(|| (i + 1).to_string())
}

fn number(n: f64) -> String {
    match n.fract() == 0.0 && n.abs() < 1e15 {
        true => format!("{n:.0}"),
        false => format!("{n:.2}"),
    }
}

/// A bar `width` cells at most, as long as `value` is of `max`. Anything not positive is no bar.
fn bar(value: f64, max: f64, width: usize) -> String {
    if max <= 0.0 || value <= 0.0 {
        return String::new();
    }
    let cells = ((value / max) * width as f64).round().max(1.0) as usize;
    BAR.to_string().repeat(cells.min(width))
}

/// One series as a sparkline: each value one block, tallest of the series the full one.
fn spark(values: &[f64]) -> String {
    let (low, high) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    values
        .iter()
        .map(|v| match high > low {
            true => STEPS[(((v - low) / (high - low)) * 7.0).round() as usize],
            false => STEPS[3],
        })
        .collect()
}

/// The chart as lines of text, at most `width` cells wide. Every line but the title is indented,
/// which is how `help.rs`'s pane tells a heading from a body.
pub fn lines(chart: &Chart, data: &ChartData, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    out.push(match chart.title.as_deref().filter(|t| !t.is_empty()) {
        Some(title) => title.to_owned(),
        None => format!(
            "{} chart",
            data.kind.class().rsplit(':').next().unwrap_or("")
        ),
    });
    let legend: Vec<String> = (0..data.series.len())
        .map(|i| format!("  {} {}", STEPS[(i * 3) % 8], name(data, i)))
        .collect();
    let legend_first = matches!(chart.legend, Some(Legend::Top | Legend::Start));
    if legend_first {
        out.extend(legend.iter().cloned());
    }
    out.push(String::new());

    let label_w = (0..data.categories.len().max(longest(data)))
        .map(|i| unicode_width::UnicodeWidthStr::width(category(data, i).as_str()))
        .max()
        .unwrap_or(0)
        .min(width / 3);
    let max = data
        .series
        .iter()
        .flat_map(|(_, v)| v.iter().copied())
        .fold(0.0, f64::max);
    match data.kind {
        ChartKind::Bar => {
            let room = width.saturating_sub(label_w + 12).max(4);
            for i in 0..longest(data) {
                for (s, (_, values)) in data.series.iter().enumerate() {
                    let Some(v) = values.get(i) else { continue };
                    let label = match s {
                        0 => category(data, i),
                        _ => String::new(),
                    };
                    out.push(format!(
                        "  {label:<label_w$} {}  {}",
                        bar(*v, max, room),
                        number(*v)
                    ));
                }
            }
        }
        ChartKind::Line => {
            for (s, (_, values)) in data.series.iter().enumerate() {
                let low = values.iter().copied().fold(f64::MAX, f64::min);
                let high = values.iter().copied().fold(f64::MIN, f64::max);
                out.push(format!("  {}", name(data, s)));
                out.push(format!(
                    "    {}   {} … {}",
                    spark(values),
                    number(low),
                    number(high)
                ));
            }
            if let Some((_, first)) = data.series.first() {
                let n = first.len();
                if n > 0 {
                    out.push(format!(
                        "  {} → {}",
                        category(data, 0),
                        category(data, n - 1)
                    ));
                }
            }
        }
        ChartKind::Pie => {
            let total: f64 = data
                .series
                .first()
                .map_or(0.0, |(_, v)| v.iter().filter(|v| **v > 0.0).sum());
            let room = width.saturating_sub(label_w + 12).max(4);
            if let Some((_, values)) = data.series.first() {
                for (i, v) in values.iter().enumerate() {
                    let share = match total > 0.0 {
                        true => v.max(0.0) / total * 100.0,
                        false => 0.0,
                    };
                    out.push(format!(
                        "  {:<label_w$} {} {share:.0}%",
                        category(data, i),
                        bar(share, 100.0, room)
                    ));
                }
            }
        }
    }
    if !legend_first && chart.legend.is_some() {
        out.push(String::new());
        out.extend(legend);
    }
    out
}

fn longest(data: &ChartData) -> usize {
    data.series.iter().map(|(_, v)| v.len()).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(kind: ChartKind) -> ChartData {
        ChartData {
            kind,
            categories: vec!["Q1".into(), "Q2".into()],
            series: vec![("Sales".into(), vec![10.0, 20.0])],
        }
    }

    fn chart(kind: ChartKind, title: Option<&str>, legend: Option<Legend>) -> Chart {
        let mut chart = Chart {
            kind,
            categories: None,
            series: Vec::new(),
            x: String::new(),
            y: String::new(),
            width: String::new(),
            height: String::new(),
            x_axis: Default::default(),
            y_axis: Default::default(),
            clockwise: true,
            title: None,
            legend: None,
        };
        chart.title = title.map(str::to_owned);
        chart.legend = legend;
        chart
    }

    #[test]
    fn a_bar_chart_is_a_bar_per_value_and_the_larger_is_longer() {
        let text = lines(
            &chart(ChartKind::Bar, Some("Takings"), None),
            &data(ChartKind::Bar),
            40,
        );
        assert_eq!(text[0], "Takings");
        let q1 = text.iter().find(|l| l.contains("Q1")).unwrap();
        let q2 = text.iter().find(|l| l.contains("Q2")).unwrap();
        assert!(
            q2.matches(BAR).count() > q1.matches(BAR).count(),
            "{text:?}"
        );
        assert!(q2.ends_with("20"), "{q2:?}");
    }

    #[test]
    fn a_pie_is_each_slices_share() {
        let text = lines(
            &chart(ChartKind::Pie, None, None),
            &data(ChartKind::Pie),
            40,
        );
        assert!(text.iter().any(|l| l.contains("33%")), "{text:?}");
        assert!(text.iter().any(|l| l.contains("67%")), "{text:?}");
    }

    #[test]
    fn a_line_is_a_sparkline_and_the_legend_goes_where_it_was_put() {
        let bottom = lines(
            &chart(ChartKind::Line, None, Some(Legend::Bottom)),
            &data(ChartKind::Line),
            40,
        );
        assert!(bottom.iter().any(|l| l.contains("▁█")), "{bottom:?}");
        assert!(bottom.last().unwrap().contains("Sales"), "{bottom:?}");
        let top = lines(
            &chart(ChartKind::Line, None, Some(Legend::Top)),
            &data(ChartKind::Line),
            40,
        );
        assert!(top[1].contains("Sales"), "{top:?}");
    }
}
