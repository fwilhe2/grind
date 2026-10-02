// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the grid's structural verbs act on, read off the selection — portable, and tested here.
//!
//! Hoisted out of the macOS shell, where it was written, when the Windows grid wanted the same
//! answers: every one of them is a decision about *over what*, and four shells deciding it four
//! ways is how a fill came to copy one cell over a whole rectangle.
//!
//! Fill Down and Fill Right (⌘D, ⌘R, Excel for Mac's keys), hiding and showing tracks, a track's
//! size, and a defined name's target are each one core call — `App::fill`,
//! `set_row_hidden`/`set_col_hidden`, `set_row_height`/`set_col_width`, `set_name` through
//! `a1::definition` — and the only decision a shell makes is *over what*. That is this file.

use std::ops::Range;

use crate::formula::display;
use crate::locale::Locale;
use crate::nav::Selection;
use crate::{App, CellValue, Pos};

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
    let one = crate::a1::format(Some(sheet), start);
    match start == end {
        true => one,
        false => format!("{one}:{}", crate::a1::format(None, end)),
    }
}

/// *Formula to Value*: every formula in `start..=end` dropped, each cell keeping the value it last
/// computed (`App::clear_formula`). Returns how many formulas it dropped.
///
/// ponytail: one undo step per formula — the core has no range form, and the trigger is a person
/// converting a large block. The macOS shell wrote the loop; the Windows grid and the browser
/// wanted it too.
pub fn formulas_to_values(app: &App, sheet: usize, start: Pos, end: Pos) -> crate::Result<usize> {
    let mut dropped = 0;
    for row in start.row..=end.row {
        for col in start.col..=end.col {
            let pos = Pos::new(row, col);
            if app.formula(sheet, pos).is_ok_and(|f| f.is_some()) {
                app.clear_formula(sheet, pos)?;
                dropped += 1;
            }
        }
    }
    Ok(dropped)
}

/// What a formula typed in display syntax — with or without its `=` — comes to at `at`, spelled
/// for a sentence, or why it could not be worked out. Nothing is stored and no undo step is made
/// (`App::preview`, `grind sheet eval`'s call): relative references are relative to `at`, the
/// active cell, as they would be typed into it.
pub fn evaluated(app: &App, sheet: usize, at: Pos, typed: &str) -> Result<String, String> {
    let typed = typed.trim();
    let written = match typed.starts_with('=') {
        true => typed.to_owned(),
        false => format!("={typed}"),
    };
    let canonical = display::from_display(&written)
        .map_err(|error| format!("{} (at character {})", error.message, error.at + 1))?;
    let value = app
        .preview(sheet, at, &canonical)
        .map_err(|error| error.to_string())?;
    Ok(match value {
        CellValue::Empty => "nothing".to_owned(),
        // Four decimals at most, in the document's own locale: an answer to read, not to store.
        CellValue::Number(n) => app.display_number((n * 1e4).round() / 1e4),
        CellValue::Text(text) => text,
        CellValue::Bool(true) => "TRUE".to_owned(),
        CellValue::Bool(false) => "FALSE".to_owned(),
    })
}

/// What [`locale`] says about text that is not a tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotATag;

/// A locale as somebody types one into Document Locale… — a tag (`de-DE`, `fr`) or nothing at
/// all, which is no locale of the document's own. `Err` for anything that is not a tag.
pub fn locale(typed: &str) -> Result<Option<Locale>, NotATag> {
    match typed.trim() {
        "" => Ok(None),
        tag => Locale::parse(tag).map(Some).ok_or(NotATag),
    }
}

/// A name's definition as Redefine… shows it to be edited: display syntax, `=` and all, so a
/// range reads `=$Sheet1.$A$1:$B$3` and an expression `=SUM($Sheet1.$A$1:$A$9)`. A definition
/// this build cannot print is shown as stored.
pub fn shown_definition(expression: &str) -> String {
    display::to_display(&format!("={expression}")).unwrap_or_else(|_| expression.to_owned())
}

