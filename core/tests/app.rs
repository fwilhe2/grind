// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The public API, exercised only from outside — which is also how we find out whether it
//! is usable from outside.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sheet_core::{App, CellValue, Entered, Observer, Pos, Recalc, RecalcMode};

fn p(row: u32, col: u32) -> Pos {
    Pos::new(row, col)
}

#[test]
fn a_new_app_has_one_empty_sheet() {
    let app = App::new();
    assert_eq!(app.sheet_count(), 1);
    assert_eq!(app.sheet_name(0).unwrap(), "Sheet1");
    assert_eq!(app.used_extent(0).unwrap(), (0, 0));
    assert_eq!(app.get(0, p(0, 0)).unwrap(), CellValue::Empty);
    assert!(!app.can_undo());
    assert!(!app.can_redo());
}

#[test]
fn a_bad_sheet_index_is_an_error_not_a_panic() {
    let app = App::new();
    assert!(app.get(9, p(0, 0)).is_err());
    assert!(app.set_cell(9, p(0, 0), 1.0).is_err());
    assert!(app.get_viewport(9, 0..1, 0..1).is_err());
    assert!(app.sheet_name(9).is_err());
    assert!(app.used_extent(9).is_err());
    // A failed edit must not leave anything on the undo stack.
    assert!(!app.can_undo());
}

#[test]
fn viewport_reads_past_the_end_are_empty_not_errors() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 1.0).unwrap();

    let v = app.get_viewport(0, 100..103, 50..52).unwrap();
    assert_eq!(v.rows, 100..103);
    assert_eq!(v.row(100).unwrap(), &[CellValue::Empty, CellValue::Empty]);
    assert_eq!(v.get(101, 51), Some(&CellValue::Empty));
}

#[test]
fn viewport_addresses_are_absolute_and_bounded() {
    let app = App::new();
    app.set_cell(0, p(5, 2), "x").unwrap();

    let v = app.get_viewport(0, 4..7, 1..4).unwrap();
    assert_eq!(v.get(5, 2), Some(&CellValue::Text("x".into())));
    assert_eq!(v.get(4, 1), Some(&CellValue::Empty));
    // Outside the requested rectangle: None, not a wrapped read.
    assert_eq!(v.get(0, 0), None);
    assert_eq!(v.get(7, 2), None);
    assert_eq!(v.get(5, 4), None);
    assert!(v.row(9).is_none());

    let rows: Vec<_> = (4..7).map(|r| v.row(r).unwrap().len()).collect();
    assert_eq!(rows, vec![3, 3, 3]);
}

#[test]
fn an_inverted_range_yields_an_empty_viewport() {
    let app = App::new();
    // Built from variables: a literal `10..3` is a clippy error, but a shell computing a
    // scroll offset can easily produce one at runtime, which is the case under test.
    let (start, end) = (10u32, 3u32);
    let v = app.get_viewport(0, start..end, 0..2).unwrap();
    assert_eq!(v.rows, 10..10);
    assert!(v.row(10).is_none());
}

#[test]
fn undo_and_redo_walk_the_history_both_ways() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 1.0).unwrap();
    app.set_cell(0, p(0, 0), 2.0).unwrap();

    assert_eq!(app.get(0, p(0, 0)).unwrap(), CellValue::Number(2.0));
    assert!(app.undo());
    assert_eq!(app.get(0, p(0, 0)).unwrap(), CellValue::Number(1.0));
    assert!(app.undo());
    assert_eq!(app.get(0, p(0, 0)).unwrap(), CellValue::Empty);

    assert!(!app.undo(), "nothing left to undo");
    assert!(!app.can_undo());

    assert!(app.redo());
    assert_eq!(app.get(0, p(0, 0)).unwrap(), CellValue::Number(1.0));
    assert!(app.redo());
    assert_eq!(app.get(0, p(0, 0)).unwrap(), CellValue::Number(2.0));
    assert!(!app.redo(), "nothing left to redo");
}

#[test]
fn a_new_edit_discards_the_redo_branch() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 1.0).unwrap();
    app.undo();
    assert!(app.can_redo());

    app.set_cell(0, p(0, 0), 9.0).unwrap();
    assert!(
        !app.can_redo(),
        "editing after undo must drop the redo branch"
    );
}

