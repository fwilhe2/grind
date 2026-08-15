// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reader semantics: are the values in the right cells, and does malformed input degrade
//! the way doc/ods-format.md §9 says it should?
//!
//! Loop A (`corpus_read.rs`) only proves documents load. This proves they load *correctly*,
//! which is the other half of the phase 2 exit criterion.

use std::path::PathBuf;

use sheet_core::{CellValue, Pos, read_bytes};

/// Wrap a `table:table` body in the smallest valid flat document.
fn doc(body: &str) -> sheet_core::Document {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:calcext="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0"
  office:mimetype="application/vnd.oasis.opendocument.spreadsheet"
  office:version="1.4">
 <office:body><office:spreadsheet>{body}</office:spreadsheet></office:body>
</office:document>"#
    );
    read_bytes("test.fods", xml.as_bytes()).expect("fixture must parse")
}

fn cell(d: &sheet_core::Document, row: u32, col: u32) -> CellValue {
    d.sheet(0).expect("one sheet").get(Pos::new(row, col))
}

fn num(n: f64) -> CellValue {
    CellValue::Number(n)
}

fn text(s: &str) -> CellValue {
    CellValue::Text(s.to_owned())
}

#[test]
fn a_float_cell_reads_its_value_not_its_display_text() {
    // The display text is deliberately a lie: `office:value` is the value.
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="float" office:value="1.5">
                 <text:p>1,50 €</text:p>
               </table:table-cell>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), num(1.5));
}

#[test]
fn sheet_names_come_from_the_file() {
    let d = doc(
        r#"<table:table table:name="Budget"><table:table-row><table:table-cell/></table:table-row></table:table>
           <table:table table:name="Notes"><table:table-row><table:table-cell/></table:table-row></table:table>"#,
    );
    assert_eq!(d.sheets.len(), 2);
    assert_eq!(d.sheets[0].name, "Budget");
    assert_eq!(d.sheets[1].name, "Notes");
}

/// The trap the plan singles out: `number-columns-repeated` is positioning, not an
/// optimisation. Ignore it and every cell after it lands in the wrong column.
#[test]
fn repeated_columns_move_later_cells_along() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="string"><text:p>a</text:p></table:table-cell>
               <table:table-cell table:number-columns-repeated="8"/>
               <table:table-cell office:value-type="string"><text:p>j</text:p></table:table-cell>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), text("a"));
    for col in 1..9 {
        assert_eq!(cell(&d, 0, col), CellValue::Empty, "col {col}");
    }
    assert_eq!(cell(&d, 0, 9), text("j"), "the repeat must skip 8 columns");
}

#[test]
fn a_repeated_cell_with_content_is_written_to_every_column() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell table:number-columns-repeated="3" office:value-type="float" office:value="7"/>
               <table:table-cell office:value-type="float" office:value="9"/>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), num(7.0));
    assert_eq!(cell(&d, 0, 1), num(7.0));
    assert_eq!(cell(&d, 0, 2), num(7.0));
    assert_eq!(cell(&d, 0, 3), num(9.0));
}

#[test]
fn repeated_rows_move_later_rows_along_and_replay_content() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row table:number-rows-repeated="2">
               <table:table-cell office:value-type="float" office:value="1"/>
             </table:table-row>
             <table:table-row table:number-rows-repeated="5">
               <table:table-cell/>
             </table:table-row>
             <table:table-row>
               <table:table-cell office:value-type="float" office:value="2"/>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), num(1.0), "repeated row replays its content");
    assert_eq!(cell(&d, 1, 0), num(1.0));
    for row in 2..7 {
        assert_eq!(cell(&d, row, 0), CellValue::Empty, "row {row}");
    }
    assert_eq!(cell(&d, 7, 0), num(2.0), "empty repeat still advances");
}

/// A trailing empty row repeated a million times is how sheets bound their extent (§3.3).
/// It must be near-free and must not inflate the used extent.
#[test]
fn a_huge_trailing_empty_repeat_is_cheap_and_changes_nothing() {
    let started = std::time::Instant::now();
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="float" office:value="1"/>
             </table:table-row>
             <table:table-row table:number-rows-repeated="1048575">
               <table:table-cell table:number-columns-repeated="16384"/>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), num(1.0));
    let sheet = d.sheet(0).unwrap();
    assert_eq!(
        (sheet.used_rows(), sheet.used_cols()),
        (1, 1),
        "empty repeats must not inflate the used extent"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "a trailing empty repeat must not be materialised cell by cell"
    );
}

