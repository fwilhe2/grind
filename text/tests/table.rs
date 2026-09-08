// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tables — the second axis of the flat block sequence. **\[ODT\]**
//!
//! `doc/text-core.md`'s Tables section, made behavioural. Everything here is about the one
//! decision the model makes: a cell is a **coordinate carried beside the block's kind**, not a
//! kind of its own, because `table-table-cell-content` is `zeroOrMore text-content`
//! (rng:16126) — the body's own production — so a cell holds paragraphs, headings and lists
//! exactly as the body does.
//!
//! What that buys is checked here too, and it is the interesting half: **nothing else had to
//! change.** `p12` is the twelfth block whether it is in a table or not, so the caret
//! operations, `App::set_char_style`, bookmarks and the outline all reach inside a cell without
//! knowing tables exist. A test that a *table* survives is a test that the reader and writer
//! agree; the tests at the end are the ones that say the model was shaped right.

use grind_text::model::{BlockKind, Document, Run};
use grind_text::{App, Caret, Form, odf};

/// A flat ODT with `body` between the usual wrappers, declaring the namespaces a table needs.
fn fodt(body: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:body><office:text>{body}</office:text></office:body>
</office:document>"#
    )
    .into_bytes()
}

/// Every block as `(text, table!rowxcolumn)`, which is the whole of what these tests assert.
fn cells(doc: &Document) -> Vec<(String, String)> {
    doc.blocks
        .iter()
        .map(|block| {
            let cell = match &block.cell {
                Some(cell) => format!("{}!r{}c{}", cell.table, cell.row, cell.column),
                None => String::new(),
            };
            (block.text(), cell)
        })
        .collect()
}

#[test]
fn a_cell_holds_blocks_and_a_block_knows_which_cell_it_is_in() {
    let doc = odf::read(&fodt(
        r#"<text:p>before</text:p>
        <table:table table:name="Prices">
         <table:table-column table:number-columns-repeated="2"/>
         <table:table-row>
          <table:table-cell><text:p>Item</text:p></table:table-cell>
          <table:table-cell><text:h text:outline-level="3">Price</text:h></table:table-cell>
         </table:table-row>
         <table:table-row>
          <table:table-cell><text:p>Oak</text:p><text:p>second paragraph</text:p></table:table-cell>
          <table:table-cell><text:list><text:list-item><text:p>a list</text:p></text:list-item></text:list></table:table-cell>
         </table:table-row>
        </table:table>
        <text:p>after</text:p>"#,
    ))
    .expect("parses");

    assert_eq!(
        cells(&doc),
        vec![
            ("before".to_owned(), String::new()),
            ("Item".to_owned(), "Prices!r0c0".to_owned()),
            ("Price".to_owned(), "Prices!r0c1".to_owned()),
            ("Oak".to_owned(), "Prices!r1c0".to_owned()),
            // Two blocks in one cell — the cell is the coordinate, not the container.
            ("second paragraph".to_owned(), "Prices!r1c0".to_owned()),
            ("a list".to_owned(), "Prices!r1c1".to_owned()),
            ("after".to_owned(), String::new()),
        ]
    );
    // A cell holds whatever the body holds, which is the schema's own claim (rng:16126) and
    // the reason `BlockKind` gained nothing.
    assert_eq!(doc.blocks[2].kind, BlockKind::Heading { level: 3 });
    assert_eq!(doc.blocks[5].kind, BlockKind::ListItem { depth: 1 });
}

/// **The normalisation that keeps reading idempotent**, and it is measured rather than chosen:
/// LibreOffice materialises a paragraph in an empty cell on every save (`doc/odt-format.md`
/// §5b), so a model that left the cell blockless would gain a block on the first round trip.
#[test]
fn an_empty_cell_is_one_empty_paragraph() {
    let doc = odf::read(&fodt(
        r#"<table:table table:name="T">
         <table:table-column/>
         <table:table-row><table:table-cell/></table:table-row>
        </table:table>"#,
    ))
    .expect("parses");
    assert_eq!(doc.blocks.len(), 1);
    assert!(doc.blocks[0].is_empty());
    assert_eq!(
        doc.blocks[0].cell.as_ref().map(|c| c.column),
        Some(0),
        "and it is in the cell, not beside it"
    );
}

