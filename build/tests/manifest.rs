// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **R11's guard: no evaluator on any read path** (`doc/dsl.md` §2).
//!
//! `doc/not-doing.md` §1 rules out macros — "a document that computes is the goal; a document
//! that *executes* is not" — and the generator is on the right side of that line only for as
//! long as nothing that *opens* a document links it. That is a fact about manifests, so it is
//! checked the way R8 is checked in `core/tests/generic.rs`: read them, and fail the build.
//!
//! Three halves, because the rule has three ways to rot:
//!
//! 1. **A reader taking the dependency.** `grind-core`, `grind-sheet`, `grind-text` or a shell
//!    naming `grind-build` would put an evaluator behind `App::open_file`, which is the whole
//!    thing R11 forbids.
//! 2. **The guard going vacuous.** A new shell nobody added below would be unchecked and this
//!    file would still pass, so the workspace's member list is read and every member has to be
//!    accounted for.
//! 3. **The one crate that *may* link it not doing so.** `grind-cli` is where `grind build`
//!    lives; if it stopped depending on this crate the verb would be gone, and a guard that
//!    passes because the feature vanished is not a guard.

/// Read at *compile* time, so this cannot pass by looking in the wrong place at runtime.
const WORKSPACE: &str = include_str!("../../Cargo.toml");

/// Every crate that must **not** name the generator, with its manifest.
const READERS: [(&str, &str); 8] = [
    ("core", include_str!("../../core/Cargo.toml")),
    ("sheet", include_str!("../../sheet/Cargo.toml")),
    ("text", include_str!("../../text/Cargo.toml")),
    (
        "ui_sheet_gtk",
        include_str!("../../ui_sheet_gtk/Cargo.toml"),
    ),
    ("ui_text_gtk", include_str!("../../ui_text_gtk/Cargo.toml")),
    ("ui_tui", include_str!("../../ui_tui/Cargo.toml")),
    ("ui_web", include_str!("../../ui_web/Cargo.toml")),
    ("ui_win32", include_str!("../../ui_win32/Cargo.toml")),
];

/// The crates R11 exempts, and why each one is allowed to link an evaluator.
const LINKERS: [(&str, &str); 2] = [
    ("build", "is the generator"),
    ("cli", "is the only binary that runs one"),
];

const CRATE: &str = "grind-build";

#[test]
fn no_read_path_links_the_generator() {
    for (name, manifest) in READERS {
        assert!(
            !manifest.contains(CRATE),
            "{name}/Cargo.toml names {CRATE}. R11: the generator is not a dependency of \
             anything that opens a document — see doc/dsl.md §2. A window that wants a \
             *Rebuild from source* button runs the `grind` binary instead."
        );
    }
}

/// The vacuity guard. A shell added to the workspace and not to [`READERS`] would leave this
/// file passing while checking one crate fewer, which is how a ratchet quietly stops.
#[test]
fn every_member_of_the_workspace_is_accounted_for() {
    let members = WORKSPACE
        .split_once("members = [")
        .expect("the workspace lists its members")
        .1
        .split_once(']')
        .expect("that list ends")
        .0;
    let members: Vec<&str> = members
        .split(',')
        .map(|entry| entry.trim().trim_matches('"'))
        .filter(|entry| !entry.is_empty())
        .collect();
    assert!(
        members.len() >= 9,
        "the workspace has more than a few crates"
    );

    for member in members {
        let checked = READERS.iter().any(|(name, _)| *name == member);
        let allowed = LINKERS.iter().any(|(name, _)| *name == member);
        assert!(
            checked || allowed,
            "{member} is in the workspace and in neither list here. Add it to READERS — or, \
             if it really is allowed to link {CRATE}, to LINKERS with the reason."
        );
    }
}

/// The other direction: `grind build` is reachable, which is what makes the rule above a
/// statement about *where* the evaluator is rather than about whether it exists.
#[test]
fn the_cli_is_the_one_binary_that_runs_a_script() {
    let cli = include_str!("../../cli/Cargo.toml");
    assert!(
        cli.contains(CRATE),
        "cli/Cargo.toml no longer names {CRATE}, so nothing can run a script at all"
    );
}
