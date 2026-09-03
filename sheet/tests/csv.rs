// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! CSV and TSV against a document, rather than against a string.
//!
//! `sheet/src/csv.rs`'s own tests cover the parsing and the field rule; these cover what
//! happens when the result reaches cells — which typing rule ran, what an export shows, and
//! that the whole of an import is one undo entry.

use grind_sheet::csv::{Dialect, Export, Import};
use grind_sheet::{App, CellValue, Pos, RecalcMode};

fn empty() -> App {
    App::new()
}

fn value(app: &App, address: &str) -> CellValue {
    let (sheet, pos, _) = resolve(app, address);
    app.get(sheet, pos).unwrap()
}

fn resolve(app: &App, address: &str) -> (usize, Pos, Pos) {
    let reference = grind_sheet::a1::parse(address).unwrap();
    grind_sheet::a1::resolve(app, &reference).unwrap()
}

fn import(app: &App, text: &str, options: &Import) {
    app.import_csv(0, Pos::new(0, 0), text, options, RecalcMode::No)
        .unwrap();
}

fn export(app: &App, range: &str, options: &Export) -> String {
    let (sheet, start, end) = resolve(app, range);
    app.export_csv(sheet, start, end, options).unwrap()
}

#[test]
fn a_field_lands_as_the_type_the_typing_rule_gives_it() {
    let app = empty();
    import(
        &app,
        "Product,Code,Qty,Ok\nWidget,007,3,TRUE\n\"Smith, J\",A1,2.5,FALSE\n",
        &Import::default(),
    );
    assert_eq!(value(&app, "A1"), CellValue::Text("Product".to_owned()));
    // The leading zero survives, which is the whole reason this is not `f64::from_str`.
    assert_eq!(value(&app, "B2"), CellValue::Text("007".to_owned()));
    assert_eq!(value(&app, "C2"), CellValue::Number(3.0));
    assert_eq!(value(&app, "D2"), CellValue::Bool(true));
    assert_eq!(value(&app, "A3"), CellValue::Text("Smith, J".to_owned()));
    assert_eq!(value(&app, "C3"), CellValue::Number(2.5));
}

#[test]
fn what_is_exported_reads_back_as_the_same_document() {
    let app = empty();
    let text = "Product,Code,Qty\n\"Smith, J\",007,3\n\"said \"\"hi\"\"\",A1,2.5\n";
    import(&app, text, &Import::default());
    let out = export(&app, "A1:C3", &Export::default());
    assert_eq!(out, text);

    // And the same document again from what it wrote — the property an export is for.
    let second = empty();
    import(&second, &out, &Import::default());
    assert_eq!(
        export(&second, "A1:C3", &Export::default()),
        out,
        "an export of an import of an export"
    );
}

#[test]
fn a_semicolon_file_in_a_comma_locale_reads_as_numbers() {
    let app = empty();
    let options = Import {
        dialect: Dialect::SEMICOLON,
        locale: grind_sheet::locale::Locale::parse("de-DE"),
        ..Import::default()
    };
    import(
        &app,
        "Artikel;Preis\nSchraube;1.234,50\nMutter;0,75\n",
        &options,
    );
    assert_eq!(value(&app, "B2"), CellValue::Number(1234.5));
    assert_eq!(value(&app, "B3"), CellValue::Number(0.75));
}

#[test]
fn the_delimiter_is_sniffed_from_the_file_itself() {
    for (text, dialect) in [
        ("a,b\n1,2\n", Dialect::COMMA),
        ("a;b\n1;2\n", Dialect::SEMICOLON),
        ("a\tb\n1\t2\n", Dialect::TAB),
    ] {
        let app = empty();
        let options = Import {
            dialect: Dialect::sniff(text),
            ..Import::default()
        };
        assert_eq!(options.dialect, dialect, "{text:?}");
        import(&app, text, &options);
        assert_eq!(value(&app, "B2"), CellValue::Number(2.0), "{text:?}");
    }
}

