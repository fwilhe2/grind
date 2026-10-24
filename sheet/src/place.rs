// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a selection is, as a name box and a status bar say it, and where a typed place points —
//! the name box's other half. Everything resolves through [`crate::a1`].

use crate::nav::Selection;
use crate::{App, a1, summary};

/// What the name box shows for a selection: what it is called, or where its active cell is.
pub fn name_box(app: &App, sheet: usize, selection: Selection) -> String {
    name_of(app, sheet, selection).unwrap_or_else(|| a1::format(None, selection.active))
}

/// What the status bar says about a selection: nothing for one cell (the name box says where it
/// is), otherwise [`summary::status_text`]'s sentence, `B2:C4 · Sum 21215.51 · Count 6 · Average
/// 3535.9`.
pub fn status(app: &App, sheet: usize, selection: Selection) -> String {
    if selection.is_single() {
        return String::new();
    }
    let (start, end) = selection.rect();
    summary::status_text(app, sheet, start, end)
}

/// [`status`], then how the document spells a number (`locale::badge`, `1.234,5 · de-DE`), for a
/// status bar with one string.
pub fn status_and_numbers(app: &App, sheet: usize, selection: Selection) -> String {
    let badge = crate::locale::badge(app.locale().as_ref());
    match status(app, sheet, selection) {
        arithmetic if arithmetic.is_empty() => badge,
        arithmetic => format!("{arithmetic}  \u{00b7}  {badge}"),
    }
}

/// The defined name covering exactly this selection, if any. Exactly: a name inside which the
/// selection merely sits would move the selection if typed back.
fn name_of(app: &App, sheet: usize, selection: Selection) -> Option<String> {
    let want = selection.rect();
    app.names().into_iter().find_map(|(name, expression)| {
        let reference = a1::parse(strip_brackets(&expression)).ok()?;
        let (found, start, end) = a1::resolve(app, &reference).ok()?;
        (found == sheet && (start, end) == want).then_some(name)
    })
}

/// The formula bar's reading of the active cell through its names — `=tax_rate*subtotal` where
/// `shown` is `=B2*B7` — or `None` when there is no formula or no name in it. For showing only; an
/// edit edits `shown`.
pub fn named_reading(app: &App, sheet: usize, at: crate::Pos, shown: &str) -> Option<String> {
    app.named_formula(sheet, at)
        .ok()
        .flatten()
        .filter(|reading| reading != shown)
}

/// Where a typed address or name points, as a selection; `None` when it is not a place on this
/// sheet, and a shell puts the old text back.
pub fn locate(app: &App, sheet: usize, text: &str) -> Option<Selection> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // A defined name first; a name that is also an address is refused when defined.
    let expression = app
        .names()
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
        .map(|(_, expression)| expression);
    let reference = match &expression {
        Some(expression) => a1::parse(strip_brackets(expression)).ok()?,
        // A bare word is a whole column to the grammar (`foo` is column 4460), which would leave
        // no three-letter word as a name. A name box reads `A1` and `Data.B2` as places, `A:A` and
        // `3:3` as a whole column or row, and `foo` as a name.
        None => match a1::parse(text).ok()? {
            reference if text.contains(':') || is_a_cell(&reference) => reference,
            _ => return None,
        },
    };
    let (found, start, end) = a1::resolve(app, &reference).ok()?;
    // A name on another sheet is refused rather than landing on the wrong sheet. The active cell
    // is the range's start, which the grid scrolls into view.
    (found == sheet).then_some(Selection {
        anchor: end,
        active: start,
    })
}

/// Whether a reference names both axes at both ends: `A1` is a place, `A` a word.
fn is_a_cell(reference: &crate::formula::lex::Reference) -> bool {
    std::iter::once(&reference.start)
        .chain(reference.end.as_ref())
        .all(|end| end.row.is_some() && end.col.is_some())
}

