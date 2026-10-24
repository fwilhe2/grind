// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Serialising a text document back to ODF. **\[ODT\]** Packaging, the manifest and escaping are
//! `grind-core`'s; this is the `office:text` content model. Both forms share one content writer,
//! differing only in the root element and whether `office:mimetype` is on it. Minimal (§1.4, R3):
//! a namespace is declared only if the document uses it, so a new document is a handful of lines.
//!
//! * Whitespace is re-encoded as elements: a consumer collapses whitespace in character data, so
//!   the writer turns spaces, tabs and newlines into `text:s`, `text:tab` and `text:line-break`
//!   (`doc/odt-format.md` §3.3).
//! * Lists are rebuilt from block depths, opening an element where the depth rises and closing
//!   where it falls.

use std::fmt::Write as _;

use grind_core::Result;
use grind_core::odf::envelope;
use grind_core::odf::names::{DRAW, FO, OFFICE, STYLE, SVG, TABLE, TEXT, XLINK};
use grind_core::odf::package::{VERSION, write_package};
use grind_core::odf::xml::esc;

use crate::model::{Block, BlockKind, Document, Run};
use crate::style::CharStyle;

pub use grind_core::Form;

/// The media type, byte for byte; readers sniff it at a fixed offset in a package (§1.1).
pub const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

pub fn write(doc: &Document, form: Form) -> Result<Vec<u8>> {
    // The projection is not XML; it leaves here, at the crate's one door out (`doc/dsl.md` §9).
    if form == Form::Projection {
        return Ok(crate::projection::save(doc));
    }
    // A save into the other form of the file this document came from is a save into its own form,
    // with every guard, moved across by `forms::convert`, which carries every part.
    if let Some(source) = doc.source.as_deref()
        && source.form != form
        && source.form != Form::Projection
    {
        let own = write(doc, source.form)?;
        let converted = grind_core::odf::forms::convert(&own, form)?;
        verify(doc, &converted)?;
        return Ok(converted);
    }
    // The file this document came from, when it is being saved in the same form.
    let source = doc.source.as_deref().filter(|source| source.form == form);

    // R6 first: a document whose blocks were only edited in place goes back as its file with those
    // elements replaced. Otherwise the body regenerates and is merged into the original
    // (`envelope::merge`), keeping what the model does not own.
    let content = match splice(doc, form) {
        Some(spliced) => spliced,
        None => {
            let reserved = source
                .map(|source| envelope::automatic_style_names(&source.bytes))
                .unwrap_or_default();
            let generated = content(doc, form, source, &reserved);
            match source.and_then(|source| envelope::merge(&source.bytes, generated.as_bytes())) {
                Some(merged) => merged,
                // An original too broken to merge into: write a whole document of our own, every
                // style declared.
                None => content(doc, form, None, &reserved).into_bytes(),
            }
        }
    };
    // Check whatever touched somebody's file before it replaces it: unreadable output, or output
    // that drops what the model never read, is refused.
    if let Some(source) = source {
        envelope::check_against(&source.bytes, &content)?;
        let lost = envelope::losses(&source.bytes, &content, &owned).unwrap_or_default();
        if !lost.is_empty() {
            return Err(grind_core::Error::WouldLose(lost));
        }
    }
    let bytes = match (form, source.and_then(|source| source.package.as_deref())) {
        (Form::Package, Some(original)) => envelope::repackage(original, &content, &[], &[], &[])?,
        (Form::Package, None) => write_package(MIMETYPE, &String::from_utf8_lossy(&content))?,
        _ => content,
    };
    // Check the bytes that will be written, package and all: a picture's `xlink:href` names a
    // package part.
    if source.is_some() {
        verify(doc, &bytes)?;
    }
    Ok(bytes)
}

/// What is about to replace somebody's file, read back, with every block's kind, table cell and
/// text compared with the document — so a splice that kept the file's bytes where it should have
/// written the model's cannot quietly undo an edit.
fn verify(doc: &Document, content: &[u8]) -> Result<()> {
    let refuse = |why: String| -> Result<()> {
        Err(grind_core::Error::Xml(format!(
            "not saved: the file would not read back as the document ({why}) — this is a bug"
        )))
    };
    let back = match super::read(content) {
        Ok(back) => back,
        Err(error) => return refuse(error.to_string()),
    };
    if back.blocks.len() != doc.blocks.len() {
        return refuse(format!(
            "{} blocks, not {}",
            back.blocks.len(),
            doc.blocks.len()
        ));
    }
    for (i, (mine, theirs)) in doc.blocks.iter().zip(&back.blocks).enumerate() {
        if mine.kind != theirs.kind || mine.text() != theirs.text() {
            return refuse(format!("block p{}", i + 1));
        }
        let cell = |b: &Block| b.cell.as_ref().map(|c| (c.table.clone(), c.row, c.column));
        if cell(mine) != cell(theirs) {
            return refuse(format!("block p{}'s table cell", i + 1));
        }
    }
    Ok(())
}

/// Whether a body key ([`envelope::body_vocabulary`]) is this writer's to change: an element of
/// `doc/text-core.md`'s scope line, or an attribute the model carries or puts back (`Origin`, a
/// spliced block's kept attributes). Anything else going down refuses the save.
fn owned(key: &str) -> bool {
    let (element, attribute) = match key.split_once('@') {
        Some((element, attribute)) => (element, Some(attribute)),
        None => (key, None),
    };
    let name = |uri: &str, local: &str| format!("{{{uri}}}{local}");
    let is = |uri: &str, locals: &[&str]| locals.iter().any(|local| element == name(uri, local));
    let modelled = is(
        TEXT,
        &["p", "h", "list", "list-item", "list-header", "span", "s"],
    ) || is(
        TEXT,
        &["tab", "line-break", "a", "bookmark", "soft-page-break"],
    ) || is(DRAW, &["frame", "image"])
        || is(OFFICE, &["binary-data", "text"])
        || is(TABLE, &["table", "table-column", "table-row", "table-cell"])
        || is(TABLE, &["covered-table-cell"]);
    // A footnote or field is kept whole (`Run::Kept`) and written back as the file's bytes, so one
    // that is gone was erased.
    let kept = is(TEXT, &["note", "note-citation", "note-body"])
        || is(TEXT, super::read::FIELDS)
        || is(OFFICE, &["annotation", "annotation-end"])
        || is(TEXT, &["change", "change-start", "change-end"])
        || is(DUBLIN_CORE, &["creator", "date"]);
    if kept {
        return true;
    }
    let Some(attribute) = attribute else {
        return modelled;
    };
    // LibreOffice's revision-save ids: editing-session bookkeeping, recomputed on its next save.
    if attribute.starts_with(&format!("{{{OFFICEOOO}}}")) {
        return true;
    }
    let attr = |uri: &str, local: &str| attribute == name(uri, local);
    match () {
        // A block's own element: the model writes it, and its other attributes are put back
        // verbatim.
        _ if is(TEXT, &["p", "h"]) => true,
        // A list opens with the attributes its `text:list` had in the file (`Origin::list`).
        _ if is(TEXT, &["list", "list-item", "list-header"]) => true,
        _ if is(TEXT, &["span"]) => attr(TEXT, "style-name"),
        _ if is(TEXT, &["s"]) => attr(TEXT, "c"),
        _ if is(TEXT, &["bookmark"]) => attr(TEXT, "name"),
        // A hyperlink's target is the model's; how a viewer opens it is a hint.
        _ if is(TEXT, &["a"]) => {
            attr(XLINK, "href")
                || attr(XLINK, "type")
                || attr(XLINK, "show")
                || attr(OFFICE, "target-frame-name")
        }
        _ if is(DRAW, &["frame"]) => {
            attr(SVG, "width") || attr(SVG, "height") || attr(TEXT, "anchor-type")
        }
        // A package's picture is written back inline, so where it was stored goes.
        _ if is(DRAW, &["image"]) => {
            attr(DRAW, "mime-type") || attribute.starts_with(&format!("{{{XLINK}}}"))
        }
        _ if is(TABLE, &["table"]) => attr(TABLE, "name"),
        _ if is(TABLE, &["table-column"]) => attr(TABLE, "number-columns-repeated"),
        _ if is(TABLE, &["table-cell"]) => {
            attr(TABLE, "number-columns-spanned") || attr(TABLE, "number-rows-spanned")
        }
        _ => false,
    }
}

