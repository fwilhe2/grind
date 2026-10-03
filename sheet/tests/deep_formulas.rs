// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A formula too deep to walk is an error in its cell, never a crash.
//!
//! Every function that walks an expression recurses once per level, so before the parser had
//! limits (`formula::parse::{MAX_NESTING, MAX_DEPTH}`) a document with `((…((1))…))` 3000 deep
//! in one cell aborted the process on recalculation — and a window takes every unsaved
//! document in it down with it. Run on a thread with the 8 MB every shell's main thread has
//! (Linux and macOS by default, Windows by `.cargo/config.toml`, the browser by its build), so
//! a formula at the limits is walked in the stack a real one would be.

use grind_sheet::formula::parse::{MAX_DEPTH, MAX_NESTING};
use grind_sheet::{App, CellValue, Pos};

fn on_a_main_threads_stack(work: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(work)
        .expect("spawns")
        .join()
        .expect("did not crash");
}

#[test]
fn formulas_at_the_limits_evaluate_and_past_them_are_errors() {
    on_a_main_threads_stack(|| {
        let nested = |n: usize| format!("={}1{}", "ABS(".repeat(n), ")".repeat(n));
        let parens = |n: usize| format!("={}1{}", "(".repeat(n), ")".repeat(n));
        let chain = |n: usize| format!("={}", vec!["1"; n].join("+"));
        let app = App::new();
        let cells = [
            (nested(MAX_NESTING), Some(1.0)),
            (parens(MAX_NESTING), Some(1.0)),
            (chain(MAX_DEPTH - 1), Some((MAX_DEPTH - 1) as f64)),
            (nested(100_000), None),
            (parens(100_000), None),
            (chain(100_000), None),
        ];
        for (row, (formula, _)) in cells.iter().enumerate() {
            app.set_formula(0, Pos::new(row as u32, 0), formula)
                .expect("a formula is stored as written");
        }
        app.recalc().expect("recalculates");
        for (row, (_, want)) in cells.iter().enumerate() {
            let got = app.get(0, Pos::new(row as u32, 0)).expect("reads");
            match want {
                Some(n) => assert_eq!(got, CellValue::Number(*n), "row {row}"),
                // An error value, which a cell holds as its name.
                None => assert_eq!(got, CellValue::Text("#NAME?".into()), "row {row}"),
            }
        }
    });
}
