// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! R7's corpus — fourteen documents that are a *requirement* rather than an opportunity.
//!
//! Loop A's corpus is LibreOffice's, lives outside the repo and skips when it is not there.
//! These are named in `doc/plan.md`'s requirements, so they are vendored and this test never
//! skips: a requirement that skips is a preference. Two sets, and they pull in opposite
//! directions, which is the point of having both.
//!
//! **`data/kb/`** — hand-written against the spec
//! (<https://github.com/fwilhe2/open-document-knowledge-base>, MIT). Worth having *because*
//! they are not LibreOffice's output: an `office:version` of 1.3, a table with no
//! `table:table-column` at all, formulas with no cached value, `<table:table-cell/>`
//! self-closed with nothing in it. They hit the tolerant reader from the sparse side.
//!
//! **`data/samples/`** — LibreOffice's own output, normalised by `odslint-clean`
//! (<https://github.com/fwilhe2/office-in-git>). The dense side: three-sheet workbooks, 137
//! formulas in one table, charts, a pivot table, conditional formatting with `calcext:`
//! icon sets, named expressions, and several hundred elements and attributes this build has
//! no model for at all. Six of the eight upstream samples are here — `Personal Budget
//! Tracker` and `Inventory Manager` were dropped because they add **zero** elements or
//! attributes the other six do not already have, and a corpus file that widens nothing is
//! only slower.
//!
//! Three things are checked, and each is a normative requirement:
//!
//! * every one of them **reads** (R5), and survives write → read with its values, formulas
//!   and names intact;
//! * everything this build **writes** validates against the ODF 1.4 RELAX NG schema (R2);
//! * what it writes stays **small** (R3) — see `a_written_document_carries_no_boilerplate`.
//!
//! What is deliberately *not* checked is that a sample's charts and pivot tables come back.
//! They do not: the writer regenerates from the model, and the model has no chart. That is
//! R6, it is phase 8's whole job, and these six files are the evidence for it — which is
//! why `the_samples_measure_what_regenerating_still_loses` prints the size instead of
//! pretending.
//!
//! `jing` on `PATH` is the one thing that can skip, because a validator is not vendorable.

use std::path::{Path, PathBuf};
use std::process::Command;

use sheet_core::{Document, Form, Pos};

/// The hand-written half of R7. Listed rather than globbed: the requirement is these eight,
/// so a file going missing must fail rather than quietly shrink the run.
const KB: [&str; 8] = [
    "filter.fods",
    "fizzbuzz.fods",
    "formula.fods",
    "minimal.fods",
    "minimal-libreoffice.fods",
    "minimal-libreoffice-cleanup.fods",
    "minimal-with-styles.fods",
    "named-range.fods",
];

/// The LibreOffice-authored half. Spaces in the names are upstream's and kept, so provenance
/// stays checkable against the sample repository.
const SAMPLES: [&str; 6] = [
    "Quarterly Sales Report.fods",
    "Sales Dashboard.fods",
    "conditional-formatting.fods",
    "custom-colors.fods",
    "spreadsheet.fods",
    "table.fods",
];

/// Every R7 document, as (directory, file name).
fn required() -> impl Iterator<Item = (&'static str, &'static str)> {
    KB.iter()
        .map(|n| ("kb", *n))
        .chain(SAMPLES.iter().map(|n| ("samples", *n)))
}

fn data(dir: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(dir)
        .join(name)
}

/// Everything about a document that a round trip must not change, as one comparable value.
///
/// Deliberately coarser than loop C's cell-by-cell `differences`: formats and styles are
/// loop C's job against LibreOffice, and what R7 is here to pin is that a document does not
/// lose its *contents* on the way through.
fn digest(doc: &Document) -> Vec<String> {
    let mut out = vec![format!("names {:?}", doc.names)];
    for sheet in &doc.sheets {
        out.push(format!("sheet {:?}", sheet.name));
        for row in 0..sheet.used_rows() {
            for col in 0..sheet.used_cols() {
                let pos = Pos::new(row, col);
                let (value, formula) = (sheet.get(pos), sheet.formula(pos));
                if !matches!(value, sheet_core::CellValue::Empty) || formula.is_some() {
                    out.push(format!(
                        "r{row}c{col} {value:?} {formula:?} {:?}",
                        sheet.kind(pos)
                    ));
                }
            }
        }
    }
    out
}

