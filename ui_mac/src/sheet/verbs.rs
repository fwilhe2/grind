// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the grid's structural verbs act on, read off the selection — portable, and tested here.
//!
//! Fill Down and Fill Right (⌘D, ⌘R, Excel for Mac's keys), hiding and showing tracks, a track's
//! size, and a defined name's target are each one core call — `App::fill`,
//! `set_row_hidden`/`set_col_hidden`, `set_row_height`/`set_col_width`, `set_name` through
//! `a1::definition` — and the only decision a shell makes is *over what*. That is this file.

use std::ops::Range;

use grind_sheet::Pos;
use grind_sheet::nav::Selection;

/// The fills Fill Down (`down`) or Fill Right makes over `selection`: for each column, its top
/// cell copied down the rest; for each row, its leftmost cell copied across. `(source, start,
/// end)`, the shape `App::fill` takes. Nothing for a selection one cell deep along the fill.
pub fn fills(selection: Selection, down: bool) -> Vec<(Pos, Pos, Pos)> {
    let (start, end) = selection.rect();
    match down {
        true if end.row > start.row => (start.col..=end.col)
            .map(|col| {
                (
                    Pos::new(start.row, col),
                    Pos::new(start.row + 1, col),
                    Pos::new(end.row, col),
                )
            })
            .collect(),
        false if end.col > start.col => (start.row..=end.row)
            .map(|row| {
                (
                    Pos::new(row, start.col),
                    Pos::new(row, start.col + 1),
                    Pos::new(row, end.col),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The rows the selection covers.
pub fn rows(selection: Selection) -> Range<u32> {
    let (start, end) = selection.rect();
    start.row..end.row + 1
}

/// The columns the selection covers.
pub fn cols(selection: Selection) -> Range<u32> {
    let (start, end) = selection.rect();
    start.col..end.col + 1
}

/// The address a name defined over the selection points at — `Sheet1.B2:C9`, sheet-qualified so
/// the name means the same place from every sheet — for `a1::definition` to turn into ODF.
pub fn name_target(sheet: &str, selection: Selection) -> String {
    let (start, end) = selection.rect();
    let one = grind_sheet::a1::format(Some(sheet), start);
    match start == end {
        true => one,
        false => format!("{one}:{}", grind_sheet::a1::format(None, end)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(a: (u32, u32), b: (u32, u32)) -> Selection {
        Selection {
            anchor: Pos::new(a.0, a.1),
            active: Pos::new(b.0, b.1),
        }
    }

    #[test]
    fn fill_down_copies_each_columns_top_cell_and_right_each_rows_first() {
        let b2_c4 = range((1, 1), (3, 2));
        assert_eq!(
            fills(b2_c4, true),
            [
                (Pos::new(1, 1), Pos::new(2, 1), Pos::new(3, 1)),
                (Pos::new(1, 2), Pos::new(2, 2), Pos::new(3, 2)),
            ]
        );
        assert_eq!(fills(b2_c4, false).len(), 3, "a fill per row");
        assert_eq!(
            fills(b2_c4, false)[0],
            (Pos::new(1, 1), Pos::new(1, 2), Pos::new(1, 2))
        );
        let one_row = range((0, 0), (0, 4));
        assert!(fills(one_row, true).is_empty(), "nothing below to fill");
    }

    #[test]
    fn the_tracks_are_the_selections_whichever_way_it_was_made() {
        let backwards = range((5, 3), (2, 1));
        assert_eq!(rows(backwards), 2..6);
        assert_eq!(cols(backwards), 1..4);
    }

    #[test]
    fn a_name_points_at_the_selection_on_its_sheet() {
        let app = grind_sheet::App::new();
        let target = name_target("Sheet1", range((1, 1), (8, 2)));
        assert_eq!(target, "Sheet1.B2:C9");
        assert_eq!(name_target("Sheet1", range((0, 0), (0, 0))), "Sheet1.A1");
        assert!(
            grind_sheet::a1::definition(&app, &target).is_ok(),
            "{target}"
        );
    }
}
