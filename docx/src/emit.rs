// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The read Word document, written as one flat ODF text document (`.fodt`).
//!
//! **Why ODF and not the model** is `doc/docx-import.md`'s first decision, and this file is it.
//! `grind_xlsx` builds a `grind_sheet::Document` through the model's API and lets the sheet's
//! writer produce ODF, because nearly everything a workbook says has a field in that model. A
//! word-processing document is the other way round: its look lives in named styles, automatic
//! paragraph styles, list styles, a page layout and a master page with a header — all of which
//! `grind_text` *reads* (for showing and printing) and never *writes*, because a save carries
//! them out of the file it came from untouched (R6, `doc/odt-format.md`). So the import writes
//! the file a save would carry them out of. `grind_text::read_bytes` then reads it like any
//! other ODF document, and an imported document is, from that point on, indistinguishable from
//! one LibreOffice wrote: the same reader, the same saves, the same guards.
//!
//! Minimal by intent (R3): one flat document, no `meta`, no `settings`, no font declarations,
//! and every style written is one some paragraph, run, list or table uses. Deterministic: the
//! same `.docx` imports to the same bytes, which is what makes an import diffable at all.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use grind_core::odf::xml::esc;

use crate::body::{Block, Image, Inline, Para, Run, Table, VMerge};
use crate::numbering::{self, Numbering};
use crate::props::{self, ParaProps, Props, Tab};
use crate::report::{Dropped, Report};
use crate::section::{Section, Which};
use crate::styles::{self, Kind, Styles};

/// Everything read from the package, ready to write.
pub struct Input<'a> {
    pub styles: &'a Styles,
    pub numbering: &'a Numbering,
    pub body: Vec<Block>,
    pub sections: Vec<Section>,
    /// The first section's default header and footer, and its first-page pair when
    /// `w:titlePg` asks for one.
    /// Each section's page and the marginals in effect on it — its own references, or the
    /// section's before it (§17.10.5) — one per entry of `sections`.
    pub pages: Vec<Page>,
    pub footnotes: HashMap<String, Vec<Block>>,
    pub endnotes: HashMap<String, Vec<Block>>,
    /// Hidden bookmarks some hyperlink points at — kept; the rest of Word's `_`-names are not.
    pub anchors: HashSet<String>,
    /// `w:defaultTabStop`, in twips.
    pub default_tab: Option<i64>,
    /// Each font's generic family, from `word/fontTable.xml`.
    pub fonts: Vec<(String, &'static str)>,
}

/// A section's marginals: the default header and footer, and the first-page pair under
/// `w:titlePg`. Each is the part's name (so two sections showing the same header can share a
/// master page) and its blocks.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub header: Option<(String, Vec<Block>)>,
    pub footer: Option<(String, Vec<Block>)>,
    pub header_first: Option<(String, Vec<Block>)>,
    pub footer_first: Option<(String, Vec<Block>)>,
    /// The even-page pair, under `w:evenAndOddHeaders` — ODF's left pages.
    pub header_even: Option<(String, Vec<Block>)>,
    pub footer_even: Option<(String, Vec<Block>)>,
}

impl Page {
    fn parts(&self) -> [Option<&str>; 6] {
        [
            &self.header,
            &self.footer,
            &self.header_first,
            &self.footer_first,
            &self.header_even,
            &self.footer_even,
        ]
        .map(|m| m.as_ref().map(|(name, _)| name.as_str()))
    }
}

/// Write the document.
pub fn write(input: Input, report: &mut Report) -> String {
    let mut w = Writer {
        input: &input,
        report,
        paragraph_styles: BTreeMap::new(),
        text_styles: BTreeMap::new(),
        list_styles: BTreeMap::new(),
        tables: Vec::new(),
        cell_styles: BTreeMap::new(),
        row_styles: BTreeMap::new(),
        lists_seen: HashSet::new(),
        outline_list: None,
        footnotes: 0,
        endnotes: 0,
        notes_written: 0,
        images: 0,
        in_note: false,
    };
    let mut body = String::new();
    // One master page per distinct page — the size, the margins and the marginals — in section
    // order; the first is `Standard`, which is the one a reader takes as the document's.
    let mut masters: Vec<usize> = Vec::new();
    let mut master_of: Vec<String> = Vec::new();
    for (k, section) in input.sections.iter().enumerate() {
        let page = input.pages.get(k).cloned().unwrap_or_default();
        let found = masters.iter().position(|&m| {
            input.sections[m].same_page(section)
                && input.sections[m].landscape == section.landscape
                && input.pages.get(m).map(Page::parts).unwrap_or_default() == page.parts()
        });
        let index = match found {
            Some(index) => index,
            None => {
                masters.push(k);
                masters.len() - 1
            }
        };
        master_of.push(master_name(index));
    }
    if masters.is_empty() {
        masters.push(0);
    }
    let blocks = section_starts(&input.body, &input.sections, &master_of);
    w.outline_list = w.find_outline_list(&blocks);
    w.blocks(&mut body, &blocks, 3);
    w.report.styles = input
        .styles
        .by_id
        .values()
        .filter(|s| matches!(s.kind, Kind::Paragraph | Kind::Character))
        .count();

    // The marginals are written after the body so their styles join the same pools; they go
    // into the master pages, which are written last.
    let mut master_pages = String::new();
    for (index, &k) in masters.iter().enumerate() {
        let page = input.pages.get(k).cloned().unwrap_or_default();
        let _ = writeln!(
            master_pages,
            "  <style:master-page style:name=\"{}\" style:page-layout-name=\"pm{}\">",
            master_name(index),
            index + 1
        );
        // The schema's order, and its rule that a left or first-page variant needs the
        // default before it (rng:12140): a document with only a first-page header gets a
        // default one that is not displayed.
        for (kind, default, left, first) in [
            (
                "header",
                &page.header,
                &page.header_even,
                &page.header_first,
            ),
            (
                "footer",
                &page.footer,
                &page.footer_even,
                &page.footer_first,
            ),
        ] {
            if default.is_none() && (left.is_some() || first.is_some()) {
                let _ = writeln!(master_pages, "   <style:{kind} style:display=\"false\"/>");
            }
            for (tag, marginal) in [
                (kind.to_owned(), default),
                (format!("{kind}-left"), left),
                (format!("{kind}-first"), first),
            ] {
                if let Some((_, blocks)) = marginal {
                    let _ = writeln!(master_pages, "   <style:{tag}>");
                    w.blocks(&mut master_pages, blocks, 4);
                    let _ = writeln!(master_pages, "   </style:{tag}>");
                }
            }
        }
        master_pages.push_str("  </style:master-page>\n");
    }

    let mut out = String::new();
    out.push_str(PROLOG);
    if !input.fonts.is_empty() {
        out.push_str(" <office:font-face-decls>\n");
        for (name, generic) in &input.fonts {
            let _ = writeln!(
                out,
                "  <style:font-face style:name=\"{}\" svg:font-family=\"{}\" style:font-family-generic=\"{generic}\"/>",
                esc(name),
                esc(&props::family(name))
            );
        }
        out.push_str(" </office:font-face-decls>\n");
    }
    out.push_str(" <office:styles>\n");
    w.default_style(&mut out);
    w.named_styles(&mut out);
    w.outline_style(&mut out);
    out.push_str(" </office:styles>\n");
    out.push_str(" <office:automatic-styles>\n");
    for (index, &k) in masters.iter().enumerate() {
        let section = input.sections.get(k).cloned().unwrap_or_default();
        let page = input.pages.get(k).cloned().unwrap_or_default();
        w.page_layout(&mut out, &section, &page, index + 1);
    }
    w.automatic_styles(&mut out);
    out.push_str(" </office:automatic-styles>\n");
    out.push_str(" <office:master-styles>\n");
    out.push_str(&master_pages);
    out.push_str(" </office:master-styles>\n");
    out.push_str(" <office:body>\n  <office:text>\n");
    out.push_str(&body);
    out.push_str("  </office:text>\n </office:body>\n</office:document>\n");
    out
}