/// Dublin Core — a comment's author and date (`dc:creator`, `dc:date` inside an
/// `office:annotation`).
const DUBLIN_CORE: &str = "http://purl.org/dc/elements/1.1/";

/// LibreOffice's own namespace for editing-session bookkeeping (`officeooo:rsid`).
const OFFICEOOO: &str = "http://openoffice.org/2009/office";

/// The file this document was read from, with the edited blocks put back in place. `None` means
/// regenerate, never failure; `odf::source` explains each boundary.
fn splice(doc: &Document, form: Form) -> Option<Vec<u8>> {
    let source = doc.source.as_deref()?;
    // Saving as the other form is a conversion, not an edit.
    if source.form != form {
        return None;
    }
    // The block sequence moved, so the file's structure and the model's no longer correspond.
    if doc.edits.structural {
        return None;
    }
    // A document that never had an image never declared `draw:`, and a splice never touches the
    // root tag, so a first image regenerates — as does a first size or alternative text in a file
    // declaring `draw:` but not `svg:`.
    let declares = |uri: &str| source.bytes.windows(uri.len()).any(|w| w == uri.as_bytes());
    let images = || doc.blocks.iter().flat_map(|b| b.runs.iter());
    if images().any(|r| matches!(r, Run::Image { .. })) && !declares(DRAW) {
        return None;
    }
    if images().any(|r| {
        matches!(r, Run::Image { width, height, title, description, .. }
            if width.is_some() || height.is_some() || title.is_some() || description.is_some())
    }) && !declares(SVG)
    {
        return None;
    }
    // Likewise `xlink:` for a document's first link, or the spliced `text:a` would name an unbound
    // prefix.
    if doc
        .blocks
        .iter()
        .flat_map(|b| b.runs.iter())
        .any(|r| matches!(r, Run::Text { href: Some(_), .. }))
        && !source
            .bytes
            .windows(XLINK.len())
            .any(|w| w == XLINK.as_bytes())
    {
        return None;
    }

    // Every character style the edited blocks need, under the name this file gives it; a
    // formatting the file has no name for cannot be spliced. Only the edited blocks: an untouched
    // block goes back as its own bytes, so a run without an exact twin among the file's automatic
    // styles must not turn an untouched save into a regenerate (`tdf97879.odt`).
    let pool = Pool::spliced(
        doc.blocks
            .iter()
            .filter(|block| doc.edits.blocks.contains(&block.id))
            .flat_map(|block| block.runs.iter())
            .filter_map(Run::props)
            .filter(|props| props.needs_style()),
        source,
    )?;
    // A formatting the file has no style for is declared at the end of its own automatic styles —
    // the one splice site outside the body — so a click on Bold stays a splice.
    let mut patches: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut declarations = pool.declarations();
    if opens_lists(doc, Some(source))
        && !envelope::automatic_style_names(&source.bytes).contains(BULLETS)
    {
        declarations.push_str(&bullets_declaration());
    }
    if !declarations.is_empty() {
        let at = envelope::automatic_styles_end(&source.bytes)?;
        patches.push((at..at, declarations));
    }

    // The elements to rewrite. An edited block in no element the file spelled means regenerating
    // rather than a document half in its original bytes.
    for block in &doc.blocks {
        if !doc.edits.blocks.contains(&block.id) {
            continue;
        }
        let at = source.blocks.get(&block.id)?;
        // A block that moved into or out of a list, to another depth or into another cell needs
        // surrounding elements the splice does not touch.
        let depth = match block.kind {
            BlockKind::ListItem { depth } => depth,
            _ => 0,
        };
        let same_cell =
            |a: &Option<crate::model::Cell>, b: &Option<crate::model::Cell>| match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => a.is_same(b),
                _ => false,
            };
        if depth != at.depth || !same_cell(&block.cell, &at.cell) {
            return None;
        }
        let mut out = String::new();
        // No indentation: the bytes before the element are the file's own.
        paragraph(&mut out, block, String::new(), &at.keep, &pool);
        patches.push((at.range.clone(), out.trim_end().to_owned()));
    }
    // An edited block whose id no longer appears (a `SetBlock` that replaced the id) would leave a
    // stale element; `structural` catches sequence changes and this catches identity changes.
    if doc
        .edits
        .blocks
        .iter()
        .any(|id| source.blocks.contains_key(id) && !doc.blocks.iter().any(|b| b.id == *id))
    {
        return None;
    }

    // In file order. Siblings never overlap, but a corrupt span would tangle bytes, so it is
    // refused.
    patches.sort_by_key(|(range, _)| range.start);
    if patches.windows(2).any(|w| w[0].0.end > w[1].0.start) {
        return None;
    }

    let mut out = Vec::with_capacity(source.bytes.len());
    let mut at = 0usize;
    for (range, text) in patches {
        out.extend_from_slice(source.bytes.get(at..range.start)?);
        out.extend_from_slice(text.as_bytes());
        at = range.end;
    }
    out.extend_from_slice(source.bytes.get(at..)?);
    Some(out)
}

/// Which namespaces a document's content actually needs (§1.4).
struct Used {
    xlink: bool,
    /// `style:` and `fo:`, which arrive together: everything written in either is a `style:style`
    /// of `fo:` properties.
    styles: bool,
    /// `draw:` and `svg:`, which arrive together: an image's frame and its size.
    image: bool,
    /// `table:`, in a document with a table — the spreadsheet's vocabulary (`doc/text-core.md`).
    table: bool,
}

