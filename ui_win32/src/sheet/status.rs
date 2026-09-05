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
//! The aggregates go through [`grind_sheet::App::preview`] over generated formulas — `SUM`,
//! `COUNTA` and `AVERAGE` — rather than through a second summing loop, so what the bar says and
//! what a cell would say cannot differ. `ui_sheet_gtk/src/chrome.rs` does the same for the same
//! reason; the shape below is deliberately its shape.

use grind_sheet::model::CellValue;
use grind_sheet::{App, Pos, a1};

use super::keymap::Selection;

/// Where the selection is, and what it adds up to.
///
/// `A1` for a single cell; `B2:C4 · Sum 21215.51 · Count 6 · Average 3535.9` for a range that
/// holds something; just the address for one that does not. A single cell is left alone
/// deliberately: it has nothing to add up and every other spreadsheet stays quiet about it.
pub fn selection_text(app: &App, sheet: usize, selection: Selection) -> String {
    let (start, end) = selection.rect();
    if selection.is_single() {
        return a1::format(None, start);
    }
    let address = format!("{}:{}", a1::format(None, start), a1::format(None, end));
    let Ok((rows, cols)) = app.used_extent(sheet) else {
        return address;
    };
    // Clamped to the used extent first: a whole-column selection — which is what clicking a
    // header gives — must not ask the evaluator to walk a million empty rows.
    let Some((start, end)) = clamp(start, end, rows, cols) else {
        return address;
    };

    let range = format!("[.{}:.{}]", a1::format(None, start), a1::format(None, end));
    // Evaluated at a cell one past the used extent: a formula is evaluated *as if* it sat
    // somewhere, and anywhere inside the range would be a circular reference.
    let at = Pos::new(rows, 0);
    let of = |formula: String| match app.preview(sheet, at, &formula) {
        Ok(CellValue::Number(n)) => Some(n),
        _ => None,
    };
    // A status bar's Count is non-empty rather than numeric, which is `COUNTA`.
    let count = of(format!("=COUNTA({range})")).unwrap_or(0.0);
    if count == 0.0 {
        return address;
    }
    let mut parts = vec![address, format!("Count {}", show(count))];
    // Sum and Average of no numbers are not zero, they are nothing — `AVERAGE` says so with
    // `#DIV/0!`, which is why both are read back as an optional number and offered together.
    if let Some(sum) = of(format!("=SUM({range})"))
        && let Some(average) = of(format!("=AVERAGE({range})"))
    {
        parts.insert(1, format!("Sum {}", show(sum)));
        parts.push(format!("Average {}", show(average)));
    }
    parts.join("  \u{00b7}  ")
}

/// The selection's rectangle, cut down to what the sheet actually uses, or `None` when the two
/// do not overlap at all.
///
/// Separate from [`selection_text`] because it is the part with an off-by-one in it: `rows` and
/// `cols` are *one past* the last used track, and a sheet that uses nothing gives zero for both.
pub fn clamp(start: Pos, end: Pos, rows: u32, cols: u32) -> Option<(Pos, Pos)> {
    if rows == 0 || cols == 0 || start.row >= rows || start.col >= cols {
        return None;
    }
    let end = Pos::new(end.row.min(rows - 1), end.col.min(cols - 1));
    (end.row >= start.row && end.col >= start.col).then_some((start, end))
}

/// What the status bar says as a whole: which sheet, how big it is, and the selection.
pub fn status_line(app: &App, sheet: usize, selection: Selection) -> String {
    let name = app
        .sheet_name(sheet)
        .unwrap_or_else(|_| String::from("Sheet1"));
    let (rows, cols) = app.used_extent(sheet).unwrap_or((0, 0));
    format!(
        "{name}  ({} of {})   {rows} \u{00d7} {cols} used   {}",
        sheet + 1,
        app.sheet_count(),
        selection_text(app, sheet, selection)
    )
}

/// What the name box shows for a selection: what it is called, or where it is.
pub fn name_box_text(app: &App, sheet: usize, selection: Selection) -> String {
    name_of(app, sheet, selection).unwrap_or_else(|| a1::format(None, selection.active))
}

/// What the formula bar shows: the active cell as it would be typed in.
///
/// [`grind_sheet::App::input_text`] and nothing else, which is the whole point of it being one
/// line here rather than a rule of this shell's own: a formula comes back in **display syntax**
/// (`=SUM(B2:B4)`, not ODF's `=SUM([.B2:.B4])`), a date comes back in the ISO spelling that can
/// be typed straight back in, and text that would otherwise be read as a number comes back with
/// its leading `\'`. What the bar shows is therefore exactly what [`super::state::to_store`]
/// takes, and the two cannot drift.
pub fn formula_bar_text(app: &App, sheet: usize, selection: Selection) -> String {
    app.input_text(sheet, selection.active).unwrap_or_default()
}

