// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The public API, exercised only from outside — which is also how we find out whether it
//! is usable from outside.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sheet_core::{App, CellValue, Observer, Pos};

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
