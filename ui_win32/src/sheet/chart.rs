// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The grid's charts — where a chart sits (portable) and putting its marks down (GDI).
//!
//! `grind_sheet::chart_paint` decides where every bar, point, slice and label of a chart goes,
//! over a `Metrics`; the Mac draws the same list. This file is this window's half: a frame in
//! pixels from the chart's ODF lengths, a `Metrics` that measures in the font the label will be
//! drawn in (decision 3: one engine for measuring and drawing), and one GDI call per mark.
//!
//! Taking hold of one is `grind_sheet::chart_frame`'s behaviour (`doc/chart-handling.md`): a
//! click selects it and it wears the accent and eight handles, the body moves it, a handle
//! resizes it, and the keys and the right-click menu are the same as every other window's. The
//! hit-testing and drag arithmetic below are portable and tested here; `win.rs` holds the state
//! and `paint_held` draws the handles.

use grind_sheet::Chart;
use grind_sheet::chart_frame::{self, Frame, Grip};

use super::geom::{GridGeom, scale};

/// A chart's frame in pixels from the corner of A1, at `dpi` and the grid's `zoom` — `draw:frame`'s own position and
/// size, or `None` for a length this build cannot read, which is a chart it does not draw rather
/// than one drawn in the wrong place.
pub fn frame_of(chart: &Chart, dpi: u32, zoom: f64) -> Option<(f64, f64, f64, f64)> {
    let px = super::geom::mm_to_px(dpi);
    let length = |text: &str| grind_sheet::style::length_mm(text).map(|mm| px(mm) * zoom);
    let (x, y, w, h) = (
        length(&chart.x)?,
        length(&chart.y)?,
        length(&chart.width)?,
        length(&chart.height)?,
    );
    (w > 0.0 && h > 0.0).then_some((x, y, w, h))
}

/// Every chart's frame on the sheet in client pixels, where it is drawn — `None` for one with a
/// length this build cannot read, which is a chart it neither draws nor lets anybody grab.
pub fn frames(charts: &[Chart], geom: &GridGeom) -> Vec<Option<Frame>> {
    let origin = geom.cell_rect(0, 0);
    charts
        .iter()
        .map(|chart| {
            let (x, y, w, h) = frame_of(chart, geom.dpi, geom.zoom)?;
            Some(Frame::new(origin.x + x, origin.y + y, w, h))
        })
        .collect()
}

/// A handle's side and its reach beyond it, in this monitor's pixels — the suite's CSS sizes
/// scaled by the DPI. Not by the zoom: a handle is chrome, and zooming out must not make the
/// chart harder to take hold of.
pub fn handle_px(dpi: u32) -> (f64, f64) {
    (
        scale(chart_frame::HANDLE, dpi).round(),
        scale(chart_frame::HANDLE_SLOP, dpi),
    )
}

/// What the point has of the sheet's charts — a handle of the selected one, else the topmost
/// chart's body — and only inside the cells' own rectangle, since a chart scrolled under a header
/// band is not drawn there.
pub fn hit(
    charts: &[Chart],
    geom: &GridGeom,
    selected: Option<usize>,
    x: f64,
    y: f64,
) -> Option<(usize, Grip)> {
    if !geom.body().contains(x, y) {
        return None;
    }
    let (size, slop) = handle_px(geom.dpi);
    chart_frame::hit(&frames(charts, geom), selected, x, y, size, slop)
}

/// A chart held by the pointer: which, by what, where it was and where the pointer pressed, and
/// the frame the drag has reached — drawn in place of the chart's own until the release writes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grab {
    pub index: usize,
    pub grip: Grip,
    pub start: Frame,
    pub from: (f64, f64),
    pub now: Frame,
    pub was_selected: bool,
}

impl Grab {
    /// The pointer at `(x, y)`, Shift `keep_ratio`: where the chart is now. The smallest size is
    /// the suite's, scaled by the DPI and the zoom, since it is a size *on the sheet*.
    pub fn follow(&mut self, x: f64, y: f64, keep_ratio: bool, geom: &GridGeom) {
        let min = scale(chart_frame::MIN_SIZE, geom.dpi) * geom.zoom;
        let keep = keep_ratio && self.grip.is_corner();
        self.now = chart_frame::dragged(
            &self.start,
            self.grip,
            x - self.from.0,
            y - self.from.1,
            min,
            keep,
        );
    }

    /// Whether the press never moved far enough to be a move — a click, which only selects.
    pub fn is_click(&self, dpi: u32) -> bool {
        let slop = scale(chart_frame::CLICK_SLOP, dpi);
        chart_frame::is_click(self.now.x - self.start.x, self.now.y - self.start.y, slop)
            && chart_frame::is_click(self.now.w - self.start.w, self.now.h - self.start.h, slop)
    }
}