const PROLOG: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
"#;

/// The `n`th master page's name: `Standard` first, as every ODF producer names the default one.
fn master_name(n: usize) -> String {
    match n {
        0 => "Standard".to_owned(),
        n => format!("Section{}", n + 1),
    }
}

/// Mark every block that begins a section starting on a new page — and drop the empty
/// paragraph that does nothing but close a section continuing on the same page, as the oracle
/// does (`doc/docx-format.md` §5.3): in Word it is where the section break is drawn, and in a
/// document with one page layout it is an empty line that was never text.
///
/// And the first block of a section whose master page differs from the one before it names its
/// own (`Para::master_page`), which is how ODF starts a page on a different page style.
fn section_starts(blocks: &[Block], sections: &[Section], masters: &[String]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    let mut ended = 0;
    let mut pending = false;
    let last = blocks.len().saturating_sub(1);
    // Whether nothing has been read of the current section before this block.
    let mut fresh = true;
    // The master page a block starting a section has to name, until one has named it.
    let mut new_master: Option<String> = None;
    for (i, block) in blocks.iter().enumerate() {
        let mut block = block.clone();
        let section_is_empty = std::mem::replace(&mut fresh, false);
        match &mut block {
            Block::Para(p) => {
                p.page_break_before |= std::mem::take(&mut pending);
                if p.master_page.is_none() {
                    p.master_page = new_master.take();
                }
                if p.facts.section.is_some() {
                    fresh = true;
                    ended += 1;
                    pending = sections.get(ended).is_some_and(Section::starts_a_page);
                    if let (Some(now), Some(next)) = (masters.get(ended - 1), masters.get(ended))
                        && now != next
                    {
                        new_master = Some(next.clone());
                    }
                    // Only before a section that continues on the same page: before one that
                    // starts a new page the oracle keeps the paragraph, on the page it ends.
                    // And only when it is not the whole of its section, which would leave the
                    // section with nothing in it — the oracle keeps that one too.
                    if i < last
                        && p.inlines.is_empty()
                        && !pending
                        && !section_is_empty
                        && new_master.is_none()
                    {
                        // A page it was itself to start passes on to what follows.
                        pending |= p.page_break_before;
                        continue;
                    }
                }
            }
            Block::Table(t) => {
                t.page_break_before |= std::mem::take(&mut pending);
                if t.master_page.is_none() {
                    t.master_page = new_master.take();
                }
            }
        }
        out.push(block);
    }
    out
}

struct Writer<'i, 'r> {
    input: &'i Input<'i>,
    report: &'r mut Report,
    /// Automatic paragraph styles: (parent, properties) → name.
    /// Automatic paragraph styles: (parent, properties, master page) → name.
    paragraph_styles: BTreeMap<(Option<String>, ParaProps, Option<String>), String>,
    text_styles: BTreeMap<(Option<String>, Props), String>,
    /// numId → list style name.
    list_styles: BTreeMap<i64, String>,
    /// Each table's automatic styles, written in order.
    tables: Vec<String>,
    cell_styles: BTreeMap<Props, String>,
    row_styles: BTreeMap<Props, String>,
    /// The list that numbers the headings ([`Writer::find_outline_list`]).
    outline_list: Option<i64>,
    /// The lists already opened once, by numId — a later one continues it.
    lists_seen: HashSet<i64>,
    footnotes: usize,
    endnotes: usize,
    /// Every note written so far, numbered or not — what keeps each one's `text:id` unique.
    notes_written: usize,
    images: usize,
    in_note: bool,
}

/// Indentation, `n` levels deep — the body's own blocks at three, as the text writer indents.
fn pad(n: usize) -> String {
    " ".repeat(n)
}

impl Writer<'_, '_> {
    fn styles(&self) -> &Styles {
        self.input.styles
    }

    // ---- blocks ----

    fn blocks(&mut self, out: &mut String, blocks: &[Block], depth: usize) {
        // The lists open around the current position, outermost first: each one's numId.
        let mut open: Vec<i64> = Vec::new();
        for block in blocks {
            match block {
                Block::Para(para) => {
                    let item = self.list_item(para);
                    match item {
                        Some((num, level)) => {
                            let want = level + 1;
                            if open.first().is_some_and(|first| *first != num) {
                                close_lists(out, &mut open, 0, depth);
                            }
                            if open.len() > want {
                                close_lists(out, &mut open, want, depth);
                            }
                            if open.len() == want {
                                let at = depth + 2 * want - 1;
                                let _ = writeln!(out, "{}</text:list-item>", pad(at));
                                let _ = writeln!(out, "{}<text:list-item>", pad(at));
                            }
                            while open.len() < want {
                                let at = depth + 2 * open.len();
                                let attributes = if open.is_empty() {
                                    self.list_attributes(num)
                                } else {
                                    String::new()
                                };
                                let _ = writeln!(out, "{}<text:list{attributes}>", pad(at));
                                let _ = writeln!(out, "{}<text:list-item>", pad(at + 1));
                                open.push(num);
                            }
                            self.report.list_items += 1;
                            self.paragraph(out, para, depth + 2 * want);
                        }
                        None => {
                            close_lists(out, &mut open, 0, depth);
                            self.paragraph(out, para, depth);
                        }
                    }
                }
                Block::Table(table) => {
                    close_lists(out, &mut open, 0, depth);
                    self.table(out, table, depth);
                }
            }
        }
        close_lists(out, &mut open, 0, depth);
    }