/// The exit criterion from doc/plan.md: undo/redo round-trips an arbitrary action
/// sequence. Deterministic, so a failure is reproducible.
#[test]
fn undo_redo_round_trips_an_arbitrary_sequence() {
    const STEPS: usize = 300;
    let app = App::new();

    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    // Snapshot after every edit, so we can check every intermediate state on the way back
    // rather than only the endpoints.
    let snapshot = |app: &App| -> Vec<CellValue> {
        let v = app.get_viewport(0, 0..8, 0..4).unwrap();
        (0..8)
            .flat_map(|r| v.row(r).unwrap().to_vec())
            .collect::<Vec<_>>()
    };

    let mut history = vec![snapshot(&app)];
    for _ in 0..STEPS {
        let pos = p((next() % 8) as u32, (next() % 4) as u32);
        let value = match next() % 4 {
            0 => CellValue::Empty,
            1 => CellValue::Number((next() % 50) as f64),
            2 => CellValue::Text(format!("s{}", next() % 5)),
            _ => CellValue::Bool(next() % 2 == 0),
        };
        app.set_cell(0, pos, value).unwrap();
        history.push(snapshot(&app));
    }

    // Undo all the way to the start, checking each state matches what it was.
    for want in history.iter().rev().skip(1) {
        assert!(app.undo());
        assert_eq!(&snapshot(&app), want, "state diverged while undoing");
    }
    assert!(!app.can_undo());
    assert_eq!(app.used_extent(0).unwrap(), (0, 0), "back to empty");

    // Redo all the way forward, checking the same states reappear in order.
    for want in history.iter().skip(1) {
        assert!(app.redo());
        assert_eq!(&snapshot(&app), want, "state diverged while redoing");
    }
    assert!(!app.can_redo());
}

#[test]
fn a_formula_carries_its_cached_value_and_undoes_as_one() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 1.0).unwrap();
    app.set_cell(0, p(1, 0), 2.0).unwrap();
    app.set_formula(0, p(2, 0), "=SUM([.A1:.A2])").unwrap();

    // The value is computed on the way in — a formula stored without one renders blank in
    // LibreOffice (doc/ods-format.md §4).
    assert_eq!(app.get(0, p(2, 0)).unwrap(), CellValue::Number(3.0));
    assert_eq!(
        app.formula(0, p(2, 0)).unwrap().as_deref(),
        Some("=SUM([.A1:.A2])")
    );
    assert_eq!(app.formula_count(0).unwrap(), 1);

    assert!(app.undo());
    assert_eq!(
        app.formula(0, p(2, 0)).unwrap(),
        None,
        "formula and value undo together"
    );
    assert_eq!(app.get(0, p(2, 0)).unwrap(), CellValue::Empty);

    assert!(app.redo());
    assert_eq!(app.get(0, p(2, 0)).unwrap(), CellValue::Number(3.0));
}

#[test]
fn clearing_a_formula_keeps_the_value_it_computed() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 4.0).unwrap();
    app.set_formula(0, p(1, 0), "=[.A1]*2").unwrap();
    app.clear_formula(0, p(1, 0)).unwrap();

    assert_eq!(app.formula(0, p(1, 0)).unwrap(), None);
    assert_eq!(app.get(0, p(1, 0)).unwrap(), CellValue::Number(8.0));

    assert!(app.undo());
    assert_eq!(
        app.formula(0, p(1, 0)).unwrap().as_deref(),
        Some("=[.A1]*2")
    );
}

