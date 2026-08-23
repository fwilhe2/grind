// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Write it, read it back, get the same document.
//!
//! Loop C proper — the differential against LibreOffice — needs `soffice`, and is
//! `sheet/tests/roundtrip.rs`'s job for spreadsheets. This is the half that needs nothing
//! installed and catches the whole class of bug where the writer emits something the reader
//! cannot get back: a lost space run, a list whose nesting did not reconstruct, an escape that
//! escaped twice.
//!
//! It is a *semantic* round trip, not a byte one. The writer regenerates (R6's splice is not
//! built for text yet), so the bytes of a document read and written back are this writer's
//! rather than the original's — but what they *mean* must survive exactly.

use grind_text::model::{Block, BlockKind, Document, Run};
use grind_text::{Form, odf};

fn text(s: &str) -> Run {
    Run::Text {
        text: s.to_owned(),
        style: None,
        href: None,
    }
}

/// Build a document from `(kind, runs)` pairs.
fn build(spec: Vec<(BlockKind, Vec<Run>)>) -> Document {
    let mut doc = Document::new();
    for (kind, runs) in spec {
        let id = doc.next_id();
        let mut block = Block::new(id, kind);
        block.runs = runs;
        doc.blocks.push(block);
    }
    doc.reindex_bookmarks();
    doc
}

/// Write, read back, and compare what the two documents *mean*.
///
/// Ids are minted fresh by the reader, so they are excluded deliberately: an id is this
/// build's handle on a block, not something the file carries.
fn roundtrip(doc: &Document, form: Form) -> Document {
    let bytes = odf::write(doc, form).expect("writes");
    let back = odf::read(&bytes).expect("reads back what it just wrote");
    assert_eq!(
        back.blocks.len(),
        doc.blocks.len(),
        "block count survived\n--- wrote ---\n{}",
        String::from_utf8_lossy(&bytes)
    );
    for (a, b) in doc.blocks.iter().zip(&back.blocks) {
        assert_eq!(
            (&a.kind, a.text(), &a.style),
            (&b.kind, b.text(), &b.style),
            "block survived\n--- wrote ---\n{}",
            String::from_utf8_lossy(&bytes)
        );
    }
    back
}

fn both_forms(doc: &Document) {
    roundtrip(doc, Form::Flat);
    roundtrip(doc, Form::Package);
}

#[test]
fn paragraphs_and_headings_survive_both_forms() {
    let doc = build(vec![
        (BlockKind::Heading { level: 1 }, vec![text("Title")]),
        (BlockKind::Paragraph, vec![text("A paragraph.")]),
        (BlockKind::Heading { level: 3 }, vec![text("Deep")]),
        (BlockKind::Paragraph, vec![]),
    ]);
    both_forms(&doc);
}

/// The one a naive writer gets wrong. XML collapses whitespace, so spaces written literally
/// come back missing — `doc/odt-format.md` §3.3.
#[test]
fn runs_of_spaces_survive_because_they_are_re_encoded() {
    for spelling in [
        "a    b",
        "  leading",
        "trailing  ",
        "  ",
        "a b c",
        "one  two   three",
        " ",
    ] {
        let doc = build(vec![(BlockKind::Paragraph, vec![text(spelling)])]);
        let back = roundtrip(&doc, Form::Flat);
        assert_eq!(
            back.blocks[0].text(),
            spelling,
            "{spelling:?} did not survive"
        );
    }
}

#[test]
fn a_nested_list_reconstructs_from_the_depths_the_model_flattened() {
    let doc = build(vec![
        (BlockKind::Paragraph, vec![text("before")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("one")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("two")]),
        (BlockKind::ListItem { depth: 2 }, vec![text("two a")]),
        (BlockKind::ListItem { depth: 2 }, vec![text("two b")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("three")]),
        (BlockKind::Paragraph, vec![text("after")]),
    ]);
    both_forms(&doc);
}

/// A jump of two levels at once has to open two elements, not one malformed one.
#[test]
fn a_list_that_jumps_two_levels_still_nests_properly() {
    let doc = build(vec![
        (BlockKind::ListItem { depth: 1 }, vec![text("one")]),
        (BlockKind::ListItem { depth: 3 }, vec![text("deep")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("back")]),
    ]);
    both_forms(&doc);
}

