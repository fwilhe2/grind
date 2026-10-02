// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The autofilter on the grid (§9.4) — portable: where its buttons are, what a choice in a
//! button's list does to the filter, and how a button is drawn.
//!
//! The core decides what a filter *means* (`grind_sheet::filter`) and what a field's list offers
//! (`filter::offered`, the order every shell's list has); this decides only the Mac's half. Data ▸
//! Filter turns one on over the selection — or, from one cell, from it to the end of what the
//! sheet uses, the browser's rule — and off again. Each heading of the range carries a button at
//! its right edge, and a click on it lists the field's values in a menu, ticked when shown:
//! choosing one hides or shows it, and Show All drops the field's condition. A field with a
//! condition has its button filled in the accent, so a filtered column is visible as one.

use std::collections::BTreeSet;

use grind_core::color::Rgb;
use grind_sheet::{Filter, Pos};

use super::geom::{Grid, Rect};
use super::paint::Palette;
use crate::ops::Op;

/// The name LibreOffice gives an autofilter nobody named; `grind sheet filter` and the browser
/// write the same one, so a document does not say which shell made it.
pub const NAME: &str = "__Anonymous_Sheet_DB__0";

/// A button's side, and its gap from the heading cell's right and top edges.
const BUTTON: f64 = 14.0;
const INSET: f64 = 2.0;

/// The rectangle a new filter covers: the selection, or from a single cell to the end of the
/// used part of the sheet — or why there is none, since a filter needs a heading and a row.
pub fn range(selection: (Pos, Pos), used: (u32, u32)) -> Result<(Pos, Pos), &'static str> {
    let (start, mut end) = selection;
    if start == end {
        end = Pos::new(used.0.saturating_sub(1), used.1.saturating_sub(1));
    }
    match end.row > start.row && end.col >= start.col {
        true => Ok((start, end)),
        false => Err("Select the rows to filter, including their headings."),
    }
}

/// Every field's button, in the sheet's own coordinates: at the right of its heading cell,
/// vertically centred. None when the document turned its buttons off, and none for a hidden
/// column, which has no cell to put one in.
pub fn buttons(filter: &Filter, grid: &Grid) -> Vec<(u32, Rect)> {
    if !filter.buttons {
        return Vec::new();
    }
    (0..=filter.end.col.saturating_sub(filter.start.col))
        .filter_map(|field| {
            let cell = grid.cell(filter.start.row, filter.column(field));
            if cell.w < BUTTON + 2.0 * INSET || cell.h <= 0.0 {
                return None;
            }
            let side = BUTTON.min(cell.h - 2.0 * INSET).max(1.0);
            let rect = Rect::new(
                cell.right() - INSET - side,
                cell.y + (cell.h - side) / 2.0,
                side,
                side,
            );
            Some((field, rect))
        })
        .collect()
}

/// The field whose button is at `(x, y)` on the sheet, if one is.
pub fn button_at(filter: &Filter, grid: &Grid, x: f64, y: f64) -> Option<u32> {
    buttons(filter, grid)
        .into_iter()
        .find(|(_, rect)| x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom())
        .map(|(field, _)| field)
}

/// Whether a value is shown in a field — what its row in the list is ticked by.
pub fn shown(filter: &Filter, field: u32, value: &str) -> bool {
    filter
        .keep
        .get(&field)
        .is_none_or(|kept| kept.contains(value))
}

/// The filter with `value` hidden in `field` if it was shown, or shown if it was hidden, out of
/// the values the field's list `offered`. Keeping every value is no condition at all, so the
/// field's entry goes rather than listing everything.
pub fn toggled(filter: &Filter, field: u32, value: &str, offered: &[String]) -> Filter {
    let mut next = filter.clone();
    let mut kept: BTreeSet<String> = match filter.keep.get(&field) {
        Some(kept) => kept.clone(),
        None => offered.iter().cloned().collect(),
    };
    if !kept.remove(value) {
        kept.insert(value.to_owned());
    }
    match offered.iter().all(|value| kept.contains(value)) {
        true => next.keep.remove(&field),
        false => next.keep.insert(field, kept),
    };
    next
}

/// The filter with `field`'s condition dropped — every value shown again.
pub fn shown_all(filter: &Filter, field: u32) -> Filter {
    let mut next = filter.clone();
    next.keep.remove(&field);
    next
}