/// `text:s`'s doctrine on two more axes: a repeat is run-length encoding, expanded on the way
/// in so that nothing downstream has to know it was ever written that way.
#[test]
fn repeated_rows_and_columns_are_expanded() {
    let doc = odf::read(&fodt(
        r#"<table:table table:name="T">
         <table:table-column table:number-columns-repeated="3"/>
         <table:table-row table:number-rows-repeated="2">
          <table:table-cell table:number-columns-repeated="3"><text:p>x</text:p></table:table-cell>
         </table:table-row>
        </table:table>"#,
    ))
    .expect("parses");
    let placed: Vec<String> = cells(&doc).into_iter().map(|(_, cell)| cell).collect();
    assert_eq!(
        placed,
        vec!["T!r0c0", "T!r0c1", "T!r0c2", "T!r1c0", "T!r1c1", "T!r1c2"],
        "two rows of three, from one row element and one cell element"
    );
    assert!(doc.blocks.iter().all(|b| b.text() == "x"));
}

/// A span is carried, and so are the positions it covers — the second half being what keeps a
/// merged table the same *shape* through a regenerate.
#[test]
fn a_span_survives_and_its_covered_positions_are_written_back() {
    let doc = odf::read(&fodt(
        r#"<table:table table:name="T">
         <table:table-column table:number-columns-repeated="2"/>
         <table:table-row>
          <table:table-cell table:number-columns-spanned="2"><text:p>wide</text:p></table:table-cell>
          <table:covered-table-cell/>
         </table:table-row>
         <table:table-row>
          <table:table-cell><text:p>a</text:p></table:table-cell>
          <table:table-cell><text:p>b</text:p></table:table-cell>
         </table:table-row>
        </table:table>"#,
    ))
    .expect("parses");
    let span = doc.blocks[0].cell.as_ref().expect("in a cell");
    assert_eq!((span.columns_spanned, span.rows_spanned), (2, 1));
    // The covered position contributes no block, and the row below is not shifted by it.
    assert_eq!(
        cells(&doc).into_iter().map(|(_, c)| c).collect::<Vec<_>>(),
        vec!["T!r0c0", "T!r1c0", "T!r1c1"]
    );

    let mut regenerated = doc.clone();
    regenerated.source = None;
    let out = String::from_utf8(odf::write(&regenerated, Form::Flat).expect("writes")).unwrap();
    assert!(out.contains(r#"table:number-columns-spanned="2""#));
    assert!(
        out.contains("<table:covered-table-cell/>"),
        "the covered position is written back, or the table changes shape:\n{out}"
    );
    let back = odf::read(out.as_bytes()).expect("reads back");
    assert_eq!(cells(&back), cells(&doc));
}

/// **A nested table is flattened into the cell that holds it.** The text is kept and the inner
/// structure is not, which is a decision rather than an omission: a nested table splits the
/// outer table's run of blocks in two, and a flat model cannot then say which half is which
/// (`text/src/model.rs`). Loop C found it, in a corpus document whose outer table came back
/// with 198 more blocks than it went in with, because its second half was written as a table of
/// its own starting at row 0.
#[test]
fn a_nested_table_reads_as_paragraphs_of_the_cell_holding_it() {
    let doc = odf::read(&fodt(
        r#"<table:table table:name="Outer">
         <table:table-column table:number-columns-repeated="2"/>
         <table:table-row>
          <table:table-cell>
           <text:p>holds a table</text:p>
           <table:table table:name="Inner">
            <table:table-column/>
            <table:table-row><table:table-cell><text:p>inner</text:p></table:table-cell></table:table-row>
           </table:table>
          </table:table-cell>
          <table:table-cell><text:p>beside it</text:p></table:table-cell>
         </table:table-row>
        </table:table>"#,
    ))
    .expect("parses");
    assert_eq!(
        cells(&doc),
        vec![
            ("holds a table".to_owned(), "Outer!r0c0".to_owned()),
            ("inner".to_owned(), "Outer!r0c0".to_owned()),
            ("beside it".to_owned(), "Outer!r0c1".to_owned()),
        ],
        "the inner table's text is kept, in the cell that held it"
    );
    // And the outer table is therefore still one run of consecutive blocks, which is what the
    // writer folds on.
    assert_eq!(doc.table(0), Some(0..3));
}

/// A table is the **maximal run of consecutive blocks naming it**, so a paragraph between two
/// tables of the same name makes them two tables. The only reading a flat sequence can offer,
/// and the one the writer, the projection and every shell share.
#[test]
fn a_table_is_the_run_of_blocks_that_name_it() {
    let app = App::new();
    app.insert(0, BlockKind::Paragraph, "before")
        .expect("inserts");
    app.insert_table(1, 2, 2, None).expect("inserts a table");
    app.insert(5, BlockKind::Paragraph, "after")
        .expect("inserts");

    let table = app.table(1).expect("p2 is in a table");
    assert_eq!(table.name, "Table1");
    assert_eq!(table.blocks, 1..5);
    assert_eq!((table.rows, table.columns), (2, 2));
    assert_eq!(app.table(0), None, "a paragraph is not in a table");
    assert_eq!(app.table(5), None);

    // A second table gets a name of its own, or the two would fold into one.
    app.insert_table(6, 1, 1, None).expect("inserts");
    assert_eq!(app.table(6).expect("the second").name, "Table2");
}

/// The claim the whole design rests on: **every other verb already worked inside a cell.**
/// Nothing in this test is table-aware — it types, formats, splits and bookmarks at addresses
/// that happen to be cells.
#[test]
fn every_other_edit_reaches_into_a_cell_unchanged() {
    let app = App::new();
    app.insert_table(0, 2, 2, None).expect("inserts a table");
    let caret = |block, offset| Caret { block, offset };

    // Typing.
    app.insert_text(caret(0, 0), "Region").expect("types");
    app.insert_text(caret(1, 0), "Revenue").expect("types");
    assert_eq!(app.input_text(0).unwrap(), "Region");

    // Formatting, over a span inside one cell.
    let mut bold = grind_text::CharStyle::default();
    bold.set_bold(true);
    app.set_char_style(caret(0, 0), caret(0, 6), &bold)
        .expect("formats");
    assert!(app.char_style(caret(0, 0), caret(0, 6)).unwrap().is_bold());

    // Splitting, which makes a *second block in the same cell* rather than a new cell.
    app.split_block(caret(0, 6)).expect("splits");
    let view = app.get_viewport(0..app.block_count());
    let first = view.get(0).expect("the first block");
    let second = view.get(1).expect("the block the split made");
    assert_eq!(first.cell, second.cell, "both in the same cell");
    assert_eq!(app.table(0).expect("still one table").blocks.len(), 5);

    // A bookmark, which is `loc.rs`'s stable address — inside a cell like anywhere else.
    app.set_bookmark("here", Some(2)).expect("names it");
    assert_eq!(app.bookmarks(), vec![("here".to_owned(), 2)]);
}

/// A table this build writes is one LibreOffice's schema allows: at least one
/// `table:table-column`, because `table-columns-and-groups` is a `oneOrMore` (rng:14200), and
/// R2 says everything written validates.
#[test]
fn a_written_table_declares_its_columns() {
    let app = App::new();
    app.insert_table(0, 1, 3, Some("Prices".to_owned()))
        .expect("inserts");
    app.insert(3, BlockKind::Paragraph, "after")
        .expect("inserts");
    let bytes = app.save_bytes(Form::Flat).expect("writes");
    let out = String::from_utf8(bytes).expect("utf-8");

    assert!(
        out.contains(r#"<table:table table:name="Prices">"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<table:table-column table:number-columns-repeated="3"/>"#),
        "one column element covering all three:\n{out}"
    );
    assert_eq!(out.matches("<table:table-row>").count(), 1);
    assert!(
        out.contains(r#"xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#),
        "and the namespace it needs, declared because it is used (R3)"
    );
}

/// A run inside a cell is a run: the writer's whitespace and formatting machinery is the same
/// one, reached through the same code path, which is what "a cell holds blocks" means in
/// practice.
#[test]
fn a_cells_content_is_written_like_any_other_block() {
    let doc = odf::read(&fodt(
        r#"<table:table table:name="T"><table:table-column/>
         <table:table-row><table:table-cell><text:p>a<text:tab/>b  c</text:p></table:table-cell></table:table-row>
        </table:table>"#,
    ))
    .expect("parses");
    assert_eq!(doc.blocks[0].text(), "a\tb  c");
    assert!(matches!(doc.blocks[0].runs[1], Run::Tab));

    let mut regenerated = doc.clone();
    regenerated.source = None;
    let bytes = odf::write(&regenerated, Form::Flat).expect("writes");
    let back = odf::read(&bytes).expect("reads back");
    assert_eq!(back.blocks[0].text(), "a\tb  c", "tab and spaces survived");
    assert_eq!(back.blocks[0].cell, doc.blocks[0].cell);
}