/// The defined name covering *exactly* this selection, if there is one.
///
/// Exactly, not overlapping: a name is a handle on one range, and offering it for a selection
/// that merely sits inside would put a word in the box that typing back would move the
/// selection somewhere else.
fn name_of(app: &App, sheet: usize, selection: Selection) -> Option<String> {
    let want = selection.rect();
    app.names().into_iter().find_map(|(name, expression)| {
        let reference = a1::parse(strip_brackets(&expression)).ok()?;
        let (found, start, end) = a1::resolve(app, &reference).ok()?;
        (found == sheet && (start, end) == want).then_some(name)
    })
}

/// Where a typed address or name points, as a selection — the other half of the name box.
///
/// `None` means "this is not a place on this sheet", and the window's answer to that is to put
/// the old text back rather than to move anywhere.
pub fn locate(app: &App, sheet: usize, text: &str) -> Option<Selection> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // A defined name first: it is document-level, and a name that is also an address is refused
    // by the core when it is defined, so this cannot shadow a cell.
    let expression = app
        .names()
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
        .map(|(_, expression)| expression);
    let reference = match &expression {
        Some(expression) => a1::parse(strip_brackets(expression)).ok()?,
        // A bare word is a *whole column* to the grammar — `foo` is `[.FOO]`, column 4460 — and
        // taking that literally means no three-letter word can ever be a name, since every one
        // of them is a column up to `XFD`. A name box wants the other reading: `A1` and
        // `Data.B2` are places, `A:A` and `3:3` are the whole column and the whole row, and
        // `foo` is a name.
        None => match a1::parse(text).ok()? {
            reference if text.contains(':') || is_a_cell(&reference) => reference,
            _ => return None,
        },
    };
    let (found, start, end) = a1::resolve(app, &reference).ok()?;
    // Going to another sheet is the sheet tabs' job (W3), not the name box's, so a name living
    // elsewhere is refused rather than silently landing on the wrong sheet.
    //
    // The active cell is the range's *start*: going to a range means looking at the top of it,
    // and the active cell is what the window scrolls into view.
    (found == sheet).then_some(Selection {
        anchor: end,
        active: start,
    })
}

/// Whether a reference names both axes at both ends — the difference between `A1` (a place)
/// and `A` (a whole column, and therefore a word).
fn is_a_cell(reference: &grind_sheet::formula::lex::Reference) -> bool {
    std::iter::once(&reference.start)
        .chain(reference.end.as_ref())
        .all(|end| end.row.is_some() && end.col.is_some())
}

/// A stored definition without ODF's brackets — `[$Sheet1.$A$1]` is how a reference is written
/// in a file, and `a1::parse` takes the address a person types.
fn strip_brackets(expression: &str) -> &str {
    expression
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(expression)
}

