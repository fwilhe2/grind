// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading Word documents — an import filter that produces an ODF text document.
//!
//! `doc/docx-import.md` is the plan and is normative; `doc/docx-format.md` holds the facts about
//! the format that had to be measured. It is `grind-xlsx`'s twin for the other application, and
//! the same rules hold:
//!
//! - **One way in, never out.** Writing `.docx` is `doc/not-doing.md` §1 and stays there.
//!   Reading is a one-way translation at the edge that produces an ODF document, and everything
//!   downstream is ODF. No Word vocabulary reaches `grind-text` (R1).
//! - **The XML form only** — Word 2007 onwards (`.docx`, `.docm`, `.dotx`). Not `.doc`, not
//!   RTF, not Word 2003 XML. **Transitional** is the target; Strict is a namespace table away
//!   (`grind_ooxml::names`).
//! - **The report is part of the output.** What a conversion dropped is as important as what it
//!   carried; see [`Report`].
//!
//! **What is different from the spreadsheet's filter** is where the translation lands. That one
//! builds a `grind_sheet::Document` through the model's API, because nearly everything a cell
//! says has a field there. A word-processing document's look lives in named styles, list
//! styles, a page and its header — which `grind_text` reads and never writes, carrying them out
//! of the file a document came from instead. So this filter writes that file: one flat ODF text
//! document ([`emit`]), which `grind_text` then reads like any other (`doc/docx-import.md`,
//! "Why ODF and not the model").
//!
//! ```no_run
//! let bytes = std::fs::read("letter.docx")?;
//! let (document, report) = grind_docx::import_bytes(&bytes)?;
//! println!("{} blocks; {}", document.blocks.len(), report.summary());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::HashMap;
use std::path::Path;

use grind_ooxml::names::{Ns, RelType, Seen};
use grind_ooxml::package::Package;

pub mod body;
pub mod emit;
pub mod numbering;
pub mod props;
pub mod report;
pub mod section;
pub mod styles;
pub mod xml;

pub use grind_ooxml::{Error, Flavour, Result};
pub use report::{Dropped, Report};

use crate::body::{Block, Ctx};
use crate::section::Which;
use crate::xml::{Handled, Reader, Word as _, WordAttrs as _};

/// The main part, found by relationship and asked what it is.
fn find_part(package: &mut Package, seen: &mut Seen) -> Result<String> {
    let part = package
        .rel_of("", RelType::OfficeDocument, seen)
        .map(|rel| rel.target)
        .or_else(|| {
            package
                .part("word/document.xml")
                .map(|_| "word/document.xml".to_owned())
        })
        .ok_or(Error::NotWordDocument)?;
    let bytes = package.part(&part).ok_or(Error::NotWordDocument)?;
    match Reader::new(&bytes).root() {
        Ok(Some((name, _))) if name.w("document") => Ok(part),
        _ => Err(Error::NotWordDocument),
    }
}

/// Whether these bytes are an OOXML word-processing document.
///
/// Sniffed from content, never from the file name, as `grind_core::kind` is: the zip, the main
/// part found by relationship, and that part's root being `w:document` — a workbook's main part
/// is found by the same relationship type, so the last question is the one that tells them
/// apart. An **encrypted** document answers `false` (it is a CFB container); [`import_bytes`]
/// says so properly.
pub fn sniff(bytes: &[u8]) -> bool {
    if !grind_ooxml::package::is_zip(bytes) {
        return false;
    }
    let Ok(mut package) = Package::open(bytes) else {
        return false;
    };
    find_part(&mut package, &mut Seen::default()).is_ok()
}

