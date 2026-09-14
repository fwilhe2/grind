// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tolerant element walker, and the one place markup compatibility is handled.
//!
//! This is `core/src/odf/context.rs`'s property rebuilt rather than borrowed: **a visitor
//! that does not recognise a child returns [`Handled::No`], and the walker consumes that
//! element and everything beneath it.** Unknown elements, whole vendor namespaces, Strict
//! spellings of things we read and every future version of Excel are therefore inert by
//! construction rather than by a match arm each. Measured justification: LibreOffice's own
//! minimal `.xlsx` output declares seven namespaces it makes no use of
//! (`doc/xlsx-format.md` §1.1).
//!
//! **Why rebuilt.** `context.rs` dispatches on `grind_core::odf::names::Ns`, a closed enum of
//! *ODF* namespace URIs, and R8 keeps it that way — OOXML's namespaces are no more generic
//! than ODF's. The shape is cheaper here anyway: SpreadsheetML is shallow and regular where
//! ODF's style tree is deep, so a visitor per element beats a context object per element.
//!
//! Two deliberate differences from the ODF reader, both consequences of what an import *is*:
//!
//! 1. **No spans.** `context.rs` records where each element began so that R6 can splice an
//!    edit back into the original bytes. An imported document has no original bytes to
//!    splice — it regenerates, because an import is authoring a document — so there is
//!    nothing to record.
//! 2. **Lossy decoding rather than byte-preserving repair.** For the same reason: with no
//!    spans to keep aligned, a stray byte can become `U+FFFD` instead of having to be
//!    replaced one-byte-for-one-byte. `repair_utf8`'s constraint does not apply here.
//!
//! Entity expansion needs no defence: quick-xml does not resolve external entities and
//! rejects unknown internal ones, so the billion-laughs class cannot arise. Depth is bounded
//! by [`MAX_DEPTH`] and a document deeper than that is refused rather than recursed into.

use std::collections::BTreeSet;

use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;

use crate::mce::{self, Alternative};
use crate::names::{self, Ns, Seen};
use crate::{Error, Result};

/// Deeper than any spreadsheet and shallower than a stack overflow. SpreadsheetML's own
/// deepest legitimate path is well under twenty.
///
/// Public because it is a property of what this reader will accept rather than an
/// implementation detail: a caller comparing us against another parser wants the number.
pub const MAX_DEPTH: usize = 256;

/// A resolved element or attribute name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub ns: Ns,
    pub local: String,
}

impl Name {
    /// Is this `<local>` in the SpreadsheetML namespace, in either flavour?
    pub fn is(&self, local: &str) -> bool {
        self.ns == Ns::Spreadsheet && self.local == local
    }
}

/// One element's resolved attributes.
///
/// ponytail: owned strings, collected per element, as in `context.rs` and for the same
/// reason — it costs an allocation per attribute and buys freedom from lifetime plumbing
/// through every visitor. Revisit only if a profile blames it.
#[derive(Debug, Default)]
pub struct Attrs {
    items: Vec<(Ns, String, String)>,
}

impl Attrs {
    /// An attribute in **no namespace**, which is what almost every SpreadsheetML attribute
    /// is: `<sheet name="Budget" sheetId="1">` has two of them.
    pub fn plain(&self, local: &str) -> Option<&str> {
        self.get(Ns::None, local)
    }

    /// An attribute in a namespace — `r:id`, `mc:Ignorable`.
    pub fn get(&self, ns: Ns, local: &str) -> Option<&str> {
        self.items
            .iter()
            .find(|(n, l, _)| *n == ns && l == local)
            .map(|(_, _, v)| v.as_str())
    }

    /// `"1"`, `"true"` and `"on"` are all true; everything else, including absence, is false.
    /// ECMA-376 §18.18.2 (`ST_Boolean`), which is XSD's boolean plus Excel's spelling of it.
    pub fn flag(&self, local: &str) -> bool {
        matches!(self.plain(local), Some("1" | "true" | "on"))
    }
}

/// What a visitor did with a child element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    /// Recognised. If the visitor did not consume the element's children itself, the walker
    /// does — either way the walker is back where it started before the next sibling.
    Yes,
    /// Not recognised. The element and its whole subtree are skipped. **This is the
    /// tolerance property**, and it is the default answer rather than an error.
    No,
}

