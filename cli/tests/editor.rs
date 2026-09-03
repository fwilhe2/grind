// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **`doc/editor-setup.md`'s two files, held to the vocabularies they describe.**
//!
//! Editor support is documentation that happens to be machine-readable, and it rots exactly the
//! way documentation rots: a function is added, a node is added, and the file somebody's
//! completion list comes from still describes last month's language. The house rule applies —
//! a document that nothing checks drifts — so each of the two files in `.vscode/` has the
//! mechanical check its genre allows.
//!
//! | File | Written by | Checked how |
//! |---|---|---|
//! | `.vscode/grind.code-snippets` | `grind definitions --snippets` | it *is* what the command prints today |
//! | `.vscode/grind-projection.code-snippets` | by hand | every node in the two grammar notes has a snippet, and every snippet names a node |
//!
//! The second is weaker than the first on purpose, and the weakness is named: this checks
//! **coverage**, not wording. A snippet whose description has gone out of date is a prose
//! problem, and prose problems are what `doc/projection-sheet.md` itself is for.
//!
//! The last two tests are `doc/dsl.md` §7's exit criteria for D12 and D14, which both come down
//! to the same thing: a guide whose examples have moved should fail the build rather than the
//! reader.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Read at *compile* time, so a test cannot pass by looking in the wrong place at runtime.
const GENERATED: &str = include_str!("../../.vscode/grind.code-snippets");
const PROJECTION: &str = include_str!("../../.vscode/grind-projection.code-snippets");
const SHEET_GRAMMAR: &str = include_str!("../../doc/projection-sheet.md");
const TEXT_GRAMMAR: &str = include_str!("../../doc/projection-text.md");
const PROJECTION_GUIDE: &str = include_str!("../../doc/projection-guide.md");
const GENERATOR_GUIDE: &str = include_str!("../../doc/generator-guide.md");

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
}

