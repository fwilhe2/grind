// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Carrying what the model does not own through a save that regenerates. **\[GENERIC\]**
//!
//! **The rule this module exists for: saving never makes an existing ODF file worse.** A
//! document type's writer regenerates the parts its model owns — the body, and the automatic
//! styles the body points at — whenever an edit cannot be spliced into the original bytes (a
//! new paragraph, a changed cell style). Before this module, regenerating meant writing a
//! *fresh* document, so one press of Enter in a LibreOffice document threw away its
//! `office:styles`, its master pages (page size, margins, headers, footers), its font
//! declarations, `office:meta` and `office:settings` — and in a package `styles.xml`,
//! `meta.xml`, `settings.xml` and every other entry besides.
//!
//! Now a regenerate is **merged into the file it came from**:
//!
//! * [`merge`] keeps the original document's prolog, its root start tag and every top-level
//!   part verbatim, and takes from the generated document only `office:body` (which replaces
//!   the original's) and the children of `office:automatic-styles`, `office:styles` and
//!   `office:font-face-decls` (which join the original's). Any namespace the generated parts
//!   use and the original root does not declare is added to it.
//! * [`repackage`] does the same one level up: every entry of the original zip survives,
//!   `content.xml` is the merged content, and the manifest keeps the original's entries.
//!
//! This is not a model of the parts it carries — nothing here understands a master page. It is
//! the same retain-and-splice trick `odf::source` plays per cell or per paragraph, played per
//! top-level part, which is why it can stay generic: the only vocabulary it knows is the
//! document envelope (`office:`) and a style's identity (`style:name`, `style:family`).

use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Write};
use std::ops::Range;

use quick_xml::NsReader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;

use super::names::Ns;
use crate::{Error, Result};

/// One element at depth one or two of a document, found by [`scan`].
#[derive(Clone, Debug)]
struct Node {
    /// 1 for a child of the root, 2 for a grandchild.
    depth: u8,
    /// Index of the depth-one node holding a depth-two one.
    parent: Option<usize>,
    ns: Ns,
    local: String,
    /// `style:name` and `style:family`, which together are a style's identity.
    name: Option<String>,
    family: Option<String>,
    /// The start tag alone.
    start: Range<usize>,
    /// Start tag through end tag (equal to `start` when self-closed).
    range: Range<usize>,
}

impl Node {
    fn is(&self, ns: Ns, local: &str) -> bool {
        self.ns == ns && self.local == local
    }

    /// What makes two style declarations "the same one" for a merge.
    fn key(&self) -> (Ns, &str, Option<&str>, Option<&str>) {
        (
            self.ns,
            self.local.as_str(),
            self.name.as_deref(),
            self.family.as_deref(),
        )
    }
}

/// The root element and the two levels below it.
#[derive(Debug)]
struct Tree {
    /// The root's start tag.
    root_start: Range<usize>,
    /// The root's end tag.
    root_end: Range<usize>,
    /// `xmlns:prefix="uri"` declarations on the root, in order.
    declarations: Vec<(String, String)>,
    nodes: Vec<Node>,
}

impl Tree {
    fn top(&self) -> impl Iterator<Item = (usize, &Node)> {
        self.nodes.iter().enumerate().filter(|(_, n)| n.depth == 1)
    }

    fn find_top(&self, local: &str) -> Option<(usize, &Node)> {
        self.top().find(|(_, n)| n.is(Ns::Office, local))
    }

    fn children(&self, parent: usize) -> impl Iterator<Item = &Node> {
        self.nodes
            .iter()
            .filter(move |n| n.depth == 2 && n.parent == Some(parent))
    }

    fn binding(&self, prefix: &str) -> Option<&str> {
        self.declarations
            .iter()
            .find(|(p, _)| p == prefix)
            .map(|(_, uri)| uri.as_str())
    }
}

fn resolve(reader: &NsReader<&[u8]>, e: &BytesStart) -> (Ns, String) {
    let (rr, local) = reader.resolver().resolve_element(e.name());
    let ns = match rr {
        ResolveResult::Bound(n) => Ns::from_uri(n.as_ref()),
        _ => Ns::Other,
    };
    (ns, local.as_ref().to_owned())
}

fn style_attribute(reader: &NsReader<&[u8]>, e: &BytesStart, wanted: &str) -> Option<String> {
    e.attributes().flatten().find_map(|attr| {
        let (rr, local) = reader.resolver().resolve_attribute(attr.key);
        let bound_style =
            matches!(rr, ResolveResult::Bound(n) if Ns::from_uri(n.as_ref()) == Ns::Style);
        (bound_style && local.as_ref() == wanted).then(|| attr.value.to_string())
    })
}

