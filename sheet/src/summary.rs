// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a selection adds up to — the Sum, Count and Average a status bar shows. Computed through
//! [`App::preview`] over generated `SUM`, `COUNTA` and `AVERAGE` formulas, so the status bar and a
//! cell holding `=SUM(…)` cannot differ; `grind sheet eval` reaches the same numbers.

use crate::model::CellValue;
use crate::{App, Pos, a1};

/// What a range holding something adds up to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    /// How many cells hold anything — `COUNTA`, since a status bar's Count is non-empty cells.
    pub count: f64,
    /// `SUM` and `AVERAGE`, together or not at all: no numbers have no average (`#DIV/0!`).
    pub totals: Option<(f64, f64)>,
}

/// What the rectangle `start..=end` of `sheet` adds up to, or `None` when it holds nothing.
/// Clamped to the used extent first, since a header click selects a million rows.
pub fn summarise(app: &App, sheet: usize, start: Pos, end: Pos) -> Option<Summary> {
    let (rows, cols) = app.used_extent(sheet).ok()?;
    let (start, end) = clamp(start, end, rows, cols)?;
    let range = format!("[.{}:.{}]", a1::format(None, start), a1::format(None, end));
    // Evaluated as if one past the used extent; inside the range would be a circular reference.
    let at = Pos::new(rows, 0);
    let of = |formula: String| match app.preview(sheet, at, &formula) {
        Ok(CellValue::Number(n)) => Some(n),
        _ => None,
    };
    let count = of(format!("=COUNTA({range})")).unwrap_or(0.0);
    if count == 0.0 {
        return None;
    }
    let totals = of(format!("=SUM({range})")).zip(of(format!("=AVERAGE({range})")));
    Some(Summary { count, totals })
}

/// `B2:C4  ·  Sum 21215.51  ·  Count 6  ·  Average 3535.9`, or the address alone for a range
/// holding nothing, with numbers spelled the document's way ([`App::display_number`]). Whether one
/// cell gets a sentence is the shell's decision.
pub fn status_text(app: &App, sheet: usize, start: Pos, end: Pos) -> String {
    let address = format!("{}:{}", a1::format(None, start), a1::format(None, end));
    let Some(summary) = summarise(app, sheet, start, end) else {
        return address;
    };
    let mut parts = vec![address];
    if let Some((sum, _)) = summary.totals {
        parts.push(format!("Sum {}", app.display_number(sum)));
    }
    parts.push(format!("Count {}", app.display_number(summary.count)));
    if let Some((_, average)) = summary.totals {
        parts.push(format!("Average {}", app.display_number(average)));
    }
    parts.join("  \u{00b7}  ")
}

/// The rectangle `start..=end` cut to what the sheet uses, or `None` when they do not overlap.
/// `rows` and `cols` are one past the last used track; an empty sheet gives zero for both.
pub fn clamp(start: Pos, end: Pos, rows: u32, cols: u32) -> Option<(Pos, Pos)> {
    if rows == 0 || cols == 0 || start.row >= rows || start.col >= cols {
        return None;
    }
    let end = Pos::new(end.row.min(rows - 1), end.col.min(cols - 1));
    (end.row >= start.row && end.col >= start.col).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAX_ROWS, RecalcMode};

    /// A three-by-one block of numbers with a label over it, which is enough for every aggregate
    /// and for the clamp.
    fn book() -> App {
        let app = App::new();
        let recalc = RecalcMode::Document;
        app.enter(0, Pos::new(0, 1), "Widgets", recalc).unwrap();
        for (row, value) in [(1, "10"), (2, "20"), (3, "30")] {
            app.enter(0, Pos::new(row, 1), value, recalc).unwrap();
        }
        app
    }

    fn text(app: &App, a: (u32, u32), b: (u32, u32)) -> String {
        status_text(app, 0, Pos::new(a.0, a.1), Pos::new(b.0, b.1))
    }

    #[test]
    fn a_range_of_numbers_is_summed_counted_and_averaged() {
        let app = book();
        assert_eq!(
            text(&app, (1, 1), (3, 1)),
            "B2:B4  \u{b7}  Sum 60  \u{b7}  Count 3  \u{b7}  Average 20"
        );
    }

    /// A label is counted (`COUNTA`) and not summed.
    #[test]
    fn a_label_is_counted_and_not_summed() {
        let app = book();
        assert_eq!(
            summarise(&app, 0, Pos::new(0, 1), Pos::new(3, 1)),
            Some(Summary {
                count: 4.0,
                totals: Some((60.0, 20.0))
            })
        );
    }

    /// Text alone has a count and no totals — not a sum of zero.
    #[test]
    fn a_range_of_labels_has_a_count_and_nothing_to_add() {
        let app = book();
        assert_eq!(text(&app, (0, 1), (0, 1)), "B1:B1  \u{b7}  Count 1");
    }

    /// A German document's sum is spelled the way its cells are.
    #[test]
    fn a_german_document_adds_up_in_german() {
        let app = book();
        app.enter(0, Pos::new(4, 1), "0.5", RecalcMode::Document)
            .unwrap();
        app.set_locale(crate::locale::Locale::parse("de-DE"))
            .unwrap();
        assert_eq!(
            text(&app, (1, 1), (4, 1)),
            "B2:B5  \u{b7}  Sum 60,5  \u{b7}  Count 4  \u{b7}  Average 15,125"
        );
    }

    #[test]
    fn a_range_holding_nothing_is_just_an_address() {
        let app = book();
        assert_eq!(text(&app, (6, 4), (8, 5)), "E7:F9");
        assert_eq!(summarise(&app, 0, Pos::new(6, 4), Pos::new(8, 5)), None);
    }

    /// A header click selects a million rows; the evaluator gets the used extent.
    #[test]
    fn a_whole_column_selection_is_clamped_to_what_is_used() {
        let app = book();
        let text = text(&app, (0, 1), (MAX_ROWS - 1, 1));
        assert!(text.starts_with("B1:B1048576"), "{text}");
        assert!(text.contains("Sum 60"), "{text}");
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
}
