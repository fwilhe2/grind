// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checkboxes in cells (`doc/ods-format.md` §3.5), against `samples/checkboxes.fods` —
//! LibreOffice's own spelling of three checkboxes linked to cells, with the properties it adds —
//! and through `App`.

use std::path::PathBuf;

use grind_sheet::{App, CellValue, Checkbox, Form, Link, Pos, RecalcMode};

fn sample() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/samples/checkboxes.fods"),
    )
    .unwrap()
}

fn open(bytes: &[u8]) -> App {
    let app = App::new();
    app.open_bytes("chores.fods", bytes).unwrap();
    app
}

fn linked(row: u32, col: u32) -> Checkbox {
    Checkbox {
        link: Some(Link {
            sheet: None,
            pos: Pos::new(row, col),
        }),
        ..Default::default()
    }
}

/// The lines of `b` that differ from `a`, by distance from either end.
fn changed<'s>(a: &'s str, b: &'s str) -> Vec<&'s str> {
    let (a, b): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    b[head..b.len().saturating_sub(tail).max(head)].to_vec()
}

#[test]
fn libreoffices_checkboxes_are_read_with_their_links() {
    let app = open(&sample());
    let boxes = app.checkboxes(0).unwrap();
    let at: Vec<(Pos, Option<Pos>)> = boxes
        .iter()
        .map(|(pos, c)| (*pos, c.link.as_ref().map(|l| l.pos)))
        .collect();
    assert_eq!(
        at,
        [
            (Pos::new(1, 1), Some(Pos::new(1, 4))),
            (Pos::new(2, 1), Some(Pos::new(2, 4))),
            (Pos::new(3, 1), Some(Pos::new(3, 4))),
        ]
    );
    let view = app.get_viewport(0, 0..6, 0..6).unwrap();
    assert_eq!(view.checkbox(1, 1), Some(true), "E2 is TRUE");
    assert_eq!(view.checkbox(2, 1), Some(false));
}

/// Ticking is the linked cell's value, recalculated in the same step: the count follows, and
/// one undo takes both back. A cell holding a boolean gets a boolean.
#[test]
fn ticking_writes_the_linked_cell_and_the_count_follows() {
    let app = open(&sample());
    // LibreOffice keeps a tick as `1`/`0` with a boolean format, so that is what the cell
    // holds — and what ticking writes back, so a sum over them still counts.
    app.enter(0, Pos::new(6, 2), "=SUM([.E2:.E4])", RecalcMode::Document)
        .unwrap();
    assert_eq!(app.get(0, Pos::new(6, 2)).unwrap(), CellValue::Number(1.0));
    assert!(app.toggle_checkbox(0, Pos::new(2, 1)).unwrap());
    assert_eq!(app.get(0, Pos::new(2, 4)).unwrap(), CellValue::Number(1.0));
    assert_eq!(app.get(0, Pos::new(6, 2)).unwrap(), CellValue::Number(2.0));
    assert!(app.undo());
    assert_eq!(app.get(0, Pos::new(2, 4)).unwrap(), CellValue::Number(0.0));
    assert_eq!(app.get(0, Pos::new(6, 2)).unwrap(), CellValue::Number(1.0));
    assert!(
        app.toggle_checkbox(0, Pos::new(5, 5)).is_err(),
        "no checkbox there"
    );

    let fresh = App::new();
    fresh
        .set_checkbox(0, Pos::new(0, 0), Some(linked(0, 1)))
        .unwrap();
    fresh.toggle_checkbox(0, Pos::new(0, 0)).unwrap();
    assert_eq!(fresh.get(0, Pos::new(0, 1)).unwrap(), CellValue::Bool(true));
}

/// R6: an untouched save is the file; ticking is one value; adding one is one element in the
/// file's own form and its cell; taking LibreOffice's own away removes exactly its element and
/// its shape, properties and all.
#[test]
fn saving_changes_only_the_checkboxes_that_changed() {
    let original = sample();
    let app = open(&original);
    assert_eq!(app.save_bytes(Form::Flat).unwrap(), original);
    let before = String::from_utf8(original.clone()).unwrap();

    app.toggle_checkbox(0, Pos::new(2, 1)).unwrap();
    let ticked = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    let lines = changed(&before, &ticked);
    assert!(
        lines.iter().all(|l| !l.contains("form:")),
        "a tick touches no control: {lines:#?}"
    );

    app.set_checkbox(0, Pos::new(4, 1), Some(linked(4, 4)))
        .unwrap();
    let added = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    assert_eq!(added.matches("<form:checkbox").count(), 4);
    assert_eq!(
        added
            .matches("form:property-name=\"ControlTypeinMSO\"")
            .count(),
        3,
        "the three LibreOffice wrote keep their own properties"
    );

    app.set_checkbox(0, Pos::new(1, 1), None).unwrap();
    let removed = app.save_bytes(Form::Flat).unwrap();
    let text = String::from_utf8(removed.clone()).unwrap();
    assert_eq!(text.matches("<form:checkbox").count(), 3);
    assert_eq!(text.matches("<draw:control").count(), 3);

    let back = open(&removed);
    let at: Vec<Pos> = back
        .checkboxes(0)
        .unwrap()
        .iter()
        .map(|(p, _)| *p)
        .collect();
    assert_eq!(at, [Pos::new(2, 1), Pos::new(3, 1), Pos::new(4, 1)]);
    assert_eq!(back.get(0, Pos::new(2, 4)).unwrap(), CellValue::Number(1.0));
}