/// A frame in client pixels as the four ODF lengths `App::reshape_chart` takes — `svg:x`,
/// `svg:y`, `svg:width`, `svg:height` — kept on the sheet.
pub fn lengths(frame: Frame, geom: &GridGeom) -> [String; 4] {
    let origin = geom.cell_rect(0, 0);
    let to_mm = |px: f64| px / (super::geom::mm_to_px(geom.dpi)(1.0) * geom.zoom);
    let mm = chart_frame::kept_on_sheet(Frame::new(
        to_mm(frame.x - origin.x),
        to_mm(frame.y - origin.y),
        to_mm(frame.w),
        to_mm(frame.h),
    ));
    [mm.x, mm.y, mm.w.max(0.1), mm.h.max(0.1)].map(grind_sheet::style::mm_length)
}

/// An arrow's nudge in client pixels at this DPI and zoom — the suite's step is a distance on the
/// sheet, so it zooms with it.
pub fn nudged(frame: Frame, key: chart_frame::Key, geom: &GridGeom) -> Frame {
    match key {
        chart_frame::Key::Nudge(dx, dy) => {
            let k = scale(1.0, geom.dpi) * geom.zoom;
            Frame::new(frame.x + dx * k, frame.y + dy * k, frame.w, frame.h)
        }
        _ => frame,
    }
}

#[cfg(windows)]
pub use gdi_half::{paint, paint_held, paint_in};

#[cfg(windows)]
mod gdi_half {
    use windows::Win32::Foundation::{COLORREF, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        HDC, IntersectClipRect, Polygon, Polyline, RestoreDC, SaveDC, SetTextColor, TextOutW,
    };

    use grind_sheet::chart_paint::{self, Colours, Mark};

    use super::frame_of;
    use crate::gdi::{self, Brush, Pen, Selected};
    use crate::sheet::geom::GridGeom;
    use crate::sheet::measure::CellMetrics;
    use crate::theme::{Mode, Rgb, Theme};

    /// Every chart on the sheet that meets the body, drawn over the cells: a card on the page,
    /// then its title, legend, plot and labels. Clipped to the body so a chart scrolled under a
    /// header band cannot show through it.
    pub fn paint(
        dc: HDC,
        geom: &GridGeom,
        theme: Theme,
        face: &str,
        charts: &[grind_sheet::Chart],
        data: &[Option<grind_sheet::ChartData>],
        held: Option<&super::Grab>,
    ) {
        if charts.is_empty() {
            return;
        }
        let origin = geom.cell_rect(0, 0);
        let body = geom.body();
        let colours = Colours {
            page: theme.background.into(),
            ink: theme.text.into(),
            grid: theme.grid_line.into(),
            header_ink: theme.text.blend(theme.background, 0.45).into(),
            accent: theme.accent.into(),
            dark: theme.mode == Mode::Dark,
        };
        let metrics = CellMetrics {
            dc,
            face,
            dpi: geom.dpi,
            default_pt: 9.0,
            zoom: geom.zoom,
        };
        // SAFETY: the DC is live; the clip is undone by the matching `RestoreDC` below.
        let saved = unsafe {
            let saved = SaveDC(dc);
            let (left, top, right, bottom) = body.edges();
            IntersectClipRect(dc, left, top, right, bottom);
            saved
        };
        for (index, chart) in charts.iter().enumerate() {
            let Some((x, y, w, h)) = frame_of(chart, geom.dpi, geom.zoom) else {
                continue;
            };
            let Some(Some(data)) = data.get(index) else {
                continue;
            };
            // A chart being dragged is drawn where the drag has put it; nothing is written yet.
            let frame = match held {
                Some(grab) if grab.index == index => {
                    let now = grab.now;
                    chart_paint::Rect::new(now.x, now.y, now.w, now.h)
                }
                _ => chart_paint::Rect::new(origin.x + x, origin.y + y, w, h),
            };
            for mark in chart_paint::draw(chart, data, frame, &colours, &metrics) {
                put(dc, face, &metrics, mark);
            }
        }
        // SAFETY: restoring the state saved above.
        unsafe {
            let _ = RestoreDC(dc, -1);
            let _ = saved;
        }
    }