/// The `prefix` of every snippet in one file, which is what somebody types.
fn prefixes(file: &str) -> Vec<String> {
    let parsed: serde_json::Value = serde_json::from_str(file).expect("a snippet file is JSON");
    let mut found: Vec<String> = parsed
        .as_object()
        .expect("a snippet file is an object")
        .values()
        .map(|entry| {
            entry["prefix"]
                .as_str()
                .expect("every snippet has a prefix")
                .to_owned()
        })
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The node names of a grammar note, from the first cell of the tables named by `under`.
///
/// Bounded by heading, exactly as `build/tests/spec.rs` bounds its scan: the other tables in
/// those documents have first cells that are not node names — `doc/projection-text.md`'s inline
/// table starts one row with `text:span`, and its *element* is not something anybody types.
fn nodes(grammar: &str, under: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    let mut inside = false;
    for line in grammar.lines() {
        if line.starts_with('#') {
            let heading = line.trim_start_matches('#').trim();
            inside = under.contains(&heading);
            continue;
        }
        if !inside {
            continue;
        }
        let Some(rest) = line.trim().strip_prefix("| `") else {
            continue;
        };
        if let Some((node, _)) = rest.split_once('`') {
            found.push(node.to_owned());
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Every node either grammar note spells, in one list — which is what a person writing a
/// `.grind` by hand has to be offered.
fn every_node() -> Vec<String> {
    let mut all = nodes(
        SHEET_GRAMMAR,
        &["The nodes", "The parts of a number format"],
    );
    all.extend(nodes(TEXT_GRAMMAR, &["The blocks"]));
    all.sort();
    all.dedup();
    all
}

/// `.vscode/grind.code-snippets` is what `grind definitions --snippets` prints today.
///
/// Checked in rather than generated on demand for `examples/grind.d.rhai`'s reason: an editor
/// should help somebody who has just cloned this repository, before they have run anything. The
/// cost is that it goes stale, and this is the test that says so — with the command to fix it
/// in the message, which is the whole of the maintenance burden.
#[test]
fn the_shipped_snippets_are_current() {
    assert_eq!(
        GENERATED.trim_end(),
        grind_build::snippets().trim_end(),
        ".vscode/grind.code-snippets is out of date — run \
         `grind definitions --snippets > .vscode/grind.code-snippets`"
    );
}

/// The vacuity guard: a scanner that found nothing would pass every test below.
#[test]
fn the_scanners_found_a_vocabulary() {
    assert!(prefixes(GENERATED).len() >= 25, "the generated snippets");
    assert!(every_node().len() >= 20, "the grammar notes' nodes");
}

#[test]
fn every_projection_node_has_a_snippet() {
    let offered = prefixes(PROJECTION);
    let missing: Vec<String> = every_node()
        .into_iter()
        .filter(|node| !offered.contains(node))
        .collect();
    assert!(
        missing.is_empty(),
        "nodes with no snippet: {missing:?}. A node that exists and is not offered is one \
         somebody has to read a Rust module to find — add it to \
         .vscode/grind-projection.code-snippets."
    );
}

#[test]
fn every_snippet_names_a_node() {
    let documented = every_node();
    let extra: Vec<String> = prefixes(PROJECTION)
        .into_iter()
        // The kind header is not a node of either grammar: it is the line above them all, and
        // `grind_core::kind` owns it rather than either application (R8).
        .filter(|prefix| prefix != "grind" && !documented.contains(prefix))
        .collect();
    assert!(
        extra.is_empty(),
        "snippets for nothing: {extra:?}. Offering a node the format does not have is worse \
         than offering none."
    );
}

/// Every `examples/…` file the two guides name, in the order they name it.
fn named_in(guide: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for line in guide.lines() {
        let mut rest = line;
        while let Some(at) = rest.find("examples/") {
            rest = &rest[at + "examples/".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
                .collect();
            // A sentence ending in the file name takes the full stop with it.
            let name = name.trim_end_matches('.').to_owned();
            if name.contains('.') {
                let path = root().join("examples").join(&name);
                if !found.contains(&path) {
                    found.push(path);
                }
            }
        }
    }
    found
}

/// **D12's exit criterion, in the part a test can hold**: a guide that names a file which has
/// moved fails the build rather than the reader.
///
/// What this does not cover, and a reader should therefore treat as prose: whether the *output*
/// shown beside a command is the output it produces today. The commands themselves are ordinary
/// verbs with their own tests in `cli.rs`; the two examples both guides are built around are
/// asserted there, cell by cell.
#[test]
fn the_projection_guide_names_files_that_exist() {
    let named = named_in(PROJECTION_GUIDE);
    assert!(!named.is_empty(), "the guide names no example at all");
    for path in named {
        assert!(path.exists(), "doc/projection-guide.md names {path:?}");
    }
}

/// **D14's exit criterion**: every script the generator's guide names is a file under
/// `examples/` that this test suite builds.
#[test]
fn the_generator_guide_names_scripts_that_build() {
    let named = named_in(GENERATOR_GUIDE);
    assert!(!named.is_empty(), "the guide names no example at all");
    let dir = std::env::temp_dir().join(format!("grind-guide-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a sandbox");
    for path in named {
        assert!(path.exists(), "doc/generator-guide.md names {path:?}");
        // A `.d.rhai` is a *definition* file — the vocabulary an editor reads, in Rhai's own
        // format for the purpose. It is not a script and does not run, which is why the guide
        // names it under "where to go next" rather than under an example.
        let definitions = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".d.rhai"));
        if !definitions && path.extension().is_some_and(|kind| kind == "rhai") {
            let out = dir.join("out.fods");
            let built = Command::new(env!("CARGO_BIN_EXE_grind"))
                .args(["build", &path.display().to_string(), "-o"])
                .arg(&out)
                .output()
                .expect("the binary runs");
            assert!(
                built.status.success(),
                "doc/generator-guide.md names {path:?}, which no longer builds: {}",
                String::from_utf8_lossy(&built.stderr)
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
