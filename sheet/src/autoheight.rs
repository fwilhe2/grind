// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Row auto-height: the rows a sheet should grow to hold what is in them. A row with no height of
//! its own is as tall as its tallest cell wants — one that wraps, or one in a larger face —
//! measured by `grind_core::layout::wrap`, the breaker that also draws the lines. The GNOME window
//! measures with Pango (`measure_rows`); this is for the shells with a `Metrics` and `Sizes`.

use grind_core::layout::{Fragment, Metrics, wrap};

use crate::{App, look};

/// How much sheet is measured for natural row heights; rows above the view still displace those
/// below, so the pass cannot be limited to the screen. Past this every row keeps its height.
pub const MAX_CELLS: u64 = 200_000;

/// The rows to grow, and to what: each one's tallest cell that wraps or names a font size, plus
/// `row_pad`, when taller than `row_h`. Plain cells are never laid out. `col_width`, `pad_x` and
/// `metrics` are in the shell's unit. Rows hidden or sized by the document are the caller's to put
/// back.
pub fn grown_rows(
    app: &App,
    sheet: usize,
    col_width: &dyn Fn(u32) -> f64,
    pad_x: f64,
    row_pad: f64,
    row_h: f64,
    metrics: &dyn Metrics,
) -> Vec<(u32, f64)> {
    let Ok((rows, cols)) = app.used_extent(sheet) else {
        return Vec::new();
    };
    if rows == 0 || cols == 0 || u64::from(rows) * u64::from(cols) > MAX_CELLS {
        return Vec::new();
    }
    let Ok(viewport) = app.get_viewport(sheet, 0..rows, 0..cols) else {
        return Vec::new();
    };
    let mut grown = Vec::new();
    for row in 0..rows {
        let mut tallest: f64 = 0.0;
        for col in 0..cols {
            let Some(style) = viewport.style(row, col) else {
                continue;
            };
            let wrapping = look::wraps(Some(style));
            if !wrapping && style.font_size.is_none() {
                continue;
            }
            let Some(text) = viewport.text(row, col).filter(|text| !text.is_empty()) else {
                continue;
            };
            let text_style = look::text_style(Some(style));
            // Width zero is `wrap`'s "do not wrap", for a cell here only for its larger face.
            let width = match wrapping {
                true => (col_width(col) - 2.0 * pad_x).max(1.0) as f32,
                false => 0.0,
            };
            let fragment = Fragment {
                text,
                style: &text_style,
            };
            let laid = wrap(std::slice::from_ref(&fragment), width, metrics);
            tallest = tallest.max(f64::from(laid.height()));
        }
        if tallest + row_pad > row_h {
            grown.push((row, (tallest + row_pad).ceil()));
        }
    }
    grown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pos, RecalcMode};
    use grind_core::layout::Fixed;

    #[test]
    fn a_wrapping_cell_grows_its_row_and_a_plain_one_does_not() {
        let app = App::new();
        app.enter(
            0,
            Pos::new(0, 0),
            "one two three four five six seven",
            RecalcMode::Document,
        )
        .unwrap();
        app.enter(0, Pos::new(1, 0), "plain", RecalcMode::Document)
            .unwrap();
        let style = crate::style::CellStyle {
            wrap: Some("wrap".into()),
            ..Default::default()
        };
        app.set_style(0, Pos::new(0, 0), Pos::new(0, 0), Some(style))
            .unwrap();
        let grown = grown_rows(&app, 0, &|_| 20.0, 4.0, 4.0, 5.0, &Fixed);
        assert_eq!(grown.len(), 1, "{grown:?}");
        assert_eq!(grown[0].0, 0, "only the styled, wrapping row");
        assert!(grown[0].1 > 5.0);
    }
}
