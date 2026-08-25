// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind_text::implemented()` against `doc/text-core.md` — the scope line made mechanical.
//!
//! The same mechanism `doc/small-group.md` uses against `funcs::implemented()`, and it matters
//! **more** here. The spreadsheet's scope line was extracted from a normative tier
//! (OpenFormula §2.3.2), so drifting from it would contradict a specification and somebody
//! would eventually notice. ODF defines no such tier for text documents, so `doc/text-core.md`
//! is a product decision — and the only thing standing between a product decision and a wish
//! list is a test that fails.

const SCOPE: &str = include_str!("../../doc/text-core.md");

/// The element a table row is *about* — its first backticked token, and only if that token
/// names an element of a document body.
///
/// **First token only**, deliberately. A row's later columns explain it, and an explanation
/// mentions other elements: "a second numbering mechanism beside `text:list`" is a reason, not
/// a listing, and reading it as one made this test claim `text:list` was out of scope while
/// the reader implemented it.
///
/// **Element prefixes only**, for the same shape of reason: the styles table in the same
/// section has rows about `style:family` values, which are not elements and are not what
/// `implemented()` enumerates.
fn subject(line: &'static str) -> Option<&'static str> {
    if !line.trim_start().starts_with('|') {
        return None;
    }
    let name = line.split('`').nth(1)?;
    (name.starts_with("text:") || name.starts_with("table:") || name.starts_with("draw:"))
        .then_some(name)
}

/// Every element the document lists as **In**.
fn in_scope() -> Vec<&'static str> {
    let (section, _) = SCOPE
        .split_once("## Not yet")
        .expect("doc/text-core.md still has its `Not yet` section");
    let (_, section) = section
        .split_once("## In — the elements this build models")
        .expect("doc/text-core.md still has its `In` section");

    // `str::lines` borrows from a `&'static str`, so the items are 'static too.
    section.lines().filter_map(subject).collect()
}

/// Every element named in the **Not yet** and **Never** sections — what must stay unbuilt.
fn out_of_scope() -> Vec<&'static str> {
    let (_, rest) = SCOPE
        .split_once("## Not yet")
        .expect("doc/text-core.md still has its `Not yet` section");
    let (rest, _) = rest
        .split_once("## What \"preserved\" means")
        .expect("doc/text-core.md still has its closing sections");

    rest.lines().filter_map(subject).collect()
}

#[test]
fn every_implemented_element_is_in_the_scope_document() {
    let scope = in_scope();
    // A parser that matched nothing would pass vacuously and quietly retire the check.
    assert!(
        scope.len() >= 8,
        "only found {} elements in doc/text-core.md — the parse is broken, not the document",
        scope.len()
    );

    for element in grind_text::implemented() {
        assert!(
            scope.contains(&element),
            "grind_text::implemented() has `{element}`, which doc/text-core.md does not list \
             as in scope. Add it there with its schema citation, or stop reading it."
        );
    }
}

#[test]
fn every_element_in_the_scope_document_is_implemented() {
    let implemented = grind_text::implemented();
    for element in in_scope() {
        assert!(
            implemented.contains(&element),
            "doc/text-core.md lists `{element}` as in scope and nothing implements it. \
             A scope line that promises is a backlog, which is what this document is not."
        );
    }
}

#[test]
fn nothing_deferred_or_refused_is_implemented() {
    // The half that keeps the line a line. `doc/text-core.md`'s "Not yet" and "Never" sections
    // are the boundary, and an element quietly appearing in both places is how a boundary
    // stops being one.
    let implemented = grind_text::implemented();
    for element in out_of_scope() {
        assert!(
            !implemented.contains(&element),
            "`{element}` is implemented but doc/text-core.md still lists it as deferred or \
             refused — move the row into the `In` section, by the rule that document states"
        );
    }
}

#[test]
fn the_scope_line_is_the_reader_and_not_a_second_list() {
    // `implemented()` is prose until something ties it to behaviour: a list that names an
    // element the reader ignores is worse than no list, because it reads as a guarantee.
    // Every element in it must actually change what a document parses to.
    let doc = grind_text::odf::read(
        br#"<?xml version="1.0"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  office:mimetype="application/vnd.oasis.opendocument.text">
  <office:body><office:text>
    <text:h text:outline-level="2">H</text:h>
    <text:p>p<text:tab/><text:line-break/><text:s text:c="2"/>
      <text:span text:style-name="S">s</text:span>
      <text:bookmark text:name="b"/></text:p>
    <text:list><text:list-item><text:p>li</text:p></text:list-item></text:list>
  </office:text></office:body>
</office:document>"#,
    )
    .expect("parses");

    assert_eq!(doc.blocks.len(), 3, "text:h, text:p and text:list-item");
    assert_eq!(doc.blocks[0].outline_level(), Some(2), "text:h");
    assert!(doc.blocks[0].text() == "H");
    assert!(doc.blocks[1].text().contains('\t'), "text:tab");
    assert!(doc.blocks[1].text().contains('\n'), "text:line-break");
    assert!(doc.blocks[1].text().contains("  "), "text:s");
    assert_eq!(doc.bookmarks.len(), 1, "text:bookmark");
    assert!(
        matches!(
            doc.blocks[2].kind,
            grind_text::BlockKind::ListItem { depth: 1 }
        ),
        "text:list and text:list-item"
    );
}