/// §9: never trust a count enough to act on it. An absurd repeat is clamped, not obeyed.
#[test]
fn an_absurd_repeat_count_is_clamped_rather_than_allocated() {
    let started = std::time::Instant::now();
    let d = doc(r#"<table:table table:name="S">
             <table:table-row table:number-rows-repeated="4000000000">
               <table:table-cell table:number-columns-repeated="4000000000" office:value-type="float" office:value="1"/>
             </table:table-row>
           </table:table>"#);
    let sheet = d.sheet(0).unwrap();
    assert!(sheet.used_rows() <= 1_048_576);
    assert!(sheet.used_cols() <= 16_384);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "clamping must bound the work, not merely the numbers"
    );
}

#[test]
fn value_types_map_to_the_right_rust_values() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="float" office:value="-2.25"/>
               <table:table-cell office:value-type="percentage" office:value="0.5"/>
               <table:table-cell office:value-type="currency" office:value="9.99" office:currency="EUR"/>
               <table:table-cell office:value-type="boolean" office:boolean-value="true"/>
               <table:table-cell office:value-type="boolean" office:boolean-value="false"/>
               <table:table-cell office:value-type="string" office:string-value="explicit"><text:p>display</text:p></table:table-cell>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), num(-2.25));
    assert_eq!(cell(&d, 0, 1), num(0.5), "percentage stores the fraction");
    assert_eq!(cell(&d, 0, 2), num(9.99));
    assert_eq!(cell(&d, 0, 3), CellValue::Bool(true));
    assert_eq!(cell(&d, 0, 4), CellValue::Bool(false));
    assert_eq!(
        cell(&d, 0, 5),
        text("explicit"),
        "an explicit string-value beats the display text"
    );
}

#[test]
fn display_text_is_the_value_when_no_explicit_one_is_given() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="string"><text:p>from text</text:p></table:table-cell>
               <table:table-cell><text:p>untyped but present</text:p></table:table-cell>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), text("from text"));
    assert_eq!(cell(&d, 0, 1), text("untyped but present"));
}

#[test]
fn several_paragraphs_become_several_lines() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="string">
                 <text:p>one</text:p><text:p>two</text:p>
               </table:table-cell>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), text("one\ntwo"));
}

#[test]
fn an_empty_paragraph_is_still_a_line() {
    // Counting paragraphs, not testing whether the accumulated text is empty: a blank
    // first or middle line contributes no characters, and testing emptiness silently eats
    // it. This is what a cell holding "a\n\nb" round-trips as, which is how it surfaced.
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="string">
                 <text:p/><text:p>after</text:p>
               </table:table-cell>
               <table:table-cell office:value-type="string">
                 <text:p>a</text:p><text:p/><text:p>b</text:p>
               </table:table-cell>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), text("\nafter"));
    assert_eq!(cell(&d, 0, 1), text("a\n\nb"));
}

#[test]
fn text_s_carries_a_count_of_spaces() {
    // ODF collapses runs of whitespace inside `text:p`, so LibreOffice writes "a    b" as
    // one literal space plus `<text:s text:c="3"/>`. Ignoring the count turns every
    // multi-space string into a single-space one — silently, and in real documents.
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="string">
                 <text:p>a <text:s text:c="3"/>b</text:p>
               </table:table-cell>
               <table:table-cell office:value-type="string">
                 <text:p><text:s/>x</text:p>
               </table:table-cell>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), text("a    b"));
    // An absent count means one, per the schema default.
    assert_eq!(cell(&d, 0, 1), text(" x"));
}

#[test]
fn what_we_write_is_what_we_read() {
    // The half of loop C that needs no LibreOffice, and so runs everywhere: our own writer
    // and reader must agree on every value the model can hold. Catches a writer that
    // mangles whitespace, newlines or XML metacharacters on a machine with no `soffice`.
    let mut d = sheet_core::Document::default();
    let sheet = d.sheet_mut(0).unwrap();
    for (row, value) in [
        text("plain"),
        text("  leading and trailing  "),
        text("inner    spaces"),
        text("tab\there"),
        text("line\n\nbreaks\n"),
        text("<xml> & \"quotes\""),
        text(""),
        num(-0.5),
        num(1e300),
        CellValue::Bool(true),
    ]
    .into_iter()
    .enumerate()
    {
        sheet.set(Pos::new(row as u32, 0), value);
    }
    sheet.set_formula(Pos::new(0, 0), "of:=1+1".into());

    for form in [sheet_core::Form::Flat, sheet_core::Form::Package] {
        let bytes = sheet_core::write_bytes(&d, form).unwrap();
        let back = read_bytes("round-trip", &bytes).unwrap();
        let (a, b) = (d.sheet(0).unwrap(), back.sheet(0).unwrap());
        assert_eq!(a.used_rows(), b.used_rows(), "{form:?}");
        for row in 0..a.used_rows() {
            let pos = Pos::new(row, 0);
            assert_eq!(a.get(pos), b.get(pos), "{form:?} row {row}");
            assert_eq!(a.formula(pos), b.formula(pos), "{form:?} row {row}");
        }
    }
}

