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
//! * **Spaces are re-encoded as `text:s`.** XML collapses runs of whitespace, so a paragraph
//!   written with its spaces literal comes back with them gone. `doc/odt-format.md` §3.3 calls
//!   this the `table:number-columns-repeated` trap in a new costume, and it is: the reader
//!   expands, the writer re-encodes, and neither half is optional.
//! * **A list is reconstructed from block depths.** The model flattens `text:list` into the
//!   block sequence (`crate::model::BlockKind`), so writing folds the depths back into nesting
//!   — opening an element where the depth rises and closing where it falls.

use std::fmt::Write as _;

use grind_core::Result;
use grind_core::odf::names::{OFFICE, TEXT, XLINK};
use grind_core::odf::package::{VERSION, write_package};
use grind_core::odf::xml::esc;

use crate::model::{Block, BlockKind, Document, Run};

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
        paragraph(&mut out, block, String::new(), &at.keep);
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
}

impl Used {
    fn of(doc: &Document) -> Self {
        Used {
            xlink: doc.blocks.iter().any(|b| {
                b.runs
                    .iter()
                    .any(|r| matches!(r, Run::Text { href: Some(_), .. }))
            }),
        }
    }
}

/// The `content.xml` payload, which in the flat form is the whole document.
fn content(doc: &Document, form: Form) -> String {
    let root = match form {
        Form::Flat => "office:document",
        Form::Package => "office:document-content",
    };
    let used = Used::of(doc);

    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = write!(
        out,
        "<{root} xmlns:office=\"{OFFICE}\" xmlns:text=\"{TEXT}\""
    );
    // `xlink:` appears only in a document that links to something.
    if used.xlink {
        let _ = write!(out, " xmlns:xlink=\"{XLINK}\"");
    }
    let _ = write!(out, " office:version=\"{VERSION}\"");
    if form == Form::Flat {
        let _ = write!(out, " office:mimetype=\"{MIMETYPE}\"");
    }
    out.push_str(">\n");
    out.push_str(" <office:body>\n  <office:text>\n");
    body(&mut out, doc);
    out.push_str("  </office:text>\n </office:body>\n");
    let _ = writeln!(out, "</{root}>");
    out
}

/// The blocks, with `text:list` nesting folded back in from their depths.
fn body(out: &mut String, doc: &Document) {
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

        paragraph(out, block, indent(open + 3), "");
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
fn paragraph(out: &mut String, block: &Block, indent: String, keep: &str) {
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
        self::run(out, run);
    }
    let _ = writeln!(out, "</{tag}>");
}

fn run(out: &mut String, run: &Run) {
    match run {
        Run::Text { text, style, href } => {
            // A hyperlink wraps its text; a span wraps it inside that. Reading composed the
            // style names into one string, so writing emits one span — see
            // `doc/text-core.md`'s flattening decision, and what it costs.
            if let Some(href) = href {
                let _ = write!(out, "<text:a xlink:href=\"{}\">", esc(href));
            }
            if let Some(style) = style {
                let _ = write!(out, "<text:span text:style-name=\"{}\">", esc(style));
            }
            spaced(out, text);
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
    }
}

/// Text with its space runs encoded as `text:s`.
///
/// **The whole reason this function exists**: XML collapses whitespace, so `a    b` written
/// literally reads back as `a b`. ODF's answer is `text:s` with a count (rng:8408), and the
/// convention every implementation follows is that the *first* space of a run stays literal
/// and the rest become the element — which keeps ordinary prose, where runs are one space
/// long, entirely free of markup.
///
/// A leading space has no first space to keep, so the whole run is encoded; otherwise a reader
/// that trims leading whitespace would eat it.
fn spaced(out: &mut String, text: &str) {
    let mut rest = text;
    let mut at_start = true;
    while let Some(i) = rest.find(' ') {
        let (before, tail) = rest.split_at(i);
        out.push_str(&esc(before));
        let spaces = tail.chars().take_while(|c| *c == ' ').count();

        // Keep one literal space unless we are at the very start of the run of text, where
        // there is nothing in front of it to anchor it.
        let literal = usize::from(!(at_start && before.is_empty()));
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
        rest = &tail[spaces..];
        at_start = false;
    }
    out.push_str(&esc(rest));
}
