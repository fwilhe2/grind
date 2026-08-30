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
use grind_core::odf::names::{DRAW, FO, OFFICE, STYLE, SVG, TEXT, XLINK};
use grind_core::odf::package::{VERSION, write_package};
use grind_core::odf::xml::esc;

use crate::model::{Block, BlockKind, Document, Run};
use crate::style::CharStyle;

pub use grind_core::Form;

/// The media type, byte for byte. Sniffed by readers at a fixed offset in the package form
/// (§1.1), so it is not somewhere to be creative.
pub const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

pub fn write(doc: &Document, form: Form) -> Result<Vec<u8>> {
    // R6 first: a document that came from a file and has only had block *contents* edited goes
    // back as that file with those elements replaced. Everything else regenerates, which is
    // always correct and is what this did before splicing existed.
    if let Some(spliced) = splice(doc, form) {
        return Ok(spliced);
    }
    match form {
        Form::Flat => Ok(content(doc, form).into_bytes()),
        Form::Package => write_package(MIMETYPE, &content(doc, Form::Package)),
        // The third form is not XML at all, so it leaves before any of this file runs
        // (`doc/dsl.md` §9, D2). It is here rather than one layer up because `write_bytes` is
        // the one door out of the crate, and a form that only *some* callers knew to handle
        // would be a form that escapes through the others.
        Form::Projection => Ok(crate::projection::project(doc).into_text().into_bytes()),
    }
}

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
}

impl Used {
    fn of(doc: &Document, pool: &Pool) -> Self {
        let runs = || doc.blocks.iter().flat_map(|b| b.runs.iter());
        Used {
            xlink: runs().any(|r| matches!(r, Run::Text { href: Some(_), .. })),
            styles: !pool.is_empty(),
            image: runs().any(|r| matches!(r, Run::Image { .. })),
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
}

impl Pool {
    /// Every distinct formatting in the document, named `T1`, `T2`, … in the order it first
    /// appears — so that saving one document twice produces the same bytes.
    fn of(doc: &Document) -> Self {
        let mut pool = Pool::default();
        for props in props_of(doc) {
            if pool.name(props).is_none() {
                let name = format!("T{}", pool.entries.len() + 1);
                pool.entries.push((props.clone(), name));
            }
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
fn content(doc: &Document, form: Form) -> String {
    let root = match form {
        Form::Package => "office:document-content",
        // The projection never reaches here — `write` refuses it before there is any XML.
        Form::Flat | Form::Projection => "office:document",
    };
    let pool = Pool::of(doc);
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
    let _ = write!(out, " office:version=\"{VERSION}\"");
    if form == Form::Flat {
        let _ = write!(out, " office:mimetype=\"{MIMETYPE}\"");
    }
    out.push_str(">\n");
    automatic_styles(&mut out, &pool);
    out.push_str(" <office:body>\n  <office:text>\n");
    body(&mut out, doc, &pool);
    out.push_str("  </office:text>\n </office:body>\n");
    let _ = writeln!(out, "</{root}>");
    out
}

/// The `office:automatic-styles` block — one `style:style` per distinct run formatting.
///
/// Ahead of `office:body`, which the schema requires and a single-pass reader depends on: a
/// span refers to a name, and the name has to be declared by the time it does.
fn automatic_styles(out: &mut String, pool: &Pool) {
    if pool.is_empty() {
        return;
    }
    out.push_str(" <office:automatic-styles>\n");
    for (props, name) in &pool.entries {
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

/// The blocks, with `text:list` nesting folded back in from their depths.
fn body(out: &mut String, doc: &Document, pool: &Pool) {
    // How many `text:list` elements are currently open. The model is flat and the file is
    // not, so this counter *is* the reconstruction: a depth rise opens elements, a fall closes
    // them, and the end of the document closes whatever is left.
    let mut open = 0u32;

    for block in &doc.blocks {
        let depth = match block.kind {
            BlockKind::ListItem { depth } => depth,
            _ => 0,
        };

        // Close deeper lists, then open shallower ones, so a jump of two levels is two
        // elements rather than a malformed one.
        while open > depth {
            open -= 1;
            let _ = writeln!(out, "{}</text:list-item>", indent(open + 2));
            let _ = writeln!(out, "{}</text:list>", indent(open + 1));
        }
        while open < depth {
            let _ = writeln!(out, "{}<text:list>", indent(open + 1));
            let _ = writeln!(out, "{}<text:list-item>", indent(open + 2));
            open += 1;
        }
        // A sibling item at the same depth closes the previous item and opens a new one.
        if depth > 0 && !just_opened(out) {
            let _ = writeln!(out, "{}</text:list-item>", indent(open + 2));
            let _ = writeln!(out, "{}<text:list-item>", indent(open + 2));
        }

        paragraph(out, block, indent(open + 3), "", pool);
    }

    while open > 0 {
        open -= 1;
        let _ = writeln!(out, "{}</text:list-item>", indent(open + 2));
        let _ = writeln!(out, "{}</text:list>", indent(open + 1));
    }
}

/// Whether the last thing written opened a list item, so the next block belongs *inside* it
/// rather than after it.
fn just_opened(out: &str) -> bool {
    out.trim_end().ends_with("<text:list-item>")
}

fn indent(depth: u32) -> String {
    " ".repeat(depth as usize + 1)
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
        } => image(out, mime, data, width.as_deref(), height.as_deref()),
    }
}

/// One `draw:frame` holding a `draw:image` — always the flat shape (no `draw:text-box`
/// wrapper, `text:anchor-type="paragraph"` always), regardless of what a document this build
/// read might have nested it in. R3's rule applied to a new element: minimal boilerplate over
/// reproducing a producer's own habits, and R6 means this only fires for an image a person
/// actually inserted or a paragraph a person actually edited — everything else splices its
/// source bytes back verbatim, wrapper and all.
fn image(out: &mut String, mime: &str, data: &[u8], width: Option<&str>, height: Option<&str>) {
    use base64::Engine as _;
    let _ = write!(out, "<draw:frame text:anchor-type=\"paragraph\"");
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