#[test]
fn recalc_is_one_undo_step_and_a_no_op_when_nothing_changed() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 1.0).unwrap();
    app.set_formula(0, p(1, 0), "=[.A1]+10").unwrap();
    app.set_formula(0, p(2, 0), "=[.A2]+100").unwrap();
    assert_eq!(app.get(0, p(2, 0)).unwrap(), CellValue::Number(111.0));

    // Change the input the two formulas depend on, without touching them.
    app.set_cell(0, p(0, 0), 2.0).unwrap();
    assert_eq!(
        app.get(0, p(1, 0)).unwrap(),
        CellValue::Number(11.0),
        "stale until a recalculation"
    );

    assert_eq!(app.recalc().unwrap().changed, 2, "both dependents changed");
    assert_eq!(app.get(0, p(1, 0)).unwrap(), CellValue::Number(12.0));
    assert_eq!(app.get(0, p(2, 0)).unwrap(), CellValue::Number(112.0));

    assert_eq!(app.recalc().unwrap().changed, 0, "already current");

    // One step back undoes the whole recalculation, not its last cell.
    assert!(app.undo());
    assert_eq!(app.get(0, p(1, 0)).unwrap(), CellValue::Number(11.0));
    assert_eq!(app.get(0, p(2, 0)).unwrap(), CellValue::Number(111.0));
}

/// This engine implements a subset of OpenFormula, so recalculating a document that uses
/// anything outside it turns a good cached value into `#NAME?`. That is honest, and it is
/// also data loss — `spoiled` is what lets a shell say so instead of writing the file back
/// quietly. Found against a real document: `table.fods` uses `SUBTOTAL`.
#[test]
fn recalc_counts_the_values_it_spoiled() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 5.0).unwrap();
    app.set_formula(0, p(1, 0), "=[.A1]*2").unwrap();

    // A cached value from elsewhere, for a function this build does not have.
    let doc = app.save_bytes(sheet_core::Form::Flat).unwrap();
    let app = App::new();
    app.open_bytes("book.fods", &doc).unwrap();
    app.set_formula(0, p(2, 0), "=SUBTOTAL(9;[.A1:.A2])")
        .unwrap();
    app.set_cell(0, p(2, 0), 169.625).unwrap();

    let recalc = app.recalc().unwrap();
    assert_eq!(recalc.changed, 1);
    assert_eq!(recalc.spoiled, 1, "a real value became an error");
    assert_eq!(
        app.get(0, p(2, 0)).unwrap(),
        CellValue::Text("#NAME?".into())
    );

    // One step back, because the whole recalculation is one action.
    assert!(app.undo());
    assert_eq!(app.get(0, p(2, 0)).unwrap(), CellValue::Number(169.625));
}

#[test]
fn recalculating_an_empty_cell_into_an_error_is_not_spoilage() {
    let app = App::new();
    app.set_formula(0, p(0, 0), "=NOSUCHFUNC()").unwrap();
    // The formula never had a good value to lose, so this must not raise the alarm.
    assert_eq!(app.recalc().unwrap().spoiled, 0);
}

#[test]
fn a_recalculated_error_is_stored_as_its_name() {
    let app = App::new();
    app.set_formula(0, p(0, 0), "=1/0").unwrap();
    // The only shape CellValue has for an error, and what LibreOffice writes into text:p
    // (doc/ods-format.md §6) — so it round-trips through our own reader.
    assert_eq!(
        app.get(0, p(0, 0)).unwrap(),
        CellValue::Text("#DIV/0!".into())
    );
}

/// The point of `--session`: an undo stack outliving the process that built it. An inverse
/// carries the value it restores, so it never has to consult the document it was recorded
/// against — which is what makes re-reading the file from disk safe.
#[test]
fn a_session_carries_history_between_apps() {
    let first = App::new();
    first.set_cell(0, p(0, 0), 1.0).unwrap();
    first.set_cell(0, p(0, 0), 2.0).unwrap();
    let bytes = first.save_bytes(sheet_core::Form::Flat).unwrap();
    let session = serde_json::to_string(&first.session()).unwrap();

    let second = App::new();
    second.open_bytes("book.fods", &bytes).unwrap();
    assert!(!second.can_undo(), "opening a document drops history");
    second.restore_session(serde_json::from_str(&session).unwrap());

    assert!(second.can_undo());
    assert!(second.undo());
    assert_eq!(second.get(0, p(0, 0)).unwrap(), CellValue::Number(1.0));
    assert!(second.undo());
    assert_eq!(second.get(0, p(0, 0)).unwrap(), CellValue::Empty);
    assert!(!second.undo());
}