/// The root and its first two levels, with every byte range. `None` for anything that is not
/// a well-formed document with one root — a merge never guesses.
fn scan(bytes: &[u8]) -> Option<Tree> {
    let mut reader = NsReader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    let mut root_start = None;
    let root_end;
    let mut declarations = Vec::new();
    let mut nodes: Vec<Node> = Vec::new();
    // Indices into `nodes` of the open depth-1 and depth-2 elements, by depth.
    let mut open: Vec<Option<usize>> = Vec::new();
    let mut top: Option<usize> = None;

    loop {
        let from = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf).ok()?;
        let span = from..reader.buffer_position() as usize;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let depth = open.len();
                if depth == 0 {
                    if root_start.is_some() {
                        return None;
                    }
                    root_start = Some(span.clone());
                    for attr in e.attributes().flatten() {
                        if let Some(prefix) = attr.key.as_ref().strip_prefix("xmlns:") {
                            declarations.push((prefix.to_owned(), attr.value.to_string()));
                        }
                    }
                    if empty {
                        root_end = span.end..span.end;
                        break;
                    }
                    open.push(None);
                    continue;
                }
                let mut index = None;
                if depth <= 2 {
                    let (ns, local) = resolve(&reader, e);
                    nodes.push(Node {
                        depth: depth as u8,
                        parent: if depth == 2 { top } else { None },
                        ns,
                        local,
                        name: style_attribute(&reader, e, "name"),
                        family: style_attribute(&reader, e, "family"),
                        start: span.clone(),
                        range: span.clone(),
                    });
                    index = Some(nodes.len() - 1);
                    if depth == 1 {
                        top = index;
                    }
                }
                if !empty {
                    open.push(index);
                }
            }
            Event::End(_) => {
                let closed = open.pop()?;
                if open.is_empty() {
                    root_end = span;
                    break;
                }
                if let Some(i) = closed {
                    nodes[i].range.end = span.end;
                }
            }
            Event::Eof => return None,
            _ => {}
        }
        buf.clear();
    }
    Some(Tree {
        root_start: root_start?,
        root_end,
        declarations,
        nodes,
    })
}

/// Where a top-level part sits in the schema's order, shared by `office:document` (rng:1060)
/// and `office:document-content` (rng:1093), which is a subsequence of it.
fn rank(node: &Node) -> Option<u8> {
    if node.ns != Ns::Office {
        return None;
    }
    Some(match node.local.as_str() {
        "meta" => 0,
        "settings" => 1,
        "scripts" => 2,
        "font-face-decls" => 3,
        "styles" => 4,
        "automatic-styles" => 5,
        "master-styles" => 6,
        "body" => 7,
        _ => return None,
    })
}

/// The qualified name of a start tag, as spelled — `office:styles` out of `<office:styles …>`.
fn qname(tag: &[u8]) -> &[u8] {
    let rest = tag.strip_prefix(b"<").unwrap_or(tag);
    let end = rest
        .iter()
        .position(|c| c.is_ascii_whitespace() || *c == b'/' || *c == b'>')
        .unwrap_or(rest.len());
    &rest[..end]
}

/// `element`, with `extra` declarations put on its own start tag. Used only when a prefix the
/// generated part uses is bound to something else on the original root, which no real
/// document does — but "no real document does" is not a reason to write a wrong one.
fn declared(element: &[u8], extra: &str) -> Vec<u8> {
    if extra.is_empty() {
        return element.to_vec();
    }
    let at = 1 + qname(element).len();
    [&element[..at], extra.as_bytes(), &element[at..]].concat()
}

/// Merge a freshly generated document into the one it was read from.
///
/// `original` and `generated` are both whole XML documents of the same root — a flat
/// `office:document`, or a package's `office:document-content`. What comes back is `original`
/// with:
///
/// * its `office:body` replaced by `generated`'s;
/// * `generated`'s automatic styles added to its own, a generated style replacing an original
///   one with the same name and family (the generated body points at the generated one);
/// * `generated`'s common styles and font declarations added where the original has none of
///   the same name and family — the original's own declaration wins, because it is the richer
///   one and the body only ever names it;
/// * any top-level part `generated` has and `original` lacks inserted in schema order;
/// * every other byte exactly as it was.
///
/// A `generated` document with no body leaves the original's alone — which is how a package's
/// `styles.xml` (`office:document-styles`, no body at all) is merged with the one a writer
/// would have written.
///
/// `None` when either document cannot be scanned — the caller then writes `generated` as it
/// is, which can only happen for an original too broken to read anything back out of.
pub fn merge(original: &[u8], generated: &[u8]) -> Option<Vec<u8>> {
    let o = scan(original)?;
    let g = scan(generated)?;
    let g_body = g.find_top("body").map(|(_, node)| node.range.clone());

    // Namespaces: what the generated parts need that the original root does not give them —
    // only the prefixes those parts actually spell.
    let taken = taken_bytes(&g, generated);
    let uses = |prefix: &str| {
        let element = format!("<{prefix}:");
        let attribute = format!(" {prefix}:");
        let window = |needle: &str| taken.windows(needle.len()).any(|w| w == needle.as_bytes());
        window(&element) || window(&attribute)
    };
    let mut on_root = String::new();
    let mut local = String::new();
    for (prefix, uri) in g.declarations.iter().filter(|(prefix, _)| uses(prefix)) {
        match o.binding(prefix) {
            Some(bound) if bound == uri => {}
            Some(_) => local.push_str(&format!(" xmlns:{prefix}=\"{uri}\"")),
            None => on_root.push_str(&format!(" xmlns:{prefix}=\"{uri}\"")),
        }
    }

    let mut out = Vec::with_capacity(original.len() + generated.len());
    // Prolog, then the root start tag with any new declarations before its `>`.
    out.extend_from_slice(&original[..o.root_start.start]);
    let tag = &original[o.root_start.clone()];
    let close = tag.len() - 1;
    out.extend_from_slice(&tag[..close]);
    out.extend_from_slice(on_root.as_bytes());
    out.extend_from_slice(&tag[close..]);

    let generated_tops: Vec<(usize, &Node)> = g.top().collect();
    let mut inserted: HashSet<usize> = HashSet::new();
    // A top-level part the original lacks goes in ahead of the first original part that the
    // schema puts after it, on a line of its own.
    let mut insert_before = |out: &mut Vec<u8>, before: Option<u8>| {
        for (i, node) in &generated_tops {
            let Some(r) = rank(node) else { continue };
            if inserted.contains(i) || o.find_top(&node.local).is_some() {
                continue;
            }
            if before.is_none_or(|b| r < b) {
                inserted.insert(*i);
                out.extend_from_slice(b"\n ");
                out.extend_from_slice(&declared(&generated[node.range.clone()], &local));
            }
        }
    };

    let mut at = o.root_start.end;
    for (index, node) in o.top() {
        out.extend_from_slice(&original[at..node.range.start]);
        if let Some(r) = rank(node) {
            insert_before(&mut out, Some(r));
        }
        let element = &original[node.range.clone()];
        match (node.ns, node.local.as_str()) {
            (Ns::Office, "body") => match &g_body {
                Some(body) => {
                    match inner_ranges(&o, index, original, &g, generated) {
                        // The file's own `office:body` and `office:text` (or `office:spreadsheet`)
                        // tags, indentation and all, around the generated content.
                        Some((o_inner, g_inner)) => {
                            let o_open = &original[node.start.end..o_inner.start];
                            out.extend_from_slice(&original[node.range.start..node.start.end]);
                            // The whitespace, then the child's start tag with any local
                            // declarations the generated content needs.
                            let lt = o_open.iter().position(|c| *c == b'<').unwrap_or(0);
                            out.extend_from_slice(&o_open[..lt]);
                            out.extend_from_slice(&declared(&o_open[lt..], &local));
                            let g_content = &generated[g_inner.clone()];
                            let trimmed = g_content.len()
                                - g_content
                                    .iter()
                                    .rev()
                                    .take_while(|c| c.is_ascii_whitespace())
                                    .count();
                            out.extend_from_slice(&g_content[..trimmed]);
                            out.extend_from_slice(leading_whitespace(
                                original,
                                o_inner.start,
                                o_inner.end,
                            ));
                            out.extend_from_slice(&original[o_inner.end..node.range.end]);
                        }
                        None => out.extend_from_slice(&declared(&generated[body.clone()], &local)),
                    }
                }
                None => out.extend_from_slice(element),
            },
            (Ns::Office, local_name @ ("automatic-styles" | "styles" | "font-face-decls")) => {
                let generated_wins = local_name == "automatic-styles";
                match g.find_top(local_name) {
                    Some((g_index, _)) => merge_children(
                        &mut out,
                        original,
                        &o,
                        index,
                        generated,
                        g.children(g_index).collect(),
                        generated_wins,
                        &local,
                    ),
                    None => out.extend_from_slice(element),
                }
            }
            _ => out.extend_from_slice(element),
        }
        at = node.range.end;
    }
    out.extend_from_slice(&original[at..o.root_end.start]);
    // Whatever is left belongs at the end — which, before the root's end tag, is where the
    // body would have been had the original had none.
    insert_before(&mut out, None);
    out.extend_from_slice(&original[o.root_end.start..]);
    Some(out)
}

