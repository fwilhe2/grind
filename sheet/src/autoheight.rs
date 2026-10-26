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

/// The rows to grow, and to what: each one's tallest cell that [`look::grows_row`] names, plus
/// `row_pad`, when taller than `row_h`. Other cells are never laid out. `col_width`, `pad_x` and
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
            // A merge on one row is measured as wide as its columns; a taller one grows none of
            // its rows (`doc/ods-format.md` §5.4).
            let mut span = col..col + 1;
            if let Some(merge) = viewport.merge_at(row, col) {
                if merge.anchor != crate::Pos::new(row, col) || merge.end.row != row {
                    continue;
                }
                span = col..merge.end.col + 1;
            }
            let style = viewport.style(row, col);
            let Some(text) = viewport
                .text(row, col)
                .filter(|text| look::grows_row(style, text))
            else {
                continue;
            };
            let wrapping = look::wraps(style);
            let text_style = look::text_style(style);
            // Width zero is `wrap`'s "do not wrap", for a cell here only for its larger face.
            let width = match wrapping {
                true => (span.map(col_width).sum::<f64>() - 2.0 * pad_x).max(1.0) as f32,
                false => 0.0,
            };
            let fragment = Fragment {
                text,
                style: &text_style,
            };
            let laid = wrap(std::slice::from_ref(&fragment), width, metrics);
            tallest = tallest.max(f64::from(look::needed_height(&laid, style)));
        }
        if tallest + row_pad > row_h {
            grown.push((row, (tallest + row_pad).ceil()));
        }
    }
    grown
}

/// The heights a grid gives its rows, in its own unit, for a `Sizes` that keeps the last entry
/// per row: what [`grown_rows`] measured, then the document's own `sized` heights — each final,
/// except a row in `measured` ([`crate::Sheet::row_measured`]), which keeps whichever is taller.
pub fn heights(
    grown: Vec<(u32, f64)>,
    sized: Vec<(u32, f64)>,
    measured: &std::collections::BTreeSet<u32>,
) -> Vec<(u32, f64)> {
    let tallest: std::collections::HashMap<u32, f64> = grown.iter().copied().collect();
    let mut out = grown;
    out.extend(sized.into_iter().map(|(row, height)| {
        match (measured.contains(&row), tallest.get(&row)) {
            (true, Some(grown)) => (row, height.max(*grown)),
            _ => (row, height),
        }
    }));
    out
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

    /// A measured height is a floor the content may raise; a chosen one is final.
    #[test]
    fn a_measured_row_grows_and_a_sized_one_does_not() {
        let measured = std::collections::BTreeSet::from([1]);
        let out = heights(
            vec![(0, 40.0), (1, 40.0), (2, 10.0)],
            vec![(0, 20.0), (1, 20.0), (2, 30.0)],
            &measured,
        );
        let last = |row: u32| out.iter().rev().find(|(r, _)| *r == row).unwrap().1;
        assert_eq!((last(0), last(1), last(2)), (20.0, 40.0, 30.0));
    }

    /// A line break grows a row with no style at all, as LibreOffice grows it.
    #[test]
    fn a_line_break_grows_an_unstyled_row() {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "one\ntwo\nthree", RecalcMode::Document)
            .unwrap();
        assert_eq!(
            grown_rows(&app, 0, &|_| 30.0, 0.0, 0.0, 1.0, &Fixed),
            [(0, 3.0)]
        );
    }

    /// Turned text needs the turned box: upright, a run of ten one-wide characters is ten tall.
    #[test]
    fn turned_text_grows_its_row_to_the_turned_box() {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "abcdefghij", RecalcMode::Document)
            .unwrap();
        let style = crate::style::CellStyle {
            rotation: Some("90".into()),
            ..Default::default()
        };
        app.set_style(0, Pos::new(0, 0), Pos::new(0, 0), Some(style))
            .unwrap();
        assert_eq!(
            grown_rows(&app, 0, &|_| 3.0, 0.0, 0.0, 1.0, &Fixed),
            [(0, 10.0)]
        );
    }

    /// LibreOffice grows a row for a merge on it alone, measured across the merge, and no row for
    /// a merge spanning two (`doc/ods-format.md` §5.4).
    #[test]
    fn a_merge_on_one_row_grows_it_and_a_taller_one_does_not() {
        let app = App::new();
        let words = "one two three four five six seven eight nine";
        let style = crate::style::CellStyle {
            wrap: Some("wrap".into()),
            ..Default::default()
        };
        for row in [0, 2] {
            app.enter(0, Pos::new(row, 0), words, RecalcMode::Document)
                .unwrap();
            app.set_style(0, Pos::new(row, 0), Pos::new(row, 0), Some(style.clone()))
                .unwrap();
        }
        app.merge(0, Pos::new(0, 0), Pos::new(0, 3)).unwrap();
        app.merge(0, Pos::new(2, 0), Pos::new(3, 3)).unwrap();
        let narrow = grown_rows(&app, 0, &|_| 5.0, 1.0, 0.0, 1.0, &Fixed);
        assert_eq!(narrow.iter().map(|g| g.0).collect::<Vec<_>>(), [0]);
        // Four columns of 12 hold the line that one column of 12 would wrap.
        let wide = grown_rows(&app, 0, &|_| 12.0, 1.0, 0.0, 1.0, &Fixed);
        assert_eq!(wide, [], "laid out across the merge's columns");
    }
}
