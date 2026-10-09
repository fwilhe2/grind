// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A chart, as a list of [`Op`]s — portable, so where every bar, point, slice and label goes is
//! tested on Linux and the Mac only puts it down.
//!
//! Where every mark goes is `grind_sheet::chart_paint`'s, hoisted out of this file when the
//! Windows grid wanted charts too; what is here is the Mac's half — where a chart's frame sits on
//! the sheet, which one a point is in, what a context menu changes — and [`draw`], which turns the
//! shared marks into this shell's [`Op`]s.
//!
//! Taking hold of one is `grind_sheet::chart_frame`'s behaviour, every window's
//! (`doc/chart-handling.md`): a click selects it and it wears the accent and eight handles
//! ([`held_ops`]), its body moves it, a handle resizes it, the keys a selected chart answers are
//! [`key`]'s, and a release is one `App::reshape_chart`. Frames are in the sheet's points, which
//! at 1× are the suite's CSS pixels near enough that its sizes are used as they are.

use grind_core::layout::Metrics;
use grind_sheet::chart_frame::{self, Frame, Grip};
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

/// What the grid knows about the charts this frame: the one selected, and the one the pointer
/// is holding, which is drawn where the drag has it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Held {
    pub selected: Option<usize>,
    pub grab: Option<Grab>,
}

/// Every chart on `sheet` that meets `view`, drawn in the sheet's own coordinates — the space
/// [`super::paint::cells`] draws in, so a chart floats over the cells under it — and then the
/// selected one's outline and handles over all of them.
pub fn charts(
    app: &App,
    sheet: usize,
    view: &Rect,
    palette: &Palette,
    metrics: &dyn Metrics,
    held: Held,
) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut handled = None;
    for (index, chart) in app.charts(sheet).unwrap_or_default().iter().enumerate() {
        let Some(mut frame) = frame_of(chart) else {
            continue;
        };
        if let Some(grab) = held.grab.filter(|grab| grab.index == index) {
            frame = rect(grab.now);
        }
        if held.selected == Some(index) || held.grab.is_some_and(|grab| grab.index == index) {
            handled = Some(frame);
        }
        if frame.intersection(view).is_empty() {
            continue;
        }
        let Ok(data) = app.chart_data(sheet, index) else {
            continue;
        };
        ops.extend(draw(chart, &data, frame, palette, metrics));
    }
    if let Some(frame) = handled {
        ops.extend(held_ops(frame, palette));
    }
    ops
}

/// A selected chart, unmistakable: a two-point outline in the accent and eight handles — the
/// page's colour inside an accent border — half outside the frame, where they read as handles.
pub fn held_ops(frame: Rect, palette: &Palette) -> Vec<Op> {
    let mut ops = ring(frame, 2.0, palette.accent);
    for (_, square) in chart_frame::handles(&self::frame(frame), chart_frame::HANDLE) {
        let square = rect(square);
        ops.push(Op::Fill {
            rect: square,
            color: palette.page,
        });
        ops.extend(ring(square, 1.5, palette.accent));
    }
    ops
}

/// A rectangle's edge `width` thick, inside it.
fn ring(r: Rect, width: f64, color: grind_core::color::Rgb) -> Vec<Op> {
    [
        Rect::new(r.x, r.y, r.w, width),
        Rect::new(r.x, r.bottom() - width, r.w, width),
        Rect::new(r.x, r.y, width, r.h),
        Rect::new(r.right() - width, r.y, width, r.h),
    ]
    .map(|rect| Op::Fill { rect, color })
    .into()
}

fn frame(r: Rect) -> Frame {
    Frame::new(r.x, r.y, r.w, r.h)
}

fn rect(f: Frame) -> Rect {
    Rect::new(f.x, f.y, f.w, f.h)
}

/// Every chart's frame on `sheet`, in the sheet's points.
pub fn frames(app: &App, sheet: usize) -> Vec<Option<Frame>> {
    app.charts(sheet)
        .unwrap_or_default()
        .iter()
        .map(|chart| frame_of(chart).map(frame))
        .collect()
}

