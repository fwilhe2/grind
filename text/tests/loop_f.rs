// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Loop F for text documents** — the projection differential (`doc/dsl.md` §8, milestone D2).
//!
//! For every document loop A already reads: project it, read the projection back, and assert
//! the two models are identical. It costs no corpus of its own, which is the point of building
//! it on loop A's, and it ratchets exactly as loop B's `FLOOR` does.
//!
//! Two halves, and the difference between them matters. The first runs over `text/tests/data/`
//! — documents LibreOffice Writer actually wrote, vendored here, globbed rather than listed —
//! and **never skips**. The second runs over `sw/qa`, needs a checkout, and says so when there
//! is none.
//!
//! **The one named gap is images**, and it is excluded by name rather than absorbed by the
//! ratchet. `doc/projection-text.md` has the reasoning: §3.8's answer is a sidecar directory,
//! and D4 made the projection a `Form`, which is reached through bytes and never a path.

use std::path::{Path, PathBuf};

use grind_text::model::{Block, Document, Run};
use grind_text::projection;

/// The vendored corpus: real Writer output, both physical forms of each file.
fn vendored() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut out = Vec::new();
    collect(&root, &mut out);
    out.sort();
    assert!(
        out.len() >= 2,
        "text/tests/data/ is globbed, and the glob found {} documents",
        out.len()
    );
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("odt") || e.eq_ignore_ascii_case("fodt"))
        {
            out.push(path);
        }
    }
}

#[test]
fn every_vendored_document_survives_being_projected_and_read_back() {
    for path in vendored() {
        let bytes = std::fs::read(&path).expect("a readable document");
        let original = grind_text::read_bytes(&path.display().to_string(), &bytes)
            .unwrap_or_else(|e| panic!("{}: loop A reads this: {e}", path.display()));

        let text = projection::project(&original).into_text();
        let back = projection::read(&text).unwrap_or_else(|e| {
            panic!("{}: its own projection will not parse: {e}", path.display())
        });

        let differences = differences(&original, &back);
        assert!(
            differences.is_empty(),
            "{} does not round-trip through the projection:\n  {}\n--- the projection ---\n{text}",
            path.display(),
            differences.join("\n  ")
        );
    }
}

/// The same property one layer out: through `write_bytes`/`read_bytes` rather than through
/// `projection::` directly, which is the door every shell and CLI verb goes through (D4).
#[test]
fn the_projection_is_a_form_like_the_other_two() {
    for path in vendored() {
        let bytes = std::fs::read(&path).expect("a readable document");
        let name = path.display().to_string();
        let original = grind_text::read_bytes(&name, &bytes).expect("loop A reads this");

        let written = grind_text::write_bytes(&original, grind_text::Form::Projection)
            .unwrap_or_else(|e| panic!("{name}: will not write as a projection: {e}"));
        let back = grind_text::read_bytes(&name, &written)
            .unwrap_or_else(|e| panic!("{name}: its own projection will not read back: {e}"));

        let differences = differences(&original, &back);
        assert!(
            differences.is_empty(),
            "{name} does not survive Form::Projection:\n  {}",
            differences.join("\n  ")
        );
    }
}

/// Projecting twice changes nothing — the writer is a function of the model and of nothing
/// else. Cheap, and it is what would catch a hash-ordered pool leaking into the output.
#[test]
fn re_projecting_changes_nothing() {
    for path in vendored() {
        let bytes = std::fs::read(&path).expect("a readable document");
        let doc = grind_text::read_bytes(&path.display().to_string(), &bytes).expect("reads");
        let once = projection::project(&doc).into_text();
        let twice = projection::project(&projection::read(&once).expect("reads")).into_text();
        assert_eq!(once, twice, "{}", path.display());
    }
}

