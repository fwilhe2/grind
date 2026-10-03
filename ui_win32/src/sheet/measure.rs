// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Measuring a cell's text the way it is drawn: one engine, GDI, for both (decision 3).
//!
//! `grind_core::layout::wrap` breaks a wrapped cell and `grind_sheet::autoheight` sizes a row, and
//! both ask a `Metrics`. This is this window's — the chart painter's and the wrapped cells' and
//! the row heights' — so a label, a wrapped line and the row that holds it agree about how wide
//! text is. Windows only: it needs a DC.

#![cfg(windows)]

use windows::Win32::Graphics::Gdi::{GetTextMetricsW, HDC, TEXTMETRICW};

use grind_core::layout::Metrics;
use grind_core::style::TextStyle;

use crate::gdi::{self, Font, Selected};

/// Measures in the font a mark will be drawn in: the cell face at the style's size, weight and
/// slant.
pub struct CellMetrics<'a> {
    pub dc: HDC,
    pub face: &'a str,
    pub dpi: u32,
    /// The size, in points, of text whose style names none.
    pub default_pt: f64,
    /// The grid's zoom, a factor on every size: text is measured at the size it is drawn at.
    pub zoom: f64,
}

impl CellMetrics<'_> {
    pub fn font(&self, style: &TextStyle) -> Font {
        let points = style
            .font_size
            .as_deref()
            .and_then(|size| size.strip_suffix("pt"))
            .and_then(|size| size.parse::<f64>().ok())
            .unwrap_or(self.default_pt);
        let px = (points * f64::from(self.dpi) * self.zoom / 72.0).round() as i32;
        Font::styled(
            self.face,
            px.max(6),
            style.font_weight.as_deref() == Some("bold"),
            matches!(style.font_style.as_deref(), Some("italic" | "oblique")),
            false,
            false,
        )
    }
}

impl Metrics for CellMetrics<'_> {
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
        let font = self.font(style);
        let _selected = Selected::font(self.dc, &font);
        let mut prefix = String::new();
        for c in text.chars() {
            prefix.push(c);
            out.push(gdi::text_width(self.dc, &prefix) as f32);
        }
    }

    fn line_height(&self, style: &TextStyle) -> f32 {
        let font = self.font(style);
        let _selected = Selected::font(self.dc, &font);
        let mut tm = TEXTMETRICW::default();
        // SAFETY: the DC is live and `tm` is a local that outlives the call.
        unsafe {
            let _ = GetTextMetricsW(self.dc, &mut tm);
        }
        tm.tmHeight as f32
    }
}