#[test]
fn a_formula_field_is_text_until_it_is_asked_for() {
    let app = empty();
    import(&app, "1,2,=SUM(A1:B1)\n", &Import::default());
    assert_eq!(
        value(&app, "C1"),
        CellValue::Text("=SUM(A1:B1)".to_owned()),
        "a formula out of a file does not evaluate itself"
    );

    let asked = empty();
    let options = Import {
        formulas: true,
        ..Import::default()
    };
    asked
        .import_csv(
            0,
            Pos::new(0, 0),
            "1,2,=SUM(A1:B1)\n",
            &options,
            RecalcMode::Document,
        )
        .unwrap();
    assert_eq!(value(&asked, "C1"), CellValue::Number(3.0));
    // Stored canonical, shown in display form — the file carried the display spelling.
    assert_eq!(
        asked
            .formula(0, resolve(&asked, "C1").1)
            .unwrap()
            .as_deref(),
        Some("=SUM([.A1:.B1])")
    );
    assert_eq!(
        export(
            &asked,
            "C1",
            &Export {
                formulas: true,
                ..Export::default()
            }
        ),
        "=SUM(A1:B1)\n"
    );
}

#[test]
fn dates_arrive_as_dates_and_leave_as_dates() {
    let app = empty();
    let options = Import {
        dates: true,
        ..Import::default()
    };
    import(
        &app,
        "When,At,What\n2026-03-15,10:30,Invoice\n2026-04-01T09:15:00,11:00,Refund\n",
        &options,
    );
    // A number with a format, not a string — which is what makes it sortable and arithmetic.
    assert!(matches!(value(&app, "A2"), CellValue::Number(_)));
    assert!(matches!(value(&app, "B2"), CellValue::Number(_)));
    // The header is not a date and did not acquire a format by sitting above one.
    assert_eq!(value(&app, "A1"), CellValue::Text("When".to_owned()));
    assert!(app.format_at(0, resolve(&app, "A1").1).unwrap().is_none());

    // And the export says what the import read. The datetime comes back with a space where
    // the file had a `T`, which is the format's own spelling and reads straight back in.
    assert_eq!(
        export(&app, "A1:C3", &Export::default()),
        "When,At,What\n2026-03-15,10:30:00,Invoice\n2026-04-01 09:15:00,11:00:00,Refund\n"
    );

    // Without the flag they are text, and exactly the text the file held.
    let plain = empty();
    import(&plain, "2026-03-15\n", &Import::default());
    assert_eq!(
        value(&plain, "A1"),
        CellValue::Text("2026-03-15".to_owned())
    );
}

#[test]
fn an_import_is_one_undo_entry_even_when_it_formats_cells() {
    let app = empty();
    let options = Import {
        dates: true,
        ..Import::default()
    };
    import(&app, "When,Amount\n2026-03-15,100\n", &options);
    assert!(matches!(value(&app, "A2"), CellValue::Number(_)));

    // One step takes back the values *and* the formats that gave them meaning. Two entries
    // would leave a date-formatted empty cell behind, which is a document nobody asked for.
    assert!(app.undo());
    assert_eq!(value(&app, "A1"), CellValue::Empty);
    assert_eq!(value(&app, "A2"), CellValue::Empty);
    assert!(app.format_at(0, resolve(&app, "A2").1).unwrap().is_none());
    assert!(!app.can_undo(), "the import was one entry, not two");
}

#[test]
fn an_export_shows_what_a_cell_shows() {
    let app = empty();
    app.enter(0, Pos::new(0, 0), "0.155", RecalcMode::No)
        .unwrap();
    let (_, pos, _) = resolve(&app, "A1");
    app.set_format(
        0,
        pos,
        pos,
        Some(grind_sheet::numfmt::preset(
            grind_sheet::numfmt::Kind::Percentage,
            1,
            false,
            "",
        )),
    )
    .unwrap();
    // The number format is the only place a document says a number is a proportion, so an
    // export that ignored it would hand on 0.155 with no way to know what it was.
    assert_eq!(export(&app, "A1", &Export::default()), "15.5%\n");
}

#[test]
fn a_file_bigger_than_one_paste_is_refused_by_name() {
    let app = empty();
    let row = "1,2,3,4,5,6,7,8,9,10\n";
    let text = row.repeat(7000);
    let error = app
        .import_csv(0, Pos::new(0, 0), &text, &Import::default(), RecalcMode::No)
        .unwrap_err()
        .to_string();
    assert!(error.contains("70000"), "{error}");
    // And nothing landed: the size check runs before the document is touched.
    assert_eq!(value(&app, "A1"), CellValue::Empty);
}
