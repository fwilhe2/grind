// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The parity ratchet: doc/plan.md's phase 6 exit criterion, which asks for "a test that
//! walks the public API and fails on anything unexposed".
//!
//! Rust cannot reflect over its own API, so this reads the source instead and checks it
//! against `doc/cli-parity.md` — the same mechanism `funcs::implemented()` is held to by
//! `doc/small-group.md`. Adding a method to `App` without saying how a user reaches it fails
//! the build; so does leaving a row behind for a method that no longer exists.

/// Both files are read at *compile* time, so this cannot pass by looking in the wrong place
/// at runtime.
const CORE: &str = include_str!("../../core/src/lib.rs");
const PARITY: &str = include_str!("../../doc/cli-parity.md");

/// Every `pub fn` in `impl App`.
fn public_methods() -> Vec<&'static str> {
    let body = CORE
        .split_once("impl App {")
        .expect("core/src/lib.rs still defines `impl App`")
        .1;
    // rustfmt puts the closing brace of a top-level impl in column 0, and nothing inside the
    // block is indented that far.
    let body = body.split_once("\n}").expect("the impl block ends").0;

    body.lines()
        .filter_map(|line| line.trim().strip_prefix("pub fn "))
        .filter_map(|rest| rest.split(['(', '<']).next())
        .collect()
}

/// Every method named in doc/cli-parity.md, with the text explaining how it is reached.
fn documented() -> Vec<(&'static str, &'static str)> {
    PARITY
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- `"))
        .filter_map(|rest| rest.split_once('`'))
        .map(|(name, how)| (name, how.trim_start_matches([' ', '—']).trim()))
        .collect()
}

#[test]
fn every_core_capability_is_reachable_from_the_cli() {
    let methods = public_methods();
    // A scanner that matched nothing would pass vacuously and quietly retire the ratchet,
    // which is the failure mode actually worth guarding against.
    assert!(
        methods.len() >= 12,
        "only found {} methods on App — the scan is broken, not the API",
        methods.len()
    );

    let documented = documented();
    for method in &methods {
        let row = documented.iter().find(|(name, _)| name == method);
        let (_, how) = row.unwrap_or_else(|| {
            panic!(
                "App::{method} is not in doc/cli-parity.md — expose it from the CLI, or say \
                 there why it is not reachable"
            )
        });
        assert!(
            !how.is_empty(),
            "App::{method} has no explanation in doc/cli-parity.md"
        );
        if let Some(reason) = how.strip_prefix("not exposed:") {
            assert!(
                reason.trim().len() > 20,
                "App::{method} is exempted without a real reason: {how:?}"
            );
        }
    }

    eprintln!(
        "cli parity: {} public App methods, all accounted for",
        methods.len()
    );
}

#[test]
fn the_parity_document_names_no_method_that_is_gone() {
    let methods = public_methods();
    // Only the rows in the sections about `App`; the last section lists free functions on
    // purpose and says so.
    let (app_rows, _) = PARITY
        .split_once("## Beyond `App`")
        .expect("doc/cli-parity.md still has its trailing section");

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
            "doc/cli-parity.md lists App::{name}, which no longer exists — delete the row"
        );
    }
}