/// **D3 for text — loop F at corpus scale.** LibreOffice's own `sw/qa`, which is loop A's
/// corpus, so it costs nothing to add.
///
/// A ratchet rather than a pass/fail, exactly like loop B's: the number below is what this
/// build achieves, it may be *raised* and never lowered, and the documents it does not manage
/// are listed by the scoreboard rather than hidden by it. Run
/// `cargo test -p grind-text --test loop_f -- --nocapture` to see them.
#[test]
fn the_corpus_projects() {
    const DEFAULT_CHECKOUT: &str = "/home/florian/code/github.com/LibreOffice/core";
    /// Documents that must survive the projection. **Raise it, never lower it.**
    const FLOOR: usize = 1755;

    let checkout = PathBuf::from(
        std::env::var("GRIND_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CHECKOUT.to_owned()),
    );
    let root = checkout.join("sw/qa");
    if !root.is_dir() {
        eprintln!(
            "skipping: no LibreOffice checkout at {DEFAULT_CHECKOUT}; \
             set GRIND_LO_CORPUS to its root to run loop F at corpus scale"
        );
        return;
    }

    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();

    let mut projected = 0usize;
    let mut unreadable = 0usize;
    let mut failures: Vec<(PathBuf, String)> = Vec::new();

    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        // Not loop F's business: a document loop A cannot read is loop A's scoreboard.
        let Ok(original) = grind_text::read_bytes(&path.display().to_string(), &bytes) else {
            unreadable += 1;
            continue;
        };
        let text = projection::project(&original).into_text();
        match projection::read(&text) {
            Err(e) => failures.push((path.clone(), format!("will not parse: {e}"))),
            Ok(back) => match differences(&original, &back) {
                d if d.is_empty() => projected += 1,
                d => failures.push((path.clone(), d.join("; "))),
            },
        }
    }

    eprintln!(
        "loop F (text): {} documents, {projected} project and read back identically, \
         {} loop A cannot read, {} differ",
        files.len(),
        unreadable,
        failures.len(),
    );
    for (path, why) in failures.iter().take(10) {
        eprintln!("  {}: {}", path.display(), truncate(why));
    }

    assert!(
        projected >= FLOOR,
        "loop F projects {projected} corpus documents, below the floor of {FLOOR}. \
         The floor is a ratchet: raise it when the number goes up, never lower it to \
         accommodate a regression."
    );
    assert!(
        failures.is_empty(),
        "{} corpus document(s) do not survive the projection. Each one is either a scope-line \
         gap — name it in doc/projection-text.md and exclude it here — or a bug in the \
         bijection.",
        failures.len()
    );
}

/// The gap, asserted rather than assumed. The day a paragraph's image comes back is the day
/// this test fails, and the fix is to delete it and stop excluding images from `differences`.
#[test]
fn images_are_the_one_named_gap() {
    let mut doc = Document::new();
    let id = doc.next_id();
    let mut block = Block::new(id, grind_text::BlockKind::Paragraph);
    block.runs = vec![
        Run::plain("before"),
        Run::Image {
            mime: "image/png".to_owned(),
            data: vec![1, 2, 3],
            width: Some("8cm".to_owned()),
            height: Some("5cm".to_owned()),
        },
        Run::plain("after"),
    ];
    doc.blocks.push(block);

    let back = projection::read(&projection::project(&doc).into_text()).expect("reads");
    assert!(
        !back.blocks[0]
            .runs
            .iter()
            .any(|r| matches!(r, Run::Image { .. })),
        "images project now — delete this test and stop excluding them from `differences`"
    );
    assert_eq!(
        back.blocks[0].text(),
        "beforeafter",
        "and the prose either side of it survives"
    );
}