    /// One chart drawn to fill `width` × `height` pixels at the DC's origin — the preview dialog's
    /// picture. The same marks as [`paint`], over the same `chart_paint::draw`, with the frame
    /// the dialog's rather than the chart's own ODF position.
    pub fn paint_in(
        dc: HDC,
        theme: Theme,
        face: &str,
        dpi: u32,
        size: (i32, i32),
        chart: &grind_sheet::Chart,
        data: &grind_sheet::ChartData,
    ) {
        let colours = Colours {
            page: theme.background.into(),
            ink: theme.text.into(),
            grid: theme.grid_line.into(),
            header_ink: theme.text.blend(theme.background, 0.45).into(),
            accent: theme.accent.into(),
            dark: theme.mode == Mode::Dark,
        };
        let metrics = CellMetrics {
            dc,
            face,
            dpi,
            default_pt: 9.0,
            zoom: 1.0,
        };
        let frame = chart_paint::Rect::new(0.0, 0.0, f64::from(size.0), f64::from(size.1));
        for mark in chart_paint::draw(chart, data, frame, &colours, &metrics) {
            put(dc, face, &metrics, mark);
        }
    }

    /// The selected chart, unmistakable (`doc/chart-handling.md`): a two-pixel accent outline
    /// and eight handles — the page's colour inside an accent border — drawn over the cell
    /// selection and clipped to the cells, for as long as it is selected.
    pub fn paint_held(
        dc: HDC,
        geom: &GridGeom,
        theme: Theme,
        frame: grind_sheet::chart_frame::Frame,
    ) {
        let body = geom.body();
        // SAFETY: the DC is live; the clip is undone by the matching `RestoreDC` below.
        unsafe {
            SaveDC(dc);
            let (left, top, right, bottom) = body.edges();
            IntersectClipRect(dc, left, top, right, bottom);
        }
        let line = crate::sheet::geom::scale(2.0, geom.dpi).round().max(1.0) as i32;
        let (l, t) = (frame.x.round() as i32, frame.y.round() as i32);
        let (r, b) = (frame.right().round() as i32, frame.bottom().round() as i32);
        ring(dc, (l, t, r, b), line, theme.accent);
        let (size, _) = super::handle_px(geom.dpi);
        let border = crate::sheet::geom::scale(1.5, geom.dpi).round().max(1.0) as i32;
        for (_, square) in grind_sheet::chart_frame::handles(&frame, size) {
            let (l, t) = (square.x.round() as i32, square.y.round() as i32);
            let (r, b) = (l + size as i32, t + size as i32);
            gdi::fill(dc, l, t, r, b, theme.background);
            ring(dc, (l, t, r, b), border, theme.accent);
        }
        // SAFETY: restoring the state saved above.
        unsafe {
            let _ = RestoreDC(dc, -1);
        }
    }

    /// A rectangle's edge `width` pixels thick, inside `(left, top, right, bottom)`.
    fn ring(dc: HDC, (l, t, r, b): (i32, i32, i32, i32), width: i32, colour: Rgb) {
        gdi::fill(dc, l, t, r, t + width, colour);
        gdi::fill(dc, l, b - width, r, b, colour);
        gdi::fill(dc, l, t, l + width, b, colour);
        gdi::fill(dc, r - width, t, r, b, colour);
    }

    fn rgb(colour: grind_core::color::Rgb) -> Rgb {
        colour.into()
    }

