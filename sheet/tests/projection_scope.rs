// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `doc/projection-sheet.md` against the projection — **§3.7's scope line, made mechanical.**
//!
//! `doc/dsl.md` §3.7 names the risk this file answers: *a hand-written grammar is a second
//! scope line, and two scope lines diverge.* The same mechanism `doc/small-group.md` uses
//! against `funcs::implemented()` and `doc/text-core.md` against `grind_text::implemented()`,
//! pointed at a serialisation rather than at a reader.
//!
//! §3.7 says the projection's vocabulary is checked "against the same two lists". For the
//! *text* projection that will be literal — `doc/text-core.md`'s element table is the
//! vocabulary, one node per element. For the spreadsheet it cannot be: a formula reaches
//! `formula::lex` as one verbatim string, so `doc/small-group.md`'s 110 functions are behind a
//! single `formula=` property and are not a vocabulary at all. What a spreadsheet has instead
//! of an element scope line is a **model**, and the model's fields are what grows. So the
//! spreadsheet's version of §3.7 is: every field of `Document` and `Sheet` has a node or a
//! named gap, checked against `sheet/src/model.rs` itself.
//!
//! Four checks, and each closes a different way the two can drift:
//!
//! | Check | Catches |
//! |---|---|
//! | the node table ⇄ `write.rs` | a node documented and never written, or written and never documented |
//! | every example parses | a grammar change nobody wrote down |
//! | every example keeps its node through a round trip | a node the reader *accepts* and then drops |
//! | the state table ⇄ `model.rs` | a new side table on `Sheet` with no spelling — the check that would have caught charts |
//!
//! **The one hole, named rather than papered over.** The writer's node names are extracted from
//! its source by pattern, so a node emitted through some *third* indirection would be invisible
//! to that half. `the_corpus_emits_nothing_undocumented` is the behavioural backstop: it
//! projects R7's fourteen vendored documents and reads the node names out of the text that
//! actually comes out. It is not complete either — it only sees what those documents exercise —
//! and between the two the gap is small and stated.

use std::collections::BTreeSet;

use grind_sheet::projection;

const GRAMMAR: &str = include_str!("../../doc/projection-sheet.md");
const WRITER: &str = include_str!("../src/projection/write.rs");
const MODEL: &str = include_str!("../src/model.rs");

/// The header a projection needs before any of these examples is a document.
const HEADER: &str = "grind spreadsheet\n";

// --- reading the grammar note ---

/// A markdown table row's cells, or `None` for a line that is not one.
///
/// Header rows and the `|---|---|` separators come back as cells too and are filtered by the
/// callers, which know what a real row of *their* table looks like.
fn cells(line: &str) -> Option<Vec<&str>> {
    let line = line.trim();
    if !line.starts_with('|') || !line.ends_with('|') {
        return None;
    }
    Some(line[1..line.len() - 1].split('|').map(str::trim).collect())
}

/// The text inside the first pair of backticks, if the cell is exactly one backticked run.
fn ticked(cell: &str) -> Option<&str> {
    let inner = cell.strip_prefix('`')?.strip_suffix('`')?;
    (!inner.contains('`')).then_some(inner)
}

/// Every `(node, example)` in the two node tables.
fn nodes() -> Vec<(&'static str, &'static str)> {
    let (section, _) = GRAMMAR
        .split_once("## The state")
        .expect("doc/projection-sheet.md still has its `The state` section");
    let (_, section) = section
        .split_once("## The nodes")
        .expect("doc/projection-sheet.md still has its `The nodes` section");
    section
        .lines()
        .filter_map(cells)
        .filter(|row| row.len() == 3)
        .filter_map(|row| Some((ticked(row[0])?, ticked(row[2])?)))
        .collect()
}

/// Every `(field, spelling)` in the state table. The spelling is a node name or `gap: …`.
fn state() -> Vec<(&'static str, &'static str)> {
    let (_, section) = GRAMMAR
        .split_once("## The state")
        .expect("doc/projection-sheet.md still has its `The state` section");
    let (section, _) = section
        .split_once("## What this document is not")
        .expect("doc/projection-sheet.md still has its closing section");
    section
        .lines()
        .filter_map(cells)
        .filter(|row| row.len() == 2)
        .filter_map(|row| Some((ticked(row[0])?, row[1])))
        .collect()
}

