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

/// A new chart's size — the GNOME window's — and its gap from the table it charts.
pub const CHART_WIDTH: &str = "12cm";
pub const CHART_HEIGHT: &str = "7.5cm";
pub const CHART_MARGIN_MM: f64 = 6.0;

/// What *Insert Chart* would make of a guess: the guessed spec — of `kind` when somebody chose one,
/// else of the kind the cells want — with the legend a new chart of that kind gets. One place, so
/// [`insert_chart`] and [`preview_insert_chart`] cannot disagree.
fn insert_spec(guessed: &crate::chart::Guess, kind: Option<crate::ChartKind>) -> crate::ChartSpec {
    let spec = crate::ChartSpec {
        kind: kind.unwrap_or(guessed.spec.kind),
        ..guessed.spec.clone()
    };
    crate::ChartSpec {
        legend: spec.default_legend(),
        ..spec
    }
}

/// *Insert Chart*, **without inserting** — the chart [`insert_chart`] would make of this selection
/// and the data it would draw (`App::preview_chart`), so a shell with no dialog can show the
/// picture before it is committed to. `kind` is [`insert_chart`]'s. Nothing is written.
pub fn preview_insert_chart(
    app: &App,
    sheet: usize,
    start: Pos,
    end: Pos,
    kind: Option<crate::ChartKind>,
) -> crate::Result<(crate::Chart, crate::ChartData)> {
    let guessed = app.suggest_chart(sheet, start, end, None)?;
    app.preview_chart(sheet, &insert_spec(&guessed, kind), None)
}

/// *Insert Chart*: a chart of the table the selection means (`App::suggest_chart` — which way the
/// series run, what names them, what kind the cells want), placed beside it at the GNOME
/// window's size. One undo step. `kind` is the kind somebody chose — the GNOME dialog's three
/// buttons, every other window's *Insert ▸ Bar/Line/Pie Chart* — and `None` the one the cells
/// want. `place` is the shell's own geometry: given the column just right of the table and its
/// first row, it answers that corner's `(x, y)` in millimetres from the sheet's origin — the one
/// thing a shell knows and the core does not. Answers the new chart's index.
pub fn insert_chart(
    app: &App,
    sheet: usize,
    start: Pos,
    end: Pos,
    kind: Option<crate::ChartKind>,
    place: impl Fn(u32, u32) -> (f64, f64),
) -> crate::Result<usize> {
    let guessed = app.suggest_chart(sheet, start, end, None)?;
    let spec = insert_spec(&guessed, kind);
    let (x, y) = place(guessed.end.col + 1, guessed.start.row);
    app.add_chart(
        sheet,
        &spec,
        &crate::style::mm_length(x + CHART_MARGIN_MM),
        &crate::style::mm_length(y),
        CHART_WIDTH,
        CHART_HEIGHT,
    )?;
    Ok(app.charts(sheet)?.len().saturating_sub(1))
}