impl Used {
    fn of(doc: &Document, pool: &Pool) -> Self {
        let runs = || doc.blocks.iter().flat_map(|b| b.runs.iter());
        Used {
            xlink: runs().any(|r| matches!(r, Run::Text { href: Some(_), .. })),
            styles: !pool.is_empty(),
            image: runs().any(|r| matches!(r, Run::Image { .. })),
            table: doc.blocks.iter().any(|b| b.cell.is_some()),
        }
    }
}

/// The character styles a document's runs need, each under the name it is written with. ODF puts
/// formatting on a `style:style` named from a `text:span`, so direct formatting means inventing
/// names, pooled so a thousand bold words share one declaration. A style here never inherits: a
/// run with a named style gets a span for the name around the span for the formatting
/// ([`crate::odf::source::Source::style_named`]).
#[derive(Default)]
struct Pool {
    /// Formatting to the name it is written under, in the order names were handed out.
    entries: Vec<(CharStyle, String)>,
    /// Names the file already declares, whose declarations are left as the file spells them.
    declared: std::collections::HashSet<String>,
    /// The file's own picture frames by what each read as ([`super::source::Source::frames`]),
    /// each handed out once so two identical pictures do not share one `draw:name`.
    frames: Vec<(u64, String)>,
    used: std::cell::RefCell<Vec<bool>>,
    /// New styles built from the one their run was read from, by name: that element renamed with
    /// only the changed properties rewritten, so unread superscript, language or letter spacing
    /// survive Bold.
    patched: std::collections::HashMap<String, String>,
}

impl Pool {
    /// Every distinct formatting in the document, named `T1`, `T2`, … in order of first
    /// appearance, so one document saves to the same bytes twice. Written for a file it came from,
    /// a formatting the file declares keeps its name, and a new one never takes a name in
    /// `reserved` — the file's automatic styles, which survive beside these (`envelope::merge`).
    fn of(
        doc: &Document,
        source: Option<&super::source::Source>,
        reserved: &std::collections::HashSet<String>,
    ) -> Self {
        let mut pool = Pool::default();
        let mut next = 1;
        for props in props_of(doc) {
            if pool.name(props).is_some() {
                continue;
            }
            // Formatting read from a style that says more than the model: its own name while the
            // model's part is unchanged, else a style built from it.
            if let Some(base) = source.and_then(|source| source.base_of(props)) {
                if base.props == *props {
                    pool.declared.insert(base.name.clone());
                    pool.entries.push((props.clone(), base.name.clone()));
                    continue;
                }
                let name = loop {
                    let name = format!("T{next}");
                    next += 1;
                    if !reserved.contains(&name)
                        && !pool.entries.iter().any(|(_, taken)| *taken == name)
                    {
                        break name;
                    }
                };
                if let Some(element) = source.and_then(|source| based(source, base, props, &name)) {
                    pool.patched.insert(name.clone(), element);
                }
                pool.entries.push((props.clone(), name));
                continue;
            }
            let reused = source
                .and_then(|source| source.style_named(props))
                .filter(|name| !pool.entries.iter().any(|(_, taken)| taken == name));
            let name = match reused {
                Some(name) => {
                    pool.declared.insert(name.to_owned());
                    name.to_owned()
                }
                None => loop {
                    let name = format!("T{next}");
                    next += 1;
                    if !reserved.contains(&name)
                        && !pool.entries.iter().any(|(_, taken)| *taken == name)
                    {
                        break name;
                    }
                },
            };
            pool.entries.push((props.clone(), name));
        }
        if let Some(source) = source {
            pool.take_frames(source);
        }
        pool
    }

    /// Remember the file's own frames, so [`Pool::frame`] can write one back.
    fn take_frames(&mut self, source: &super::source::Source) {
        self.frames = source
            .frames
            .iter()
            .filter_map(|(key, range)| {
                let bytes = source.bytes.get(range.clone())?;
                Some((*key, std::str::from_utf8(bytes).ok()?.to_owned()))
            })
            .collect();
        self.used = std::cell::RefCell::new(vec![false; self.frames.len()]);
    }

    /// The file's own `draw:frame` for this image run, if the file spelled one that read as
    /// exactly this picture and it has not been written yet.
    fn frame(&self, run: &Run) -> Option<&str> {
        let key = super::source::image_key(run)?;
        let mut used = self.used.borrow_mut();
        let at = self
            .frames
            .iter()
            .enumerate()
            .position(|(i, (k, _))| *k == key && !used[i])?;
        used[at] = true;
        Some(&self.frames[at].1)
    }

