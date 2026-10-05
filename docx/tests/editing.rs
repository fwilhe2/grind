// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! An imported document is a document to **work in**, not a picture of one: opened the way a
//! shell opens it (`grind_docx::open`, then `App::open_bytes`), edited the way a person edits —
//! typing, Enter, Backspace, bold — and saved in both forms, every save has to go through and
//! read back as what was edited.
//!
//! This is what makes "emit ODF and read it back" more than a trick. The import's styles, list
//! styles and page are carried out of the file the import wrote, by the same envelope every
//! LibreOffice document is saved through, and that machinery *refuses* a save that would drop
//! something (`Error::WouldLose`). A refusal here is an import writing ODF its own suite cannot
//! edit — a bug in this filter, whatever the edit was.

use std::path::{Path, PathBuf};

use grind_text::{App, Caret, CharStyle, Form};

/// The inline elements the text model has no run for — what an edited paragraph holding one
/// is refused a save over.
const INLINE_GAP: [&str; 3] = ["text:note", "text:page-number", "text:page-count"];

/// Each edit, on a fresh copy of the document.
/// One edit, named.
type Edit = (&'static str, Box<dyn Fn(&App) -> grind_text::Result<()>>);

fn edits(app: &App) -> Vec<Edit> {
    let blocks = app.block_count();
    let mut out: Vec<Edit> = Vec::new();
    for index in [0, blocks / 2, blocks.saturating_sub(1)] {
        out.push((
            "type",
            Box::new(move |app: &App| {
                app.insert_text(
                    Caret {
                        block: index,
                        offset: 0,
                    },
                    "Edited ",
                )
            }),
        ));
        out.push((
            "enter",
            Box::new(move |app: &App| {
                let len = app
                    .input_text(index)
                    .map(|t| t.chars().count())
                    .unwrap_or(0);
                app.split_block(Caret {
                    block: index,
                    offset: len / 2,
                })
            }),
        ));
        out.push((
            "bold",
            Box::new(move |app: &App| {
                let len = app
                    .input_text(index)
                    .map(|t| t.chars().count())
                    .unwrap_or(0);
                let bold = CharStyle {
                    font_weight: Some("bold".into()),
                    ..CharStyle::default()
                };
                app.set_char_style(
                    Caret {
                        block: index,
                        offset: 0,
                    },
                    Caret {
                        block: index,
                        offset: len,
                    },
                    &bold,
                )
                .map(|_| ())
            }),
        ));
        if index > 0 {
            out.push((
                "backspace",
                Box::new(move |app: &App| app.join_block(index)),
            ));
        }
    }
    out
}

/// Every edit on `bytes`, saved both ways. Returns the failures, described.
fn try_edits(name: &str, bytes: &[u8]) -> Vec<String> {
    let Ok((odf, _)) = grind_docx::open(bytes) else {
        return Vec::new();
    };
    let nested = has_nested_table(&String::from_utf8_lossy(&odf));
    let probe = App::new();
    probe
        .open_bytes("import.fodt", &odf)
        .expect("an import opens");
    let mut failures = Vec::new();
    for (what, edit) in edits(&probe) {
        let app = App::new();
        app.open_bytes("import.fodt", &odf).unwrap();
        // An edit the model itself declines (joining into a table cell, say) is not a save.
        if edit(&app).is_err() {
            continue;
        }
        let blocks = app.block_count();
        for form in [Form::Flat, Form::Package] {
            match app.save_bytes(form) {
                Ok(saved) => match grind_text::read_bytes("saved", &saved) {
                    Ok(back) if back.blocks.len() == blocks => {}
                    Ok(back) => failures.push(format!(
                        "{name}: {what} then a {form:?} save reads back {} blocks, not {blocks}",
                        back.blocks.len()
                    )),
                    Err(e) => failures.push(format!("{name}: {what}, {form:?}: unreadable: {e}")),
                },
                // The one refusal this accepts, and it is the text model's rather than this
                // filter's: a paragraph holding a footnote or a page field is refused an edit
                // that would regenerate it, in every ODF document, because the model has no run
                // for either (`doc/text-core.md`'s named gap, `doc/docx-import.md` §"What it
                // costs"). Everything else is a bug.
                Err(grind_core::Error::WouldLose(lost))
                    if lost
                        .iter()
                        .all(|l| INLINE_GAP.iter().any(|g| l.contains(g))) => {}
                // The model's other named gap: a table inside a table cell is read as the outer
                // cell's paragraphs (`crate::model::Cell`), so an edit that regenerates the
                // outer table is refused rather than allowed to flatten the inner one.
                Err(grind_core::Error::WouldLose(_)) if nested => {}
                Err(e) => failures.push(format!("{name}: {what}, {form:?}: {e}")),
            }
        }
    }
    failures
}

/// Whether a `table:table` opens inside a `table:table-cell`.
fn has_nested_table(odf: &str) -> bool {
    let mut depth = 0i32;
    let mut at = 0;
    while let Some(i) = odf[at..].find('<') {
        let rest = &odf[at + i..];
        if rest.starts_with("<table:table-cell")
            && !rest[..rest.find('>').unwrap_or(0)].ends_with('/')
        {
            depth += 1;
        } else if rest.starts_with("</table:table-cell>") {
            depth -= 1;
        } else if rest.starts_with("<table:table ") && depth > 0 {
            return true;
        }
        at += i + 1;
    }
    false
}

fn docx_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "docx"))
        .collect();
    out.sort();
    out
}

#[test]
fn every_vendored_import_takes_every_edit_and_saves() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut failures = Vec::new();
    for path in docx_files(&dir) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        failures.extend(try_edits(&name, &std::fs::read(&path).unwrap()));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The same over Word's own documents, where there is a checkout to read them from.
#[test]
fn word_written_imports_take_every_edit_and_save() {
    let Ok(root) = std::env::var("GRIND_LO_CORPUS") else {
        eprintln!("skipping: set GRIND_LO_CORPUS to edit LibreOffice's Word corpus");
        return;
    };
    let dir = PathBuf::from(root).join("sw/qa/extras/ooxmlimport/data");
    let mut failures = Vec::new();
    let mut tried = 0;
    for path in docx_files(&dir) {
        let bytes = std::fs::read(&path).unwrap();
        if !grind_docx::sniff(&bytes) {
            continue;
        }
        tried += 1;
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        failures.extend(try_edits(&name, &bytes));
    }
    eprintln!("edited {tried} imports; {} failures", failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
