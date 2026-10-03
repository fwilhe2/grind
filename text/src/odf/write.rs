// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Serialising a text document back to ODF. **\[ODT\]**
//!
//! The package layout, the manifest, the ODF version and XML escaping are the same for every
//! document type and live in `grind-core` (§1.1, §1.3) — this file is only the `office:text`
//! content model. The two physical forms share one content writer and differ in exactly two
//! places: the root element name and whether `office:mimetype` sits on it.
//!
//! **Minimal by intent (§1.4, R3):** no `styles.xml`, no `meta.xml`, no `settings.xml`, and a
//! namespace is declared only if the document actually uses it. A new text document is a
//! handful of lines rather than the several hundred a full office suite writes.
//!
//! Two things this writer has to get right that a naive one does not:
//!
//! * **Whitespace is re-encoded as elements.** XML character data is whitespace, and an ODF
//!   consumer collapses a run of it to one space — so a paragraph written with its spaces,
//!   tabs and newlines literal comes back with all three gone. `doc/odt-format.md` §3.3 calls
//!   this the `table:number-columns-repeated` trap in a new costume, and it is: the reader
//!   expands, the writer re-encodes into `text:s`, `text:tab` and `text:line-break`, and
//!   neither half is optional.
//! * **A list is reconstructed from block depths.** The model flattens `text:list` into the
//!   block sequence (`crate::model::BlockKind`), so writing folds the depths back into nesting
//!   — opening an element where the depth rises and closing where it falls.

use std::fmt::Write as _;

use grind_core::Result;
use grind_core::odf::envelope;
use grind_core::odf::names::{DRAW, FO, OFFICE, STYLE, SVG, TABLE, TEXT, XLINK};
use grind_core::odf::package::{VERSION, write_package};
use grind_core::odf::xml::esc;

use crate::model::{Block, BlockKind, Document, Run};
use crate::style::CharStyle;

pub use grind_core::Form;

/// The media type, byte for byte. Sniffed by readers at a fixed offset in the package form
/// (§1.1), so it is not somewhere to be creative.
pub const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

pub fn write(doc: &Document, form: Form) -> Result<Vec<u8>> {
    // The third form is not XML at all, so it leaves before any of this file runs
    // (`doc/dsl.md` §9, D2). It is here rather than one layer up because `write_bytes` is
    // the one door out of the crate, and a form that only *some* callers knew to handle
    // would be a form that escapes through the others.
    if form == Form::Projection {
        return Ok(crate::projection::save(doc));
    }
    // The file this document came from, when it is being saved in the same form. A save into
    // the other form is a conversion and starts from nothing.
    let source = doc.source.as_deref().filter(|source| source.form == form);

    // R6 first: a document that came from a file and has only had block *contents* edited goes
    // back as that file with those elements replaced. Otherwise the body regenerates — and is
    // **merged into the original** (`envelope::merge`), so the styles, master pages, metadata
    // and settings the model does not own are carried rather than dropped. Saving never makes
    // an existing file worse.
    let content = match splice(doc, form) {
        Some(spliced) => spliced,
        None => {
            let reserved = source
                .map(|source| envelope::automatic_style_names(&source.bytes))
                .unwrap_or_default();
            let generated = content(doc, form, source, &reserved);
            match source.and_then(|source| envelope::merge(&source.bytes, generated.as_bytes())) {
                Some(merged) => merged,
                // An original too broken to merge into: write a whole document of our own,
                // every style declared, rather than one leaning on declarations it lost.
                None => content(doc, form, None, &reserved).into_bytes(),
            }
        }
    };
    // Whatever touched somebody's file is checked before it can replace it: a save that fails
    // leaves the original on disk, and one that writes something unreadable — or that drops
    // what the model never read — does not.
    if let Some(source) = source {
        envelope::check_against(&source.bytes, &content)?;
        let lost = envelope::losses(&source.bytes, &content, &owned).unwrap_or_default();
        if !lost.is_empty() {
            return Err(grind_core::Error::WouldLose(lost));
        }
    }
    match (form, source.and_then(|source| source.package.as_deref())) {
        (Form::Package, Some(original)) => envelope::repackage(original, &content, &[], &[], &[]),
        (Form::Package, None) => write_package(MIMETYPE, &String::from_utf8_lossy(&content)),
        _ => Ok(content),
    }
}

