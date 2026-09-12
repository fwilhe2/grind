// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `App::format_table` end to end: the autofilter, the banding, the totals row and the named
//! range all land in one undo step and all survive our own round trip — every construct it
//! writes (`table:database-range`, `style:style`, an ordinary formula, `table:named-range`) is
//! already loop-C-covered on its own, so this is the seam between them rather than a new one.

use grind_sheet::{App, CellValue, Form, Pos, TableOptions, TotalsFunction};

fn sample() -> App {
    let app = App::new();
    app.set_cell(0, Pos::new(0, 0), "Product").unwrap();
    app.set_cell(0, Pos::new(0, 1), "Qty").unwrap();
    for (row, (product, qty)) in [("Chair", 3.0), ("Desk", 1.0), ("Lamp", 4.0)]
        .into_iter()
        .enumerate()
    {
        let row = row as u32 + 1;
        app.set_cell(0, Pos::new(row, 0), product).unwrap();
        app.set_cell(0, Pos::new(row, 1), qty).unwrap();
    }
    app
}

#[test]
fn one_undo_step_covers_filter_banding_totals_and_the_name() {
    let app = sample();
    let options = TableOptions {
        header: true,
        totals: Some(TotalsFunction::Sum),
        name: None,
    };
    let name = app
        .format_table(0, Pos::new(0, 0), Pos::new(3, 1), options)
        .expect("formats as a table");
    assert_eq!(name, "Table1");

    assert!(app.filter(0).unwrap().is_some(), "autofilter applied");
    assert_eq!(
        app.names(),
        vec![("table1".to_owned(), "[$Sheet1.$A$1:.$B$5]".to_owned())],
        "the named range covers the header through the totals row"
    );

    // One Ctrl+Z takes the whole thing back.
    assert!(app.undo());
    assert!(app.filter(0).unwrap().is_none(), "the filter is gone again");
    assert!(app.names().is_empty(), "and so is the name");
}

#[test]
fn it_survives_our_own_round_trip() {
    let app = sample();
    let options = TableOptions {
        header: true,
        totals: Some(TotalsFunction::Sum),
        name: Some("Sales".to_owned()),
    };
    app.format_table(0, Pos::new(0, 0), Pos::new(3, 1), options)
        .unwrap();

    let bytes = app.save_bytes(Form::Flat).expect("writes");
    let back = grind_sheet::read_bytes("out.fods", &bytes).expect("reads back");
    let sheet = back.sheet(0).expect("one sheet");

    assert!(sheet.filter().is_some(), "the autofilter round-trips");
    assert_eq!(
        back.name("Sales"),
        Some("[$Sheet1.$A$1:.$B$5]"),
        "the named range round-trips"
    );
    assert_eq!(sheet.get(Pos::new(4, 0)), CellValue::Text("Total".into()));
    assert_eq!(sheet.formula(Pos::new(4, 1)), Some("SUM([.B2:.B4])"));
    assert_eq!(sheet.get(Pos::new(4, 1)), CellValue::Number(8.0));

    let header = sheet.style(Pos::new(0, 0)).expect("the header is styled");
    assert_eq!(header.font_weight.as_deref(), Some("bold"));
    let band_one = sheet.style(Pos::new(1, 0)).expect("a data row is styled");
    let band_two = sheet.style(Pos::new(2, 0)).expect("the next row too");
    assert_ne!(
        band_one.background, band_two.background,
        "adjacent rows alternate shading"
    );
}

/// The cached values `table_format::summarise` writes are computed *beside* the evaluator
/// rather than by it (the plan is built with the write lock held), so this is the check that
/// the two agree: format with each aggregate in turn, recalculate, and assert nothing moved.
/// A `Recalc` that reports a changed cell is `summarise` disagreeing with `formula::funcs`.
#[test]
fn every_aggregate_caches_what_recalculating_would_have_computed() {
    for function in TotalsFunction::ALL {
        let app = sample();
        let options = TableOptions {
            header: true,
            totals: Some(*function),
            name: None,
        };
        app.format_table(0, Pos::new(0, 0), Pos::new(3, 1), options)
            .unwrap();
        let before = app.get_viewport(0, 4..5, 1..2).unwrap().get(4, 1).cloned();
        let outcome = app.recalc().expect("recalculates");
        let after = app.get_viewport(0, 4..5, 1..2).unwrap().get(4, 1).cloned();
        assert_eq!(before, after, "{function}: the cached value was wrong");
        assert_eq!(
            outcome.changed, 0,
            "{function}: recalculating moved a cell the plan had already answered"
        );
    }
}

#[test]
fn a_totals_row_can_be_an_average_and_says_so_in_its_label() {
    let app = sample();
    let options = TableOptions {
        header: true,
        totals: Some(TotalsFunction::Average),
        name: None,
    };
    app.format_table(0, Pos::new(0, 0), Pos::new(3, 1), options)
        .unwrap();
    let bytes = app.save_bytes(Form::Flat).expect("writes");
    let back = grind_sheet::read_bytes("out.fods", &bytes).expect("reads back");
    let sheet = back.sheet(0).expect("one sheet");
    assert_eq!(sheet.get(Pos::new(4, 0)), CellValue::Text("Average".into()));
    assert_eq!(sheet.formula(Pos::new(4, 1)), Some("AVERAGE([.B2:.B4])"));
}

#[test]
fn without_a_header_row_zero_is_data_and_gets_banded_like_any_other() {
    let app = sample();
    let options = TableOptions {
        header: false,
        totals: None,
        name: None,
    };
    app.format_table(0, Pos::new(1, 0), Pos::new(3, 1), options)
        .unwrap();
    assert!(!app.filter(0).unwrap().unwrap().contains_header);
    let viewport = app.get_viewport(0, 0..4, 0..2).unwrap();
    assert!(
        viewport.style(1, 0).is_some(),
        "the first row of the range is banded, not treated as a heading"
    );
}