#[test]
fn a_session_naming_a_missing_sheet_fails_to_undo_rather_than_panicking() {
    let app = App::new();
    let stale: sheet_core::Session = serde_json::from_str(
        r#"{"undo":[{"SetCell":{"sheet":9,"pos":{"row":0,"col":0},"value":"Empty"}}]}"#,
    )
    .unwrap();
    app.restore_session(stale);
    assert!(app.can_undo());
    assert!(
        !app.undo(),
        "an action for a sheet that is gone simply does not apply"
    );
}

struct Counter(AtomicUsize);

impl Observer for Counter {
    fn changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn every_change_notifies_the_observer() {
    let app = App::new();
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    app.set_observer(counter.clone());

    app.set_cell(0, p(0, 0), 1.0).unwrap();
    app.undo();
    app.redo();

    assert_eq!(counter.0.load(Ordering::SeqCst), 3);
}

/// An observer reads the app from inside the notification — exactly what a shell does when
/// it repaints. If the write lock were still held while notifying, this would hang forever
/// instead of failing, so it is the one test worth having before there is any UI at all.
struct Reader {
    app: Mutex<Option<Arc<App>>>,
    saw: Mutex<Vec<CellValue>>,
}

impl Observer for Reader {
    fn changed(&self) {
        let app = self.app.lock().unwrap().clone().expect("app wired up");
        // Both a point read and a viewport read: each takes the state lock.
        let value = app.get(0, p(0, 0)).unwrap();
        let _ = app.get_viewport(0, 0..4, 0..4).unwrap();
        let _ = app.can_undo();
        self.saw.lock().unwrap().push(value);
    }
}

#[test]
fn an_observer_may_read_the_app_without_deadlocking() {
    let app = Arc::new(App::new());
    let reader = Arc::new(Reader {
        app: Mutex::new(None),
        saw: Mutex::new(Vec::new()),
    });
    *reader.app.lock().unwrap() = Some(app.clone());
    app.set_observer(reader.clone());

    app.set_cell(0, p(0, 0), 1.0).unwrap();
    app.set_cell(0, p(0, 0), 2.0).unwrap();
    app.undo();

    let saw = reader.saw.lock().unwrap().clone();
    assert_eq!(
        saw,
        vec![
            CellValue::Number(1.0),
            CellValue::Number(2.0),
            CellValue::Number(1.0)
        ],
        "the observer must see the state as it is *after* each change"
    );
}

#[test]
fn the_app_is_shareable_across_threads() {
    let app = Arc::new(App::new());
    let mut handles = Vec::new();
    for t in 0..4u32 {
        let app = app.clone();
        handles.push(std::thread::spawn(move || {
            for row in 0..50u32 {
                app.set_cell(0, p(row, t), f64::from(row)).unwrap();
                let _ = app.get_viewport(0, 0..50, 0..4).unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(app.used_extent(0).unwrap(), (50, 4));
    for t in 0..4u32 {
        assert_eq!(app.get(0, p(49, t)).unwrap(), CellValue::Number(49.0));
    }
}

// --- sheets ---

#[test]
fn a_sheet_can_be_added_renamed_and_removed() {
    let app = App::new();
    assert_eq!(app.add_sheet("Data").unwrap(), 1);
    assert_eq!(app.sheet_count(), 2);
    assert_eq!(app.sheet_name(1).unwrap(), "Data");

    app.rename_sheet(1, "Q3 Actuals").unwrap();
    assert_eq!(app.sheet_name(1).unwrap(), "Q3 Actuals");
    // Renaming a sheet to what it is already called is not a duplicate.
    app.rename_sheet(1, "Q3 Actuals").unwrap();

    app.remove_sheet(1).unwrap();
    assert_eq!(app.sheet_count(), 1);
}

/// A sheet name is how a reference names one (§5.8), and that lookup is case-insensitive —
/// so two sheets differing only in case would make `[$data.$A$1]` mean either.
#[test]
fn a_sheet_name_must_be_unique_and_non_empty() {
    let app = App::new();
    assert!(app.add_sheet("sheet1").is_err());
    assert!(app.add_sheet("  ").is_err());
    assert!(app.add_sheet("").is_err());
    // A refused edit leaves nothing behind — not a sheet, and not an undo entry.
    assert_eq!(app.sheet_count(), 1);
    assert!(!app.can_undo());

    app.add_sheet("Data").unwrap();
    assert!(app.rename_sheet(1, "SHEET1").is_err());
}

/// A document with no sheet is a spreadsheet with nowhere to type.
#[test]
fn the_last_sheet_cannot_be_removed() {
    let app = App::new();
    assert!(app.remove_sheet(0).is_err());
    assert_eq!(app.sheet_count(), 1);
    assert!(!app.can_undo());
}

/// The one that makes deleting a sheet safe: the inverse carries the whole sheet, so undo
/// brings back the cells rather than an empty sheet with the right name.
#[test]
fn undoing_a_sheet_deletion_brings_the_cells_back() {
    let app = App::new();
    app.add_sheet("Data").unwrap();
    app.set_cell(1, p(4, 2), 42.0).unwrap();
    app.set_formula(1, p(5, 2), "=[.C5]*2").unwrap();

    app.remove_sheet(1).unwrap();
    assert_eq!(app.sheet_count(), 1);

    assert!(app.undo());
    assert_eq!(app.sheet_count(), 2);
    assert_eq!(app.sheet_name(1).unwrap(), "Data");
    assert_eq!(app.get(1, p(4, 2)).unwrap(), CellValue::Number(42.0));
    assert_eq!(
        app.formula(1, p(5, 2)).unwrap().as_deref(),
        Some("=[.C5]*2")
    );
    assert_eq!(app.get(1, p(5, 2)).unwrap(), CellValue::Number(84.0));
}

/// Removing a sheet shifts every later index, and the undo stack holds entries recorded in
/// the *old* numbering. It survives because the stack is strictly ordered: the older entry
/// is only applied once the removal above it has been undone and the numbering is back.
#[test]
fn undo_survives_the_index_shift_a_removal_causes() {
    let app = App::new();
    app.add_sheet("Middle").unwrap();
    app.add_sheet("Last").unwrap();
    app.set_cell(2, p(0, 0), 7.0).unwrap(); // Last.A1, recorded as sheet 2
    app.remove_sheet(1).unwrap(); // Last is now sheet 1

    assert!(app.undo()); // puts Middle back, so Last is sheet 2 again
    assert!(app.undo()); // clears Last.A1, addressed as sheet 2
    assert_eq!(app.sheet_count(), 3);
    assert_eq!(app.get(2, p(0, 0)).unwrap(), CellValue::Empty);
}

/// A deleted sheet has to survive the session file too, or `--session` would undo a deletion
/// into an empty sheet. `Pos`-keyed maps are the trap: JSON has no key but a string.
#[test]
fn a_deleted_sheet_survives_a_session_round_trip() {
    let app = App::new();
    app.add_sheet("Data").unwrap();
    app.set_cell(1, p(0, 0), "kept").unwrap();
    app.remove_sheet(1).unwrap();

    let json = serde_json::to_string(&app.session()).unwrap();
    // A fresh app, as a stateless shell has: the document is re-read and only the stacks
    // come from the file.
    let restored = App::new();
    restored.restore_session(serde_json::from_str(&json).unwrap());

    assert!(restored.undo());
    assert_eq!(restored.sheet_count(), 2);
    assert_eq!(
        restored.get(1, p(0, 0)).unwrap(),
        CellValue::Text("kept".into())
    );
}

// --- the typing rule, and what a shell needs behind it (doc/gtk-shell.md C3–C6) ---

/// Every branch of `enter`'s interpretation, including the two that exist so that a string
/// can be typed at all.
#[test]
fn enter_reads_what_was_typed_the_way_every_spreadsheet_does() {
    let app = App::new();
    for (row, input, kind, value) in [
        (0, "12.5", Entered::Number, CellValue::Number(12.5)),
        (1, "TRUE", Entered::Bool, CellValue::Bool(true)),
        (2, "hello", Entered::Text, CellValue::Text("hello".into())),
        // The `'` rule: without it neither of these two cells could hold what it holds.
        (
            3,
            "'=SUM(A1)",
            Entered::Text,
            CellValue::Text("=SUM(A1)".into()),
        ),
        (4, "'123", Entered::Text, CellValue::Text("123".into())),
        (5, "", Entered::Cleared, CellValue::Empty),
    ] {
        let outcome = app.enter(0, p(row, 0), input, RecalcMode::No).unwrap();
        assert_eq!(outcome.kind, kind, "{input}");
        assert_eq!(outcome.cells, 1);
        assert_eq!(app.get(0, p(row, 0)).unwrap(), value, "{input}");
    }

    let outcome = app.enter(0, p(6, 0), "=[.A1]*2", RecalcMode::No).unwrap();
    assert_eq!(outcome.kind, Entered::Formula);
    assert_eq!(app.get(0, p(6, 0)).unwrap(), CellValue::Number(25.0));
    assert_eq!(
        app.formula(0, p(6, 0)).unwrap().as_deref(),
        Some("=[.A1]*2")
    );
}

/// Typing a value over a formula has to remove the formula. Two actions — set the value,
/// clear the formula — would leave a stale formula beside a fresh cached value, which is
/// exactly the disagreement `stale` exists to report.
#[test]
fn typing_over_a_formula_removes_it() {
    let app = App::new();
    app.enter(0, p(0, 0), "=1+1", RecalcMode::No).unwrap();
    app.enter(0, p(0, 0), "7", RecalcMode::No).unwrap();
    assert_eq!(app.formula(0, p(0, 0)).unwrap(), None);
    assert_eq!(app.get(0, p(0, 0)).unwrap(), CellValue::Number(7.0));

    // And one undo puts both back.
    assert!(app.undo());
    assert_eq!(app.formula(0, p(0, 0)).unwrap().as_deref(), Some("=1+1"));
}

/// The whole point of `RecalcMode::Document`: an edit and the ripple it causes are one
/// undo entry, so Ctrl+Z takes back what the user did rather than half of it.
#[test]
fn an_edit_and_its_ripple_undo_together() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 1.0).unwrap();
    app.set_formula(0, p(1, 0), "=[.A1]*10").unwrap();
    assert_eq!(app.get(0, p(1, 0)).unwrap(), CellValue::Number(10.0));

    let outcome = app.enter(0, p(0, 0), "5", RecalcMode::Document).unwrap();
    assert_eq!(
        outcome.recalc.unwrap(),
        Recalc {
            changed: 1,
            spoiled: 0
        }
    );
    assert_eq!(app.get(0, p(1, 0)).unwrap(), CellValue::Number(50.0));

    assert!(app.undo());
    assert_eq!(app.get(0, p(0, 0)).unwrap(), CellValue::Number(1.0));
    assert_eq!(
        app.get(0, p(1, 0)).unwrap(),
        CellValue::Number(10.0),
        "one undo, both cells"
    );
    // And it really was one entry: the next undo is the formula's own, not the ripple's.
    assert!(app.undo());
    assert_eq!(app.formula(0, p(1, 0)).unwrap(), None);
}

/// The edit lands even when its recalculation cannot: refusing would make a document that
/// uses one unimplemented function read-only, which is worse than leaving it stale.
#[test]
fn an_edit_that_would_spoil_a_cell_still_commits_without_recalculating() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 5.0).unwrap();
    app.set_formula(0, p(1, 0), "=SUBTOTAL(9;[.A1:.A1])")
        .unwrap();
    app.set_cell(0, p(1, 0), 5.0).unwrap(); // a cached value from a better evaluator

    let outcome = app.enter(0, p(0, 0), "6", RecalcMode::Document).unwrap();
    let recalc = outcome.recalc.unwrap();
    assert_eq!(recalc.spoiled, 1);
    assert!(
        recalc.changed > 0,
        "and it says how much is now out of date"
    );
    assert_eq!(
        app.get(0, p(0, 0)).unwrap(),
        CellValue::Number(6.0),
        "the edit stands"
    );
    assert_eq!(
        app.get(0, p(1, 0)).unwrap(),
        CellValue::Number(5.0),
        "the cached value was not replaced with #NAME?"
    );
}

/// `preview` is called while someone is typing, from a worker thread. All three properties
/// are what makes that safe, and none of them is visible from the value it returns.
#[test]
fn preview_writes_nothing_notifies_nobody_and_records_no_history() {
    let app = Arc::new(App::new());
    app.set_cell(0, p(0, 0), 4.0).unwrap();
    let counter = Arc::new(Counter(AtomicUsize::new(0)));
    app.set_observer(counter.clone());

    assert_eq!(
        app.preview(0, p(9, 9), "=[.A1]*3").unwrap(),
        CellValue::Number(12.0)
    );
    assert_eq!(counter.0.load(Ordering::SeqCst), 0, "no observer tick");
    assert!(
        app.undo() && !app.can_undo(),
        "only the one edit is in the history"
    );
    assert_eq!(
        app.get(0, p(9, 9)).unwrap(),
        CellValue::Empty,
        "nothing stored"
    );

    // Only a read lock: a second reader can hold one at the same time.
    let held = app.get_viewport(0, 0..1, 0..1).unwrap();
    assert_eq!(
        app.preview(0, p(0, 1), "=1+1").unwrap(),
        CellValue::Number(2.0)
    );
    drop(held);
}

#[test]
fn clearing_a_range_is_one_undo_step_and_takes_the_formulas_with_it() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 1.0).unwrap();
    app.set_formula(0, p(1, 1), "=[.A1]+1").unwrap();
    app.set_cell(0, p(3, 3), "far away").unwrap();