#[test]
fn styled_runs_and_entities_inside_a_paragraph_survive() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="string">
                 <text:p>a <text:span>bold</text:span> word &amp; more</text:p>
               </table:table-cell>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), text("a bold word & more"));
}

/// §9: indentation between structural elements must never become cell content. This falls
/// out of the default no-op `text` callback rather than a whitespace-skipping pass.
#[test]
fn pretty_printing_whitespace_is_not_cell_content() {
    let d = doc("<table:table table:name=\"S\">\n\n   \
           <table:table-row>\n      \
             <table:table-cell/>\n      \
             <table:table-cell office:value-type=\"float\" office:value=\"1\"/>\n   \
           </table:table-row>\n\n\
         </table:table>");
    assert_eq!(cell(&d, 0, 0), CellValue::Empty);
    assert_eq!(cell(&d, 0, 1), num(1.0));
    assert_eq!(d.sheet(0).unwrap().used_cols(), 2);
}

/// §8.1: dispatch keys on the namespace URI. The prefix a document happens to use carries
/// no meaning, so this file — identical but for hostile prefixes and a default namespace —
/// must read the same.
#[test]
fn prefixes_are_irrelevant_only_the_namespace_uri_matters() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<zz:document
  xmlns:zz="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:q="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  zz:version="1.4">
 <zz:body><zz:spreadsheet>
  <table table:name="S" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0">
   <table-row>
    <table-cell zz:value-type="string"><q:p>hello</q:p></table-cell>
    <table-cell zz:value-type="float" zz:value="3"/>
   </table-row>
  </table>
 </zz:spreadsheet></zz:body>
</zz:document>"#;
    let d = read_bytes("odd-prefixes.fods", xml.as_bytes()).expect("must parse");
    assert_eq!(d.sheets.len(), 1);
    assert_eq!(d.sheets[0].name, "S");
    assert_eq!(cell(&d, 0, 0), text("hello"));
    assert_eq!(cell(&d, 0, 1), num(3.0));
}

/// §9: `calcext:value-type` is an accepted alias for `office:value-type`.
#[test]
fn the_calcext_value_type_alias_is_honoured() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell calcext:value-type="float" office:value="42"/>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), num(42.0));
}

/// §9: unknown elements, attributes and whole foreign namespaces are inert. Nothing here
/// detects junk — it simply has no handler.
#[test]
fn foreign_content_is_ignored_without_disturbing_the_document() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:vendor="http://example.invalid/private/v9"
  office:version="1.4">
 <vendor:preamble><vendor:junk vendor:x="1">ignore me</vendor:junk></vendor:preamble>
 <office:body>
  <office:spreadsheet>
   <table:table table:name="S" vendor:secret="hide">
    <vendor:sidecar><table:table-row><table:table-cell office:value-type="float" office:value="666"/></table:table-row></vendor:sidecar>
    <table:table-row>
     <table:table-cell office:value-type="float" office:value="1" vendor:flag="yes">
       <vendor:annotation>not text</vendor:annotation>
     </table:table-cell>
     <table:table-cell office:value-type="string"><text:p>real</text:p></table:table-cell>
    </table:table-row>
   </table:table>
  </office:spreadsheet>
 </office:body>
</office:document>"#;
    let d = read_bytes("foreign.fods", xml.as_bytes()).expect("must parse");
    assert_eq!(cell(&d, 0, 0), num(1.0));
    assert_eq!(
        cell(&d, 0, 1),
        text("real"),
        "text from an ignored subtree must not leak into the cell"
    );
    assert_eq!(
        d.sheet(0).unwrap().used_rows(),
        1,
        "rows inside an unrecognised wrapper must not be imported"
    );
}

/// §9: a malformed value degrades to a safe default, scoped to its own cell.
#[test]
fn malformed_values_degrade_without_losing_the_document() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell office:value-type="float" office:value="not-a-number"/>
               <table:table-cell office:value-type="float"/>
               <table:table-cell office:value-type="float" office:value="NaN"/>
               <table:table-cell office:value-type="rumpelstiltskin"><text:p>kept</text:p></table:table-cell>
               <table:table-cell office:value-type="float" office:value="5"/>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), num(0.0));
    assert_eq!(cell(&d, 0, 1), num(0.0));
    assert_eq!(cell(&d, 0, 2), num(0.0), "NaN is not finite; degrade to 0");
    assert_eq!(
        cell(&d, 0, 3),
        text("kept"),
        "an unknown value-type keeps what is visible"
    );
    assert_eq!(cell(&d, 0, 4), num(5.0), "later cells are unaffected");
}

