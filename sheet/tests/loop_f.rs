// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Loop F — the projection differential** (`doc/dsl.md` §8).
//!
//! For every document this build can read: project it, read the projection back, and assert
//! the two models are identical. That is the bijection, and the whole of layer 0 stands on it
//! — a projection that is not 1:1 with the model is a prettier `.csv`, and `doc/dsl.md` §1 is
//! the argument that the format is only worth having if it round-trips.
//!
//! It costs **no corpus**: every document loop A already reads is a case. So it runs over R7's
//! fourteen vendored documents, which never skip, *and* — as D3 — over loop A's whole 359,
//! which skip without a LibreOffice checkout. The corpus half is at **359/359, nothing
//! differing**, which is the bijection's evidence rather than a promise about it.
//!
//! What it compares is the **model**, never the bytes. `at`/`row` and `cell` are two spellings
//! of the same state and the reader takes both (§3.4), so byte equality would be asserting the
//! writer's taste rather than the format's promise. It compares in *both* directions for the
//! same reason loop C does: `document → projection → document` catches a writer that drops
//! something, and re-projecting the result catches a *reader* that drops it instead.
//!
//! ## The named gaps
//!
//! A document that will not round-trip is either a scope-line gap or a bug, and the difference
//! is what this file measures rather than what it assumes. The gaps as of D1:
//!
//! * **Charts.** `Sheet::charts` has no projection node yet (§3.8 puts them in for bijectivity
//!   rather than for authoring). They are excluded from the comparison *by name* below, and
//!   `charts_are_the_one_named_gap` fails the day one is projected — so the exclusion cannot
//!   outlive the gap.
//!
//! Nothing else is excused. A difference in a value, a formula, a style, a number format, a
//! track size, a filter or a named expression is a bug in the projection.

use std::path::{Path, PathBuf};

use grind_sheet::model::{Document, Pos, Sheet};
use grind_sheet::projection;

/// R7's fourteen, which is loop F's corpus on day one. Globbed rather than listed: unlike
/// `kb.rs`, this test is not the *requirement* that those files exist — it is a property that
/// has to hold for whatever is there, including a document added tomorrow.
fn corpus() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut out = Vec::new();
    for dir in ["kb", "samples"] {
        for entry in std::fs::read_dir(root.join(dir)).expect("the vendored corpus is there") {
            let path = entry.expect("a readable directory entry").path();
            if path.extension().is_some_and(|e| e == "fods" || e == "ods") {
                out.push(path);
            }
        }
    }
    out.sort();
    assert!(
        out.len() >= 14,
        "R7's corpus is fourteen documents, not {}",
        out.len()
    );
    out
}

#[test]
fn every_document_survives_being_projected_and_read_back() {
    let mut checked = 0;
    for path in corpus() {
        let bytes = std::fs::read(&path).expect("a readable document");
        let original = grind_sheet::read_bytes(&path.display().to_string(), &bytes)
            .unwrap_or_else(|e| panic!("{}: loop A reads this: {e}", path.display()));

        let text = projection::project(&original).into_text();
        let back = projection::read(&text).unwrap_or_else(|e| {
            panic!("{}: its own projection will not parse: {e}", path.display())
        });

        let differences = differences(&original, &back);
        assert!(
            differences.is_empty(),
            "{} does not round-trip through the projection:\n  {}\n--- the projection ---\n{text}",
            path.display(),
            differences.join("\n  ")
        );
        checked += 1;
    }
    assert!(checked >= 14, "loop F ran over {checked} documents");
}

/// **D4 — the projection is a `Form`.** The same property as above, one layer out: through
/// `write_bytes`/`read_bytes` rather than through `projection::` directly.
///
/// It is a separate test because it asserts a different thing. The one above says the grammar
/// is bijective; this one says the *crate's own door* knows about it — that a caller who never
/// heard of `grind_sheet::projection` and only ever asks for a form gets a projection out and
/// the same document back in. Every shell and every CLI verb is such a caller, which is how D4
/// is reached without any of them changing (rule 5).
#[test]
fn the_projection_is_a_form_like_the_other_two() {
    for path in corpus() {
        let bytes = std::fs::read(&path).expect("a readable document");
        let name = path.display().to_string();
        let original = grind_sheet::read_bytes(&name, &bytes).expect("loop A reads this");

        let written = grind_sheet::write_bytes(&original, grind_sheet::Form::Projection)
            .unwrap_or_else(|e| panic!("{name}: will not write as a projection: {e}"));
        // Sniffed from the bytes, not from the name — `book.fods` here holds KDL, and reading
        // has to notice. This is the half of D4 that was already true and is asserted anyway,
        // because it is what makes the other half safe.
        let back = grind_sheet::read_bytes(&name, &written)
            .unwrap_or_else(|e| panic!("{name}: its own projection will not read back: {e}"));

        let differences = differences(&original, &back);
        assert!(
            differences.is_empty(),
            "{name} does not survive Form::Projection:\n  {}",
            differences.join("\n  ")
        );
    }
}