/// The visitor, as a trait object.
///
/// `&mut dyn` rather than a generic parameter on purpose: [`Reader::walk`] recurses (an
/// `mc:AlternateContent` hands its fallback's children to the *same* visitor), and a generic
/// would monomorphise into `walk::<&mut F>`, `walk::<&mut &mut F>`, … without end.
type Visitor<'a, 'v> = &'v mut dyn FnMut(&mut Reader<'a>, &Name, &Attrs) -> Result<Handled>;

/// One parse event, after namespace resolution.
enum Ev {
    Start(Name, Attrs),
    Empty(Name, Attrs),
    Text(String),
    End,
    /// A declaration, comment, processing instruction or doctype. Never interesting.
    Other,
    Eof,
}

pub struct Reader<'a> {
    inner: NsReader<&'a [u8]>,
    buf: Vec<u8>,
    /// Open elements. The walker's whole bookkeeping is this number.
    depth: usize,
    /// Which flavour's URIs have been seen, for the report.
    pub seen: Seen,
    /// Namespace prefixes the file said we must understand (`mc:MustUnderstand`).
    pub must_understand: BTreeSet<String>,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        let mut inner = NsReader::from_reader(bytes);
        // Off, so that `xml:space="preserve"` is honoured by default rather than by
        // detection: 51 of the corpus's 360 workbooks have a shared string that needs it
        // (`doc/xlsx-format.md` §1.4), and a reader that trims corrupts them silently.
        inner.config_mut().trim_text(false);
        Self {
            inner,
            buf: Vec::new(),
            depth: 0,
            seen: Seen::default(),
            must_understand: BTreeSet::new(),
        }
    }

    /// The document element, or `None` for a part with no elements at all.
    pub fn root(&mut self) -> Result<Option<(Name, Attrs)>> {
        loop {
            match self.next()? {
                Ev::Start(name, attrs) | Ev::Empty(name, attrs) => return Ok(Some((name, attrs))),
                Ev::Eof => return Ok(None),
                _ => {}
            }
        }
    }

    /// Walk the children of the element that was just started, handing each to `visit`.
    ///
    /// A visitor may call this again for a child it recognises; one that does not is exactly
    /// what makes the subtree disappear.
    pub fn children<F>(&mut self, mut visit: F) -> Result<()>
    where
        F: FnMut(&mut Reader<'a>, &Name, &Attrs) -> Result<Handled>,
    {
        self.walk(&mut visit)
    }

    /// This element's string value — all text beneath it, concatenated.
    ///
    /// Right for the leaves that want it (`<t>`, `<v>`, `<f>`) and right for `<si>` holding
    /// rich-text `<r>` runs, which is how "runs flattened" falls out rather than being
    /// implemented. **Wrong for any element a producer might have pretty-printed**, since
    /// indentation between children is text too; no real producer indents inside
    /// `sharedStrings` or `sheetData`, and anything that might be indented is walked with
    /// [`Reader::children`] instead.
    pub fn text(&mut self) -> Result<String> {
        let base = self.depth;
        let mut out = String::new();
        loop {
            match self.next()? {
                Ev::Text(text) => out.push_str(&text),
                Ev::End if self.depth < base => return Ok(out),
                Ev::Eof => return Ok(out),
                _ => {}
            }
        }
    }

    /// Consume whatever is left of the current element, back to `depth`.
    fn skip_to(&mut self, depth: usize) -> Result<()> {
        while self.depth > depth {
            if matches!(self.next()?, Ev::Eof) {
                return Ok(());
            }
        }
        Ok(())
    }

    fn walk(&mut self, visit: Visitor<'a, '_>) -> Result<()> {
        let base = self.depth;
        loop {
            match self.next()? {
                Ev::Start(name, attrs) => {
                    if name.ns == Ns::Mce {
                        // The one place markup compatibility is handled. Doing it here rather
                        // than in each visitor is the whole design: `mc:AlternateContent` can
                        // wrap a sparkline group, a slicer, a data validation, a drawing or a
                        // `sheetPr`, so per-site handling is per-site bugs.
                        if mce::classify(&name.local) == Alternative::Wrapper {
                            self.alternate(&mut *visit)?;
                        }
                    } else {
                        visit(self, &name, &attrs)?;
                    }
                    // Whatever the visitor did or did not consume, the next sibling starts
                    // here. A visitor that returns `No` never moved, so this skips the whole
                    // subtree; one that walked the children itself makes this a no-op.
                    self.skip_to(base)?;
                }
                Ev::Empty(name, attrs) => {
                    if name.ns != Ns::Mce {
                        visit(self, &name, &attrs)?;
                    }
                }
                Ev::End if self.depth < base => return Ok(()),
                Ev::Eof => return Ok(()),
                _ => {}
            }
        }
    }

    /// `mc:AlternateContent`: take the first `mc:Choice` whose requirements we meet, and
    /// otherwise the `mc:Fallback`.
    ///
    /// We understand no extension namespace ([`mce::satisfies`]), so in practice the fallback
    /// is what gets read. The chosen branch's *children* go to the same visitor, so from
    /// where the caller sits the alternation is not there at all — which is what MCE is for.
    fn alternate(&mut self, visit: Visitor<'a, '_>) -> Result<()> {
        let base = self.depth;
        let mut taken = false;
        loop {
            match self.next()? {
                Ev::Start(name, attrs) => {
                    let take = !taken
                        && name.ns == Ns::Mce
                        && match mce::classify(&name.local) {
                            Alternative::Choice => {
                                mce::satisfies(attrs.plain("Requires").unwrap_or(""))
                            }
                            Alternative::Fallback => true,
                            _ => false,
                        };
                    if take {
                        taken = true;
                        self.walk(&mut *visit)?;
                    }
                    self.skip_to(base)?;
                }
                Ev::End if self.depth < base => return Ok(()),
                Ev::Eof => return Ok(()),
                _ => {}
            }
        }
    }

    fn next(&mut self) -> Result<Ev> {
        // Field-by-field, so that the event may borrow `buf` while `inner` stays reachable
        // for name resolution. The alternative is what `context.rs` does — keep the reader
        // and the buffer as separate locals — which a struct with a `next` method cannot.
        let Self {
            inner,
            buf,
            depth,
            seen,
            must_understand,
        } = self;
        buf.clear();
        let event = inner
            .read_event_into(&mut *buf)
            .map_err(|e| Error::Xml(e.to_string()))?;
        Ok(match event {
            Event::Start(e) => {
                *depth += 1;
                if *depth > MAX_DEPTH {
                    return Err(Error::Xml(format!(
                        "element nesting deeper than {MAX_DEPTH}"
                    )));
                }
                let name = resolved_name(inner, &e, seen);
                let attrs = collect_attrs(inner, &e, seen, must_understand);
                Ev::Start(name, attrs)
            }
            Event::Empty(e) => {
                let name = resolved_name(inner, &e, seen);
                let attrs = collect_attrs(inner, &e, seen, must_understand);
                Ev::Empty(name, attrs)
            }
            // `saturating_sub` guards the root: a stray close tag must not underflow the
            // depth and turn every later `skip_to` into a walk to the end of the file.
            Event::End(_) => {
                *depth = depth.saturating_sub(1);
                Ev::End
            }
            // Line endings normalised (XML 1.0 §2.11): a workbook written on Windows must
            // not put a stray `\r` into a cell.
            Event::Text(e) => Ev::Text(e.xml10_content().into_owned()),
            // `&amp;`, `&#10;` — one event each rather than part of the text, and a
            // *character* of the content rather than markup. An entity this parser cannot
            // resolve is dropped rather than fatal: without a DTD only the five predefined
            // ones and numeric references can appear, so anything else is a document naming
            // something that does not exist, and tolerance says it still loads. This is also
            // where billion-laughs would have to start, and it cannot: nothing here expands
            // a definition, because nothing here reads one.
            Event::GeneralRef(r) => Ev::Text(match r.resolve_char_ref() {
                Ok(Some(c)) => c.to_string(),
                Ok(None) => quick_xml::escape::resolve_predefined_entity(&r)
                    .unwrap_or_default()
                    .to_owned(),
                Err(_) => String::new(),
            }),
            // Verbatim by definition: no entity references inside it.
            Event::CData(e) => Ev::Text(e.into_inner().into_owned()),
            Event::Eof => Ev::Eof,
            _ => Ev::Other,
        })
    }
}

