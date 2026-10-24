// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The element-context stack. **\[GENERIC\]**
//!
//! A streaming parser driving a stack of per-element contexts, dispatched by resolved `(namespace,
//! local-name)` (`doc/ods-format.md` §8). A context that does not recognise a child returns
//! `None`, and the driver pushes [`Ignore`] for that element and its whole subtree, so unknown
//! elements, attributes and vendor namespaces are inert by construction. `text` defaults to a
//! no-op as well, so the whitespace between elements is discarded without a special pass.

use std::io::BufRead;

use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;

use super::names::{Name, Ns};
use crate::{Error, Result};

/// Resolved attributes of one element.
///
/// ponytail: owned strings per element rather than borrowed from the parse buffer — an allocation
/// per attribute, in exchange for no lifetimes through every context. Revisit only if a profile
/// blames it.
#[derive(Debug, Default)]
pub struct Attrs {
    items: Vec<(Name, String)>,
    span: std::ops::Range<usize>,
}

impl Attrs {
    /// Where this element's start tag sits in the bytes being parsed, so a writer can splice a
    /// cell back into its place (R6). For an `Event::Empty` this is the whole element; for an
    /// `Event::Start`, the start tag only.
    pub fn span(&self) -> std::ops::Range<usize> {
        self.span.clone()
    }

    /// Every attribute's name, in document order — for a reader asking whether an element says
    /// anything beyond what it reads.
    pub fn names(&self) -> impl Iterator<Item = &Name> {
        self.items.iter().map(|(name, _)| name)
    }

    pub fn get(&self, ns: Ns, local: &str) -> Option<&str> {
        self.items
            .iter()
            .find(|(n, _)| n.is(ns, local))
            .map(|(_, v)| v.as_str())
    }

    /// First of several accepted spellings of the same idea — a value type is written both
    /// `office:value-type` and `calcext:value-type` (§9).
    pub fn get_any(&self, candidates: &[(Ns, &str)]) -> Option<&str> {
        candidates
            .iter()
            .find_map(|(ns, local)| self.get(*ns, local))
    }

