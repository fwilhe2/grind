// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the two read-outs say: the name box, and the status bar.
//!
//! **Portable, and tested against a real `grind_sheet::App`** — which is the reason this is a
//! file of its own rather than three lines in `win.rs`. `App` compiles and runs on Linux, so
//! everything here can be asserted on the development machine against a document built in the
//! test, with no window anywhere. What is left for the window is putting the string on screen.
//!
//! The aggregates are `grind_sheet::summary`'s — [`grind_sheet::App::preview`] over generated
//! formulas, hoisted out of this file, `ui_sheet_gtk/src/chrome.rs` and `ui_tui` when the macOS
//! shell was about to be the fourth caller (`doc/macos-shell.md`, M1) — and the name box's two
//! halves and the status bar's quiet single cell are `grind_sheet::place`'s, hoisted from here and
//! the GNOME window's `chrome.rs` when the Mac's name box would have been the third. What is left
//! here is the formula bar's text and the names this window calls them by.

use grind_sheet::{App, place};

use super::keymap::Selection;

/// Where the selection is, and what it adds up to — `grind_sheet::place::status`, which is
/// nothing for a single cell and `summary`'s sentence otherwise.
pub fn selection_text(app: &App, sheet: usize, selection: Selection) -> String {
    place::status(app, sheet, selection)
}

/// What the name box shows for a selection: what it is called, or where it is —
/// `grind_sheet::place::name_box`, shared with every shell's name box.
pub fn name_box_text(app: &App, sheet: usize, selection: Selection) -> String {
    place::name_box(app, sheet, selection)
}

/// What the formula bar shows: the active cell as it would be typed in.
///
/// [`grind_sheet::App::input_text`] and nothing else, which is the whole point of it being one
/// line here rather than a rule of this shell's own: a formula comes back in **display syntax**
/// (`=SUM(B2:B4)`, not ODF's `=SUM([.B2:.B4])`), a date comes back in the ISO spelling that can
/// be typed straight back in, and text that would otherwise be read as a number comes back with
/// its leading `\'`. What the bar shows is therefore exactly what
/// [`grind_sheet::formula::display::to_input`] takes, and the two cannot drift.
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