/// A document ending inside a list must still close every element it opened.
#[test]
fn a_document_ending_in_a_list_is_still_well_formed() {
    let doc = build(vec![
        (BlockKind::Paragraph, vec![text("intro")]),
        (BlockKind::ListItem { depth: 2 }, vec![text("last")]),
    ]);
    both_forms(&doc);
}

#[test]
fn tabs_breaks_bookmarks_and_links_survive() {
    let doc = build(vec![(
        BlockKind::Paragraph,
        vec![
            Run::Bookmark {
                name: "start".to_owned(),
            },
            text("see "),
            Run::Text {
                text: "the docs".to_owned(),
                style: None,
                href: Some("https://example.invalid/a?b=1&c=2".to_owned()),
            },
            Run::Tab,
            Run::Break,
            Run::Text {
                text: "emphasised".to_owned(),
                style: Some("Emph".to_owned()),
                href: None,
            },
        ],
    )]);
    let back = roundtrip(&doc, Form::Flat);
    assert_eq!(back.bookmarks.len(), 1, "the anchor came back");
    let hrefs: Vec<_> = back.blocks[0]
        .runs
        .iter()
        .filter_map(|r| match r {
            Run::Text { href: Some(h), .. } => Some(h.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        hrefs,
        vec!["https://example.invalid/a?b=1&c=2"],
        "an ampersand in a URL is escaped once, not twice"
    );
}

#[test]
fn markup_characters_in_text_are_escaped_rather_than_emitted() {
    let doc = build(vec![(
        BlockKind::Paragraph,
        vec![text("a < b & c > d \"quoted\" 'single'")],
    )]);
    let back = roundtrip(&doc, Form::Flat);
    assert_eq!(back.blocks[0].text(), "a < b & c > d \"quoted\" 'single'");
}

/// R3: nothing written that the document does not use.
#[test]
fn the_output_carries_only_the_boilerplate_it_needs() {
    let plain = build(vec![(BlockKind::Paragraph, vec![text("hi")])]);
    let bytes = odf::write(&plain, Form::Flat).expect("writes");
    let xml = String::from_utf8(bytes).expect("utf-8");
    assert!(
        !xml.contains("xmlns:xlink"),
        "a document with no link declares no xlink namespace:\n{xml}"
    );
    assert!(
        xml.lines().count() < 12,
        "a one-paragraph document should be a handful of lines, not a template:\n{xml}"
    );

    let linked = build(vec![(
        BlockKind::Paragraph,
        vec![Run::Text {
            text: "x".to_owned(),
            style: None,
            href: Some("https://x/".to_owned()),
        }],
    )]);
    let xml = String::from_utf8(odf::write(&linked, Form::Flat).expect("writes")).expect("utf-8");
    assert!(
        xml.contains("xmlns:xlink"),
        "and a document with one does:\n{xml}"
    );
}

/// The package form has to be a package: `mimetype` first, stored, byte-exact (§1.1).
#[test]
fn the_package_form_is_sniffable_as_a_text_document() {
    let doc = build(vec![(BlockKind::Paragraph, vec![text("hi")])]);
    let bytes = odf::write(&doc, Form::Package).expect("writes");
    assert_eq!(
        grind_text::kind(&bytes),
        Some(grind_text::DocumentKind::Text),
        "a reader must be able to tell what this is before parsing it"
    );
}

/// Writing does not change the document — the property every "save" depends on.
#[test]
fn writing_twice_produces_the_same_bytes() {
    let doc = build(vec![
        (BlockKind::Heading { level: 2 }, vec![text("H")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("a  b")]),
    ]);
    let once = odf::write(&doc, Form::Flat).expect("writes");
    let twice = odf::write(&doc, Form::Flat).expect("writes");
    assert_eq!(once, twice);

    // And reading-then-writing is stable: the second generation equals the first.
    let back = odf::read(&once).expect("reads");
    let again = odf::write(&back, Form::Flat).expect("writes");
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&again),
        "a document that has been through the reader writes identically"
    );
}
