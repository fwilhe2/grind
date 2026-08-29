// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `doc/view-modes.md` §6 — and the first thing to say is what is *not* here.
//!
//! **Loop C cannot check this feature.** Every other feature in this project is verified by
//! round-tripping it through LibreOffice, and this one writes nothing for LibreOffice to
//! see. That is not a gap; it is the claim. So the checks are chosen to match it:
//!
//! * **The writes-nothing test** — open every R7 document, ask for every overlay, read the
//!   whole sheet through them, save, and assert the bytes are byte-identical to the input.
//!   The feature's entire promise is that it changes nothing, and this is that promise,
//!   mechanically. It is the headline check and it never skips.
//! * **Totality and disjointness** — every cell gets exactly one role, over the same corpus.
//! * **Roles agree with the formulas** — a cell classified *computed, local* has a formula
//!   whose every reference is local, and one classified *constant, unnamed* really is read
//!   by something. Checked against the index rather than against a second opinion.
//!
//! Loops A and B check the third of those over hundreds of documents at no corpus cost;
//! this file is the half that must run everywhere, so it uses R7's vendored documents, which
//! never skip.

use std::path::{Path, PathBuf};

use grind_sheet::formula::display::from_display;
use grind_sheet::formula::eval::Address;
use grind_sheet::view::{Analysis, CellRole, Names, Overlays};
use grind_sheet::{App, Form, Pos};

/// Every R7 document, as (directory, file name) — the same list `kb.rs` requires, and for
/// the same reason it is listed rather than globbed.
fn required() -> Vec<PathBuf> {
    let kb = [
        "filter.fods",
        "fizzbuzz.fods",
        "formula.fods",
        "minimal.fods",
        "minimal-libreoffice.fods",
        "minimal-libreoffice-cleanup.fods",
        "minimal-with-styles.fods",
        "named-range.fods",
    ];
    let samples = [
        "Quarterly Sales Report.fods",
        "Sales Dashboard.fods",
        "conditional-formatting.fods",
        "custom-colors.fods",
        "spreadsheet.fods",
        "table.fods",
    ];
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    kb.iter()
        .map(|n| data.join("kb").join(n))
        .chain(samples.iter().map(|n| data.join("samples").join(n)))
        .collect()
}

/// Read every cell of every sheet through the overlays, exactly as a shell scrolling the
/// whole document would.
fn read_everything(app: &App, overlays: Overlays) {
    for sheet in 0..app.sheet_count() {
        let (rows, cols) = app.used_extent(sheet).unwrap();
        let view = app
            .get_viewport_with(sheet, 0..rows, 0..cols, overlays)
            .unwrap();
        for row in 0..rows {
            for col in 0..cols {
                let role = view.role(row, col);
                assert_eq!(role.is_some(), overlays.roles);
                let _ = view.name_at(row, col);
            }
        }
        let _ = view.names();
    }
}

#[test]
fn asking_for_every_overlay_changes_not_one_byte_of_the_document() {
    // §1 and §6's headline. A document a shell opens read-only must come back out as it went
    // in — if looking at a file in role mode changed it, the mode would be a write.
    for path in required() {
        let before = std::fs::read(&path).unwrap();
        let app = App::new();
        app.open_bytes(&path.display().to_string(), &before)
            .unwrap();
        // The unmodified save is the baseline: R6's splicing writer returns the original
        // bytes for a document nothing changed, and that is what the overlays must not
        // disturb.
        let baseline = app.save_bytes(Form::Flat).unwrap();
        read_everything(&app, Overlays::ALL);
        read_everything(&app, Overlays::ROLES);
        read_everything(&app, Overlays::NAMES);
        let after = app.save_bytes(Form::Flat).unwrap();
        assert_eq!(
            baseline.len(),
            after.len(),
            "{} changed size after being read",
            path.display()
        );
        assert!(
            baseline == after,
            "{} changed after being read through the overlays",
            path.display()
        );
        assert_eq!(before, after, "{} is not returned verbatim", path.display());
    }
}

#[test]
fn every_cell_of_every_r7_document_gets_exactly_one_role() {
    // Totality. `role` is one function returning one value, so disjointness is structural —
    // what this asserts is that it is *total*: no cell of any real document falls through.
    let mut seen = std::collections::BTreeSet::new();
    for path in required() {
        let app = App::new();
        app.open_file(&path).unwrap();
        for sheet in 0..app.sheet_count() {
            let (rows, cols) = app.used_extent(sheet).unwrap();
            let view = app
                .get_viewport_with(sheet, 0..rows, 0..cols, Overlays::ROLES)
                .unwrap();
            for row in 0..rows {
                for col in 0..cols {
                    let role = view
                        .role(row, col)
                        .unwrap_or_else(|| panic!("{}: no role at r{row}c{col}", path.display()));
                    assert!(CellRole::ALL.contains(&role));
                    seen.insert(role);
                }
            }
        }
    }
    // The corpus is not a set of unit tests, so this is a floor rather than an equality: if
    // it stops covering the ordinary roles, the check above has stopped meaning anything.
    for role in [
        CellRole::Empty,
        CellRole::Label,
        CellRole::ComputedLocal,
        CellRole::InputUnnamed,
    ] {
        assert!(seen.contains(&role), "no {} in R7's corpus", role.name());
    }
}