/// Read a Word document into the flat ODF text document that says the same thing, and the
/// report of what that left out.
///
/// The bytes are what [`open`] hands a shell; [`import_bytes`] reads them.
pub fn convert(bytes: &[u8]) -> Result<(Vec<u8>, Report)> {
    let mut report = Report::default();
    let mut seen = Seen::default();
    let mut package = Package::open(bytes)?;
    let main = find_part(&mut package, &mut seen)?;
    let rels = package.rels(&main, &mut seen);
    let target = |kind: RelType| {
        rels.iter()
            .find(|rel| rel.kind == kind && !rel.external)
            .map(|rel| rel.target.clone())
    };

    let fonts = target(RelType::Theme)
        .and_then(|part| package.part(&part))
        .map(|bytes| styles::read_fonts(&bytes))
        .unwrap_or_default();
    let styles = match target(RelType::Styles).and_then(|part| package.part(&part)) {
        Some(bytes) => styles::read(&bytes, &fonts),
        None => styles::read(b"", &fonts),
    };
    let numbering = target(RelType::Numbering)
        .and_then(|part| package.part(&part))
        .map(|bytes| numbering::read(&bytes, &fonts))
        .unwrap_or_default();
    let fonts_table = target(RelType::FontTable)
        .and_then(|part| package.part(&part))
        .map(|bytes| styles::read_font_table(&bytes))
        .unwrap_or_default();
    let settings = target(RelType::Settings)
        .and_then(|part| package.part(&part))
        .map(|bytes| settings(&bytes))
        .unwrap_or_default();
    let default_tab = settings.default_tab;
    if rels
        .iter()
        .any(|rel| rel.target.ends_with("vbaproject.bin"))
        || package.part("word/vbaProject.bin").is_some()
    {
        report.drop_one(Dropped::Macro);
    }

    let mut ctx = Ctx::new(&mut package, &fonts, &mut report, &mut seen);

    // The notes first: the body cites them, and `emit` writes each where it is cited.
    let footnotes = match target(RelType::Footnotes) {
        Some(part) => notes(&mut ctx, &part),
        None => HashMap::new(),
    };
    let endnotes = match target(RelType::Endnotes) {
        Some(part) => notes(&mut ctx, &part),
        None => HashMap::new(),
    };

    let comments = match target(RelType::Comments) {
        Some(part) => comments(&mut ctx, &part),
        None => HashMap::new(),
    };

    // The body.
    ctx.enter(&main);
    let bytes = ctx.package.part(&main).ok_or(Error::NotWordDocument)?;
    let mut reader = Reader::new(&bytes);
    reader.root()?;
    let mut body = Vec::new();
    let read = reader.children(|r, name, _| {
        if name.w("body") {
            body::blocks_into(r, &mut ctx, &mut body)?;
            Ok(Handled::Yes)
        } else {
            Ok(Handled::No)
        }
    });
    // XML that stops being XML part-way through is a damaged document, not a foreign one:
    // everything before the damage was read and is kept, and the rest is counted. LibreOffice
    // opens such a file too (`doc/docx-format.md` §1.5); refusing it would make the import less
    // tolerant than the oracle over a fault in one equation.
    if let Err(error) = read {
        match error {
            Error::Xml(_) => ctx.report.drop_one(Dropped::Damaged),
            error => return Err(error),
        }
    }
    ctx.seen.transitional |= reader.seen.transitional;
    ctx.seen.strict |= reader.seen.strict;
    ctx.report
        .must_understand
        .append(&mut reader.must_understand);
    let body = body::page_breaks(body);
    let sections = std::mem::take(&mut ctx.sections);

    // The page is the first section's; every later one is asked whether it differs.
    // Every section's page is carried, each distinct one a master page of its own
    // (`emit::write`); what is counted is what a section says that no page style holds.
    for section in &sections {
        if section.columns.is_some() {
            ctx.report.drop_one(Dropped::Columns);
        }
    }
    let rels: HashMap<String, String> = ctx
        .package
        .rels(&main, ctx.seen)
        .into_iter()
        .map(|rel| (rel.id, rel.target))
        .collect();
    // A section with no reference of a kind shows the section before it's (§17.10.5), so the
    // references in effect are carried forward section by section. Each part is read once.
    let mut stories: HashMap<String, Option<Vec<Block>>> = HashMap::new();
    let mut read = |ctx: &mut Ctx, part: &str| -> Option<(String, Vec<Block>)> {
        let blocks = stories
            .entry(part.to_owned())
            .or_insert_with(|| story(ctx, part))
            .clone()?;
        Some((part.to_owned(), blocks))
    };
    let mut in_effect: [Option<String>; 6] = Default::default();
    let mut pages = Vec::with_capacity(sections.len());
    for section in &sections {
        let pick = |refs: &[(Which, String)], which: Which| {
            emit::pick(refs, which).and_then(|id| rels.get(id)).cloned()
        };
        let own = [
            pick(&section.headers, Which::Default),
            pick(&section.footers, Which::Default),
            pick(&section.headers, Which::First),
            pick(&section.footers, Which::First),
            pick(&section.headers, Which::Even),
            pick(&section.footers, Which::Even),
        ];
        for (slot, own) in in_effect.iter_mut().zip(own) {
            if own.is_some() {
                *slot = own;
            }
        }
        let mut get = |i: usize| in_effect[i].clone().and_then(|part| read(&mut ctx, &part));
        let (header, footer) = (get(0), get(1));
        let (header_first, footer_first) = if section.title_page {
            (get(2), get(3))
        } else {
            (None, None)
        };
        // Word shows an even-page variant only when the document says even and odd pages
        // differ (`w:evenAndOddHeaders`, §17.10.1), and a first-page one only under
        // `w:titlePg`; a variant it would not show is not carried and not a loss.
        let (header_even, footer_even) = if settings.even_and_odd {
            (get(4), get(5))
        } else {
            (None, None)
        };
        pages.push(emit::Page {
            header,
            footer,
            header_first,
            footer_first,
            header_even,
            footer_even,
        });
    }
    let anchors = std::mem::take(&mut ctx.anchors);
    let changes = std::mem::take(&mut ctx.changes);
    drop(ctx);

    report.flavour = seen.flavour();
    let input = emit::Input {
        styles: &styles,
        numbering: &numbering,
        body,
        sections,
        pages,
        footnotes,
        endnotes,
        anchors,
        default_tab,
        fonts: fonts_table,
        comments,
        changes,
    };
    let odf = emit::write(input, &mut report);
    Ok((odf.into_bytes(), report))
}