    /// The pool built only from names `source` declares, or `None` when the document needs a
    /// formatting the file has no name for. A splice replaces block elements only, so the names it
    /// uses must already be in the file.
    fn spliced<'a>(
        needed: impl Iterator<Item = &'a CharStyle>,
        source: &super::source::Source,
    ) -> Option<Self> {
        let mut pool = Pool::default();
        let reserved = envelope::automatic_style_names(&source.bytes);
        let standard = envelope::prefix_for(&source.bytes, STYLE).as_deref() == Some("style")
            && envelope::prefix_for(&source.bytes, FO).as_deref() == Some("fo");
        let mut next = 1;
        let mut fresh = |pool: &Pool| loop {
            let name = format!("T{next}");
            next += 1;
            if !reserved.contains(&name) && !pool.entries.iter().any(|(_, n)| *n == name) {
                break name;
            }
        };
        for props in needed {
            if pool.name(props).is_some() {
                continue;
            }
            match source.base_of(props) {
                // Unchanged formatting from a style the model does not wholly read: its name.
                Some(base) if base.props == *props => {
                    pool.declared.insert(base.name.clone());
                    pool.entries.push((props.clone(), base.name.clone()));
                }
                // Changed: a style built from that one, declared beside the file's own.
                Some(base) => {
                    let name = fresh(&pool);
                    pool.patched
                        .insert(name.clone(), based(source, base, props, &name)?);
                    pool.entries.push((props.clone(), name));
                }
                None => match source.style_named(props) {
                    Some(name) => {
                        pool.declared.insert(name.to_owned());
                        pool.entries.push((props.clone(), name.to_owned()));
                    }
                    // A formatting the file has no style for: one of this writer's own.
                    None if standard => {
                        let name = fresh(&pool);
                        pool.entries.push((props.clone(), name));
                    }
                    None => return None,
                },
            }
        }
        pool.take_frames(source);
        Some(pool)
    }

    /// Every style this pool declares that the file does not, as `style:style` elements for a
    /// splice to insert into the file's `office:automatic-styles`.
    fn declarations(&self) -> String {
        let mut out = String::new();
        for (props, name) in &self.entries {
            if self.declared.contains(name) {
                continue;
            }
            match self.patched.get(name) {
                Some(element) => out.push_str(element),
                None => {
                    let _ = write!(
                        out,
                        "<style:style style:name=\"{}\" style:family=\"text\">\
                         <style:text-properties{}/></style:style>",
                        esc(name),
                        props.attributes()
                    );
                }
            }
        }
        out
    }

    fn name(&self, props: &CharStyle) -> Option<&str> {
        self.entries
            .iter()
            .find(|(style, _)| style.same(props))
            .map(|(_, name)| name.as_str())
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `base`'s element in the file, renamed `name`, with what differs from `props` rewritten
/// (`envelope::patch_style`). `None` when the file spells `style:` or `fo:` with other prefixes or
/// the element is missing, and the run gets a style of its own.
fn based(
    source: &super::source::Source,
    base: &super::source::TextStyle,
    props: &CharStyle,
    name: &str,
) -> Option<String> {
    let standard = envelope::prefix_for(&source.bytes, STYLE).as_deref() == Some("style")
        && envelope::prefix_for(&source.bytes, FO).as_deref() == Some("fo");
    if !standard {
        return None;
    }
    let range = envelope::automatic_style_range(&source.bytes, "text", &base.name)?;
    let element = std::str::from_utf8(source.bytes.get(range)?).ok()?;
    envelope::patch_style(
        element,
        &[("style:name", Some(name))],
        &[("style:text-properties", props.patch_against(&base.props))],
        &["style:text-properties"],
    )
}

/// Every non-plain run formatting in the document, in document order.
fn props_of(doc: &Document) -> impl Iterator<Item = &CharStyle> {
    doc.blocks
        .iter()
        .flat_map(|block| block.runs.iter())
        .filter_map(Run::props)
        .filter(|props| props.needs_style())
}

/// The `content.xml` payload, which in the flat form is the whole document.
fn content(
    doc: &Document,
    form: Form,
    source: Option<&super::source::Source>,
    reserved: &std::collections::HashSet<String>,
) -> String {
    let root = match form {
        Form::Package => "office:document-content",
        // The projection never reaches here — `write` refuses it before there is any XML.
        Form::Flat | Form::Projection => "office:document",
    };
    // An index's generated entries are never written as blocks; the index element is carried
    // whole.
    let without;
    let doc = match doc.blocks.iter().any(|block| block.generated) {
        true => {
            let mut copy = doc.clone();
            copy.blocks.retain(|block| !block.generated);
            without = copy;
            &without
        }
        false => doc,
    };
    let pool = Pool::of(doc, source, reserved);
    let mut used = Used::of(doc, &pool);
    let bullets = opens_lists(doc, source) && !reserved.contains(BULLETS);
    used.styles |= bullets;

    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = write!(
        out,
        "<{root} xmlns:office=\"{OFFICE}\" xmlns:text=\"{TEXT}\""
    );
    // `xlink:` appears only in a document that links to something.
    if used.xlink {
        let _ = write!(out, " xmlns:xlink=\"{XLINK}\"");
    }
    // `style:` and `fo:` only in one that formats something.
    if used.styles {
        let _ = write!(out, " xmlns:style=\"{STYLE}\" xmlns:fo=\"{FO}\"");
    }
    // `draw:` and `svg:` only in one that has a picture in it.
    if used.image {
        let _ = write!(out, " xmlns:draw=\"{DRAW}\" xmlns:svg=\"{SVG}\"");
    }
    // `table:` only in one that has a table in it.
    if used.table {
        let _ = write!(out, " xmlns:table=\"{TABLE}\"");
    }
    let _ = write!(out, " office:version=\"{VERSION}\"");
    if form == Form::Flat {
        let _ = write!(out, " office:mimetype=\"{MIMETYPE}\"");
    }
    out.push_str(">\n");
    automatic_styles(&mut out, &pool, bullets);
    out.push_str(" <office:body>\n  <office:text>\n");
    let origin = source.map(|source| Origin::new(source, &doc.edits.blocks, &doc.list_marks));
    body(&mut out, doc, &pool, origin.as_ref());
    out.push_str("  </office:text>\n </office:body>\n");
    let _ = writeln!(out, "</{root}>");
    out
}

/// The list style a list this writer opens is given; a list from the file keeps its own
/// (`Origin::list`). Without one LibreOffice draws no mark at all (`doc/odt-format.md` §5c fact
/// 15).
const BULLETS: &str = "Lgrind1";

/// [`BULLETS`]'s declaration: the marks every window draws (`grind_text::paint::bullet`), a
/// quarter inch further in per level, in LibreOffice's label-alignment form.
fn bullets_declaration() -> String {
    let mut out = format!("<text:list-style style:name=\"{BULLETS}\">");
    for level in 1..=10u32 {
        let margin = 0.635 * f64::from(level + 1);
        let _ = write!(
            out,
            "<text:list-level-style-bullet text:level=\"{level}\" text:bullet-char=\"{}\">\
             <style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
             <style:list-level-label-alignment text:label-followed-by=\"listtab\" \
             text:list-tab-stop-position=\"{margin:.3}cm\" fo:text-indent=\"-0.635cm\" \
             fo:margin-left=\"{margin:.3}cm\"/></style:list-level-properties>\
             </text:list-level-style-bullet>",
            crate::paint::bullet(level)
        );
    }
    out.push_str("</text:list-style>");
    out
}

/// Whether `doc` has a list this writer opens, needing [`BULLETS`].
fn opens_lists(doc: &Document, source: Option<&super::source::Source>) -> bool {
    doc.blocks.iter().any(|block| {
        matches!(block.kind, BlockKind::ListItem { .. })
            && source
                .and_then(|source| source.lists.get(&block.id))
                .and_then(|lists| lists.first())
                .is_none()
    })
}

/// The `office:automatic-styles` block — one `style:style` per distinct run formatting — ahead of
/// `office:body`, so a name is declared before a span uses it.
fn automatic_styles(out: &mut String, pool: &Pool, bullets: bool) {
    if !bullets
        && pool
            .entries
            .iter()
            .all(|(_, name)| pool.declared.contains(name))
    {
        return;
    }
    out.push_str(" <office:automatic-styles>\n");
    if bullets {
        let _ = writeln!(out, "  {}", bullets_declaration());
    }
    for (props, name) in &pool.entries {
        if pool.declared.contains(name) {
            continue;
        }
        if let Some(element) = pool.patched.get(name) {
            let _ = writeln!(out, "  {element}");
            continue;
        }
        let _ = writeln!(
            out,
            "  <style:style style:name=\"{}\" style:family=\"text\">",
            esc(name)
        );
        let _ = writeln!(out, "   <style:text-properties{}/>", props.attributes());
        out.push_str("  </style:style>\n");
    }
    out.push_str(" </office:automatic-styles>\n");
}

/// The body: runs of ordinary blocks and the tables between them. Two folds over one flat
/// sequence: `text:list` from block depths and `table:table` from cell coordinates
/// (`crate::model::Cell`). Tables are the outer fold, since a cell holds blocks and a list item
/// holds no table.
fn body(out: &mut String, doc: &Document, pool: &Pool, origin: Option<&Origin>) {
    // What the file had in its body without a block, by the index of the block it now follows
    // (`None` for before every block).
    let placed = origin
        .map(|origin| origin.placements(doc))
        .unwrap_or_default();
    let carry = |out: &mut String, after: Option<usize>| {
        for bytes in placed.get(&after).into_iter().flatten() {
            let _ = writeln!(out, "{}{bytes}", pad(3, origin));
        }
    };
    carry(out, None);

    // The sections open in the output, outermost first; each unit is wrapped in its first block's
    // sections, closing and opening only the difference.
    let mut open: Vec<usize> = Vec::new();
    let mut index = 0;
    while index < doc.blocks.len() {
        let wanted = section_path(doc, doc.blocks[index].section);
        let keep = open.iter().zip(&wanted).take_while(|(a, b)| a == b).count();
        while open.len() > keep {
            open.pop();
            let _ = writeln!(out, "{}</text:section>", pad(3, origin));
        }
        // An untouched section goes back as the file's own bytes.
        if let (Some(&outer), Some(origin)) = (wanted.get(keep), origin)
            && let Some(bytes) = untouched_section(doc, outer, index, origin)
        {
            let _ = writeln!(out, "{}{bytes}", pad(3, Some(origin)));
            let held = doc.sections[outer].blocks.len();
            for i in index..index + held {
                carry(out, Some(i));
            }
            index += held;
            continue;
        }
        for &section in &wanted[keep..] {
            let _ = writeln!(out, "{}{}", pad(3, origin), doc.sections[section].start);
            open.push(section);
        }
        let here = doc.blocks[index].section;
        match doc.blocks[index].cell {
            None => {
                let mut end = doc.blocks[index..]
                    .iter()
                    .position(|block| block.cell.is_some() || block.section != here)
                    .map_or(doc.blocks.len(), |offset| index + offset);
                // A kept element after one of these blocks ends the run, so a list it followed
                // closes before it as in the file.
                if let Some(at) = (index..end).find(|i| placed.contains_key(&Some(*i))) {
                    end = at + 1;
                }
                blocks(out, &doc.blocks[index..end], 0, pool, origin);
                carry(out, Some(end - 1));
                index = end;
            }
            Some(_) => {
                // The maximal run naming this table, asked of the model.
                let range = doc.table(index).unwrap_or(index..index + 1);
                // An untouched table goes back as the file's own bytes.
                match origin.and_then(|origin| untouched_table(doc, range.clone(), origin)) {
                    Some(bytes) => {
                        let _ = writeln!(out, "{}{bytes}", pad(3, origin));
                    }
                    None => table(out, doc, range.clone(), pool, origin),
                }
                for i in range.clone() {
                    carry(out, Some(i));
                }
                index = range.end;
            }
        }
    }
    for _ in open {
        let _ = writeln!(out, "{}</text:section>", pad(3, origin));
    }
}

/// The file's own bytes for the section at `section`, when the blocks from `index` on are
/// exactly the blocks it held, in order, and none of them has been edited.
fn untouched_section<'a>(
    doc: &Document,
    section: usize,
    index: usize,
    origin: &Origin<'a>,
) -> Option<&'a str> {
    let info = doc.sections.get(section)?;
    let held = doc.blocks.get(index..index + info.blocks.len())?;
    let same = held.len() == info.blocks.len()
        && held
            .iter()
            .zip(&info.blocks)
            .all(|(block, id)| block.id == *id && !origin.edited.contains(id));
    if !same || held.is_empty() {
        return None;
    }
    std::str::from_utf8(origin.source.bytes.get(info.range.clone()?)?).ok()
}

