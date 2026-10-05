// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Conditional formatting's one rule type (`doc/conditional-format.md`), against
//! `samples/conditional-rules.fods` — LibreOffice's own save of two rules on one cell and one
//! rule over two differently styled cell styles (`doc/ods-format.md` §3.6) — and through `App`.

use std::path::PathBuf;

use grind_sheet::rule::Rule;
use grind_sheet::style::CellStyle;
use grind_sheet::{App, CellValue, Form, Pos};

fn sample() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/samples/conditional-rules.fods"),
    )
    .unwrap()
}

fn open(bytes: &[u8]) -> App {
    let app = App::new();
    app.open_bytes("rules.fods", bytes).unwrap();
    app
}

fn fill(colour: &str) -> CellStyle {
    CellStyle {
        background: Some(colour.into()),
        ..CellStyle::default()
    }
}

fn background(app: &App, row: u32, col: u32) -> Option<String> {
    let view = app.get_viewport(0, row..row + 1, col..col + 1).unwrap();
    view.style(row, col).and_then(|s| s.background.clone())
}

#[test]
fn libreoffices_rules_are_read_in_their_order_and_merged_over_their_cells() {
    let app = open(&sample());
    let rules = app.rules(0).unwrap();
    assert_eq!(
        rules,
        [
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "[.A1]>0", fill("#ff0000")),
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "[.A1]>1", fill("#00ff00")),
            // Two cell styles (bold, italic), one rule: LibreOffice reads it back as one, and so
            // does this build.
            Rule::over(Pos::new(1, 1), Pos::new(2, 2), "[.$A2]>0", fill("#00ff00")),
        ]
    );
}

#[test]
fn the_viewport_draws_the_first_rule_that_holds_over_the_cells_own_style() {
    let app = open(&sample());
    // A1 = 5: both rules hold, the first is drawn.
    assert_eq!(background(&app, 0, 0).as_deref(), Some("#ff0000"));
    // A2 = 1: row 2's cells are green, and keep their own bold and italic under it.
    assert_eq!(background(&app, 1, 1).as_deref(), Some("#00ff00"));
    let view = app.get_viewport(0, 1..2, 1..3).unwrap();
    assert_eq!(
        view.style(1, 1).unwrap().font_weight.as_deref(),
        Some("bold")
    );
    assert_eq!(
        view.style(1, 2).unwrap().font_style.as_deref(),
        Some("italic")
    );
    // A3 = 0: nothing holds on row 3.
    assert_eq!(background(&app, 2, 1), None);
    // And it follows the value.
    app.set_cell(0, Pos::new(2, 0), CellValue::Number(3.0))
        .unwrap();
    assert_eq!(background(&app, 2, 1).as_deref(), Some("#00ff00"));
}

#[test]
fn an_untouched_save_is_the_files_own_bytes() {
    let bytes = sample();
    let app = open(&bytes);
    app.set_cell(0, Pos::new(2, 0), CellValue::Number(3.0))
        .unwrap();
    app.set_cell(0, Pos::new(2, 0), CellValue::Number(0.0))
        .unwrap();
    let saved = app.save_bytes(Form::Flat).unwrap();
    let back = open(&saved);
    assert_eq!(back.rules(0).unwrap(), app.rules(0).unwrap());
}

#[test]
fn rules_written_by_this_build_read_back_as_they_were() {
    let app = App::new();
    for (row, value) in [5.0, 1.0, 0.0].into_iter().enumerate() {
        app.set_cell(0, Pos::new(row as u32, 0), CellValue::Number(value))
            .unwrap();
        app.set_cell(0, Pos::new(row as u32, 3), CellValue::Text("x".into()))
            .unwrap();
    }
    let rules = vec![
        Rule {
            ranges: vec![
                (Pos::new(0, 1), Pos::new(1, 2)),
                (Pos::new(2, 3), Pos::new(2, 3)),
            ],
            base: Pos::new(0, 1),
            condition: "[.$A1]>0".into(),
            style: CellStyle {
                color: Some("#ffffff".into()),
                ..fill("#32ad6d")
            },
        },
        Rule::over(Pos::new(0, 0), Pos::new(2, 0), "[.A1]=0", fill("#ff0000")),
    ];
    app.set_rules(0, rules.clone()).unwrap();
    for form in [Form::Flat, Form::Package] {
        let saved = app.save_bytes(form).unwrap();
        let back = App::new();
        back.open_bytes("back.ods", &saved).unwrap();
        // The file states the order only where two rules meet on a cell — a map's place in its
        // style — so two that never do come back in the order their first cell is met.
        let mut back_rules = back.rules(0).unwrap();
        back_rules.reverse();
        assert_eq!(back_rules, rules, "{form:?}");
        // The blank cells a rule covers are drawn too.
        assert_eq!(background(&back, 0, 2).as_deref(), Some("#32ad6d"));
        assert_eq!(background(&back, 2, 0).as_deref(), Some("#ff0000"));
    }
    // One undo step takes them all away.
    assert!(app.undo());
    assert!(app.rules(0).unwrap().is_empty());
}