    /// `(numId, ilvl)` for a paragraph that is a list item — numbered directly or by its style,
    /// with a numbering definition to show. A heading is never one: it keeps its level, and its
    /// number is counted.
    fn list_item(&mut self, para: &Para) -> Option<(i64, usize)> {
        let (num, level) = self.numbering_of(para)?;
        let num = num?;
        if num == 0 || !self.input.numbering.lists.contains_key(&num) {
            return None;
        }
        if let Some(outline) = self.outline(para) {
            // A heading keeps its level. Its number is the document's outline numbering when
            // it is numbered by the list that numbers the headings (`outline_list`), at its own
            // level; otherwise it is lost — unless it shows nothing anyway, which is how a
            // document with outline numbering switched off says it.
            let ilvl = level.unwrap_or(0).clamp(0, 8);
            let by_outline = self.outline_list == Some(num) && ilvl == outline;
            let shown = self.input.numbering.lists[&num]
                .level(ilvl as usize)
                .is_some_and(|l| l.format != "none" && !l.text.is_empty());
            if shown && !by_outline {
                self.report.drop_one(Dropped::HeadingNumber);
            }
            return None;
        }
        Some((num, level.unwrap_or(0).clamp(0, 8) as usize))
    }

    /// The opening attributes of a list: its style, and either an `xml:id` the first time this
    /// numbering is used or a `text:continue-list` naming that one — Word counts on across a
    /// paragraph that interrupts a list, and so must the document.
    fn list_attributes(&mut self, num: i64) -> String {
        let style = self.list_style(num);
        let id = format!("list{num}");
        if self.lists_seen.insert(num) {
            format!(" xml:id=\"{id}\" text:style-name=\"{style}\"")
        } else {
            format!(" text:style-name=\"{style}\" text:continue-list=\"{id}\"")
        }
    }

    fn list_style(&mut self, num: i64) -> String {
        let next = self.list_styles.len() + 1;
        self.list_styles
            .entry(num)
            .or_insert_with(|| format!("L{next}"))
            .clone()
    }

    /// A paragraph's `(numId, ilvl)`: each half its own if it states it, else its style
    /// chain's — and a level stated nowhere is the one the list itself ties to the paragraph's
    /// style (`w:lvl/w:pStyle`), or the first.
    fn numbering_of(&self, para: &Para) -> Option<(Option<i64>, Option<i64>)> {
        let style = self.style_id(para);
        let from_style = style.as_deref().and_then(|s| self.styles().numbering(s));
        let direct = para.facts.numbering;
        let num = direct
            .and_then(|(n, _)| n)
            .or_else(|| from_style.and_then(|(n, _)| n));
        let mut level = direct
            .and_then(|(_, l)| l)
            .or_else(|| from_style.and_then(|(_, l)| l));
        if direct.is_none() && from_style.is_none() {
            return None;
        }
        if level.is_none()
            && let (Some(num), Some(style)) = (num, style.as_deref())
        {
            level = self
                .input
                .numbering
                .lists
                .get(&num)
                .and_then(|list| {
                    list.levels
                        .iter()
                        .position(|l| l.style.as_deref() == Some(style))
                })
                .map(|i| i as i64);
        }
        Some((num, level))
    }

    /// The numbering a heading carries, `(numId, ilvl)`, shown — directly or by its style.
    fn heading_number(&self, para: &Para) -> Option<(i64, i64)> {
        self.outline(para)?;
        let (num, level) = self.numbering_of(para)?;
        let num = num.filter(|n| *n != 0)?;
        let ilvl = level.unwrap_or(0).clamp(0, 8);
        self.input
            .numbering
            .lists
            .get(&num)?
            .level(ilvl as usize)
            .filter(|l| l.format != "none" && !l.text.is_empty())?;
        Some((num, ilvl))
    }

    /// The list that numbers the document's headings, if one does consistently: the one most
    /// headings are numbered by, every one of them at the list level of its own outline level.
    /// It becomes ODF's outline numbering (`text:outline-style`), which numbers headings by
    /// level — the shape Word's numbered headings have.
    fn find_outline_list(&self, blocks: &[Block]) -> Option<i64> {
        let mut counts: BTreeMap<i64, (usize, bool)> = BTreeMap::new();
        for block in blocks {
            let Block::Para(para) = block else { continue };
            let (Some((num, ilvl)), Some(outline)) =
                (self.heading_number(para), self.outline(para))
            else {
                continue;
            };
            let entry = counts.entry(num).or_insert((0, true));
            entry.0 += 1;
            entry.1 &= ilvl == outline;
        }
        counts
            .into_iter()
            .filter(|(_, (_, consistent))| *consistent)
            .max_by_key(|(num, (count, _))| (*count, std::cmp::Reverse(*num)))
            .map(|(num, _)| num)
    }

    /// Whether the outline numbering would number a heading at `outline` that Word did not.
    fn unnumbered_at_numbered_level(&self, para: &Para, outline: i64) -> bool {
        let Some(list) = self
            .outline_list
            .and_then(|n| self.input.numbering.lists.get(&n))
        else {
            return false;
        };
        let numbered_level = list
            .level(outline as usize)
            .is_some_and(|l| l.format != "none" && !l.text.is_empty());
        numbered_level
            && self
                .heading_number(para)
                .is_none_or(|(num, ilvl)| Some(num) != self.outline_list || ilvl != outline)
    }

    /// The paragraph's style id: its own, or the document's default paragraph style.
    fn style_id(&self, para: &Para) -> Option<String> {
        para.facts
            .style
            .clone()
            .filter(|id| self.styles().get(id).is_some())
            .or_else(|| self.styles().default_paragraph.clone())
    }

    /// The paragraph's outline level, 0-based, if it is a heading.
    fn outline(&self, para: &Para) -> Option<i64> {
        match para.facts.outline {
            Some(level) => (0..9).contains(&level).then_some(level),
            None => self
                .style_id(para)
                .and_then(|id| self.styles().outline(&id)),
        }
    }

