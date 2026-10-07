// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Charts — the one decoration that may read any sheet, and so the last thing materialised.
//!
//! `doc/generator-spec.md` §7 used to list charts as a gap, with `grind sheet chart-add` as the
//! answer and "nobody hand-writes one" as the reason. Then somebody wanted a generated
//! spreadsheet *with a graph in it* to be the demo (`examples/loc/`), and a demo that ends in
//! three shell commands after the script is a demo of the shell, not of the script. So a
//! chart is sayable now, in exactly `grind sheet chart-add`'s vocabulary:
//!
//! ```rhai
//! let s = sheet("Sales");
//! // …rows…
//! s.chart(chart("line")
//!     .title("Sales by quarter")
//!     .categories("A2:A5")
//!     .series("B2:B5", "B1")
//!     .position("6cm", "0cm").size("14cm", "8cm"));
//! ```
//!
//! **Nothing here decides what a chart is.** The builder fills a `grind_sheet::ChartSpec` and
//! `App::add_chart` resolves its ranges exactly as it does for the CLI and the GNOME dialog —
//! an unqualified range means the sheet the chart is on, `Data.B2:B9` another one. The two
//! things a spec does not carry, the frame's place and a series' colour, go through the two
//! calls a shell would make for them, so a generated chart is one a person could have drawn.

use grind_core::style;
use grind_sheet::{App, ChartAxis, ChartKind, ChartLegend, ChartSpec};
use rhai::{Array, Engine, EvalAltResult};

use crate::hint::hint;
use crate::sheet::Sheet;

type Res<T> = Result<T, Box<EvalAltResult>>;

/// A chart as a script describes it — a request, resolved when the document exists.
///
/// A plain value rather than a shared handle like [`Sheet`]: every method returns a modified
/// copy, the way [`crate::sheet`]'s `Style` and `Format` do, because a chart is said once and
/// then handed to `s.chart(…)` rather than written to after it is placed.
#[derive(Clone)]
pub struct Chart(Request);

#[derive(Clone)]
struct Request {
    kind: ChartKind,
    title: Option<String>,
    categories: Option<String>,
    series: Vec<(String, Option<String>)>,
    /// `None` is "whatever a new chart gets" — `ChartSpec::default_legend`; `Some(None)` is a
    /// script that said `legend("none")`.
    legend: Option<Option<ChartLegend>>,
    x_label: Option<String>,
    y_label: Option<String>,
    gridlines: bool,
    x: String,
    y: String,
    width: String,
    height: String,
    /// ODF colours, already checked — one per series, or one per slice of a pie.
    colors: Vec<String>,
}

impl Chart {
    fn new(kind: &str) -> Res<Chart> {
        let kind = match kind {
            "bar" => ChartKind::Bar,
            "line" => ChartKind::Line,
            "pie" => ChartKind::Pie,
            other => return Err(format!("{other}: a chart is a bar, a line or a pie").into()),
        };
        Ok(Chart(Request {
            kind,
            title: None,
            categories: None,
            series: Vec::new(),
            legend: None,
            x_label: None,
            y_label: None,
            gridlines: false,
            // `App::add_chart`'s own defaults for a chart nobody placed.
            x: "0cm".to_owned(),
            y: "0cm".to_owned(),
            width: "10cm".to_owned(),
            height: "8cm".to_owned(),
            colors: Vec::new(),
        }))
    }

    /// A copy with one thing changed — see the type's comment for why not a shared handle.
    fn with(&self, f: impl FnOnce(&mut Request)) -> Chart {
        let mut out = self.clone();
        f(&mut out.0);
        out
    }

    fn with_res(&self, f: impl FnOnce(&mut Request) -> Res<()>) -> Res<Chart> {
        let mut out = self.clone();
        f(&mut out.0)?;
        Ok(out)
    }

    /// Add the chart to `sheet` of a finished document. Every sheet exists by now, so a range
    /// on any of them resolves.
    pub fn place(&self, app: &App, sheet: usize) -> Result<(), String> {
        let request = &self.0;
        if request.series.is_empty() {
            return Err("a chart needs at least one .series(…)".to_owned());
        }
        let mut spec = ChartSpec::new(request.kind);
        spec.title.clone_from(&request.title);
        spec.categories.clone_from(&request.categories);
        spec.series.clone_from(&request.series);
        spec.legend = match request.legend {
            Some(legend) => legend,
            None => spec.default_legend(),
        };
        spec.x_axis = ChartAxis {
            label: request.x_label.clone(),
            ..ChartAxis::default()
        };
        spec.y_axis = ChartAxis {
            label: request.y_label.clone(),
            gridlines: request.gridlines,
            ..ChartAxis::default()
        };
        let say = |e: grind_sheet::Error| e.to_string();
        app.add_chart(
            sheet,
            &spec,
            &request.x,
            &request.y,
            &request.width,
            &request.height,
        )
        .map_err(say)?;
        if request.colors.is_empty() {
            return Ok(());
        }
        // The colours are a *style*, not part of what a chart is — `chart-style`'s, and so
        // `set_chart_style`'s, read back from the chart just added.
        let charts = app.charts(sheet).map_err(say)?;
        let index = charts.len() - 1;
        let added = &charts[index];
        let mut series = added.series.clone();
        match request.kind {
            ChartKind::Pie => {
                if let Some(only) = series.first_mut() {
                    only.point_colors = request.colors.iter().cloned().map(Some).collect();
                }
            }
            _ => {
                for (one, color) in series.iter_mut().zip(&request.colors) {
                    one.color = Some(color.clone());
                }
            }
        }
        app.set_chart_style(
            sheet,
            index,
            added.x_axis.clone(),
            added.y_axis.clone(),
            series,
        )
        .map_err(say)
    }
}

