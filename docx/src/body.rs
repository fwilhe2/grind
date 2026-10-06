// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The story parts — `word/document.xml`'s body, and every header, footer, footnote and endnote,
//! which hold the same content (§17.3, `EG_BlockLevelElts`) — read into a small intermediate
//! form that `emit.rs` writes as ODF.
//!
//! The intermediate form exists for two reasons that are both about Word rather than ODF:
//!
//! 1. **Some facts arrive after the content they change.** A section's `w:sectPr` closes it, so
//!    whether the *next* paragraph starts a page is known only when the section after it is
//!    read; a hidden bookmark is kept only if some hyperlink later points at it; and a page
//!    break in the middle of a run splits the paragraph around it. Each is a pass over this
//!    form rather than a second pass over XML.
//! 2. **Word's fields are a state machine that crosses elements.** `w:fldChar begin`, the
//!    instruction in `w:instrText`, `separate`, the shown result, `end` — spread over runs,
//!    and a table of contents' over *paragraphs*. [`Ctx`] carries that stack across the whole
//!    story, so what is shown is decided in one place.
//!
//! Everything Word wraps content in without changing it — `w:sdt` (a content control),
//! `w:smartTag`, `w:customXml`, `w:ins`, `w:moveTo`, `w:dir`, `w:bdo` — is read *through*;
//! everything that hides content — `w:del`, `w:moveFrom` — is skipped. Both are counted when
//! they are tracked changes.

use std::collections::{BTreeMap, HashMap, HashSet};

use grind_ooxml::names::{Ns, RelType};
use grind_ooxml::package::{Package, Rel};

use crate::props::{self, Fonts, ParaFacts, Props};
use crate::report::{Dropped, Report};
use crate::section::Section;
use crate::xml::{Attrs, Handled, Name, Reader, Word as _, WordAttrs as _};

/// A run's formatting, its character style and its hyperlink.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub props: Props,
    /// A character style *id*.
    pub style: Option<String>,
    pub href: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub data: Vec<u8>,
    pub mime: String,
    /// In EMU.
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// `wp:inline` — in the line, as a character — rather than `wp:anchor`, floating.
    pub inline: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String, Run),
    Tab(Run),
    Break(Run),
    /// `w:br w:type="page"` (or `column`) — split out of the paragraph by [`page_breaks`].
    PageBreak,
    Bookmark(String),
    /// A footnote or endnote reference, by the note's id in its part.
    Note {
        endnote: bool,
        id: String,
        run: Run,
        /// A mark of the author's own in place of the next number (`w:customMarkFollows`) —
        /// empty while the text that is the mark has not been read yet.
        mark: Option<String>,
    },
    Image(Image),
    PageNumber(Run),
    PageCount(Run),
}

#[derive(Clone, Debug, Default)]
pub struct Para {
    pub facts: ParaFacts,
    pub inlines: Vec<Inline>,
    /// Starts a page: a page break before it, or a section starting one.
    pub page_break_before: bool,
    /// The master page it starts, when its section's page differs from the one before.
    pub master_page: Option<String>,
}

/// `w:vMerge` (§17.4.85): the cell that starts a vertical merge, or one it covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VMerge {
    #[default]
    None,
    Restart,
    Continue,
}

#[derive(Clone, Debug, Default)]
pub struct Cell {
    pub span: u32,
    pub vmerge: VMerge,
    pub fill: Option<String>,
    /// `w:tcBorders`, by side — `Some(None)` is a border explicitly removed.
    pub borders: BTreeMap<&'static str, Option<String>>,
    pub valign: Option<&'static str>,
    pub margins: BTreeMap<&'static str, i64>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub header: bool,
    /// `w:trHeight` in twips, and whether it is exact rather than a least height.
    pub height: Option<(i64, bool)>,
    pub before: u32,
    pub after: u32,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, Default)]
pub struct Table {
    pub style: Option<String>,
    pub look: crate::styles::TableLook,
    /// `w:tblPr/w:jc`.
    pub align: Option<&'static str>,
    /// Column widths in twips (`w:tblGrid`).
    pub grid: Vec<i64>,
    pub rows: Vec<Row>,
    pub page_break_before: bool,
    /// As [`Para::master_page`].
    pub master_page: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Block {
    Para(Para),
    Table(Table),
}

/// A field being read: its instruction so far, whether its result has started, and what the
/// result does.
#[derive(Clone, Debug, Default)]
struct Field {
    instruction: String,
    in_result: bool,
    /// The result's text is not shown — a page number, whose text is the page's own.
    hide_result: bool,
    /// The href to put back when a `HYPERLINK` field's result ends.
    outer_href: Option<Option<String>>,
    emitted: bool,
}

/// What reading a story needs beside the XML.
pub struct Ctx<'p, 'a> {
    pub package: &'p mut Package<'a>,
    pub fonts: &'p Fonts,
    pub report: &'p mut Report,
    pub seen: &'p mut grind_ooxml::names::Seen,
    /// The relationships of the part being read, by id.
    pub rels: HashMap<String, Rel>,
    fields: Vec<Field>,
    href: Option<String>,
    /// Every `w:hyperlink/@w:anchor` and `\l` target — the hidden bookmarks worth keeping.
    pub anchors: HashSet<String>,
    /// Every section, in order, as its `w:sectPr` was met.
    pub sections: Vec<Section>,
    /// Text boxes read inside the paragraph being read, waiting to follow it ([`text_box`]).
    pub floating: Vec<Block>,
    /// Pictures already read, by part — one image used twice is read once.
    images: HashMap<String, Option<(Vec<u8>, String)>>,
}

impl<'p, 'a> Ctx<'p, 'a> {
    pub fn new(
        package: &'p mut Package<'a>,
        fonts: &'p Fonts,
        report: &'p mut Report,
        seen: &'p mut grind_ooxml::names::Seen,
    ) -> Self {
        Ctx {
            package,
            fonts,
            report,
            seen,
            rels: HashMap::new(),
            fields: Vec::new(),
            href: None,
            anchors: HashSet::new(),
            sections: Vec::new(),
            images: HashMap::new(),
            floating: Vec::new(),
        }
    }

