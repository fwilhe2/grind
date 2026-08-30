// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `doc/projection-text.md` against the code, and against `doc/text-core.md`. **D2's §3.7.**
//!
//! The spreadsheet's twin of this file (`sheet/tests/projection_scope.rs`) had to invent its own
//! answer, because a spreadsheet turned out to have no element scope line to check against.
//! This one does not have to invent anything: `doc/text-core.md` **is** a scope line, and
//! `text/tests/scope.rs` already holds `grind_text::implemented()` to it. So the chain is
//! complete and mechanical, and it is the chain §3.7 described:
//!
//! ```text
//! doc/text-core.md  ←  scope.rs  →  grind_text::implemented()
//!         ↑                                   ↑
//!         └──── this file ────────────────────┘  →  doc/projection-text.md  →  write.rs
//! ```
//!
//! An element cannot enter the scope line without a projection spelling, and a projection node
//! cannot exist without the writer emitting it.
//!
//! **The one hole, named rather than papered over.** Check (2) reads node names out of
//! `write.rs` by looking for `out.begin("…")`, which is a *source pattern*: a writer that
//! computed a node name would slip past it. `the_corpus_emits_nothing_undocumented` is the
//! behavioural backstop — it reads the node names out of real projected documents instead of
//! out of the source — and between them the gap is small and stated.

use std::collections::BTreeSet;

const GRAMMAR: &str = include_str!("../../doc/projection-text.md");
const WRITER: &str = include_str!("../src/projection/write.rs");
const HEADER: &str = "grind text\n";

// --- reading the document ---

/// The cells of a markdown table row, or nothing if the line is not one.
fn cells(line: &str) -> Option<Vec<&str>> {
    let line = line.trim();
    if !line.starts_with('|') || line.starts_with("|---") {
        return None;
    }
    Some(
        line.trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>(),
    )
}

/// The first backticked token in a cell.
fn ticked(cell: &str) -> Option<&str> {
    let token = cell.split('`').nth(1)?;
    (!token.is_empty()).then_some(token)
}

/// The rows of `## The blocks` — `(node, example)`.
fn nodes() -> Vec<(&'static str, &'static str)> {
    section("## The blocks", "`style=` on any")
        .lines()
        .filter_map(|line| {
            let cells = cells(line)?;
            let node = ticked(cells.first()?)?;
            let example = ticked(cells.get(2)?)?;
            Some((node, example))
        })
        .collect()
}

/// The rows of `## The inline notation` — `(element, example)`, where the example is empty for
/// a `gap:` row.
fn inline() -> Vec<(&'static str, &'static str)> {
    section("## The inline notation", "The attribute form")
        .lines()
        .filter_map(|line| {
            let cells = cells(line)?;
            let element = ticked(cells.first()?)?;
            Some((element, cells.get(2).and_then(|c| ticked(c)).unwrap_or("")))
        })
        .collect()
}

/// Everything a row says about an element, for finding a `gap:` reason.
fn row_for(element: &str) -> Option<&'static str> {
    GRAMMAR.lines().find(|line| {
        cells(line)
            .and_then(|c| c.first().and_then(|first| ticked(first)))
            .is_some_and(|first| first == element)
    })
}

fn section(from: &str, to: &str) -> &'static str {
    let (_, rest) = GRAMMAR
        .split_once(from)
        .unwrap_or_else(|| panic!("doc/projection-text.md still has its `{from}` section"));
    rest.split_once(to)
        .unwrap_or_else(|| panic!("`{from}` still ends at `{to}`"))
        .0
}

// --- reading the code ---

/// Every node name the writer actually emits.
fn written() -> BTreeSet<&'static str> {
    WRITER
        .match_indices("out.begin(\"")
        .filter_map(|(at, m)| {
            let rest = &WRITER[at + m.len()..];
            rest.split('"').next()
        })
        .collect()
}

/// The node names in a projection's text: the first word of every line that is not blank, a
/// comment, or a closing brace.
fn emitted(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with('}'))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| *word != "grind")
        .map(str::to_owned)
        .collect()
}

// --- the checks ---

#[test]
fn every_documented_node_is_one_the_writer_emits() {
    let written = written();
    assert!(
        written.len() >= 3,
        "found {} node names in write.rs — the parse is broken, not the writer",
        written.len()
    );
    for (node, _) in nodes() {
        // `list` is the reader's alone, and the document says so in the same row.
        if node == "list" {
            continue;
        }
        assert!(
            written.contains(node),
            "doc/projection-text.md documents the node `{node}` and \
             text/src/projection/write.rs never emits one. Either it is a spelling the reader \
             takes and the writer does not — say so in its row, as `list` does — or the row is \
             a promise."
        );
    }
}

#[test]
fn every_node_the_writer_emits_is_documented() {
    let documented: BTreeSet<&str> = nodes().into_iter().map(|(node, _)| node).collect();
    for node in written() {
        assert!(
            documented.contains(node),
            "text/src/projection/write.rs emits `{node}` and doc/projection-text.md does not \
             document it. A node nobody wrote down is how a grammar and a model drift apart."
        );
    }
}