/// Everything [`merge`] can take from a generated document: its body and the children of its
/// styles containers.
fn taken_bytes(g: &Tree, generated: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, node) in g.top() {
        match node.local.as_str() {
            "body" => out.extend_from_slice(&generated[node.range.clone()]),
            "automatic-styles" | "styles" | "font-face-decls" => {
                for child in g.children(index) {
                    out.extend_from_slice(&generated[child.range.clone()]);
                }
            }
            _ => out.extend_from_slice(&generated[node.range.clone()]),
        }
    }
    out
}

/// Inside the one element each `office:body` holds — `office:text`, `office:spreadsheet` — when
/// both bodies hold exactly one, of the same name: the byte range of its content in the
/// original and in the generated document. `None` sends the caller back to replacing the whole
/// body, which is always correct and only costs the file's own tags.
fn inner_ranges(
    o: &Tree,
    o_body: usize,
    original: &[u8],
    g: &Tree,
    generated: &[u8],
) -> Option<(Range<usize>, Range<usize>)> {
    let (g_body, _) = g.find_top("body")?;
    let only = |tree: &Tree, body: usize| {
        let children: Vec<&Node> = tree.children(body).collect();
        (children.len() == 1).then(|| children[0].clone())
    };
    let oc = only(o, o_body)?;
    let gc = only(g, g_body)?;
    if (oc.ns, &oc.local) != (gc.ns, &gc.local) || oc.start == oc.range || gc.start == gc.range {
        return None;
    }
    let end_tag = |bytes: &[u8], node: &Node| {
        bytes[node.range.clone()]
            .iter()
            .rposition(|c| *c == b'<')
            .map(|p| node.range.start + p)
    };
    Some((
        oc.start.end..end_tag(original, &oc)?,
        gc.start.end..end_tag(generated, &gc)?,
    ))
}