#[test]
fn a_changed_rule_replaces_libreoffices_own_copy_and_keeps_everything_else() {
    let bytes = sample();
    let app = open(&bytes);
    let mut rules = app.rules(0).unwrap();
    rules[2].style = fill("#0000ff");
    rules.remove(0);
    app.set_rules(0, rules.clone()).unwrap();
    let saved = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    let back = open(saved.as_bytes());
    assert_eq!(back.rules(0).unwrap(), rules);
    assert!(!saved.contains("calcext:conditional-format"), "{saved}");
    // The cells keep their own bold and italic.
    let view = back.get_viewport(0, 1..2, 1..3).unwrap();
    assert_eq!(
        view.style(1, 1).unwrap().font_weight.as_deref(),
        Some("bold")
    );
    assert_eq!(
        view.style(1, 1).unwrap().background.as_deref(),
        Some("#0000ff")
    );
}

#[test]
fn two_rules_on_one_cell_keep_their_order_through_a_save() {
    let app = App::new();
    app.set_cell(0, Pos::new(0, 0), CellValue::Number(5.0))
        .unwrap();
    let rules = vec![
        Rule::over(Pos::new(0, 0), Pos::new(0, 0), "[.A1]>1", fill("#00ff00")),
        Rule::over(Pos::new(0, 0), Pos::new(0, 0), "[.A1]>0", fill("#ff0000")),
    ];
    app.set_rules(0, rules.clone()).unwrap();
    let back = open(&app.save_bytes(Form::Flat).unwrap());
    assert_eq!(back.rules(0).unwrap(), rules);
    assert_eq!(background(&back, 0, 0).as_deref(), Some("#00ff00"));
}

#[test]
fn a_condition_that_is_not_a_formula_is_refused() {
    let app = App::new();
    let bad = Rule::over(Pos::new(0, 0), Pos::new(0, 0), "[.A1]>(", fill("#ff0000"));
    assert!(app.set_rules(0, vec![bad]).is_err());
    assert!(app.rules(0).unwrap().is_empty());
}

#[test]
fn the_reference_index_knows_what_a_rule_reads_on_every_cell_it_covers() {
    use grind_sheet::formula::eval::Address;
    let doc = grind_sheet::read_bytes("rules.fods", &sample()).unwrap();
    let index = grind_sheet::graph::RefIndex::build(&doc);
    // `[.$A2]>0` over B2:C3 reads A2 from row 2 and A3 from row 3 — and nothing reads A4.
    assert!(index.read_by_a_rule(Address::new(0, Pos::new(1, 0))));
    assert!(index.read_by_a_rule(Address::new(0, Pos::new(2, 0))));
    assert!(index.is_referenced(Address::new(0, Pos::new(2, 0))));
    assert!(!index.is_referenced(Address::new(0, Pos::new(3, 0))));
}

#[test]
fn renaming_a_sheet_or_a_name_carries_every_rule_that_uses_it() {
    let app = App::new();
    app.add_sheet("Ticks").unwrap();
    app.set_name("limit", "=5").unwrap();
    app.set_rules(
        0,
        vec![
            Rule::over(Pos::new(0, 0), Pos::new(3, 0), "[$Ticks.$A1]", fill("#00ff00")),
            Rule::over(Pos::new(0, 1), Pos::new(3, 1), "[.B1]>limit", fill("#ff0000")),
        ],
    )
    .unwrap();
    app.rename_sheet(1, "Done").unwrap();
    app.rename_name("limit", "cap").unwrap();
    let rules = app.rules(0).unwrap();
    assert_eq!(rules[0].condition, "[$Done.$A1]");
    assert_eq!(rules[1].condition, "[.B1]>cap");
    app.inline_name("cap").unwrap();
    assert_eq!(app.rules(0).unwrap()[1].condition, "[.B1]>5");
}