/// What somebody typed into Redefine… or Define Name… as the expression `App::set_name` stores:
/// a formula in display syntax when it starts `=`, and otherwise an address or a range
/// (`crate::a1::definition`, the CLI's reading).
pub fn definition(app: &App, typed: &str) -> Result<String, String> {
    let typed = typed.trim();
    match typed.starts_with('=') {
        true => display::from_display(typed)
            .map(|canonical| canonical.trim_start_matches('=').to_owned())
            .map_err(|error| error.message),
        false => crate::a1::definition(app, typed).map_err(|error| error.to_string()),
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
    fn formula_to_value_keeps_what_the_cell_showed() {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "2", crate::RecalcMode::Document)
            .unwrap();
        app.enter(0, Pos::new(1, 0), "=[.A1]*3", crate::RecalcMode::Document)
            .unwrap();
        assert_eq!(
            formulas_to_values(&app, 0, Pos::new(0, 0), Pos::new(1, 0)).unwrap(),
            1,
            "only the formula is dropped"
        );
        assert_eq!(app.formula(0, Pos::new(1, 0)).unwrap(), None);
        assert_eq!(app.value_text(0, Pos::new(1, 0)).unwrap(), "6");
    }

    #[test]
    fn the_tracks_are_the_selections_whichever_way_it_was_made() {
        let backwards = range((5, 3), (2, 1));
        assert_eq!(rows(backwards), 2..6);
        assert_eq!(cols(backwards), 1..4);
    }

    #[test]
    fn a_name_points_at_the_selection_on_its_sheet() {
        let app = App::new();
        let target = name_target("Sheet1", range((1, 1), (8, 2)));
        assert_eq!(target, "Sheet1.B2:C9");
        assert_eq!(name_target("Sheet1", range((0, 0), (0, 0))), "Sheet1.A1");
        assert!(crate::a1::definition(&app, &target).is_ok(), "{target}");
    }

    #[test]
    fn a_formula_is_worked_out_where_the_cursor_is_and_nothing_is_stored() {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "2", crate::RecalcMode::Document)
            .unwrap();
        app.enter(0, Pos::new(1, 0), "3", crate::RecalcMode::Document)
            .unwrap();
        assert_eq!(
            evaluated(&app, 0, Pos::new(2, 0), "=SUM(A1:A2)"),
            Ok("5".into())
        );
        assert_eq!(
            evaluated(&app, 0, Pos::new(2, 0), "A1*10"),
            Ok("20".into()),
            "no = needed"
        );
        assert_eq!(
            evaluated(&app, 0, Pos::new(2, 0), "1/3"),
            Ok("0.3333".into())
        );
        assert!(evaluated(&app, 0, Pos::new(2, 0), "=SUM(").is_err());
        assert_eq!(app.used_extent(0).unwrap(), (2, 1), "nothing stored");
    }

    #[test]
    fn a_locale_is_a_tag_or_nothing() {
        assert_eq!(locale(" de-DE "), Ok(Locale::parse("de-DE")));
        assert_eq!(locale(""), Ok(None));
        assert_eq!(locale("not a tag"), Err(NotATag));
    }

    #[test]
    fn a_definition_is_shown_in_display_syntax_and_read_back_from_it() {
        let app = App::new();
        app.set_name("rate", "[$Sheet1.$B$1]").unwrap();
        let (_, stored) = app.names().into_iter().next().unwrap();
        let shown = shown_definition(&stored);
        assert!(shown.starts_with('=') && !shown.contains('['), "{shown}");
        assert_eq!(
            definition(&app, &shown),
            Ok(stored),
            "what is shown reads back"
        );
        let range = definition(&app, "A1:A3").unwrap();
        app.set_name("block", &range).unwrap();
        let sum = definition(&app, "=SUM(A1:A3)").unwrap();
        assert!(sum.starts_with("SUM(") && sum.contains("[."), "{sum}");
        assert!(definition(&app, "=SUM(").is_err());
    }
}
