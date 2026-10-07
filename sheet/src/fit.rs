// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fitting tracks to what is in them — a column as wide as its widest text, a row given back to
//! its content.
//!
//! The measuring is a font question and the core has no font, so a shell hands in its
//! `Metrics` the way it does for [`crate::autoheight`]; what is decided here is everything
//! else, so six clients cannot answer it six ways: *which* text a column is measured by (what
//! each cell displays, in the face its own style names), which columns a selection means (the
//! ones in use — a whole row selected is sixteen thousand columns and no text past the used
//! extent), and what writing the answer is ([`App::fit`]: one undo step for the whole gesture,
//! a row "fitted" by taking its own height away, since a row without one already grows to
//! hold what is in it).
//!
//! Hoisted out of the macOS shell (`ui_mac/src/sheet/resize.rs`) when the CLI wanted the same
//! answer — rule 4: whatever a window can do, `grind sheet fit` can.

use std::ops::Range;

use grind_core::layout::Metrics;
use grind_core::style::TextStyle;

use crate::{App, look};

/// The widest text in `col`, in `metrics`' own unit — each cell's displayed text in the face
/// its own style sets it in, as `style` turns that into a [`TextStyle`] (normally
/// [`look::text_style`]; a caller with no family of its own may name one). Zero when the column
/// holds nothing to show.
pub fn widest(
    app: &App,
    sheet: usize,
    col: u32,
    metrics: &dyn Metrics,
    style: &dyn Fn(Option<&crate::style::CellStyle>) -> TextStyle,
) -> f64 {
    let rows = app.used_extent(sheet).map_or(0, |(rows, _)| rows);
    let mut advances = Vec::new();
    app.get_viewport(sheet, 0..rows, col..col + 1)
        .map(|viewport| {
            (0..rows)
                .filter_map(|row| {
                    let text = viewport.text(row, col).filter(|text| !text.is_empty())?;
                    advances.clear();
                    metrics.advances(text, &style(viewport.style(row, col)), &mut advances);
                    advances.last().copied().map(f64::from)
                })
                .fold(0.0, f64::max)
        })
        .unwrap_or(0.0)
}

/// [`widest`] in the face [`look::text_style`] gives each cell — what every shell draws with.
pub fn widest_drawn(app: &App, sheet: usize, col: u32, metrics: &dyn Metrics) -> f64 {
    widest(app, sheet, col, metrics, &look::text_style)
}

/// The columns of `cols` worth fitting: those up to the used extent, and never none — a
/// selection wholly past the used extent still fits its first column, which [`width`] gives
/// back to the default.
pub fn columns_in_use(app: &App, sheet: usize, cols: Range<u32>) -> Range<u32> {
    let used = app.used_extent(sheet).map_or(0, |(_, cols)| cols);
    cols.start..cols.end.min(used.max(cols.start + 1))
}

/// Every column in use on the sheet — what *Fit Content to Cells* fits.
pub fn all_columns(app: &App, sheet: usize) -> Range<u32> {
    0..app.used_extent(sheet).map_or(0, |(_, cols)| cols)
}

/// The width a column fits to, as the length a document stores, from its [`widest`] text and
/// the room `pad` a shell keeps beside it — both in the shell's unit, `mm_per_unit` of a
/// millimetre each. **An empty column goes back to the default** (`None`), as LibreOffice's
/// optimal width does, rather than shrinking to a sliver nobody can click.
pub fn width(widest: f64, pad: f64, mm_per_unit: f64) -> Option<String> {
    (widest > 0.0).then(|| crate::style::mm_length((widest + pad) * mm_per_unit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pos, RecalcMode};
    use grind_core::layout::Fixed;

    fn app() -> App {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "short", RecalcMode::Document)
            .unwrap();
        app.enter(
            0,
            Pos::new(1, 0),
            "a much longer label",
            RecalcMode::Document,
        )
        .unwrap();
        app.enter(0, Pos::new(0, 2), "x", RecalcMode::Document)
            .unwrap();
        app
    }

    #[test]
    fn a_column_is_as_wide_as_its_widest_text() {
        let app = app();
        assert_eq!(widest_drawn(&app, 0, 0, &Fixed), 19.0);
        assert_eq!(widest_drawn(&app, 0, 1, &Fixed), 0.0, "B holds nothing");
        assert_eq!(widest_drawn(&app, 0, 2, &Fixed), 1.0);
    }

    #[test]
    fn an_empty_column_goes_back_to_the_default() {
        assert_eq!(width(0.0, 12.0, 1.0), None);
        assert_eq!(width(8.0, 2.0, 2.0).as_deref(), Some("20.000mm"));
    }

    #[test]
    fn a_selection_is_cut_to_the_columns_in_use() {
        let app = app();
        assert_eq!(columns_in_use(&app, 0, 0..16384), 0..3);
        assert_eq!(columns_in_use(&app, 0, 1..2), 1..2);
        assert_eq!(columns_in_use(&app, 0, 40..50), 40..41, "never none");
        assert_eq!(all_columns(&app, 0), 0..3);
    }

    #[test]
    fn a_fit_is_one_undo_step() {
        let app = app();
        app.set_row_height(0, 0..2, Some("1cm".into())).unwrap();
        app.set_row_height(0, 9..10, Some("1cm".into())).unwrap();
        let changed = app
            .fit(
                0,
                vec![
                    (0, width(40.0, 0.0, 1.0)),
                    (1, width(0.0, 4.0, 1.0)),
                    (2, width(1.0, 4.0, 1.0)),
                ],
                0..u32::MAX,
            )
            .unwrap();
        assert_eq!(changed, 2 + 3, "B was already at the default");
        assert!(app.row_heights(0).unwrap().is_empty());
        assert_eq!(app.col_widths(0).unwrap().len(), 2);
        assert!(app.undo());
        assert_eq!(
            app.row_heights(0).unwrap().len(),
            3,
            "one ⌘Z takes it all back"
        );
        assert!(app.col_widths(0).unwrap().is_empty());
    }

    #[test]
    fn rows_outside_the_range_keep_their_height() {
        let app = app();
        app.set_row_height(0, 0..1, Some("1cm".into())).unwrap();
        app.set_row_height(0, 5..6, Some("1cm".into())).unwrap();
        app.fit(0, Vec::new(), 4..8).unwrap();
        assert_eq!(app.row_heights(0).unwrap(), vec![(0, "1cm".to_owned())]);
    }
}
