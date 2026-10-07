// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind sheet fit`'s measuring: the one thing about fitting a column the core leaves to its
//! caller, since the core has no font (`grind_sheet::fit`).
//!
//! With the `pdf` feature — on by default — text is shaped in the bundled Liberation faces
//! (`grind_print::Typesetter`), the metric twins of Arial and Times, so a width the CLI fits is
//! the width LibreOffice would draw that text at. Without it the CLI has no font at all, and
//! estimates: an average glyph a little over half an em.

use std::ops::Range;

use grind_core::layout::Metrics;
use grind_core::style::TextStyle;
use grind_sheet::{App, look};

/// The room a fitted column keeps beyond its widest text, in points: the macOS grid's padding
/// either side of a cell's text and its slack, so the last glyph never touches the next line.
const PAD_PT: f64 = 2.0 * 4.0 + 4.0;

/// What a cell's text is measured in: its own size, weight and slant, and Calc's default face
/// and size where the style names none.
fn style_of(style: Option<&grind_sheet::style::CellStyle>) -> TextStyle {
    let mut text = look::text_style(style);
    text.font_family
        .get_or_insert_with(|| "Liberation Sans".into());
    text.font_size
        .get_or_insert_with(|| format!("{}pt", look::DEFAULT_FONT_PT));
    text
}

/// The CLI's `Metrics`, in points.
#[cfg(feature = "pdf")]
pub fn measure() -> impl Metrics {
    grind_print::Typesetter::new(grind_print::Fonts::bundled())
}

/// The CLI's `Metrics`, in points — estimated, with no font in this build.
#[cfg(not(feature = "pdf"))]
pub fn measure() -> impl Metrics {
    Estimate
}

#[cfg(not(feature = "pdf"))]
struct Estimate;

#[cfg(not(feature = "pdf"))]
impl Metrics for Estimate {
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
        let size = style
            .font_size
            .as_deref()
            .and_then(grind_core::style::length_mm)
            .map_or(look::DEFAULT_FONT_PT, |mm| mm * 72.0 / 25.4);
        let em = (size * 0.55) as f32;
        out.extend((1..=text.chars().count()).map(|n| n as f32 * em));
    }

    fn line_height(&self, _: &TextStyle) -> f32 {
        look::DEFAULT_FONT_PT as f32 * 1.2
    }
}

/// Each of `cols` with the width that fits it, `None` for one with nothing to show.
pub fn widths(
    app: &App,
    sheet: usize,
    cols: Range<u32>,
    metrics: &dyn Metrics,
) -> Vec<(u32, Option<String>)> {
    cols.map(|col| {
        let widest = grind_sheet::fit::widest(app, sheet, col, metrics, &style_of);
        (col, grind_sheet::fit::width(widest, PAD_PT, 25.4 / 72.0))
    })
    .collect()
}