/// One styles container, the original's children and the generated ones together.
#[allow(clippy::too_many_arguments)]
fn merge_children(
    out: &mut Vec<u8>,
    original: &[u8],
    o: &Tree,
    index: usize,
    generated: &[u8],
    incoming: Vec<&Node>,
    generated_wins: bool,
    local: &str,
) {
    let node = &o.nodes[index];
    let existing: Vec<&Node> = o.children(index).collect();
    let keys: HashSet<_> = existing.iter().map(|n| n.key()).collect();
    let incoming_keys: HashSet<_> = incoming.iter().map(|n| n.key()).collect();
    let adding: Vec<&Node> = incoming
        .into_iter()
        .filter(|n| generated_wins || !keys.contains(&n.key()))
        .collect();
    let replaced = |n: &Node| generated_wins && incoming_keys.contains(&n.key());

    if adding.is_empty() {
        out.extend_from_slice(&original[node.range.clone()]);
        return;
    }
    let self_closed = node.start == node.range;
    if self_closed {
        // `<office:automatic-styles/>` — open it up.
        let tag = &original[node.start.clone()];
        out.extend_from_slice(&tag[..tag.len() - 2]);
        out.push(b'>');
    } else {
        out.extend_from_slice(&original[node.start.clone()]);
    }
    let body_end = if self_closed {
        node.start.end
    } else {
        // The end tag starts at the last `<` of the element.
        original[node.range.clone()]
            .iter()
            .rposition(|c| *c == b'<')
            .map_or(node.range.end, |p| node.range.start + p)
    };
    // Each new child goes on a line of its own, indented the way the file indents the
    // children it already has, and ahead of the whitespace that precedes the end tag.
    let indent = existing
        .first()
        .map(|first| leading_whitespace(original, node.start.end, first.range.start))
        .filter(|ws| !ws.is_empty())
        .unwrap_or(b"\n  ");
    let tail = body_end - leading_whitespace(original, node.start.end, body_end).len();

    let mut at = node.start.end;
    for child in &existing {
        if replaced(child) {
            // The child and the line it stood on.
            let ws = leading_whitespace(original, at, child.range.start).len();
            out.extend_from_slice(&original[at..child.range.start - ws]);
            at = child.range.end;
        }
    }
    out.extend_from_slice(&original[at.min(tail)..tail]);
    for child in &adding {
        out.extend_from_slice(indent);
        out.extend_from_slice(&declared(&generated[child.range.clone()], local));
    }
    if self_closed {
        out.extend_from_slice(b"\n </");
        out.extend_from_slice(qname(&original[node.start.clone()]));
        out.push(b'>');
    } else {
        out.extend_from_slice(&original[tail.max(at)..node.range.end]);
    }
}

/// The whitespace immediately before `end`, not reaching back past `floor`.
fn leading_whitespace(bytes: &[u8], floor: usize, end: usize) -> &[u8] {
    let start = bytes[floor..end]
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(floor, |p| floor + p + 1);
    &bytes[start..end]
}