/// The file's own bytes for the table occupying `range`, when it holds exactly the blocks it
/// was read with, in order, none of them edited.
fn untouched_table<'a>(
    doc: &Document,
    range: std::ops::Range<usize>,
    origin: &Origin<'a>,
) -> Option<&'a str> {
    let name = &doc.blocks[range.start].cell.as_ref()?.table;
    let recorded = origin.source.tables.get(name)?;
    let held = &doc.blocks[range];
    let same = !held.is_empty()
        && held.len() == recorded.blocks.len()
        && held
            .iter()
            .zip(&recorded.blocks)
            .all(|(block, id)| block.id == *id && !origin.edited.contains(id));
    if !same {
        return None;
    }
    std::str::from_utf8(origin.source.bytes.get(recorded.range.clone()?)?).ok()
}

/// The sections a block in `section` is inside, outermost first.
fn section_path(doc: &Document, section: Option<usize>) -> Vec<usize> {
    let mut path = Vec::new();
    let mut at = section;
    while let Some(index) = at.filter(|i| *i < doc.sections.len() && path.len() < 64) {
        path.push(index);
        at = doc.sections[index].parent;
    }
    path.reverse();
    path
}

/// The file a regenerated body is written for, and what changed since. An unedited block is
/// written as the file's own bytes, keeping what the model never read in it; an edited one keeps
/// its unmodelled attributes; a list opens with its `text:list`'s attributes; and every body
/// element without a block goes back after the block it followed.
struct Origin<'a> {
    source: &'a super::source::Source,
    edited: &'a std::collections::BTreeSet<crate::model::BlockId>,
    /// Which list item each list block was read in (`crate::numbering::ListMark`) — a header, an
    /// item's later paragraph, an item with its own start value — so the fold writes those back.
    marks: &'a crate::numbering::Marks,
    /// Original lists already opened, by where they start: a second opening (the same list split
    /// by an edit) must not repeat its `xml:id`.
    opened: std::cell::RefCell<std::collections::HashSet<usize>>,
    /// How much deeper or shallower than this writer the file indents its body, measured off the
    /// first kept element.
    shift: isize,
}

impl<'a> Origin<'a> {
    fn new(
        source: &'a super::source::Source,
        edited: &'a std::collections::BTreeSet<crate::model::BlockId>,
        marks: &'a crate::numbering::Marks,
    ) -> Self {
        // The whitespace in front of the first element of the body, back to its line's start.
        let first = source
            .blocks
            .values()
            .map(|at| at.range.start)
            .chain(source.siblings.iter().map(|s| s.range.start))
            .min();
        let shift = first
            .and_then(|start| {
                let line = source.bytes[..start]
                    .iter()
                    .rposition(|c| *c == b'\n')
                    .map_or(0, |p| p + 1);
                let lead = &source.bytes[line..start];
                lead.iter()
                    .all(|c| *c == b' ')
                    .then(|| lead.len() as isize - 4)
            })
            .unwrap_or(0);
        Origin {
            source,
            edited,
            marks,
            opened: Default::default(),
            shift,
        }
    }