    fn paragraph(&mut self, out: &mut String, para: &Para, depth: usize) {
        let style_id = self.style_id(para);
        let named = style_id
            .as_deref()
            .and_then(|id| self.styles().odf_name(id))
            .map(str::to_owned);
        let mut direct = para.facts.props.clone();
        if para.page_break_before {
            direct.para.set("fo:break-before", "page");
        }
        // Word measures a tab stop from the page margin and ODF from the paragraph's indent.
        if let Some(tabs) = &direct.tabs {
            let indent = self.left_indent(style_id.as_deref(), &direct);
            direct.tabs = Some(shift_tabs(tabs, indent));
        }
        let style = if direct.is_empty() && para.master_page.is_none() {
            named
        } else {
            Some(self.paragraph_style(named, direct, para.master_page.clone()))
        };
        let (tag, level) = match self.outline(para) {
            Some(level) => {
                self.report.headings += 1;
                ("text:h", Some(level + 1))
            }
            None => ("text:p", None),
        };
        self.report.paragraphs += 1;
        let _ = write!(out, "{}<{tag}", pad(depth));
        if let Some(style) = &style {
            let _ = write!(out, " text:style-name=\"{}\"", esc(style));
        }
        if let Some(level) = level {
            let _ = write!(out, " text:outline-level=\"{level}\"");
            // A heading the outline numbering would number and Word did not stays unnumbered.
            if self.unnumbered_at_numbered_level(para, i64::from(level) - 1) {
                out.push_str(" text:is-list-header=\"true\"");
            }
        }
        let mut content = String::new();
        self.inlines(&mut content, &para.inlines);
        if content.is_empty() {
            out.push_str("/>\n");
        } else {
            let _ = writeln!(out, ">{content}</{tag}>");
        }
    }

    /// The left indent a paragraph's tab stops are measured from, in twips.
    fn left_indent(&self, style: Option<&str>, direct: &ParaProps) -> i64 {
        let mut resolved = self.styles().defaults.clone();
        if let Some(style) = style {
            resolved.layer(&styles::resolved(self.styles(), style));
        }
        resolved.layer(direct);
        resolved
            .para
            .get("fo:margin-left")
            .and_then(points)
            .map_or(0, |pt| (pt * 20.0).round() as i64)
    }

    fn paragraph_style(
        &mut self,
        parent: Option<String>,
        props: ParaProps,
        master: Option<String>,
    ) -> String {
        let next = self.paragraph_styles.len() + 1;
        self.paragraph_styles
            .entry((parent, props, master))
            .or_insert_with(|| format!("P{next}"))
            .clone()
    }

    fn text_style(&mut self, run: &Run) -> Option<String> {
        let parent = run
            .style
            .as_deref()
            .and_then(|id| self.styles().odf_name(id))
            .map(str::to_owned);
        if run.props.is_empty() {
            return parent;
        }
        let next = self.text_styles.len() + 1;
        Some(
            self.text_styles
                .entry((parent, run.props.clone()))
                .or_insert_with(|| format!("T{next}"))
                .clone(),
        )
    }

    // ---- inlines ----

    fn inlines(&mut self, out: &mut String, inlines: &[Inline]) {
        // Group by hyperlink: a `text:a` around every stretch of runs naming the same target.
        let href_of = |inline: &Inline| -> Option<String> {
            match inline {
                Inline::Text(_, run)
                | Inline::Tab(run)
                | Inline::Break(run)
                | Inline::PageNumber(run)
                | Inline::PageCount(run) => run.href.clone(),
                _ => None,
            }
        };
        // Whether the last character written was a space, or nothing yet — a space there must
        // be `text:s`, because XML collapses it (`doc/odt-format.md` §3.3).
        let mut space = true;
        let mut i = 0;
        while i < inlines.len() {
            let href = href_of(&inlines[i]);
            let mut j = i + 1;
            while j < inlines.len() && href.is_some() && href_of(&inlines[j]) == href {
                j += 1;
            }
            if let Some(href) = &href {
                let _ = write!(
                    out,
                    "<text:a xlink:type=\"simple\" xlink:href=\"{}\">",
                    esc(href)
                );
            }
            for inline in &inlines[i..j] {
                self.inline(out, inline, &mut space);
            }
            if href.is_some() {
                out.push_str("</text:a>");
            }
            i = j;
        }
    }

    fn inline(&mut self, out: &mut String, inline: &Inline, space: &mut bool) {
        match inline {
            Inline::Text(text, run) => {
                let style = self.text_style(run);
                let mut body = String::new();
                encode(&mut body, text, space);
                match style {
                    Some(style) => {
                        let _ = write!(
                            out,
                            "<text:span text:style-name=\"{}\">{body}</text:span>",
                            esc(&style)
                        );
                    }
                    None => out.push_str(&body),
                }
            }
            Inline::Tab(_) => {
                out.push_str("<text:tab/>");
                *space = false;
            }
            Inline::Break(_) => {
                out.push_str("<text:line-break/>");
                *space = true;
            }
            Inline::PageBreak => {}
            Inline::Bookmark(name) => {
                if !name.starts_with('_') || self.input.anchors.contains(name) {
                    let _ = write!(out, "<text:bookmark text:name=\"{}\"/>", esc(name));
                }
            }
            Inline::PageNumber(run) | Inline::PageCount(run) => {
                let tag = if matches!(inline, Inline::PageNumber(_)) {
                    "text:page-number"
                } else {
                    "text:page-count"
                };
                let field = format!("<{tag} text:select-page=\"current\">1</{tag}>");
                let field = if tag == "text:page-count" {
                    format!("<{tag}>1</{tag}>")
                } else {
                    field
                };
                match self.text_style(run) {
                    Some(style) => {
                        let _ = write!(
                            out,
                            "<text:span text:style-name=\"{}\">{field}</text:span>",
                            esc(&style)
                        );
                    }
                    None => out.push_str(&field),
                }
                *space = false;
            }
            Inline::Note {
                endnote,
                id,
                run,
                mark,
            } => self.note(out, *endnote, id, run, mark.as_deref(), space),
            Inline::Image(image) => {
                self.image(out, image);
                *space = false;
            }
        }
    }

    fn note(
        &mut self,
        out: &mut String,
        endnote: bool,
        id: &str,
        run: &Run,
        mark: Option<&str>,
        space: &mut bool,
    ) {
        // A note inside a note is not valid ODF (rng:8465 excludes it), and Word does not write
        // one either; a malformed file's is dropped rather than nested.
        if self.in_note {
            return;
        }
        let notes = if endnote {
            &self.input.endnotes
        } else {
            &self.input.footnotes
        };
        let Some(blocks) = notes.get(id) else {
            return;
        };
        let mut blocks = blocks.clone();
        trim_leading_space(&mut blocks);
        // A note with a mark of its own takes no number, and the next numbered one carries on
        // from the last (measured, `doc/docx-format.md` §2.5).
        let mark = mark.filter(|m| !m.is_empty());
        self.notes_written += 1;
        let number = self.notes_written;
        let class = if endnote { "endnote" } else { "footnote" };
        let citation = match mark {
            Some(mark) => mark.to_owned(),
            None if endnote => {
                self.endnotes += 1;
                roman(self.endnotes)
            }
            None => {
                self.footnotes += 1;
                self.footnotes.to_string()
            }
        };
        self.report.notes += 1;
        let prefix = if endnote { "edn" } else { "ftn" };
        let style = self.text_style(run);
        if let Some(style) = &style {
            let _ = write!(out, "<text:span text:style-name=\"{}\">", esc(style));
        }
        let label = match mark {
            Some(mark) => format!(" text:label=\"{}\"", esc(mark)),
            None => String::new(),
        };
        let _ = write!(
            out,
            "<text:note text:id=\"{prefix}{number}\" text:note-class=\"{class}\">\
             <text:note-citation{label}>{}</text:note-citation><text:note-body>",
            esc(&citation)
        );
        self.in_note = true;
        let mut body = String::new();
        self.blocks(&mut body, &blocks, 0);
        self.in_note = false;
        // A note body is written on one line: whitespace between its paragraphs would be text
        // in the paragraph that holds the note.
        for line in body.lines() {
            out.push_str(line.trim_start());
        }
        out.push_str("</text:note-body></text:note>");
        if style.is_some() {
            out.push_str("</text:span>");
        }
        *space = false;
    }

