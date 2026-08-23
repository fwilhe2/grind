// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **R6 for text documents**: writing changes as little of the XML as it can.
//!
//! `doc/suite.md` calls this differentiator #1 — *a word processor whose files live in git* —
//! and this file is what decides whether that sentence is true. The properties are the ones
//! `sheet/tests/kb.rs` holds the spreadsheet to:
//!
//! 1. Opening a document and saving it unchanged returns **exactly** its bytes. That is what
//!    makes reading a `.fodt` not show up as a commit.
//! 2. Editing one paragraph changes **one element**, and nothing else — indentation, unknown
//!    markup and other vendors' extensions included.
//! 3. Everything the model does not carry survives, precisely because the writer never
//!    regenerated it and therefore never had to understand it.
//! 4. The boundaries are asserted directly rather than left to be inferred. A fallback that
//!    fires silently would make the requirement untestable.

use grind_text::model::BlockKind;
use grind_text::{App, Form, odf};

/// A document with rather more in it than this build models: change tracking, a section, an
/// index, an annotation, a vendor's own element, and attributes on the paragraphs themselves.
///
/// That is the point. `doc/text-core.md` puts ten of `text-content`'s sixteen alternatives out
/// of scope, and R6 is the reason that is a defensible product decision rather than data loss.
const RICH: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:vendor="urn:example:vendor"
  office:mimetype="application/vnd.oasis.opendocument.text"
  office:version="1.4">
  <office:body>
    <office:text>
      <text:tracked-changes><text:changed-region text:id="c1"/></text:tracked-changes>
      <text:h text:outline-level="1" text:style-name="Heading_20_1" xml:id="h1">Title</text:h>
      <text:p text:style-name="Standard" text:class-names="a b" xml:id="p1">First paragraph.</text:p>
      <text:section text:name="s1"><text:p>Inside a section.</text:p></text:section>
      <text:table-of-content><text:index-body><text:p>TOC</text:p></text:index-body></text:table-of-content>
      <text:p vendor:tracking="7">Second paragraph.</text:p>
      <office:annotation><text:p>a comment</text:p></office:annotation>
    </office:text>
  </office:body>
</office:document>
"#;

fn open(bytes: &[u8]) -> App {
    let app = App::new();
    app.open_bytes("rich.fodt", bytes).expect("opens");
    app
}

/// Lines that differ between two documents, as a unified-ish list.
fn changed_lines(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let removed = a
        .iter()
        .filter(|l| !b.contains(l))
        .map(|l| (*l).to_owned())
        .collect();
    let added = b
        .iter()
        .filter(|l| !a.contains(l))
        .map(|l| (*l).to_owned())
        .collect();
    (removed, added)
}

/// The property a `.fodt` in version control cares about most: opening a document to look at
/// it must not show up as a commit.
#[test]
fn saving_an_unedited_document_returns_its_bytes_exactly() {
    let app = open(RICH.as_bytes());
    let out = app.save_bytes(Form::Flat).expect("saves");
    assert_eq!(
        String::from_utf8_lossy(&out),
        RICH,
        "an unedited save is not the same file"
    );
}

/// R6 proper.
#[test]
fn editing_one_paragraph_changes_one_line() {
    let app = open(RICH.as_bytes());
    // "First paragraph." is the third block: the heading, then it, then the one inside the
    // section — which this build reads past — then "Second paragraph.".
    let index = app
        .get_viewport(0..app.block_count())
        .iter()
        .position(|b| b.text == "First paragraph.")
        .expect("the paragraph is there");
    app.set_text(index, "First paragraph, edited.")
        .expect("edits");

    let out = String::from_utf8(app.save_bytes(Form::Flat).expect("saves")).expect("utf-8");
    let (removed, added) = changed_lines(RICH, &out);
    assert_eq!(removed.len(), 1, "removed: {removed:#?}");
    assert_eq!(added.len(), 1, "added: {added:#?}");
    assert!(added[0].contains("First paragraph, edited."), "{added:#?}");

    // Everything the model does not carry is still there, untouched.
    for kept in [
        "text:tracked-changes",
        "text:section",
        "text:table-of-content",
        "office:annotation",
        "vendor:tracking=\"7\"",
        "xmlns:vendor",
    ] {
        assert!(out.contains(kept), "{kept} was lost:\n{out}");
    }
}

/// The load-bearing half of the splice: a replaced element keeps the attributes the model does
/// not model. Dropping `xml:id` and `text:class-names` because the writer re-derived the start
/// tag would be a worse bug than a large diff.
#[test]
fn a_spliced_element_keeps_the_attributes_the_model_does_not_carry() {
    let app = open(RICH.as_bytes());
    let index = app
        .get_viewport(0..app.block_count())
        .iter()
        .position(|b| b.text == "First paragraph.")
        .expect("there");
    app.set_text(index, "the new content").expect("edits");

    let out = String::from_utf8(app.save_bytes(Form::Flat).expect("saves")).expect("utf-8");
    let line = out
        .lines()
        .find(|l| l.contains("the new content"))
        .expect("the edited line");
    assert!(line.contains("xml:id=\"p1\""), "{line}");
    assert!(line.contains("text:class-names=\"a b\""), "{line}");
    // And what the writer *does* produce is not doubled.
    assert_eq!(line.matches("text:style-name").count(), 1, "{line}");
}