/// **D3 — loop F at corpus scale.** The same property over LibreOffice's own `sc/qa` corpus,
/// which is loop A's, so it costs no corpus of its own.
///
/// A ratchet rather than a pass/fail: like loop B's `FLOOR`, the number below is what this
/// build achieves, it may be *raised* and never lowered, and the failures it tolerates are
/// listed by the scoreboard rather than hidden by it. Run
/// `cargo test -p grind-sheet --test loop_f -- --nocapture` to see them.
///
/// Skips with a notice when there is no checkout, exactly as loop A does — the corpus is not
/// vendorable and a test that cannot run must say so rather than pass quietly.
#[test]
fn the_corpus_projects() {
    const DEFAULT_CHECKOUT: &str = "/home/florian/code/github.com/LibreOffice/core";
    /// Documents that must survive the projection. **Raise it, never lower it.**
    ///
    /// It is at the whole corpus, which is a stronger result than the ratchet shape implies:
    /// there is nothing to triage, and the number is here so that the day something *is*
    /// triaged, the loop notices instead of absorbing it.
    const FLOOR: usize = 359;

    let root = PathBuf::from(
        std::env::var("GRIND_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CHECKOUT.to_owned()),
    )
    .join("sc/qa/unit/data");
    if !root.is_dir() {
        eprintln!(
            "skipping: no LibreOffice checkout at {DEFAULT_CHECKOUT}; \
             set GRIND_LO_CORPUS to its root to run loop F at corpus scale"
        );
        return;
    }

    let mut files = Vec::new();
    for dir in ["ods", "fods"] {
        collect(&root.join(dir), &mut files);
    }
    files.sort();

    let (mut matched, mut unreadable) = (0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        // A document loop A cannot read is loop A's problem, not this one's.
        let Ok(original) = grind_sheet::read_file(path) else {
            unreadable += 1;
            continue;
        };
        let text = projection::project(&original).into_text();
        match projection::read(&text) {
            Ok(back) => match differences(&original, &back) {
                diff if diff.is_empty() => matched += 1,
                diff => failures.push(format!("{}: {}", path.display(), diff[0])),
            },
            Err(e) => failures.push(format!("{}: will not parse back: {e}", path.display())),
        }
    }

    println!(
        "loop F: {matched}/{} projected and read back identically \
         ({unreadable} unreadable, {} differing)",
        files.len() - unreadable,
        failures.len()
    );
    for failure in failures.iter().take(40) {
        println!("  {failure}");
    }
    assert!(
        matched >= FLOOR,
        "loop F fell to {matched} from its floor of {FLOOR} — the ratchet only goes up"
    );
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ods" | "fods")
        ) {
            out.push(path);
        }
    }
}

/// The other direction, and the one that catches a *reader* that drops something: a projection
/// re-projected has to be byte-identical, because both texts come from the same writer over
/// models that this test has just proved equal.
#[test]
fn re_projecting_changes_nothing() {
    for path in corpus() {
        let bytes = std::fs::read(&path).expect("a readable document");
        let original = grind_sheet::read_bytes(&path.display().to_string(), &bytes)
            .expect("loop A reads this");
        let once = projection::project(&original).into_text();
        let twice = projection::project(&projection::read(&once).expect("parses")).into_text();
        assert_eq!(
            once,
            twice,
            "{} projects differently the second time",
            path.display()
        );
    }
}

/// The exclusion above, held to its own expiry date.
#[test]
fn charts_are_the_one_named_gap() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/samples/Sales Dashboard.fods");
    let bytes = std::fs::read(&path).expect("a readable document");
    let doc = grind_sheet::read_bytes("dashboard", &bytes).expect("reads");
    assert!(
        doc.sheets.iter().any(|s| !s.charts().is_empty()),
        "this sample is here because it has a chart in it"
    );
    let back = projection::read(&projection::project(&doc).into_text()).expect("parses");
    assert!(
        back.sheets.iter().all(|s| s.charts().is_empty()),
        "charts now survive the projection — drop the exclusion in `differences` rather than \
         leaving a standing excuse (doc/dsl.md §3.8)"
    );
}