// --- reading the code ---

/// Every node name the writer emits, from its source.
///
/// Two shapes, because there are two: `out.begin("cell")`, and `long_node(out, "year", …)` for
/// the parts whose only attribute is `long`. See this module's note about what that misses.
fn written() -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for pattern in ["out.begin(\"", "long_node(out, \""] {
        let mut rest = WRITER;
        while let Some(at) = rest.find(pattern) {
            rest = &rest[at + pattern.len()..];
            if let Some(end) = rest.find('"') {
                out.insert(&rest[..end]);
            }
        }
    }
    out
}

/// The field names of one `pub struct` in `sheet/src/model.rs`, qualified as the grammar note
/// spells them.
fn fields(name: &str) -> Vec<String> {
    let (_, body) = MODEL
        .split_once(&format!("pub struct {name} {{"))
        .unwrap_or_else(|| panic!("sheet/src/model.rs still declares `pub struct {name}`"));
    let (body, _) = body
        .split_once("\n}")
        .expect("a struct declaration ends at a closing brace in column zero");
    body.lines()
        .filter_map(|line| {
            // A field is `    name: Type,` or `    pub name: Type,`. Attributes, doc comments
            // and comments all fail one of these, which is why this is a filter and not a
            // parser: nothing else in these two declarations has that shape.
            let line = line.trim();
            let line = line.strip_prefix("pub ").unwrap_or(line);
            let (field, _) = line.split_once(": ")?;
            field
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_')
                .then(|| format!("{name}::{field}"))
        })
        .collect()
}

/// Every node name in a projection's text — the first word of every line that is not a
/// comment, a closing brace or blank.
fn emitted(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with('}'))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| *word != "grind" && !word.starts_with('{'))
        .map(str::to_owned)
        .collect()
}

// --- the checks ---

#[test]
fn every_documented_node_is_one_the_writer_emits() {
    let written = written();
    // A parser that matched nothing would pass vacuously and quietly retire the check.
    assert!(
        written.len() >= 20,
        "only found {} node names in write.rs — the extraction is broken, not the writer",
        written.len()
    );
    for (node, _) in nodes() {
        assert!(
            written.contains(node),
            "doc/projection-sheet.md documents `{node}` and sheet/src/projection/write.rs \
             never emits it. A documented node nothing writes is a promise, which is what \
             this document is not."
        );
    }
}

#[test]
fn every_node_the_writer_emits_is_documented() {
    let documented: BTreeSet<&str> = nodes().into_iter().map(|(node, _)| node).collect();
    assert!(documented.len() >= 20, "the grammar table did not parse");
    for node in written() {
        assert!(
            documented.contains(node),
            "sheet/src/projection/write.rs emits `{node}` and doc/projection-sheet.md does \
             not document it. This is the drift doc/dsl.md §3.7 exists to catch: add a row \
             with what it carries and an example."
        );
    }
}

#[test]
fn every_example_is_a_projection_that_reads() {
    let nodes = nodes();
    assert!(nodes.len() >= 20, "the grammar table did not parse");
    for (node, example) in nodes {
        let source = format!("{HEADER}{example}\n");
        projection::read(&source).unwrap_or_else(|e| {
            panic!(
                "doc/projection-sheet.md's example for `{node}` no longer reads: {e}\n  \
                 {example}"
            )
        });
    }
}

#[test]
fn every_example_still_holds_its_node_after_a_round_trip() {
    // The check that separates *accepted* from *carried*. A node the reader parses and then
    // throws away passes `every_example_is_a_projection_that_reads` and fails here, because
    // projecting the model it produced would not put the node back.
    for (node, example) in nodes() {
        let source = format!("{HEADER}{example}\n");
        let doc = projection::read(&source).expect("the example reads");
        let back = projection::project(&doc).into_text();
        assert!(
            emitted(&back).contains(node),
            "doc/projection-sheet.md's example for `{node}` reads, and projecting what it \
             produced does not put `{node}` back — so the node is accepted and dropped.\n  \
             {example}\n--- came back as ---\n{back}"
        );
    }
}