    /// The file's own bytes for a block nobody edited.
    fn verbatim(&self, block: &Block) -> Option<&'a str> {
        if self.edited.contains(&block.id) {
            return None;
        }
        let at = self.source.blocks.get(&block.id)?;
        std::str::from_utf8(self.source.bytes.get(at.range.clone())?).ok()
    }

    /// The unmodelled attributes of an edited block's original element.
    fn keep(&self, block: &Block) -> &'a str {
        self.source
            .blocks
            .get(&block.id)
            .map_or("", |at| at.keep.as_str())
    }

    /// How many levels of list `a` and `b` share in the file; `u32::MAX` when either was not in
    /// it, since a new block continues the list it was made in.
    fn shared_lists(&self, a: &Block, b: &Block) -> u32 {
        match (self.source.lists.get(&a.id), self.source.lists.get(&b.id)) {
            (Some(a), Some(b)) => a.iter().zip(b).take_while(|(x, y)| x.0 == y.0).count() as u32,
            _ => u32::MAX,
        }
    }

    /// Whether `block`'s outermost list is one this writer opens, needing [`BULLETS`].
    fn new_list(&self, block: &Block) -> bool {
        self.source
            .lists
            .get(&block.id)
            .and_then(|lists| lists.first())
            .is_none()
    }

    /// The attributes for the `text:list` opened at `level` (0 outermost) around `block`.
    fn list(&self, block: &Block, level: u32) -> String {
        let Some((start, attributes)) = self
            .source
            .lists
            .get(&block.id)
            .and_then(|lists| lists.get(level as usize))
        else {
            return String::new();
        };
        if self.opened.borrow_mut().insert(*start) {
            return attributes.clone();
        }
        super::source::attributes(format!("<x{attributes}>").as_bytes(), &["xml:id"])
    }

    /// Every kept body element, by the index of the block it now follows: the one it followed in
    /// the file, or the nearest earlier surviving one.
    fn placements(&self, doc: &Document) -> std::collections::HashMap<Option<usize>, Vec<&'a str>> {
        let index: std::collections::HashMap<crate::model::BlockId, usize> = doc
            .blocks
            .iter()
            .enumerate()
            .map(|(i, block)| (block.id, i))
            .collect();
        // The file's blocks in file order, to walk back from a block that is gone.
        let mut order: Vec<(usize, crate::model::BlockId)> = self
            .source
            .blocks
            .iter()
            .map(|(id, at)| (at.range.start, *id))
            .collect();
        order.sort();
        let mut placed: std::collections::HashMap<Option<usize>, Vec<&'a str>> = Default::default();
        for sibling in &self.source.siblings {
            let Some(bytes) = self
                .source
                .bytes
                .get(sibling.range.clone())
                .and_then(|b| std::str::from_utf8(b).ok())
            else {
                continue;
            };
            let after = sibling.after.and_then(|id| {
                index.get(&id).copied().or_else(|| {
                    order
                        .iter()
                        .rev()
                        .filter(|(start, _)| *start < sibling.range.start)
                        .find_map(|(_, id)| index.get(id).copied())
                })
            });
            placed.entry(after).or_default().push(bytes);
        }
        placed
    }
}

/// One `table:table` (rng:15939) from its cells' blocks. A position no block names is an empty
/// cell, and one covered by a span is `table:covered-table-cell` (rng:14298), so a merged table
/// keeps its shape.
fn table(
    out: &mut String,
    doc: &Document,
    range: std::ops::Range<usize>,
    pool: &Pool,
    origin: Option<&Origin>,
) {
    let name = match doc.blocks[range.start].cell.as_ref() {
        Some(cell) => cell.table.clone(),
        None => return,
    };
    let (rows, columns) = doc.table_extent(range.clone());
    let blocks_in = &doc.blocks[range];
    // How the file spelled this table: its start tag, columns, rows and cells keep their own
    // attributes and styles.
    let recorded = origin.and_then(|origin| origin.source.tables.get(&name));

    match recorded {
        Some(recorded) => {
            let _ = writeln!(out, "{}{}", pad(3, origin), recorded.start);
        }
        None => {
            let _ = writeln!(
                out,
                "{}<table:table table:name=\"{}\">",
                pad(3, origin),
                esc(&name)
            );
        }
    }
    match recorded.filter(|r| r.columns.contains("table-column")) {
        Some(recorded) => {
            let _ = writeln!(out, "{}{}", pad(4, origin), recorded.columns);
        }
        None => {
            // At least one `table:table-column` is required (`oneOrMore`, rng:14200).
            let repeat = match columns.max(1) {
                1 => String::new(),
                n => format!(" table:number-columns-repeated=\"{n}\""),
            };
            let _ = writeln!(out, "{}<table:table-column{repeat}/>", pad(4, origin));
        }
    }
    let header = |row: u32| recorded.is_some_and(|r| r.header_rows.contains(&row));
    // A recorded cell's start tag with the model's spans and no repeat, since the model expanded
    // every repeat.
    let cell_tag = |row: u32, column: u32, element: &str, spans: Option<&crate::model::Cell>| {
        let tag = recorded
            .and_then(|r| r.cells.get(&(row, column)))
            .filter(|tag| tag.starts_with(&format!("<{element}")));
        match tag {
            Some(tag) => {
                let columns = spans
                    .filter(|c| c.columns_spanned > 1)
                    .map(|c| c.columns_spanned.to_string());
                let rows = spans
                    .filter(|c| c.rows_spanned > 1)
                    .map(|c| c.rows_spanned.to_string());
                let tag = envelope::set_attributes(
                    tag,
                    &[
                        ("table:number-columns-repeated", None),
                        ("table:number-columns-spanned", columns.as_deref()),
                        ("table:number-rows-spanned", rows.as_deref()),
                    ],
                );
                tag.trim_end_matches('>').to_owned()
            }
            None => format!(
                "<{element}{}",
                spans.map(span_attributes).unwrap_or_default()
            ),
        }
    };

    for row in 0..rows.max(1) {
        if header(row) && (row == 0 || !header(row - 1)) {
            let _ = writeln!(out, "{}<table:table-header-rows>", pad(4, origin));
        }
        let row_tag = recorded
            .and_then(|r| r.rows.get(&row))
            .filter(|tag| tag.starts_with("<table:table-row"))
            .map(|tag| envelope::set_attributes(tag, &[("table:number-rows-repeated", None)]))
            .unwrap_or_else(|| "<table:table-row>".to_owned());
        let _ = writeln!(out, "{}{row_tag}", pad(4, origin));
        let mut column = 0;
        while column < columns.max(1) {
            let cell = blocks_in.iter().find_map(|b| {
                b.cell
                    .as_ref()
                    .filter(|c| c.row == row && c.column == column)
            });
            match cell {
                Some(cell) => {
                    let open = cell_tag(row, column, "table:table-cell", Some(cell));
                    let content: Vec<Block> = blocks_in
                        .iter()
                        .filter(|b| b.cell.as_ref().is_some_and(|c| c.is_same(cell)))
                        .cloned()
                        .collect();
                    if content.iter().all(Block::is_empty) && content.len() <= 1 {
                        // An empty cell in the schema's shortest form; read back it is one empty
                        // paragraph, as LibreOffice reads it (`doc/odt-format.md` §5b).
                        let _ = writeln!(out, "{}{open}/>", pad(5, origin));
                    } else {
                        let _ = writeln!(out, "{}{open}>", pad(5, origin));
                        blocks(out, &content, 3, pool, origin);
                        let _ = writeln!(out, "{}</table:table-cell>", pad(5, origin));
                    }
                    column += 1;
                }
                // Nothing names this position: covered by a span above or to the left (the covered
                // element, counted as a column), or a gap (an empty cell).
                None => {
                    let element = match covered(blocks_in, row, column) {
                        true => "table:covered-table-cell",
                        false => "table:table-cell",
                    };
                    // A position no block names is written as the file had it, when it had it.
                    let element = recorded
                        .and_then(|r| r.cells.get(&(row, column)))
                        .map(|tag| {
                            if tag.starts_with("<table:covered-table-cell") {
                                "table:covered-table-cell"
                            } else {
                                "table:table-cell"
                            }
                        })
                        .unwrap_or(element);
                    let open = cell_tag(row, column, element, None);
                    let _ = writeln!(out, "{}{open}/>", pad(5, origin));
                    column += 1;
                }
            }
        }
        let _ = writeln!(out, "{}</table:table-row>", pad(4, origin));
        if header(row) && !header(row + 1) {
            let _ = writeln!(out, "{}</table:table-header-rows>", pad(4, origin));
        }
    }
    let _ = writeln!(out, "{}</table:table>", pad(3, origin));
}