/// A hand-written projection is read the way `doc/dsl.md` §3.4 promises, both spellings of a
/// cell and all.
#[test]
fn the_authoring_spellings_mean_what_the_document_says_they_do() {
    let source = r#"grind spreadsheet

// A comment, which the model has no room for and does not need one for.
sheet Sales {
    at A1 {
        row Region Q1 Q2
        row North  4200 4800
    }
    cell B3 "=SUM([.B1:.B2])"
    style B1:C1 bold=#true background=navy
    format B2:C2 currency decimals=2 symbol="EUR"
}
"#;
    let doc = projection::read(source).expect("parses");
    let sheet = &doc.sheets[0];
    assert_eq!(sheet.name, "Sales");
    assert_eq!(sheet.get(Pos::new(0, 0)), "Region".into());
    assert_eq!(sheet.get(Pos::new(1, 1)), 4200.0.into());
    assert_eq!(
        sheet.formula(Pos::new(2, 1)),
        Some("of:=SUM([.B1:.B2])"),
        "a leading `=` in the value position is the formula, spelled ODF's way on the way in"
    );
    assert_eq!(
        sheet
            .style(Pos::new(0, 2))
            .and_then(|s| s.background.clone()),
        Some("#001f3f".to_owned()),
        "a palette name is resolved to the colour a document stores"
    );
    assert_eq!(
        sheet.format(Pos::new(1, 2)),
        Some(&grind_sheet::numfmt::preset(
            grind_sheet::numfmt::Kind::Currency,
            2,
            false,
            "EUR"
        ))
    );
}

/// The span map, in both directions, over a real document (§6.2).
#[test]
fn the_span_map_finds_a_cell_and_a_cell_finds_its_line() {
    let doc = projection::read(
        "grind spreadsheet\nsheet Sales {\n  at A1 {\n    row one two\n  }\n  cell C9 \"=[.A1]\"\n}\n",
    )
    .expect("parses");
    let projected = projection::project(&doc);

    // A cell on a line of its own anchors the line.
    let span = projected.span_of("Sales.C9").expect("the cell is anchored");
    assert!(
        projected.text()[span.clone()].starts_with("cell C9"),
        "{:?}",
        &projected.text()[span.clone()]
    );
    assert_eq!(projected.address_at(span.start + 1), Some("Sales.C9"));

    // A cell inside a grid row anchors its own value, not the whole row.
    let span = projected.span_of("Sales.B1").expect("the cell is anchored");
    assert_eq!(&projected.text()[span.clone()], "two");
    assert_eq!(projected.address_at(span.start), Some("Sales.B1"));
    assert_eq!(
        projected.address_at(0),
        None,
        "the header is not part of any cell"
    );
}

// --- D5: R6 for the projection ---

/// A hand-written projection, with everything in it the model has no room for: a comment, blank
/// lines, a column of numbers lined up by eye, and a value spelled `4200.0` where the writer
/// would have said `4200`.
const BY_HAND: &str = r#"grind spreadsheet

// Q3 forecast. Nothing below this line is in the model.
sheet Sales {
    at A1 {
        row Region  Q1      Q2
        row North   4200.0  4800
        row South   3100    3300
    }

    cell B5 15400 formula="of:=SUM([.B2:.B3])"
}
"#;

/// Which lines two texts differ on, as `(before, after)` — what "one cell edited changes one
/// line" is measured with rather than asserted about.
fn changed_lines<'a>(before: &'a str, after: &'a str) -> Vec<(&'a str, &'a str)> {
    let (a, b): (Vec<_>, Vec<_>) = (before.lines().collect(), after.lines().collect());
    assert_eq!(a.len(), b.len(), "a splice does not change the line count");
    a.into_iter().zip(b).filter(|(x, y)| x != y).collect()
}

/// Read `BY_HAND`, apply `edit`, and save it back as a projection through the crate's own door.
fn edited(edit: grind_sheet::action::Action) -> String {
    let mut doc = projection::read(BY_HAND).expect("parses");
    doc.apply(edit).expect("applies");
    String::from_utf8(grind_sheet::write_bytes(&doc, grind_sheet::Form::Projection).expect("saves"))
        .expect("utf-8")
}