    assert_eq!(
        app.clear_range(0, p(0, 0), p(2, 2)).unwrap(),
        2,
        "only the two"
    );
    assert_eq!(app.get(0, p(1, 1)).unwrap(), CellValue::Empty);
    assert_eq!(app.formula(0, p(1, 1)).unwrap(), None);
    assert_eq!(
        app.get(0, p(3, 3)).unwrap(),
        CellValue::Text("far away".into())
    );

    assert!(app.undo());
    assert_eq!(app.get(0, p(0, 0)).unwrap(), CellValue::Number(1.0));
    assert_eq!(
        app.formula(0, p(1, 1)).unwrap().as_deref(),
        Some("=[.A1]+1")
    );

    // A rectangle nobody could mean is refused by size rather than served slowly.
    assert!(app.clear_range(0, p(0, 0), p(1_000_000, 100)).is_err());
}

#[test]
fn a_pasted_rectangle_lands_cell_by_cell_in_one_step() {
    let app = App::new();
    let rows = vec![
        vec!["1".to_owned(), "2".to_owned()],
        vec!["=[.B2]*2".to_owned()], // ragged, and a formula
    ];
    let outcome = app
        .enter_range(0, p(1, 1), &rows, RecalcMode::Document)
        .unwrap();
    assert_eq!(outcome.cells, 3);
    assert_eq!(outcome.kind, Entered::Number, "the anchor's kind");
    assert_eq!(app.get(0, p(1, 1)).unwrap(), CellValue::Number(1.0));
    assert_eq!(app.get(0, p(1, 2)).unwrap(), CellValue::Number(2.0));
    assert_eq!(app.get(0, p(2, 1)).unwrap(), CellValue::Number(2.0));

    assert!(app.undo());
    assert_eq!(app.get(0, p(1, 1)).unwrap(), CellValue::Empty);
    assert!(!app.can_undo());
}

