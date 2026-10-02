// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A track's edge dragged in a header band, and one double-clicked — portable.
//!
//! The GNOME window's gesture (`doc/sheet-shell.md`, M8), in this shell's points: the pointer
//! within [`GRAB`] of a column's right edge or a row's bottom one picks that track up, a drag
//! sizes it — and every other selected whole track with it, as one undo step, since dragging
//! three selected headers wider is one gesture — and a double-click fits a column to what it
//! holds or gives a row back to its content. Nothing is written until the button comes up; the
//! drag itself is drawn from [`Sizes::with`], so a hundred motion events are not a hundred undo
//! steps. What is written is an ODF length (`style::mm_length`), so a width survives a round
//! trip as the document's own and never as a number of this screen's points.

use std::ops::Range;

use grind_core::layout::Metrics;
use grind_sheet::nav::Selection;
use grind_sheet::tracks::Sizes;
use grind_sheet::{App, MAX_COLS, MAX_ROWS, look};

use super::geom::PT_PER_MM;
use super::paint::{PAD_X, width};

/// How near an edge, in points, the pointer picks it up — either side of it.
pub const GRAB: f64 = 3.0;

/// The narrowest a drag makes a track: thinner than this is hiding, which is its own verb.
pub const MIN_TRACK: f64 = 6.0;

/// What a fitted column is given beyond its widest text, so the last glyph does not touch the
/// next column's line — the GNOME window's slack.
const FIT_SLACK: f64 = 4.0;

/// Which edge a header band runs along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Columns,
    Rows,
}

/// The track whose far edge is within [`GRAB`] of `at` along a band, of the first `shown` —
/// the nearer one when two are, and never a hidden one, which has no edge of its own to drag.
pub fn edge(sizes: &Sizes, shown: u32, at: f64) -> Option<u32> {
    let inside = sizes.at(at.max(0.0));
    // The track before is the last *shown* one: a hidden track between shares its edge.
    let before = inside.checked_sub(1).and_then(|at| sizes.prev_visible(at));
    [before, Some(inside)]
        .into_iter()
        .flatten()
        .filter(|track| *track < shown && sizes.size_of(*track) > 0.0)
        .map(|track| {
            let end = sizes.offset_of(track) + sizes.size_of(track);
            (track, (end - at).abs())
        })
        .filter(|(_, distance)| *distance <= GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(track, _)| track)
}

/// A track picked up by its edge: where the pointer was and how big the track was then.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    pub axis: Axis,
    pub track: u32,
    pub from: f64,
    pub size: f64,
}

impl Drag {
    /// How big the track is with the pointer at `at`.
    pub fn size_at(&self, at: f64) -> f64 {
        (self.size + at - self.from).max(MIN_TRACK)
    }
}

/// The tracks a drag of `track` sizes: every selected one when the selection is whole tracks
/// along this axis and `track` is among them, and otherwise `track` alone.
pub fn tracks(selection: Selection, axis: Axis, track: u32) -> Range<u32> {
    let (start, end) = selection.rect();
    let (whole, range) = match axis {
        Axis::Columns => (
            start.row == 0 && end.row == MAX_ROWS - 1,
            start.col..end.col + 1,
        ),
        Axis::Rows => (
            start.col == 0 && end.col == MAX_COLS - 1,
            start.row..end.row + 1,
        ),
    };
    match whole && range.contains(&track) {
        true => range,
        false => track..track + 1,
    }
}

/// A size in points as the length a document stores.
pub fn length(points: f64) -> String {
    grind_sheet::style::mm_length(points / PT_PER_MM)
}

/// How wide `col` must be to show everything in it on one line — its widest text in that
/// text's own face, the cell's padding either side and a little slack — and never narrower
/// than [`MIN_TRACK`]. An empty column fits to the narrowest.
pub fn fit_width(app: &App, sheet: usize, col: u32, metrics: &dyn Metrics) -> f64 {
    let rows = app.used_extent(sheet).map_or(0, |(rows, _)| rows);
    let widest = app
        .get_viewport(sheet, 0..rows, col..col + 1)
        .map(|viewport| {
            (0..rows)
                .filter_map(|row| {
                    let text = viewport.text(row, col).filter(|text| !text.is_empty())?;
                    let style = look::text_style(viewport.style(row, col));
                    Some(width(metrics, text, &style))
                })
                .fold(0.0, f64::max)
        })
        .unwrap_or(0.0);
    match widest > 0.0 {
        true => (widest + 2.0 * PAD_X + FIT_SLACK).max(MIN_TRACK),
        false => MIN_TRACK.max(super::geom::COL_W / 4.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::layout::Fixed;
    use grind_sheet::{Pos, RecalcMode};

    fn columns() -> Sizes {
        // A 72, B 100, C hidden, D 72, …
        Sizes::new(72.0, 10, vec![(1, 100.0), (2, 0.0)])
    }

    #[test]
    fn an_edge_is_picked_up_from_either_side_and_a_hidden_track_has_none() {
        let sizes = columns();
        assert_eq!(edge(&sizes, 10, 72.0), Some(0));
        assert_eq!(edge(&sizes, 10, 70.0), Some(0), "just inside A");
        assert_eq!(edge(&sizes, 10, 74.5), Some(0), "just inside B");
        assert_eq!(edge(&sizes, 10, 120.0), None, "the middle of B");
        // B's end and the hidden C's are one line; the edge is B's.
        assert_eq!(edge(&sizes, 10, 172.0), Some(1));
        assert_eq!(edge(&sizes, 1, 172.0), None, "past what is shown");
    }

    #[test]
    fn a_drag_sizes_by_how_far_the_pointer_went_and_no_thinner_than_the_floor() {
        let drag = Drag {
            axis: Axis::Columns,
            track: 1,
            from: 172.0,
            size: 100.0,
        };
        assert_eq!(drag.size_at(200.0), 128.0);
        assert_eq!(drag.size_at(0.0), MIN_TRACK);
    }

    #[test]
    fn a_drag_sizes_every_selected_whole_track_it_is_among() {
        let columns = Selection {
            anchor: Pos::new(MAX_ROWS - 1, 1),
            active: Pos::new(0, 3),
        };
        assert_eq!(tracks(columns, Axis::Columns, 2), 1..4);
        assert_eq!(tracks(columns, Axis::Columns, 5), 5..6, "not among them");
        assert_eq!(tracks(columns, Axis::Rows, 2), 2..3, "not whole rows");
        let cells = Selection::default();
        assert_eq!(tracks(cells, Axis::Columns, 0), 0..1);
    }

    #[test]
    fn a_column_fits_its_widest_text() {
        let app = App::new();
        for (row, text) in ["a", "a much longer label", "mid"].iter().enumerate() {
            app.enter(0, Pos::new(row as u32, 0), text, RecalcMode::Document)
                .unwrap();
        }
        let fitted = fit_width(&app, 0, 0, &Fixed);
        let widest = width(&Fixed, "a much longer label", &Default::default());
        assert_eq!(fitted, widest + 2.0 * PAD_X + FIT_SLACK);
        assert!(
            fit_width(&app, 0, 5, &Fixed) >= MIN_TRACK,
            "an empty column"
        );
    }

    #[test]
    fn a_length_is_what_the_document_stores() {
        let length = length(72.0);
        let mm = grind_sheet::style::length_mm(&length).unwrap();
        assert!((mm - 25.4).abs() < 0.01, "{length}");
    }
}
