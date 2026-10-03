// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The grid's charts — where a chart sits (portable) and putting its marks down (GDI).
//!
//! `grind_sheet::chart_paint` decides where every bar, point, slice and label of a chart goes,
//! over a `Metrics`; the Mac draws the same list. This file is this window's half: a frame in
//! pixels from the chart's ODF lengths, a `Metrics` that measures in the font the label will be
//! drawn in (decision 3: one engine for measuring and drawing), and one GDI call per mark.
//! Read-only here — a chart is a picture on this grid; adding one is Sheet ▸ Insert Chart.

use grind_sheet::Chart;

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

#[cfg(windows)]
pub use gdi_half::{paint, paint_in};

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
            let frame = chart_paint::Rect::new(origin.x + x, origin.y + y, w, h);
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
}