/// The buttons that meet `view`: a rounded square with a chevron in it, in the header's ground
/// and ink — or the accent and the page's ground when the field has a condition.
pub fn ops(filter: &Filter, grid: &Grid, view: &Rect, palette: &Palette) -> Vec<Op> {
    let mut ops = Vec::new();
    for (field, rect) in buttons(filter, grid) {
        if rect.intersection(view).is_empty() {
            continue;
        }
        let filtered = filter.keep.contains_key(&field);
        let (ground, ink): (Rgb, Rgb) = match filtered {
            true => (palette.accent, palette.page),
            false => (palette.header, palette.header_ink),
        };
        ops.push(Op::Fill {
            rect,
            color: ground,
        });
        // A downward chevron, a little under half the button wide.
        let (cx, cy, half) = (
            rect.x + rect.w / 2.0,
            rect.y + rect.h / 2.0 + 1.0,
            rect.w * 0.22,
        );
        ops.push(Op::Path {
            points: vec![
                (cx - half, cy - half / 2.0),
                (cx, cy + half / 2.0),
                (cx + half, cy - half / 2.0),
            ],
            fill: None,
            stroke: Some((ink, 1.5)),
        });
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::{App, RecalcMode};

    fn offered() -> Vec<String> {
        ["Chair", "Desk", "Lamp"].map(str::to_owned).to_vec()
    }

    #[test]
    fn a_filter_covers_the_selection_or_runs_from_one_cell_to_the_used_end() {
        let cell = Pos::new(0, 0);
        assert_eq!(
            range((cell, cell), (5, 3)),
            Ok((cell, Pos::new(4, 2))),
            "one cell: to the end of the used part"
        );
        assert_eq!(
            range((cell, Pos::new(3, 1)), (5, 3)),
            Ok((cell, Pos::new(3, 1)))
        );
        assert!(
            range((cell, Pos::new(0, 4)), (5, 5)).is_err(),
            "no data row"
        );
        assert!(
            range((cell, cell), (1, 1)).is_err(),
            "nothing under the heading"
        );
    }

    #[test]
    fn a_choice_hides_or_shows_one_value_and_all_shown_is_no_condition() {
        let filter = Filter::new(NAME, Pos::new(0, 0), Pos::new(3, 1));
        let fewer = toggled(&filter, 1, "Desk", &offered());
        assert_eq!(
            fewer.keep.get(&1),
            Some(&["Chair", "Lamp"].map(str::to_owned).into())
        );
        assert!(!shown(&fewer, 1, "Desk"));
        assert!(shown(&fewer, 0, "anything"), "another field is untouched");
        let back = toggled(&fewer, 1, "Desk", &offered());
        assert_eq!(back.keep.get(&1), None, "every value kept is no condition");
        assert_eq!(shown_all(&fewer, 1), filter);
    }

    #[test]
    fn every_heading_has_a_button_at_its_right_edge() {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "Product", RecalcMode::Document)
            .unwrap();
        let grid = Grid::of(&app, 0);
        let filter = Filter::new(NAME, Pos::new(0, 0), Pos::new(3, 1));
        let buttons = buttons(&filter, &grid);
        assert_eq!(buttons.len(), 2);
        let (field, rect) = buttons[1];
        assert_eq!(field, 1);
        let cell = grid.cell(0, 1);
        assert!(rect.right() <= cell.right() && rect.x > cell.x + cell.w / 2.0);
        let (x, y) = (rect.x + 1.0, rect.y + 1.0);
        assert_eq!(button_at(&filter, &grid, x, y), Some(1));
        assert_eq!(button_at(&filter, &grid, cell.x + 1.0, y), None);
        let hidden = Filter {
            buttons: false,
            ..filter
        };
        assert!(super::buttons(&hidden, &grid).is_empty());
    }

    #[test]
    fn a_filtered_field_is_drawn_in_the_accent() {
        let grid = Grid::of(&App::new(), 0);
        let mut filter = Filter::new(NAME, Pos::new(0, 0), Pos::new(3, 1));
        filter.keep.insert(0, ["Desk".to_owned()].into());
        let view = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let grounds: Vec<Rgb> = ops(&filter, &grid, &view, &Palette::LIGHT)
            .into_iter()
            .filter_map(|op| match op {
                Op::Fill { color, .. } => Some(color),
                _ => None,
            })
            .collect();
        assert_eq!(grounds, [Palette::LIGHT.accent, Palette::LIGHT.header]);
    }
}
