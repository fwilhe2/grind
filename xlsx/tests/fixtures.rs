// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hand-built packages for the boundaries a corpus does not reliably contain.
//!
//! `doc/xlsx-import.md`'s verification section asks for these, and the reason is in its risk
//! 3: **LibreOffice's xlsx files are minimal reproductions of bugs, so they over-represent
//! the strange and under-represent the ordinary.** Loop A′ proves 360 files import; it does
//! not prove that the workbook is found by *relationship*, because every one of those files
//! also happens to put it where convention says.
//!
//! Every package here is assembled from XML written out in full, rather than vendored as a
//! binary. That is the point: a fixture you can read in the source is one a reviewer can
//! check, and a `.xlsx` in git is eight files nobody will ever open.

use std::io::{Cursor, Write};

use grind_xlsx::{Dropped, Flavour};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const MAIN_STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const PKG: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MCE: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// Assemble a zip from `(part name, content)` pairs.
fn package(parts: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut out));
        let options: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in parts {
            zip.start_file(*name, options).expect("a zip entry");
            zip.write_all(content.as_bytes()).expect("writing an entry");
        }
        zip.finish().expect("a finished zip");
    }
    out
}

/// The package relationship part, pointing at the workbook wherever it is.
fn root_rels(workbook_at: &str) -> String {
    format!(
        r#"<Relationships xmlns="{PKG}">
             <Relationship Id="rId1" Type="{REL}/officeDocument" Target="{workbook_at}"/>
           </Relationships>"#
    )
}

fn workbook(sheets: &str) -> String {
    format!(r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets>{sheets}</sheets></workbook>"#)
}

fn sheet_names(bytes: &[u8]) -> Vec<String> {
    let (document, _) = grind_xlsx::import_bytes(bytes).expect("an importable workbook");
    document.sheets.iter().map(|s| s.name.clone()).collect()
}

/// The claim the whole `package.rs` design rests on, and the one loop A′ cannot make: a
/// workbook is found by **relationship**, not by its conventional path.
#[test]
fn the_workbook_is_found_where_the_relationship_says() {
    let bytes = package(&[
        ("_rels/.rels", root_rels("parts/book.xml")),
        (
            "parts/book.xml",
            workbook(r#"<sheet name="Elsewhere" sheetId="1"/>"#),
        ),
    ]);
    assert_eq!(sheet_names(&bytes), ["Elsewhere"]);
}

/// …and when a package declares nothing at all, the conventional path is the *last* resort
/// rather than the first. Half a broken file being readable beats none of it.
#[test]
fn a_package_with_no_relationships_falls_back_to_convention() {
    let bytes = package(&[(
        "xl/workbook.xml",
        workbook(r#"<sheet name="Salvaged" sheetId="1"/>"#),
    )]);
    assert_eq!(sheet_names(&bytes), ["Salvaged"]);
}

#[test]
fn a_zip_that_is_not_a_spreadsheet_says_so() {
    let bytes = package(&[("hello.txt", "not a workbook".to_owned())]);
    let err = grind_xlsx::import_bytes(&bytes).expect_err("no workbook part");
    assert!(matches!(err, grind_xlsx::Error::NotSpreadsheet), "{err}");
}

// ---- markup compatibility, end to end through a real package ----

/// The fixture the plan asks for by name: the choice and the fallback hold **different**
/// sheet lists, so a reader that takes the wrong one — or both — fails visibly.
#[test]
fn an_alternation_contributes_its_fallback_and_nothing_else() {
    let book = format!(
        r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}" xmlns:mc="{MCE}"
                     xmlns:x14="http://example.invalid/x14" mc:Ignorable="x14">
             <mc:AlternateContent>
               <mc:Choice Requires="x14"><sheets><sheet name="FromChoice"/></sheets></mc:Choice>
               <mc:Fallback><sheets><sheet name="FromFallback"/></sheets></mc:Fallback>
             </mc:AlternateContent>
           </workbook>"#
    );
    let bytes = package(&[
        ("_rels/.rels", root_rels("xl/workbook.xml")),
        ("xl/workbook.xml", book),
    ]);
    assert_eq!(sheet_names(&bytes), ["FromFallback"]);
}

#[test]
fn must_understand_is_reported_and_the_workbook_still_imports() {
    let book = format!(
        r#"<workbook xmlns="{MAIN}" xmlns:mc="{MCE}" xmlns:x14="http://example.invalid/x14"
                     mc:MustUnderstand="x14">
             <sheets><sheet name="Sparklines"/></sheets>
           </workbook>"#
    );
    let bytes = package(&[
        ("_rels/.rels", root_rels("xl/workbook.xml")),
        ("xl/workbook.xml", book),
    ]);
    let (document, report) = grind_xlsx::import_bytes(&bytes).expect("still importable");
    assert_eq!(document.sheets[0].name, "Sparklines");
    assert_eq!(
        report.must_understand.iter().cloned().collect::<Vec<_>>(),
        ["x14"],
        "reported, because refusing a workbook over one slicer is the same mistake as \
         refusing it over one chart"
    );
}

// ---- flavours ----

#[test]
fn a_strict_package_imports_and_says_it_was_strict() {
    let bytes = package(&[
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG}">
                     <Relationship Id="rId1" Type="{REL_STRICT}/officeDocument" Target="xl/workbook.xml"/>
                   </Relationships>"#
            ),
        ),
        (
            "xl/workbook.xml",
            format!(
                r#"<workbook xmlns="{MAIN_STRICT}" conformance="strict">
                     <sheets><sheet name="Conforming"/></sheets>
                   </workbook>"#
            ),
        ),
    ]);
    let (document, report) = grind_xlsx::import_bytes(&bytes).expect("Strict is read too");
    assert_eq!(document.sheets[0].name, "Conforming");
    assert_eq!(report.flavour, Flavour::Strict);
}