/// A stored definition without ODF's brackets, for `a1::parse`.
fn strip_brackets(expression: &str) -> &str {
    expression
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(expression)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pos, RecalcMode};

    /// A column of numbers with a label over it, which is enough for every aggregate and for the
    /// clamp.
    fn book() -> App {
        let app = App::new();
        app.enter(0, Pos::new(0, 1), "Widgets", RecalcMode::Document)
            .unwrap();
        for (row, value) in [(1, "10"), (2, "20"), (3, "30")] {
            app.enter(0, Pos::new(row, 1), value, RecalcMode::Document)
                .unwrap();
        }
        app
    }

    /// Define a name over an address in the spelling a definition needs (`a1::as_definition`).
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
    fn one_cell_leaves_the_status_bar_quiet() {
        let app = book();
        assert_eq!(status(&app, 0, Selection::at(Pos::new(1, 1))), "");
        assert!(status(&app, 0, from((1, 1), (3, 1))).contains("Sum 60"));
    }

    /// A column header selects a million rows; the evaluator gets the used extent's range.
    #[test]
    fn a_whole_column_selection_is_clamped_to_what_is_used() {
        let text = status(&book(), 0, Selection::whole_col(1));
        assert!(text.starts_with("B1:B1048576"), "{text}");
        assert!(text.contains("Sum 60"), "{text}");
    }

    #[test]
    fn the_name_box_shows_a_name_only_when_the_selection_is_exactly_it() {
        let app = book();
        assert_eq!(name_box(&app, 0, Selection::at(Pos::new(3, 1))), "B4");
        name(&app, "widgets", "B2:B4");
        assert_eq!(name_box(&app, 0, from((1, 1), (3, 1))), "widgets");
        // One cell short of it is not it, and the box falls back to the *active* cell.
        assert_eq!(name_box(&app, 0, from((1, 1), (2, 1))), "B3");
    }

    #[test]
    fn an_address_is_a_place_and_a_range_is_selected_from_the_top() {
        let app = book();
        let to = locate(&app, 0, "C7").unwrap();
        assert_eq!(to.active, Pos::new(6, 2));
        assert!(to.is_single());
        assert_eq!(locate(&app, 0, "  c7 ").unwrap().active, Pos::new(6, 2));
        let range = locate(&app, 0, "B2:C4").unwrap();
        assert_eq!(
            range.active,
            Pos::new(1, 1),
            "the active cell is the top left"
        );
        assert_eq!(range.rect(), (Pos::new(1, 1), Pos::new(3, 2)));
    }

    /// A word is a name, an address a place, and a whole column must be asked for as a range.
    #[test]
    fn a_word_is_a_name_and_a_column_has_to_be_asked_for_as_a_range() {
        let app = book();
        for word in ["widgets", "foo", "abc", "Total", "B"] {
            assert_eq!(
                locate(&app, 0, word),
                None,
                "{word} is a name, not a column"
            );
        }
        name(&app, "widgets", "B2:B4");
        assert_eq!(
            locate(&app, 0, "widgets").unwrap().rect(),
            (Pos::new(1, 1), Pos::new(3, 1))
        );
        assert_eq!(
            locate(&app, 0, "B:B").unwrap().rect(),
            (Pos::new(0, 1), Pos::new(3, 1)),
            "a column, clamped to the used extent"
        );
    }

    #[test]
    fn a_formula_is_read_through_its_names_only_when_it_uses_one() {
        let app = book();
        let recalc = crate::RecalcMode::Document;
        app.enter(0, Pos::new(5, 1), "=[.B2]*2", recalc).unwrap();
        let shown = app.input_text(0, Pos::new(5, 1)).unwrap();
        assert_eq!(named_reading(&app, 0, Pos::new(5, 1), &shown), None);
        name(&app, "tax_rate", "B2");
        assert_eq!(
            named_reading(&app, 0, Pos::new(5, 1), &shown).as_deref(),
            Some("=tax_rate*2")
        );
        assert_eq!(
            named_reading(&app, 0, Pos::new(1, 1), "10"),
            None,
            "a value"
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