    fn put(dc: HDC, _face: &str, metrics: &CellMetrics, mark: Mark) {
        match mark {
            Mark::Fill { rect, color } => {
                let (l, t) = (rect.x.round() as i32, rect.y.round() as i32);
                // A hairline stays a pixel: rounding the far edge alone could erase it.
                let r = (rect.right().round() as i32).max(l + 1);
                let b = (rect.bottom().round() as i32).max(t + 1);
                gdi::fill(dc, l, t, r, b, rgb(color));
            }
            Mark::Text {
                x,
                top,
                text,
                style,
                color,
                clip,
            } => {
                let wide: Vec<u16> = text.encode_utf16().collect();
                if wide.is_empty() {
                    return;
                }
                let font = metrics.font(&style);
                let _selected = Selected::font(dc, &font);
                // SAFETY: the DC is live; the clip is undone by `RestoreDC`, and the buffer is
                // a local that outlives the call.
                unsafe {
                    let saved = SaveDC(dc);
                    IntersectClipRect(
                        dc,
                        clip.x.round() as i32,
                        clip.y.round() as i32,
                        clip.right().round() as i32,
                        clip.bottom().round() as i32,
                    );
                    SetTextColor(dc, COLORREF(rgb(color).colorref()));
                    let _ = TextOutW(dc, x.round() as i32, top.round() as i32, &wide);
                    let _ = RestoreDC(dc, saved);
                }
            }
            Mark::Path {
                points,
                fill,
                stroke,
            } => {
                let points: Vec<POINT> = points
                    .iter()
                    .map(|(x, y)| POINT {
                        x: x.round() as i32,
                        y: y.round() as i32,
                    })
                    .collect();
                if points.len() < 2 {
                    return;
                }
                let pen = stroke.map(|(color, width)| Pen::solid(rgb(color), width.round() as i32));
                match fill {
                    Some(color) => {
                        let brush = Brush::solid(rgb(color));
                        let outline = pen.unwrap_or_else(|| Pen::solid(rgb(color), 1));
                        let _brush = Selected::brush(dc, &brush);
                        let _pen = Selected::pen(dc, &outline);
                        // SAFETY: the DC is live and the points a local slice.
                        unsafe {
                            let _ = Polygon(dc, &points);
                        }
                    }
                    None => {
                        let Some(pen) = pen else { return };
                        let _pen = Selected::pen(dc, &pen);
                        // SAFETY: as above.
                        unsafe {
                            let _ = Polyline(dc, &points);
                        }
                    }
                }
            }
        }
        let _ = RECT::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chart_frame_scales_with_the_dpi_and_an_unreadable_length_is_no_frame() {
        let mut chart = grind_sheet::Chart::new(
            grind_sheet::ChartKind::Bar,
            "2.54cm".into(),
            "0cm".into(),
            "5.08cm".into(),
            "2.54cm".into(),
        );
        let (x, _, w, h) = frame_of(&chart, 96, 1.0).unwrap();
        assert!((x - 96.0).abs() < 0.5 && (w - 192.0).abs() < 0.5 && (h - 96.0).abs() < 0.5);
        let (x2, ..) = frame_of(&chart, 192, 1.0).unwrap();
        assert!((x2 - 192.0).abs() < 0.5);
        let (x3, ..) = frame_of(&chart, 96, 2.0).unwrap();
        assert!((x3 - 192.0).abs() < 0.5, "a zoom scales the frame");
        chart.width = "wide".into();
        assert!(frame_of(&chart, 96, 1.0).is_none());
    }

    fn geom() -> GridGeom {
        GridGeom {
            format_h: 0.0,
            strip_h: 0.0,
            banner_h: 0.0,
            hint_h: 0.0,
            header_w: 40.0,
            header_h: 20.0,
            status_h: 22.0,
            rows: super::super::geom::Sizes::new(20.0, grind_sheet::MAX_ROWS, vec![]),
            cols: super::super::geom::Sizes::new(80.0, grind_sheet::MAX_COLS, vec![]),
            first_row: 0,
            first_col: 0,
            width: 800.0,
            height: 600.0,
            dpi: 96,
            zoom: 1.0,
        }
    }

    fn chart() -> Chart {
        // 1in from A1's corner each way, 2in by 1in: (40+96, 20+96) to (328, 212) at 96 dpi.
        Chart::new(
            grind_sheet::ChartKind::Bar,
            "2.54cm".into(),
            "2.54cm".into(),
            "5.08cm".into(),
            "2.54cm".into(),
        )
    }

    #[test]
    fn a_chart_is_hit_where_it_is_drawn_and_its_handles_only_once_selected() {
        let (g, charts) = (geom(), [chart()]);
        assert_eq!(hit(&charts, &g, None, 200.0, 150.0), Some((0, Grip::Body)));
        // The bottom-right corner, a pixel or two outside: nothing until it is selected.
        assert_eq!(hit(&charts, &g, None, 330.0, 214.0), None);
        assert_eq!(
            hit(&charts, &g, Some(0), 330.0, 214.0),
            Some((0, Grip::SouthEast))
        );
        // Over the header band a chart is not there to be hit.
        assert_eq!(hit(&charts, &g, Some(0), 200.0, 10.0), None);
    }

    #[test]
    fn a_drag_and_its_release_round_trip_through_the_documents_lengths() {
        let g = geom();
        let start = frames(&[chart()], &g)[0].unwrap();
        let mut grab = Grab {
            index: 0,
            grip: Grip::East,
            start,
            from: (328.0, 160.0),
            now: start,
            was_selected: true,
        };
        grab.follow(330.0, 161.0, false, &g);
        assert!(grab.is_click(96), "two pixels is a click");
        grab.follow(424.0, 300.0, false, &g);
        assert!(!grab.is_click(96));
        let [x, y, w, h] = lengths(grab.now, &g);
        assert_eq!(
            (x.as_str(), y.as_str(), h.as_str()),
            ("25.400mm", "25.400mm", "25.400mm")
        );
        assert_eq!(w, "76.200mm", "an inch wider, the left edge held");
        // Dragged past A1 it stops at the corner.
        let far = Frame::new(0.0, 0.0, start.w, start.h);
        assert_eq!(lengths(far, &g)[0], "0.000mm");
    }

    #[test]
    fn a_nudge_zooms_with_the_sheet() {
        let mut g = geom();
        g.zoom = 2.0;
        let f = Frame::new(100.0, 100.0, 50.0, 50.0);
        let moved = nudged(f, chart_frame::nudge(1, 0, false), &g);
        assert_eq!(moved.x, 100.0 + chart_frame::NUDGE * 2.0);
    }
}