/// **The literal §3.7 check**, and the reason this file reads differently from its spreadsheet
/// twin: a text document has a scope line, so the projection can be held to it directly.
#[test]
fn every_element_in_scope_has_a_spelling() {
    let spelled: BTreeSet<&str> = inline().into_iter().map(|(element, _)| element).collect();
    let elements = grind_text::implemented();
    assert!(
        elements.len() >= 8,
        "grind_text::implemented() returned {} elements",
        elements.len()
    );
    for element in elements {
        // `text:p`, `text:h`, `text:list` and `text:list-item` are blocks, and their spellings
        // are the nodes rather than the notation. The mapping is by name and deliberately
        // hard-coded here: it is four rows, and a rule that derived it would be a third scope
        // line to keep in step.
        let block = match element {
            "text:p" => Some("p"),
            "text:h" => Some("h"),
            "text:list-item" => Some("li"),
            "text:list" => Some("list"),
            _ => None,
        };
        if let Some(node) = block {
            assert!(
                nodes().iter().any(|(name, _)| *name == node),
                "`{element}` is in doc/text-core.md's scope line and the projection has no \
                 `{node}` node"
            );
            continue;
        }
        assert!(
            spelled.contains(element),
            "`{element}` is in doc/text-core.md's scope line and doc/projection-text.md gives \
             it no spelling. §3.7: an element that enters the scope line without one fails the \
             build. Add a row — a notation, or `gap:` and the reason."
        );
    }
}

#[test]
fn every_gap_has_a_reason_and_no_example() {
    for (element, example) in inline() {
        let row = row_for(element).expect("the row it came from");
        let gap = row.contains("gap:");
        assert_eq!(
            gap,
            example.is_empty(),
            "`{element}`: a row is either a spelling with an example or a gap with a reason, \
             never both and never neither"
        );
        if gap {
            // Long enough to be a reason rather than a shrug — the same bar
            // `sheet/tests/projection_scope.rs` holds its gaps to.
            let reason = row.split("gap:").nth(1).unwrap_or_default();
            assert!(
                reason.len() > 20,
                "`{element}` is a gap with no reason on it: {reason:?}"
            );
        }
    }
}

#[test]
fn every_example_is_a_projection_that_reads() {
    for (name, example) in examples() {
        let source = format!("{HEADER}{example}\n");
        grind_text::projection::read(&source)
            .unwrap_or_else(|e| panic!("the example for `{name}` does not parse:\n{source}\n{e}"));
    }
}

/// Accepted is not carried. Read the example, project the model it produced, and what the row
/// claims has to still be there — a spelling the reader takes and then discards passes the
/// parse check and fails this one.
#[test]
fn every_example_still_holds_its_spelling_after_a_round_trip() {
    for (name, example) in examples() {
        let source = format!("{HEADER}{example}\n");
        let doc = grind_text::projection::read(&source).expect("it parses");
        let text = grind_text::projection::project(&doc).into_text();

        // The strongest form this check can take, and the one the writer earns by being
        // canonical: the example has to come back **exactly**. Every row is written in the
        // spelling the writer emits, so anything less than equality would let a notation the
        // reader quietly normalises away sit in the table looking supported.
        //
        // `list` is the one exception, and the document names it in the same row: it is a
        // *reading* spelling, re-emitted flat, so what has to survive is the depth it gave.
        if name == "list" {
            assert!(
                text.contains("li 1 "),
                "the `list` example lost its item's depth:\n{text}"
            );
            continue;
        }
        assert!(
            text.contains(example),
            "`{name}`'s example does not come back as it went in:\n--- in ---\n{source}\
             --- out ---\n{text}"
        );
    }
}

/// Every example in the document, block and inline alike, with the thing it is about.
fn examples() -> Vec<(&'static str, &'static str)> {
    nodes()
        .into_iter()
        .chain(inline())
        .filter(|(_, example)| !example.is_empty())
        .collect()
}

/// The behavioural backstop for check (2): node names read out of documents this build really
/// projected, rather than out of the source. It is what would catch a writer that computed a
/// node name instead of spelling one.
#[test]
fn the_vendored_corpus_emits_nothing_undocumented() {
    let documented: BTreeSet<&str> = nodes().into_iter().map(|(node, _)| node).collect();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut seen = BTreeSet::new();
    let mut documents = 0;
    for entry in std::fs::read_dir(&root).expect("tests/data").flatten() {
        let path = entry.path();
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(doc) = grind_text::read_bytes(&path.display().to_string(), &bytes) else {
            continue;
        };
        documents += 1;
        seen.extend(emitted(grind_text::projection::project(&doc).text()));
    }
    assert!(documents >= 2, "read {documents} vendored documents");
    for node in &seen {
        assert!(
            documented.contains(node.as_str()),
            "projecting the vendored corpus emits `{node}`, which doc/projection-text.md does \
             not document"
        );
    }
}
