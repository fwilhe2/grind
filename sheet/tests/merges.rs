// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Merged cells: `table:number-columns-spanned`/`table:number-rows-spanned` on the top-left
//! cell and `table:covered-table-cell` for the rest (rng:16102, rng:14298), against
//! `samples/Quarterly Sales Report.fods` — LibreOffice's own merges — and through `App`.

use std::path::PathBuf;

use grind_sheet::{App, CellValue, Form, Pos, Span};

fn quarterly() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/samples/Quarterly Sales Report.fods")
}

fn p(address: &str) -> Pos {
    let reference = grind_sheet::a1::parse(address).unwrap();
    grind_sheet::a1::resolve(&App::new(), &reference).unwrap().1
}

#[test]
fn libreoffice_merges_are_read() {
    let doc = grind_sheet::read_file(&quarterly()).expect("loads");
    let sheet = doc.sheet(0).expect("a sheet");
    let merges: Vec<(Pos, Span)> = sheet.merges().collect();
    assert!(
        merges.contains(&(p("A1"), Span { cols: 6, rows: 1 })),
        "the title"
    );
    assert!(
        merges.contains(&(p("A5"), Span { cols: 2, rows: 2 })),
        "a 2×2 block"
    );
    assert_eq!(sheet.merge_at(p("F1")).map(|(a, _)| a), Some(p("A1")));
    assert!(sheet.covered(p("B6")));
    assert!(!sheet.covered(p("A5")), "the top-left cell is not covered");
    assert_eq!(sheet.merge_at(p("A3")), None);
}

#[test]
fn merges_survive_our_own_round_trip_in_both_forms() {
    let doc = grind_sheet::read_file(&quarterly()).expect("loads");
    // Regenerated rather than spliced: a document with no source writes every row itself.
    let fresh = grind_sheet::Document {
        source: None,
        ..doc.clone()
    };
    for form in [Form::Flat, Form::Package] {
        let bytes = grind_sheet::write_bytes(&fresh, form).expect("writes");
        let back = grind_sheet::read_bytes("out", &bytes).expect("reads back");
        assert_eq!(
            back.sheet(0).unwrap().merges().collect::<Vec<_>>(),
            doc.sheet(0).unwrap().merges().collect::<Vec<_>>(),
            "{form:?}"
        );
    }
}

/// A covered cell has the content model of any other, and LibreOffice keeps a value there out
/// of sight: so does this model, and unmerging shows it again.
#[test]
fn a_value_under_a_merge_is_kept_and_comes_back_on_unmerge() {
    let app = App::new();
    app.set_cell(0, p("B2"), "Heading").unwrap();
    app.set_cell(0, p("C2"), 42.0).unwrap();
    assert!(app.merge(0, p("B2"), p("D2")).unwrap());

    let bytes = app.save_bytes(Form::Flat).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        text.contains("table:number-columns-spanned=\"3\""),
        "{text}"
    );
    assert!(text.contains("<table:covered-table-cell"), "{text}");

    let back = App::new();
    back.open_bytes("book.fods", &bytes).unwrap();
    assert_eq!(back.merges(0).unwrap(), vec![(p("B2"), p("D2"))]);
    assert_eq!(back.get(0, p("C2")).unwrap(), CellValue::Number(42.0));
    assert_eq!(back.unmerge(0, p("C2"), p("C2")).unwrap(), 1);
    assert_eq!(back.merges(0).unwrap(), vec![]);
    assert_eq!(back.get(0, p("C2")).unwrap(), CellValue::Number(42.0));
}

#[test]
fn merging_replaces_what_it_overlaps_in_one_undo_step() {
    let app = App::new();
    app.merge(0, p("A1"), p("B1")).unwrap();
    app.merge(0, p("D1"), p("E2")).unwrap();
    // Across both, from the bottom-right corner: one merge, anchored top-left.
    assert!(app.merge(0, p("E2"), p("A1")).unwrap());
    assert!(
        !app.merge(0, p("A1"), p("E2")).unwrap(),
        "already exactly that"
    );
    assert_eq!(app.merges(0).unwrap(), vec![(p("A1"), p("E2"))]);
    assert_eq!(app.merge_at(0, p("C2")).unwrap(), Some((p("A1"), p("E2"))));

    assert!(app.undo());
    assert_eq!(
        app.merges(0).unwrap(),
        vec![(p("A1"), p("B1")), (p("D1"), p("E2"))]
    );
    assert!(app.redo());
    assert_eq!(app.merges(0).unwrap(), vec![(p("A1"), p("E2"))]);

    assert!(
        app.merge(0, p("C3"), p("C3")).is_err(),
        "one cell is no merge"
    );
}