    fn image(&mut self, out: &mut String, image: &Image) {
        use base64::Engine as _;
        self.images += 1;
        self.report.images += 1;
        let anchor = if image.inline { "as-char" } else { "char" };
        let _ = write!(out, "<draw:frame text:anchor-type=\"{anchor}\"");
        if let Some(width) = image.width.filter(|w| *w > 0) {
            let _ = write!(out, " svg:width=\"{}\"", props::emu(width));
        }
        if let Some(height) = image.height.filter(|h| *h > 0) {
            let _ = write!(out, " svg:height=\"{}\"", props::emu(height));
        }
        let _ = write!(
            out,
            "><draw:image draw:mime-type=\"{}\"><office:binary-data>{}</office:binary-data></draw:image></draw:frame>",
            esc(&image.mime),
            base64::engine::general_purpose::STANDARD.encode(&image.data)
        );
    }

    // ---- tables ----

    fn table(&mut self, out: &mut String, table: &Table, depth: usize) {
        self.report.tables += 1;
        let n = self.report.tables;
        let name = format!("Table{n}");
        let style_id = table
            .style
            .clone()
            .filter(|id| self.styles().get(id).is_some())
            .or_else(|| self.styles().default_table.clone());
        let mut look = style_id
            .as_deref()
            .map(|id| self.styles().table_look(id))
            .unwrap_or_default();
        if look.conditional {
            self.report.drop_one(Dropped::TableStyleCondition);
        }
        for (k, v) in &table.look.borders {
            look.borders.insert(k, v.clone());
        }
        for (k, v) in &table.look.margins {
            look.margins.insert(k, *v);
        }
        if table.look.fill.is_some() {
            look.fill.clone_from(&table.look.fill);
        }

        // The grid: Word's own column widths, or as many columns as the widest row asks for.
        let widest = table
            .rows
            .iter()
            .map(|row| {
                row.before as usize
                    + row.after as usize
                    + row.cells.iter().map(|c| c.span as usize).sum::<usize>()
            })
            .max()
            .unwrap_or(0);
        let columns = table.grid.len().max(widest).max(1);

        // Table and column styles.
        let mut styles = String::new();
        let width: i64 = table.grid.iter().sum();
        let mut table_props = Props::default();
        if width > 0 {
            table_props.set("style:width", props::twips(width));
        }
        table_props.set(
            "table:align",
            match table.align {
                Some("center") => "center",
                Some("end") => "right",
                _ => "left",
            },
        );
        if table.page_break_before {
            table_props.set("fo:break-before", "page");
        }
        table_props.set("table:border-model", "collapsing");
        let master = table
            .master_page
            .as_deref()
            .map(|m| format!(" style:master-page-name=\"{}\"", esc(m)))
            .unwrap_or_default();
        let _ = writeln!(
            styles,
            "  <style:style style:name=\"{name}\" style:family=\"table\"{master}><style:table-properties{}/></style:style>",
            table_props.attributes()
        );
        for (i, w) in table.grid.iter().enumerate() {
            let _ = writeln!(
                styles,
                "  <style:style style:name=\"{name}.C{}\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"{}\"/></style:style>",
                i + 1,
                props::twips(*w)
            );
        }
        self.tables.push(styles);

        // Where each cell sits, and how far down each vertical merge reaches.
        let positions: Vec<Vec<usize>> = table
            .rows
            .iter()
            .map(|row| {
                let mut at = row.before as usize;
                row.cells
                    .iter()
                    .map(|cell| {
                        let here = at;
                        at += cell.span as usize;
                        here
                    })
                    .collect()
            })
            .collect();
        let continues = |r: usize, column: usize| -> bool {
            table.rows.get(r).is_some_and(|row| {
                row.cells
                    .iter()
                    .zip(&positions[r])
                    .any(|(cell, at)| *at == column && cell.vmerge == VMerge::Continue)
            })
        };

        let _ = writeln!(
            out,
            "{}<table:table table:name=\"{name}\" table:style-name=\"{name}\">",
            pad(depth)
        );
        if table.grid.is_empty() {
            let _ = writeln!(
                out,
                "{}<table:table-column table:number-columns-repeated=\"{columns}\"/>",
                pad(depth + 1)
            );
        } else {
            for i in 0..table.grid.len() {
                let _ = writeln!(
                    out,
                    "{}<table:table-column table:style-name=\"{name}.C{}\"/>",
                    pad(depth + 1),
                    i + 1
                );
            }
            if columns > table.grid.len() {
                let _ = writeln!(
                    out,
                    "{}<table:table-column table:number-columns-repeated=\"{}\"/>",
                    pad(depth + 1),
                    columns - table.grid.len()
                );
            }
        }
        // Only the rows at the top can repeat on every page — `table:table-header-rows` is one
        // group at the start (rng:13931), as Word's own repeated rows are.
        let headers = table.rows.iter().take_while(|row| row.header).count();
        let rows = table.rows.len();
        for (r, row) in table.rows.iter().enumerate() {
            if r == 0 && headers > 0 {
                let _ = writeln!(out, "{}<table:table-header-rows>", pad(depth + 1));
            }
            let inner = depth + 1 + usize::from(r < headers);
            let mut row_props = Props::default();
            if let Some((height, exact)) = row.height {
                row_props.set(
                    if exact {
                        "style:row-height"
                    } else {
                        "style:min-row-height"
                    },
                    props::twips(height),
                );
            }
            let _ = write!(out, "{}<table:table-row", pad(inner));
            if !row_props.is_empty() {
                let style = self.row_style(row_props);
                let _ = write!(out, " table:style-name=\"{style}\"");
            }
            out.push_str(">\n");
            let mut at = 0usize;
            let empty_cell = |out: &mut String| {
                let _ = writeln!(
                    out,
                    "{}<table:table-cell office:value-type=\"string\"><text:p/></table:table-cell>",
                    pad(inner + 1)
                );
            };
            for _ in 0..row.before {
                empty_cell(out);
                at += 1;
            }
            for (cell, column) in row.cells.iter().zip(&positions[r]) {
                let column = *column;
                let span = cell.span as usize;
                at = column + span;
                if cell.vmerge == VMerge::Continue && r > 0 {
                    for _ in 0..span {
                        let _ = writeln!(out, "{}<table:covered-table-cell/>", pad(inner + 1));
                    }
                    continue;
                }
                let mut down = 1;
                if cell.vmerge == VMerge::Restart {
                    while continues(r + down, column) {
                        down += 1;
                    }
                }
                let last_row = r + down >= rows;
                let cell_props = cell_look(
                    cell,
                    &look,
                    r == 0,
                    last_row,
                    column == 0,
                    column + span >= columns,
                );
                let style = self.cell_style(cell_props);
                let _ = write!(
                    out,
                    "{}<table:table-cell table:style-name=\"{style}\" office:value-type=\"string\"",
                    pad(inner + 1)
                );
                if span > 1 {
                    let _ = write!(out, " table:number-columns-spanned=\"{span}\"");
                }
                if down > 1 {
                    let _ = write!(out, " table:number-rows-spanned=\"{down}\"");
                }
                out.push_str(">\n");
                if cell.blocks.is_empty() {
                    let _ = writeln!(out, "{}<text:p/>", pad(inner + 2));
                } else {
                    self.blocks(out, &cell.blocks, inner + 2);
                }
                let _ = writeln!(out, "{}</table:table-cell>", pad(inner + 1));
                for _ in 1..span {
                    let _ = writeln!(out, "{}<table:covered-table-cell/>", pad(inner + 1));
                }
            }
            while at < columns {
                empty_cell(out);
                at += 1;
            }
            let _ = writeln!(out, "{}</table:table-row>", pad(inner));
            if r + 1 == headers {
                let _ = writeln!(out, "{}</table:table-header-rows>", pad(depth + 1));
            }
        }
        let _ = writeln!(out, "{}</table:table>", pad(depth));
    }