/// Whether a body key ([`envelope::body_vocabulary`]) is this writer's to change: an element of
/// `doc/text-core.md`'s scope line, or an attribute the model carries or the writer puts back
/// (`Origin`, a spliced block's kept attributes). A count of anything else going down is
/// content the model never saw, and the save is refused rather than allowed to drop it.
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
    let Some(attribute) = attribute else {
        return modelled;
    };
    // LibreOffice's revision-save ids: editing-session bookkeeping, recomputed on its next save.
    if attribute.starts_with(&format!("{{{OFFICEOOO}}}")) {
        return true;
    }
    let attr = |uri: &str, local: &str| attribute == name(uri, local);
    match () {
        // A block's own element: what the model writes, and every other attribute is put back
        // verbatim — from the file's bytes for a block nobody edited, from its kept attributes
        // for one somebody did.
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

/// LibreOffice's own namespace for editing-session bookkeeping (`officeooo:rsid`).
const OFFICEOOO: &str = "http://openoffice.org/2009/office";

/// The file this document was read from, with the edited blocks put back in place.
///
/// `None` means "not applicable, regenerate" — never "failed". Every condition below is a
/// documented boundary of the trick rather than an error, and `odf::source` says why each one
/// is where it is.
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
    // A document that never carried an image never declared `draw:`/`svg:` either, and
    // splicing patches individual block elements without ever touching the root tag that
    // would have to carry the declaration — so an image appearing for the first time forces a
    // regenerate, the same way a style name the file has no room for does, just below. A
    // document read *with* an image already has the declaration on its own root, so this only
    // ever fires for one a person just added.
    if doc
        .blocks
        .iter()
        .flat_map(|b| b.runs.iter())
        .any(|r| matches!(r, Run::Image { .. }))
        && !source
            .bytes
            .windows(DRAW.len())
            .any(|w| w == DRAW.as_bytes())
    {
        return None;
    }

    // Every character style the document now needs, under the name this *file* gives it. A
    // formatting the file has no name for cannot be spliced, because the declaration would have
    // to go somewhere these patches do not reach.
    let pool = Pool::spliced(doc, source)?;

    // Which elements have to be rewritten. Every edited block must sit in one the file
    // actually spelled — one that does not means regenerating, because a document half in its
    // original bytes and half not would lose the other half silently.
    let mut patches: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for block in &doc.blocks {
        if !doc.edits.blocks.contains(&block.id) {
            continue;
        }
        let at = source.blocks.get(&block.id)?;
        let mut out = String::new();
        // No indentation: the bytes before the element are still the file's own, so the
        // element goes back exactly where it started.
        paragraph(&mut out, block, String::new(), &at.keep, &pool);
        patches.push((at.range.clone(), out.trim_end().to_owned()));
    }
    // An edited block whose id the source knows but which no longer appears — a `SetBlock` that
    // replaced the id — would leave a stale element behind. `structural` catches the sequence
    // changing; this catches the identity changing without it.
    if doc
        .edits
        .blocks
        .iter()
        .any(|id| source.blocks.contains_key(id) && !doc.blocks.iter().any(|b| b.id == *id))
    {
        return None;
    }

    // In file order, so the untouched stretches between are copied without seeking back.
    // Elements do not overlap by construction — they are siblings — but a corrupted span would
    // produce tangled bytes rather than an error, so refuse instead of trusting it.
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
    /// `style:` and `fo:`, which arrive together: the only thing this writer puts in either is
    /// a `style:style` full of `fo:` properties, so one flag covers both.
    styles: bool,
    /// `draw:` and `svg:`, which arrive together for the same reason — the only thing either
    /// namespace carries here is one image's frame and its size.
    image: bool,
    /// `table:`, in a document that has a table in it. The one namespace this writer shares
    /// with the spreadsheet, and it is the same vocabulary — a text table's rows and cells are
    /// spelled exactly as a sheet's are (`doc/text-core.md`).
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

/// The character styles a document's runs need, each under the name it will be written with.
///
/// ODF has no way to put formatting on a run directly: `fo:font-weight` lives on a
/// `style:style`, and a `text:span` refers to it by name. So writing direct formatting means
/// **inventing names**, and this is where they are invented — `grind_sheet::odf::write`'s cell
/// style pool, for prose, and pooling for the same reason: two runs that are bold in the same
/// way must share one declaration, or a document of a thousand bold words carries a thousand
/// identical styles.
///
/// A style here **never inherits**. A run that also carries a named style gets a span for the
/// name wrapped around the span for the formatting, rather than an automatic style whose parent
/// is the name — see [`crate::odf::source::Source::style_named`] for what that keeps true.
#[derive(Default)]
struct Pool {
    /// Formatting to the name it is written under, in the order names were handed out.
    entries: Vec<(CharStyle, String)>,
    /// Names taken from the file this document was read from, whose declaration is already
    /// there and is left exactly as the file spells it rather than re-declared.
    declared: std::collections::HashSet<String>,
}

impl Pool {
    /// Every distinct formatting in the document, named `T1`, `T2`, … in the order it first
    /// appears — so that saving one document twice produces the same bytes.
    ///
    /// Regenerating a document read from a file, a formatting the file already declares keeps
    /// the file's name for it, and a new one never takes a name in `reserved` — the file's own
    /// automatic styles, which survive the save beside these (`envelope::merge`) and may be
    /// named from outside the body.
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
        pool
    }

    /// The same pool built entirely out of names `source` already declares — `None` when the
    /// document needs a formatting the file has no name for, which is a regenerate.
    ///
    /// Splicing replaces block elements and nothing else, so a name it refers to has to already
    /// be in the bytes around them. Rather than splicing a second site inside
    /// `office:automatic-styles` — a *second* fragile offset, for an edit that is rare — the
    /// writer takes the honest fallback the spreadsheet takes for a cell style the file has no
    /// entry for.
    fn spliced(doc: &Document, source: &super::source::Source) -> Option<Self> {
        let mut pool = Pool::default();
        for props in props_of(doc) {
            if pool.name(props).is_some() {
                continue;
            }
            let name = source.style_named(props)?;
            pool.entries.push((props.clone(), name.to_owned()));
        }
        Some(pool)
    }

    fn name(&self, props: &CharStyle) -> Option<&str> {
        self.entries
            .iter()
            .find(|(style, _)| style == props)
            .map(|(_, name)| name.as_str())
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Every non-plain run formatting in the document, in document order.
fn props_of(doc: &Document) -> impl Iterator<Item = &CharStyle> {
    doc.blocks
        .iter()
        .flat_map(|block| block.runs.iter())
        .filter_map(Run::props)
        .filter(|props| !props.is_plain())
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
    let pool = Pool::of(doc, source, reserved);
    let used = Used::of(doc, &pool);

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
    automatic_styles(&mut out, &pool);
    out.push_str(" <office:body>\n  <office:text>\n");
    let origin = source.map(|source| Origin::new(source, &doc.edits.blocks));
    body(&mut out, doc, &pool, origin.as_ref());
    out.push_str("  </office:text>\n </office:body>\n");
    let _ = writeln!(out, "</{root}>");
    out
}

/// The `office:automatic-styles` block — one `style:style` per distinct run formatting.
///
/// Ahead of `office:body`, which the schema requires and a single-pass reader depends on: a
/// span refers to a name, and the name has to be declared by the time it does.
fn automatic_styles(out: &mut String, pool: &Pool) {
    if pool
        .entries
        .iter()
        .all(|(_, name)| pool.declared.contains(name))
    {
        return;
    }
    out.push_str(" <office:automatic-styles>\n");
    for (props, name) in &pool.entries {
        if pool.declared.contains(name) {
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

/// The body: runs of ordinary blocks, and the tables between them.
///
/// **Two folds, one flat sequence.** A `text:list` is reconstructed from block depths and a
/// `table:table` from the cell coordinates the blocks carry (`crate::model::Cell`) — both are
/// the same trade the model makes on purpose, and both are a walk rather than a traversal.
/// Tables are the outer one because a cell holds blocks and a list item does not hold a table
/// in this model.
fn body(out: &mut String, doc: &Document, pool: &Pool, origin: Option<&Origin>) {
    // What the file had in its body that the model has no block for, by the index of the block
    // it now follows — `None` for ahead of every block.
    let placed = origin
        .map(|origin| origin.placements(doc))
        .unwrap_or_default();
    let carry = |out: &mut String, after: Option<usize>| {
        for bytes in placed.get(&after).into_iter().flatten() {
            let _ = writeln!(out, "{}{bytes}", pad(3, origin));
        }
    };
    carry(out, None);

    let mut index = 0;
    while index < doc.blocks.len() {
        match doc.blocks[index].cell {
            None => {
                let mut end = doc.blocks[index..]
                    .iter()
                    .position(|block| block.cell.is_some())
                    .map_or(doc.blocks.len(), |offset| index + offset);
                // A kept element after one of these blocks ends the run there, so that a list
                // it followed closes before it, as it did in the file.
                if let Some(at) = (index..end).find(|i| placed.contains_key(&Some(*i))) {
                    end = at + 1;
                }
                blocks(out, &doc.blocks[index..end], 0, pool, origin);
                carry(out, Some(end - 1));
                index = end;
            }
            Some(_) => {
                // The maximal run naming this table — the model's own fold, asked of the
                // model rather than repeated here.
                let range = doc.table(index).unwrap_or(index..index + 1);
                table(out, doc, range.clone(), pool, origin);
                for i in range.clone() {
                    carry(out, Some(i));
                }
                index = range.end;
            }
        }
    }
}

/// The file a regenerated body is being written for, and what has changed since it was read.
///
/// A body regenerates because its block *sequence* changed — and that says nothing about the
/// blocks themselves. One that was not edited is written as the file's own bytes, which keeps
/// whatever the model never read inside it (a footnote, a field, a vendor's attribute); one that
/// was keeps its unmodelled attributes; a list opens with the attributes its `text:list` had;
/// and every element of the body the model has no block for is put back after the block it
/// followed. What is left for the model to write is only what it changed.
struct Origin<'a> {
    source: &'a super::source::Source,
    edited: &'a std::collections::BTreeSet<crate::model::BlockId>,
    /// The original lists already opened once, by where they start — a second opening (the
    /// same list, split by an edit) must not repeat its `xml:id`.
    opened: std::cell::RefCell<std::collections::HashSet<usize>>,
    /// How much deeper (or shallower) than this writer's own the file indents a block of its
    /// body — measured off the first element of the body it kept.
    shift: isize,
}

impl<'a> Origin<'a> {
    fn new(
        source: &'a super::source::Source,
        edited: &'a std::collections::BTreeSet<crate::model::BlockId>,
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

    /// Every kept element of the body, by the index of the block it now follows: the block it
    /// followed in the file, or — when that one is gone — the nearest earlier block of the
    /// file's that is still here.
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

/// One `table:table` (rng:15939), from the blocks of its cells.
///
/// The rectangle comes from the coordinates: a position no block names is written as an empty
/// cell, and a position covered by a span is written as `table:covered-table-cell` (rng:14298),
/// which is what keeps a merged table the same shape after a regenerate.
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

    let _ = writeln!(
        out,
        "{}<table:table table:name=\"{}\">",
        pad(3, origin),
        esc(&name)
    );
    // At least one `table:table-column` is required — `table-columns-and-groups` is a
    // `oneOrMore` (rng:14200) — so a table with no columns at all still declares one, and R2
    // (everything written validates) is why that is not a detail.
    let repeat = match columns.max(1) {
        1 => String::new(),
        n => format!(" table:number-columns-repeated=\"{n}\""),
    };
    let _ = writeln!(out, "{}<table:table-column{repeat}/>", pad(4, origin));

    for row in 0..rows.max(1) {
        let _ = writeln!(out, "{}<table:table-row>", pad(4, origin));
        let mut column = 0;
        while column < columns.max(1) {
            let cell = blocks_in.iter().find_map(|b| {
                b.cell
                    .as_ref()
                    .filter(|c| c.row == row && c.column == column)
            });
            match cell {
                Some(cell) => {
                    let spans = span_attributes(cell);
                    let content: Vec<Block> = blocks_in
                        .iter()
                        .filter(|b| b.cell.as_ref().is_some_and(|c| c.is_same(cell)))
                        .cloned()
                        .collect();
                    if content.iter().all(Block::is_empty) && content.len() <= 1 {
                        // An empty cell, written the way the schema's shortest form allows.
                        // Reading it back makes one empty paragraph again — the normalisation
                        // `TableCell::end` performs, and the same one LibreOffice performs
                        // (`doc/odt-format.md` §5b), so the two agree.
                        let _ = writeln!(out, "{}<table:table-cell{spans}/>", pad(5, origin));
                    } else {
                        let _ = writeln!(out, "{}<table:table-cell{spans}>", pad(5, origin));
                        blocks(out, &content, 3, pool, origin);
                        let _ = writeln!(out, "{}</table:table-cell>", pad(5, origin));
                    }
                    column += 1;
                }
                // Nothing names this position. Either a cell above or to the left covers it —
                // in which case ODF wants the covered element, and a reader counts it as a
                // column — or the model simply has a gap, which is an empty cell.
                None => {
                    let element = match covered(blocks_in, row, column) {
                        true => "table:covered-table-cell",
                        false => "table:table-cell",
                    };
                    let _ = writeln!(out, "{}<{element}/>", pad(5, origin));
                    column += 1;
                }
            }
        }
        let _ = writeln!(out, "{}</table:table-row>", pad(4, origin));
    }
    let _ = writeln!(out, "{}</table:table>", pad(3, origin));
}

/// `table:number-columns-spanned` / `table:number-rows-spanned` (rng:16102), written only where
/// they say something — a span of one is the default and LibreOffice drops it too.
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

/// A sequence of blocks, with `text:list` nesting folded back in from their depths.
///
/// `extra` is how much further in this sequence sits than the body does — three levels inside a
/// table cell, nothing at the top. The fold itself is the same either way, which is the point
/// of it being one function: a list inside a cell nests exactly as a list in the body does.
fn blocks(out: &mut String, list: &[Block], extra: u32, pool: &Pool, origin: Option<&Origin>) {
    // How many `text:list` elements are currently open. The model is flat and the file is
    // not, so this counter *is* the reconstruction: a depth rise opens elements, a fall closes
    // them, and the end of the sequence closes whatever is left.
    let mut open = 0u32;
    // Where a block outside any list sits: three levels into the body, `extra` more in a cell.
    let base = 3 + extra;

    for block in list {
        let depth = match block.kind {
            BlockKind::ListItem { depth } => depth,
            _ => 0,
        };

        // Close deeper lists, then open shallower ones, so a jump of two levels is two
        // elements rather than a malformed one.
        // A tree, one level per element: a list at `base`, its items one deeper, what they hold
        // one deeper again — the shape LibreOffice indents a list in, so a regenerated list in
        // its file lines up with the one it replaced.
        while open > depth {
            open -= 1;
            let _ = writeln!(out, "{}</text:list-item>", pad(base + 2 * open + 1, origin));
            let _ = writeln!(out, "{}</text:list>", pad(base + 2 * open, origin));
        }
        while open < depth {
            let attributes = origin.map_or(String::new(), |origin| origin.list(block, open));
            let _ = writeln!(
                out,
                "{}<text:list{attributes}>",
                pad(base + 2 * open, origin)
            );
            let _ = writeln!(out, "{}<text:list-item>", pad(base + 2 * open + 1, origin));
            open += 1;
        }
        // A sibling item at the same depth closes the previous item and opens a new one.
        if depth > 0 && !just_opened(out) {
            let item = pad(base + 2 * (open - 1) + 1, origin);
            let _ = writeln!(out, "{item}</text:list-item>");
            let _ = writeln!(out, "{item}<text:list-item>");
        }

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
        let _ = writeln!(out, "{}</text:list-item>", pad(base + 2 * open + 1, origin));
        let _ = writeln!(out, "{}</text:list>", pad(base + 2 * open, origin));
    }
}

/// Whether the last thing written opened a list item, so the next block belongs *inside* it
/// rather than after it.
fn just_opened(out: &str) -> bool {
    out.trim_end().ends_with("<text:list-item>")
}

/// `depth + 1` spaces — moved to wherever the file being regenerated indents its body, so a new
/// paragraph in a LibreOffice document sits at LibreOffice's depth and an Enter is two lines of
/// diff rather than every line of the body.
fn pad(depth: u32, origin: Option<&Origin>) -> String {
    let shift = origin.map_or(0, |origin| origin.shift);
    " ".repeat((depth as isize + 1 + shift).max(0) as usize)
}

/// One `text:p` or `text:h`.
///
/// `keep` is the original element's unmanaged attributes when this is a splice — everything
/// the file said that the model does not carry (`text:class-names`, `xml:id`, a vendor's own),
/// put back verbatim so that replacing an element does not quietly drop half of it. Empty when
/// regenerating, because then there is no original to keep anything from.
fn paragraph(out: &mut String, block: &Block, indent: String, keep: &str, pool: &Pool) {
    let (tag, extra) = match block.kind {
        BlockKind::Heading { level } => ("text:h", format!(" text:outline-level=\"{level}\"")),
        _ => ("text:p", String::new()),
    };
    let style = match &block.style {
        Some(name) => format!(" text:style-name=\"{}\"", esc(name)),
        None => String::new(),
    };
    // An empty paragraph is a real thing — it is how a document spaces itself — and is written
    // self-closed rather than skipped.
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
            // Three nested wrappers, outermost first: the link, the document's own style name,
            // and this build's generated one for the direct formatting. Reading composed the
            // style names into one string, so writing emits one span for them — see
            // `doc/text-core.md`'s flattening decision, and what it costs.
            //
            // The generated span goes *inside* the named one rather than inheriting from it,
            // which is what keeps a round trip from composing a name into itself
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
        Run::Image {
            mime,
            data,
            width,
            height,
            anchor,
        } => image(
            out,
            mime,
            data,
            width.as_deref(),
            height.as_deref(),
            anchor.as_deref(),
        ),
    }
}

/// One `draw:frame` holding a `draw:image` — always the flat shape (no `draw:text-box`
/// wrapper), regardless of what a document this build read might have nested it in. R3's rule
/// applied to a new element: minimal boilerplate over reproducing a producer's own habits, and
/// R6 means this only fires for an image a person actually inserted or a paragraph a person
/// actually edited — everything else splices its source bytes back verbatim, wrapper and all.
///
/// **The anchor is the document's own**, and that is not boilerplate: a frame written without
/// `text:anchor-type` takes ODF's default of `paragraph`, which moves an `as-char` picture from
/// where it was typed to the top of its paragraph. Loop C caught exactly that, in a corpus
/// document whose image sat at the end of a list item and came back at the front. `paragraph`
/// is still the default for an image this build inserts, which has no anchor of its own.
fn image(
    out: &mut String,
    mime: &str,
    data: &[u8],
    width: Option<&str>,
    height: Option<&str>,
    anchor: Option<&str>,
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
    out.push_str("</office:binary-data></draw:image></draw:frame>");
}

/// Character data, with every piece of significant whitespace written as the element ODF has
/// for it.
///
/// **The whole reason this function exists**: XML character data is whitespace, and an ODF
/// consumer collapses a run of it to one space. So `a    b`, a tab and a newline written
/// literally all read back as a single space, and the text the user typed is gone. ODF's
/// answers are `text:s` with a count (rng:8408), `text:tab` and `text:line-break`, and this is
/// where a paragraph's text is translated into them.
///
/// The convention every implementation follows for spaces is that the *first* of a run stays
/// literal and the rest become the element, which keeps ordinary prose — where runs are one
/// space long — entirely free of markup. A space with no character data in front of it inside
/// this run has nothing to anchor it, so the whole run is encoded instead; that covers a
/// leading space and a space following a `text:tab`, both of which a reader would otherwise
/// trim.
///
/// `\r` and `\r\n` both become one `text:line-break`, and so read back as `\n`. That is not
/// this writer choosing: XML line-ending normalisation says a parser hands `\n` back for either
/// (XML 1.0 §2.11), so writing anything else would only be a lie about what a reader will see.
///
/// Loop C is what turned the tab and the line break from theory into code — the model has had
/// [`Run::Tab`] and [`Run::Break`] since S4, but a tab *character* inside a [`Run::Text`] was
/// written literally, and LibreOffice handed it back as a space.
fn characters(out: &mut String, text: &str) {
    // Whether literal character data has been written since the last element. A space needs
    // something in front of it to survive; markup does not count.
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
