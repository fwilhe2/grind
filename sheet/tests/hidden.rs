// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Rows and columns hidden by hand, against `samples/hidden-rows-cols.fods`: LibreOffice's
//! own `table:visibility="collapse"` on column C and row 3 (both 1-based, as a person wrote
//! them; 0-based col 2 and row 2 in the core).

use std::path::PathBuf;

fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/samples/hidden-rows-cols.fods")
}

#[test]
fn libreoffice_collapsed_column_c_and_row_three() {
    let doc = grind_sheet::read_file(&sample()).expect("loads");
    let sheet = doc.sheet(0).expect("one sheet");

    assert!(sheet.col_hidden(2), "column C");
    assert!(!sheet.col_hidden(0), "column A stayed visible");
    assert!(!sheet.col_hidden(1), "column B stayed visible");
    assert_eq!(sheet.hidden_cols().collect::<Vec<_>>(), vec![2]);

    assert!(sheet.row_manually_hidden(2), "row 3");
    assert!(!sheet.row_manually_hidden(1), "row 2 stayed visible");
    assert_eq!(sheet.manually_hidden_rows().collect::<Vec<_>>(), vec![2]);

    // A manually hidden row is not what the (nonexistent, here) filter hides — the two
    // stay distinguishable even though both mean "do not draw this".
    assert!(sheet.hidden_rows(doc.null_date).is_empty());
}

/// Our own file says the same thing: hidden columns and rows both survive being written
/// and read back, spelled `table:visibility="collapse"`.
#[test]
fn hidden_tracks_survive_our_own_round_trip() {
    let doc = grind_sheet::read_file(&sample()).expect("loads");
    let bytes = grind_sheet::write_bytes(&doc, grind_sheet::Form::Flat).expect("writes");
    let back = grind_sheet::read_bytes("out.fods", &bytes).expect("reads back");
    let sheet = back.sheet(0).expect("one sheet");

    assert_eq!(sheet.hidden_cols().collect::<Vec<_>>(), vec![2]);
    assert_eq!(sheet.manually_hidden_rows().collect::<Vec<_>>(), vec![2]);
    assert_eq!(
        bytes
            .windows(b"table:visibility=\"collapse\"".len())
            .filter(|w| *w == b"table:visibility=\"collapse\"")
            .count(),
        2,
        "one collapse mark for the column, one for the row"
    );
}

/// Hiding and unhiding through `App` — undo included — end to end.
#[test]
fn hide_and_unhide_through_app() {
    let app = grind_sheet::App::new();
    app.open_bytes("book.fods", &std::fs::read(sample()).expect("read"))
        .expect("opens");

    assert_eq!(app.hidden_cols(0).unwrap(), vec![2]);
    assert_eq!(app.manually_hidden_rows(0).unwrap(), vec![2]);

    // Hide column A too.
    let changed = app.set_col_hidden(0, 0..1, true).unwrap();
    assert_eq!(changed, 1);
    assert_eq!(app.hidden_cols(0).unwrap(), vec![0, 2]);

    // Unhide column C.
    app.set_col_hidden(0, 2..3, false).unwrap();
    assert_eq!(app.hidden_cols(0).unwrap(), vec![0]);

    assert!(app.undo(), "unhiding C");
    assert_eq!(app.hidden_cols(0).unwrap(), vec![0, 2]);
    assert!(app.undo(), "hiding A");
    assert_eq!(app.hidden_cols(0).unwrap(), vec![2]);

    // Rows behave the same way.
    app.set_row_hidden(0, 0..1, true).unwrap();
    assert_eq!(app.manually_hidden_rows(0).unwrap(), vec![0, 2]);
    app.set_row_hidden(0, 2..3, false).unwrap();
    assert_eq!(app.manually_hidden_rows(0).unwrap(), vec![0]);
}
