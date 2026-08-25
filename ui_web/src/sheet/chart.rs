// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart, as SVG.
//!
//! The third renderer of `doc/chart-format.md`'s three shapes, after the GTK shell's
//! `snapshot()` and the writer's own `chart:` XML. It draws from
//! [`grind_sheet::ChartData`] and [`grind_sheet::Chart`] exactly as the other two do, and —
//! the part that matters — scales the plot against [`grind_sheet::axis_ticks`] rather than
//! against the tallest bar, so a chart has the same axis in a browser as it has in a window.
//! That function lives in the core for this reason (`doc/chart-format.md`, "Where the scale
//! lives").
//!
//! SVG rather than a `<canvas>`: the marks are elements, so the browser's own zoom, text
//! rendering, printing and copy-the-picture all work without this shell implementing any of
//! them, and a colour the document chose is an attribute rather than a paint call.
//!
//! **Read-only, for now.** The GTK shell lets a click on a bar assign it a colour; here a
//! chart is a picture. `doc/chart-format.md`'s shell section names that gap.

use grind_sheet::{Chart, ChartAxis, ChartData, ChartKind, axis_ticks, effective_color};

/// How far the plot is inset from the frame, in the SVG's own units (which are CSS pixels,
/// since the `<svg>` is sized in them and its `viewBox` matches).
const INSET: f64 = 6.0;
const GROUP_GAP: f64 = 0.2;
/// Room for an axis title — one line of text, the same fixed band the GTK painter reserves.
const TITLE_SPACE: f64 = 16.0;
/// One line of tick text, and the gap between a tick label and the plot.
const TICK_H: f64 = 12.0;
const TICK_GAP: f64 = 4.0;
/// How wide one character of tick text is taken to be.
///
/// Estimated rather than measured, unlike the GTK painter's Pango pass: nothing here
/// hit-tests against the plot, so a gutter a few pixels out costs a few pixels of plot and
/// nothing else. Measuring would mean laying the text out before deciding where to put it.
const TICK_CHAR_W: f64 = 6.2;

/// The whole `<svg>` for one chart, as a string of markup.
///
/// A string rather than a tree of `createElementNS` calls: an SVG is written once per repaint
/// and never edited in place, `innerHTML` is one call to the same parser the page itself came
/// from, and every value that reaches it here is a number or a colour this build produced.
pub fn svg(chart: &Chart, data: &ChartData, w: f64, h: f64) -> String {
    let mut out = format!(
        "<svg viewBox=\"0 0 {w:.1} {h:.1}\" preserveAspectRatio=\"none\" \
         xmlns=\"http://www.w3.org/2000/svg\">"
    );
    let ticks = axis_ticks(max_value(data));
    let (x_axis, y_axis) = axes(chart, data);
    let plot = plot_rect(&x_axis, &y_axis, &ticks, w, h);

    if plot.2 > 0.0 && plot.3 > 0.0 {
        grid(&mut out, &x_axis, &y_axis, data, &ticks, plot);
        match data.kind {
            ChartKind::Bar => bars(&mut out, chart, data, &ticks, plot),
            ChartKind::Line => lines(&mut out, chart, data, &ticks, plot),
            ChartKind::Pie => pie(&mut out, chart, data, plot),
        }
        ticks_text(&mut out, &x_axis, &y_axis, data, &ticks, plot);
    }
    titles(&mut out, &x_axis, &y_axis, w, h);
    out.push_str("</svg>");
    out
}

/// A pie has no axes, so nothing an axis carries costs it any of its circle — the same rule
/// the GTK painter applies, stated once in each renderer because it is about the *picture*.
fn axes(chart: &Chart, data: &ChartData) -> (ChartAxis, ChartAxis) {
    match data.kind {
        ChartKind::Pie => (ChartAxis::bare(), ChartAxis::bare()),
        _ => (chart.x_axis.clone(), chart.y_axis.clone()),
    }
}

/// `(x, y, w, h)` of the plot inside the frame, once the axes have taken their room.
type Plot = (f64, f64, f64, f64);