/// `table:number-columns-spanned` / `table:number-rows-spanned` (rng:16102), only when not one.
fn span_attributes(cell: &crate::model::Cell) -> String {
    let mut out = String::new();
    if cell.columns_spanned > 1 {
        let _ = write!(
            out,
            " table:number-columns-spanned=\"{}\"",
            cell.columns_spanned
        );
    }
    if cell.rows_spanned > 1 {
        let _ = write!(out, " table:number-rows-spanned=\"{}\"", cell.rows_spanned);
    }
    out
}

/// Whether some cell's span reaches over this position.
fn covered(blocks: &[Block], row: u32, column: u32) -> bool {
    blocks.iter().filter_map(|b| b.cell.as_ref()).any(|cell| {
        (cell.columns_spanned > 1 || cell.rows_spanned > 1)
            && row >= cell.row
            && row < cell.row + cell.rows_spanned.max(1)
            && column >= cell.column
            && column < cell.column + cell.columns_spanned.max(1)
            && !(row == cell.row && column == cell.column)
    })
}

/// A sequence of blocks with `text:list` nesting folded back in from their depths. `extra` is how
/// much deeper than the body it sits — three levels in a table cell — so a list in a cell nests as
/// one in the body.
fn blocks(out: &mut String, list: &[Block], extra: u32, pool: &Pool, origin: Option<&Origin>) {
    // How many `text:list` elements are open: a depth rise opens, a fall closes, and the end
    // closes the rest.
    let mut open = 0u32;
    // Where a block outside any list sits: three levels into the body, `extra` more in a cell.
    let base = 3 + extra;

    let mut previous: Option<&Block> = None;
    // The item element open at each level, and whether the last thing written opened one.
    let mut items: Vec<&'static str> = Vec::new();
    let mut fresh = false;
    // The item a block opens: a header as a header, a start value kept, as the file had them.
    let item = |block: &Block| -> (&'static str, String) {
        match origin.and_then(|origin| origin.marks.get(&block.id)) {
            Some(mark) if mark.header => ("text:list-header", String::new()),
            Some(mark) => (
                "text:list-item",
                mark.start
                    .map(|n| format!(" text:start-value=\"{n}\""))
                    .unwrap_or_default(),
            ),
            None => ("text:list-item", String::new()),
        }
    };
    for block in list {
        let depth = match block.kind {
            BlockKind::ListItem { depth } => depth,
            _ => 0,
        };
        // Items the file held in different lists stay in different lists, even when nothing
        // separates them.
        let shared = match (origin, previous) {
            (Some(origin), Some(previous)) => origin.shared_lists(previous, block),
            _ => u32::MAX,
        };
        previous = Some(block);
        while open > depth.min(shared) {
            open -= 1;
            let name = items.pop().unwrap_or("text:list-item");
            let _ = writeln!(out, "{}</{name}>", pad(base + 2 * open + 1, origin));
            let _ = writeln!(out, "{}</text:list>", pad(base + 2 * open, origin));
            fresh = false;
        }

        // Close deeper lists, then open shallower ones, one element per level, indented as
        // LibreOffice indents a list.
        while open > depth {
            open -= 1;
            let name = items.pop().unwrap_or("text:list-item");
            let _ = writeln!(out, "{}</{name}>", pad(base + 2 * open + 1, origin));
            let _ = writeln!(out, "{}</text:list>", pad(base + 2 * open, origin));
            fresh = false;
        }
        while open < depth {
            // A list the file had opens with its own attributes; a new one with the list style
            // that gives it marks.
            let attributes = match origin {
                Some(origin) if !(open == 0 && origin.new_list(block)) => origin.list(block, open),
                _ if open == 0 => format!(" text:style-name=\"{BULLETS}\""),
                _ => String::new(),
            };
            let _ = writeln!(
                out,
                "{}<text:list{attributes}>",
                pad(base + 2 * open, origin)
            );
            let (name, attributes) = match open + 1 == depth {
                true => item(block),
                false => ("text:list-item", String::new()),
            };
            let _ = writeln!(
                out,
                "{}<{name}{attributes}>",
                pad(base + 2 * open + 1, origin)
            );
            items.push(name);
            fresh = true;
            open += 1;
        }
        // A sibling at the same depth closes the previous item and opens another, unless it is
        // that item's later paragraph.
        let later = origin
            .and_then(|origin| origin.marks.get(&block.id))
            .is_some_and(|mark| !mark.first);
        if depth > 0 && !fresh && !later {
            let at = pad(base + 2 * (open - 1) + 1, origin);
            let closed = items.pop().unwrap_or("text:list-item");
            let (name, attributes) = item(block);
            let _ = writeln!(out, "{at}</{closed}>");
            let _ = writeln!(out, "{at}<{name}{attributes}>");
            items.push(name);
        }
        fresh = false;

        let at = pad(base + 2 * open, origin);
        match origin.and_then(|origin| origin.verbatim(block)) {
            Some(bytes) => {
                let _ = writeln!(out, "{at}{bytes}");
            }
            None => paragraph(
                out,
                block,
                at,
                origin.map_or("", |origin| origin.keep(block)),
                pool,
            ),
        }
    }

    while open > 0 {
        open -= 1;
        let name = items.pop().unwrap_or("text:list-item");
        let _ = writeln!(out, "{}</{name}>", pad(base + 2 * open + 1, origin));
        let _ = writeln!(out, "{}</text:list>", pad(base + 2 * open, origin));
    }
}

