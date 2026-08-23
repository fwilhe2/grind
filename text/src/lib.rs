// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ODF word processor core. **\[ODT\]**
//!
//! The second document type of the suite (`doc/suite.md`, phase 10). It builds on
//! `grind-core` exactly as `grind-sheet` does — same packaging, same namespace resolution,
//! same tolerant element-context stack — and adds the one thing that is its own: the
//! `office:text` content model.
//!
//! Two documents are normative here and are worth reading before the code:
//!
//! * **`doc/odt-format.md`** — the clean-room notes. Every structural claim cites
//!   `doc/OpenDocument-v1.4-schema.rng` by line, and §5 is a list of things about LibreOffice
//!   that are **not yet verified** and may not be implemented until they are.
//! * **`doc/text-core.md`** — the scope line. Unlike `doc/small-group.md` it was *invented*
//!   rather than extracted, because ODF defines no evaluator tier for text — which is exactly
//!   why [`implemented`] is checked against it by `tests/scope.rs` rather than trusted.
//!
//! **Where this build is.** S4–S5: the model, addressing, the reader and a regenerating
//! writer. No R6 splicing yet, and no `App` (S6).

pub mod loc;
pub mod model;
pub mod odf;

pub use grind_core::{DocumentKind, Form, Observer, Result, kind};
pub use loc::Loc;
pub use model::{Block, BlockId, BlockKind, Document, Run};

use std::path::Path;

/// Every element of the ODF text content model this build understands.
///
/// The mechanical half of `doc/text-core.md`, and the anti-bloat rule made checkable: adding
/// an element to the reader without adding it to that document fails the build, and so does
/// listing one there that nothing implements (`tests/scope.rs`).
///
/// This is `grind_sheet::formula::funcs::implemented()`'s counterpart, and it matters more:
/// the spreadsheet's scope line could be *extracted* from a normative tier, so drifting from
/// it would contradict a specification. Text has no such tier, so the only thing between this
/// list and a wish list is the test.
pub fn implemented() -> Vec<&'static str> {
    vec![
        // Block level
        "text:p",
        "text:h",
        "text:list",
        "text:list-item",
        // Inline
        "text:span",
        "text:s",
        "text:tab",
        "text:line-break",
        "text:a",
        "text:bookmark",
    ]
}

/// Read a `.odt` (package) or `.fodt` (flat) document from bytes.
///
/// Paired with [`read_file`] from the start because the browser has no filesystem, and this
/// is not retrofittable later (doc/plan.md, rule 5).
///
/// The form is sniffed from the bytes, so `name` is only ever a label for diagnostics.
pub fn read_bytes(_name: &str, bytes: &[u8]) -> Result<Document> {
    odf::read(bytes)
}

pub fn read_file(path: &Path) -> Result<Document> {
    read_bytes(&path.display().to_string(), &std::fs::read(path)?)
}

/// Serialise a document. See [`Form`].
pub fn write_bytes(doc: &Document, form: Form) -> Result<Vec<u8>> {
    odf::write(doc, form)
}

/// Write a document, choosing the form from the extension — `.fodt` flat, anything else the
/// package form ([`Form::from_path`]).
pub fn write_file(doc: &Document, path: &Path) -> Result<()> {
    std::fs::write(path, write_bytes(doc, Form::from_path(path))?)?;
    Ok(())
}