/// The authoring spellings, which are the reader's whole tolerance: what a person types and
/// what the writer emits are two ways of saying one thing.
#[test]
fn the_authoring_spellings_mean_what_the_document_says_they_do() {
    let doc = projection::read(
        "grind text\n\
         // A comment, which the model has no room for and does not need one for.\n\
         li \"no depth given\"\n\
         list {\n    li \"in a list\"\n    list {\n        li \"deeper\"\n    }\n}\n",
    )
    .expect("reads");
    let depths: Vec<_> = doc
        .blocks
        .iter()
        .map(|b| match b.kind {
            grind_text::BlockKind::ListItem { depth } => depth,
            ref other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(depths, [1, 1, 2], "a `list` block is how deep it is nested");

    // And the writer answers in the flat spelling, because the model is flat.
    let text = projection::project(&doc).into_text();
    assert!(!text.contains("list {"), "{text}");
    assert!(text.contains("li 2 \"deeper\""), "{text}");
}

#[test]
fn the_span_map_finds_a_block_by_every_name_it_has() {
    let doc =
        projection::read("grind text\nh 1 \"One\"\nh 2 \"One point one\"\np \"{#intro}text\"\n")
            .expect("reads");
    let out = projection::project(&doc);

    // The same paragraph, three ways to ask for it — `loc.rs`'s whole vocabulary.
    let by_position = out.span_of("p3").expect("p3 is anchored");
    let by_bookmark = out.span_of("#intro").expect("#intro is anchored");
    assert_eq!(by_position, by_bookmark);
    assert!(out.text()[by_position.clone()].starts_with("p \""));

    let section = out.span_of("§1.1").expect("an outline path is an address");
    assert!(out.text()[section].starts_with("h 2 "));

    // And back: a byte in the paragraph's line reports an address for it.
    assert!(
        out.address_at(by_position.start + 1).is_some(),
        "the span map goes both ways"
    );
}

// --- comparison ---

/// Every difference between two documents, in the terms the projection is responsible for.
///
/// Blocks, their kind, their style name and their runs. Not `BlockId`s: an id is minted by
/// whichever `Document` handed it out and is a fact about a session, never about a document —
/// the same reason `Document::source` and `Document::edits` are not projected.
fn differences(a: &Document, b: &Document) -> Vec<String> {
    let mut out = Vec::new();
    if a.blocks.len() != b.blocks.len() {
        out.push(format!("{} blocks vs {}", a.blocks.len(), b.blocks.len()));
        return out;
    }
    for (i, (x, y)) in a.blocks.iter().zip(&b.blocks).enumerate() {
        block_differences(i, x, y, &mut out);
    }
    if a.bookmarks.len() != b.bookmarks.len() {
        out.push(format!(
            "{} bookmarks vs {}",
            a.bookmarks.len(),
            b.bookmarks.len()
        ));
    }
    out
}

fn block_differences(i: usize, a: &Block, b: &Block, out: &mut Vec<String>) {
    if a.kind != b.kind {
        out.push(format!("p{}: {:?} vs {:?}", i + 1, a.kind, b.kind));
    }
    if a.style != b.style {
        out.push(format!("p{}: style {:?} vs {:?}", i + 1, a.style, b.style));
    }
    let (x, y) = (spellable(a), spellable(b));
    if x != y {
        out.push(format!("p{}: runs {x:?} vs {y:?}", i + 1));
    }
}

/// A block's runs, with the one named gap taken out of both sides and both sides normalised.
///
/// **Filtering images from *both* sides** is what makes that an exclusion rather than a
/// blindfold: everything else about a paragraph containing an image is still compared, so
/// losing the words around one still fails.
///
/// **Normalising is not a loosening**, and this is the one thing in loop F worth reading twice.
/// `text/src/odf/write.rs` (its `text_content`, and the note above it) turns a literal `\t`
/// inside a `Run::Text` into `<text:tab/>` and a literal `\n` into `<text:line-break/>` — so in
/// *this model* a tab character in a run and a `Run::Tab` beside it are two spellings of one
/// document, and the ODF writer has always said so. The projection reads `\t` back as the
/// element, which makes it the normalising side of a difference that was already there. What
/// this function does is compare the two documents the way the ODF writer would see them, which
/// is the same move `is_the_same_document_the_odf_writer_would_write` proves once directly.
///
/// It reaches 59 corpus documents whose paragraphs hold raw XML indentation — the reader does
/// not collapse whitespace the way ODF §white-space does, which is a *reader* question and one
/// this loop is not the place to answer.
fn spellable(block: &Block) -> Vec<Run> {
    let mut runs = Vec::new();
    for run in &block.runs {
        match run {
            Run::Image { .. } => {}
            Run::Text {
                text,
                style,
                props,
                href,
            } if text.contains(['\t', '\n', '\r']) => {
                let mut rest = text.as_str();
                while let Some(i) = rest.find(['\t', '\n', '\r']) {
                    if i > 0 {
                        runs.push(Run::Text {
                            text: rest[..i].to_owned(),
                            style: style.clone(),
                            props: props.clone(),
                            href: href.clone(),
                        });
                    }
                    let tail = &rest[i..];
                    let (element, width) = match tail.as_bytes()[0] {
                        b'\t' => (Run::Tab, 1),
                        // `\r\n` is one break, exactly as the ODF writer counts it.
                        b'\r' if tail.as_bytes().get(1) == Some(&b'\n') => (Run::Break, 2),
                        _ => (Run::Break, 1),
                    };
                    runs.push(element);
                    rest = &tail[width..];
                }
                if !rest.is_empty() {
                    runs.push(Run::Text {
                        text: rest.to_owned(),
                        style: style.clone(),
                        props: props.clone(),
                        href: href.clone(),
                    });
                }
            }
            run => runs.push(run.clone()),
        }
    }
    grind_text::model::coalesce(&mut runs);
    runs
}

/// The claim `spellable` makes, proved directly rather than asserted in a comment: a literal
/// tab in a run and a `Run::Tab` beside it write the **same bytes**, so comparing them as equal
/// is comparing documents rather than excusing a difference.
#[test]
fn a_literal_tab_and_a_tab_run_are_the_same_document() {
    let build = |runs: Vec<Run>| {
        let mut doc = Document::new();
        let id = doc.next_id();
        let mut block = Block::new(id, grind_text::BlockKind::Paragraph);
        block.runs = runs;
        doc.blocks.push(block);
        grind_text::write_bytes(&doc, grind_text::Form::Flat).expect("writes")
    };
    assert_eq!(
        build(vec![Run::plain("a\tb\nc")]),
        build(vec![
            Run::plain("a"),
            Run::Tab,
            Run::plain("b"),
            Run::Break,
            Run::plain("c"),
        ]),
    );
}

fn truncate(text: &str) -> String {
    match text.char_indices().nth(160) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}
