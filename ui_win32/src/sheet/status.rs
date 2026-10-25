// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the read-outs say: the name box, the formula bar and the status bar — portable, tested
//! against a real `grind_sheet::App`. The aggregates are `grind_sheet::summary`'s
//! ([`grind_sheet::App::preview`] over generated formulas) and the name box and status bar's words
//! `grind_sheet::place`'s; what is here is the formula bar's text and this window's names for
//! them.

use grind_sheet::{App, place};

use super::keymap::Selection;

/// The status bar's right end: where the selection is and what it adds up to
/// (`grind_sheet::place::status`, nothing for one cell), then how the document spells a number
/// (`grind_sheet::locale::badge`, `1.234,5 · de-DE`), always there. Data ▸ Document Locale…
/// changes it.
pub fn status_right(app: &App, sheet: usize, selection: Selection) -> String {
    place::status_and_numbers(app, sheet, selection)
}

/// What the name box shows for a selection: what it is called, or where it is —
/// `grind_sheet::place::name_box`, shared with every shell's name box.
pub fn name_box_text(app: &App, sheet: usize, selection: Selection) -> String {
    place::name_box(app, sheet, selection)
}

/// What the formula bar shows: the active cell as it would be typed —
/// [`grind_sheet::App::input_text`]: a formula in display syntax, a date in its ISO spelling, text
/// that would read as a number with its leading `\'` — exactly what
/// [`grind_sheet::formula::display::to_input`] takes.
pub fn formula_bar_text(app: &App, sheet: usize, selection: Selection) -> String {
    app.input_text(sheet, selection.active).unwrap_or_default()
}

/// Where a typed address or name points, as a selection — the other half of the name box,
/// `grind_sheet::place::locate`.
pub fn locate(app: &App, sheet: usize, text: &str) -> Option<Selection> {
    place::locate(app, sheet, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::Pos;

    /// A three-by-two block of numbers with a label over it, which is enough for every
    /// aggregate and for the clamp.
    fn book() -> App {
        let app = App::new();
        let recalc = grind_sheet::RecalcMode::Document;
        app.enter(0, Pos::new(0, 1), "Widgets", recalc).unwrap();
        for (row, value) in [(1, "10"), (2, "20"), (3, "30")] {
            app.enter(0, Pos::new(row, 1), value, recalc).unwrap();
        }
        app
    }

    fn from(a: (u32, u32), b: (u32, u32)) -> Selection {
        Selection {
            anchor: Pos::new(a.0, a.1),
            active: Pos::new(b.0, b.1),
        }
    }

    /// The far end of the bar says how this document spells a number, beside the arithmetic or
    /// on its own for one cell.
    #[test]
    fn the_status_bar_ends_with_the_number_spelling() {
        let app = book();
        assert_eq!(
            status_right(&app, 0, Selection::at(Pos::new(1, 1))),
            "1,234.5"
        );
        app.set_locale(grind_sheet::locale::Locale::parse("de-DE"))
            .unwrap();
        let right = status_right(&app, 0, from((1, 1), (3, 1)));
        assert!(
            right.ends_with("  \u{00b7}  1.234,5 \u{00b7} de-DE"),
            "{right}"
        );
        assert!(right.contains("60"), "{right}");
    }

    /// The bar shows what would be typed back in, not what the cell displays — which for a
    /// formula is the display syntax rather than the ODF one the document stores.
    #[test]
    fn the_formula_bar_shows_the_cell_as_it_would_be_typed() {
        let app = book();
        app.enter(
            0,
            Pos::new(4, 1),
            "=SUM([.B2:.B4])",
            grind_sheet::RecalcMode::Document,
        )
        .unwrap();
        assert_eq!(
            formula_bar_text(&app, 0, Selection::at(Pos::new(4, 1))),
            "=SUM(B2:B4)"
        );
        assert_eq!(
            formula_bar_text(&app, 0, Selection::at(Pos::new(1, 1))),
            "10"
        );
        assert_eq!(
            formula_bar_text(&app, 0, Selection::at(Pos::new(0, 1))),
            "Widgets"
        );
        // An empty cell says nothing at all, rather than "0" or a placeholder.
        assert_eq!(formula_bar_text(&app, 0, Selection::at(Pos::new(9, 9))), "");
        // It follows the *active* cell of a range, not its corner.
        assert_eq!(formula_bar_text(&app, 0, from((0, 1), (1, 1))), "10");
    }
}