/// **D5's headline.** One cell edited is one line of `git diff`, and everything the model does
/// not carry is still there afterwards.
#[test]
fn one_cell_edited_changes_one_line() {
    let after = edited(grind_sheet::action::Action::SetCell {
        sheet: 0,
        pos: Pos::new(1, 1),
        value: grind_sheet::model::CellValue::Number(4300.0),
    });
    assert_eq!(
        changed_lines(BY_HAND, &after),
        [(
            "        row North   4200.0  4800",
            "        row North   4300  4800"
        )],
        "one line, and the alignment on either side of the value is the file's own"
    );
    assert!(
        after.contains("// Q3 forecast."),
        "a comment the model has no room for survives an edit (doc/dsl.md §3.1)"
    );
    assert_eq!(
        projection::read(&after)
            .expect("parses")
            .sheet(0)
            .unwrap()
            .get(Pos::new(1, 1)),
        grind_sheet::model::CellValue::Number(4300.0),
        "and the file still means what the model says"
    );
}

/// A `cell` node is replaced whole, because everything about that cell is on it — and the
/// indentation in front of it and the newline after it are still the file's.
#[test]
fn a_formula_edited_changes_its_own_line() {
    let after = edited(grind_sheet::action::Action::SetFormula {
        sheet: 0,
        pos: Pos::new(4, 1),
        formula: Some("of:=SUM([.B2:.B4])".to_owned()),
        value: grind_sheet::model::CellValue::Number(15400.0),
    });
    assert_eq!(
        changed_lines(BY_HAND, &after),
        [(
            "    cell B5 15400 formula=\"of:=SUM([.B2:.B3])\"",
            "    cell B5 15400 formula=\"of:=SUM([.B2:.B4])\""
        )]
    );
}

/// Saving a projection nobody edited gives back the bytes that were read — the strongest form of
/// "the writer never regenerates what nobody touched", and the one that covers hand alignment,
/// blank lines and comments in a single assertion.
#[test]
fn saving_an_untouched_projection_returns_the_file() {
    let doc = projection::read(BY_HAND).expect("parses");
    let out = grind_sheet::write_bytes(&doc, grind_sheet::Form::Projection).expect("saves");
    assert_eq!(String::from_utf8(out).expect("utf-8"), BY_HAND);
}

/// The same, over every document the corpus has: project one, read *that* back, save it
/// untouched, and the bytes have to be the projection they came from.
#[test]
fn every_projected_document_saves_back_unchanged() {
    for path in corpus() {
        let bytes = std::fs::read(&path).expect("a readable document");
        let name = path.display().to_string();
        let original = grind_sheet::read_bytes(&name, &bytes).expect("loop A reads this");
        let projected = projection::project(&original).into_text();
        let reopened = projection::read(&projected).expect("its own projection reads back");
        let saved = grind_sheet::write_bytes(&reopened, grind_sheet::Form::Projection)
            .expect("saves as a projection");
        assert_eq!(
            String::from_utf8(saved).expect("utf-8"),
            projected,
            "{name}: an untouched save is not the file that was read"
        );
    }
}

/// The **shape boundary** (`grind_core::projection::source`), and the case that makes it
/// necessary: a plain number in a grid row turns into a formula, which a grid cannot hold. The
/// splice refuses and the document regenerates — the comment is lost, the meaning is not.
#[test]
fn a_value_that_becomes_a_formula_regenerates_rather_than_lying() {
    let after = edited(grind_sheet::action::Action::SetFormula {
        sheet: 0,
        pos: Pos::new(1, 1),
        formula: Some("of:=4200*2".to_owned()),
        value: grind_sheet::model::CellValue::Number(8400.0),
    });
    assert!(
        !after.contains("// Q3 forecast."),
        "this is the honest cost of the boundary, and it is what makes the next line true"
    );
    let back = projection::read(&after).expect("parses");
    assert_eq!(
        back.sheet(0).unwrap().formula(Pos::new(1, 1)),
        Some("of:=4200*2"),
        "a spliced value would have read back as the *string* `=4200*2` in a grid"
    );
}

/// A style is a `style` node over a range, which no cell's site reaches. `Edits::only_values` is
/// already the flag that says so, and this is it doing the same job for a second form.
#[test]
fn a_style_edit_regenerates() {
    let after = edited(grind_sheet::action::Action::SetStyle {
        sheet: 0,
        pos: Pos::new(0, 0),
        style: Some(Box::new(grind_sheet::style::CellStyle {
            font_weight: Some("bold".to_owned()),
            ..Default::default()
        })),
    });
    assert!(!after.contains("// Q3 forecast."), "regenerated");
    assert!(after.contains("style A1 bold=#true"), "and it says so");
}

