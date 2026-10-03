// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Saving never makes an existing file worse**, held to every spreadsheet vendored here that
//! somebody else's program wrote — R7's documents and the samples — under ten ordinary edits.
//!
//! Each save has exactly two allowed outcomes. Either it writes, and then the file reads back
//! with the edit in it (the writer's own read-back check) and with every byte outside
//! `office:body` the original's; or it is **refused** with `Error::WouldLose`, naming what it
//! would have dropped, and nothing is written. Any other outcome — an error of another kind, a
//! file that does not open — fails here.
//!
//! The refusals are listed, so the list is a ratchet: the day this build learns to carry what
//! one of them names, the save goes through, this test fails, and the row is deleted.

use std::path::PathBuf;

use grind_sheet::model::Pos;
use grind_sheet::{App, Form};

/// Saves refused today, by document and edit — each for a reason the message names.
///
/// Renaming a sheet rewrites the ranges of every chart that reads it, and a chart is
/// regenerated whole: its date axis, its own number styles and any drawing grouped beside it
/// in `table:shapes` would go. Patching a chart's ranges in place is the fix.
const REFUSED: &[(&str, &str)] = &[
    ("Quarterly Sales Report.fods", "rename"),
    ("spreadsheet.fods", "rename"),
];

type Edit = (&'static str, fn(&App));

const EDITS: &[Edit] = &[
    ("value", |a| {
        a.set_cell(0, Pos::new(0, 0), 7.0).unwrap();
    }),
    ("bold", |a| {
        let bold = grind_sheet::style::CellStyle {
            font_weight: Some("bold".into()),
            ..Default::default()
        };
        a.set_style(0, Pos::new(1, 1), Pos::new(1, 1), Some(bold))
            .unwrap();
    }),
    ("far", |a| {
        a.set_cell(0, Pos::new(300, 2), 5.0).unwrap();
    }),
    ("width", |a| {
        a.set_col_width(0, 0..1, Some("4cm".into())).unwrap();
    }),
    ("height", |a| {
        a.set_row_height(0, 0..1, Some("1cm".into())).unwrap();
    }),
    ("hide", |a| {
        a.set_row_hidden(0, 1..2, true).unwrap();
    }),
    ("rename", |a| {
        a.rename_sheet(0, "Renamed").unwrap();
    }),
    ("add", |a| {
        a.add_sheet("Extra").unwrap();
    }),
    ("remove", |a| {
        if a.sheet_count() > 1 {
            a.remove_sheet(a.sheet_count() - 1).unwrap();
        }
    }),
    ("name", |a| {
        a.set_name("zz_new", "[.$A$1]").unwrap();
    }),
];

fn documents() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = ["kb", "samples"]
        .iter()
        .flat_map(|dir| {
            std::fs::read_dir(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/data")
                    .join(dir),
            )
            .expect("a fixture directory")
        })
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "fods" || e == "ods"))
        .collect();
    files.sort();
    files
}

/// Everything before `office:body` and after it, which no edit to a cell may touch.
fn outside_body(xml: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let text = String::from_utf8_lossy(xml);
    let start = text.find("<office:body").expect("a body");
    let end = text.rfind("</office:body>").expect("a body");
    (
        text[..start].as_bytes().to_vec(),
        text[end..].as_bytes().to_vec(),
    )
}

/// `text` with every ` xmlns:prefix="…"` taken out.
fn without_declarations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(" xmlns:") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let end = after
            .find('"')
            .and_then(|open| {
                after[open + 1..]
                    .find('"')
                    .map(|close| open + 1 + close + 1)
            })
            .unwrap_or(after.len());
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[test]
fn every_edit_to_somebody_elses_spreadsheet_saves_or_is_refused() {
    let mut refused = Vec::new();
    for path in documents() {
        let bytes = std::fs::read(&path).expect("reads");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let form = match bytes.starts_with(b"PK") {
            true => Form::Package,
            false => Form::Flat,
        };
        for (label, edit) in EDITS {
            let app = App::new();
            app.open_bytes(&name, &bytes).expect("opens");
            edit(&app);
            match app.save_bytes(form) {
                Ok(out) => {
                    App::new().open_bytes(&name, &out).unwrap_or_else(|e| {
                        panic!("{name}, {label}: saved a file that does not open: {e}")
                    });
                    if form == Form::Flat {
                        let (before, after) = (outside_body(&bytes), outside_body(&out));
                        // A new automatic style for what the edit wrote is the one thing allowed
                        // to appear outside the body, and only by being added — with the
                        // namespace declarations it needs on the root.
                        let added = |old: &[u8], new: &[u8]| {
                            let old = without_declarations(&String::from_utf8_lossy(old));
                            let new = without_declarations(&String::from_utf8_lossy(new));
                            old.lines().all(|line| new.contains(line))
                        };
                        assert!(
                            added(&before.0, &after.0),
                            "{name}, {label}: the head changed"
                        );
                        assert_eq!(before.1, after.1, "{name}, {label}: the tail changed");
                    }
                }
                Err(grind_sheet::Error::Odf(grind_core::Error::WouldLose(what))) => {
                    assert!(
                        !what.is_empty(),
                        "{name}, {label}: refused without saying why"
                    );
                    refused.push((name.clone(), label.to_string()));
                }
                Err(other) => panic!("{name}, {label}: neither saved nor refused: {other}"),
            }
        }
    }
    let expected: Vec<(String, String)> = REFUSED
        .iter()
        .map(|(n, l)| (n.to_string(), l.to_string()))
        .collect();
    assert_eq!(refused, expected, "the refusals moved — update REFUSED");
}