    /// Point the context at a part: its relationships become the ones `r:id`s resolve in.
    pub fn enter(&mut self, part: &str) {
        self.rels = self
            .package
            .rels(part, self.seen)
            .into_iter()
            .map(|rel| (rel.id.clone(), rel))
            .collect();
        self.fields.clear();
        self.href = None;
    }

    /// Whether a field's instruction is being read — any field, however deep.
    fn in_instruction(&self) -> bool {
        self.fields.iter().any(|f| !f.in_result)
    }

    /// Whether a run's text is shown here: not inside any field's instruction, and not inside
    /// a result this filter replaces with a field of its own.
    fn showing(&self) -> bool {
        self.fields.iter().all(|f| f.in_result && !f.hide_result)
    }

    fn picture(&mut self, target: &str) -> Option<(Vec<u8>, String)> {
        if let Some(cached) = self.images.get(target) {
            return cached.clone();
        }
        let found = self.package.part(target).map(|bytes| {
            let mime = grind_text::picture::mime(&bytes)
                .map(str::to_owned)
                .unwrap_or_else(|| by_extension(target).to_owned());
            (bytes, mime)
        });
        self.images.insert(target.to_owned(), found.clone());
        found
    }
}

/// A picture's media type from its part name, for the formats `picture::mime` does not sniff —
/// Windows' own metafiles, which old Word documents are full of.
fn by_extension(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("emf") => "image/x-emf",
        Some("wmf") => "image/x-wmf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("tif" | "tiff") => "image/tiff",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// Read the story under the element the reader has just opened — `w:body`, `w:hdr`, `w:ftr`, a
/// `w:footnote`, a `w:tc`.
pub fn read_blocks(r: &mut Reader, ctx: &mut Ctx) -> grind_ooxml::Result<Vec<Block>> {
    let mut out = Vec::new();
    blocks_into(r, ctx, &mut out)?;
    Ok(out)
}

/// [`read_blocks`] into a list the caller holds — so that what was read before a damaged part
/// stopped being XML is still there to keep.
pub fn blocks_into(r: &mut Reader, ctx: &mut Ctx, out: &mut Vec<Block>) -> grind_ooxml::Result<()> {
    // A bookmark between paragraphs belongs to the next one.
    let mut pending: Vec<Inline> = Vec::new();
    r.children(|r, name, attrs| {
        if name.ns != Ns::Word {
            if is_math(name) {
                ctx.report.drop_one(Dropped::Equation);
            }
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "p" => {
                let mut para = read_paragraph(r, ctx)?;
                if !pending.is_empty() {
                    let mut inlines = std::mem::take(&mut pending);
                    inlines.append(&mut para.inlines);
                    para.inlines = inlines;
                }
                // A paragraph mark inside a field's instruction is part of the instruction:
                // the paragraph does not end there in what the field shows, so what it showed
                // joins the next one (`doc/docx-format.md` §2.3).
                let boxes = std::mem::take(&mut ctx.floating);
                if ctx.in_instruction() {
                    pending = para.inlines;
                } else if para.facts.drop_cap {
                    // Word keeps a drop cap as a framed paragraph of its own; it is the first
                    // letter of the next paragraph, and is read as that — its size is lost.
                    pending = para.inlines;
                } else {
                    out.push(Block::Para(para));
                }
                out.extend(boxes);
            }
            "tbl" => {
                let table = read_table(r, ctx)?;
                out.push(Block::Table(table));
                out.extend(std::mem::take(&mut ctx.floating));
            }
            "sdt" => {
                if content_of_sdt(r, |r| blocks_into(r, ctx, out))? {
                    ctx.report.drop_one(Dropped::ContentControl);
                }
            }
            "customXml" | "ins" | "moveTo" => {
                if matches!(name.local.as_str(), "ins" | "moveTo") {
                    ctx.report.drop_one(Dropped::TrackedChange);
                }
                blocks_into(r, ctx, out)?;
            }
            "del" | "moveFrom" => {
                ctx.report.drop_one(Dropped::TrackedChange);
                return Ok(Handled::No);
            }
            "bookmarkStart" => {
                if let Some(mark) = bookmark(attrs) {
                    pending.push(mark);
                }
            }
            "sectPr" => {
                let section = crate::section::read(r)?;
                ctx.sections.push(section);
            }
            "altChunk" => ctx.report.drop_one(Dropped::EmbeddedObject),
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    // A bookmark after the last paragraph of a story: on an empty paragraph of its own would
    // add a block, so it goes on the last paragraph instead, at its end.
    if !pending.is_empty()
        && let Some(Block::Para(last)) = out.last_mut()
    {
        last.inlines.append(&mut pending);
    }
    Ok(())
}

/// `w:sdt` — a content control: its `w:sdtContent` read in place, the rest ignored. Whether it
/// was a *control* — a drop-down, a date picker, a checkbox, placeholder text — rather than a
/// plain wrapper is the answer, since the control is what the text does not keep.
fn content_of_sdt(
    r: &mut Reader,
    mut inside: impl FnMut(&mut Reader) -> grind_ooxml::Result<()>,
) -> grind_ooxml::Result<bool> {
    let mut control = false;
    r.children(|r, name, _| {
        if name.w("sdtContent") {
            inside(r)?;
            Ok(Handled::Yes)
        } else if name.w("sdtPr") {
            r.children(|_, name, _| {
                control |= matches!(
                    name.local.as_str(),
                    "dropDownList" | "comboBox" | "date" | "checkbox" | "showingPlcHdr" | "text"
                );
                Ok(Handled::No)
            })?;
            Ok(Handled::Yes)
        } else {
            Ok(Handled::No)
        }
    })?;
    Ok(control)
}

/// OMML: `m:oMath` and `m:oMathPara`, recognised by name since the math namespace is in no
/// table — nothing else is read from it.
fn is_math(name: &Name) -> bool {
    name.ns == Ns::Other && matches!(name.local.as_str(), "oMath" | "oMathPara")
}

fn bookmark(attrs: &Attrs) -> Option<Inline> {
    let name = attrs.w("name")?;
    (!name.is_empty()).then(|| Inline::Bookmark(name.to_owned()))
}

fn read_paragraph(r: &mut Reader, ctx: &mut Ctx) -> grind_ooxml::Result<Para> {
    let mut para = Para::default();
    r.children(|r, name, attrs| {
        if name.w("pPr") {
            para.facts = props::read_ppr(r)?;
            if para.facts.frame {
                ctx.report.drop_one(Dropped::Frame);
            }
            if let Some(section) = para.facts.section.clone() {
                ctx.sections.push(*section);
            }
            return Ok(Handled::Yes);
        }
        content(r, name, attrs, ctx, &mut para.inlines)
    })?;
    Ok(para)
}

/// One child of paragraph content — a `w:p`'s, a `w:hyperlink`'s, an inline `w:sdtContent`'s.
fn content(
    r: &mut Reader,
    name: &Name,
    attrs: &Attrs,
    ctx: &mut Ctx,
    out: &mut Vec<Inline>,
) -> grind_ooxml::Result<Handled> {
    if name.ns != Ns::Word {
        if is_math(name) {
            ctx.report.drop_one(Dropped::Equation);
        }
        return Ok(Handled::No);
    }
    match name.local.as_str() {
        "r" => read_run(r, ctx, out)?,
        "hyperlink" => {
            let target = attrs
                .rel("id")
                .and_then(|id| ctx.rels.get(id))
                .filter(|rel| rel.kind == RelType::Hyperlink || rel.external)
                .map(|rel| rel.target.clone());
            let anchor = attrs.w("anchor").map(str::to_owned);
            if let Some(anchor) = &anchor {
                ctx.anchors.insert(anchor.clone());
            }
            let href = match (target, anchor) {
                (Some(url), Some(anchor)) => Some(format!("{url}#{anchor}")),
                (Some(url), None) => Some(url),
                (None, Some(anchor)) => Some(format!("#{anchor}")),
                (None, None) => None,
            };
            let inner = href.or_else(|| ctx.href.clone());
            let outer = std::mem::replace(&mut ctx.href, inner);
            inline_children(r, ctx, out)?;
            ctx.href = outer;
        }
        "fldSimple" => {
            let instruction = attrs.w("instr").unwrap_or_default().to_owned();
            begin_field(ctx);
            if let Some(field) = ctx.fields.last_mut() {
                field.instruction = instruction;
            }
            separate_field(ctx, out, &Run::default());
            inline_children(r, ctx, out)?;
            end_field(ctx, out, &Run::default());
        }
        "smartTag" | "customXml" | "dir" | "bdo" => inline_children(r, ctx, out)?,
        "sdt" => {
            if content_of_sdt(r, |r| inline_children(r, ctx, out))? {
                ctx.report.drop_one(Dropped::ContentControl);
            }
        }
        "ins" | "moveTo" => {
            ctx.report.drop_one(Dropped::TrackedChange);
            inline_children(r, ctx, out)?;
        }
        "del" | "moveFrom" => {
            ctx.report.drop_one(Dropped::TrackedChange);
            return Ok(Handled::No);
        }
        "bookmarkStart" => {
            if let Some(mark) = bookmark(attrs) {
                out.push(mark);
            }
        }
        "commentRangeStart" => ctx.report.drop_one(Dropped::Comment),
        _ => return Ok(Handled::No),
    }
    Ok(Handled::Yes)
}

fn inline_children(
    r: &mut Reader,
    ctx: &mut Ctx,
    out: &mut Vec<Inline>,
) -> grind_ooxml::Result<()> {
    r.children(|r, name, attrs| content(r, name, attrs, ctx, out))
}

fn begin_field(ctx: &mut Ctx) {
    ctx.fields.push(Field::default());
}

/// The instruction is complete: decide what the result means.
fn separate_field(ctx: &mut Ctx, out: &mut Vec<Inline>, run: &Run) {
    // Only the innermost field's instruction is being decided; an outer one still collecting
    // its instruction keeps everything hidden anyway.
    let Some(field) = ctx.fields.last_mut() else {
        return;
    };
    if field.in_result {
        return;
    }
    field.in_result = true;
    let instruction = field.instruction.trim().to_owned();
    let word = instruction
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    match word.as_str() {
        "PAGE" | "NUMPAGES" | "SECTIONPAGES" => {
            field.hide_result = true;
            field.emitted = true;
            let inline = if word == "PAGE" {
                Inline::PageNumber(run.clone())
            } else {
                Inline::PageCount(run.clone())
            };
            if ctx.fields[..ctx.fields.len() - 1]
                .iter()
                .all(|f| f.in_result && !f.hide_result)
            {
                out.push(inline);
            }
        }
        "HYPERLINK" => {
            let href = hyperlink_target(&instruction);
            if let Some(target) = href.as_deref().and_then(|h| h.strip_prefix('#')) {
                ctx.anchors.insert(target.to_owned());
            }
            if href.is_some() {
                let field = ctx.fields.last_mut().expect("just looked at it");
                field.outer_href = Some(ctx.href.clone());
                ctx.href = href;
            }
        }
        "FORMTEXT" | "FORMCHECKBOX" | "FORMDROPDOWN" => ctx.report.drop_one(Dropped::FormField),
        // `ASK` and `SET` give a bookmark a value and show nothing themselves (§17.16.5.4,
        // §17.16.5.56) — their cached result is what a later `REF` shows, not what they do
        // (`doc/docx-format.md` §2.3).
        "ASK" | "SET" => {
            field.hide_result = true;
            ctx.report.drop_one(Dropped::Field);
        }
        // An empty instruction is a field nobody filled in, and shows its result as text with
        // nothing lost.
        "" => {}
        _ => ctx.report.drop_one(Dropped::Field),
    }
}

fn end_field(ctx: &mut Ctx, out: &mut Vec<Inline>, run: &Run) {
    // A field with no `separate` has no result; its meaning is decided now.
    if ctx.fields.last().is_some_and(|f| !f.in_result) {
        separate_field(ctx, out, run);
    }
    if let Some(field) = ctx.fields.pop()
        && let Some(outer) = field.outer_href
    {
        ctx.href = outer;
    }
}

/// `HYPERLINK "https://…"` or `HYPERLINK \l "anchor"`.
fn hyperlink_target(instruction: &str) -> Option<String> {
    let rest = instruction.trim_start().get("HYPERLINK".len()..)?.trim();
    let mut url = None;
    let mut anchor = None;
    let mut parts = rest.split('"');
    let mut before = parts.next().unwrap_or_default().to_owned();
    while let Some(quoted) = parts.next() {
        if before.trim_end().ends_with("\\l") {
            anchor = Some(quoted.to_owned());
        } else if url.is_none() && !before.trim_end().ends_with(['o', 't', 'm']) {
            url = Some(quoted.to_owned());
        }
        before = parts.next().unwrap_or_default().to_owned();
    }
    if url.is_none() && anchor.is_none() {
        // Unquoted: the first word that is not a switch.
        url = rest
            .split_whitespace()
            .find(|w| !w.starts_with('\\'))
            .map(str::to_owned);
    }
    match (url, anchor) {
        (Some(url), Some(anchor)) => Some(format!("{url}#{anchor}")),
        (Some(url), None) => Some(url),
        (None, Some(anchor)) => Some(format!("#{anchor}")),
        (None, None) => None,
    }
}

/// One `w:r`.
fn read_run(r: &mut Reader, ctx: &mut Ctx, out: &mut Vec<Inline>) -> grind_ooxml::Result<()> {
    let mut run = Run {
        href: ctx.href.clone(),
        ..Run::default()
    };
    let fonts = ctx.fonts;
    r.children(|r, name, attrs| {
        if name.ns != Ns::Word {
            if is_math(name) {
                ctx.report.drop_one(Dropped::Equation);
            }
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "rPr" => {
                let facts = props::read_rpr(r, fonts)?;
                run.props = facts.props;
                run.style = facts.style;
            }
            "t" => {
                let preserve = attrs.get(Ns::Other, "space") == Some("preserve");
                let text = whitespace(&r.text()?, preserve);
                if ctx.showing() {
                    push_shown(out, &text, &run);
                }
            }
            "instrText" => {
                let text = r.text()?;
                if let Some(field) = ctx.fields.last_mut()
                    && !field.in_result
                {
                    field.instruction.push_str(&text);
                }
            }
            "fldChar" => match attrs.w("fldCharType") {
                Some("begin") => begin_field(ctx),
                Some("separate") => separate_field(ctx, out, &run),
                Some("end") => end_field(ctx, out, &run),
                _ => {}
            },
            "tab" | "ptab" if ctx.showing() => out.push(Inline::Tab(run.clone())),
            "br" if ctx.showing() => match attrs.w("type") {
                Some("page" | "column") => out.push(Inline::PageBreak),
                _ => out.push(Inline::Break(run.clone())),
            },
            "cr" if ctx.showing() => out.push(Inline::Break(run.clone())),
            "noBreakHyphen" if ctx.showing() => push_text(out, "\u{2011}", &run),
            "softHyphen" if ctx.showing() => push_text(out, "\u{00AD}", &run),
            "sym" if ctx.showing() => {
                if let Some(c) = attrs
                    .w("char")
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .and_then(char::from_u32)
                {
                    let mut symbol = run.clone();
                    if let Some(font) = attrs.w("font") {
                        symbol.props.set("fo:font-family", props::family(font));
                    }
                    push_shown(out, &c.to_string(), &symbol);
                }
            }
            "footnoteReference" | "endnoteReference" if ctx.showing() => {
                if let Some(id) = attrs.w("id") {
                    out.push(Inline::Note {
                        endnote: name.local == "endnoteReference",
                        id: id.to_owned(),
                        run: run.clone(),
                        // Waiting for the text that follows, which is the mark itself.
                        mark: attrs
                            .w("customMarkFollows")
                            .is_some_and(|v| matches!(v, "1" | "true" | "on"))
                            .then(String::new),
                    });
                }
            }
            "drawing" if ctx.showing() => {
                if let Some(image) = read_drawing(r, ctx)? {
                    out.push(Inline::Image(image));
                }
            }
            "pict" if ctx.showing() => {
                if let Some(image) = read_vml(r, ctx)? {
                    out.push(Inline::Image(image));
                }
            }
            "object" => ctx.report.drop_one(Dropped::EmbeddedObject),
            "commentReference" => {}
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })
}

/// A `w:t`'s text as Word shows it. A line break in the character data is a space either way —
/// a break is `w:br`, never a newline (`doc/docx-format.md` §2.2) — and without
/// `xml:space="preserve"` every run of whitespace, a tab included, is one space. It is not
/// trimmed: a run that is a single space between two words is how some producers write one.
fn whitespace(text: &str, preserve: bool) -> String {
    if preserve {
        return text.replace("\r\n", " ").replace(['\r', '\n'], " ");
    }
    let mut out = String::with_capacity(text.len());
    let mut gap = false;
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\r' | '\n') {
            gap = true;
        } else {
            if gap {
                out.push(' ');
                gap = false;
            }
            out.push(c);
        }
    }
    if gap {
        out.push(' ');
    }
    out
}

/// Append text a run shows — unless a note reference just before it is waiting for its own
/// mark (`w:customMarkFollows`), in which case this text *is* that mark.
fn push_shown(out: &mut Vec<Inline>, text: &str, run: &Run) {
    if let Some(Inline::Note { mark, .. }) = out.last_mut()
        && mark.as_deref() == Some("")
    {
        *mark = Some(text.to_owned());
    } else {
        push_text(out, text, run);
    }
}

/// Append text, joining it to a text run of the same formatting just before.
fn push_text(out: &mut Vec<Inline>, text: &str, run: &Run) {
    if text.is_empty() {
        return;
    }
    if let Some(Inline::Text(last, last_run)) = out.last_mut()
        && last_run == run
    {
        last.push_str(text);
        return;
    }
    out.push(Inline::Text(text.to_owned(), run.clone()));
}

/// What a `w:drawing` holds, found wherever DrawingML put it.
#[derive(Default)]
struct Found {
    inline: bool,
    width: Option<i64>,
    height: Option<i64>,
    embed: Option<String>,
    linked: bool,
    text_box: bool,
    picture: bool,
}

/// `w:txbxContent` — a text box's story, read as blocks of their own and set **after the
/// paragraph that anchors it** ([`Ctx::floating`]): the text model has nowhere to float a box,
/// and its text in the flow beats its text gone. Its position is what is counted. The field
/// and hyperlink state of the paragraph around it is put aside while it is read, since a text
/// box is a story of its own.
fn text_box(r: &mut Reader, ctx: &mut Ctx) -> grind_ooxml::Result<()> {
    let fields = std::mem::take(&mut ctx.fields);
    let href = ctx.href.take();
    let read = read_blocks(r, ctx);
    ctx.fields = fields;
    ctx.href = href;
    ctx.floating.extend(read?);
    Ok(())
}

/// `w:drawing` — DrawingML (§20.4). Only a picture is carried: the `a:blip` a `pic:pic` fills
/// with, at the size `wp:extent` gives. Anything else in a drawing — a shape, a chart, a text
/// box — is counted.
fn read_drawing(r: &mut Reader, ctx: &mut Ctx) -> grind_ooxml::Result<Option<Image>> {
    let mut found = Found::default();
    fn walk(r: &mut Reader, found: &mut Found, ctx: &mut Ctx) -> grind_ooxml::Result<()> {
        r.children(|r, name, attrs| {
            match (name.ns, name.local.as_str()) {
                (Ns::WordDrawing, "inline") => found.inline = true,
                (Ns::WordDrawing, "extent") => {
                    found.width = attrs.plain("cx").and_then(|v| v.parse().ok());
                    found.height = attrs.plain("cy").and_then(|v| v.parse().ok());
                }
                (Ns::Picture, "pic") => found.picture = true,
                (Ns::Drawing, "blip") => {
                    if let Some(id) = attrs.get(Ns::Relationships, "embed") {
                        found.embed.get_or_insert_with(|| id.to_owned());
                    } else if attrs.get(Ns::Relationships, "link").is_some() {
                        found.linked = true;
                    }
                }
                (Ns::Word, "txbxContent") => {
                    found.text_box = true;
                    text_box(r, ctx)?;
                    return Ok(Handled::Yes);
                }
                _ => {}
            }
            walk(r, found, ctx)?;
            Ok(Handled::Yes)
        })
    }
    walk(r, &mut found, ctx)?;
    if found.text_box {
        ctx.report.drop_one(Dropped::TextBox);
    }
    let picture = found
        .embed
        .filter(|_| found.picture)
        .and_then(|id| ctx.rels.get(&id).map(|rel| rel.target.clone()))
        .and_then(|target| ctx.picture(&target));
    let Some((data, mime)) = picture else {
        if found.linked {
            ctx.report.drop_one(Dropped::LinkedPicture);
        } else if !found.text_box {
            ctx.report.drop_one(Dropped::Drawing);
        }
        return Ok(None);
    };
    if !found.inline {
        ctx.report.drop_one(Dropped::FloatingPosition);
    }
    Ok(Some(Image {
        data,
        mime,
        width: found.width,
        height: found.height,
        inline: found.inline,
    }))
}

/// `w:pict` — VML, the drawing language before DrawingML and still the fallback a text box or
/// a watermark is written in: `v:imagedata r:id` is the picture, and `v:shape`'s CSS `style`
/// its size.
fn read_vml(r: &mut Reader, ctx: &mut Ctx) -> grind_ooxml::Result<Option<Image>> {
    let mut embed = None;
    let mut size = (None, None);
    let mut boxed_any = false;
    let mut floating = false;
    fn walk(
        r: &mut Reader,
        embed: &mut Option<String>,
        size: &mut (Option<i64>, Option<i64>),
        boxed: &mut bool,
        floating: &mut bool,
        ctx: &mut Ctx,
    ) -> grind_ooxml::Result<()> {
        r.children(|r, name, attrs| {
            if name.ns == Ns::Word && name.local == "txbxContent" {
                *boxed = true;
                text_box(r, ctx)?;
                return Ok(Handled::Yes);
            }
            if name.ns == Ns::Vml {
                match name.local.as_str() {
                    "imagedata" => {
                        if let Some(id) = attrs.get(Ns::Relationships, "id") {
                            embed.get_or_insert_with(|| id.to_owned());
                        }
                    }
                    "textbox" => *boxed = true,
                    "shape" | "rect" | "image" => {
                        if let Some(style) = attrs.plain("style") {
                            let (w, h, absolute) = css_size(style);
                            if size.0.is_none() {
                                *size = (w, h);
                            }
                            *floating |= absolute;
                        }
                    }
                    _ => {}
                }
            }
            walk(r, embed, size, boxed, floating, ctx)?;
            Ok(Handled::Yes)
        })
    }
    walk(r, &mut embed, &mut size, &mut boxed_any, &mut floating, ctx)?;
    if boxed_any {
        ctx.report.drop_one(Dropped::TextBox);
    }
    let picture = embed
        .and_then(|id| ctx.rels.get(&id).map(|rel| rel.target.clone()))
        .and_then(|target| ctx.picture(&target));
    let Some((data, mime)) = picture else {
        if !boxed_any {
            ctx.report.drop_one(Dropped::Drawing);
        }
        return Ok(None);
    };
    if floating {
        ctx.report.drop_one(Dropped::FloatingPosition);
    }
    Ok(Some(Image {
        data,
        mime,
        width: size.0,
        height: size.1,
        inline: !floating,
    }))
}

/// `width:100pt;height:50.5pt;position:absolute` → sizes in EMU, and whether it floats.
fn css_size(style: &str) -> (Option<i64>, Option<i64>, bool) {
    let mut width = None;
    let mut height = None;
    let mut absolute = false;
    for declaration in style.split(';') {
        let Some((key, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "width" => width = css_length(value),
            "height" => height = css_length(value),
            "position" => absolute = value == "absolute",
            _ => {}
        }
    }
    (width, height, absolute)
}

fn css_length(value: &str) -> Option<i64> {
    let number = value.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let n: f64 = number.trim().parse().ok()?;
    let points = match &value[number.len()..] {
        "pt" | "" => n,
        "in" => n * 72.0,
        "cm" => n * 72.0 / 2.54,
        "mm" => n * 72.0 / 25.4,
        "px" => n * 0.75,
        _ => return None,
    };
    Some((points * 12_700.0) as i64)
}

/// `w:tbl`.
fn read_table(r: &mut Reader, ctx: &mut Ctx) -> grind_ooxml::Result<Table> {
    let mut table = Table::default();
    let mut own = crate::styles::TableLook::default();
    r.children(|r, name, _| {
        if name.ns != Ns::Word {
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "tblPr" => {
                // Read twice in effect: the style's name and alignment here, its borders and
                // margins by the reader `styles.rs` uses for a table style's own `w:tblPr`.
                let mut style = None;
                let mut align = None;
                let mut fill = None;
                r.children(|r, name, attrs| {
                    if name.w("tblStyle") {
                        style = attrs.val().map(str::to_owned);
                    } else if name.w("jc") {
                        align = attrs.val().and_then(props::alignment);
                    } else if name.w("shd") {
                        fill = props::shading(attrs);
                    } else if crate::styles::table_look_child(r, name, &mut own)? {
                    } else if name.w("tblpPr") {
                        ctx.report.drop_one(Dropped::Frame);
                        return Ok(Handled::No);
                    } else {
                        return Ok(Handled::No);
                    }
                    Ok(Handled::Yes)
                })?;
                table.style = style;
                table.align = align;
                own.fill = fill;
            }
            "tblGrid" => {
                r.children(|_, name, attrs| {
                    if name.w("gridCol") {
                        table.grid.push(attrs.int("w").unwrap_or(0).max(0));
                    }
                    Ok(Handled::Yes)
                })?;
            }
            "tr" => table.rows.push(read_row(r, ctx)?),
            "sdt" => {
                content_of_sdt(r, |r| {
                    r.children(|r, name, _| {
                        if name.w("tr") {
                            table.rows.push(read_row(r, ctx)?);
                            Ok(Handled::Yes)
                        } else {
                            Ok(Handled::No)
                        }
                    })
                })?;
            }
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    table.look = own;
    Ok(table)
}

fn read_row(r: &mut Reader, ctx: &mut Ctx) -> grind_ooxml::Result<Row> {
    let mut row = Row::default();
    r.children(|r, name, _| {
        if name.ns != Ns::Word {
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "trPr" => {
                r.children(|_, name, attrs| {
                    if name.w("tblHeader") {
                        row.header = attrs.on();
                    } else if name.w("trHeight") {
                        if let Some(h) = attrs.int("val").filter(|h| *h > 0) {
                            row.height = Some((h, attrs.w("hRule") == Some("exact")));
                        }
                    } else if name.w("gridBefore") {
                        row.before = attrs.int("val").unwrap_or(0).clamp(0, 64) as u32;
                    } else if name.w("gridAfter") {
                        row.after = attrs.int("val").unwrap_or(0).clamp(0, 64) as u32;
                    }
                    Ok(Handled::Yes)
                })?;
            }
            "tc" => row.cells.push(read_cell(r, ctx)?),
            "sdt" => {
                content_of_sdt(r, |r| {
                    r.children(|r, name, _| {
                        if name.w("tc") {
                            row.cells.push(read_cell(r, ctx)?);
                            Ok(Handled::Yes)
                        } else {
                            Ok(Handled::No)
                        }
                    })
                })?;
            }
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    Ok(row)
}

fn read_cell(r: &mut Reader, ctx: &mut Ctx) -> grind_ooxml::Result<Cell> {
    let mut cell = Cell {
        span: 1,
        ..Cell::default()
    };
    let mut blocks = Vec::new();
    // The cell's properties come first and its content after; a visitor that takes `tcPr` and
    // hands everything else to the story reader reads both in one walk.
    let mut pending: Vec<Inline> = Vec::new();
    r.children(|r, name, attrs| {
        if name.w("tcPr") {
            r.children(|r, name, attrs| {
                match name.local.as_str() {
                    _ if name.ns != Ns::Word => return Ok(Handled::No),
                    "gridSpan" => cell.span = attrs.int("val").unwrap_or(1).clamp(1, 64) as u32,
                    "vMerge" => {
                        cell.vmerge = match attrs.val() {
                            Some("restart") => VMerge::Restart,
                            _ => VMerge::Continue,
                        }
                    }
                    "hMerge" => {}
                    "shd" => cell.fill = props::shading(attrs),
                    "vAlign" => {
                        cell.valign = match attrs.val() {
                            Some("center") => Some("middle"),
                            Some("bottom") => Some("bottom"),
                            Some("top") => Some("top"),
                            _ => None,
                        }
                    }
                    "tcBorders" => {
                        r.children(|_, name, attrs| {
                            let side = match name.local.as_str() {
                                "top" => "top",
                                "bottom" => "bottom",
                                "left" | "start" => "left",
                                "right" | "end" => "right",
                                _ => return Ok(Handled::No),
                            };
                            cell.borders
                                .insert(side, props::border(attrs).map(|(line, _)| line));
                            Ok(Handled::Yes)
                        })?;
                    }
                    "tcMar" => {
                        r.children(|_, name, attrs| {
                            let side = match name.local.as_str() {
                                "top" => "top",
                                "bottom" => "bottom",
                                "left" | "start" => "left",
                                "right" | "end" => "right",
                                _ => return Ok(Handled::No),
                            };
                            if matches!(attrs.w("type"), None | Some("dxa"))
                                && let Some(w) = attrs.int("w")
                            {
                                cell.margins.insert(side, w);
                            }
                            Ok(Handled::Yes)
                        })?;
                    }
                    _ => return Ok(Handled::No),
                }
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        // Everything else is the cell's story, read the way `blocks_into` reads one.
        match (name.ns, name.local.as_str()) {
            (Ns::Word, "p") => {
                let mut para = read_paragraph(r, ctx)?;
                if !pending.is_empty() {
                    let mut inlines = std::mem::take(&mut pending);
                    inlines.append(&mut para.inlines);
                    para.inlines = inlines;
                }
                blocks.push(Block::Para(para));
                blocks.append(&mut ctx.floating);
            }
            (Ns::Word, "tbl") => blocks.push(Block::Table(read_table(r, ctx)?)),
            (Ns::Word, "sdt") => {
                if content_of_sdt(r, |r| blocks_into(r, ctx, &mut blocks))? {
                    ctx.report.drop_one(Dropped::ContentControl);
                }
            }
            (Ns::Word, "customXml") => blocks_into(r, ctx, &mut blocks)?,
            (Ns::Word, "bookmarkStart") => {
                if let Some(mark) = bookmark(attrs) {
                    pending.push(mark);
                }
            }
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    cell.blocks = blocks;
    Ok(cell)
}

/// Split every paragraph at its page breaks: what follows a break is a paragraph of its own
/// that starts a page — the only way ODF has to say "a page starts here", since it has no
/// break element inside a paragraph (`doc/docx-format.md` §2.4). A break at a paragraph's very
/// end starts the *next* block on a page; one at its very start, the paragraph itself.
pub fn page_breaks(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    let mut carry = false;
    for block in blocks {
        match block {
            Block::Table(mut table) => {
                table.page_break_before |= std::mem::take(&mut carry);
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        cell.blocks = page_breaks(std::mem::take(&mut cell.blocks))
                            .into_iter()
                            .map(|mut b| {
                                // Inside a cell a page break means nothing a table can do.
                                if let Block::Para(p) = &mut b {
                                    p.page_break_before = false;
                                }
                                b
                            })
                            .collect();
                    }
                }
                out.push(Block::Table(table));
            }
            Block::Para(mut para) => {
                para.page_break_before |= std::mem::take(&mut carry);
                if !para.inlines.contains(&Inline::PageBreak) {
                    out.push(Block::Para(para));
                    continue;
                }
                let inlines = std::mem::take(&mut para.inlines);
                let mut pieces: Vec<Vec<Inline>> = vec![Vec::new()];
                for inline in inlines {
                    if inline == Inline::PageBreak {
                        pieces.push(Vec::new());
                    } else {
                        pieces.last_mut().expect("never empty").push(inline);
                    }
                }
                let visible =
                    |piece: &[Inline]| piece.iter().any(|i| !matches!(i, Inline::Bookmark(_)));
                let last = pieces.len() - 1;
                let mut emitted: Vec<Para> = Vec::new();
                // Bookmarks with no text of their own beside them, waiting for some.
                let mut leading: Vec<Inline> = Vec::new();
                for (i, mut piece) in pieces.into_iter().enumerate() {
                    // An empty first or last piece is no paragraph at all — the break was at
                    // the paragraph's edge. An empty piece *between* two breaks is an empty
                    // page, and is a paragraph of its own.
                    if !visible(&piece) && (i == 0 || i == last) {
                        if i == last && i > 0 {
                            carry = true;
                            match emitted.last_mut() {
                                Some(previous) => previous.inlines.append(&mut piece),
                                None => leading.append(&mut piece),
                            }
                        } else {
                            leading.append(&mut piece);
                        }
                        continue;
                    }
                    let mut inlines = std::mem::take(&mut leading);
                    inlines.append(&mut piece);
                    emitted.push(Para {
                        facts: para.facts.clone(),
                        inlines,
                        page_break_before: i > 0 || para.page_break_before,
                        master_page: if i == 0 {
                            para.master_page.clone()
                        } else {
                            None
                        },
                    });
                }
                if emitted.is_empty() {
                    // A paragraph that was nothing but a break is still a paragraph — and may
                    // close a section — so it stays, empty, and the next block starts the page.
                    emitted.push(Para {
                        facts: para.facts.clone(),
                        inlines: std::mem::take(&mut leading),
                        page_break_before: para.page_break_before,
                        master_page: para.master_page.clone(),
                    });
                }
                // The section a paragraph closes is closed by its last piece only.
                let count = emitted.len();
                for (i, mut p) in emitted.into_iter().enumerate() {
                    if i + 1 < count {
                        p.facts.section = None;
                    }
                    out.push(Block::Para(p));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hyperlink_fields_target() {
        assert_eq!(
            hyperlink_target(r#"HYPERLINK "https://example.invalid/" "#).as_deref(),
            Some("https://example.invalid/")
        );
        assert_eq!(
            hyperlink_target(r#" HYPERLINK \l "_Toc123" \h "#).as_deref(),
            Some("#_Toc123")
        );
        assert_eq!(
            hyperlink_target(r#"HYPERLINK https://example.invalid/x"#).as_deref(),
            Some("https://example.invalid/x")
        );
    }

    #[test]
    fn a_css_size_is_emu() {
        let (w, h, floating) = css_size("width:72pt;height:1in;position:absolute;z-index:1");
        assert_eq!((w, h, floating), (Some(914_400), Some(914_400), true));
    }

    fn para(inlines: Vec<Inline>) -> Block {
        Block::Para(Para {
            inlines,
            ..Para::default()
        })
    }

    fn text(s: &str) -> Inline {
        Inline::Text(s.into(), Run::default())
    }

    fn shape(blocks: &[Block]) -> Vec<(String, bool)> {
        blocks
            .iter()
            .map(|b| match b {
                Block::Para(p) => (
                    p.inlines
                        .iter()
                        .map(|i| match i {
                            Inline::Text(t, _) => t.as_str(),
                            _ => "?",
                        })
                        .collect(),
                    p.page_break_before,
                ),
                Block::Table(_) => ("table".into(), false),
            })
            .collect()
    }

    #[test]
    fn a_page_break_splits_its_paragraph() {
        let got = page_breaks(vec![para(vec![text("a"), Inline::PageBreak, text("b")])]);
        assert_eq!(shape(&got), [("a".into(), false), ("b".into(), true)]);
    }

    #[test]
    fn a_page_break_at_the_end_moves_to_the_next_block() {
        let got = page_breaks(vec![
            para(vec![text("a"), Inline::PageBreak]),
            para(vec![text("b")]),
        ]);
        assert_eq!(shape(&got), [("a".into(), false), ("b".into(), true)]);
    }

    #[test]
    fn a_page_break_at_the_start_is_the_paragraphs_own() {
        let got = page_breaks(vec![para(vec![Inline::PageBreak, text("b")])]);
        assert_eq!(shape(&got), [("b".into(), true)]);
    }

    #[test]
    fn a_paragraph_that_is_only_a_break_starts_the_next_page() {
        let got = page_breaks(vec![
            para(vec![text("a")]),
            para(vec![Inline::PageBreak]),
            para(vec![text("b")]),
        ]);
        assert_eq!(
            shape(&got),
            [("a".into(), false), ("".into(), false), ("b".into(), true)]
        );
    }
}