/// `depth + 1` spaces, shifted to the file's own body indentation, so an Enter in a LibreOffice
/// document is two lines of diff.
fn pad(depth: u32, origin: Option<&Origin>) -> String {
    let shift = origin.map_or(0, |origin| origin.shift);
    " ".repeat((depth as isize + 1 + shift).max(0) as usize)
}

/// One `text:p` or `text:h`. `keep` is the original element's unmanaged attributes when splicing
/// (`text:class-names`, `xml:id`, a vendor's own), put back verbatim; empty when regenerating.
fn paragraph(out: &mut String, block: &Block, indent: String, keep: &str, pool: &Pool) {
    let (tag, extra) = match block.kind {
        BlockKind::Heading { level } => ("text:h", format!(" text:outline-level=\"{level}\"")),
        _ => ("text:p", String::new()),
    };
    let style = match &block.style {
        Some(name) => format!(" text:style-name=\"{}\"", esc(name)),
        None => String::new(),
    };
    // An empty paragraph spaces a document, and is written self-closed.
    if block.runs.is_empty() {
        let _ = writeln!(out, "{indent}<{tag}{style}{extra}{keep}/>");
        return;
    }
    let _ = write!(out, "{indent}<{tag}{style}{extra}{keep}>");
    for run in &block.runs {
        self::run(out, run, pool);
    }
    let _ = writeln!(out, "</{tag}>");
}

fn run(out: &mut String, run: &Run, pool: &Pool) {
    match run {
        Run::Text {
            text,
            style,
            props,
            href,
        } => {
            // Three wrappers, outermost first: the link, the document's style name, and the
            // generated style for the direct formatting — inside the named one rather than
            // inheriting from it, so a round trip never composes a name into itself
            // (`crate::odf::source::Source::style_named`).
            if let Some(href) = href {
                let _ = write!(out, "<text:a xlink:href=\"{}\">", esc(href));
            }
            if let Some(style) = style {
                let _ = write!(out, "<text:span text:style-name=\"{}\">", esc(style));
            }
            let direct = pool.name(props);
            if let Some(name) = direct {
                let _ = write!(out, "<text:span text:style-name=\"{}\">", esc(name));
            }
            characters(out, text);
            if direct.is_some() {
                out.push_str("</text:span>");
            }
            if style.is_some() {
                out.push_str("</text:span>");
            }
            if href.is_some() {
                out.push_str("</text:a>");
            }
        }
        Run::Tab => out.push_str("<text:tab/>"),
        Run::Break => out.push_str("<text:line-break/>"),
        Run::Bookmark { name } => {
            let _ = write!(out, "<text:bookmark text:name=\"{}\"/>", esc(name));
        }
        // An element the model keeps rather than reads goes back as the file spelled it.
        Run::Kept { xml, .. } => out.push_str(xml),
        Run::Image { .. } if let Some(frame) = pool.frame(run) => out.push_str(frame),
        Run::Image {
            mime,
            data,
            width,
            height,
            anchor,
            title,
            description,
        } => image(
            out,
            mime,
            data,
            [width.as_deref(), height.as_deref()],
            anchor.as_deref(),
            [title.as_deref(), description.as_deref()],
        ),
    }
}

/// One `draw:frame` holding a `draw:image`, always the flat shape (R3): an unedited paragraph
/// splices its own frame back, wrapper and all. The anchor is the document's own, since without
/// `text:anchor-type` an `as-char` picture would move to the top of its paragraph (loop C found
/// it); an inserted image gets `paragraph`.
fn image(
    out: &mut String,
    mime: &str,
    data: &[u8],
    [width, height]: [Option<&str>; 2],
    anchor: Option<&str>,
    [title, description]: [Option<&str>; 2],
) {
    use base64::Engine as _;
    let _ = write!(
        out,
        "<draw:frame text:anchor-type=\"{}\"",
        esc(anchor.unwrap_or("paragraph"))
    );
    if let Some(width) = width {
        let _ = write!(out, " svg:width=\"{}\"", esc(width));
    }
    if let Some(height) = height {
        let _ = write!(out, " svg:height=\"{}\"", esc(height));
    }
    let _ = write!(out, "><draw:image draw:mime-type=\"{}\">", esc(mime));
    out.push_str("<office:binary-data>");
    out.push_str(&base64::engine::general_purpose::STANDARD.encode(data));
    out.push_str("</office:binary-data></draw:image>");
    // After the image and in this order — `common-draw-shape-accessibility`, rng:1756.
    if let Some(title) = title {
        let _ = write!(out, "<svg:title>{}</svg:title>", esc(title));
    }
    if let Some(description) = description {
        let _ = write!(out, "<svg:desc>{}</svg:desc>", esc(description));
    }
    out.push_str("</draw:frame>");
}

/// Character data with every significant whitespace written as ODF's element for it: a consumer
/// collapses character-data whitespace, so runs of spaces become `text:s` with a count (rng:8408),
/// tabs `text:tab`, newlines `text:line-break`.
///
/// The first space of a run stays literal, so ordinary prose has no markup. A space with nothing
/// written before it in this run — at the start, or after a `text:tab` — would be trimmed, so its
/// whole run is encoded. `\r` and `\r\n` become one `text:line-break`, which reads back as `\n`
/// (XML 1.0 §2.11).
fn characters(out: &mut String, text: &str) {
    // Whether character data has been written since the last element; a space needs something in
    // front of it.
    let mut anchored = false;
    let mut rest = text;

    while let Some(i) = rest.find([' ', '\t', '\n', '\r']) {
        let (before, tail) = rest.split_at(i);
        if !before.is_empty() {
            out.push_str(&esc(before));
            anchored = true;
        }
        let eaten = match tail.as_bytes()[0] {
            b' ' => {
                let spaces = tail.bytes().take_while(|c| *c == b' ').count();
                let literal = usize::from(anchored);
                for _ in 0..literal {
                    out.push(' ');
                }
                match spaces - literal {
                    0 => {}
                    1 => out.push_str("<text:s/>"),
                    n => {
                        let _ = write!(out, "<text:s text:c=\"{n}\"/>");
                    }
                }
                spaces
            }
            b'\t' => {
                out.push_str("<text:tab/>");
                1
            }
            // `\r\n` is one line ending, not two.
            b'\r' => {
                out.push_str("<text:line-break/>");
                1 + usize::from(tail.as_bytes().get(1) == Some(&b'\n'))
            }
            _ => {
                out.push_str("<text:line-break/>");
                1
            }
        };
        rest = &tail[eaten..];
        anchored = false;
    }

    out.push_str(&esc(rest));
}