/// What an editor shows and what pressing Enter means have to be inverses, or opening a
/// cell and closing it again quietly changes the document. Every kind of cell, in one loop.
#[test]
fn what_an_editor_shows_enters_back_as_the_same_cell() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 5.0).unwrap();
    let cells = [
        (1, "42"),
        (2, "hello"),
        (3, "TRUE"),
        (4, "'007"),      // text that would otherwise read as a number
        (5, "'=SUM(A1)"), // text that would otherwise read as a formula
        (6, "=[.A1]*2"),  // a formula, which comes back in display form
        (7, ""),
    ];
    for (row, input) in cells {
        app.enter(0, p(row, 0), input, RecalcMode::No).unwrap();
    }
    for (row, input) in cells {
        let before = app.get(0, p(row, 0)).unwrap();
        let shown = app.input_text(0, p(row, 0)).unwrap();
        // The one step a shell always takes: what an editor holds is display form, and
        // `enter` takes the canonical syntax the file stores.
        let typed = match shown.starts_with('=') {
            true => sheet_core::formula::display::from_display(&shown).unwrap(),
            false => shown.clone(),
        };
        app.enter(0, p(row, 0), &typed, RecalcMode::No).unwrap();
        assert_eq!(
            app.get(0, p(row, 0)).unwrap(),
            before,
            "{input} showed as {shown:?}"
        );
    }
    // A formula is shown the way a formula bar shows one, and stored the way ODF does.
    assert_eq!(app.input_text(0, p(6, 0)).unwrap(), "=A1*2");
    assert_eq!(
        app.formula(0, p(6, 0)).unwrap().as_deref(),
        Some("=[.A1]*2")
    );
}

