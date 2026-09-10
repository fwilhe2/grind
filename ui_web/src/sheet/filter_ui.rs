// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The autofilter's dropdown (§9.4): what values a field's popover offers, and what choosing
//! among them means.
//!
//! **Portable, and tested on the host** like [`super::keymap`] and [`super::assist`] — nothing
//! here touches the DOM. `mod.rs` builds the popover's checkboxes from [`field_values`] and
//! turns what was ticked into a [`Chosen`], the same split `ui_sheet_gtk/src/filter_ui.rs`
//! makes between the core's own question ("what does a field's column hold") and the widget
//! that asks it.
//!
//! The core decides what a filter *means* (`grind_sheet::filter`); this decides only what a
//! field's list should offer. The values are read from the same [`grind_sheet::Viewport`] the
//! grid itself draws from, so the list can never offer a value that would then match nothing.

use std::collections::BTreeSet;

use grind_sheet::{Filter, Viewport};

/// How many distinct values a field's list will show.
///
/// ponytail: a column with more than this many distinct values gets a truncated list and no
/// way to reach the rest — the same ceiling `ui_sheet_gtk`'s popover has, for the same reason:
/// a search box over the values is what makes a list this long usable at all, and is worth
/// building the first time a real document wants it.
pub const MAX_VALUES: usize = 500;

/// What the empty cell is labelled in the list. A blank row would read as a rendering bug, and
/// the value behind it really is the empty string (`grind_sheet::filter`).
pub const EMPTY_LABEL: &str = "(empty)";

/// The distinct values a field's column holds, in the order the list shows them.
///
/// A `BTreeSet` so the order is the model's own — [`Filter::keep`] stores its values the same
/// way, and a list that reordered itself between openings is its own small bug. Includes the
/// values the filter is *currently* hiding: unchecking one only works if it is still offered.
pub fn field_values(cells: &Viewport, filter: &Filter, field: u32) -> Vec<String> {
    let col = filter.column(field);
    (filter.first_data_row()..=filter.end.row)
        .filter_map(|row| cells.text(row, col))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_VALUES)
        .collect()
}

/// What the popover decided, handed back to whoever opened it.
pub enum Chosen {
    /// Keep exactly these values in this field.
    Keep(BTreeSet<String>),
    /// Drop this field's condition — every value shows again.
    Clear,
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::{App, CellValue, Pos};

    /// The list is the column's distinct display text — deduplicated, in the model's own
    /// order, and *including* the values the filter is currently hiding.
    #[test]
    fn the_list_offers_every_value_the_column_holds() {
        let app = App::new();
        for (row, product) in ["Product", "Desk", "Chair", "Desk", ""].iter().enumerate() {
            app.set_cell(
                0,
                Pos::new(row as u32, 1),
                CellValue::Text((*product).to_owned()),
            )
            .unwrap();
        }
        let mut filter = Filter::new("f", Pos::new(0, 1), Pos::new(4, 1));
        // Chair is filtered out, so its rows are hidden — and it still has to be offered.
        filter.keep.insert(0, ["Desk".to_owned()].into());

        let cells = app.get_viewport(0, 0..5, 0..3).unwrap();
        assert_eq!(
            field_values(&cells, &filter, 0),
            vec!["".to_owned(), "Chair".to_owned(), "Desk".to_owned()],
            "the heading is not one of its own values, and an empty cell is one"
        );
    }

    /// The strings offered are the ones the core matches on, which is the whole contract
    /// between this module and `grind_sheet::filter`.
    #[test]
    fn every_offered_value_matches_something() {
        let app = App::new();
        app.set_cell(0, Pos::new(0, 0), "Price").unwrap();
        // A number, so the display text is a rendering rather than the stored value.
        app.set_cell(0, Pos::new(1, 0), CellValue::Number(1.5))
            .unwrap();
        app.set_cell(0, Pos::new(2, 0), CellValue::Number(2.0))
            .unwrap();
        let filter = Filter::new("f", Pos::new(0, 0), Pos::new(2, 0));
        let cells = app.get_viewport(0, 0..3, 0..1).unwrap();

        for value in field_values(&cells, &filter, 0) {
            let mut one = filter.clone();
            one.keep.insert(0, [value.clone()].into());
            let hidden = one.hidden_rows(
                &grind_sheet::read_bytes(
                    "x.fods",
                    &app.save_bytes(grind_sheet::Form::Flat).unwrap(),
                )
                .unwrap()
                .sheets[0],
                0,
            );
            assert!(
                hidden.len() < 2,
                "keeping {value:?} hid every data row, so the list offered a value the \
                 filter cannot match"
            );
        }
    }
}