    fn cell_style(&mut self, props: Props) -> String {
        let next = self.cell_styles.len() + 1;
        self.cell_styles
            .entry(props)
            .or_insert_with(|| format!("Cell{next}"))
            .clone()
    }

    fn row_style(&mut self, props: Props) -> String {
        let next = self.row_styles.len() + 1;
        self.row_styles
            .entry(props)
            .or_insert_with(|| format!("Row{next}"))
            .clone()
    }

    // ---- styles ----

    fn default_style(&self, out: &mut String) {
        let mut defaults = self.styles().defaults.clone();
        if let Some(tab) = self.input.default_tab.filter(|t| *t > 0) {
            defaults
                .para
                .set("style:tab-stop-distance", props::twips(tab));
        }
        let _ = writeln!(out, "  <style:default-style style:family=\"paragraph\">");
        write_props(out, &defaults, "   ");
        out.push_str("  </style:default-style>\n");
    }

    fn named_styles(&self, out: &mut String) {
        let styles = self.styles();
        for id in &styles.order {
            let style = &styles.by_id[id];
            let family = match style.kind {
                Kind::Paragraph => "paragraph",
                Kind::Character => "text",
                _ => continue,
            };
            let Some(name) = styles.odf_name(id) else {
                continue;
            };
            let display = styles::display_name(style);
            let _ = write!(
                out,
                "  <style:style style:name=\"{}\" style:family=\"{family}\"",
                esc(name)
            );
            if display != name {
                let _ = write!(out, " style:display-name=\"{}\"", esc(&display));
            }
            if let Some(parent) = style
                .based_on
                .as_deref()
                .filter(|p| styles.get(p).is_some_and(|p| p.kind == style.kind))
                .and_then(|p| styles.odf_name(p))
            {
                let _ = write!(out, " style:parent-style-name=\"{}\"", esc(parent));
            }
            if let Some(next) = style
                .next
                .as_deref()
                .filter(|_| style.kind == Kind::Paragraph)
                .and_then(|n| styles.odf_name(n))
            {
                let _ = write!(out, " style:next-style-name=\"{}\"", esc(next));
            }
            if style.kind == Kind::Paragraph
                && let Some(level) = styles.outline(id)
            {
                let _ = write!(out, " style:default-outline-level=\"{}\"", level + 1);
            }
            let mut props = style.props.clone();
            if style.kind == Kind::Character {
                props.para = Props::default();
                props.tabs = None;
            } else if let Some(tabs) = &props.tabs {
                let indent = self.left_indent(Some(id), &ParaProps::default());
                props.tabs = Some(shift_tabs(tabs, indent));
            }
            if props.is_empty() {
                out.push_str("/>\n");
            } else {
                out.push_str(">\n");
                write_props(out, &props, "   ");
                out.push_str("  </style:style>\n");
            }
        }
    }

    fn page_layout(&self, out: &mut String, s: &Section, page: &Page, number: usize) {
        // Word's own default page when a document states none: US Letter, one-inch margins
        // (§17.6.13's and §17.6.11's absent-element behaviour is Word's application default —
        // `doc/docx-format.md` §5.1).
        let width = s.width.unwrap_or(12_240);
        let height = s.height.unwrap_or(15_840);
        let top = s.top.unwrap_or(1440).abs();
        let bottom = s.bottom.unwrap_or(1440).abs();
        let left = s.left.unwrap_or(1440) + s.gutter.unwrap_or(0);
        let right = s.right.unwrap_or(1440);
        let has_header =
            page.header.is_some() || page.header_first.is_some() || page.header_even.is_some();
        let has_footer =
            page.footer.is_some() || page.footer_first.is_some() || page.footer_even.is_some();
        // With a header, ODF's page margin runs to the *header*, and the header's own height
        // takes up the rest of the way to the body — Word's `w:header` is that first distance
        // and `w:top` the whole of it (`doc/docx-format.md` §5.2).
        let header_at = s.header.unwrap_or(720).clamp(0, top);
        let footer_at = s.footer.unwrap_or(720).clamp(0, bottom);
        let mut p = Props::default();
        p.set("fo:page-width", props::twips(width));
        p.set("fo:page-height", props::twips(height));
        p.set(
            "style:print-orientation",
            if s.landscape { "landscape" } else { "portrait" },
        );
        p.set(
            "fo:margin-top",
            props::twips(if has_header { header_at } else { top }),
        );
        p.set(
            "fo:margin-bottom",
            props::twips(if has_footer { footer_at } else { bottom }),
        );
        p.set("fo:margin-left", props::twips(left));
        p.set("fo:margin-right", props::twips(right));
        let _ = writeln!(
            out,
            "  <style:page-layout style:name=\"pm{number}\"><style:page-layout-properties{}/>",
            p.attributes()
        );
        if has_header {
            let _ = writeln!(
                out,
                "   <style:header-style><style:header-footer-properties fo:min-height=\"{}\" fo:margin-bottom=\"0pt\"/></style:header-style>",
                props::twips(top - header_at)
            );
        }
        if has_footer {
            let _ = writeln!(
                out,
                "   <style:footer-style><style:header-footer-properties fo:min-height=\"{}\" fo:margin-top=\"0pt\"/></style:footer-style>",
                props::twips(bottom - footer_at)
            );
        }
        out.push_str("  </style:page-layout>\n");
    }

