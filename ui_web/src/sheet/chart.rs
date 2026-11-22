// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart, as SVG: [`grind_sheet::chart_paint`]'s marks as elements, so the browser's zoom, text,
//! printing and copy-the-picture work for free. Where every mark goes is the core's; text is
//! measured by estimate, since the markup is written before anything is laid out.

use std::fmt::Write as _;

use grind_core::color::Rgb;
use grind_core::layout::Metrics;
use grind_core::style::TextStyle;
use grind_sheet::chart_paint::{self, Colours, Mark};
use grind_sheet::{Chart, ChartData};

use crate::ink;

/// The size chart text is set at, in CSS pixels, and how wide one character of it is taken to be.
const FONT_PX: f64 = 12.0;
const CHAR_W: f64 = 6.6;
const LINE_H: f64 = 15.0;

/// Chart text by estimate: every character [`CHAR_W`] wide, a bold one a little wider.
struct Estimate;

impl Metrics for Estimate {
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
        let bold = style.font_weight.as_deref() == Some("bold");
        let w = if bold { CHAR_W * 1.08 } else { CHAR_W };
        let mut x = 0.0;
        for c in text.chars() {
            x += crate::sheet::layout::text_cells(c.encode_utf8(&mut [0; 4])) as f64 * w;
            out.push(x as f32);
        }
    }

    fn line_height(&self, _: &TextStyle) -> f32 {
        LINE_H as f32
    }
}

/// The page's colours in the scheme the browser paints: the hairlines are the stylesheet's
/// `--line`.
fn colours(dark: bool) -> Colours {
    let (page, text) = if dark { ink::DARK } else { ink::LIGHT };
    Colours {
        page,
        ink: text,
        grid: grind_core::color::mix(page, text, 0.2),
        accent: (0x1c, 0x71, 0xd8),
        dark,
    }
}

/// The whole `<svg>` for one chart, as markup: every value reaching it is a number, a colour or
/// escaped text.
pub fn svg(chart: &Chart, data: &ChartData, w: f64, h: f64) -> String {
    svg_in(chart, data, w, h, ink::page_is_dark())
}

fn svg_in(chart: &Chart, data: &ChartData, w: f64, h: f64, dark: bool) -> String {
    let mut out = format!(
        "<svg viewBox=\"0 0 {w:.1} {h:.1}\" preserveAspectRatio=\"none\" \
         xmlns=\"http://www.w3.org/2000/svg\" font-size=\"{FONT_PX}\">"
    );
    let frame = chart_paint::Rect::new(0.0, 0.0, w, h);
    for mark in chart_paint::draw(chart, data, frame, &colours(dark), &Estimate) {
        put(&mut out, mark);
    }
    out.push_str("</svg>");
    out
}

fn put(out: &mut String, mark: Mark) {
    let color = |rgb: Rgb| ink::hex(rgb);
    let points = |points: &[(f64, f64)]| {
        points
            .iter()
            .map(|(x, y)| format!("{x:.1},{y:.1}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let _ = match mark {
        Mark::Fill { rect, color: c } => write!(
            out,
            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"{}\"/>",
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            color(c)
        ),
        Mark::Path {
            points: at,
            fill: Some(fill),
            stroke,
        } => {
            let stroke = stroke.map_or(String::new(), |(c, w)| {
                format!(" stroke=\"{}\" stroke-width=\"{w}\"", color(c))
            });
            write!(
                out,
                "<path d=\"M{}Z\" fill=\"{}\"{stroke}/>",
                points(&at).replace(' ', " L"),
                color(fill)
            )
        }
        Mark::Path {
            points: at,
            fill: None,
            stroke: Some((c, w)),
        } => write!(
            out,
            "<polyline points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{w}\" \
             stroke-linejoin=\"round\" stroke-linecap=\"round\"/>",
            points(&at),
            color(c)
        ),
        Mark::Path { .. } => Ok(()),
        Mark::Text {
            x,
            top,
            text,
            style,
            color: c,
            turn,
            ..
        } => {
            let weight = match style.font_weight.as_deref() {
                Some("bold") => " font-weight=\"bold\"",
                _ => "",
            };
            // The core turns counter-clockwise about the box's top-left corner; SVG clockwise.
            let turn = if turn == 0.0 {
                String::new()
            } else {
                format!(" transform=\"rotate({:.1} {x:.1} {top:.1})\"", -turn)
            };
            write!(
                out,
                "<text x=\"{x:.1}\" y=\"{top:.1}\" dominant-baseline=\"text-before-edge\" \
                 fill=\"{}\"{weight}{turn}>{}</text>",
                color(c),
                escape(&text)
            )
        }
    };
}

/// A chart's text is the document's, so it can be anything at all — including `<`.
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
    use grind_sheet::{ChartKind, ChartLegend};

    fn data(kind: ChartKind) -> ChartData {
        ChartData {
            kind,
            categories: vec!["CDU".into(), "SPD".into()],
            series: vec![("Votes".into(), vec![90.0, 60.0])],
            value_format: None,
            kinds: Vec::new(),
            stacked: false,
        }
    }

    fn chart(kind: ChartKind) -> Chart {
        let mut chart = Chart::new(kind, "0cm".into(), "0cm".into(), "8cm".into(), "5cm".into());
        chart.series = vec![grind_sheet::ChartSeries {
            values: "Sheet1.B2:Sheet1.B3".into(),
            label: None,
            color: None,
            point_colors: Vec::new(),
            name: None,
            kind: None,
        }];
        chart
    }

    #[test]
    fn every_kind_draws_its_own_mark() {
        let drawn = |kind| svg_in(&chart(kind), &data(kind), 300.0, 200.0, false);
        assert_eq!(drawn(ChartKind::Bar).matches("<path").count(), 2);
        assert!(drawn(ChartKind::Line).contains("<polyline"));
        assert_eq!(drawn(ChartKind::Pie).matches("<path").count(), 2);
    }

    /// A title and a legend are drawn, and a legend's words are escaped like every other piece of
    /// a document's text.
    #[test]
    fn a_title_and_a_legend_are_drawn_and_their_words_escaped() {
        let data = ChartData {
            kind: ChartKind::Bar,
            categories: vec!["a".into(), "b".into()],
            series: vec![
                ("Sales <net>".into(), vec![1.0, 2.0]),
                ("Costs".into(), vec![3.0, 4.0]),
            ],
            value_format: None,
            kinds: Vec::new(),
            stacked: false,
        };
        let mut dressed = chart(ChartKind::Bar);
        dressed.series.push(dressed.series[0].clone());
        dressed.title = Some("Q1".into());
        dressed.legend = Some(ChartLegend::End);
        let drawn = svg_in(&dressed, &data, 400.0, 200.0, false);
        assert!(drawn.contains("font-weight=\"bold\"") && drawn.contains(">Q1</text>"));
        assert!(drawn.contains("Sales &lt;net&gt;"));
        assert!(!drawn.contains("<net>"));
    }

    #[test]
    fn a_dark_page_draws_a_dark_card() {
        let drawn = svg_in(
            &chart(ChartKind::Bar),
            &data(ChartKind::Bar),
            300.0,
            200.0,
            true,
        );
        assert!(drawn.contains(&format!("fill=\"{}\"", ink::hex(ink::DARK.0))));
    }
}