fn plot_rect(
    x_axis: &ChartAxis,
    y_axis: &ChartAxis,
    ticks: &grind_sheet::Ticks,
    w: f64,
    h: f64,
) -> Plot {
    let widest = ticks
        .values
        .iter()
        .map(|value| ticks.label(*value).chars().count())
        .max()
        .unwrap_or(1) as f64;

    let mut left = INSET;
    if y_axis.label.is_some() {
        left += TITLE_SPACE;
        if y_axis.tick_labels {
            left += TICK_GAP;
        }
    }
    if y_axis.tick_labels {
        left += widest * TICK_CHAR_W + TICK_GAP;
    }
    let mut bottom = INSET;
    if x_axis.label.is_some() {
        bottom += TITLE_SPACE;
    }
    if x_axis.tick_labels {
        bottom += TICK_H + TICK_GAP;
    }
    // The topmost tick label is centred on the plot's own top edge, so half of it is above.
    let top = INSET
        + match y_axis.tick_labels {
            true => TICK_H / 2.0,
            false => 0.0,
        };
    (
        left,
        top,
        (w - left - INSET).max(0.0),
        (h - bottom - top).max(0.0),
    )
}

fn max_value(data: &ChartData) -> f64 {
    data.series
        .iter()
        .flat_map(|(_, values)| values.iter().copied())
        .fold(0.0_f64, f64::max)
        .max(1.0)
}

fn category_count(data: &ChartData) -> usize {
    data.categories.len().max(
        data.series
            .iter()
            .map(|(_, values)| values.len())
            .max()
            .unwrap_or(0),
    )
}

/// Where a value sits vertically — the one place the scale is applied.
fn value_y(value: f64, ticks: &grind_sheet::Ticks, plot: Plot) -> f64 {
    plot.1 + plot.3 - (value.max(0.0) / ticks.max()) * plot.3
}

/// Where each category's tick sits horizontally: the centre of a bar's group, or a line's own
/// point. Shared by the gridlines and the tick labels so the two line up.
fn category_x(data: &ChartData, plot: Plot) -> Vec<f64> {
    let count = category_count(data);
    if count == 0 {
        return Vec::new();
    }
    match data.kind {
        ChartKind::Line if count > 1 => {
            let step = plot.2 / (count - 1) as f64;
            (0..count).map(|i| plot.0 + i as f64 * step).collect()
        }
        _ => {
            let step = plot.2 / count as f64;
            (0..count)
                .map(|i| plot.0 + (i as f64 + 0.5) * step)
                .collect()
        }
    }
}

fn grid(
    out: &mut String,
    x_axis: &ChartAxis,
    y_axis: &ChartAxis,
    data: &ChartData,
    ticks: &grind_sheet::Ticks,
    plot: Plot,
) {
    if y_axis.gridlines {
        for value in &ticks.values {
            let y = value_y(*value, ticks, plot);
            line(out, plot.0, y, plot.0 + plot.2, y);
        }
    }
    if x_axis.gridlines {
        for x in category_x(data, plot) {
            line(out, x, plot.1, x, plot.1 + plot.3);
        }
    }
}

fn line(out: &mut String, x1: f64, y1: f64, x2: f64, y2: f64) {
    out.push_str(&format!(
        "<line class=\"axis\" x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\"/>"
    ));
}

fn bars(out: &mut String, chart: &Chart, data: &ChartData, ticks: &grind_sheet::Ticks, plot: Plot) {
    let count = category_count(data);
    if count == 0 {
        return;
    }
    let series = data.series.len().max(1);
    let group = plot.2 / count as f64;
    let width = (group * (1.0 - GROUP_GAP) / series as f64).max(1.0);
    let gap = (group - width * series as f64) / (series as f64 + 1.0);
    for cat in 0..count {
        for (s, (_, values)) in data.series.iter().enumerate() {
            let Some(value) = values.get(cat) else {
                continue;
            };
            let y = value_y(*value, ticks, plot);
            let x = plot.0 + cat as f64 * group + gap + s as f64 * (width + gap);
            let height = plot.1 + plot.3 - y;
            out.push_str(&format!(
                "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{width:.1}\" height=\"{height:.1}\" \
                 fill=\"{}\"/>",
                effective_color(chart, s, Some(cat))
            ));
        }
    }
}

