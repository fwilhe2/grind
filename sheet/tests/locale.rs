// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document's own locale — `App::set_locale`, and everything it decides: how an unmarked
//! number is shown, how a typed one is read, and that the two agree (`doc/ods-format.md` §5.2,
//! "The document's own language").

use grind_sheet::locale::Locale;
use grind_sheet::numfmt::{self, Kind};
use grind_sheet::{App, CellValue, Form, Pos, RecalcMode};

fn german() -> Option<Locale> {
    Locale::parse("de-DE")
}

fn shown(app: &App, pos: Pos) -> String {
    app.get_viewport(0, pos.row..pos.row + 1, pos.col..pos.col + 1)
        .unwrap()
        .text(pos.row, pos.col)
        .unwrap_or_default()
        .to_owned()
}

fn value(app: &App, pos: Pos) -> CellValue {
    app.get_viewport(0, pos.row..pos.row + 1, pos.col..pos.col + 1)
        .unwrap()
        .get(pos.row, pos.col)
        .cloned()
        .unwrap_or(CellValue::Empty)
}

fn enter(app: &App, pos: Pos, text: &str) {
    app.enter(0, pos, text, RecalcMode::No).unwrap();
}

/// A number with no format, and one whose format names no locale, are spelled the document's
/// way; a format that names its own keeps it.
#[test]
fn a_document_locale_spells_every_unmarked_number() {
    let app = App::new();
    let (a1, a2, a3) = (Pos::new(0, 0), Pos::new(1, 0), Pos::new(2, 0));
    for pos in [a1, a2, a3] {
        app.set_cell(0, pos, 1234.5).unwrap();
    }
    let unmarked = numfmt::preset(Kind::Number, 2, true, numfmt::DEFAULT_CURRENCY);
    app.set_format(0, a2, a2, Some(unmarked.clone())).unwrap();
    app.set_format(0, a3, a3, Some(unmarked.in_locale(Locale::parse("en-US"))))
        .unwrap();
    assert_eq!(
        (shown(&app, a1), shown(&app, a2), shown(&app, a3)),
        ("1234.5".into(), "1,234.50".into(), "1,234.50".into())
    );

    app.set_locale(german()).unwrap();
    assert_eq!(app.locale(), german());
    assert_eq!(
        (shown(&app, a1), shown(&app, a2), shown(&app, a3)),
        ("1234,5".into(), "1.234,50".into(), "1,234.50".into()),
        "the plain number and the unmarked format speak German; the marked one keeps its own"
    );
    assert_eq!(app.display_number(-0.25), "-0,25");

    // One step back, and every number is spelled as it was.
    assert!(app.undo());
    assert_eq!(app.locale(), None);
    assert_eq!(shown(&app, a1), "1234.5");
}

/// Typing in a German document reads German — and only well-formed German.
#[test]
fn a_typed_number_is_read_the_documents_way() {
    let app = App::new();
    app.set_locale(german()).unwrap();
    let at = Pos::new(0, 0);
    for (typed, want) in [
        ("1,5", CellValue::Number(1.5)),
        ("1.234,5", CellValue::Number(1234.5)),
        ("-0,25", CellValue::Number(-0.25)),
        ("1e3", CellValue::Number(1000.0)),
        ("007", CellValue::Number(7.0)),
        // Not one and a half: `.` groups thousands here, and `1.5` is not a group of three.
        ("1.5", CellValue::Text("1.5".into())),
        ("1,2,3", CellValue::Text("1,2,3".into())),
    ] {
        enter(&app, at, typed);
        assert_eq!(value(&app, at), want, "{typed:?}");
    }
    // A document with no locale reads what it always read.
    let plain = App::new();
    enter(&plain, at, "1,5");
    assert_eq!(value(&plain, at), CellValue::Text("1,5".into()));
    enter(&plain, at, "1.5");
    assert_eq!(value(&plain, at), CellValue::Number(1.5));
}

/// What the formula bar puts in front of somebody is what typing it back reads as the same
/// number — in every locale, including none.
#[test]
fn what_the_formula_bar_shows_types_back_as_the_same_number() {
    let at = Pos::new(0, 0);
    for locale in [
        None,
        Locale::parse("en-US"),
        german(),
        Locale::parse("fr-FR"),
    ] {
        let app = App::new();
        app.set_locale(locale.clone()).unwrap();
        for n in [
            0.0,
            1.5,
            -1234.5,
            0.1,
            1e20,
            1.25e-7,
            123_456_789.0,
            -0.000_5,
        ] {
            app.set_cell(0, at, n).unwrap();
            let text = app.input_text(0, at).unwrap();
            enter(&app, at, &text);
            assert_eq!(
                value(&app, at),
                CellValue::Number(n),
                "{locale:?}: {n} shown as {text:?}"
            );
        }
    }
}