    fn automatic_styles(&mut self, out: &mut String) {
        for ((parent, props, master), name) in by_name(&self.paragraph_styles) {
            let _ = write!(
                out,
                "  <style:style style:name=\"{name}\" style:family=\"paragraph\""
            );
            if let Some(parent) = parent {
                let _ = write!(out, " style:parent-style-name=\"{}\"", esc(parent));
            }
            if let Some(master) = master {
                let _ = write!(out, " style:master-page-name=\"{}\"", esc(master));
            }
            out.push_str(">\n");
            write_props(out, props, "   ");
            out.push_str("  </style:style>\n");
        }
        for ((parent, props), name) in by_name(&self.text_styles) {
            let _ = write!(
                out,
                "  <style:style style:name=\"{name}\" style:family=\"text\""
            );
            if let Some(parent) = parent {
                let _ = write!(out, " style:parent-style-name=\"{}\"", esc(parent));
            }
            let _ = writeln!(
                out,
                "><style:text-properties{}/></style:style>",
                props.attributes()
            );
        }
        for (num, name) in self.list_styles.clone() {
            self.list_style_definition(out, num, &name);
        }
        for styles in &self.tables {
            out.push_str(styles);
        }
        for (props, name) in by_name(&self.cell_styles) {
            let _ = writeln!(
                out,
                "  <style:style style:name=\"{name}\" style:family=\"table-cell\"><style:table-cell-properties{}/></style:style>",
                props.attributes()
            );
        }
        for (props, name) in by_name(&self.row_styles) {
            let _ = writeln!(
                out,
                "  <style:style style:name=\"{name}\" style:family=\"table-row\"><style:table-row-properties{}/></style:style>",
                props.attributes()
            );
        }
    }

    /// `text:outline-style` from the list that numbers the headings — nothing when none does.
    fn outline_style(&self, out: &mut String) {
        let Some(list) = self
            .outline_list
            .and_then(|n| self.input.numbering.lists.get(&n))
        else {
            return;
        };
        out.push_str("  <text:outline-style style:name=\"Outline\">\n");
        for (i, level) in list.levels.iter().enumerate() {
            let n = i + 1;
            let (prefix, suffix, shown) = numbering::label_parts(&level.text);
            let format = if level.format == "bullet" || level.text.is_empty() {
                ""
            } else {
                numbering::num_format(&level.format)
            };
            let mut attributes = format!(" style:num-format=\"{format}\"");
            if !format.is_empty() {
                if !prefix.is_empty() {
                    let _ = write!(attributes, " style:num-prefix=\"{}\"", esc(&prefix));
                }
                if !suffix.is_empty() {
                    let _ = write!(attributes, " style:num-suffix=\"{}\"", esc(&suffix));
                }
                if shown > 1 {
                    let _ = write!(attributes, " text:display-levels=\"{}\"", shown.min(n));
                }
                if level.start != 1 {
                    let _ = write!(attributes, " text:start-value=\"{}\"", level.start.max(0));
                }
            }
            let _ = writeln!(
                out,
                "   <text:outline-level-style text:level=\"{n}\"{attributes}/>"
            );
        }
        out.push_str("  </text:outline-style>\n");
    }

    fn list_style_definition(&self, out: &mut String, num: i64, name: &str) {
        let Some(list) = self.input.numbering.lists.get(&num) else {
            return;
        };
        let _ = writeln!(out, "  <text:list-style style:name=\"{name}\">");
        for (i, level) in list.levels.iter().enumerate() {
            let n = i + 1;
            let left = level.left.unwrap_or(720 * n as i64);
            let hanging = level.hanging.unwrap_or(360);
            let position = format!(
                "<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\"><style:list-level-label-alignment text:label-followed-by=\"listtab\" text:list-tab-stop-position=\"{}\" fo:text-indent=\"{}\" fo:margin-left=\"{}\"/></style:list-level-properties>",
                props::twips(left),
                props::twips(-hanging),
                props::twips(left)
            );
            if level.format == "bullet" {
                let bullet = numbering::bullet(&level.text);
                let _ = writeln!(
                    out,
                    "   <text:list-level-style-bullet text:level=\"{n}\" text:bullet-char=\"{}\">{position}</text:list-level-style-bullet>",
                    esc(&bullet.to_string())
                );
            } else {
                let (prefix, suffix, shown) = numbering::label_parts(&level.text);
                let mut attributes = format!(
                    " style:num-format=\"{}\"",
                    numbering::num_format(&level.format)
                );
                if !prefix.is_empty() {
                    let _ = write!(attributes, " style:num-prefix=\"{}\"", esc(&prefix));
                }
                if !suffix.is_empty() {
                    let _ = write!(attributes, " style:num-suffix=\"{}\"", esc(&suffix));
                }
                if shown > 1 {
                    let _ = write!(attributes, " text:display-levels=\"{}\"", shown.min(n));
                }
                if level.start != 1 {
                    let _ = write!(attributes, " text:start-value=\"{}\"", level.start.max(0));
                }
                let _ = writeln!(
                    out,
                    "   <text:list-level-style-number text:level=\"{n}\"{attributes}>{position}</text:list-level-style-number>"
                );
            }
        }
        out.push_str("  </text:list-style>\n");
    }
}

/// Close lists until only `keep` remain open.
fn close_lists(out: &mut String, open: &mut Vec<i64>, keep: usize, depth: usize) {
    while open.len() > keep {
        let level = open.len();
        let at = depth + 2 * (level - 1);
        let _ = writeln!(out, "{}</text:list-item>", pad(at + 1));
        let _ = writeln!(out, "{}</text:list>", pad(at));
        open.pop();
    }
}

/// A pool's entries in the order their names were handed out — `P1`, `P2`, … — so the styles
/// are written in a stable order that reads like the document.
fn by_name<K>(pool: &BTreeMap<K, String>) -> Vec<(&K, &String)> {
    let mut entries: Vec<_> = pool.iter().collect();
    entries.sort_by_key(|(_, name)| {
        let digits: String = name.chars().filter(char::is_ascii_digit).collect();
        digits.parse::<usize>().unwrap_or(0)
    });
    entries
}