fn lines(
    out: &mut String,
    chart: &Chart,
    data: &ChartData,
    ticks: &grind_sheet::Ticks,
    plot: Plot,
) {
    let count = category_count(data);
    if count < 2 {
        return;
    }
    let step = plot.2 / (count - 1) as f64;
    for (s, (_, values)) in data.series.iter().enumerate() {
        if values.len() < 2 {
            continue;
        }
        let points: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                format!(
                    "{:.1},{:.1}",
                    plot.0 + i as f64 * step,
                    value_y(*value, ticks, plot)
                )
            })
            .collect();
        out.push_str(&format!(
            "<polyline fill=\"none\" stroke-width=\"2\" stroke-linejoin=\"round\" \
             stroke-linecap=\"round\" stroke=\"{}\" points=\"{}\"/>",
            effective_color(chart, s, None),
            points.join(" ")
        ));
    }
}

fn pie(out: &mut String, chart: &Chart, data: &ChartData, plot: Plot) {
    let Some((_, values)) = data.series.first() else {
        return;
    };
    let total: f64 = values.iter().map(|v| v.max(0.0)).sum();
    if total <= 0.0 {
        return;
    }
    let (cx, cy) = (plot.0 + plot.2 / 2.0, plot.1 + plot.3 / 2.0);
    let r = (plot.2.min(plot.3) / 2.0).max(1.0);
    let mut angle = -std::f64::consts::FRAC_PI_2;
    for (i, value) in values.iter().enumerate() {
        let value = value.max(0.0);
        if value <= 0.0 {
            continue;
        }
        let sweep = (value / total) * std::f64::consts::TAU;
        let end = angle + sweep;
        // SVG's own arc, rather than the fan of segments the GTK painter draws: `gsk` has no
        // arc primitive and this does.
        let large = i32::from(sweep > std::f64::consts::PI);
        out.push_str(&format!(
            "<path fill=\"{}\" d=\"M{cx:.1} {cy:.1} L{:.2} {:.2} A{r:.2} {r:.2} 0 {large} 1 {:.2} \
             {:.2} Z\"/>",
            effective_color(chart, 0, Some(i)),
            cx + r * angle.cos(),
            cy + r * angle.sin(),
            cx + r * end.cos(),
            cy + r * end.sin(),
        ));
        angle = end;
    }
}

fn ticks_text(
    out: &mut String,
    x_axis: &ChartAxis,
    y_axis: &ChartAxis,
    data: &ChartData,
    ticks: &grind_sheet::Ticks,
    plot: Plot,
) {
    if y_axis.tick_labels {
        for value in &ticks.values {
            let y = value_y(*value, ticks, plot);
            out.push_str(&format!(
                "<text class=\"tick\" text-anchor=\"end\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
                plot.0 - TICK_GAP,
                y + TICK_H / 3.0,
                escape(&ticks.label(*value))
            ));
        }
    }
    if x_axis.tick_labels {
        // A label that would overlap the one before it is dropped rather than drawn over it —
        // the same rule as the GTK painter, with the width estimated rather than measured.
        let mut drawn_to = f64::NEG_INFINITY;
        for (i, x) in category_x(data, plot).into_iter().enumerate() {
            let Some(text) = data.categories.get(i).filter(|t| !t.is_empty()) else {
                continue;
            };
            let half = text.chars().count() as f64 * TICK_CHAR_W / 2.0;
            if x - half < drawn_to + TICK_GAP {
                continue;
            }
            drawn_to = x + half;
            out.push_str(&format!(
                "<text class=\"tick\" text-anchor=\"middle\" x=\"{x:.1}\" y=\"{:.1}\">{}</text>",
                plot.1 + plot.3 + TICK_GAP + TICK_H * 0.8,
                escape(text)
            ));
        }
    }
}