fn show(n: f64) -> String {
    grind_sheet::formula::value::format_number(n)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Define a name over an address, in the sheet-qualified absolute spelling a definition
    /// has to have (§5.11) — `a1::as_definition`'s, so the test cannot disagree with the core
    /// about how a name is written.
    fn name(app: &App, name: &str, address: &str) {
        let reference = a1::parse(address).unwrap();
        let definition = a1::as_definition(app, &reference).unwrap();
        app.set_name(name, &definition).unwrap();
    }

    fn from(a: (u32, u32), b: (u32, u32)) -> Selection {
        Selection {
            anchor: Pos::new(a.0, a.1),
            active: Pos::new(b.0, b.1),
        }
    }

    #[test]
    fn one_cell_is_an_address_and_nothing_else() {
        let app = book();
        assert_eq!(
            selection_text(&app, 0, Selection::at(Pos::new(1, 1))),
            "B2",
            "a single cell has nothing to add up"
        );
    }

    #[test]
    fn a_range_of_numbers_is_summed_counted_and_averaged() {
        let app = book();
        let text = selection_text(&app, 0, from((1, 1), (3, 1)));
        assert_eq!(
            text,
            "B2:B4  \u{b7}  Sum 60  \u{b7}  Count 3  \u{b7}  Average 20"
        );
    }

    /// The aggregates come from the evaluator, so a range holding text counts it and does not
    /// sum it — `COUNTA` is non-empty, `SUM` ignores text. Getting this from `App::preview`
    /// rather than from a loop here is what makes that true without this file knowing it.
    #[test]
    fn a_label_is_counted_and_not_summed() {
        let app = book();
        let text = selection_text(&app, 0, from((0, 1), (3, 1)));
        assert!(text.starts_with("B1:B4"), "{text}");
        assert!(text.contains("Sum 60"), "{text}");
        assert!(text.contains("Count 4"), "{text}");
        assert!(text.contains("Average 20"), "{text}");
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

    #[test]
    fn a_range_holding_nothing_is_just_an_address() {
        let app = book();
        assert_eq!(selection_text(&app, 0, from((6, 4), (8, 5))), "E7:F9");
    }

    /// Clicking a column header selects a million rows. Reading them would walk the whole
    /// sheet, so the range handed to the evaluator is the used extent's.
    #[test]
    fn a_whole_column_selection_is_clamped_to_what_is_used() {
        let app = book();
        let text = selection_text(&app, 0, Selection::whole_col(1));
        assert!(
            text.starts_with("B1:B1048576"),
            "the address is honest: {text}"
        );
        assert!(text.contains("Sum 60"), "the arithmetic is not: {text}");
    }

    #[test]
    fn the_clamp_answers_nothing_when_the_selection_is_past_the_end() {
        assert_eq!(
            clamp(Pos::new(0, 0), Pos::new(9, 9), 4, 2),
            Some((Pos::new(0, 0), Pos::new(3, 1)))
        );
        assert_eq!(clamp(Pos::new(5, 0), Pos::new(9, 9), 4, 2), None);
        assert_eq!(clamp(Pos::new(0, 0), Pos::new(9, 9), 0, 0), None);
    }

    #[test]
    fn the_status_line_names_the_sheet_and_its_extent() {
        let app = book();
        let line = status_line(&app, 0, Selection::at(Pos::new(1, 1)));
        assert!(line.contains("(1 of 1)"), "{line}");
        assert!(line.contains("4 \u{d7} 2 used"), "{line}");
        assert!(line.ends_with("B2"), "{line}");
    }

    #[test]
    fn the_name_box_shows_the_address_when_nothing_is_named() {
        let app = book();
        assert_eq!(name_box_text(&app, 0, Selection::at(Pos::new(3, 1))), "B4");
    }

    #[test]
    fn a_name_over_exactly_the_selection_is_what_the_box_shows() {
        let app = book();
        name(&app, "widgets", "B2:B4");
        assert_eq!(name_box_text(&app, 0, from((1, 1), (3, 1))), "widgets");
        // One cell short of it is not it: a word in the box has to be a word that, typed back,
        // selects what is selected.
        // ...and the box falls back to where the *active* cell is.
        assert_eq!(name_box_text(&app, 0, from((1, 1), (2, 1))), "B3");
    }

    #[test]
    fn an_address_typed_into_the_box_is_a_place() {
        let app = book();
        let to = locate(&app, 0, "C7").unwrap();
        assert_eq!(to.active, Pos::new(6, 2));
        assert!(to.is_single());
        // Case and space are a person typing, not a different address.
        assert_eq!(locate(&app, 0, "  c7 ").unwrap().active, Pos::new(6, 2));
    }

    #[test]
    fn a_range_typed_into_the_box_selects_it_from_the_top() {
        let app = book();
        let to = locate(&app, 0, "B2:C4").unwrap();
        assert_eq!(to.active, Pos::new(1, 1), "the active cell is the top-left");
        assert_eq!(to.rect(), (Pos::new(1, 1), Pos::new(3, 2)));
    }

    /// The ambiguity this box exists to resolve, pinned: a word is a name, an address is a
    /// place, and a whole column has to be asked for as a range. Without the last rule every
    /// word up to `XFD` would silently be a column instead of a name.
    #[test]
    fn a_word_is_a_name_and_a_column_has_to_be_asked_for_as_a_range() {
        let app = book();
        assert_eq!(locate(&app, 0, "widgets"), None, "not defined yet");
        name(&app, "widgets", "B2:B4");
        assert_eq!(
            locate(&app, 0, "widgets").unwrap().rect(),
            (Pos::new(1, 1), Pos::new(3, 1))
        );
        // `B` alone is a word, not column B; `B:B` is column B, clamped to the used extent.
        assert_eq!(locate(&app, 0, "B"), None);
        assert_eq!(
            locate(&app, 0, "B:B").unwrap().rect(),
            (Pos::new(0, 1), Pos::new(3, 1))
        );
    }

    #[test]
    fn nonsense_goes_nowhere() {
        let app = book();
        for text in ["", "   ", "!!", "A0", "Sheet9.A1"] {
            assert_eq!(locate(&app, 0, text), None, "{text:?}");
        }
    }
}