/// A text that would read as a number in *this* document is quoted when it is put back in
/// front of somebody, so editing it and pressing Enter keeps it text.
#[test]
fn a_text_that_looks_like_a_german_number_is_quoted_for_editing() {
    let app = App::new();
    app.set_locale(german()).unwrap();
    let at = Pos::new(0, 0);
    app.set_cell(0, at, "1,5").unwrap();
    assert_eq!(app.input_text(0, at).unwrap(), "'1,5");
    app.set_cell(0, at, "1.5").unwrap();
    assert_eq!(
        app.input_text(0, at).unwrap(),
        "1.5",
        "not a number here, so no quote"
    );
}

/// The locale survives a save in both forms, and a document without one writes nothing for it.
#[test]
fn a_document_locale_survives_a_save_and_none_is_written_as_nothing() {
    let app = App::new();
    app.set_cell(0, Pos::new(0, 0), 1234.5).unwrap();
    let flat = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    assert!(!flat.contains("office:styles"), "R3: no locale, no element");

    app.set_locale(german()).unwrap();
    for form in [Form::Flat, Form::Package] {
        let bytes = app.save_bytes(form).unwrap();
        let back = App::new();
        back.open_bytes("test", &bytes).unwrap();
        assert_eq!(back.locale(), german(), "{form:?}");
        assert_eq!(shown(&back, Pos::new(0, 0)), "1234,5", "{form:?}");
    }
    let flat = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    assert!(flat.contains(
        "<style:default-style style:family=\"table-cell\"><style:text-properties \
         fo:language=\"de\" fo:country=\"DE\"/></style:default-style>"
    ));
    // A language with no country is a locale too, and writes no empty country.
    app.set_locale(Locale::parse("de")).unwrap();
    let flat = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    assert!(flat.contains("fo:language=\"de\"/>"));
}

/// A CSV is read by its own separators, whatever the document's — an English file imported into
/// a German document is still numbers.
#[test]
fn a_csv_is_read_by_its_own_separators_whatever_the_documents_are() {
    let app = App::new();
    app.set_locale(german()).unwrap();
    let options = grind_sheet::csv::Import::sniffed("price\n1234.5\n");
    app.import_csv(
        0,
        Pos::new(0, 0),
        "price\n1234.5\n",
        &options,
        RecalcMode::No,
    )
    .unwrap();
    assert_eq!(value(&app, Pos::new(1, 0)), CellValue::Number(1234.5));
    assert_eq!(shown(&app, Pos::new(1, 0)), "1234,5");
}

/// A filter keeps rows by what they show, and in a German document they show German — which is
/// also how a German LibreOffice saves the values a filter keeps.
#[test]
fn a_filter_keeps_rows_by_their_german_spelling() {
    let app = App::new();
    app.set_locale(german()).unwrap();
    app.set_cell(0, Pos::new(0, 0), "Price").unwrap();
    app.set_cell(0, Pos::new(1, 0), 1.5).unwrap();
    app.set_cell(0, Pos::new(2, 0), 2.5).unwrap();
    let mut filter = grind_sheet::Filter::new("f", Pos::new(0, 0), Pos::new(2, 0));
    filter.keep.insert(0, ["1,5".to_owned()].into());
    app.set_filter(0, Some(filter)).unwrap();
    assert_eq!(app.hidden_rows(0).unwrap(), vec![2]);
}

/// A format picker's sample: what a cell would show under a format nobody has set yet, in the
/// document's own spelling — and nothing changes.
#[test]
fn a_cell_shown_as_a_format_it_does_not_have_writes_nothing() {
    let app = App::new();
    app.set_locale(german()).unwrap();
    let at = Pos::new(0, 0);
    app.set_cell(0, at, 1234.5).unwrap();
    let euro = numfmt::preset(Kind::Currency, 2, true, "€");
    assert_eq!(app.shown_as(0, at, Some(&euro)).unwrap(), "1.234,50\u{a0}€");
    assert_eq!(app.shown_as(0, at, None).unwrap(), "1234,5");
    assert_eq!(
        app.format_at(0, at).unwrap(),
        None,
        "the cell keeps no format"
    );
    // The locale and the value are the only steps: nothing the samples did is one.
    assert!(app.undo() && app.undo());
    assert!(!app.can_undo());
}