/// A document read from a file, with the bytes it came from deliberately forgotten.
///
/// R6's splicing writer (`odf::source`) returns a document's own file back when nothing has
/// been edited, which is the right answer and makes a write→read test tautological. Dropping
/// `source` is how a test asks for the *regenerating* writer, and it is the path every
/// document built in memory takes anyway.
fn regenerating(dir: &str, name: &str) -> Document {
    let mut doc = sheet_core::read_file(&data(dir, name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    doc.source = None;
    doc
}

#[test]
fn every_required_document_reads_and_round_trips() {
    for (dir, name) in required() {
        let path = data(dir, name);
        let doc = regenerating(dir, name);
        assert_eq!(
            digest(&sheet_core::read_file(&path).unwrap()),
            digest(&doc),
            "{name}: forgetting the source changed the document"
        );

        // Both forms, because the requirement is about ODF and not about flat XML. A
        // package is the same content model in a zip (§7.3).
        for form in [Form::Flat, Form::Package] {
            let bytes = sheet_core::write_bytes(&doc, form)
                .unwrap_or_else(|e| panic!("{name} as {form:?}: {e}"));
            let back = sheet_core::read_bytes(name, &bytes)
                .unwrap_or_else(|e| panic!("{name} as {form:?}, reading back: {e}"));
            assert_eq!(
                digest(&doc),
                digest(&back),
                "{name} as {form:?}: contents changed on the way through"
            );
        }
    }
}

/// `fizzbuzz.fods` is eighteen formulas and not one cached value, which is legal and which
/// LibreOffice renders blank until it recalculates. Pinned separately because it is the one
/// file whose *point* is that reading it correctly produces an empty grid.
///
/// It is also the reason `ROW` exists here at all: the formula is
/// `IF(MOD(ROW();15)=0;"fizzbuzz";…)`, §2.3.2 E) admits `ROWS` and `COLUMNS` and not the
/// singulars, and this file recalculating to eighteen `#NAME?` was the evidence that moved
/// `ROW` and `COLUMN` in (`doc/small-group.md`, *Beyond the Small Group*). So the assertion
/// is fizzbuzz itself — a scope decision that does not produce the right answer was not
/// worth making.
#[test]
fn a_document_of_formulas_with_no_cached_values_recalculates() {
    let app = sheet_core::App::new();
    app.open_file(&data("kb", "fizzbuzz.fods")).unwrap();
    assert_eq!(app.formula_count(0).unwrap(), 18);
    assert_eq!(app.used_extent(0).unwrap(), (0, 0), "no cached values to read");

    app.recalc().unwrap();
    assert_eq!(app.used_extent(0).unwrap(), (18, 1));
    let played: Vec<String> = (0..18)
        .map(|row| match app.get(0, Pos::new(row, 0)).unwrap() {
            sheet_core::CellValue::Text(t) => t,
            sheet_core::CellValue::Number(n) => format!("{n}"),
            other => panic!("row {} is {other:?}", row + 1),
        })
        .collect();
    let want = [
        "1", "2", "fizz", "4", "buzz", "fizz", "7", "8", "fizz", "buzz", "11", "fizz", "13",
        "14", "fizzbuzz", "16", "17", "fizz",
    ];
    assert_eq!(played, want);
}

// --- schema validity ---------------------------------------------------------------------

/// `jing -i` rather than `xmllint --relaxng`: the ODF schema uses RELAX NG constructs
/// xmllint does not implement, and `-i` turns off the ID/IDREF checking that makes jing
/// reject the schema itself over `draw:control` (an ODF quirk, not ours).
fn jing(path: &Path) -> Option<Result<(), String>> {
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("../doc/OpenDocument-v1.4-schema.rng");
    let out = Command::new("jing").arg("-i").arg(schema).arg(path).output().ok()?;
    Some(if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

#[test]
fn everything_we_write_is_valid_odf() {
    let dir = std::env::temp_dir().join(format!("sheet-kb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut failures = Vec::new();
    for (from, name) in required() {
        let doc = regenerating(from, name);
        let path = dir.join(name);
        sheet_core::write_file(&doc, &path).unwrap();

        match jing(&path) {
            None => {
                eprintln!("skipping: no `jing` on PATH; schema validity unchecked");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            Some(Ok(())) => {}
            Some(Err(report)) => failures.push(format!("{name}:\n{report}")),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        failures.is_empty(),
        "documents we wrote are not valid ODF 1.4:\n{}",
        failures.join("\n")
    );
}

// --- R3 and R6, which are the same measurement read two ways -----------------------------

/// R3: a document carries only the boilerplate it uses.
///
/// The number that makes this concrete is the *preamble* — everything before the first
/// `<table:table-cell`. LibreOffice's own `minimal-libreoffice.fods` spends 200-odd lines
/// on namespace declarations, `office:meta`, `office:settings` and a style catalogue before
/// the first cell; a document with no styles must spend almost none, and one with styles
/// must spend only what its styles need. A ceiling rather than an exact count, because the
/// point is the order of magnitude and an exact count would fail on every legitimate edit.
#[test]
fn a_written_document_carries_no_boilerplate() {
    for (dir, name, ceiling) in [
        // Twelve cells, no styles — but a date, a time, a currency and a percentage, so the
        // preamble is the four pooled number styles those need and nothing else. 13 today.
        ("kb", "minimal.fods", 20),
        // LibreOffice's own 482 lines, written back: three cells and no formats, so the
        // preamble is the root element and the two elements above the first table. 7 today.
        ("kb", "minimal-libreoffice.fods", 12),
        // Styled, so the preamble is the pooled automatic styles and number formats —
        // proportional to the distinct formats used, never to the size of the file. 24 today
        // for both, though `table.fods` is three times the cells.
        ("kb", "minimal-with-styles.fods", 40),
        ("samples", "table.fods", 40),
    ] {
        let doc = regenerating(dir, name);
        let xml = String::from_utf8(sheet_core::write_bytes(&doc, Form::Flat).unwrap()).unwrap();
        let preamble = xml
            .split_once("<table:table-cell")
            .unwrap_or_else(|| panic!("{name}: wrote no cells"))
            .0
            .lines()
            .count();
        assert!(
            preamble <= ceiling,
            "{name}: {preamble} lines before the first cell, ceiling is {ceiling}"
        );
    }
}

/// What the *regenerating* writer still drops, printed rather than asserted.
///
/// These six are LibreOffice's output: charts, a pivot table, conditional formatting,
/// `office:settings`, a font catalogue. Regenerating from the model loses all of it, and
/// that path is still what a document built in memory, or converted between forms, or edited
/// in a way splicing cannot express, takes. So the number stays on the record — R6 is
/// satisfied by *not going through here*, not by this shrinking.
///
/// It asserts only what must not regress: the output is smaller, never larger. A regenerated
/// document growing past LibreOffice's own would mean R3 broke.
#[test]
fn the_samples_measure_what_regenerating_still_loses() {
    eprintln!("what regenerating from the model drops (the path R6 avoids):");
    for name in SAMPLES {
        let before = std::fs::metadata(data("samples", name)).unwrap().len();
        let doc = regenerating("samples", name);
        let after = sheet_core::write_bytes(&doc, Form::Flat).unwrap().len() as u64;
        eprintln!(
            "  {name:32} {before:>7} -> {after:>7} bytes  ({}% kept)",
            after * 100 / before
        );
        assert!(
            after < before,
            "{name}: regenerating grew the document, {before} -> {after}"
        );
    }
}

// --- R6: writing changes as little of the XML as it can ----------------------------------

/// Setting one number changes **one element**, in every R7 document that has a cell to set.
///
/// The requirement in its own words: editing one number must not produce the hundred-line
/// diff LibreOffice's own save does, and a flat file must stay easy to `git diff`. So the
/// assertion is a diff, counted in changed lines against the original file — and the ceiling
/// is low enough that regenerating (which changes every line of all fourteen) cannot pass by
/// accident.
///
/// The ceiling is on *lines*, not on elements, because that is what a reader of a diff sees
/// — but it is a **constant**, which is the property that matters: one element goes out
/// however large the document is. Removals get more headroom than additions because a file
/// is free to spell one cell across several lines (`Quarterly Sales Report` puts every
/// attribute on its own), while the replacement is always a single line. Regenerating
/// changes every line of all fourteen, so nothing here can pass by falling back.
#[test]
fn setting_one_number_changes_one_element() {
    for (dir, name) in required() {
        let before = std::fs::read_to_string(data(dir, name)).unwrap();
        let app = sheet_core::App::new();
        app.open_file(&data(dir, name)).unwrap();

        // The first cell that holds a value. Not a fixed address: A1 is inside a repeated
        // *row* in `conditional-formatting.fods` and absent altogether from `fizzbuzz.fods`,
        // and both of those fall back by design. A cell with a value is one the file spelled.
        let at = first_value(&sheet_core::read_file(&data(dir, name)).unwrap());
        app.set_cell(0, at, 42.0).unwrap();
        let after = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();

        let (removed, added) = changed_lines(&before, &after);
        assert!(
            removed <= 10 && added <= 2,
            "{name}: setting one cell changed {removed} lines and added {added}; \
             the document is {} lines",
            before.lines().count()
        );
        assert!(
            after.contains("office:value=\"42\""),
            "{name}: the new value is not in the output"
        );

        // And it still reads back as the document it now is.
        let back = sheet_core::read_bytes(name, after.as_bytes()).unwrap();
        assert_eq!(
            back.sheet(0).unwrap().get(at),
            sheet_core::CellValue::Number(42.0),
            "{name}: the spliced cell did not read back"
        );
    }
}

/// Writing into a `table:number-columns-repeated` run splits **that element** and no more.
///
/// The case R6 would otherwise be true only in name for: LibreOffice writes a row of empty
/// cells as one element, so if a repeated element could not be split, every "put a number in
/// an empty cell" — the ordinary edit — would regenerate the file. `custom-colors.fods` row
/// 3 is `<table:table-cell table:number-columns-repeated="5"/>` and nothing else, which
/// makes the before and after readable in one line each.
#[test]
fn a_value_written_into_a_repeated_run_splits_only_that_element() {
    let name = "custom-colors.fods";
    let before = std::fs::read_to_string(data("samples", name)).unwrap();
    assert!(
        before.contains("<table:table-cell table:number-columns-repeated=\"5\"/>"),
        "the fixture no longer has the run this test is about"
    );

    let app = sheet_core::App::new();
    app.open_file(&data("samples", name)).unwrap();
    app.set_cell(0, Pos::new(2, 1), 99.0).unwrap(); // B3, the middle of a run of five
    let after = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();

    assert!(
        after.contains(
            "<table:table-cell/>\
             <table:table-cell office:value-type=\"float\" office:value=\"99\"/>\
             <table:table-cell table:number-columns-repeated=\"3\"/>"
        ),
        "the run did not split into before / cell / after"
    );
    let (removed, added) = changed_lines(&before, &after);
    assert_eq!((removed, added), (1, 1), "one line out, one line in");

    // The run re-forms when the value goes away again, so an edit and its undo leave the
    // document as it was rather than as five separate elements.
    app.undo();
    let undone = String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap();
    assert_eq!(undone, before, "undo did not restore the file");
}

/// Saving a document nobody edited returns its bytes untouched — the limit case of R6, and
/// the one a `.fods` in version control cares about most: opening a file to look at it must
/// not show up as a commit.
#[test]
fn saving_an_unedited_document_changes_nothing_at_all() {
    for (dir, name) in required() {
        let before = std::fs::read(data(dir, name)).unwrap();
        let doc = sheet_core::read_file(&data(dir, name)).unwrap();
        assert_eq!(
            sheet_core::write_bytes(&doc, Form::Flat).unwrap(),
            before,
            "{name}: an untouched document did not come back byte for byte"
        );
    }
}

/// Splicing refuses rather than guesses, and the refusal is the whole writer falling back.
///
/// Named cases rather than an inferred one: a fallback that fires silently would make R6
/// untestable, and each of these is a boundary `odf::source` documents.
#[test]
fn what_cannot_be_spliced_regenerates() {
    let doc = |edit: &dyn Fn(&sheet_core::App)| {
        let app = sheet_core::App::new();
        app.open_file(&data("kb", "minimal-libreoffice.fods")).unwrap();
        edit(&app);
        String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap()
    };
    let regenerated = |xml: &str| !xml.contains("office:settings");

    // A cell in a row the file does not spell at all.
    assert!(regenerated(&doc(&|app| {
        app.set_cell(0, Pos::new(500, 0), 1.0).unwrap();
    })));
    // A number format, which needs a `style:style` the source file does not contain.
    assert!(regenerated(&doc(&|app| {
        app.set_format(0, Pos::new(0, 0), Pos::new(0, 0), Some(sheet_core::numfmt::preset(
            sheet_core::numfmt::Kind::Percentage,
            1,
            false,
            "",
        )))
        .unwrap();
    })));
    // And the case that does splice, so the three above are not passing for a shared reason.
    assert!(!regenerated(&doc(&|app| {
        app.set_cell(0, Pos::new(0, 0), 7.0).unwrap();
    })));
}

/// The first cell of sheet 0 holding a value or a formula, in row-major order.
/// `fizzbuzz.fods` is why formulas count: it has eighteen and not one cached value.
fn first_value(doc: &Document) -> Pos {
    let sheet = doc.sheet(0).expect("a sheet");
    for row in 0..sheet.used_rows() {
        for col in 0..sheet.used_cols() {
            let pos = Pos::new(row, col);
            if !sheet.get(pos).is_empty() || sheet.formula(pos).is_some() {
                return pos;
            }
        }
    }
    // `used_rows` is the extent of *values*, so a document of formulas with no cached
    // values — `fizzbuzz.fods` — has none at all to walk.
    sheet.formulas().next().map(|(pos, _)| pos).expect("a cell")
}

/// Lines removed and added between two texts, by the shortest honest route: a line that
/// appears in both, the same number of times, did not change.
fn changed_lines(before: &str, after: &str) -> (usize, usize) {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for line in before.lines() {
        *counts.entry(line).or_default() += 1;
    }
    for line in after.lines() {
        *counts.entry(line).or_default() -= 1;
    }
    (
        counts.values().filter(|n| **n > 0).map(|n| *n as usize).sum(),
        counts.values().filter(|n| **n < 0).map(|n| -*n as usize).sum(),
    )
}

/// `style::PALETTE` is held against a document, not against its own comment.
///
/// `custom-colors.fods` is the clrs.cc palette as LibreOffice wrote it, with each colour's
/// **name** in the cell it fills — so the table a shell offers by default can be checked
/// against a real `fo:background-color` rather than trusted. A colour that drifts here is a
/// document whose swatch labelled "navy" is not navy.
#[test]
fn the_default_palette_is_the_one_the_sample_document_uses() {
    let app = sheet_core::App::new();
    app.open_file(&data("samples", "custom-colors.fods")).unwrap();
    let (rows, cols) = app.used_extent(0).unwrap();

    let mut found = 0;
    for row in 0..rows {
        for col in 0..cols {
            let pos = Pos::new(row, col);
            let sheet_core::CellValue::Text(label) = app.get(0, pos).unwrap() else {
                continue;
            };
            let Some(expected) = sheet_core::style::palette(&label) else {
                continue; // the caption row, which names no colour
            };
            let background = app
                .style_at(0, pos)
                .unwrap()
                .and_then(|style| style.background);
            assert_eq!(
                background.as_deref(),
                Some(expected),
                "{label} at {pos:?} is not the palette's {expected}"
            );
            found += 1;
        }
    }
    assert_eq!(
        found,
        sheet_core::style::PALETTE.len(),
        "the fixture stopped covering the whole palette"
    );
}