/// Everything a script may say about a chart.
pub fn register(engine: &mut Engine) {
    engine.register_type_with_name::<Chart>("Chart");

    hint(
        engine,
        "chart",
        ["kind: string", "Chart"],
        [
            "/// A chart — `\"bar\"`, `\"line\"` or `\"pie\"` — not yet on any sheet.",
            "///",
            "/// Give it `.series(…)` and put it on a sheet with `s.chart(…)`. Ranges are",
            "/// addresses, `\"B2:B9\"` on the sheet it is put on or `\"Data.B2:B9\"` on another.",
        ],
        |kind: &str| -> Res<Chart> { Chart::new(kind) },
    );
    hint(
        engine,
        "chart",
        ["sheet: Sheet", "chart: Chart", "Sheet"],
        [
            "/// Put a chart on this sheet.",
            "///",
            "/// Placed after every sheet of the document has been filled, so it may chart",
            "/// cells on any of them.",
        ],
        |sheet: &mut Sheet, chart: Chart| {
            sheet.chart(chart);
            sheet.clone()
        },
    );
    hint(
        engine,
        "title",
        ["chart: Chart", "text: string", "Chart"],
        ["/// The chart's own title, drawn above it."],
        |c: &mut Chart, text: &str| c.with(|r| r.title = Some(text.to_owned())),
    );
    hint(
        engine,
        "categories",
        ["chart: Chart", "range: string", "Chart"],
        ["/// The labels along the category axis, or a pie's slice names — `\"A2:A9\"`."],
        |c: &mut Chart, range: &str| c.with(|r| r.categories = Some(range.to_owned())),
    );
    hint(
        engine,
        "series",
        ["chart: Chart", "values: string", "Chart"],
        ["/// One series of numbers to draw — `\"B2:B9\"`. Say it again for another."],
        |c: &mut Chart, values: &str| c.with(|r| r.series.push((values.to_owned(), None))),
    );
    hint(
        engine,
        "series",
        ["chart: Chart", "values: string", "name: string", "Chart"],
        [
            "/// One series, with the cell that names it — `.series(\"B2:B9\", \"B1\")`.",
            "///",
            "/// The name is an *address*, as in a spreadsheet, so the legend follows the cell.",
        ],
        |c: &mut Chart, values: &str, name: &str| {
            c.with(|r| r.series.push((values.to_owned(), Some(name.to_owned()))))
        },
    );
    hint(
        engine,
        "legend",
        ["chart: Chart", "where: string", "Chart"],
        [
            "/// Where the legend goes: `end`, `bottom`, `top`, `start`, or `none`.",
            "///",
            "/// Left unsaid, a chart with more than one thing to tell apart gets one at the end.",
        ],
        |c: &mut Chart, place: &str| -> Res<Chart> {
            let legend = match place {
                "none" => None,
                "end" => Some(ChartLegend::End),
                "bottom" => Some(ChartLegend::Bottom),
                "top" => Some(ChartLegend::Top),
                "start" => Some(ChartLegend::Start),
                other => {
                    return Err(format!(
                        "{other}: a legend goes at the end, bottom, top or start, or none"
                    )
                    .into());
                }
            };
            Ok(c.with(|r| r.legend = Some(legend)))
        },
    );
    hint(
        engine,
        "x_label",
        ["chart: Chart", "text: string", "Chart"],
        ["/// A title for the category axis."],
        |c: &mut Chart, text: &str| c.with(|r| r.x_label = Some(text.to_owned())),
    );
    hint(
        engine,
        "y_label",
        ["chart: Chart", "text: string", "Chart"],
        ["/// A title for the value axis."],
        |c: &mut Chart, text: &str| c.with(|r| r.y_label = Some(text.to_owned())),
    );
    hint(
        engine,
        "gridlines",
        ["chart: Chart", "Chart"],
        ["/// Rule gridlines across the plot from the value axis."],
        |c: &mut Chart| c.with(|r| r.gridlines = true),
    );
    hint(
        engine,
        "position",
        ["chart: Chart", "x: string", "y: string", "Chart"],
        [
            "/// Where the chart's top-left corner sits on the sheet, as two ODF lengths from",
            "/// the sheet's own corner — `.position(\"0cm\", \"6cm\")`.",
        ],
        |c: &mut Chart, x: &str, y: &str| {
            c.with(|r| {
                r.x = x.to_owned();
                r.y = y.to_owned();
            })
        },
    );
    hint(
        engine,
        "size",
        ["chart: Chart", "width: string", "height: string", "Chart"],
        ["/// How big the chart is, as two ODF lengths — `.size(\"16cm\", \"9cm\")`."],
        |c: &mut Chart, width: &str, height: &str| {
            c.with(|r| {
                r.width = width.to_owned();
                r.height = height.to_owned();
            })
        },
    );
    hint(
        engine,
        "colors",
        ["chart: Chart", "colors: array", "Chart"],
        [
            "/// Colours in the order of the series — or of the slices, for a pie. Palette names",
            "/// (`navy`, `teal`, …) or `#rrggbb`. Fewer than there are series is fine: the rest",
            "/// take the default cycle.",
        ],
        |c: &mut Chart, colors: Array| -> Res<Chart> {
            c.with_res(|r| {
                r.colors = colors
                    .into_iter()
                    .map(|color| {
                        let name = color
                            .into_string()
                            .map_err(|t| format!("a colour is a string, not {t}"))?;
                        style::color(&name).map_err(Into::into)
                    })
                    .collect::<Res<Vec<String>>>()?;
                Ok(())
            })
        },
    );
}