/// A date-formatted cell edits as the date it displays, not the serial underneath — the
/// same "editor shows what enter takes" guarantee as the loop above, but for a cell whose
/// format says it holds a date rather than a plain number.
#[test]
fn a_date_formatted_cell_is_edited_as_a_date_not_a_serial() {
    let app = App::new();
    app.set_format(
        0,
        p(0, 0),
        p(0, 0),
        Some(sheet_core::numfmt::preset(
            sheet_core::numfmt::Kind::Date,
            0,
            false,
            "",
        )),
    )
    .unwrap();
    app.enter(0, p(0, 0), "2027-08-20", RecalcMode::No).unwrap();
    let shown = app.input_text(0, p(0, 0)).unwrap();
    assert_eq!(
        shown, "2027-08-20",
        "the editor should show the date, not its serial"
    );

    let before = app.get(0, p(0, 0)).unwrap();
    app.enter(0, p(0, 0), &shown, RecalcMode::No).unwrap();
    assert_eq!(
        app.get(0, p(0, 0)).unwrap(),
        before,
        "re-entering what was shown must round-trip"
    );
}

/// C7 and C8 together: what a toolbar does. A bold button is a read, a field, and a write —
/// and the grid learns about it from the viewport rather than by asking per cell.
#[test]
fn a_style_reads_back_and_rides_in_the_viewport() {
    let app = App::new();
    app.set_cell(0, p(0, 0), 1.5).unwrap();
    assert_eq!(
        app.style_at(0, p(0, 0)).unwrap(),
        None,
        "a plain cell has no style"
    );
    assert_eq!(app.format_at(0, p(0, 0)).unwrap(), None);

    // Read, merge, write — the flow `set_style`'s docs promise and the one a bold toggle is.
    let mut style = app.style_at(0, p(0, 0)).unwrap().unwrap_or_default();
    style.font_weight = Some("bold".into());
    app.set_style(0, p(0, 0), p(0, 0), Some(style)).unwrap();
    style = app.style_at(0, p(0, 0)).unwrap().unwrap();
    assert_eq!(style.font_weight.as_deref(), Some("bold"));
    style.background = Some("#ffff00".into());
    app.set_style(0, p(0, 0), p(0, 0), Some(style)).unwrap();

    app.set_format(
        0,
        p(0, 0),
        p(0, 0),
        Some(sheet_core::numfmt::preset(
            sheet_core::numfmt::Kind::Percentage,
            1,
            false,
            "",
        )),
    )
    .unwrap();
    let format = app.format_at(0, p(0, 0)).unwrap().unwrap();
    assert_eq!(format.kind, sheet_core::numfmt::Kind::Percentage);

    let v = app.get_viewport(0, 0..2, 0..2).unwrap();
    let seen = v
        .style(0, 0)
        .expect("the styled cell carries its style into the viewport");
    assert_eq!(seen.font_weight.as_deref(), Some("bold"));
    assert_eq!(seen.background.as_deref(), Some("#ffff00"));
    assert_eq!(v.style(1, 1), None, "an unstyled cell");
    assert_eq!(
        v.style(9, 9),
        None,
        "and one outside the viewport, which needs no distinction"
    );
    assert_eq!(
        v.text(0, 0),
        Some("150.0%"),
        "the format is still what decides the text"
    );

    // Clearing is the same call with `None`, and the viewport forgets it too.
    app.set_style(0, p(0, 0), p(0, 0), None).unwrap();
    assert_eq!(app.style_at(0, p(0, 0)).unwrap(), None);
    assert_eq!(app.get_viewport(0, 0..1, 0..1).unwrap().style(0, 0), None);
    assert!(app.style_at(9, p(0, 0)).is_err() && app.format_at(9, p(0, 0)).is_err());
}