/// Read a Word document.
///
/// Never fetches anything, never runs a macro, and never fails on a construct it cannot carry:
/// what it cannot carry is counted in the [`Report`].
pub fn import_bytes(bytes: &[u8]) -> Result<(grind_text::Document, Report)> {
    let (odf, report) = convert(bytes)?;
    let document = grind_text::read_bytes("import.fodt", &odf)
        .map_err(|e| Error::Xml(format!("the imported document would not read: {e}")))?;
    Ok((document, report))
}

/// What a **shell** opens: the document imported as a flat ODF text document, ready for
/// `App::open_bytes` under [`suggested_name`], with the report beside it — `grind_xlsx::open`'s
/// shape, so a shell's import is the same few lines for either application.
pub fn open(bytes: &[u8]) -> Result<(Vec<u8>, Report)> {
    convert(bytes)
}

/// The name an imported document goes by until it is saved: `letter.docx` becomes
/// `letter.fodt`, the flat form `doc/flat-first.md` makes every save dialog's default.
///
/// **A shell must never save back to the document's own path.** One way in, never out: an
/// imported document is a new, unsaved ODF document, and writing ODF into a file called `.docx`
/// would be the one way this filter could destroy a user's data.
pub fn suggested_name(document: &str) -> String {
    let stem = match document.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.contains(['/', '\\']) => stem,
        _ => document,
    };
    format!("{stem}.fodt")
}

/// The filesystem twin. [`import_bytes`] is the real function — rule 5, and the browser has no
/// filesystem.
pub fn import_file(path: &Path) -> Result<(grind_text::Document, Report)> {
    import_bytes(&std::fs::read(path)?)
}

/// Read a header or footer part.
fn story(ctx: &mut Ctx, part: &str) -> Option<Vec<Block>> {
    let bytes = ctx.package.part(part)?;
    ctx.enter(part);
    let mut reader = Reader::new(&bytes);
    reader.root().ok()??;
    let blocks = body::read_blocks(&mut reader, ctx).ok()?;
    Some(body::page_breaks(blocks))
}