/// Record what a URI says about the file's flavour.
///
/// Public because a relationship *type* is Part 1 vocabulary too and votes the same way, and
/// those are attribute values read in `package.rs` rather than namespaces resolved here.
pub fn note_flavour(seen: &mut Seen, uri: &str) {
    match names::family(uri) {
        Some(names::Flavour::Transitional) => seen.transitional = true,
        Some(names::Flavour::Strict) => seen.strict = true,
        _ => {}
    }
}

fn resolved_name(reader: &NsReader<&[u8]>, e: &BytesStart, seen: &mut Seen) -> Name {
    let (rr, local) = reader.resolver().resolve_element(e.name());
    let ns = match rr {
        ResolveResult::Bound(uri) => {
            note_flavour(seen, uri.as_ref());
            Ns::from_uri(uri.as_ref())
        }
        _ => Ns::None,
    };
    Name {
        ns,
        local: local.as_ref().to_owned(),
    }
}

fn collect_attrs(
    reader: &NsReader<&[u8]>,
    e: &BytesStart,
    seen: &mut Seen,
    must_understand: &mut BTreeSet<String>,
) -> Attrs {
    let mut items = Vec::new();
    for attr in e.attributes() {
        // A malformed attribute is not worth failing a document over.
        let Ok(attr) = attr else { continue };
        let (rr, local) = reader.resolver().resolve_attribute(attr.key);
        let ns = match rr {
            ResolveResult::Bound(uri) => {
                note_flavour(seen, uri.as_ref());
                Ns::from_uri(uri.as_ref())
            }
            // An unprefixed attribute is in **no** namespace — not the default one. That is
            // XML's rule and not a shortcut: it is why `Attrs::plain` exists.
            _ => Ns::None,
        };
        let Ok(value) = attr.normalized_value(XmlVersion::Implicit1_0) else {
            continue;
        };
        let local = local.as_ref().to_owned();
        if ns == Ns::Mce && local == "MustUnderstand" {
            must_understand.extend(mce::must_understand(&value).map(str::to_owned));
        }
        items.push((ns, local, value.into_owned()));
    }
    Attrs { items }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect `(namespace, local)` for every child the visitor recognises, where
    /// "recognises" means SpreadsheetML — everything else is skipped, which is the point.
    fn sheet_children(xml: &str) -> Vec<String> {
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap().expect("a root element");
        let mut seen = Vec::new();
        reader
            .children(|_, name, _| {
                if name.ns == Ns::Spreadsheet {
                    seen.push(name.local.clone());
                    Ok(Handled::Yes)
                } else {
                    Ok(Handled::No)
                }
            })
            .unwrap();
        seen
    }

    const MAIN: &str = crate::names::MAIN_T;
    const MCE: &str = crate::names::MCE;

    #[test]
    fn an_unknown_element_takes_its_whole_subtree_with_it() {
        let xml = format!(
            r#"<root xmlns="{MAIN}" xmlns:x="http://example.invalid/x">
                 <a/>
                 <x:junk><b/><x:deeper><c/></x:deeper></x:junk>
                 <d/>
               </root>"#
        );
        // `b` and `c` are SpreadsheetML elements *inside* a foreign one and must not be
        // seen: tolerance is structural, so an ignored element hides everything beneath it.
        assert_eq!(sheet_children(&xml), ["a", "d"]);
    }

    #[test]
    fn a_visitor_that_says_no_does_not_desync_the_walk() {
        let xml = format!(r#"<root xmlns="{MAIN}"><a><deep><deeper/></deep></a><b/></root>"#);
        // The visitor recognises `a` and `b` but walks neither one's children; the walker
        // still arrives at `b`.
        assert_eq!(sheet_children(&xml), ["a", "b"]);
    }

    #[test]
    fn nesting_deeper_than_the_bound_is_refused_rather_than_recursed_into() {
        let xml = format!(
            "<root xmlns=\"{MAIN}\">{}{}</root>",
            "<a>".repeat(MAX_DEPTH + 10),
            "</a>".repeat(MAX_DEPTH + 10)
        );
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap();
        let err = reader.children(|r, _, _| {
            r.children(|_, _, _| Ok(Handled::No))?;
            Ok(Handled::Yes)
        });
        assert!(matches!(err, Err(Error::Xml(_))), "got {err:?}");
    }

    // ---- markup compatibility ----

    /// The fixture `doc/xlsx-import.md`'s verification section asks for: the choice and the
    /// fallback hold **different** elements, so taking the wrong one is visible.
    #[test]
    fn the_fallback_is_read_and_the_choice_is_not() {
        let xml = format!(
            r#"<root xmlns="{MAIN}" xmlns:mc="{MCE}" xmlns:x14="http://example.invalid/x14">
                 <before/>
                 <mc:AlternateContent>
                   <mc:Choice Requires="x14"><fromchoice/></mc:Choice>
                   <mc:Fallback><fromfallback/></mc:Fallback>
                 </mc:AlternateContent>
                 <after/>
               </root>"#
        );
        assert_eq!(
            sheet_children(&xml),
            ["before", "fromfallback", "after"],
            "the fallback's children belong to the alternation's parent, and the choice is gone"
        );
    }

    #[test]
    fn an_alternation_with_no_fallback_contributes_nothing() {
        let xml = format!(
            r#"<root xmlns="{MAIN}" xmlns:mc="{MCE}">
                 <before/>
                 <mc:AlternateContent>
                   <mc:Choice Requires="x14"><fromchoice/></mc:Choice>
                 </mc:AlternateContent>
                 <after/>
               </root>"#
        );
        assert_eq!(sheet_children(&xml), ["before", "after"]);
    }

    /// Only the *first* acceptable branch is taken — never both, which is the failure a
    /// naive reader produces and the one that doubles whatever was inside.
    #[test]
    fn exactly_one_branch_is_taken() {
        let xml = format!(
            r#"<root xmlns="{MAIN}" xmlns:mc="{MCE}">
                 <mc:AlternateContent>
                   <mc:Choice Requires=""><first/></mc:Choice>
                   <mc:Choice Requires=""><second/></mc:Choice>
                   <mc:Fallback><third/></mc:Fallback>
                 </mc:AlternateContent>
               </root>"#
        );
        assert_eq!(sheet_children(&xml), ["first"]);
    }

    #[test]
    fn must_understand_is_collected_and_is_not_a_refusal() {
        let xml = format!(r#"<root xmlns="{MAIN}" xmlns:mc="{MCE}" mc:MustUnderstand="x14 xr"/>"#);
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap().expect("a root");
        assert_eq!(
            reader
                .must_understand
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["x14", "xr"]
        );
    }

    // ---- namespaces and flavour ----

    #[test]
    fn the_prefix_is_never_the_key() {
        // A default namespace on one element and an odd prefix on the other; both are the
        // same element as far as dispatch is concerned.
        let xml = format!(r#"<ns0:root xmlns:ns0="{MAIN}"><ns0:a/><b xmlns="{MAIN}"/></ns0:root>"#);
        assert_eq!(sheet_children(&xml), ["a", "b"]);
    }

    #[test]
    fn the_flavour_is_recorded_rather_than_branched_on() {
        let strict = format!(r#"<workbook xmlns="{}"/>"#, crate::names::MAIN_S);
        let mut reader = Reader::new(strict.as_bytes());
        reader.root().unwrap();
        assert_eq!(reader.seen.flavour(), names::Flavour::Strict);
    }

    #[test]
    fn an_unprefixed_attribute_is_in_no_namespace() {
        let xml = format!(r#"<sheet xmlns="{MAIN}" name="Budget" sheetId="1"/>"#);
        let mut reader = Reader::new(xml.as_bytes());
        let (_, attrs) = reader.root().unwrap().expect("a root");
        assert_eq!(attrs.plain("name"), Some("Budget"));
        // Not in the element's default namespace, which is the mistake this guards.
        assert_eq!(attrs.get(Ns::Spreadsheet, "name"), None);
    }

    // ---- text ----

    #[test]
    fn leading_and_trailing_space_survives() {
        let xml = format!(r#"<t xmlns="{MAIN}" xml:space="preserve"> a </t>"#);
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap();
        assert_eq!(reader.text().unwrap(), " a ");
    }

    #[test]
    fn a_string_value_flattens_the_runs_beneath_it() {
        let xml = format!(r#"<si xmlns="{MAIN}"><r><t>one</t></r><r><t> two</t></r></si>"#);
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap();
        assert_eq!(reader.text().unwrap(), "one two");
    }

    #[test]
    fn entities_are_resolved_but_only_the_predefined_ones() {
        let xml = format!(r#"<t xmlns="{MAIN}">a &amp; b &lt; c</t>"#);
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap();
        assert_eq!(reader.text().unwrap(), "a & b < c");
    }
}