#[test]
fn formula_text_is_kept_verbatim_beside_the_cached_value() {
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell table:formula="of:=SUM([.A2:.A10])" office:value-type="float" office:value="55"/>
               <table:table-cell table:formula="of:=COM.MICROSOFT.UNIQUE([.B1])" office:value-type="float" office:value="0"/>
             </table:table-row>
           </table:table>"#);
    let sheet = d.sheet(0).unwrap();
    assert_eq!(cell(&d, 0, 0), num(55.0), "the cached value is used as-is");
    assert_eq!(sheet.formula(Pos::new(0, 0)), Some("of:=SUM([.A2:.A10])"));
    assert_eq!(
        sheet.formula(Pos::new(0, 1)),
        Some("of:=COM.MICROSOFT.UNIQUE([.B1])"),
        "a vendor function we cannot evaluate is still preserved"
    );
}

#[test]
fn covered_cells_hold_their_grid_position() {
    // A 2-wide merge: the covered cell is written out, not omitted (§3.4).
    let d = doc(r#"<table:table table:name="S">
             <table:table-row>
               <table:table-cell table:number-columns-spanned="2" table:number-rows-spanned="1"
                                 office:value-type="string"><text:p>merged</text:p></table:table-cell>
               <table:covered-table-cell/>
               <table:table-cell office:value-type="float" office:value="3"/>
             </table:table-row>
           </table:table>"#);
    assert_eq!(cell(&d, 0, 0), text("merged"));
    assert_eq!(cell(&d, 0, 1), CellValue::Empty);
    assert_eq!(cell(&d, 0, 2), num(3.0), "the covered cell took column 1");
}

#[test]
fn a_document_with_no_sheets_is_empty_not_an_error() {
    let d = doc("");
    assert!(d.sheets.is_empty());
}

// --- against a real LibreOffice-authored file ---

fn corpus() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var("SHEET_LO_CORPUS").unwrap_or_else(|_| {
        "/home/florian/code/github.com/LibreOffice/core/sc/qa/unit/data".to_owned()
    }));
    root.is_dir().then_some(root)
}

/// Hand-checked against the file's own XML: sheet `Data`, A1 a string, eight repeated
/// empty cells, then J1 — which is exactly the repeat arithmetic that silently corrupts a
/// reader that treats `number-columns-repeated` as a hint.
#[test]
fn a_real_libreoffice_file_lands_its_cells_where_the_xml_says() {
    let Some(root) = corpus() else {
        eprintln!("skipping: set SHEET_LO_CORPUS to run against the LibreOffice corpus");
        return;
    };
    let path = root.join("fods/lookup_source.fods");
    let d = sheet_core::read_file(&path).expect("must read");

    let data = d
        .sheets
        .iter()
        .find(|s| s.name == "Data")
        .expect("a sheet named Data");

    assert_eq!(data.get(Pos::new(0, 0)), text("Equal orientation vertical"));
    for col in 1..9 {
        assert_eq!(data.get(Pos::new(0, col)), CellValue::Empty, "col {col}");
    }
    assert!(
        matches!(data.get(Pos::new(0, 9)), CellValue::Text(s) if s.starts_with("The sheet “Data” serves as source")),
        "J1 must hold the note, which means the 8-column repeat was honoured; got {:?}",
        data.get(Pos::new(0, 9))
    );

    assert_eq!(data.get(Pos::new(1, 0)), text("key"));
    assert_eq!(data.get(Pos::new(1, 1)), CellValue::Empty);
    assert_eq!(data.get(Pos::new(1, 2)), text("value"));
}

/// Formulas are not evaluated yet, but a file full of them must come back with its cached
/// values and its formula text intact.
#[test]
fn a_real_formula_file_keeps_both_halves() {
    let Some(root) = corpus() else {
        return;
    };
    let path = root.join("functions/mathematical/fods/sum.fods");
    let Ok(d) = sheet_core::read_file(&path) else {
        eprintln!("skipping: {} not present", path.display());
        return;
    };
    let total: usize = d.sheets.iter().map(sheet_core::Sheet::formula_count).sum();
    assert!(total > 10, "expected many formulas, found {total}");

    let has_cached_value = d
        .sheets
        .iter()
        .any(|s| (0..40).any(|r| (0..10).any(|c| !s.get(Pos::new(r, c)).is_empty())));
    assert!(has_cached_value, "cached values must be imported too");
}
