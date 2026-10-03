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

/// Every element and attribute outside `office:body`, counted — the page layout, the named
/// styles, the fonts, the metadata, the settings.
fn envelope_vocabulary(flat: &[u8]) -> std::collections::BTreeMap<String, usize> {
    use grind_core::odf::envelope;
    let mut whole = envelope::document_vocabulary(flat).expect("reads");
    for (key, count) in envelope::body_vocabulary(flat).expect("reads") {
        if let Some(total) = whole.get_mut(&key) {
            *total = total.saturating_sub(count);
        }
    }
    whole.retain(|_, count| *count > 0);
    whole
}

/// **Saving into the other form is not a way round any of this.** A `.fodt` saved as an `.odt`
/// — or an `.ods` as a `.fods` — used to start from nothing and drop everything the model does
/// not own, with no error. Untouched, the document in the other form is every element and
/// attribute of the original, counted the same; after an edit, nothing outside the body has
/// gone, and an edit a same-form save refuses is refused here too.
#[test]
fn saving_into_the_other_form_carries_everything() {
    use grind_core::odf::{envelope, forms};
    let flat = |bytes: &[u8]| forms::convert(bytes, Form::Flat).expect("converts to flat");
    for path in documents() {
        let bytes = std::fs::read(&path).expect("reads");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let other = match Form::from_path(&path) {
            Form::Package => Form::Flat,
            _ => Form::Package,
        };
        let original = flat(&bytes);

        let app = App::new();
        app.open_bytes(&name, &bytes).expect("opens");
        let out = app
            .save_bytes(other)
            .unwrap_or_else(|e| panic!("{name}: an untouched save into the other form: {e}"));
        assert_eq!(
            envelope::document_vocabulary(&flat(&out)),
            envelope::document_vocabulary(&original),
            "{name}: not the same document in the other form"
        );

        let before = envelope_vocabulary(&original);
        for (label, edit) in EDITS {
            let app = App::new();
            app.open_bytes(&name, &bytes).expect("opens");
            edit(&app);
            match app.save_bytes(other) {
                Ok(out) => {
                    let after = envelope_vocabulary(&flat(&out));
                    for (key, count) in &before {
                        let left = after.get(key).copied().unwrap_or(0);
                        assert!(
                            left >= *count,
                            "{name}, {label}: {key} went from {count} to {left} in the other form"
                        );
                    }
                }
                Err(grind_sheet::Error::Odf(grind_core::Error::WouldLose(_))) => assert!(
                    REFUSED.iter().any(|(n, l)| *n == name && *l == *label),
                    "{name}, {label}: refused in the other form only"
                ),
                Err(other) => panic!("{name}, {label}: neither saved nor refused: {other}"),
            }
        }
    }
}

/// A cell style with what the model reads (a colour) and what it does not: a rotation, cell
/// protection, shrink-to-fit, a parent style and a conditional `style:map` — the shape
/// LibreOffice writes. B1 names a common style directly.
const STYLED: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
 <office:styles>
  <style:style style:name="Accent" style:family="table-cell"><style:text-properties fo:font-style="italic"/></style:style>
 </office:styles>
 <office:automatic-styles>
  <style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Accent">
   <style:table-cell-properties style:rotation-angle="45" style:cell-protect="none" style:shrink-to-fit="true"/>
   <style:text-properties fo:color="#ff0000" fo:font-weight="bold"/>
   <style:map style:condition="cell-content()&gt;3" style:apply-style-name="Accent" style:base-cell-address="Sheet1.A1"/>
  </style:style>
 </office:automatic-styles>
 <office:body>
  <office:spreadsheet>
   <table:table table:name="Sheet1">
    <table:table-row>
     <table:table-cell table:style-name="ce1" office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell>
     <table:table-cell table:style-name="Accent" office:value-type="float" office:value="2"><text:p>2</text:p></table:table-cell>
    </table:table-row>
   </table:table>
  </office:spreadsheet>
 </office:body>
</office:document>
"##;

/// The element of the automatic style a cell names, out of a flat document.
fn style_of(xml: &str, cell: usize) -> String {
    let tags: Vec<&str> = xml
        .match_indices("<table:table-cell ")
        .map(|(i, _)| &xml[i..])
        .collect();
    let tag = &tags[cell][..tags[cell].find('>').unwrap()];
    let name = tag
        .split("table:style-name=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let start = xml
        .find(&format!("<style:style style:name=\"{name}\""))
        .unwrap();
    let end = start + xml[start..].find("</style:style>").unwrap();
    xml[start..end].to_owned()
}

/// **Restyling a cell keeps everything about its style the model does not read** — the hole
/// `doc/not-doing.md` named: the new style used to be built from the model's reading of the
/// old one, so a rotation, a protection flag, a parent style or a conditional format went with
/// the first click on Bold. It is the old style's element now, renamed, with only what the
/// model owns rewritten.
#[test]
fn restyling_a_cell_keeps_what_the_model_does_not_read() {
    let restyle = |change: fn(&mut grind_sheet::style::CellStyle), col: u32| {
        let app = App::new();
        app.open_bytes("styled.fods", STYLED.as_bytes()).unwrap();
        let pos = Pos::new(0, col);
        let mut style = app.style_at(0, pos).unwrap().unwrap_or_default();
        change(&mut style);
        app.set_style(0, pos, pos, Some(style)).unwrap();
        String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap()
    };

    let out = restyle(|s| s.background = Some("#ffff00".into()), 0);
    let style = style_of(&out, 0);
    for kept in [
        "style:parent-style-name=\"Accent\"",
        "style:rotation-angle=\"45\"",
        "style:cell-protect=\"none\"",
        "style:shrink-to-fit=\"true\"",
        "fo:color=\"#ff0000\"",
        "fo:font-weight=\"bold\"",
        "<style:map style:condition=",
        "fo:background-color=\"#ffff00\"",
    ] {
        assert!(style.contains(kept), "{kept} is missing from\n{style}");
    }

    // Clearing what the model owns clears exactly that.
    let out = restyle(|s| s.font_weight = None, 0);
    let style = style_of(&out, 0);
    assert!(!style.contains("font-weight"), "{style}");
    assert!(style.contains("style:rotation-angle=\"45\""), "{style}");

    // A cell naming a common style directly keeps it as its new style's parent.
    let out = restyle(|s| s.font_weight = Some("bold".into()), 1);
    let style = style_of(&out, 1);
    assert!(
        style.contains("style:parent-style-name=\"Accent\""),
        "{style}"
    );
    assert!(style.contains("fo:font-weight=\"bold\""), "{style}");
}

/// **A byte-order mark does not shift the file under a save.** `quick-xml` measures positions
/// after one, so every range a reader recorded in such a file used to point three bytes early,
/// and the first splice cut `<office:spreadsheet>` short — refused by the writer's own check on
/// `sc/qa`'s `tdf117948_CollapseBeforeShape.ods`. Every vendored document, with one put in front.
#[test]
fn a_byte_order_mark_does_not_shift_a_save() {
    for path in documents()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "fods"))
    {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = [b"\xEF\xBB\xBF".as_slice(), &std::fs::read(&path).unwrap()].concat();
        let app = App::new();
        app.open_bytes(&name, &bytes).expect("opens");
        app.set_cell(0, Pos::new(0, 0), 7.0).unwrap();
        let out = app
            .save_bytes(Form::Flat)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = App::new();
        back.open_bytes(&name, &out).expect("reopens");
        assert_eq!(
            back.get(0, Pos::new(0, 0)).unwrap(),
            grind_sheet::CellValue::Number(7.0),
            "{name}"
        );
    }
}