#[test]
fn a_role_says_what_the_formulas_and_the_index_say() {
    // The agreement check: a role is not a fourth opinion about a document. Everything here
    // is re-derived from the reference index and the formulas, and must match.
    for path in required() {
        let doc = grind_sheet::read_file(&path).unwrap();
        let analysis = Analysis::build(&doc);
        for (index, sheet) in doc.sheets.iter().enumerate() {
            for row in 0..sheet.used_rows() {
                for col in 0..sheet.used_cols() {
                    let at = Address::new(index, Pos::new(row, col));
                    let role = analysis.role(sheet, at);
                    let where_ = format!("{} {index}:r{row}c{col}", path.display());
                    match role {
                        CellRole::ComputedLocal => {
                            assert!(sheet.formula(at.pos).is_some(), "{where_}");
                            assert!(
                                analysis.refs().reads(at).iter().all(|a| a.sheet == index),
                                "{where_} is computed-local but reads another sheet"
                            );
                        }
                        CellRole::ComputedCrossSheet => {
                            assert!(sheet.formula(at.pos).is_some(), "{where_}");
                            assert!(
                                analysis.refs().reads(at).iter().any(|a| a.sheet != index),
                                "{where_} is computed-cross-sheet but reads only its own"
                            );
                        }
                        CellRole::Error | CellRole::Stale => {
                            assert!(sheet.formula(at.pos).is_some(), "{where_}");
                        }
                        CellRole::ConstantUnnamed => {
                            // §4.2's definition, mechanically: a literal, referenced, unnamed.
                            assert!(sheet.formula(at.pos).is_none(), "{where_}");
                            assert!(analysis.refs().singled_out_by(at) >= 2, "{where_}");
                            assert!(analysis.name_at(at).is_none(), "{where_}");
                        }
                        CellRole::InputNamed => {
                            assert!(analysis.name_at(at).is_some(), "{where_}");
                        }
                        CellRole::InputUnnamed | CellRole::Label => {
                            assert!(sheet.formula(at.pos).is_none(), "{where_}");
                            assert!(analysis.name_at(at).is_none(), "{where_}");
                        }
                        CellRole::Empty => {
                            assert!(sheet.get(at.pos).is_empty(), "{where_}");
                            assert!(sheet.formula(at.pos).is_none(), "{where_}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn a_formula_read_through_its_names_reads_back() {
    // §3.3's check, on the corpus that never skips. Loop B runs the same assertion over
    // 75845 formulas where a LibreOffice checkout exists; this is the half that runs
    // everywhere, and it is the one that gates a `cargo test` with no corpus.
    for path in required() {
        let doc = grind_sheet::read_file(&path).unwrap();
        let names = Names::build(&doc);
        for (index, sheet) in doc.sheets.iter().enumerate() {
            for (pos, formula) in sheet.formulas() {
                let Ok(expr) = grind_sheet::formula::parse::parse(formula) else {
                    continue; // loop B owns parsing.
                };
                let at = Address::new(index, pos);
                let shown = names.display(&doc, at, formula).expect("it just parsed");
                let want = format!("={}", names.substitute(&doc, at, &expr));
                assert_eq!(
                    from_display(&shown).as_deref(),
                    Ok(want.as_str()),
                    "{} {index}:{pos:?}: {formula} -> {shown}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn the_named_range_document_anchors_its_name() {
    // R7 has a document whose whole subject is a named expression, so the name overlay has a
    // corpus case rather than only the unit tests' synthetic ones.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/kb/named-range.fods");
    let doc = grind_sheet::read_file(&path).unwrap();
    let analysis = Analysis::build(&doc);
    assert!(
        !doc.names.is_empty(),
        "named-range.fods declares no names any more"
    );
    // Every anchor points inside a sheet that exists, and names the cell it says it names.
    for anchor in analysis.anchors() {
        assert!(anchor.sheet < doc.sheets.len());
        assert!(!anchor.rows.is_empty() && !anchor.cols.is_empty());
        let at = Address::new(anchor.sheet, Pos::new(anchor.rows.start, anchor.cols.start));
        assert_eq!(analysis.name_at(at), Some(anchor.name.as_str()));
    }
}