/// Read a footnotes or endnotes part: every real note by id. The separators Word keeps in the
/// same part (`w:type="separator"`, `continuationSeparator`) are its own furniture and not
/// notes.
fn notes(ctx: &mut Ctx, part: &str) -> HashMap<String, Vec<Block>> {
    let mut out = HashMap::new();
    let Some(bytes) = ctx.package.part(part) else {
        return out;
    };
    ctx.enter(part);
    let mut reader = Reader::new(&bytes);
    if !matches!(reader.root(), Ok(Some(_))) {
        return out;
    }
    let _ = reader.children(|r, name, attrs| {
        if !(name.w("footnote") || name.w("endnote")) {
            return Ok(Handled::No);
        }
        if attrs.w("type").is_some_and(|t| t != "normal") {
            return Ok(Handled::No);
        }
        let Some(id) = attrs.w("id").map(str::to_owned) else {
            return Ok(Handled::No);
        };
        let blocks = body::read_blocks(r, ctx)?;
        out.insert(id, body::page_breaks(blocks));
        Ok(Handled::Yes)
    });
    out
}

/// Read the comments part: every comment by id, with its author, date and paragraphs.
fn comments(ctx: &mut Ctx, part: &str) -> HashMap<String, emit::Comment> {
    let mut out = HashMap::new();
    let Some(bytes) = ctx.package.part(part) else {
        return out;
    };
    ctx.enter(part);
    let mut reader = Reader::new(&bytes);
    if !matches!(reader.root(), Ok(Some(_))) {
        return out;
    }
    let _ = reader.children(|r, name, attrs| {
        if !name.w("comment") {
            return Ok(Handled::No);
        }
        let Some(id) = attrs.w("id").map(str::to_owned) else {
            return Ok(Handled::No);
        };
        let author = attrs.w("author").map(str::to_owned);
        let date = attrs.w("date").map(str::to_owned);
        let blocks = body::read_blocks(r, ctx)?;
        out.insert(
            id,
            emit::Comment {
                author,
                date,
                blocks: body::page_breaks(blocks),
            },
        );
        Ok(Handled::Yes)
    });
    out
}

/// What `word/settings.xml` says that this filter reads.
#[derive(Default)]
struct Settings {
    /// `w:defaultTabStop`, in twips.
    default_tab: Option<i64>,
    /// `w:evenAndOddHeaders` — even pages have headers and footers of their own.
    even_and_odd: bool,
}

fn settings(bytes: &[u8]) -> Settings {
    let mut out = Settings::default();
    let mut reader = Reader::new(bytes);
    if !matches!(reader.root(), Ok(Some(_))) {
        return out;
    }
    let _ = reader.children(|_, name, attrs| {
        if name.ns == Ns::Word {
            match name.local.as_str() {
                "defaultTabStop" => out.default_tab = attrs.int("val"),
                "evenAndOddHeaders" => out.even_and_odd = attrs.on(),
                _ => {}
            }
        }
        Ok(Handled::No)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_imported_document_is_offered_under_an_odf_name() {
        assert_eq!(suggested_name("letter.docx"), "letter.fodt");
        assert_eq!(suggested_name("/home/a/CV v2.docm"), "/home/a/CV v2.fodt");
        assert_eq!(suggested_name("no-extension"), "no-extension.fodt");
        assert_eq!(suggested_name("dir.v2/doc"), "dir.v2/doc.fodt");
    }

    #[test]
    fn nothing_that_is_not_a_package_sniffs_as_one() {
        assert!(!sniff(b""));
        assert!(!sniff(b"<?xml version=\"1.0\"?><office:document/>"));
        assert!(!sniff(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]));
    }

    #[test]
    fn a_locked_document_says_so_rather_than_claiming_to_be_junk() {
        let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        bytes.extend_from_slice(&[0u8; 512]);
        let err = import_bytes(&bytes).expect_err("a locked document is not importable");
        assert!(err.is_encrypted(), "got {err}");
    }
}