/// A paragraph style's properties: `style:paragraph-properties` with its tab stops, then
/// `style:text-properties`.
fn write_props(out: &mut String, props: &ParaProps, indent: &str) {
    if !props.para.is_empty() || props.tabs.is_some() {
        let _ = write!(
            out,
            "{indent}<style:paragraph-properties{}",
            props.para.attributes()
        );
        match &props.tabs {
            Some(tabs) if !tabs.is_empty() => {
                out.push_str("><style:tab-stops>");
                for tab in tabs {
                    let _ = write!(
                        out,
                        "<style:tab-stop style:position=\"{}\" style:type=\"{}\"",
                        props::twips(tab.position),
                        tab.kind
                    );
                    if tab.kind == "char" {
                        out.push_str(" style:char=\".\"");
                    }
                    if let Some(leader) = tab.leader {
                        let _ = write!(
                            out,
                            " style:leader-style=\"solid\" style:leader-text=\"{}\"",
                            esc(&leader.to_string())
                        );
                    }
                    out.push_str("/>");
                }
                out.push_str("</style:tab-stops></style:paragraph-properties>\n");
            }
            _ => out.push_str("/>\n"),
        }
    }
    if !props.text.is_empty() {
        let _ = writeln!(
            out,
            "{indent}<style:text-properties{}/>",
            props.text.attributes()
        );
    }
}

/// Tab stops measured from the page margin, re-measured from an indent of `indent` twips.
fn shift_tabs(tabs: &[Tab], indent: i64) -> Vec<Tab> {
    tabs.iter()
        .filter(|t| !t.clear)
        .map(|t| Tab {
            position: t.position - indent,
            ..t.clone()
        })
        .filter(|t| t.position >= 0)
        .collect()
}

/// `36pt` → `36.0`.
fn points(length: &str) -> Option<f64> {
    length.trim().strip_suffix("pt")?.trim().parse().ok()
}

/// A cell's look: its own borders, fill, alignment and margins over the table's.
fn cell_look(
    cell: &crate::body::Cell,
    table: &styles::TableLook,
    top: bool,
    bottom: bool,
    left: bool,
    right: bool,
) -> Props {
    let mut p = Props::default();
    let side = |own: &str, edge: bool, outer: &str, inner: &str| -> Option<String> {
        match cell.borders.get(own) {
            Some(line) => line.clone(),
            None => table
                .borders
                .get(if edge { outer } else { inner })
                .cloned()
                .flatten(),
        }
    };
    for (key, line) in [
        ("fo:border-top", side("top", top, "top", "insideH")),
        (
            "fo:border-bottom",
            side("bottom", bottom, "bottom", "insideH"),
        ),
        ("fo:border-left", side("left", left, "left", "insideV")),
        ("fo:border-right", side("right", right, "right", "insideV")),
    ] {
        p.set(key, line.unwrap_or_else(|| "none".into()));
    }
    if let Some(fill) = cell.fill.clone().or_else(|| table.fill.clone()) {
        p.set("fo:background-color", fill);
    }
    if let Some(valign) = cell.valign {
        p.set("style:vertical-align", valign);
    }
    // Word's cell margins: the cell's own, then the table's, then what is assumed — 108 twips
    // left and right, nothing above and below, measured on the oracle (`doc/docx-format.md`
    // §6.2).
    for (side, key, fallback) in [
        ("top", "fo:padding-top", 0),
        ("bottom", "fo:padding-bottom", 0),
        ("left", "fo:padding-left", 108),
        ("right", "fo:padding-right", 108),
    ] {
        let twips = cell
            .margins
            .get(side)
            .or_else(|| table.margins.get(side))
            .copied()
            .unwrap_or(fallback);
        p.set(key, props::twips(twips.max(0)));
    }
    p
}

/// Text, with every piece of significant whitespace as the element ODF has for it — `text:s`
/// for a space XML would collapse, `text:tab`, `text:line-break` (`doc/odt-format.md` §3.3).
fn encode(out: &mut String, text: &str, space: &mut bool) {
    let mut run = 0usize;
    let flush = |out: &mut String, run: &mut usize| {
        match *run {
            0 => {}
            1 => out.push_str("<text:s/>"),
            n => {
                let _ = write!(out, "<text:s text:c=\"{n}\"/>");
            }
        }
        *run = 0;
    };
    for c in text.chars() {
        match c {
            ' ' if *space => run += 1,
            ' ' => {
                out.push(' ');
                *space = true;
            }
            '\t' => {
                flush(out, &mut run);
                out.push_str("<text:tab/>");
                *space = false;
            }
            '\n' | '\r' => {
                flush(out, &mut run);
                out.push_str("<text:line-break/>");
                *space = true;
            }
            // Characters XML 1.0 cannot carry at all — a control character a producer let
            // through — are dropped rather than making the whole document unreadable.
            c if (c as u32) < 0x20 => {}
            c => {
                flush(out, &mut run);
                out.push_str(&esc(&c.to_string()));
                *space = false;
            }
        }
    }
    flush(out, &mut run);
}

/// Drop the space Word puts between a note's own number and its text: ODF draws the citation
/// apart from the body, and a leading space there would be an indent.
fn trim_leading_space(blocks: &mut [Block]) {
    if let Some(Block::Para(first)) = blocks.first_mut()
        && let Some(Inline::Text(text, _)) = first
            .inlines
            .iter_mut()
            .find(|i| !matches!(i, Inline::Bookmark(_)))
    {
        *text = text.trim_start().to_owned();
    }
}

/// Lower-case roman numerals — Word's default endnote numbering.
fn roman(mut n: usize) -> String {
    const TABLE: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, numeral) in TABLE {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    out
}

/// Which marginal a section reference names, for `lib.rs`.
pub fn pick(refs: &[(Which, String)], which: Which) -> Option<&str> {
    refs.iter()
        .find(|(w, _)| *w == which)
        .map(|(_, id)| id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_is_written_as_odf_elements() {
        let mut out = String::new();
        let mut space = true;
        encode(&mut out, " a  b\tc", &mut space);
        assert_eq!(out, "<text:s/>a <text:s/>b<text:tab/>c");
        let mut out = String::new();
        let mut space = false;
        encode(&mut out, "x   ", &mut space);
        assert_eq!(out, "x <text:s text:c=\"2\"/>");
    }

    #[test]
    fn roman_numerals() {
        assert_eq!(roman(1), "i");
        assert_eq!(roman(4), "iv");
        assert_eq!(roman(14), "xiv");
    }

    #[test]
    fn tabs_move_to_the_indent() {
        let tabs = [Tab {
            position: 2880,
            kind: "left",
            leader: None,
            clear: false,
        }];
        assert_eq!(shift_tabs(&tabs, 720)[0].position, 2160);
        assert!(
            shift_tabs(&tabs, 3000).is_empty(),
            "a stop behind the indent is gone"
        );
    }
}