#[test]
fn every_field_of_the_model_has_a_node_or_a_named_gap() {
    let state = state();
    let documented: BTreeSet<&str> = state.iter().map(|(field, _)| *field).collect();
    assert_eq!(
        documented.len(),
        state.len(),
        "a field is listed twice in doc/projection-sheet.md's state table"
    );

    let model: Vec<String> = fields("Document")
        .into_iter()
        .chain(fields("Sheet"))
        .collect();
    assert!(
        model.len() >= 18,
        "only found {} fields in sheet/src/model.rs — the extraction is broken",
        model.len()
    );
    for field in &model {
        assert!(
            documented.contains(field.as_str()),
            "`{field}` is state a spreadsheet carries and doc/projection-sheet.md does not \
             say how the projection spells it. Give it a node, or a `gap:` row with the \
             reason — silently dropping it is how a bijection stops being one."
        );
    }
    for (field, _) in &state {
        assert!(
            model.contains(&(*field).to_owned()),
            "doc/projection-sheet.md's state table names `{field}`, which sheet/src/model.rs \
             no longer has. Drop the row rather than leaving a standing claim."
        );
    }
}

#[test]
fn every_node_a_field_claims_is_a_node_that_exists() {
    // The state table's other half: a field pointing at a node the grammar does not define
    // would read as covered while covering nothing. A `gap:` row is exempt, and is required to
    // say *why* — an unexplained exemption is how a ratchet quietly stops ratcheting, which is
    // `doc/cli-parity-sheet.md`'s rule applied here.
    let documented: BTreeSet<&str> = nodes().into_iter().map(|(node, _)| node).collect();
    for (field, spelling) in state() {
        if let Some(reason) = spelling.strip_prefix("gap:") {
            assert!(
                reason.trim().len() > 20,
                "`{field}` is a gap with no reason worth the name"
            );
            continue;
        }
        let claimed: Vec<&str> = spelling.split_whitespace().filter_map(ticked).collect();
        assert!(
            !claimed.is_empty(),
            "`{field}`'s spelling names no node and is not a `gap:` — one or the other"
        );
        for node in claimed {
            assert!(
                documented.contains(node),
                "`{field}` claims to be spelled `{node}`, which the node table does not define"
            );
        }
    }
}

#[test]
fn the_corpus_emits_nothing_undocumented() {
    // The behavioural backstop for the one hole this module's note names: the writer's node
    // names are extracted by pattern, so a node emitted through a third indirection would be
    // invisible to that half. This half never looks at the source — it reads the node names out
    // of text that really came out of the writer, over R7's fourteen documents.
    let documented: BTreeSet<&str> = nodes().into_iter().map(|(node, _)| node).collect();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut seen = BTreeSet::new();
    let mut checked = 0;
    for dir in ["kb", "samples"] {
        for entry in std::fs::read_dir(root.join(dir)).expect("the vendored corpus is there") {
            let path = entry.expect("a readable directory entry").path();
            if !path.extension().is_some_and(|e| e == "fods" || e == "ods") {
                continue;
            }
            let doc = grind_sheet::read_file(&path).expect("loop A reads this");
            seen.extend(emitted(projection::project(&doc).text()));
            checked += 1;
        }
    }
    assert!(
        checked >= 14,
        "only {checked} documents — the corpus is missing"
    );
    for node in &seen {
        assert!(
            documented.contains(node.as_str()),
            "projecting R7's corpus emits `{node}`, which doc/projection-sheet.md does not \
             document"
        );
    }
    // And the corpus is worth having as a check: it must exercise a real share of the grammar,
    // or "nothing undocumented came out" is only saying that nothing came out.
    assert!(
        seen.len() >= 10,
        "R7's corpus only exercised {} of the {} nodes; this check has gone vacuous",
        seen.len(),
        documented.len()
    );
}
