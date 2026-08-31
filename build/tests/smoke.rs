// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a script can build, asserted against the document that came out.
//!
//! The exit criterion for D7 is `examples/sample-sheet.sh`'s document, generated, and
//! `cli/tests/cli.rs` runs the real script end to end. This file is the layer under that: one
//! feature per test, so a failure says which part of the host API broke rather than that the
//! budget is wrong.

use grind_build::{Artifact, build};
use grind_sheet::{App, CellValue, Pos};

/// The spreadsheet a script built, or the message it failed with.
fn spreadsheet(source: &str) -> App {
    match build(source, "<test>").unwrap_or_else(|e| panic!("{e}")) {
        Artifact::Spreadsheet(app) => app,
        Artifact::Text(_) => panic!("that script built a text document"),
    }
}

fn at(app: &App, address: &str) -> CellValue {
    let reference = grind_sheet::a1::parse(address).expect("an address");
    let (sheet, pos, _) = grind_sheet::a1::resolve(app, &reference).expect("a place");
    app.get(sheet, pos).expect("a cell")
}

fn error(source: &str) -> String {
    build(source, "model.rhai")
        .err()
        .expect("that script fails")
        .to_string()
}

#[test]
fn a_bare_sheet_is_a_one_sheet_document() {
    let app = spreadsheet(r#"sheet("Budget")"#);
    assert_eq!(app.sheet_count(), 1);
    assert_eq!(app.sheet_name(0).unwrap(), "Budget");
}

/// The table in `sheet.rs`'s module comment, executed. A script has types, so nothing is
/// re-derived from a spelling: `"2091"` is text without anybody asking for it to be.
#[test]
fn a_value_is_what_the_script_typed() {
    let app = spreadsheet(
        r#"
        let s = sheet("S");
        s.push(["Housing", 1800, 1825.5, true, "2091", (), "'=literal"]);
        s
        "#,
    );
    assert_eq!(at(&app, "A1"), CellValue::Text("Housing".into()));
    assert_eq!(at(&app, "B1"), CellValue::Number(1800.0));
    assert_eq!(at(&app, "C1"), CellValue::Number(1825.5));
    assert_eq!(at(&app, "D1"), CellValue::Bool(true));
    assert_eq!(at(&app, "E1"), CellValue::Text("2091".into()));
    assert_eq!(at(&app, "F1"), CellValue::Empty);
    assert_eq!(at(&app, "G1"), CellValue::Text("=literal".into()));
}

/// A formula is stored verbatim in ODF syntax and evaluated as it lands, which is what
/// `App::set_formula` does for every other caller — a generated document is not a document
/// with no cached values in it.
#[test]
fn a_formula_is_stored_and_answered() {
    let app = spreadsheet(
        r#"
        let s = sheet("S");
        s.push([1]);
        s.push([2]);
        s.push(["=SUM([.A1:.A2])", formula("A1*10")]);
        s
        "#,
    );
    assert_eq!(
        app.formula(0, Pos::new(2, 0)).unwrap().as_deref(),
        Some("=SUM([.A1:.A2])")
    );
    assert_eq!(at(&app, "A3"), CellValue::Number(3.0));
    // `formula(…)` is the display spelling, converted by the core's own converter — the one
    // behind `grind sheet fmt --from-display` — so what lands in the file is ODF syntax.
    assert_eq!(
        app.formula(0, Pos::new(2, 1)).unwrap().as_deref(),
        Some("=[.A1]*10")
    );
}

/// `sum_above()` is the one thing the script cannot say for itself, because the answer
/// depends on where the row landed.
#[test]
fn sum_above_reaches_the_contiguous_numbers_over_it() {
    let app = spreadsheet(
        r#"
        let s = sheet("S");
        s.push(["Region", "Sales"]);      // a header: text, so not part of the run
        s.push(["North", 400]);
        s.push(["South", 380]);
        s.push(["Total", sum_above()]);
        s
        "#,
    );
    assert_eq!(
        app.formula(0, Pos::new(3, 1)).unwrap().as_deref(),
        Some("=SUM([.B2:.B3])")
    );
    assert_eq!(at(&app, "B4"), CellValue::Number(780.0));
}

#[test]
fn sum_above_with_nothing_over_it_says_so() {
    let message = error(
        r#"
        let s = sheet("S");
        s.push([sum_above()]);
        s
        "#,
    );
    assert!(message.contains("no run of numbers above"), "{message}");
}

/// A style and a format are the projection's own vocabulary, and the row-level shorthand is
/// `doc/dsl.md` §4.2's `row(cells).bold()`.
#[test]
fn a_row_carries_its_own_styling() {
    let app = spreadsheet(
        r#"
        let s = sheet("S");
        s.push(row(["Region", "Sales"]).bold());
        s.push(row(["North", 400]).format(format("currency").symbol("€").decimals(2)));
        s.style("A1:B1", style().background("silver").align("center"));
        s
        "#,
    );
    // The row said bold and the range said silver-and-centred, and the header cell has all
    // three: in a script, styling *layers* — `Layered`, and the argument is with it.
    let head = app.style_at(0, Pos::new(0, 0)).unwrap().expect("a style");
    assert_eq!(head.font_weight.as_deref(), Some("bold"));
    assert_eq!(head.background.as_deref(), Some("#dddddd"));
    assert_eq!(head.align.as_deref(), Some("center"));

    let money = app.format_at(0, Pos::new(1, 1)).unwrap().expect("a format");
    assert_eq!(money.kind, grind_sheet::numfmt::Kind::Currency);
    assert_eq!(money.render(&CellValue::Number(400.0), 0), "400.00\u{a0}€");
}

/// A colour is the one palette `style::PALETTE` holds, resolved in the core — the same
/// attribute `--background silver` and a GUI's silver swatch write.
#[test]
fn a_colour_outside_the_vocabulary_is_an_error_where_it_was_written() {
    let message = error(
        r#"
        let s = sheet("S");
        s.push([1]);
        s.style("A1", style().background("puce"));
        s
        "#,
    );
    assert!(message.contains("puce"), "{message}");
    assert!(message.starts_with("model.rhai:4:"), "{message}");
}

/// Widths and heights are ODF lengths, and a whole-column range means the column the script
/// actually filled — which is why the decorations are applied after the cells.
#[test]
fn a_whole_column_means_the_one_the_script_filled() {
    let app = spreadsheet(
        r#"
        let s = sheet("S");
        s.push(["Housing", 1800]);
        s.push(["Groceries", 500]);
        s.width("A:A", "4cm");
        s.style("B:B", style().italic());
        s
        "#,
    );
    assert_eq!(app.col_widths(0).unwrap(), vec![(0, "4cm".to_owned())]);
    assert!(app.style_at(0, Pos::new(1, 1)).unwrap().is_some());
    // Two rows were filled, so two rows are styled — not a million.
    assert!(app.style_at(0, Pos::new(2, 1)).unwrap().is_none());
}

/// A name is document-level in ODF and said on the sheet it names, so the definition comes
/// out sheet-qualified and absolute — `a1::as_definition`'s rule, not a second one.
#[test]
fn a_named_range_is_qualified_with_the_sheet_that_said_it() {
    let app = spreadsheet(
        r#"
        let s = sheet("Budget");
        s.push([1800]);
        s.push([500]);
        s.name("budgeted", "A1:A2");
        s.name("biggest", "=MAX(budgeted)");
        s
        "#,
    );
    let named = |wanted: &str| {
        app.names()
            .into_iter()
            .find(|(name, _)| name == wanted)
            .map(|(_, definition)| definition)
            .expect("that name is defined")
    };
    assert_eq!(named("budgeted"), "[$Budget.$A$1:.$A$2]");
    assert_eq!(named("biggest"), "MAX(budgeted)");
}

/// Several sheets, and the second one's ranges resolved against *it* — `a1::resolve_in`, the
/// reason an unqualified address in a script does not silently mean the first sheet.
#[test]
fn a_second_sheet_is_a_peer() {
    let app = spreadsheet(
        r#"
        let d = spreadsheet();
        let budget = sheet("Budget");
        budget.push([1800]);
        let journal = sheet("Journal");
        journal.push(["opening", 12]);
        journal.push(["closing", 30]);
        journal.style("A1:A2", style().italic());
        d.push(budget);
        d.push(journal);
        d
        "#,
    );
    assert_eq!(app.sheet_count(), 2);
    assert_eq!(app.sheet_name(1).unwrap(), "Journal");
    assert!(app.style_at(1, Pos::new(1, 0)).unwrap().is_some());
    assert!(app.style_at(0, Pos::new(0, 0)).unwrap().is_none());
}

/// The footgun a shared handle removes: a sheet pushed into a document and then written to
/// again is still the same sheet.
#[test]
fn a_sheet_pushed_into_a_document_is_not_a_copy_of_it() {
    let app = spreadsheet(
        r#"
        let d = spreadsheet();
        let s = sheet("S");
        d.push(s);
        s.push(["late", 1]);
        d
        "#,
    );
    assert_eq!(at(&app, "A1"), CellValue::Text("late".into()));
}

/// A script is ordinary code, and this is the whole reason for a generator: the loop is the
/// document's structure, said once.
#[test]
fn a_loop_writes_the_rows() {
    let app = spreadsheet(
        r#"
        fn header(cells) { row(cells).bold() }
        let s = sheet("Sales");
        let regions = ["North", "South", "East", "West"];
        s.push(header(["Region", "Q1"]));
        for r in regions { s.push([r, 100]); }
        s.push(row(["Total", sum_above()]).bold());
        s
        "#,
    );
    assert_eq!(at(&app, "A5"), CellValue::Text("West".into()));
    assert_eq!(at(&app, "B6"), CellValue::Number(400.0));
}

/// `at` is the address arithmetic a script would otherwise do by hand — and the conversion
/// happens in `a1.rs`, which is the workspace's only one.
#[test]
fn an_address_comes_from_the_core() {
    let app = spreadsheet(
        r#"
        let s = sheet("S");
        let first = s.push(["Housing", 1800]);
        s.set(s.at(first, 2), formula(s.at(first, 1) + "*2"));
        s
        "#,
    );
    assert_eq!(at(&app, "C1"), CellValue::Number(3600.0));
}

// ---------------------------------------------------------------------------
// The word processor
// ---------------------------------------------------------------------------

fn text(source: &str) -> grind_text::App {
    match build(source, "<test>").unwrap_or_else(|e| panic!("{e}")) {
        Artifact::Text(app) => app,
        Artifact::Spreadsheet(_) => panic!("that script built a spreadsheet"),
    }
}

#[test]
fn a_text_document_is_blocks_in_the_order_they_were_said() {
    let app = text(
        r#"
        let d = text();
        d.heading(1, "Quarterly report");
        d.bookmark("intro");
        d.para("Revenue rose on the year.");
        d.item(1, "North");
        d
        "#,
    );
    assert_eq!(app.block_count(), 3);
    assert_eq!(app.input_text(0).unwrap(), "Quarterly report");
    assert_eq!(app.outline().len(), 1);
    assert_eq!(app.bookmarks(), vec![("intro".to_owned(), 0)]);
}

/// The notation is `grind_text::markdown`'s, which is the point: four shells and a generator
/// read `**` one way, because there is one reader.
#[test]
fn the_inline_notation_is_the_one_the_suite_already_has() {
    let app = text(r#"text().para("Revenue rose **12%** on the year.")"#);
    assert_eq!(app.input_text(0).unwrap(), "Revenue rose 12% on the year.");
    let bold = app
        .get_viewport(0..1)
        .iter()
        .flat_map(|block| block.runs.iter())
        .any(|run| run.props.is_bold());
    assert!(bold, "the **12%** is bold");
}

// ---------------------------------------------------------------------------
// What a script must not do
// ---------------------------------------------------------------------------

/// The layer boundary, as an error message: a script returns a document, and anything else is
/// a script that forgot to.
#[test]
fn a_script_that_returns_something_else_is_told_so() {
    let message = error("1 + 1");
    assert!(
        message.contains("has to end with the document it built"),
        "{message}"
    );
}

/// §2's bound, from the outside: a script that does not terminate is a build error with a
/// line number.
#[test]
fn a_runaway_script_stops() {
    let message = error("let i = 0;\nwhile true { i += 1; }\ntext()");
    assert!(message.starts_with("model.rhai:2:"), "{message}");
    assert!(message.contains("operations"), "{message}");
}