/// Restyle the chart at `index` on `sheet` from a person's words — `line`, `bar` or `pie`;
/// `title=Quarterly sales` (to the end of the line) or `no-title`; `legend=top|bottom|start|end`
/// or `legend=none`; `width=12cm` and `height=8cm` — everything else about it kept
/// (`ChartSpec::of`, then `App::edit_chart`, then `App::reshape_chart` for a size). What a window
/// with no chart dialog asks in place of one. Answers the sentence to say, or why it could not.
///
/// ponytail: words for the look *and* a size are two undo steps, since `edit_chart` and
/// `reshape_chart` are two actions; the trigger is somebody minding the second ⌘Z, and the
/// upgrade is one `Action::Batch` of both.
pub fn restyle_chart(
    app: &App,
    sheet: usize,
    index: usize,
    words: &str,
) -> std::result::Result<String, String> {
    let chart = app
        .charts(sheet)
        .map_err(|e| e.to_string())?
        .get(index)
        .cloned()
        .ok_or("there is no such chart")?;
    let mut spec = crate::ChartSpec::of(&chart);
    let mut said = Vec::new();
    let (mut width, mut height) = (None, None);
    let mut rest = words.trim();
    while !rest.is_empty() {
        let (word, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        rest = after.trim_start();
        match word {
            "bar" => spec.kind = crate::ChartKind::Bar,
            "line" => spec.kind = crate::ChartKind::Line,
            "pie" => spec.kind = crate::ChartKind::Pie,
            "no-title" => spec.title = None,
            "legend=none" => spec.legend = None,
            "legend=top" => spec.legend = Some(crate::ChartLegend::Top),
            "legend=bottom" => spec.legend = Some(crate::ChartLegend::Bottom),
            "legend=start" => spec.legend = Some(crate::ChartLegend::Start),
            "legend=end" => spec.legend = Some(crate::ChartLegend::End),
            _ if word.starts_with("width=") || word.starts_with("height=") => {
                let (which, length) = word.split_once('=').unwrap_or((word, ""));
                if !crate::style::length_mm(length).is_some_and(|mm| mm > 0.0) {
                    return Err(format!("“{length}” is not a length — 12cm, 4in or 300pt"));
                }
                match which {
                    "width" => width = Some(length.to_owned()),
                    _ => height = Some(length.to_owned()),
                }
            }
            _ if word.starts_with("title=") => {
                // The title is the rest of the line: it has spaces in it.
                let title = format!("{} {}", &word["title=".len()..], rest);
                spec.title = Some(title.trim().to_owned()).filter(|t| !t.is_empty());
                rest = "";
            }
            other => {
                return Err(format!("“{other}” is not a chart word — {CHART_WORDS}"));
            }
        }
        said.push(word);
    }
    if said.is_empty() {
        return Err(format!("say what to change — {CHART_WORDS}"));
    }
    if spec != crate::ChartSpec::of(&chart) {
        app.edit_chart(sheet, index, &spec)
            .map_err(|e| e.to_string())?;
    }
    if width.is_some() || height.is_some() {
        app.reshape_chart(
            sheet,
            index,
            &chart.x,
            &chart.y,
            width.as_deref().unwrap_or(&chart.width),
            height.as_deref().unwrap_or(&chart.height),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok("The chart is changed.".to_owned())
}

/// The chart at `index` on `sheet` turned into a `kind` — everything else about it kept, one undo
/// step (`ChartSpec::of`, then `App::edit_chart`). What a chart's own menu offers as three items,
/// where the GNOME dialog has three buttons. Answers the sentence to say, or why it could not;
/// a chart already of that kind is left alone and says so.
pub fn set_chart_kind(
    app: &App,
    sheet: usize,
    index: usize,
    kind: crate::ChartKind,
) -> std::result::Result<String, String> {
    let chart = app
        .charts(sheet)
        .map_err(|e| e.to_string())?
        .get(index)
        .cloned()
        .ok_or("there is no such chart")?;
    let name = kind.name().to_lowercase();
    if chart.kind == kind {
        return Ok(format!("The chart is already a {name} chart."));
    }
    let spec = crate::ChartSpec {
        kind,
        ..crate::ChartSpec::of(&chart)
    };
    app.edit_chart(sheet, index, &spec)
        .map_err(|e| e.to_string())?;
    Ok(format!("The chart is a {name} chart now."))
}

/// Every word [`restyle_chart`] takes, for a prompt to say.
pub const CHART_WORDS: &str =
    "bar, line, pie, title=…, no-title, legend=top|bottom|start|end|none, width=12cm, height=8cm";

/// *Move the last chart here* — the chart at `index` with its top-left corner at the active
/// cell's, its size kept, one undo step (`App::reshape_chart`). `place` is [`insert_chart`]'s:
/// the shell's own answer to where a column and a row begin, in millimetres. What a window that
/// does not hit-test a chart offers in place of dragging one.
pub fn move_chart(
    app: &App,
    sheet: usize,
    index: usize,
    at: Pos,
    place: impl Fn(u32, u32) -> (f64, f64),
) -> std::result::Result<String, String> {
    let chart = app
        .charts(sheet)
        .map_err(|e| e.to_string())?
        .get(index)
        .cloned()
        .ok_or("there is no such chart")?;
    let (x, y) = place(at.col, at.row);
    app.reshape_chart(
        sheet,
        index,
        &crate::style::mm_length(x),
        &crate::style::mm_length(y),
        &chart.width,
        &chart.height,
    )
    .map_err(|e| e.to_string())?;
    Ok(format!(
        "The chart now starts at {}.",
        crate::a1::format(None, at)
    ))
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
    fn a_chart_goes_in_beside_the_table_it_charts() {
        let app = App::new();
        for (r, row) in [["Month", "Sales"], ["Jan", "5"], ["Feb", "7"]]
            .iter()
            .enumerate()
        {
            for (c, text) in row.iter().enumerate() {
                app.enter(
                    0,
                    Pos::new(r as u32, c as u32),
                    text,
                    crate::RecalcMode::Document,
                )
                .unwrap();
            }
        }
        let index = insert_chart(&app, 0, Pos::new(0, 0), Pos::new(2, 1), None, |col, row| {
            (f64::from(col) * 20.0, f64::from(row) * 5.0)
        })
        .unwrap();
        assert_eq!(index, 0);
        assert_eq!(app.charts(0).unwrap().len(), 1);
        assert!(
            insert_chart(&app, 0, Pos::new(0, 0), Pos::new(2, 1), None, |_, _| (
                0.0, 0.0
            ))
            .is_ok()
        );
        assert_eq!(app.charts(0).unwrap().len(), 2);
    }

    #[test]
    fn a_chart_goes_in_as_the_kind_asked_for_and_changes_kind() {
        let app = App::new();
        for (r, row) in [["Month", "Sales"], ["Jan", "5"], ["Feb", "7"]]
            .iter()
            .enumerate()
        {
            for (c, text) in row.iter().enumerate() {
                app.enter(
                    0,
                    Pos::new(r as u32, c as u32),
                    text,
                    crate::RecalcMode::Document,
                )
                .unwrap();
            }
        }
        let (start, end) = (Pos::new(0, 0), Pos::new(2, 1));
        let guessed = app.suggest_chart(0, start, end, None).unwrap().spec.kind;
        for kind in crate::ChartKind::ALL {
            let (shown, _) = preview_insert_chart(&app, 0, start, end, Some(kind)).unwrap();
            assert_eq!(shown.kind, kind, "the preview is of the kind asked for");
            // A pie names its slices; one bar or line series needs no legend.
            assert_eq!(shown.legend.is_some(), kind == crate::ChartKind::Pie);
        }
        let (shown, _) = preview_insert_chart(&app, 0, start, end, None).unwrap();
        assert_eq!(
            shown.kind, guessed,
            "no kind asked for is the one the cells want"
        );

        let index = insert_chart(&app, 0, start, end, Some(crate::ChartKind::Pie), |_, _| {
            (0.0, 0.0)
        })
        .unwrap();
        assert_eq!(app.charts(0).unwrap()[index].kind, crate::ChartKind::Pie);
        let said = set_chart_kind(&app, 0, index, crate::ChartKind::Line).unwrap();
        assert!(said.contains("line"), "{said}");
        assert_eq!(app.charts(0).unwrap()[index].kind, crate::ChartKind::Line);
        assert!(
            set_chart_kind(&app, 0, index, crate::ChartKind::Line)
                .unwrap()
                .contains("already")
        );
        assert!(app.undo());
        assert_eq!(app.charts(0).unwrap()[index].kind, crate::ChartKind::Pie);
        assert!(set_chart_kind(&app, 0, 9, crate::ChartKind::Bar).is_err());
    }

    #[test]
    fn a_chart_is_restyled_in_words() {
        let app = App::new();
        for (r, row) in [["Month", "Sales"], ["Jan", "5"], ["Feb", "7"]]
            .iter()
            .enumerate()
        {
            for (c, text) in row.iter().enumerate() {
                app.enter(
                    0,
                    Pos::new(r as u32, c as u32),
                    text,
                    crate::RecalcMode::Document,
                )
                .unwrap();
            }
        }
        insert_chart(&app, 0, Pos::new(0, 0), Pos::new(2, 1), None, |_, _| {
            (0.0, 0.0)
        })
        .unwrap();
        restyle_chart(&app, 0, 0, "pie legend=top title=Quarterly sales").unwrap();
        let chart = app.charts(0).unwrap()[0].clone();
        assert_eq!(chart.kind, crate::ChartKind::Pie);
        assert_eq!(chart.legend, Some(crate::ChartLegend::Top));
        assert_eq!(chart.title.as_deref(), Some("Quarterly sales"));
        restyle_chart(&app, 0, 0, "no-title legend=none line").unwrap();
        let chart = app.charts(0).unwrap()[0].clone();
        assert_eq!(
            (chart.kind, chart.legend, chart.title),
            (crate::ChartKind::Line, None, None)
        );
        assert!(restyle_chart(&app, 0, 0, "shout").is_err());
        assert!(restyle_chart(&app, 0, 0, "").is_err());
        assert!(restyle_chart(&app, 0, 5, "pie").is_err());

        // A size is words too, the look untouched by it.
        restyle_chart(&app, 0, 0, "width=12cm height=8cm").unwrap();
        let sized = app.charts(0).unwrap()[0].clone();
        assert_eq!(
            (sized.width.as_str(), sized.height.as_str()),
            ("12cm", "8cm")
        );
        assert_eq!(sized.kind, crate::ChartKind::Line);
        assert!(restyle_chart(&app, 0, 0, "width=wide").is_err());

        // Moved: the corner at the cell, the size kept, one undo step.
        let said = move_chart(&app, 0, 0, Pos::new(4, 2), |col, row| {
            (f64::from(col) * 20.0, f64::from(row) * 5.0)
        })
        .unwrap();
        assert!(said.contains("C5"), "{said}");
        let moved = app.charts(0).unwrap()[0].clone();
        assert_eq!(
            (
                grind_core::style::length_mm(&moved.x),
                grind_core::style::length_mm(&moved.y)
            ),
            (Some(40.0), Some(20.0))
        );
        assert_eq!(moved.width, "12cm");
        assert!(app.undo());
        assert_eq!(app.charts(0).unwrap()[0].x, sized.x);
        assert!(move_chart(&app, 0, 3, Pos::new(0, 0), |_, _| (0.0, 0.0)).is_err());
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
