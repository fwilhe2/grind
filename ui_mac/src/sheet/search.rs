// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where Find goes next (M5): `App::find` and `find::step`, turned into the one cell to select —
//! on whichever sheet it is — and how many there are. Portable, and tested here.
//!
//! What a match *is* is `grind_sheet::find`'s: a cell's input text, the formula in display syntax,
//! case folded unless asked; and where Next goes from a cell that is not a hit is `find::step`'s,
//! which the GNOME, Windows and browser find bars already share. This is only the glue the Mac's
//! `performFindPanelAction:` needs.

use grind_sheet::find::{self, Search, Towards};
use grind_sheet::{App, Pos};

/// A hit to go to: where it is, and which of how many.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub sheet: usize,
    pub pos: Pos,
    pub index: usize,
    pub count: usize,
}

/// The hit a step from `at` — a sheet and a cell — lands on, across every sheet, wrapping at
/// either end; `None` when nothing holds `needle`.
pub fn next(app: &App, needle: &str, at: (usize, Pos), towards: Towards) -> Option<Found> {
    let hits = app.find(&Search::new(needle)).ok()?;
    let places: Vec<(usize, Pos)> = hits.iter().map(|hit| (hit.sheet, hit.pos)).collect();
    let index = find::step(&places, at, towards)?;
    let (sheet, pos) = places[index];
    Some(Found {
        sheet,
        pos,
        index,
        count: places.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::RecalcMode;

    fn book() -> App {
        let app = App::new();
        app.add_sheet("Two").unwrap();
        for (sheet, row, text) in [(0, 1, "Rent"), (0, 4, "rent due"), (1, 2, "Rent")] {
            app.enter(sheet, Pos::new(row, 0), text, RecalcMode::Document)
                .unwrap();
        }
        app
    }

    #[test]
    fn next_goes_on_across_sheets_and_wraps() {
        let app = book();
        let first = next(&app, "rent", (0, Pos::new(0, 0)), Towards::Here).unwrap();
        assert_eq!(
            (first.sheet, first.pos, first.index, first.count),
            (0, Pos::new(1, 0), 0, 3)
        );
        let second = next(&app, "rent", (0, first.pos), Towards::Next).unwrap();
        assert_eq!(second.pos, Pos::new(4, 0), "case is folded");
        let third = next(&app, "rent", (0, second.pos), Towards::Next).unwrap();
        assert_eq!(
            (third.sheet, third.pos),
            (1, Pos::new(2, 0)),
            "onto the next sheet"
        );
        let wrapped = next(&app, "rent", (1, third.pos), Towards::Next).unwrap();
        assert_eq!(wrapped.index, 0, "and round again");
        let back = next(&app, "rent", (0, first.pos), Towards::Previous).unwrap();
        assert_eq!(back.sheet, 1, "Previous from the first is the last");
    }

    #[test]
    fn nothing_found_is_none() {
        assert_eq!(
            next(&book(), "mortgage", (0, Pos::new(0, 0)), Towards::Next),
            None
        );
        assert_eq!(next(&book(), "", (0, Pos::new(0, 0)), Towards::Next), None);
    }
}