/// What `(x, y)` has of the charts on `sheet` (`chart_frame::hit`): a handle of the selected one,
/// which reach a little outside it, before the topmost chart's body.
pub fn hit(
    app: &App,
    sheet: usize,
    selected: Option<usize>,
    x: f64,
    y: f64,
) -> Option<(usize, Grip)> {
    chart_frame::hit(
        &frames(app, sheet),
        selected,
        x,
        y,
        chart_frame::HANDLE,
        chart_frame::HANDLE_SLOP,
    )
}

/// A chart held by the pointer: which, by what, where it was, where the pointer pressed, and the
/// frame the drag has reached — drawn in place of the chart's own until the release writes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grab {
    pub index: usize,
    pub grip: Grip,
    pub start: Frame,
    pub from: (f64, f64),
    pub now: Frame,
}

impl Grab {
    /// Take hold of chart `index` by `grip` at `(x, y)`.
    pub fn new(app: &App, sheet: usize, index: usize, grip: Grip, x: f64, y: f64) -> Option<Grab> {
        let start = frames(app, sheet).get(index).copied().flatten()?;
        Some(Grab {
            index,
            grip,
            start,
            from: (x, y),
            now: start,
        })
    }

    /// The pointer at `(x, y)`, ⇧ held or not: where the chart is now.
    pub fn follow(&mut self, x: f64, y: f64, keep_ratio: bool) {
        let keep = keep_ratio && self.grip.is_corner();
        self.now = chart_frame::dragged(
            &self.start,
            self.grip,
            x - self.from.0,
            y - self.from.1,
            chart_frame::MIN_SIZE,
            keep,
        );
    }

    /// Whether the press never moved far enough to be a move — a click, which only selects.
    pub fn is_click(&self) -> bool {
        let slop = chart_frame::CLICK_SLOP;
        chart_frame::is_click(self.now.x - self.start.x, self.now.y - self.start.y, slop)
            && chart_frame::is_click(self.now.w - self.start.w, self.now.h - self.start.h, slop)
    }
}

/// A frame in the sheet's points as the four ODF lengths `App::reshape_chart` takes, kept on the
/// sheet — never above or left of A1, where nothing can show it.
pub fn lengths(frame: Frame) -> [String; 4] {
    let mm = chart_frame::kept_on_sheet(frame);
    [mm.x, mm.y, mm.w, mm.h].map(|pt| grind_sheet::style::mm_length(pt / PT_PER_MM))
}

/// What a selector does to a selected chart — the keys every window gives one. `None` is a key
/// that means nothing to a chart, which lets go of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyAction {
    Key(chart_frame::Key),
    /// Return: change it — its title, kind and legend, the context menu's own items.
    Change,
}