/// Read a document, refusing one that is not a text document.
///
/// The reader is tolerant by construction (§8), so handing it a spreadsheet produces an empty
/// text document rather than an error — which is exactly wrong for a user who opened the
/// wrong file. [`grind_core::kind`] is checked first, and the error names the app that does
/// open it.
pub fn open_bytes(name: &str, bytes: &[u8]) -> Result<Document> {
    match kind(bytes) {
        Some(DocumentKind::Text) => read_bytes(name, bytes),
        other => Err(grind_core::Error::UnsupportedKind(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat text document with `body` as its `office:text` content.
    fn fodt(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  office:mimetype="application/vnd.oasis.opendocument.text"
  office:version="1.4">
  <office:body><office:text>{body}</office:text></office:body>
</office:document>"#
        )
    }

    fn read(body: &str) -> Document {
        odf::read(fodt(body).as_bytes()).expect("the document parses")
    }

    #[test]
    fn paragraphs_and_headings_come_back_in_order() {
        let doc = read(
            r#"<text:h text:outline-level="1">Title</text:h>
               <text:p>First.</text:p>
               <text:p>Second.</text:p>"#,
        );
        assert_eq!(doc.blocks.len(), 3);
        assert_eq!(doc.blocks[0].outline_level(), Some(1));
        assert_eq!(doc.blocks[0].text(), "Title");
        assert_eq!(doc.blocks[1].text(), "First.");
        assert_eq!(doc.text(), "Title\nFirst.\nSecond.");
    }

    #[test]
    fn text_s_is_expanded_because_xml_would_otherwise_lose_the_spaces() {
        // rng:8408 — ODF's run-length encoding of spaces. The trap that
        // table:number-columns-repeated is, wearing different clothes.
        let doc = read(r#"<text:p>a<text:s text:c="4"/>b<text:s/>c</text:p>"#);
        assert_eq!(doc.blocks[0].text(), "a    b c");
    }

    #[test]
    fn a_repeat_count_is_clamped_rather_than_believed() {
        // §9: never trust the file's number. Four billion spaces is a memory-exhaustion
        // vector, not an intent.
        let doc = read(r#"<text:p><text:s text:c="4000000000"/></text:p>"#);
        assert!(doc.blocks[0].len() <= 4096);
    }

    #[test]
    fn nested_spans_compose_their_style_names() {
        // doc/text-core.md: the model is flat, so reading composes the stack down each branch.
        let doc = read(
            r#"<text:p>plain <text:span text:style-name="B">bold \
<text:span text:style-name="I">both</text:span></text:span></text:p>"#,
        );
        let runs = &doc.blocks[0].runs;
        let styled: Vec<_> = runs
            .iter()
            .filter_map(|r| match r {
                Run::Text { text, style, .. } => Some((text.as_str(), style.as_deref())),
                _ => None,
            })
            .collect();
        assert_eq!(styled[0].1, None, "text outside any span carries no style");
        assert_eq!(styled[1].1, Some("B"));
        assert_eq!(styled[2].1, Some("B I"), "outermost first, both kept");
    }

    #[test]
    fn a_list_flattens_into_blocks_that_know_their_depth() {
        let doc = read(
            r#"<text:list><text:list-item><text:p>one</text:p></text:list-item>
                 <text:list-item><text:p>two</text:p>
                   <text:list><text:list-item><text:p>two a</text:p></text:list-item></text:list>
                 </text:list-item></text:list>"#,
        );
        let kinds: Vec<_> = doc.blocks.iter().map(|b| b.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::ListItem { depth: 1 },
                BlockKind::ListItem { depth: 1 },
                BlockKind::ListItem { depth: 2 },
            ]
        );
        assert_eq!(doc.blocks[2].text(), "two a");
    }

    #[test]
    fn tabs_and_breaks_are_elements_rather_than_characters() {
        let doc = read(r#"<text:p>a<text:tab/>b<text:line-break/>c</text:p>"#);
        assert!(matches!(doc.blocks[0].runs[1], Run::Tab));
        assert_eq!(doc.blocks[0].text(), "a\tb\nc");
    }

    #[test]
    fn a_hyperlink_carries_its_target_onto_the_text_inside_it() {
        let doc = read(r#"<text:p>see <text:a xlink:href="https://x/">here</text:a>.</text:p>"#);
        let hrefs: Vec<_> = doc.blocks[0]
            .runs
            .iter()
            .filter_map(|r| match r {
                Run::Text { text, href, .. } => Some((text.as_str(), href.as_deref())),
                _ => None,
            })
            .collect();
        assert_eq!(hrefs[0], ("see ", None));
        assert_eq!(hrefs[1], ("here", Some("https://x/")));
        assert_eq!(hrefs[2], (".", None), "the href closes with the element");
    }

    #[test]
    fn a_bookmark_is_indexed_and_contributes_no_text() {
        let doc = read(r#"<text:p><text:bookmark text:name="intro"/>Hello</text:p>"#);
        assert_eq!(doc.blocks[0].text(), "Hello");
        assert_eq!(doc.bookmarks.get("intro"), Some(&doc.blocks[0].id));
    }

    #[test]
    fn everything_outside_the_scope_line_is_inert_rather_than_an_error() {
        // §8's whole design, arriving unchanged in a second document type. None of these cost
        // a line of code in the reader, and the paragraphs around them still read.
        let doc = read(
            r#"<text:p>before</text:p>
               <text:section text:name="s"><text:p>inside a section</text:p></text:section>
               <text:table-of-content><text:index-body><text:p>TOC</text:p></text:index-body></text:table-of-content>
               <text:bibliography/>
               <office:annotation><text:p>a comment</text:p></office:annotation>
               <text:p>after</text:p>"#,
        );
        assert_eq!(
            doc.text(),
            "before\nafter",
            "an unrecognised subtree is swallowed whole, contents included"
        );
    }

    #[test]
    fn an_unbounded_outline_level_loads_because_the_schema_permits_one() {
        // rng:6867 — positiveInteger, no ceiling. Tolerance on the way in (R5).
        let doc = read(r#"<text:h text:outline-level="9">deep</text:h>"#);
        assert_eq!(doc.blocks[0].outline_level(), Some(9));
        // A heading that says it is a heading is one, whatever else it fails to say.
        let doc = read(r#"<text:h>no level</text:h>"#);
        assert_eq!(doc.blocks[0].outline_level(), Some(1));
    }

    #[test]
    fn the_prefix_carries_no_meaning() {
        // §8.1: dispatch is on the URI. The same document under different prefixes reads the
        // same, and this is the second document type proving it.
        let odd = r#"<?xml version="1.0" encoding="UTF-8"?>
<ns0:document xmlns:ns0="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:zz="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  ns0:mimetype="application/vnd.oasis.opendocument.text">
  <ns0:body><ns0:text><zz:p>hello</zz:p></ns0:text></ns0:body>
</ns0:document>"#;
        let doc = odf::read(odd.as_bytes()).expect("parses");
        assert_eq!(doc.text(), "hello");
    }

    #[test]
    fn a_spreadsheet_is_refused_rather_than_read_as_an_empty_document() {
        let ods = br#"<?xml version="1.0"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  office:mimetype="application/vnd.oasis.opendocument.spreadsheet"/>"#;
        let err = open_bytes("book.ods", ods).expect_err("not a text document");
        assert!(
            err.to_string().contains("grind sheet"),
            "the error names the app that does open it: {err}"
        );
    }
}
