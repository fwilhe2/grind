// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The parity ratchet: doc/plan.md's phase 6 exit criterion, which asks for "a test that
//! walks the public API and fails on anything unexposed" — now **once per application**
//! (doc/suite.md's R9).
//!
//! Rust cannot reflect over its own API, so this reads the source instead and checks it
//! against a parity document — the same mechanism `funcs::implemented()` is held to by
//! `doc/small-group.md`. Adding a method to an `App` without saying how a user reaches it
//! fails the build; so does leaving a row behind for a method that no longer exists.
//!
//! **The registry is the point of the suite version.** One `APPS` array names every
//! (application, core source, parity document) triple, so a second document type is three
//! lines here rather than a copy of this file. A copy is how two ratchets end up with one of
//! them quietly turned off.

/// Every application, its core's crate root, and the document that has to account for it.
///
/// Read at *compile* time, so this cannot pass by looking in the wrong place at runtime.
const APPS: [App; 2] = [
    App {
        name: "sheet",
        core: include_str!("../../sheet/src/lib.rs"),
        core_path: "sheet/src/lib.rs",
        parity: include_str!("../../doc/cli-parity-sheet.md"),
        parity_path: "doc/cli-parity-sheet.md",
        least: 12,
    },
    App {
        name: "text",
        core: include_str!("../../text/src/lib.rs"),
        core_path: "text/src/lib.rs",
        parity: include_str!("../../doc/cli-parity-text.md"),
        parity_path: "doc/cli-parity-text.md",
        least: 12,
    },
];

struct App {
    name: &'static str,
    core: &'static str,
    core_path: &'static str,
    parity: &'static str,
    parity_path: &'static str,
    /// The smallest API that is plausibly complete. A scanner that matched nothing would pass
    /// vacuously and quietly retire the ratchet, which is the failure mode actually worth
    /// guarding against.
    least: usize,
}

/// Every `pub fn` in `impl App`.
fn public_methods(app: &App) -> Vec<&'static str> {
    let body = app
        .core
        .split_once("impl App {")
        .unwrap_or_else(|| panic!("{} still defines `impl App`", app.core_path))
        .1;
    // rustfmt puts the closing brace of a top-level impl in column 0, and nothing inside the
    // block is indented that far.
    let body = body
        .split_once("\n}")
        .unwrap_or_else(|| panic!("{}'s impl block ends", app.core_path))
        .0;

    body.lines()
        .filter_map(|line| line.trim().strip_prefix("pub fn "))
        .filter_map(|rest| rest.split(['(', '<']).next())
        .collect()
}

/// Every method named in a parity document, with the text explaining how it is reached.
fn documented(app: &App) -> Vec<(&'static str, &'static str)> {
    app.parity
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- `"))
        .filter_map(|rest| rest.split_once('`'))
        .map(|(name, how)| (name, how.trim_start_matches([' ', '—']).trim()))
        .collect()
}

#[test]
fn every_core_capability_is_reachable_from_the_cli() {
    for app in &APPS {
        let methods = public_methods(app);
        assert!(
            methods.len() >= app.least,
            "only found {} methods on {}'s App — the scan is broken, not the API",
            methods.len(),
            app.name
        );

        let documented = documented(app);
        for method in &methods {
            let row = documented.iter().find(|(name, _)| name == method);
            let (_, how) = row.unwrap_or_else(|| {
                panic!(
                    "App::{method} is not in {} — expose it from `grind {}`, or say there why \
                     it is not reachable",
                    app.parity_path, app.name
                )
            });
            assert!(
                !how.is_empty(),
                "App::{method} has no explanation in {}",
                app.parity_path
            );
            if let Some(reason) = how.strip_prefix("not exposed:") {
                assert!(
                    reason.trim().len() > 20,
                    "App::{method} is exempted without a real reason: {how:?}"
                );
            }
        }

        eprintln!(
            "cli parity: grind {} — {} public App methods, all accounted for",
            app.name,
            methods.len()
        );
    }
}

#[test]
fn the_parity_document_names_no_method_that_is_gone() {
    for app in &APPS {
        let methods = public_methods(app);
        // Only the rows in the sections about `App`; the last section lists free functions on
        // purpose and says so.
        let (app_rows, _) = app
            .parity
            .split_once("## Beyond `App`")
            .unwrap_or_else(|| panic!("{} still has its trailing section", app.parity_path));

        for line in app_rows.lines() {
            let Some((name, _)) = line
                .trim()
                .strip_prefix("- `")
                .and_then(|rest| rest.split_once('`'))
            else {
                continue;
            };
            assert!(
                methods.contains(&name),
                "{} lists App::{name}, which no longer exists — delete the row",
                app.parity_path
            );
        }
    }
}

#[test]
fn every_app_has_its_own_parity_document() {
    // R9 is per-app, so two applications sharing one document would satisfy every test above
    // while checking half as much. Cheap to assert, and the assertion is the requirement.
    for (i, app) in APPS.iter().enumerate() {
        assert!(
            app.parity_path.contains(app.name),
            "grind {}'s parity document is {} — name it after the app it accounts for",
            app.name,
            app.parity_path
        );
        for other in &APPS[i + 1..] {
            assert_ne!(
                app.parity_path, other.parity_path,
                "grind {} and grind {} share a parity document",
                app.name, other.name
            );
        }
    }
}
