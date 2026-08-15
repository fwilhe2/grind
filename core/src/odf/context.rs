// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The element-context stack. **[GENERIC]**
//!
//! A streaming parser driving a stack of per-element context objects, dispatched by
//! resolved `(namespace, local-name)` (doc/ods-format.md §8). This is the whole reason the
//! reader tolerates messy documents, and it is a property of the shape rather than a
//! feature bolted on:
//!
//! **A context that does not recognise a child returns `None`, and the driver pushes
//! [`Ignore`] — whose callbacks all do nothing — for that element and everything beneath
//! it.** Unknown elements, unknown attributes, whole foreign vendor namespaces and
//! newer-ODF features are therefore inert by construction. Nothing has to detect junk.
//!
//! The second half falls out the same way: `text` defaults to a no-op, so the indentation
//! whitespace between elements that every pretty-printer emits is discarded without a
//! special pass. Only contexts that deliberately override `text` — paragraphs — collect it.
//!
//! Nothing here is spreadsheet-specific; §10 reuses it unchanged for text documents.

use std::io::BufRead;

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::NsReader;

use super::names::{Name, Ns};
use crate::{Error, Result};

/// Resolved attributes of one element.
///
/// ponytail: collected into a `Vec` of owned strings per element rather than borrowed from
/// the parse buffer. Costs an allocation per attribute; buys freedom from lifetime
/// plumbing through every context. Revisit only if a profile blames it.
#[derive(Debug, Default)]
pub struct Attrs {
    items: Vec<(Name, String)>,
}

impl Attrs {
    pub fn get(&self, ns: Ns, local: &str) -> Option<&str> {
        self.items
            .iter()
            .find(|(n, _)| n.is(ns, local))
            .map(|(_, v)| v.as_str())
    }

    /// First of several accepted spellings of the same idea.
    ///
    /// Real ecosystems split on a namespace for one concept — cell value-type is written
    /// both `office:value-type` and `calcext:value-type`, and LibreOffice handles both with
    /// the same code (§9). Resolving by meaning across a small alias set beats rejecting
    /// the document.
    pub fn get_any(&self, candidates: &[(Ns, &str)]) -> Option<&str> {
        candidates.iter().find_map(|(ns, local)| self.get(*ns, local))
    }

    /// A count attribute, e.g. `table:number-columns-repeated`.
    ///
    /// Never trust the file's number (§9): missing or unparseable means 1, and anything
    /// larger than `limit` is clamped. A document claiming 4 000 000 000 repeats is
    /// otherwise a one-line memory-exhaustion vector.
    pub fn count(&self, ns: Ns, local: &str, limit: u32) -> u32 {
        self.get(ns, local)
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(1)
            .clamp(1, i64::from(limit)) as u32
    }
}

/// One element's handler. All callbacks default to doing nothing, which is what makes
/// unrecognised input inert.
pub trait Context<S> {
    /// Return `Some(child)` to handle a recognised child element. Returning `None` makes
    /// the driver install [`Ignore`], swallowing that element's entire subtree.
    fn start_child(
        &mut self,
        _name: &Name,
        _attrs: &Attrs,
        _sink: &mut S,
    ) -> Option<Box<dyn Context<S>>> {
        None
    }

    /// Character data. Default no-op — this is why indentation is not cell content.
    fn text(&mut self, _text: &str, _sink: &mut S) {}

    fn end(&mut self, _sink: &mut S) {}
}

/// The handler for everything we do not recognise. Deliberately empty.
pub struct Ignore;

impl<S> Context<S> for Ignore {}

/// How deep the element stack may go before we call the document malformed. Guards against
/// a crafted file nesting until the box allocations exhaust memory.
const MAX_DEPTH: usize = 256;

fn collect_attrs<R: BufRead>(reader: &NsReader<R>, e: &BytesStart) -> Result<Attrs> {
    let mut items = Vec::new();
    for attr in e.attributes() {
        // A malformed attribute is not worth failing a document over; skip it.
        let Ok(attr) = attr else { continue };
        let (rr, local) = reader.resolve_attribute(attr.key);
        let ns = match rr {
            ResolveResult::Bound(n) => Ns::from_uri(n.as_ref()),
            // Unprefixed attributes are in no namespace at all — not the default one.
            _ => Ns::Other,
        };
        let Ok(value) = attr.decode_and_unescape_value(reader.decoder()) else {
            continue;
        };
        items.push((
            Name::new(ns, String::from_utf8_lossy(local.as_ref()).into_owned()),
            value.into_owned(),
        ));
    }
    Ok(Attrs { items })
}

fn resolved_name<R: BufRead>(reader: &NsReader<R>, e: &BytesStart) -> Name {
    let (rr, local) = reader.resolve_element(e.name());
    let ns = match rr {
        ResolveResult::Bound(n) => Ns::from_uri(n.as_ref()),
        _ => Ns::Other,
    };
    Name::new(ns, String::from_utf8_lossy(local.as_ref()).into_owned())
}

/// Drive `root` over the XML in `input`, mutating `sink`.
pub fn parse<R: BufRead, S>(input: R, root: Box<dyn Context<S>>, sink: &mut S) -> Result<()> {
    let mut reader = NsReader::from_reader(input);
    let config = reader.config_mut();
    config.trim_text(false);
    // Entity expansion is off by default in quick-xml, so the billion-laughs class of
    // attack does not apply. Depth is bounded below.

    let mut stack: Vec<Box<dyn Context<S>>> = vec![root];
    let mut buf = Vec::new();

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| Error::Xml(e.to_string()))?;

        match event {
            Event::Start(e) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(Error::Xml(format!("element nesting deeper than {MAX_DEPTH}")));
                }
                let name = resolved_name(&reader, &e);
                let attrs = collect_attrs(&reader, &e)?;
                let child = stack
                    .last_mut()
                    .expect("stack never empties")
                    .start_child(&name, &attrs, sink)
                    .unwrap_or_else(|| Box::new(Ignore));
                stack.push(child);
            }
            // `<table:table-cell/>` — start and end in one event, and by far the most
            // common element in a sheet.
            Event::Empty(e) => {
                let name = resolved_name(&reader, &e);
                let attrs = collect_attrs(&reader, &e)?;
                if let Some(mut child) = stack
                    .last_mut()
                    .expect("stack never empties")
                    .start_child(&name, &attrs, sink)
                {
                    child.end(sink);
                }
            }
            Event::End(_) => {
                // Guard the root: a stray close tag must not empty the stack.
                if stack.len() > 1 {
                    let mut done = stack.pop().expect("checked len");
                    done.end(sink);
                }
            }
            Event::Text(t) => {
                // Entities (`&amp;`, `&#10;`) are the document's, not ours — resolve them
                // here so contexts only ever see real characters.
                let Ok(text) = t.unescape() else { continue };
                if !text.is_empty() {
                    stack
                        .last_mut()
                        .expect("stack never empties")
                        .text(&text, sink);
                }
            }
            Event::CData(t) => {
                if let Ok(text) = std::str::from_utf8(&t) {
                    stack
                        .last_mut()
                        .expect("stack never empties")
                        .text(text, sink);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    // Close anything the document left open rather than discarding its content.
    while stack.len() > 1 {
        let mut done = stack.pop().expect("checked len");
        done.end(sink);
    }
    if let Some(mut root) = stack.pop() {
        root.end(sink);
    }
    Ok(())
}