/// Measured in the wild: `universal-content-strict.xlsx` carries both families in one
/// `_rels/.rels` (`doc/xlsx-format.md` §1.2), so `Mixed` is a real answer and not a
/// hypothetical one.
#[test]
fn a_package_carrying_both_families_is_mixed() {
    let bytes = package(&[
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG}">
                     <Relationship Id="rId1" Type="{REL_STRICT}/officeDocument" Target="xl/workbook.xml"/>
                   </Relationships>"#
            ),
        ),
        ("xl/workbook.xml", workbook(r#"<sheet name="Both"/>"#)),
    ]);
    let (_, report) = grind_xlsx::import_bytes(&bytes).expect("mixed is still a workbook");
    assert_eq!(report.flavour, Flavour::Mixed);
}

// ---- what real producers write ----

#[test]
fn a_target_with_a_leading_slash_names_the_same_part() {
    let bytes = package(&[
        ("_rels/.rels", root_rels("/xl/workbook.xml")),
        ("xl/workbook.xml", workbook(r#"<sheet name="Absolute"/>"#)),
    ]);
    assert_eq!(sheet_names(&bytes), ["Absolute"]);
}

#[test]
fn part_names_compare_without_regard_to_case() {
    let bytes = package(&[
        ("_rels/.rels", root_rels("XL/Workbook.XML")),
        ("xl/workbook.xml", workbook(r#"<sheet name="Shouty"/>"#)),
    ]);
    assert_eq!(sheet_names(&bytes), ["Shouty"]);
}

/// Zip-slip. A relationship target that climbs out of the package names a file on the
/// machine running the converter, and there is no reading of it that is a part.
#[test]
fn a_relationship_pointing_out_of_the_package_reaches_nothing() {
    let bytes = package(&[
        ("_rels/.rels", root_rels("../../../../etc/passwd")),
        ("xl/workbook.xml", workbook(r#"<sheet name="Unreached"/>"#)),
    ]);
    // It falls back to the conventional path rather than reading anything outside — the
    // escape simply resolves to no part at all.
    assert_eq!(sheet_names(&bytes), ["Unreached"]);
}

/// `.xlsm` is the same XML with a macro beside it. The data imports; the macro is counted
/// and **never executed**.
#[test]
fn a_macro_is_data_to_be_counted() {
    let bytes = package(&[
        ("_rels/.rels", root_rels("xl/workbook.xml")),
        ("xl/workbook.xml", workbook(r#"<sheet name="Automated"/>"#)),
        ("xl/vbaProject.bin", "\u{0}not executed, ever".to_owned()),
    ]);
    let (document, report) = grind_xlsx::import_bytes(&bytes).expect("a macro workbook imports");
    assert_eq!(document.sheets[0].name, "Automated");
    assert_eq!(report.dropped[&Dropped::Macro], 1);
    assert!(!report.lossless());
}

/// An external workbook link is recorded and **never fetched**. A converter that makes
/// network requests is a different threat model.
#[test]
fn an_external_link_is_not_followed() {
    let bytes = package(&[
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="{PKG}">
                     <Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/>
                     <Relationship Id="rId2" Type="{REL}/externalLink" Target="https://example.invalid/other.xlsx" TargetMode="External"/>
                   </Relationships>"#
            ),
        ),
        ("xl/workbook.xml", workbook(r#"<sheet name="Linked"/>"#)),
    ]);
    // The assertion that matters is that this returns at all, promptly, having made no
    // request. `example.invalid` is unresolvable by RFC 6761, so a reader that *did* fetch
    // would hang or fail rather than pass quietly.
    assert_eq!(sheet_names(&bytes), ["Linked"]);
}

#[test]
fn a_sheet_whose_part_is_missing_keeps_its_tab() {
    let bytes = package(&[
        ("_rels/.rels", root_rels("xl/workbook.xml")),
        (
            "xl/workbook.xml",
            workbook(r#"<sheet name="Orphan" sheetId="1" r:id="rId404"/>"#),
        ),
    ]);
    assert_eq!(sheet_names(&bytes), ["Orphan"]);
}

/// A workbook part that is not XML at all. The *file* is readable and the workbook is not,
/// which is the structural failure case rather than the tolerance one.
#[test]
fn a_workbook_part_that_will_not_parse_is_an_error() {
    let bytes = package(&[
        ("_rels/.rels", root_rels("xl/workbook.xml")),
        ("xl/workbook.xml", "<workbook><sheets>".to_owned()),
    ]);
    let err = grind_xlsx::import_bytes(&bytes).expect_err("truncated XML");
    assert!(matches!(err, grind_xlsx::Error::Xml(_)), "{err}");
}