/// A document that came from a `.fods` has no projection to splice into, so asking for one
/// writes a fresh projection rather than nothing.
#[test]
fn a_document_read_from_odf_has_no_projection_to_splice() {
    let mut doc = Document::default();
    doc.apply(grind_sheet::action::Action::SetCell {
        sheet: 0,
        pos: Pos::new(0, 0),
        value: grind_sheet::model::CellValue::Number(1.0),
    })
    .expect("applies");
    let out = grind_sheet::write_bytes(&doc, grind_sheet::Form::Projection).expect("saves");
    let text = String::from_utf8(out).expect("utf-8");
    assert!(text.starts_with("grind spreadsheet\n"), "{text}");
    assert!(text.contains("row 1"), "{text}");
}

// --- comparing two models ---

/// Every way the two documents differ, in words. A `Vec` rather than a bool because the whole
/// value of this loop is the *first line of the failure* naming the cell.
fn differences(a: &Document, b: &Document) -> Vec<String> {
    let mut out = Vec::new();
    if a.null_date != b.null_date {
        out.push(format!("null-date {} vs {}", a.null_date, b.null_date));
    }
    if a.null_year != b.null_year {
        out.push(format!("null-year {} vs {}", a.null_year, b.null_year));
    }
    if a.names != b.names {
        out.push(format!("named expressions {:?} vs {:?}", a.names, b.names));
    }
    if a.sheets.len() != b.sheets.len() {
        out.push(format!("{} sheets vs {}", a.sheets.len(), b.sheets.len()));
        return out;
    }
    for (x, y) in a.sheets.iter().zip(&b.sheets) {
        sheet_differences(x, y, &mut out);
    }
    out
}

fn sheet_differences(a: &Sheet, b: &Sheet, out: &mut Vec<String>) {
    let name = &a.name;
    if a.name != b.name {
        out.push(format!("sheet name {:?} vs {:?}", a.name, b.name));
    }
    // The union of the two extents, so a cell that only one of them has is still compared.
    let rows = a.used_rows().max(b.used_rows());
    let cols = a.used_cols().max(b.used_cols());
    for row in 0..rows {
        for col in 0..cols {
            let pos = Pos::new(row, col);
            let at = || format!("{name}.{}", grind_sheet::a1::format(None, pos));
            if a.get(pos) != b.get(pos) {
                out.push(format!("{}: {:?} vs {:?}", at(), a.get(pos), b.get(pos)));
            }
            if a.formula(pos) != b.formula(pos) {
                out.push(format!(
                    "{}: formula {:?} vs {:?}",
                    at(),
                    a.formula(pos),
                    b.formula(pos)
                ));
            }
            if a.kind(pos) != b.kind(pos) {
                out.push(format!(
                    "{}: kind {:?} vs {:?}",
                    at(),
                    a.kind(pos),
                    b.kind(pos)
                ));
            }
            if a.format(pos) != b.format(pos) {
                out.push(format!(
                    "{}: format {:?} vs {:?}",
                    at(),
                    a.format(pos),
                    b.format(pos)
                ));
            }
            if a.style(pos) != b.style(pos) {
                out.push(format!(
                    "{}: style {:?} vs {:?}",
                    at(),
                    a.style(pos),
                    b.style(pos)
                ));
            }
        }
    }
    let widths = |s: &Sheet| {
        s.col_widths()
            .map(|(c, w)| (c, w.to_owned()))
            .collect::<Vec<_>>()
    };
    if widths(a) != widths(b) {
        out.push(format!(
            "{name}: column widths {:?} vs {:?}",
            widths(a),
            widths(b)
        ));
    }
    let heights = |s: &Sheet| {
        s.row_heights()
            .map(|(r, h)| (r, h.to_owned()))
            .collect::<Vec<_>>()
    };
    if heights(a) != heights(b) {
        out.push(format!(
            "{name}: row heights {:?} vs {:?}",
            heights(a),
            heights(b)
        ));
    }
    let hidden_cols = |s: &Sheet| s.hidden_cols().collect::<Vec<_>>();
    if hidden_cols(a) != hidden_cols(b) {
        out.push(format!("{name}: hidden columns"));
    }
    let hidden_rows = |s: &Sheet| s.manually_hidden_rows().collect::<Vec<_>>();
    if hidden_rows(a) != hidden_rows(b) {
        out.push(format!("{name}: hidden rows"));
    }
    if a.filter() != b.filter() {
        out.push(format!(
            "{name}: filter {:?} vs {:?}",
            a.filter(),
            b.filter()
        ));
    }
    // Charts: the one named gap (§3.8). Not compared, and `charts_are_the_one_named_gap`
    // above is what stops this staying true once they are projected.
}