    /// A count attribute such as `table:number-columns-repeated`. Missing or unparseable is 1, and
    /// anything over `limit` is clamped: a file claiming 4 000 000 000 repeats would otherwise
    /// exhaust memory (§9).
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

fn collect_attrs<R: BufRead>(
    reader: &NsReader<R>,
    e: &BytesStart,
    span: std::ops::Range<usize>,
) -> Result<Attrs> {
    let mut items = Vec::new();
    for attr in e.attributes() {
        // A malformed attribute is not worth failing a document over; skip it.
        let Ok(attr) = attr else { continue };
        let (rr, local) = reader.resolver().resolve_attribute(attr.key);
        let ns = match rr {
            ResolveResult::Bound(n) => Ns::from_uri(n.as_ref()),
            // Unprefixed attributes are in no namespace at all — not the default one.
            _ => Ns::Other,
        };
        // XML attribute-value normalization (§3.3.3): entity references resolved, literal
        // tabs and newlines folded to spaces. ODF documents carry no XML declaration saying
        // 1.1, so 1.0's rules are the ones that apply.
        let Ok(value) = attr.normalized_value(XmlVersion::Implicit1_0) else {
            continue;
        };
        items.push((Name::new(ns, local.as_ref()), value.into_owned()));
    }
    Ok(Attrs { items, span })
}

fn resolved_name<R: BufRead>(reader: &NsReader<R>, e: &BytesStart) -> Name {
    let (rr, local) = reader.resolver().resolve_element(e.name());
    let ns = match rr {
        ResolveResult::Bound(n) => Ns::from_uri(n.as_ref()),
        _ => Ns::Other,
    };
    Name::new(ns, local.as_ref())
}

/// Replace every byte that is not part of valid UTF-8 with `?`, one byte for one byte, so a
/// document with a stray byte (a fuzzer's corpus file, `ofz64109-1.fodt`) still loads.
/// Length-preserving because [`Attrs::span`] indexes into the original bytes; `U+FFFD` is three
/// bytes.
fn repair_utf8(bytes: &mut [u8]) {
    let mut at = 0;
    while let Err(e) = std::str::from_utf8(&bytes[at..]) {
        let start = at + e.valid_up_to();
        // `None` means the input ends mid-sequence: everything left is unusable.
        let len = e.error_len().unwrap_or(bytes.len() - start);
        bytes[start..start + len].fill(b'?');
        at = start + len;
    }
}

/// Drive `root` over the XML in `input`, mutating `sink`.
pub fn parse<R: BufRead, S>(mut input: R, root: Box<dyn Context<S>>, sink: &mut S) -> Result<()> {
    // Read it all up front rather than streaming: every caller already holds the whole part in
    // memory (a `content.xml` out of a zip, or a flat document's bytes), and [`repair_utf8`]
    // has to see a whole sequence at a time to know whether it is broken.
    let mut input_bytes = Vec::new();
    input.read_to_end(&mut input_bytes)?;
    repair_utf8(&mut input_bytes);

    let mut reader = NsReader::from_reader(input_bytes.as_slice());
    let config = reader.config_mut();
    config.trim_text(false);
    // Entity expansion is off by default in quick-xml, so the billion-laughs class of
    // attack does not apply. Depth is bounded below.

    let mut stack: Vec<Box<dyn Context<S>>> = vec![root];
    let mut buf = Vec::new();
    // Positions as indices into `input_bytes`, a byte-order mark included (`xml::bom_len`).
    let shift = super::xml::bom_len(&input_bytes);

    loop {
        // Where this event begins, for [`Attrs::span`]. `buffer_position` after a read points
        // just past the event, so the previous one's end is this one's start — and because
        // `trim_text` is off, the whitespace between two elements arrives as its own `Text`
        // event rather than being folded into the next element's span.
        let from = reader.buffer_position() as usize + shift;
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| Error::Xml(e.to_string()))?;
        let span = from..reader.buffer_position() as usize + shift;

        match event {
            Event::Start(e) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(Error::Xml(format!(
                        "element nesting deeper than {MAX_DEPTH}"
                    )));
                }
                let name = resolved_name(&reader, &e);
                let attrs = collect_attrs(&reader, &e, span)?;
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
                let attrs = collect_attrs(&reader, &e, span)?;
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
                // Line endings are normalized (XML 1.0 §2.11) — a document written on
                // Windows must not put a stray `\r` into a cell. Entity references are not
                // in here at all: the parser hands each one over as its own `GeneralRef`.
                let text = t.xml10_content();
                if !text.is_empty() {
                    stack
                        .last_mut()
                        .expect("stack never empties")
                        .text(&text, sink);
                }
            }
            // `&amp;`, `&#10;` — each a character of the content, handed to the same `text`
            // callback. An entity this parser cannot resolve is dropped rather than fatal (§9).
            Event::GeneralRef(r) => {
                let resolved = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(c.to_string()),
                    Ok(None) => quick_xml::escape::resolve_predefined_entity(&r).map(str::to_owned),
                    Err(_) => None,
                };
                if let Some(text) = resolved {
                    stack
                        .last_mut()
                        .expect("stack never empties")
                        .text(&text, sink);
                }
            }
            Event::CData(t) => {
                // CDATA is verbatim by definition: no entity references inside it.
                stack
                    .last_mut()
                    .expect("stack never empties")
                    .text(&t.into_inner(), sink);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The contexts below use a made-up `x:` vocabulary rather than a real ODF one, and
    /// deliberately: nothing in this module knows a document type (R8).
    #[derive(Default)]
    struct Collect;

    impl Context<String> for Collect {
        fn start_child(
            &mut self,
            _name: &Name,
            _attrs: &Attrs,
            _sink: &mut String,
        ) -> Option<Box<dyn Context<String>>> {
            Some(Box::new(Collect))
        }

        fn text(&mut self, text: &str, sink: &mut String) {
            sink.push_str(text);
        }
    }

    fn text_of(xml: &str) -> String {
        let mut out = String::new();
        parse(xml.as_bytes(), Box::new(Collect), &mut out).expect("parses");
        out
    }

    /// An entity reference is a character of the document, and arrives from the parser as its
    /// own event: the run it sits between must not lose it.
    #[test]
    fn an_entity_reference_is_content_and_not_markup() {
        assert_eq!(text_of("<x:p>a&amp;b</x:p>"), "a&b");
        assert_eq!(text_of("<x:p>&quot;𧌒&quot;</x:p>"), "\"𧌒\"");
        assert_eq!(text_of("<x:p>a&#10;b&#x41;</x:p>"), "a\nbA");
        // Undefined without a DTD, and §9 says the document still loads without it.
        assert_eq!(text_of("<x:p>a&nosuch;b</x:p>"), "ab");
    }

    #[test]
    fn a_stray_byte_costs_that_byte_and_not_the_document() {
        let mut bytes = b"<x:p>a\xffb</x:p>".to_vec();
        let mut out = String::new();
        parse(bytes.as_slice(), Box::new(Collect), &mut out).expect("parses");
        assert_eq!(out, "a?b");

        // Length-preserving, byte for byte — R6's spans index the caller's original bytes.
        let before = bytes.len();
        repair_utf8(&mut bytes);
        assert_eq!(bytes.len(), before);
        // A truncated multi-byte sequence at the very end goes the same way.
        let mut cut = "é".as_bytes()[..1].to_vec();
        repair_utf8(&mut cut);
        assert_eq!(cut, b"?");
    }
}
