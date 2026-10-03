// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Row auto-height (L3): the rows a sheet should grow to hold what is in them.
//!
//! A row with no height of its own is as tall as its tallest cell *wants* to be — a cell that
//! wraps onto more lines, or one set in a larger face — measured by `grind_core::layout::wrap`,
//! the breaker that also draws the wrapped lines, so a row is never sized by one engine and drawn
//! by another. Hoisted out of the macOS shell when the Windows grid wanted wrapped cells: the
//! GNOME window has its own copy over Pango (`measure_rows`), and this is the one for the shells
//! that have a `Metrics` and a grid of `Sizes`.

use grind_core::layout::{Fragment, Metrics, wrap};

use crate::{App, look};

/// How much sheet is measured for natural row heights. A row above the view still displaces the
/// ones below it, so the pass cannot be limited to what is on screen — past this much document
/// every row keeps its height, the GNOME window's bound.
pub const MAX_CELLS: u64 = 200_000;

/// The rows to grow, and to what: for each, the tallest of its cells that wraps or names a font
/// size, plus `row_pad`, when that is taller than `row_h`. A cell with no style is never laid
/// out, which is what keeps this a cheap pass over a sheet where nine cells in ten are plain.
///
/// `col_width` is the shell's own width of a column in its unit, `pad_x` the room a cell keeps
/// either side of its text, and `metrics` measures in that same unit. Rows hidden or sized by the
/// document are the caller's to put back over the answer.
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
            // A width of zero is `wrap`'s own "do not wrap": one line per hard break, which is
            // what a cell here only for its larger face wants.
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
        let mut style = crate::style::CellStyle::default();
        style.wrap = Some("wrap".into());
        app.set_style(0, Pos::new(0, 0), Pos::new(0, 0), Some(style))
            .unwrap();
        let grown = grown_rows(&app, 0, &|_| 20.0, 4.0, 4.0, 5.0, &Fixed);
        assert_eq!(grown.len(), 1, "{grown:?}");
        assert_eq!(grown[0].0, 0, "only the styled, wrapping row");
        assert!(grown[0].1 > 5.0);
    }
}