/// A document of our own writes checkboxes the schema's way, in both forms, and reads them back.
#[test]
fn checkboxes_survive_our_own_round_trip_in_both_forms() {
    let app = App::new();
    app.set_cell(0, Pos::new(0, 4), true).unwrap();
    app.set_checkbox(0, Pos::new(0, 1), Some(linked(0, 4)))
        .unwrap();
    app.set_checkbox(
        0,
        Pos::new(1, 1),
        Some(Checkbox {
            checked: true,
            label: Some("Done".into()),
            ..Default::default()
        }),
    )
    .unwrap();
    for form in [Form::Flat, Form::Package] {
        let back = App::new();
        back.open_bytes("book", &app.save_bytes(form).unwrap())
            .unwrap();
        assert_eq!(
            back.checkboxes(0).unwrap(),
            app.checkboxes(0).unwrap(),
            "{form:?}"
        );
    }
    let text = app.project().text().to_owned();
    assert!(text.contains("checkbox B1 link=E1"), "{text}");
    assert!(
        text.contains("checkbox B2 checked=#true label=Done"),
        "{text}"
    );
    let back = App::new();
    back.open_bytes("book.grind", text.as_bytes()).unwrap();
    assert_eq!(back.checkboxes(0).unwrap(), app.checkboxes(0).unwrap());
}

/// Typing into a checkbox's own cell keeps the checkbox — in the value-only splice and in a save
/// that regenerates the row — under the identifier the file's form already names.
#[test]
fn typing_into_a_checkboxs_cell_keeps_the_checkbox() {
    let app = open(&sample());
    app.enter(0, Pos::new(1, 1), "=1+1", RecalcMode::No)
        .unwrap();
    let spliced = app.save_bytes(Form::Flat).unwrap();
    let back = open(&spliced);
    assert_eq!(back.checkboxes(0).unwrap(), app.checkboxes(0).unwrap());
    assert_eq!(
        String::from_utf8_lossy(&spliced)
            .matches("draw:control=\"control1\"")
            .count(),
        1
    );

    // And with a style change beside it, which makes the save regenerate rows.
    app.set_style(
        0,
        Pos::new(1, 2),
        Pos::new(1, 2),
        Some(grind_sheet::style::CellStyle {
            font_weight: Some("bold".into()),
            ..Default::default()
        }),
    )
    .unwrap();
    app.enter(0, Pos::new(2, 1), "x", RecalcMode::No).unwrap();
    let regenerated = app.save_bytes(Form::Flat).unwrap();
    let back = open(&regenerated);
    assert_eq!(back.checkboxes(0).unwrap(), app.checkboxes(0).unwrap());
}

/// Renaming the sheet respells every checkbox's link in place — its element, its identifier and
/// its properties kept — and a checkbox on another sheet linked to the renamed one follows it.
#[test]
fn renaming_a_sheet_carries_the_checkboxes_links() {
    let app = open(&sample());
    let other = app.add_sheet("Summary").unwrap();
    app.set_checkbox(
        other,
        Pos::new(0, 0),
        Some(Checkbox {
            link: Some(Link {
                sheet: Some("Chores".into()),
                pos: Pos::new(1, 4),
            }),
            ..Default::default()
        }),
    )
    .unwrap();
    app.rename_sheet(0, "Jobs").unwrap();
    let saved = app.save_bytes(Form::Flat).unwrap();
    let text = String::from_utf8_lossy(&saved);
    assert!(
        text.contains("form:linked-cell=\"Jobs.E2\""),
        "respelled in place"
    );
    assert!(text.contains("xml:id=\"control1\""), "its identifier kept");
    assert!(text.contains("form:linked-cell=\"Jobs.E2\"") && !text.contains("Chores.E"));
    let back = open(&saved);
    assert_eq!(back.checkboxes(0).unwrap(), app.checkboxes(0).unwrap());
    let summary = back.checkboxes(other).unwrap();
    assert_eq!(
        summary[0].1.link.as_ref().and_then(|l| l.sheet.as_deref()),
        Some("Jobs")
    );
}