/// Editing a heading has to keep its level and its own attributes too.
#[test]
fn editing_a_heading_keeps_its_level() {
    let app = open(RICH.as_bytes());
    app.set_text(0, "Retitled").expect("edits");
    let out = String::from_utf8(app.save_bytes(Form::Flat).expect("saves")).expect("utf-8");

    let (removed, added) = changed_lines(RICH, &out);
    assert_eq!(removed.len(), 1, "removed: {removed:#?}");
    assert_eq!(added.len(), 1, "added: {added:#?}");
    assert!(added[0].contains("text:outline-level=\"1\""), "{added:#?}");
    assert!(added[0].contains("xml:id=\"h1\""), "{added:#?}");
    // And it reads back as a heading.
    let back = open(out.as_bytes());
    assert_eq!(back.outline().len(), 1);
    assert_eq!(back.outline()[0].text, "Retitled");
}

// --- the boundaries, asserted rather than inferred -------------------------------------------

/// Inserting a block changes the **sequence**, which the file's structure is. Splicing that
/// means deciding where the bytes go and with what indentation, for a diff that is no longer
/// obviously smaller — so the document regenerates, and this test is what keeps that a stated
/// rule rather than an accident.
#[test]
fn inserting_a_block_regenerates_rather_than_splicing() {
    let app = open(RICH.as_bytes());
    app.insert(0, BlockKind::Paragraph, "new first")
        .expect("inserts");
    let out = String::from_utf8(app.save_bytes(Form::Flat).expect("saves")).expect("utf-8");

    assert!(out.contains("new first"));
    // The regenerating writer knows only what the model carries, so the unmodelled markup is
    // gone. That is the documented cost, and `doc/not-doing.md` carries the row.
    assert!(
        !out.contains("text:tracked-changes"),
        "a regenerate should not be claiming to preserve things:\n{out}"
    );
}

#[test]
fn deleting_a_block_regenerates_too() {
    let app = open(RICH.as_bytes());
    app.delete(0..1).expect("deletes");
    let out = String::from_utf8(app.save_bytes(Form::Flat).expect("saves")).expect("utf-8");
    assert!(!out.contains("text:section"), "regenerated, as documented");
}

/// A zip has no diff to preserve, so the package form always regenerates — and a document read
/// from one carries no source at all.
#[test]
fn the_package_form_never_splices() {
    let app = open(RICH.as_bytes());
    let packaged = app.save_bytes(Form::Package).expect("saves");
    let reopened = open(&packaged);
    // Reading it back and saving it flat cannot return the original bytes, because there were
    // none to keep.
    let flat = reopened.save_bytes(Form::Flat).expect("saves");
    assert_ne!(String::from_utf8_lossy(&flat), RICH);
    // But the content still survives the trip, which is what the round-trip tests cover.
    assert_eq!(reopened.outline().len(), 1);
}

/// A document this program authored has no file to splice into, and saving it must still work.
#[test]
fn a_new_document_has_nothing_to_splice_and_writes_anyway() {
    let app = App::new();
    app.insert(0, BlockKind::Heading { level: 1 }, "Fresh")
        .expect("inserts");
    let out = app.save_bytes(Form::Flat).expect("saves");
    let back = odf::read(&out).expect("reads");
    assert_eq!(back.blocks[0].text(), "Fresh");
}

/// Two edits in one document are two lines, not a rewrite — and they must come out in file
/// order however they were made.
#[test]
fn two_edits_are_two_lines_and_stay_in_file_order() {
    let app = open(RICH.as_bytes());
    let view = app.get_viewport(0..app.block_count());
    let second = view
        .iter()
        .position(|b| b.text == "Second paragraph.")
        .expect("there");
    let first = view
        .iter()
        .position(|b| b.text == "First paragraph.")
        .expect("there");

    // Edited last-first on purpose: the patch list has to sort by position, not by edit order.
    app.set_text(second, "Second, edited.").expect("edits");
    app.set_text(first, "First, edited.").expect("edits");

    let out = String::from_utf8(app.save_bytes(Form::Flat).expect("saves")).expect("utf-8");
    let (removed, added) = changed_lines(RICH, &out);
    assert_eq!(removed.len(), 2, "removed: {removed:#?}");
    assert_eq!(added.len(), 2, "added: {added:#?}");
    assert!(
        out.find("First, edited.") < out.find("Second, edited."),
        "the two edits landed out of order:\n{out}"
    );
}