/// Every `style:name` an original document's `office:automatic-styles` declares — the names a
/// writer must not hand out again, so that what it generates never collides with what the
/// file already uses outside the body (a header's own paragraph style, say).
pub fn automatic_style_names(original: &[u8]) -> HashSet<String> {
    let Some(tree) = scan(original) else {
        return HashSet::new();
    };
    tree.find_top("automatic-styles")
        .map(|(index, _)| {
            tree.children(index)
                .filter_map(|n| n.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The `style:text-properties` start tag inside the depth-two element `parent` of `bytes`'
/// `office:styles` whose name is `None` and family is `family` — a `style:default-style`.
/// Generic for the one caller that patches a document-wide default (the spreadsheet's locale),
/// and named by structure rather than by any document type's vocabulary.
pub fn default_style_range(bytes: &[u8], family: &str) -> Option<Range<usize>> {
    let tree = scan(bytes)?;
    let (index, _) = tree.find_top("styles")?;
    tree.children(index)
        .find(|n| n.is(Ns::Style, "default-style") && n.family.as_deref() == Some(family))
        .map(|n| n.range.clone())
}

/// **The last thing before bytes reach a file**: is this well-formed XML whose every element
/// and attribute prefix is bound? A writer that merges or splices into somebody's document
/// calls this on the result and refuses to write when it fails — a save that errors leaves the
/// file on disk exactly as it was, and a save that writes a file nothing can open is the worst
/// way there is to make a document worse.
pub fn check(document: &[u8]) -> Result<()> {
    let bad = |why: String| {
        Err(Error::Xml(format!(
            "refusing to write a broken document: {why}"
        )))
    };
    let mut reader = NsReader::from_reader(document);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut roots = 0usize;
    loop {
        let event = match reader.read_event_into(&mut buf) {
            Ok(event) => event,
            Err(error) => return bad(error.to_string()),
        };
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                if depth == 0 {
                    roots += 1;
                }
                if let (ResolveResult::Unknown(prefix), _) =
                    reader.resolver().resolve_element(e.name())
                {
                    return bad(format!("unbound prefix {prefix}"));
                }
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        return bad("a malformed attribute".into());
                    };
                    if let (ResolveResult::Unknown(prefix), _) =
                        reader.resolver().resolve_attribute(attr.key)
                    {
                        return bad(format!("unbound prefix {prefix}"));
                    }
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return bad("a close tag with nothing open".into()),
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    match (depth, roots) {
        (0, 1) => Ok(()),
        (0, n) => bad(format!("{n} root elements")),
        _ => bad("an element left open".into()),
    }
}

/// [`check`], measured against the file being replaced: refused only when `output` is broken
/// and `original` was not. A document that arrived already broken — an undeclared prefix, which
/// the reader tolerates (R5) — is not made worse by writing it back in the state it came in,
/// and refusing would leave its owner unable to save at all.
pub fn check_against(original: &[u8], output: &[u8]) -> Result<()> {
    if original == output {
        return Ok(());
    }
    match check(output) {
        Err(error) if check(original).is_ok() => Err(error),
        _ => Ok(()),
    }
}

/// Every element and every attribute inside `office:body`, counted — keyed by namespace URI
/// rather than by prefix (`{uri}local` for an element, `{uri}element@{uri}attribute` for an
/// attribute), so that two documents spelling one namespace with two prefixes still agree.
///
/// What a save is held to. A writer knows which of these keys it *owns* — the ones its model
/// carries and regenerates, whose counts may move with any edit — and [`losses`] says which of
/// the rest went down. Those are content the model never saw, and a save that drops them makes
/// the file worse. `None` when the document cannot be read at all.
pub fn body_vocabulary(document: &[u8]) -> Option<BTreeMap<String, usize>> {
    vocabulary_within(document, None)
}

/// [`body_vocabulary`], counting only elements whose start tag lies inside one of `ranges` —
/// `None` for the whole body.
fn vocabulary_within(
    document: &[u8],
    ranges: Option<&[Range<usize>]>,
) -> Option<BTreeMap<String, usize>> {
    let mut reader = NsReader::from_reader(document);
    let mut buf = Vec::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    // Depth inside `office:body`, 0 when outside it.
    let mut inside = 0usize;
    loop {
        let from = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf).ok()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let uri = match ns {
                    ResolveResult::Bound(n) => n.as_ref().to_owned(),
                    _ => String::new(),
                };
                let element = format!("{{{uri}}}{}", local.as_ref());
                let counted = ranges.is_none_or(|ranges| ranges.iter().any(|r| r.contains(&from)));
                if inside == 0 {
                    if Ns::from_uri(&uri) == Ns::Office && local.as_ref() == "body" && !empty {
                        inside = 1;
                    }
                    buf.clear();
                    continue;
                }
                if !counted {
                    if !empty {
                        inside += 1;
                    }
                    buf.clear();
                    continue;
                }
                *counts.entry(element.clone()).or_default() += 1;
                for attr in e.attributes().flatten() {
                    if attr.key.as_ref().starts_with("xmlns") {
                        continue;
                    }
                    let (ns, local) = reader.resolver().resolve_attribute(attr.key);
                    let uri = match ns {
                        ResolveResult::Bound(n) => n.as_ref().to_owned(),
                        _ => String::new(),
                    };
                    *counts
                        .entry(format!("{element}@{{{uri}}}{}", local.as_ref()))
                        .or_default() += 1;
                }
                if !empty {
                    inside += 1;
                }
            }
            Event::End(_) if inside > 0 => {
                inside -= 1;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Some(counts)
}

/// What `output`'s body has fewer of than `original`'s, among the keys `owned` does not claim
/// — each spelled with the prefixes `original` declares (`2 × text:note`,
/// `text:list@text:style-name`). Empty is the only answer a save may go ahead on.
pub fn losses(original: &[u8], output: &[u8], owned: &dyn Fn(&str) -> bool) -> Option<Vec<String>> {
    losses_allowing(original, output, owned, &[])
}

/// [`losses`], with everything inside `removed` — byte ranges of `original` a person deleted
/// outright, a whole sheet — allowed to go: the deletion is the edit, not a loss.
pub fn losses_allowing(
    original: &[u8],
    output: &[u8],
    owned: &dyn Fn(&str) -> bool,
    removed: &[Range<usize>],
) -> Option<Vec<String>> {
    if original == output {
        return Some(Vec::new());
    }
    let mut before = body_vocabulary(original)?;
    if !removed.is_empty() {
        for (key, count) in vocabulary_within(original, Some(removed))? {
            if let Some(total) = before.get_mut(&key) {
                *total = total.saturating_sub(count);
            }
        }
    }
    let after = body_vocabulary(output)?;
    let prefixes: Vec<(String, String)> = scan(original)
        .map(|tree| tree.declarations)
        .unwrap_or_default();
    let spell = |key: &str| {
        let mut out = key.to_owned();
        for (prefix, uri) in &prefixes {
            out = out.replace(&format!("{{{uri}}}"), &format!("{prefix}:"));
        }
        out.replace("{}", "")
    };
    Some(
        before
            .iter()
            .filter(|(key, _)| !owned(key))
            .filter_map(|(key, count)| {
                let left = after.get(key).copied().unwrap_or(0);
                (left < *count).then(|| match count - left {
                    1 => spell(key),
                    n => format!("{n} × {}", spell(key)),
                })
            })
            .collect(),
    )
}

/// The prefix `document` binds to `uri` on its root, if any — what a caller patching an
/// attribute in that namespace has to spell it with.
pub fn prefix_for(document: &[u8], uri: &str) -> Option<String> {
    scan(document)?
        .declarations
        .into_iter()
        .find(|(_, bound)| bound == uri)
        .map(|(prefix, _)| prefix)
}

/// The start tag at the front of `element` with each `(name, value)` set: replaced where the
/// tag has the attribute, appended where it does not, and removed where `value` is `None`.
/// Every other byte of the tag is kept. Names are qualified as the document spells them.
pub fn set_attributes(element: &str, changes: &[(&str, Option<&str>)]) -> String {
    let end = element.find('>').unwrap_or(element.len());
    let self_closed = element[..end].ends_with('/');
    let body_end = if self_closed { end - 1 } else { end };
    let mut tag = element[..body_end].to_owned();
    for (name, value) in changes {
        let pattern = format!(" {name}=");
        if let Some(at) = tag.find(&pattern) {
            let value_start = at + pattern.len();
            let quote = tag[value_start..].chars().next().unwrap_or('"');
            let value_end = tag[value_start + 1..]
                .find(quote)
                .map_or(tag.len(), |p| value_start + 1 + p + 1);
            match value {
                Some(v) => tag.replace_range(
                    at..value_end,
                    &format!(" {name}=\"{}\"", crate::odf::xml::esc(v)),
                ),
                None => tag.replace_range(at..value_end, ""),
            }
        } else if let Some(v) = value {
            tag.push_str(&format!(" {name}=\"{}\"", crate::odf::xml::esc(v)));
        }
    }
    // An attribute removed from the end of a tag that broke its lines leaves the break behind.
    let mut out = tag.trim_end().to_owned();
    out.push_str(&element[body_end..]);
    out
}

/// A package rebuilt from the one it was read from.
///
/// Every entry of `original` survives byte for byte (raw-copied, compression and all), except:
///
/// * `content.xml`, which is `content`;
/// * each path in `replace`, which is that entry's new bytes — a `styles.xml` the writer had
///   to change, a chart it regenerated — and any path in `replace` the original lacked, added;
/// * everything under a directory in `drop` — parts the model read and has now regenerated
///   under another name, which left in place would be orphans the manifest still declares;
/// * the thumbnail, when `content` differs from the original's: a picture of text that is no
///   longer there misinforms a file manager and can show what a person deleted;
/// * `META-INF/manifest.xml`, which is the original's entries for what survived plus one for
///   each part added (`added_entries`, already-formed `manifest:file-entry` elements).
pub fn repackage(
    original: &[u8],
    content: &[u8],
    replace: &[(String, Vec<u8>)],
    drop: &[String],
    added_entries: &[String],
) -> Result<Vec<u8>> {
    let zip = |e: zip::result::ZipError| Error::Package(e.to_string());
    let mut archive = zip::ZipArchive::new(Cursor::new(original)).map_err(zip)?;

    let original_content = super::package::content_xml(original)?;
    let content_changed = original_content != content;
    let dropped = |path: &str| {
        drop.iter().any(|dir| path.starts_with(&format!("{dir}/")))
            || (content_changed && path.starts_with("Thumbnails/"))
    };

    let mut names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index_raw(i).ok().map(|f| f.name().to_owned()))
        .collect();
    names.retain(|n| !n.is_empty());

    let manifest = super::package::part(original, "META-INF/manifest.xml")
        .map(|m| rewrite_manifest(&m, &|path| !dropped(path), added_entries))
        .transpose()?;

    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut written: HashSet<String> = HashSet::new();
    // `mimetype` first and stored, whatever order the original had it in (§1.1).
    if let Some(i) = names.iter().position(|n| n == "mimetype") {
        let file = archive.by_index_raw(i).map_err(zip)?;
        w.raw_copy_file(file).map_err(zip)?;
        written.insert("mimetype".to_owned());
    }
    for (i, name) in names.iter().enumerate() {
        if written.contains(name) || dropped(name) {
            continue;
        }
        match name.as_str() {
            "content.xml" => {
                w.start_file("content.xml", deflated).map_err(zip)?;
                w.write_all(content)?;
            }
            "META-INF/manifest.xml" => {
                w.start_file(name.as_str(), deflated).map_err(zip)?;
                w.write_all(manifest.as_deref().unwrap_or_default().as_bytes())?;
            }
            _ => match replace.iter().find(|(path, _)| path == name) {
                Some((_, bytes)) => {
                    w.start_file(name.as_str(), deflated).map_err(zip)?;
                    w.write_all(bytes)?;
                }
                None => {
                    let file = archive.by_index_raw(i).map_err(zip)?;
                    w.raw_copy_file(file).map_err(zip)?;
                }
            },
        }
        written.insert(name.clone());
    }
    for (path, bytes) in replace {
        if written.insert(path.clone()) {
            let options = if path == "mimetype" { stored } else { deflated };
            w.start_file(path.as_str(), options).map_err(zip)?;
            w.write_all(bytes)?;
        }
    }
    if !written.contains("content.xml") {
        w.start_file("content.xml", deflated).map_err(zip)?;
        w.write_all(content)?;
    }
    Ok(w.finish().map_err(zip)?.into_inner())
}

/// The original manifest with the entries for dropped parts taken out and `added` put in
/// before its end tag. Everything else — the order, the attributes, a vendor's own entries —
/// is the original's bytes.
fn rewrite_manifest(
    manifest: &[u8],
    keep: &dyn Fn(&str) -> bool,
    added: &[String],
) -> Result<String> {
    let text = String::from_utf8_lossy(manifest).into_owned();
    let mut reader = NsReader::from_reader(text.as_bytes());
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut cut: Vec<Range<usize>> = Vec::new();
    let mut kept: HashSet<String> = HashSet::new();
    let mut root_end = None;
    let mut depth = 0usize;
    loop {
        let from = reader.buffer_position() as usize;
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| Error::Package(format!("manifest: {e}")))?;
        let span = from..reader.buffer_position() as usize;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                if depth == 1 {
                    let path = e.attributes().flatten().find_map(|a| {
                        a.key
                            .as_ref()
                            .ends_with("full-path")
                            .then(|| a.value.to_string())
                    });
                    if let Some(path) = path {
                        if path == "/" || keep(&path) {
                            kept.insert(path);
                        } else {
                            // The entry and the whitespace in front of it.
                            let start = text[..span.start]
                                .rfind(|c: char| !c.is_whitespace())
                                .map_or(span.start, |p| p + 1);
                            let end = match empty {
                                true => span.end,
                                false => {
                                    crate::odf::xml::element_extent(text.as_bytes(), span.clone())
                                        .map_or(span.end, |r| r.end)
                                }
                            };
                            cut.push(start..end);
                        }
                    }
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    root_end = Some(span.start);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let root_end = root_end.ok_or_else(|| Error::Package("manifest has no root".into()))?;
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for range in &cut {
        if range.start < at {
            continue;
        }
        out.push_str(&text[at..range.start]);
        at = range.end;
    }
    out.push_str(&text[at..root_end]);
    let trailing_ws = out.len() - out.trim_end().len();
    let indent = out.split_off(out.len() - trailing_ws);
    for entry in added {
        let path = entry
            .split("full-path=\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .unwrap_or_default();
        if !kept.contains(path) {
            out.push_str("\n ");
            out.push_str(entry);
        }
    }
    out.push_str(&indent);
    out.push_str(&text[root_end..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A made-up body vocabulary (`x:`), as everywhere in this crate: R8 keeps document types out.
    const O: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
    const S: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";

    fn original() -> String {
        format!(
            "<?xml version=\"1.0\"?>\n<office:document xmlns:office=\"{O}\" xmlns:style=\"{S}\">\n \
             <office:meta><m/></office:meta>\n \
             <office:styles><style:style style:name=\"Body\" style:family=\"p\"/></office:styles>\n \
             <office:automatic-styles><style:style style:name=\"T1\" style:family=\"t\"/>\
             <style:page-layout style:name=\"pm1\"/></office:automatic-styles>\n \
             <office:master-styles><style:master-page style:name=\"Default\"/></office:master-styles>\n \
             <office:body><old/></office:body>\n</office:document>\n"
        )
    }

    fn generated() -> String {
        format!(
            "<?xml version=\"1.0\"?>\n<office:document xmlns:office=\"{O}\" xmlns:style=\"{S}\" \
             xmlns:x=\"urn:x\">\n <office:automatic-styles><style:style style:name=\"T1\" \
             style:family=\"t\"><new/></style:style><style:style style:name=\"T9\" \
             style:family=\"t\"/></office:automatic-styles>\n <office:body><x:new/></office:body>\n\
             </office:document>\n"
        )
    }

    #[test]
    fn the_body_is_replaced_and_every_other_part_kept() {
        let merged =
            String::from_utf8(merge(original().as_bytes(), generated().as_bytes()).unwrap())
                .unwrap();
        assert!(merged.contains("<office:meta><m/></office:meta>"));
        assert!(merged.contains("<style:master-page style:name=\"Default\"/>"));
        assert!(merged.contains("<style:style style:name=\"Body\" style:family=\"p\"/>"));
        assert!(merged.contains("<style:page-layout style:name=\"pm1\"/>"));
        assert!(merged.contains("<office:body><x:new/></office:body>"));
        assert!(!merged.contains("<old/>"));
        // The generated T1 replaced the original one; T9 joined.
        assert!(merged.contains("style:name=\"T1\" style:family=\"t\"><new/>"));
        assert_eq!(merged.matches("style:name=\"T1\"").count(), 1);
        assert!(merged.contains("style:name=\"T9\""));
        // The new prefix is declared on the original root.
        assert!(merged.contains("xmlns:x=\"urn:x\""));
        assert!(scan(merged.as_bytes()).is_some(), "well-formed");
    }

    #[test]
    fn a_part_the_original_lacks_goes_in_schema_order() {
        let original = format!(
            "<office:document xmlns:office=\"{O}\"><office:meta/><office:body><a/></office:body>\
             </office:document>"
        );
        let generated = format!(
            "<office:document xmlns:office=\"{O}\" xmlns:style=\"{S}\"><office:automatic-styles>\
             <style:style style:name=\"T1\"/></office:automatic-styles><office:body><b/></office:body>\
             </office:document>"
        );
        let merged =
            String::from_utf8(merge(original.as_bytes(), generated.as_bytes()).unwrap()).unwrap();
        let meta = merged.find("<office:meta/>").unwrap();
        let styles = merged.find("<office:automatic-styles>").unwrap();
        let body = merged.find("<office:body>").unwrap();
        assert!(meta < styles && styles < body, "{merged}");
        assert!(merged.contains("<b/>") && !merged.contains("<a/>"));
    }

    #[test]
    fn the_original_common_style_wins_over_a_generated_one_of_the_same_name() {
        let original = format!(
            "<office:document xmlns:office=\"{O}\" xmlns:style=\"{S}\"><office:styles>\
             <style:default-style style:family=\"c\"><rich/></style:default-style></office:styles>\
             <office:body/></office:document>"
        );
        let generated = format!(
            "<office:document xmlns:office=\"{O}\" xmlns:style=\"{S}\"><office:styles>\
             <style:default-style style:family=\"c\"/><style:style style:name=\"N\"/></office:styles>\
             <office:body><b/></office:body></office:document>"
        );
        let merged =
            String::from_utf8(merge(original.as_bytes(), generated.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains("<rich/>"));
        assert_eq!(
            merged.matches("default-style").count(),
            2,
            "one element, open and close"
        );
        assert!(merged.contains("style:name=\"N\""));
    }

    #[test]
    fn a_conflicting_prefix_is_declared_locally_rather_than_rebound() {
        let original = format!(
            "<office:document xmlns:office=\"{O}\" xmlns:x=\"urn:other\"><office:body/></office:document>"
        );
        let generated = format!(
            "<office:document xmlns:office=\"{O}\" xmlns:x=\"urn:x\"><office:body><x:b/></office:body>\
             </office:document>"
        );
        let merged =
            String::from_utf8(merge(original.as_bytes(), generated.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains("xmlns:x=\"urn:other\""));
        assert!(
            merged.contains("<office:body xmlns:x=\"urn:x\"><x:b/>"),
            "{merged}"
        );
    }

    #[test]
    fn the_check_refuses_what_a_reader_could_not_open() {
        assert!(check(original().as_bytes()).is_ok());
        assert!(check(b"<a:x xmlns:a=\"u\"><a:y/></a:x>").is_ok());
        assert!(
            check(b"<a:x xmlns:a=\"u\"><b:y/></a:x>").is_err(),
            "unbound element prefix"
        );
        assert!(
            check(b"<a:x xmlns:a=\"u\" b:z=\"1\"/>").is_err(),
            "unbound attribute prefix"
        );
        assert!(
            check(b"<a:x xmlns:a=\"u\"><a:y></a:x>").is_err(),
            "mismatched close"
        );
        assert!(check(b"<a:x xmlns:a=\"u\">").is_err(), "left open");
        assert!(check(b"<x/><y/>").is_err(), "two roots");

        // Already broken on the way in: writing it back is not a regression.
        let broken = b"<a:x xmlns:a=\"u\"><b:y/></a:x>";
        assert!(check_against(broken, broken).is_ok());
        assert!(check_against(broken, b"<a:x xmlns:a=\"u\"><b:y/><a:z/></a:x>").is_ok());
        assert!(check_against(b"<a:x xmlns:a=\"u\"/>", broken).is_err());
    }

    #[test]
    fn a_body_losing_what_nobody_owns_is_named() {
        let original = format!(
            "<office:document xmlns:office=\"{O}\" xmlns:x=\"urn:x\"><office:meta><x:gone/></office:meta>\
             <office:body><x:p x:style=\"a\">one<x:note/></x:p><x:p>two</x:p></office:body>\
             </office:document>"
        );
        // A different prefix for the same namespace is the same vocabulary.
        let output = format!(
            "<office:document xmlns:office=\"{O}\" xmlns:y=\"urn:x\"><office:body>\
             <y:p>one, edited</y:p><y:p>two</y:p><y:p>three</y:p></office:body></office:document>"
        );
        let owned = |key: &str| key == "{urn:x}p";
        let lost = losses(original.as_bytes(), output.as_bytes(), &owned).unwrap();
        assert_eq!(
            lost,
            ["x:note", "x:p@x:style"],
            "outside the body does not count"
        );
        let owned = |key: &str| key.starts_with("{urn:x}p");
        let lost = losses(original.as_bytes(), output.as_bytes(), &owned).unwrap();
        assert_eq!(lost, ["x:note"]);
    }

    #[test]
    fn attributes_are_set_appended_and_removed_in_place() {
        let tag = "<s:p a:x=\"1\" a:y='2'/>rest";
        assert_eq!(
            set_attributes(
                tag,
                &[("a:x", Some("9")), ("a:z", Some("3")), ("a:y", None)]
            ),
            "<s:p a:x=\"9\" a:z=\"3\"/>rest"
        );
    }

    #[test]
    fn a_part_with_no_body_merges_its_styles_only() {
        let original = format!(
            "<office:document-styles xmlns:office=\"{O}\" xmlns:style=\"{S}\"><office:styles>\
             <style:style style:name=\"A\"/></office:styles><office:master-styles/>\
             </office:document-styles>"
        );
        let generated = format!(
            "<office:document-styles xmlns:office=\"{O}\" xmlns:style=\"{S}\"><office:styles>\
             <style:default-style style:family=\"c\"/></office:styles></office:document-styles>"
        );
        let merged =
            String::from_utf8(merge(original.as_bytes(), generated.as_bytes()).unwrap()).unwrap();
        assert!(merged.contains("style:name=\"A\""));
        assert!(merged.contains("style:family=\"c\""));
        assert!(merged.contains("<office:master-styles/>"));
    }

    #[test]
    fn a_package_keeps_every_entry_and_drops_only_a_stale_thumbnail() {
        use std::io::Write as _;
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, body) in [
            ("mimetype", "application/x-test"),
            ("content.xml", "<old/>"),
            ("styles.xml", "<styles/>"),
            ("meta.xml", "<meta/>"),
            ("Thumbnails/thumbnail.png", "png"),
            ("Object 1/content.xml", "<chart/>"),
            (
                "META-INF/manifest.xml",
                "<manifest:manifest xmlns:manifest=\"urn:m\">\n \
                 <manifest:file-entry manifest:full-path=\"/\"/>\n \
                 <manifest:file-entry manifest:full-path=\"styles.xml\"/>\n \
                 <manifest:file-entry manifest:full-path=\"Thumbnails/thumbnail.png\"/>\n \
                 <manifest:file-entry manifest:full-path=\"Object 1/\"/>\n \
                 <manifest:file-entry manifest:full-path=\"Object 1/content.xml\"/>\n\
                 </manifest:manifest>",
            ),
        ] {
            w.start_file(name, stored).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        let original = w.finish().unwrap().into_inner();

        let out = repackage(
            &original,
            b"<new/>",
            &[("Object 2/content.xml".to_owned(), b"<c2/>".to_vec())],
            &["Object 1".to_owned()],
            &["<manifest:file-entry manifest:full-path=\"Object 2/content.xml\"/>".to_owned()],
        )
        .unwrap();
        let get = |p: &str| super::super::package::part(&out, p);
        assert_eq!(get("content.xml").as_deref(), Some(&b"<new/>"[..]));
        assert_eq!(get("styles.xml").as_deref(), Some(&b"<styles/>"[..]));
        assert_eq!(get("meta.xml").as_deref(), Some(&b"<meta/>"[..]));
        assert_eq!(get("Object 2/content.xml").as_deref(), Some(&b"<c2/>"[..]));
        assert_eq!(get("Thumbnails/thumbnail.png"), None);
        assert_eq!(get("Object 1/content.xml"), None);
        let manifest = String::from_utf8(get("META-INF/manifest.xml").unwrap()).unwrap();
        assert!(manifest.contains("full-path=\"styles.xml\""));
        assert!(manifest.contains("full-path=\"Object 2/content.xml\""));
        assert!(!manifest.contains("Thumbnails"));
        assert!(!manifest.contains("Object 1"));
        assert!(out.starts_with(b"PK\x03\x04") && out[30..38] == *b"mimetype");
    }
}
