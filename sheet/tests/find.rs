// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Find and replace over cells, against a document.
//!
//! `sheet/src/find.rs`'s own tests cover what a match is; these cover what it is a match
//! *of* — every cell's input text — and that a replace goes back in through the typing rule,
//! all of it in one undo entry.

use grind_sheet::find::Search;
use grind_sheet::{App, CellValue, Pos, RecalcMode};

fn p(address: &str) -> Pos {
    let reference = grind_sheet::a1::parse(address).unwrap();
    grind_sheet::a1::resolve(&App::new(), &reference).unwrap().1
}

fn enter(app: &App, sheet: usize, address: &str, input: &str) {
    app.enter(sheet, p(address), input, RecalcMode::No).unwrap();
}

/// Two sheets: a price list with a total, and a second sheet naming the same fruit.
fn book() -> App {
    let app = App::new();
    enter(&app, 0, "A1", "Apples");
    enter(&app, 0, "B1", "3");
    enter(&app, 0, "A2", "Pears");
    enter(&app, 0, "B2", "4");
    enter(&app, 0, "B3", "=SUM([.B1:.B2])");
    app.add_sheet("Notes").unwrap();
    enter(&app, 1, "A1", "apples are red");
    app
}

fn addresses(app: &App, search: &Search) -> Vec<String> {
    app.find(search)
        .unwrap()
        .iter()
        .map(|hit| hit.address())
        .collect()
}

#[test]
fn find_reads_every_sheet_in_reading_order_ignoring_case() {
    let app = book();
    assert_eq!(
        addresses(&app, &Search::new("apples")),
        ["Sheet1.A1", "Notes.A1"]
    );
}

#[test]
fn find_searches_a_formula_as_the_formula_bar_shows_it() {
    let app = book();
    let hits = app.find(&Search::new("sum(b1")).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].address(), "Sheet1.B3");
    assert_eq!(hits[0].text, "=SUM(B1:B2)");
}

#[test]
fn case_whole_cell_and_one_sheet_narrow_it() {
    let app = book();
    let exact = Search {
        match_case: true,
        ..Search::new("apples")
    };
    assert_eq!(addresses(&app, &exact), ["Notes.A1"]);

    let whole = Search {
        whole_cell: true,
        ..Search::new("3")
    };
    assert_eq!(addresses(&app, &whole), ["Sheet1.B1"]);

    let first = Search {
        sheet: Some(0),
        ..Search::new("apples")
    };
    assert_eq!(addresses(&app, &first), ["Sheet1.A1"]);
    assert!(
        app.find(&Search {
            sheet: Some(7),
            ..Search::new("x")
        })
        .is_err()
    );
}

#[test]
fn a_range_narrows_it_to_a_rectangle_and_replace_to_one_cell() {
    let app = book();
    let column_b = Search {
        range: Some((p("B1"), p("B9"))),
        ..Search::new("3")
    };
    assert_eq!(addresses(&app, &column_b), ["Sheet1.B1"]);

    // One cell is a range with both corners on it — what a find bar's Replace button is.
    let one = Search {
        sheet: Some(1),
        range: Some((p("A1"), p("A1"))),
        ..Search::new("apples")
    };
    let done = app.replace(&one, "Plums", RecalcMode::No).unwrap();
    assert_eq!(done.cells, 1);
    assert_eq!(
        app.get(0, p("A1")).unwrap(),
        CellValue::Text("Apples".into())
    );
    assert_eq!(
        app.get(1, p("A1")).unwrap(),
        CellValue::Text("Plums are red".into())
    );

    // A range past the end of the sheet walks nothing that is not there.
    let far = Search {
        range: Some((p("Z100"), p("ZZ9999"))),
        ..Search::new("a")
    };
    assert!(app.find(&far).unwrap().is_empty());
}

#[test]
fn an_empty_needle_finds_and_replaces_nothing() {
    let app = book();
    assert!(app.find(&Search::new("")).unwrap().is_empty());
    let done = app.replace(&Search::new(""), "x", RecalcMode::No).unwrap();
    assert_eq!(done.cells, 0);
}

#[test]
fn replace_is_one_undo_step_across_sheets() {
    let app = book();
    let done = app
        .replace(&Search::new("apples"), "Plums", RecalcMode::No)
        .unwrap();
    assert_eq!(done.cells, 2);
    assert!(done.refused.is_empty());
    assert_eq!(
        app.get(0, p("A1")).unwrap(),
        CellValue::Text("Plums".into())
    );
    assert_eq!(
        app.get(1, p("A1")).unwrap(),
        CellValue::Text("Plums are red".into())
    );

    assert!(app.undo());
    assert_eq!(
        app.get(0, p("A1")).unwrap(),
        CellValue::Text("Apples".into())
    );
    assert_eq!(
        app.get(1, p("A1")).unwrap(),
        CellValue::Text("apples are red".into())
    );
}

#[test]
fn a_replaced_formula_is_still_a_formula_and_recalculates() {
    let app = book();
    enter(&app, 0, "B4", "10");
    let done = app
        .replace(&Search::new("B2)"), "B2)+B4", RecalcMode::Document)
        .unwrap();
    assert_eq!(done.cells, 1);
    assert_eq!(app.input_text(0, p("B3")).unwrap(), "=SUM(B1:B2)+B4");
    assert_eq!(app.get(0, p("B3")).unwrap(), CellValue::Number(17.0));
}

#[test]
fn replaced_text_is_read_by_the_typing_rule() {
    let app = App::new();
    enter(&app, 0, "A1", "123");
    enter(&app, 0, "A2", "'007");
    app.replace(&Search::new("3"), "4", RecalcMode::No).unwrap();
    app.replace(&Search::new("7"), "8", RecalcMode::No).unwrap();
    // A number stays a number, and a text cell that looks like one stays text: its input
    // text carries the `'` that says so, and the replace goes through that same rule.
    assert_eq!(app.get(0, p("A1")).unwrap(), CellValue::Number(124.0));
    assert_eq!(app.get(0, p("A2")).unwrap(), CellValue::Text("008".into()));
}

#[test]
fn a_formula_the_replace_would_break_is_refused_and_the_rest_still_happens() {
    let app = book();
    enter(&app, 0, "C1", "see SUM(B1:B2)");
    let done = app
        .replace(&Search::new("SUM("), "SUM", RecalcMode::No)
        .unwrap();
    // `=SUMB1:B2)` does not parse, so B3 is left exactly as it was.
    assert_eq!(done.refused.len(), 1);
    assert_eq!(done.refused[0].0.address(), "Sheet1.B3");
    assert_eq!(app.input_text(0, p("B3")).unwrap(), "=SUM(B1:B2)");
    // C1 is text, so its replace is just text, and it still happens.
    assert_eq!(done.cells, 1);
    assert_eq!(
        app.get(0, p("C1")).unwrap(),
        CellValue::Text("see SUMB1:B2)".into())
    );
}
