// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind sheet lint` against real documents — `doc/dsl.md` §4.3, D6.
//!
//! The unit tests beside the rules (`sheet/src/lint.rs`) build the document each rule is about;
//! this runs every rule over R7's fourteen vendored documents, which nobody wrote for a linter.
//! Two things are checked and they are the two that matter:
//!
//! 1. **Linting writes nothing.** Open, ask for every rule including the hints, save, and assert
//!    the bytes are identical. `doc/view-modes.md`'s headline check, for the same reason — a
//!    feature that only reads cannot be verified by loop C, so it is verified here.
//! 2. **It survives every one of them.** A rule that panics on a document LibreOffice wrote is
//!    the failure mode a corpus exists to find, and the fourteen documents between them have
//!    charts, filters, named expressions, styles and formulas that this build cannot evaluate.

use std::path::{Path, PathBuf};

use grind_core::lint::{Options, Severity};
use grind_sheet::App;

/// R7's documents, both halves. Listed the way `kb.rs` lists them — the requirement is these
/// files, so one going missing must fail rather than quietly shrink the run.
const DOCUMENTS: [(&str, &str); 14] = [
    ("kb", "filter.fods"),
    ("kb", "fizzbuzz.fods"),
    ("kb", "formula.fods"),
    ("kb", "minimal.fods"),
    ("kb", "minimal-libreoffice.fods"),
    ("kb", "minimal-libreoffice-cleanup.fods"),
    ("kb", "minimal-with-styles.fods"),
    ("kb", "named-range.fods"),
    ("samples", "Quarterly Sales Report.fods"),
    ("samples", "Sales Dashboard.fods"),
    ("samples", "conditional-formatting.fods"),
    ("samples", "custom-colors.fods"),
    ("samples", "spreadsheet.fods"),
    ("samples", "table.fods"),
];

fn data(dir: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(dir)
        .join(name)
}

fn everything() -> Options {
    Options {
        hints: true,
        off: Vec::new(),
    }
}

fn opened(path: &Path) -> App {
    let app = App::new();
    app.open_file(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    app
}

/// The promise: reading a document this way leaves its bytes alone.
#[test]
fn linting_writes_nothing() {
    for (dir, name) in DOCUMENTS {
        let path = data(dir, name);
        let before = std::fs::read(&path).expect("a readable document");
        let app = opened(&path);

        let report = app.lint(&everything());
        // Read the findings rather than only counting them, so nothing here is optimised away
        // and every address really is produced.
        for diagnostic in &report.diagnostics {
            assert!(!diagnostic.rule.is_empty(), "{name}");
            assert!(!diagnostic.message.is_empty(), "{name}: {diagnostic}");
        }

        let after = app
            .save_bytes(grind_sheet::Form::Flat)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            before, after,
            "{name}: linting changed the document — R6 says an untouched save is the bytes that \
             were read, and a lint touches nothing at all"
        );
    }
}

/// Every rule, over every document, without a panic — and the ones that fire, named.
///
/// The assertion is deliberately weak about *which* documents say what: these files are
/// vendored for other reasons and are free to change. What it does pin is that the corpus
/// exercises more than one rule, so a linter that silently stopped finding anything would fail
/// here rather than pass quietly.
#[test]
fn the_corpus_exercises_more_than_one_rule() {
    let mut fired: Vec<&'static str> = Vec::new();
    for (dir, name) in DOCUMENTS {
        let report = opened(&data(dir, name)).lint(&everything());
        for diagnostic in report.diagnostics {
            if !fired.contains(&diagnostic.rule) {
                fired.push(diagnostic.rule);
            }
        }
    }
    fired.sort_unstable();
    assert!(
        fired.len() >= 2,
        "R7's documents between them fired only {fired:?}"
    );
}

/// A hint is off unless it is asked for — the same rule as in the unit tests, held against a
/// document that really does use colours outside the palette.
#[test]
fn hints_stay_off_until_they_are_asked_for() {
    // `custom-colors.fods` is `PALETTE` as LibreOffice wrote it, so it is the one document in
    // the corpus guaranteed *not* to fire `off-palette`; the dashboard is the opposite case.
    let loud = opened(&data("samples", "Sales Dashboard.fods")).lint(&everything());
    let quiet = opened(&data("samples", "Sales Dashboard.fods")).lint(&Options::default());
    assert_eq!(
        quiet
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Hint)
            .count(),
        0,
        "the default report carries no hints"
    );
    assert!(
        loud.len() >= quiet.len(),
        "asking for hints cannot lose a finding"
    );
}