/// The viewport carries a merge whose top-left cell has scrolled out of it, with that cell's
/// text — which is what a renderer draws over the part still in view.
#[test]
fn the_viewport_carries_a_merge_whose_anchor_is_out_of_view() {
    let app = App::new();
    app.set_cell(0, p("A1"), "Title").unwrap();
    app.merge(0, p("A1"), p("D3")).unwrap();
    let view = app.get_viewport(0, 1..10, 2..10).unwrap();
    let merge = view.merge_at(2, 3).expect("C3 is under the merge");
    assert_eq!((merge.anchor, merge.end), (p("A1"), p("D3")));
    assert_eq!(merge.text, "Title");
    assert!(view.covered(1, 2));
    assert!(view.merge_at(5, 5).is_none());
}

/// R6: an untouched save is the file, and a merge made in it rewrites only the rows it spans.
#[test]
fn merging_in_a_file_rewrites_only_the_rows_it_spans() {
    let original = std::fs::read(quarterly()).unwrap();
    let app = App::new();
    app.open_bytes("q.fods", &original).unwrap();
    assert_eq!(app.save_bytes(Form::Flat).unwrap(), original);

    // Row 3 is the blank one between the subtitle and the cards.
    app.merge(0, p("A3"), p("F3")).unwrap();
    let saved = app.save_bytes(Form::Flat).unwrap();
    let (before, after) = (
        String::from_utf8_lossy(&original),
        String::from_utf8_lossy(&saved),
    );
    let changed: Vec<_> = diff_lines(&before, &after);
    assert_eq!(
        changed,
        vec![
            "     <table:table-cell table:number-columns-spanned=\"6\"/>\
             <table:covered-table-cell table:number-columns-repeated=\"5\"/>"
        ],
        "the row's own element, split — its style and every other row untouched"
    );

    let back = App::new();
    back.open_bytes("q.fods", &saved).unwrap();
    assert!(back.merges(0).unwrap().contains(&(p("A3"), p("F3"))));
    assert!(back.merges(0).unwrap().contains(&(p("A1"), p("F1"))));

    // Unmerging LibreOffice's own 2×1 cards: the spans go from the top-left cells and the
    // covered cells become cells again, each keeping its style.
    assert_eq!(back.unmerge(0, p("A4"), p("F4")).unwrap(), 3);
    let unmerged = String::from_utf8(back.save_bytes(Form::Flat).unwrap()).unwrap();
    let again = App::new();
    again.open_bytes("q.fods", unmerged.as_bytes()).unwrap();
    assert!(again.merge_at(0, p("B4")).unwrap().is_none());
    assert!(unmerged.contains("<table:table-cell table:style-name=\"ce5\"/>"));
    assert_eq!(
        again.merges(0).unwrap().len(),
        back.merges(0).unwrap().len()
    );
}

/// The lines of `b` that are not lines of `a` at the same distance from either end — enough
/// to count a one-row splice without a diff library.
fn diff_lines<'s>(a: &'s str, b: &'s str) -> Vec<&'s str> {
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
fn merges_project_and_read_back() {
    let app = App::new();
    app.set_cell(0, p("B2"), "Heading").unwrap();
    app.merge(0, p("B2"), p("D2")).unwrap();
    app.merge(0, p("A4"), p("A6")).unwrap();
    let text = app.project().text().to_owned();
    assert!(text.contains("merge B2:D2"), "{text}");
    assert!(text.contains("merge A4:A6"), "{text}");

    let back = App::new();
    back.open_bytes("book.grind", text.as_bytes()).unwrap();
    assert_eq!(back.merges(0).unwrap(), app.merges(0).unwrap());

    let overlapping = text.replace("merge A4:A6", "merge A4:A6\n    merge A5:B5");
    assert!(
        App::new()
            .open_bytes("bad.grind", overlapping.as_bytes())
            .is_err(),
        "two merges over one cell are refused"
    );
}

/// A value typed into the covered half of a file's merge used to be refused, since the model
/// had nowhere to put it. A covered cell holds one like any other now, and the merge stays.
#[test]
fn a_value_typed_under_a_files_merge_is_saved_and_the_merge_kept() {
    let original = std::fs::read(quarterly()).unwrap();
    let app = App::new();
    app.open_bytes("q.fods", &original).unwrap();
    app.set_cell(0, p("B4"), "hidden").unwrap();
    // And in the same row as a new merge, a value typed into its top-left cell.
    app.set_cell(0, p("A3"), "Cards").unwrap();
    app.merge(0, p("A3"), p("C3")).unwrap();
    let saved = app
        .save_bytes(Form::Flat)
        .expect("saves rather than refusing");

    let back = App::new();
    back.open_bytes("q.fods", &saved).unwrap();
    assert_eq!(
        back.get(0, p("B4")).unwrap(),
        CellValue::Text("hidden".into())
    );
    assert_eq!(back.merge_at(0, p("B4")).unwrap(), Some((p("A4"), p("B4"))));
    assert_eq!(
        back.get(0, p("A3")).unwrap(),
        CellValue::Text("Cards".into())
    );
    assert_eq!(back.merge_at(0, p("C3")).unwrap(), Some((p("A3"), p("C3"))));
}