pub fn key(selector: &str) -> Option<KeyAction> {
    use KeyAction::{Change, Key as K};
    Some(match selector {
        "deleteBackward:" | "deleteForward:" | "deleteWordBackward:" => K(chart_frame::Key::Delete),
        "cancelOperation:" => K(chart_frame::Key::Deselect),
        "moveLeft:" => K(chart_frame::nudge(-1, 0, false)),
        "moveRight:" => K(chart_frame::nudge(1, 0, false)),
        "moveUp:" => K(chart_frame::nudge(0, -1, false)),
        "moveDown:" => K(chart_frame::nudge(0, 1, false)),
        "moveLeftAndModifySelection:" => K(chart_frame::nudge(-1, 0, true)),
        "moveRightAndModifySelection:" => K(chart_frame::nudge(1, 0, true)),
        "moveUpAndModifySelection:" => K(chart_frame::nudge(0, -1, true)),
        "moveDownAndModifySelection:" => K(chart_frame::nudge(0, 1, true)),
        "insertNewline:" => Change,
        _ => return None,
    })
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
        let near = charts(&app, 0, &frame, &Palette::LIGHT, &Fixed, Held::default());
        assert!(!near.is_empty());
        let far = Rect::new(frame.right() + 1000.0, 0.0, 100.0, 100.0);
        assert!(charts(&app, 0, &far, &Palette::LIGHT, &Fixed, Held::default()).is_empty());
    }

    #[test]
    fn the_topmost_chart_under_a_point_is_found_and_changed() {
        let app = sheet_with("bar");
        let frame = frame_of(&app.charts(0).unwrap()[0]).unwrap();
        assert_eq!(
            hit(&app, 0, None, frame.x + 5.0, frame.y + 5.0),
            Some((0, Grip::Body))
        );
        assert_eq!(hit(&app, 0, None, frame.x - 5.0, frame.y + 5.0), None);
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
        let ground = |held| {
            charts(&app, 0, &view, &Palette::LIGHT, &Fixed, held)
                .into_iter()
                .find_map(|op| match op {
                    Op::Fill { rect, .. } => Some(rect),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(ground(Held::default()), frame);
        let mut grab = Grab::new(&app, 0, 0, Grip::Body, 50.0, 50.0).unwrap();
        grab.follow(52.0, 49.0, false);
        assert!(grab.is_click(), "a few points is a click");
        grab.follow(80.0, 90.0, false);
        assert!(!grab.is_click());
        let held = Held {
            selected: Some(0),
            grab: Some(grab),
        };
        assert_eq!(ground(held), frame.offset(30.0, 40.0));
        let mut far = grab.now;
        far.x = 72.0;
        far.y = -1000.0;
        let [x, y, ..] = lengths(far);
        assert!((grind_sheet::style::length_mm(&x).unwrap() - 25.4).abs() < 0.01);
        assert_eq!(
            grind_sheet::style::length_mm(&y),
            Some(0.0),
            "never above row 1"
        );
    }

    #[test]
    fn a_selected_chart_wears_its_handles_and_they_resize_it() {
        let app = sheet_with("bar");
        let frame = frame_of(&app.charts(0).unwrap()[0]).unwrap();
        let view = Rect::new(0.0, 0.0, 2000.0, 2000.0);
        let accent = |held| {
            charts(&app, 0, &view, &Palette::LIGHT, &Fixed, held)
                .iter()
                .filter(
                    |op| matches!(op, Op::Fill { color, .. } if *color == Palette::LIGHT.accent),
                )
                .count()
        };
        assert_eq!(accent(Held::default()), 0, "unselected: no handles");
        let selected = Held {
            selected: Some(0),
            grab: None,
        };
        assert_eq!(
            accent(selected),
            4 + 8 * 4,
            "the outline and eight ringed squares"
        );
        // The bottom-right handle, just outside the corner, only once it is selected.
        let (x, y) = (frame.right() + 2.0, frame.bottom() + 2.0);
        assert_eq!(hit(&app, 0, None, x, y), None);
        assert_eq!(hit(&app, 0, Some(0), x, y), Some((0, Grip::SouthEast)));
        let mut grab = Grab::new(&app, 0, 0, Grip::SouthEast, x, y).unwrap();
        grab.follow(x + 36.0, y, false);
        let [.., w, h] = lengths(grab.now);
        let pt = |length: &str| grind_sheet::style::length_mm(length).unwrap() * PT_PER_MM;
        assert!((pt(&w) - (frame.w + 36.0)).abs() < 0.01);
        assert!((pt(&h) - frame.h).abs() < 0.01);
    }

    #[test]
    fn the_keys_a_selected_chart_answers() {
        assert_eq!(
            key("deleteBackward:"),
            Some(KeyAction::Key(chart_frame::Key::Delete))
        );
        assert_eq!(
            key("cancelOperation:"),
            Some(KeyAction::Key(chart_frame::Key::Deselect))
        );
        assert_eq!(
            key("moveRightAndModifySelection:"),
            Some(KeyAction::Key(chart_frame::nudge(1, 0, true)))
        );
        assert_eq!(key("insertNewline:"), Some(KeyAction::Change));
        assert_eq!(key("insertTab:"), None);
    }
}
