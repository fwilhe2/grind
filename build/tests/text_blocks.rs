// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The text document's host API beyond paragraphs: tables, and page breaks.

use grind_build::{Artifact, build};
use grind_sheet::Form;

fn text(script: &str) -> grind_text::App {
    match build(script, "doc.rhai") {
        Ok(Artifact::Text(app)) => *app,
        Ok(_) => panic!("that script built a spreadsheet"),
        Err(e) => panic!("{e}"),
    }
}

fn flat(app: &grind_text::App) -> String {
    String::from_utf8(app.save_bytes(Form::Flat).expect("it saves")).expect("text")
}

/// A table is `rows × columns` blocks that know where they sit, and its cells are read in the
/// notation the whole suite types in.
#[test]
fn a_table_is_rows_of_cells() {
    let app = text(
        r#"let d = text();
           d.para("Before");
           d.table([["**Item**", "Price"], ["Tea", "3,50"], ["Cake", "4,00"]]);
           d.para("After");
           d"#,
    );
    assert_eq!(app.block_count(), 1 + 6 + 1);
    let table = app.table(1).expect("one table");
    assert_eq!((table.rows, table.columns), (3, 2));
    let xml = flat(&app);
    assert!(xml.contains("<table:table "), "{xml}");
    assert!(xml.contains("Cake") && xml.contains("4,00"));
    // The first row's first cell is bold: the notation reached it.
    let first = app.get_viewport(table.blocks.start..table.blocks.start + 1);
    assert_eq!(first.iter().next().expect("a block").text, "Item");
}

/// A document may open with a table, and the empty paragraph a new document starts with does
/// not stay in front of it.
#[test]
fn a_document_may_open_with_a_table() {
    let app = text(r#"let d = text(); d.table([["a", "b"]]); d"#);
    assert_eq!(app.block_count(), 2);
    assert_eq!(app.table(0).expect("a table").blocks.start, 0);
}

/// A column can be set in a style the template declares, one style per column.
#[test]
fn a_column_takes_a_paragraph_style() {
    let app =
        text(r#"let d = text(); d.table([["Tea", "3,50"], ["Cake", "4,00"]], ["", "Amount"]); d"#);
    let xml = flat(&app);
    assert_eq!(
        xml.matches(r#"text:style-name="Amount""#).count(),
        2,
        "{xml}"
    );
}

#[test]
fn a_ragged_table_is_refused_by_row() {
    let error = build(
        r#"let d = text(); d.table([["a", "b"], ["c"]]); d"#,
        "x.rhai",
    )
    .err()
    .expect("refused")
    .to_string();
    assert!(error.contains("row 2 has 1 cells"), "{error}");
}

#[test]
fn a_block_verb_refuses_a_table() {
    let error = build(
        r#"let d = text(); d.table([["a"]]); d.bookmark("t"); d"#,
        "x.rhai",
    )
    .err()
    .expect("refused")
    .to_string();
    assert!(
        error.contains("one block") && error.contains("table"),
        "{error}"
    );
}

/// `page_break()` is a paragraph style the document declares — derived from the block's own, so
/// it keeps its look — whatever the document started from, and none.
#[test]
fn a_page_break_is_a_declared_style_on_the_next_block() {
    let app = text(
        r#"let d = text();
           d.para("one");
           d.page_break();
           d.para("two");
           d.page_break();
           d.para("Letter", "three");
           d"#,
    );
    let xml = flat(&app);
    assert!(xml.contains(r#"fo:break-before="page""#), "{xml}");
    assert!(xml.contains(r#"style:name="grind-break-none""#), "{xml}");
    assert!(xml.contains(r#"style:name="grind-break-Letter""#), "{xml}");
    assert!(xml.contains(r#"style:parent-style-name="Letter""#), "{xml}");
    assert!(xml.contains(r#"text:style-name="grind-break-none""#));
    assert_eq!(app.block_count(), 3);
}

#[test]
fn a_page_break_before_a_table_is_refused() {
    let error = build(
        r#"let d = text(); d.para("x"); d.page_break(); d.table([["a"]]); d"#,
        "x.rhai",
    )
    .err()
    .expect("refused")
    .to_string();
    assert!(error.contains("page_break"), "{error}");
}