fn titles(out: &mut String, x_axis: &ChartAxis, y_axis: &ChartAxis, w: f64, h: f64) {
    if let Some(text) = &x_axis.label {
        out.push_str(&format!(
            "<text class=\"axis-title\" text-anchor=\"middle\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
            w / 2.0,
            h - INSET,
            escape(text)
        ));
    }
    if let Some(text) = &y_axis.label {
        // Rotated about its own anchor, which is the middle of the left edge.
        out.push_str(&format!(
            "<text class=\"axis-title\" text-anchor=\"middle\" transform=\"translate({:.1} {:.1}) \
             rotate(-90)\">{}</text>",
            INSET + TITLE_SPACE * 0.6,
            h / 2.0,
            escape(text)
        ));
    }
}

/// The five characters that are not text inside markup. A category's name comes out of a
/// document, so it can be anything at all — including `<`.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(kind: ChartKind) -> ChartData {
        ChartData {
            kind,
            categories: vec!["CDU".into(), "SPD".into()],
            series: vec![("Votes".into(), vec![90.0, 60.0])],
        }
    }

    /// A chart whose series match the data above — which is what the render path always has,
    /// since both come from the same `App` call in the same frame.
    fn chart(kind: ChartKind) -> Chart {
        let mut chart = Chart::new(kind, "0cm".into(), "0cm".into(), "8cm".into(), "5cm".into());
        chart.series = vec![grind_sheet::ChartSeries {
            values: "Sheet1.B2:Sheet1.B3".into(),
            label: None,
            color: None,
            point_colors: Vec::new(),
        }];
        chart
    }

    #[test]
    fn every_kind_draws_its_own_mark() {
        assert!(svg(&chart(ChartKind::Bar), &data(ChartKind::Bar), 300.0, 200.0).contains("<rect"));
        assert!(
            svg(
                &chart(ChartKind::Line),
                &data(ChartKind::Line),
                300.0,
                200.0
            )
            .contains("<polyline")
        );
        assert!(svg(&chart(ChartKind::Pie), &data(ChartKind::Pie), 300.0, 200.0).contains("<path"));
    }

    /// The scale is the core's, so the topmost gridline is the top of the plot rather than
    /// the tallest bar — the same property the GTK painter is tested for.
    #[test]
    fn the_plot_is_scaled_to_the_top_tick() {
        let ticks = axis_ticks(90.0);
        assert_eq!(ticks.max(), 100.0);
        let plot = (10.0, 10.0, 100.0, 100.0);
        assert_eq!(value_y(100.0, &ticks, plot), plot.1);
        assert!(value_y(90.0, &ticks, plot) > plot.1);
    }

    #[test]
    fn a_pie_keeps_its_whole_frame_whatever_its_axes_say() {
        let mut chart = chart(ChartKind::Pie);
        chart.x_axis.label = Some("Party".into());
        chart.y_axis.gridlines = true;
        let data = data(ChartKind::Pie);
        let (x, y) = (
            plot_rect(
                &axes(&chart, &data).0,
                &axes(&chart, &data).1,
                &axis_ticks(90.0),
                300.0,
                200.0,
            ),
            plot_rect(
                &ChartAxis::bare(),
                &ChartAxis::bare(),
                &axis_ticks(90.0),
                300.0,
                200.0,
            ),
        );
        assert_eq!(x, y);
    }

    /// A category called `<script>` is text, and has to leave as text.
    #[test]
    fn a_categorys_own_name_is_escaped() {
        let mut data = data(ChartKind::Bar);
        data.categories[0] = "<b>&".into();
        let mut chart = chart(ChartKind::Bar);
        chart.x_axis.tick_labels = true;
        let svg = svg(&chart, &data, 300.0, 200.0);
        assert!(svg.contains("&lt;b&gt;&amp;"), "{svg}");
        assert!(!svg.contains("<b>"), "{svg}");
    }

    #[test]
    fn an_axis_that_carries_nothing_gives_the_plot_the_whole_frame() {
        let ticks = axis_ticks(90.0);
        let bare = plot_rect(&ChartAxis::bare(), &ChartAxis::bare(), &ticks, 300.0, 200.0);
        let dressed = plot_rect(
            &ChartAxis::default(),
            &ChartAxis {
                label: Some("Votes".into()),
                ..ChartAxis::default()
            },
            &ticks,
            300.0,
            200.0,
        );
        assert!(dressed.2 < bare.2, "the y scale and title take width");
        assert!(dressed.3 < bare.3, "the categories take height");
    }
}
