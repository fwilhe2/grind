// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Moving a whole document between ODF's two physical forms without losing anything.
//! **\[GENERIC\]**
//!
//! A document type's writer regenerates only what its model owns, and carries the rest of the
//! file it came from through a save by splicing into that file's own bytes (`envelope`). That
//! works when the save is in the form the file was read in. Saving a `.fodt` as an `.odt` — or
//! an `.ods` as a `.fods` — used to *start from nothing*, and so dropped the page layout, the
//! named styles, the metadata and, worse, every footnote, field and frame the model does not
//! read, with no error at all.
//!
//! Now a save into the other form is a save into the document's **own** form first, with every
//! guard that has (`WouldLose`, the read-back), and then [`convert`] — which moves every part
//! across rather than reinterpreting any of them, and **refuses** (`Error::WouldLose`) rather
//! than drops whatever the other form has no place for.
//!
//! * **Flat to package** splits `office:document`'s parts into `content.xml`, `styles.xml`,
//!   `meta.xml` and `settings.xml` (§3.1–§3.4). Automatic styles go into both of the first two,
//!   because each part's styles are its own and either may be pointed at; a picture's
//!   `office:binary-data` stays where it is, which a package allows. An embedded document
//!   (`draw:object` holding an `office:document`, a chart) becomes its own `Object N/`
//!   sub-document, since LibreOffice drops one written inline in a package
//!   (`doc/chart-format.md`).
//! * **Package to flat** is the same in reverse, and has the more to refuse: every picture a
//!   part points at is inlined as `office:binary-data`, every embedded document as a nested
//!   `office:document`, and what a flat file has no place for — an entry nothing points at,
//!   two declarations of one automatic style that disagree, a prefix bound two ways — is a
//!   refusal naming it. Only what is a cache of something else is let go: thumbnails, the
//!   user-interface configuration, a picture standing in for an embedded document, and a
//!   `manifest.rdf` that says nothing but which parts the package has.
//!
//! The only vocabulary here is the envelope's (`office:`), `xlink:href`, and the local names
//! of the elements a picture can hang off — never a document type's (R8).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read, Write};

use quick_xml::NsReader;
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

use super::Form;
use super::envelope::{self, Tree};
use super::names::{OFFICE, XLINK};
use crate::{Error, Result};

/// Elements whose `xlink:href` may name a picture (or an OLE blob) in the package, and which
/// may carry the same bytes as an `office:binary-data` child instead (rng:5383 and its
/// siblings) — by qualified name, spelled with [`QUALIFIED`]'s prefixes whatever the document
/// binds. Anything else pointing at a file in the package (a sound, an embedded font) has no
/// such child, and is refused.
const BINARY: &[&str] = &[
    "draw:image",
    "draw:fill-image",
    "style:background-image",
    "text:list-level-style-image",
    "draw:object-ole",
];

/// The prefix [`BINARY`] spells each namespace with.
const QUALIFIED: &[(&str, &str)] = &[
    (super::names::DRAW, "draw"),
    (super::names::STYLE, "style"),
    (super::names::TEXT, "text"),
];

/// A file of a package by its path — an entry, or an embedded document by its directory.
type Part = (String, Vec<u8>);

/// One declaration merged from two parts: its identity (name and family) and its bytes.
type Child = (Vec<u8>, Vec<u8>);

/// The XML declaration a part written out of a fragment starts with.
const PROLOG: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

/// `bytes` — a package or a flat document — in `to`'s form, every part carried. The bytes
/// back unchanged when they already are; `Error::WouldLose` naming what the other form has no
/// place for, rather than a smaller document.
pub fn convert(bytes: &[u8], to: Form) -> Result<Vec<u8>> {
    match (super::package::is_package(bytes), to) {
        (true, Form::Package) | (false, Form::Flat) => Ok(bytes.to_vec()),
        (false, Form::Package) => to_package(bytes),
        (true, Form::Flat) => to_flat(bytes),
        (_, Form::Projection) => Err(Error::Package(
            "the projection is not one of ODF's forms".to_owned(),
        )),
    }
}

fn refuse(what: String) -> Error {
    Error::WouldLose(vec![what])
}

fn malformed(part: &str) -> Error {
    Error::Package(format!("{part} is not a well-formed document"))
}

// --- flat to package -----------------------------------------------------------------------

/// One flat document as the entries of a package (paths relative to it) and the manifest
/// entries they need, its own media type first.
struct Split {
    mimetype: String,
    version: Option<String>,
    entries: Vec<(String, Vec<u8>)>,
    /// `(full-path, media-type)`, without the package's own `/`.
    manifest: Vec<(String, String)>,
}

fn to_package(flat: &[u8]) -> Result<Vec<u8>> {
    let split = split(flat)?;
    let version = split.version.as_deref().unwrap_or("1.4");
    let zip = |e: zip::result::ZipError| Error::Package(e.to_string());
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    // `mimetype` first, stored, no extra field (§1.1).
    w.start_file("mimetype", stored).map_err(zip)?;
    w.write_all(split.mimetype.as_bytes())?;

    let esc = super::xml::esc;
    let mut manifest = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" \
         manifest:version=\"{}\">\n \
         <manifest:file-entry manifest:full-path=\"/\" manifest:version=\"{}\" \
         manifest:media-type=\"{}\"/>\n",
        esc(version),
        esc(version),
        esc(&split.mimetype)
    );
    for (path, media) in &split.manifest {
        manifest.push_str(&format!(
            " <manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{}\"/>\n",
            esc(path),
            esc(media)
        ));
    }
    manifest.push_str("</manifest:manifest>\n");
    w.start_file("META-INF/manifest.xml", deflated)
        .map_err(zip)?;
    w.write_all(manifest.as_bytes())?;
    for (path, bytes) in &split.entries {
        w.start_file(path.as_str(), deflated).map_err(zip)?;
        w.write_all(bytes)?;
    }
    Ok(w.finish().map_err(zip)?.into_inner())
}

/// Split one `office:document` into a package's parts, an embedded document into its own
/// directory beside them.
fn split(flat: &[u8]) -> Result<Split> {
    let (flat, objects) = lift_objects(flat)?;
    let flat = flat.as_slice();
    let tree = envelope::scan(flat).ok_or_else(|| malformed("the document"))?;
    let root = &flat[tree.root_start.clone()];
    let root_name = envelope::qname(root);
    let prefix = std::str::from_utf8(root_name)
        .ok()
        .and_then(|name| name.split_once(':'))
        .map(|(prefix, _)| prefix.to_owned())
        .ok_or_else(|| malformed("the document"))?;
    let attribute = |local: &str| root_attribute(root, OFFICE, local);
    let mimetype = attribute("mimetype").ok_or_else(|| {
        refuse("office:mimetype (a flat document that does not say what it is)".to_owned())
    })?;
    let version = attribute("version");

    let mut parts: BTreeMap<&str, Vec<u8>> = BTreeMap::new();
    for (_, node) in tree.top() {
        if node.ns != super::names::Ns::Office || envelope::rank(node).is_none() {
            return Err(refuse(format!(
                "{} (a part of the document a package has no place for)",
                String::from_utf8_lossy(envelope::qname(&flat[node.start.clone()]))
            )));
        }
        parts.insert(node.local.as_str(), flat[node.range.clone()].to_vec());
    }
    let prolog = &flat[..tree.root_start.start];
    let prolog = if prolog.trim_ascii().is_empty() {
        PROLOG
    } else {
        prolog
    };
    let part = |local: &str, pieces: &[&str]| -> Option<Vec<u8>> {
        if !pieces.iter().any(|piece| parts.contains_key(piece)) {
            return None;
        }
        let tag = renamed(root, &format!("{prefix}:{local}"));
        let tag = envelope::set_attributes(&tag, &[(&format!("{prefix}:mimetype"), None)]);
        let mut out = prolog.to_vec();
        out.extend_from_slice(tag.as_bytes());
        for piece in pieces {
            if let Some(bytes) = parts.get(piece) {
                out.extend_from_slice(bytes);
            }
        }
        out.extend_from_slice(format!("</{prefix}:{local}>").as_bytes());
        Some(out)
    };

    let mut entries = Vec::new();
    let mut manifest = Vec::new();
    let mut add = |path: &str, bytes: Option<Vec<u8>>| {
        if let Some(bytes) = bytes {
            entries.push((path.to_owned(), bytes));
            manifest.push((path.to_owned(), "text/xml".to_owned()));
        }
    };
    add(
        "content.xml",
        Some(
            part(
                "document-content",
                &["scripts", "font-face-decls", "automatic-styles", "body"],
            )
            .unwrap_or_default(),
        ),
    );
    add(
        "styles.xml",
        part(
            "document-styles",
            &[
                "font-face-decls",
                "styles",
                "automatic-styles",
                "master-styles",
            ],
        ),
    );
    add("meta.xml", part("document-meta", &["meta"]));
    add("settings.xml", part("document-settings", &["settings"]));

    for (directory, sub) in objects {
        let inner = split(&sub)?;
        manifest.push((format!("{directory}/"), inner.mimetype.clone()));
        for (path, media) in inner.manifest {
            manifest.push((format!("{directory}/{path}"), media));
        }
        for (path, bytes) in inner.entries {
            entries.push((format!("{directory}/{path}"), bytes));
        }
    }
    Ok(Split {
        mimetype,
        version,
        entries,
        manifest,
    })
}

/// Every `draw:object` holding a whole `office:document`, replaced by a reference to an
/// `Object N` directory — the shape LibreOffice reads in a package — and the documents taken
/// out, by that directory.
fn lift_objects(flat: &[u8]) -> Result<(Vec<u8>, Vec<Part>)> {
    let (found, documents) = embedded(flat)?;
    if found.is_empty() {
        return Ok((flat.to_vec(), documents));
    }
    let tree = envelope::scan(flat).ok_or_else(|| malformed("the document"))?;
    let xlink = tree
        .declarations
        .iter()
        .find(|(_, uri)| uri == XLINK)
        .map(|(prefix, _)| prefix.clone());
    let (xlink, declare) = match xlink {
        Some(prefix) => (prefix, false),
        None => ("xlink".to_owned(), true),
    };
    let mut out = Vec::with_capacity(flat.len());
    let mut at = 0;
    if declare {
        // On the root, so every part split from it has it.
        let end = tree.root_start.end - 1;
        let end = if flat[end - 1] == b'/' { end - 1 } else { end };
        out.extend_from_slice(&flat[..end]);
        out.extend_from_slice(format!(" xmlns:{xlink}=\"{XLINK}\"").as_bytes());
        at = end;
    }
    for (n, object) in found.iter().enumerate() {
        out.extend_from_slice(&flat[at..object.element.start]);
        let tag = std::str::from_utf8(&flat[object.start.clone()])
            .map_err(|_| malformed("the document"))?;
        let tag = envelope::set_attributes(
            tag.trim_end_matches('>').trim_end_matches('/'),
            &[
                (
                    &format!("{xlink}:href"),
                    Some(&format!("./Object {}", n + 1)),
                ),
                (&format!("{xlink}:type"), Some("simple")),
                (&format!("{xlink}:show"), Some("embed")),
                (&format!("{xlink}:actuate"), Some("onLoad")),
            ],
        );
        out.extend_from_slice(tag.as_bytes());
        out.extend_from_slice(b"/>");
        at = object.element.end;
    }
    out.extend_from_slice(&flat[at..]);
    Ok((out, documents))
}

/// One `draw:object` with a document inside it.
struct Embedded {
    /// The `draw:object` element, start tag through end tag.
    element: std::ops::Range<usize>,
    /// Its start tag.
    start: std::ops::Range<usize>,
}

/// Every outermost `draw:object` in `flat` whose content is an `office:document`, and those
/// documents, each under the `Object N` directory it will live in — with the namespace
/// declarations it inherited from the root put on its own, since a part of a package declares
/// its own.
fn embedded(flat: &[u8]) -> Result<(Vec<Embedded>, Vec<Part>)> {
    let declarations = envelope::scan(flat)
        .map(|tree| tree.declarations)
        .unwrap_or_default();
    let mut reader = NsReader::from_reader(flat);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut found = Vec::new();
    let mut documents = Vec::new();
    let mut skip_to = 0usize;
    // A `draw:object` just opened, waiting to see whether its first child is a document.
    let mut pending: Option<(std::ops::Range<usize>, std::ops::Range<usize>)> = None;
    loop {
        let from = reader.buffer_position() as usize;
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|_| malformed("the document"))?;
        let span = from..reader.buffer_position() as usize;
        if from < skip_to {
            if matches!(event, Event::Eof) {
                break;
            }
            buf.clear();
            continue;
        }
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let bound =
                    |uri: &str| matches!(ns, ResolveResult::Bound(ref n) if n.as_ref() == uri);
                if let Some((object, element)) = pending.take()
                    && bound(OFFICE)
                    && local.as_ref() == "document"
                    && let Some(document) = super::xml::element_extent(flat, span.clone())
                {
                    documents.push((
                        format!("Object {}", documents.len() + 1),
                        declared_on(&flat[document], &declarations),
                    ));
                    skip_to = element.end;
                    found.push(Embedded {
                        element,
                        start: object,
                    });
                } else if matches!(event, Event::Start(_))
                    && bound(super::names::DRAW)
                    && local.as_ref() == "object"
                    && let Some(element) = super::xml::element_extent(flat, span.clone())
                {
                    pending = Some((span, element));
                }
            }
            Event::Text(ref t) if t.as_ref().trim().is_empty() => {}
            Event::Eof => break,
            _ => pending = None,
        }
        buf.clear();
    }
    Ok((found, documents))
}

/// `element` with each of `declarations` it does not make itself added to its start tag.
fn declared_on(element: &[u8], declarations: &[(String, String)]) -> Vec<u8> {
    let at = 1 + envelope::qname(element).len();
    let tag_end = element
        .iter()
        .position(|c| *c == b'>')
        .unwrap_or(element.len());
    let tag = &element[..tag_end];
    let extra: String = declarations
        .iter()
        .filter(|(prefix, _)| {
            let needle = format!("xmlns:{prefix}=");
            !tag.windows(needle.len()).any(|w| w == needle.as_bytes())
        })
        .map(|(prefix, uri)| format!(" xmlns:{prefix}=\"{}\"", super::xml::esc(uri)))
        .collect();
    [&element[..at], extra.as_bytes(), &element[at..]].concat()
}

/// `tag` — a start tag — under another qualified name.
fn renamed(tag: &[u8], name: &str) -> String {
    let old = envelope::qname(tag).len();
    format!("<{name}{}", String::from_utf8_lossy(&tag[1 + old..]))
}

/// An attribute of a start tag by namespace and local name, read with the declarations the
/// tag itself carries — which, for a root, are all of them.
fn root_attribute(tag: &[u8], uri: &str, local: &str) -> Option<String> {
    let mut closed = tag.to_vec();
    if !closed.ends_with(b"/>") {
        closed.pop();
        closed.extend_from_slice(b"/>");
    }
    let mut reader = NsReader::from_reader(closed.as_slice());
    let mut buf = Vec::new();
    let Ok(Event::Empty(e)) = reader.read_event_into(&mut buf) else {
        return None;
    };
    e.attributes().flatten().find_map(|attr| {
        let (ns, name) = reader.resolver().resolve_attribute(attr.key);
        let bound = matches!(ns, ResolveResult::Bound(ref n) if n.as_ref() == uri);
        (bound && name.as_ref() == local)
            .then(|| {
                attr.normalized_value(XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
            .flatten()
    })
}

// --- package to flat -----------------------------------------------------------------------

/// Every file of a package, by path.
struct Archive {
    files: BTreeMap<String, Vec<u8>>,
}

impl Archive {
    fn read(bytes: &[u8]) -> Result<Self> {
        let zip = |e: zip::result::ZipError| Error::Package(e.to_string());
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(zip)?;
        let mut files = BTreeMap::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(zip)?;
            if file.is_dir() {
                continue;
            }
            let mut out = Vec::new();
            file.read_to_end(&mut out)
                .map_err(|e| Error::Package(format!("{} will not decompress: {e}", file.name())))?;
            files.insert(file.name().to_owned(), out);
        }
        Ok(Self { files })
    }

    fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    fn is_directory(&self, path: &str) -> bool {
        let dir = format!("{path}/");
        self.files.keys().any(|name| name.starts_with(&dir))
    }

    /// `manifest:media-type` by `manifest:full-path`.
    fn media_types(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let Some(manifest) = self.file("META-INF/manifest.xml") else {
            return out;
        };
        let mut reader = NsReader::from_reader(manifest);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e) | Event::Empty(e)) => {
                    let mut path = None;
                    let mut media = None;
                    for attr in e.attributes().flatten() {
                        let (_, local) = reader.resolver().resolve_attribute(attr.key);
                        let value = attr
                            .normalized_value(XmlVersion::Implicit1_0)
                            .ok()
                            .map(|v| v.into_owned());
                        match local.as_ref() {
                            "full-path" => path = value,
                            "media-type" => media = value,
                            _ => {}
                        }
                    }
                    if let (Some(path), Some(media)) = (path, media) {
                        out.insert(path, media);
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        out
    }
}

fn to_flat(bytes: &[u8]) -> Result<Vec<u8>> {
    let archive = Archive::read(bytes)?;
    let mimetype = archive
        .file("mimetype")
        .map(|m| String::from_utf8_lossy(m).trim().to_owned())
        .ok_or_else(|| refuse("mimetype (a package that does not say what it is)".to_owned()))?;
    let types = archive.media_types();
    let mut used = HashSet::new();
    let document = flat_of(&archive, "", &mimetype, &types, &mut used)?;
    let left: Vec<String> = archive
        .files
        .keys()
        .filter(|name| !used.contains(*name) && !cache(name, &archive))
        .map(|name| format!("{name} (a part of the package a flat file has no place for)"))
        .collect();
    if !left.is_empty() {
        return Err(Error::WouldLose(left));
    }
    Ok(document)
}

/// What a flat file may go without: the package's own bookkeeping, and what is only ever a
/// cache of something else in it — regenerated by whatever opens it next.
fn cache(name: &str, archive: &Archive) -> bool {
    matches!(name, "mimetype" | "META-INF/manifest.xml" | "layout-cache")
        || name.starts_with("Thumbnails/")
        || name.starts_with("Configurations2/")
        || name.contains("/Configurations2/")
        || name.starts_with("ObjectReplacements/")
        || name.contains("/ObjectReplacements/")
        || ((name == "manifest.rdf" || name.ends_with("/manifest.rdf"))
            && archive.file(name).is_some_and(rdf_says_nothing))
}

/// Whether a `manifest.rdf` says nothing beyond which parts the package has — every
/// `rdf:about` and `rdf:resource` the document itself, one of its XML parts, or a vocabulary's
/// own URI. LibreOffice writes exactly that into every file; anything more is metadata about
/// the content (an `xml:id`'s annotation), and that is not dropped.
fn rdf_says_nothing(rdf: &[u8]) -> bool {
    let mut reader = NsReader::from_reader(rdf);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) => {
                for attr in e.attributes().flatten() {
                    let (_, local) = reader.resolver().resolve_attribute(attr.key);
                    if !matches!(local.as_ref(), "about" | "resource") {
                        continue;
                    }
                    let Ok(value) = attr.normalized_value(XmlVersion::Implicit1_0) else {
                        return false;
                    };
                    let plain = value.is_empty()
                        || value.contains("://")
                        || matches!(
                            value.as_ref(),
                            "content.xml" | "styles.xml" | "meta.xml" | "settings.xml"
                        );
                    if !plain {
                        return false;
                    }
                }
            }
            Ok(Event::Text(_)) => {}
            Ok(Event::Eof) => return true,
            Err(_) => return false,
            _ => {}
        }
        buf.clear();
    }
}

/// The document in directory `base` of the package (`""` the package itself, `Object 1/` an
/// embedded one) as one `office:document`, with every part it points at inlined.
fn flat_of(
    archive: &Archive,
    base: &str,
    mimetype: &str,
    types: &HashMap<String, String>,
    used: &mut HashSet<String>,
) -> Result<Vec<u8>> {
    let mut parts: Vec<(&str, Vec<u8>)> = Vec::new();
    for name in ["content.xml", "styles.xml", "meta.xml", "settings.xml"] {
        let path = format!("{base}{name}");
        if let Some(bytes) = archive.file(&path) {
            used.insert(path);
            let bytes = inline(bytes, base, archive, types, used)?;
            parts.push((name, bytes));
        }
    }
    let Some(content) = parts.iter().position(|(name, _)| *name == "content.xml") else {
        return Err(refuse(format!("{base}content.xml (missing)")));
    };
    // An embedded formula's `content.xml` is MathML rather than an ODF document, and in a flat
    // file it is that element alone inside the `draw:object` (rng:5306's `math:math` choice).
    // Measured: LibreOffice's own flat export writes `tdf89191-1.odt`'s formula exactly so and
    // carries none of the object's `settings.xml` — there is nowhere in that shape to put it.
    if !base.is_empty() {
        let (_, bytes) = &parts[content];
        let tree = envelope::scan(bytes).ok_or_else(|| malformed("content.xml"))?;
        let root = &bytes[tree.root_start.clone()];
        let name = envelope::qname(root);
        let local = name.rsplit(|c| *c == b':').next().unwrap_or(name);
        if local != b"document-content" {
            if local != b"math" {
                return Err(refuse(format!(
                    "{base}content.xml (an embedded document of no ODF kind)"
                )));
            }
            return Ok(bytes[tree.root_start.start..tree.root_end.end].to_vec());
        }
    }
    let trees: Vec<Tree> = parts
        .iter()
        .map(|(name, bytes)| envelope::scan(bytes).ok_or_else(|| malformed(name)))
        .collect::<Result<_>>()?;

    // One set of declarations for the one root: each part's, a prefix bound two ways refused.
    let mut declarations: Vec<(String, String)> = Vec::new();
    for tree in &trees {
        for (prefix, uri) in &tree.declarations {
            match declarations.iter().find(|(p, _)| p == prefix) {
                Some((_, bound)) if bound != uri => {
                    return Err(refuse(format!(
                        "xmlns:{prefix} (bound to two namespaces by two parts)"
                    )));
                }
                Some(_) => {}
                None => declarations.push((prefix.clone(), uri.clone())),
            }
        }
    }

    // Each top-level part, by its place in `office:document` (rng:1060). The two that both
    // `content.xml` and `styles.xml` hold are merged child by child.
    let mut single: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
    let mut merged: BTreeMap<u8, Vec<Child>> = BTreeMap::new();
    for ((name, bytes), tree) in parts.iter().zip(&trees) {
        for (index, node) in tree.top() {
            let Some(rank) = envelope::rank(node) else {
                return Err(refuse(format!(
                    "{} in {base}{name} (a part a flat document has no place for)",
                    String::from_utf8_lossy(envelope::qname(&bytes[node.start.clone()]))
                )));
            };
            if matches!(node.local.as_str(), "font-face-decls" | "automatic-styles") {
                let children = merged.entry(rank).or_default();
                for child in tree.children(index) {
                    let key = format!("{:?}", child.key()).into_bytes();
                    let element = bytes[child.range.clone()].to_vec();
                    match children.iter_mut().find(|(k, _)| *k == key) {
                        Some((_, existing)) if *existing != element => {
                            // A font declared by both parts with more said about it in one —
                            // its panose number, its pitch — is the fuller declaration, and
                            // the other loses nothing to it. Anything else is two meanings.
                            let fuller = node.local == "font-face-decls"
                                && match (attributes(existing), attributes(&element)) {
                                    (Some(a), Some(b)) if a.is_subset(&b) => {
                                        *existing = element;
                                        true
                                    }
                                    (Some(a), Some(b)) => b.is_subset(&a),
                                    _ => false,
                                };
                            if !fuller {
                                return Err(refuse(format!(
                                    "{} {} (declared differently by two parts)",
                                    node.local,
                                    child.name.as_deref().unwrap_or("?")
                                )));
                            }
                        }
                        Some(_) => {}
                        None => children.push((key, element)),
                    }
                }
            } else if single
                .insert(rank, bytes[node.range.clone()].to_vec())
                .is_some()
            {
                return Err(refuse(format!(
                    "{} (held by two parts)",
                    String::from_utf8_lossy(envelope::qname(&bytes[node.start.clone()]))
                )));
            }
        }
    }

    let (_, content_bytes) = &parts[content];
    let tree = &trees[content];
    let root = &content_bytes[tree.root_start.clone()];
    let prefix = std::str::from_utf8(envelope::qname(root))
        .ok()
        .and_then(|name| name.split_once(':'))
        .map(|(prefix, _)| prefix.to_owned())
        .ok_or_else(|| malformed("content.xml"))?;
    let mut tag = renamed(root, &format!("{prefix}:document"));
    let extra: String = declarations
        .iter()
        .filter(|(p, _)| !tree.declarations.iter().any(|(q, _)| q == p))
        .map(|(p, uri)| format!(" xmlns:{p}=\"{}\"", super::xml::esc(uri)))
        .collect();
    tag = envelope::set_attributes(&tag, &[(&format!("{prefix}:mimetype"), Some(mimetype))]);
    let at = tag.find('>').unwrap_or(tag.len());
    let at = if tag[..at].ends_with('/') { at - 1 } else { at };
    tag.insert_str(at, &extra);

    let mut out = if base.is_empty() {
        let prolog = &content_bytes[..tree.root_start.start];
        if prolog.trim_ascii().is_empty() {
            PROLOG.to_vec()
        } else {
            prolog.to_vec()
        }
    } else {
        Vec::new()
    };
    out.extend_from_slice(tag.as_bytes());
    for rank in 0..=7u8 {
        if let Some(bytes) = single.get(&rank) {
            out.extend_from_slice(bytes);
        }
        if let Some(children) = merged.get(&rank) {
            let local = if rank == 3 {
                "font-face-decls"
            } else {
                "automatic-styles"
            };
            out.extend_from_slice(format!("<{prefix}:{local}>").as_bytes());
            for (_, element) in children {
                out.extend_from_slice(element);
            }
            out.extend_from_slice(format!("</{prefix}:{local}>").as_bytes());
        }
    }
    out.extend_from_slice(format!("</{prefix}:document>").as_bytes());
    Ok(out)
}

/// The attributes of a childless element, as written, for comparing two declarations of one
/// font. `None` for anything with content, which is compared whole.
fn attributes(element: &[u8]) -> Option<HashSet<(String, String)>> {
    let mut reader = NsReader::from_reader(element);
    let mut buf = Vec::new();
    let Ok(Event::Empty(e)) = reader.read_event_into(&mut buf) else {
        return None;
    };
    e.attributes()
        .map(|attr| {
            let attr = attr.ok()?;
            // Unescaped: `&apos;Liberation Sans&apos;` in one part and `'Liberation Sans'` in
            // the other is one font (`spreadsheet13e.ods`).
            let value = attr.normalized_value(XmlVersion::Implicit1_0).ok()?;
            Some((attr.key.as_ref().to_owned(), value.into_owned()))
        })
        .collect()
}

/// `part` with every `xlink:href` into the package replaced by what it points at: a file as an
/// `office:binary-data` child, a directory (an embedded document) as a nested
/// `office:document`. A link out of the package, or to nothing, is left as it is.
fn inline(
    part: &[u8],
    base: &str,
    archive: &Archive,
    types: &HashMap<String, String>,
    used: &mut HashSet<String>,
) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let office = envelope::prefix_for(part, OFFICE).unwrap_or_else(|| "office".to_owned());
    let mut reader = NsReader::from_reader(part);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut edits: Vec<(std::ops::Range<usize>, Vec<u8>)> = Vec::new();
    loop {
        let from = reader.buffer_position() as usize;
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|_| malformed("a part"))?;
        let span = from..reader.buffer_position() as usize;
        let (e, empty) = match event {
            Event::Start(ref e) => (e, false),
            Event::Empty(ref e) => (e, true),
            Event::Eof => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        let mut href = None;
        let mut xlink_prefix = None;
        for attr in e.attributes().flatten() {
            let (ns, local) = reader.resolver().resolve_attribute(attr.key);
            if matches!(ns, ResolveResult::Bound(ref n) if n.as_ref() == XLINK) {
                xlink_prefix = attr.key.prefix().map(|p| p.as_ref().to_owned());
                if local.as_ref() == "href" {
                    href = attr
                        .normalized_value(XmlVersion::Implicit1_0)
                        .ok()
                        .map(|v| v.into_owned());
                }
            }
        }
        let Some(href) = href else {
            buf.clear();
            continue;
        };
        let external = href.is_empty()
            || href.starts_with('#')
            || href.starts_with('/')
            || href.starts_with("../")
            || href
                .split('/')
                .next()
                .is_some_and(|first| first.contains(':'));
        if external {
            buf.clear();
            continue;
        }
        let path = format!(
            "{base}{}",
            href.trim_start_matches("./").trim_end_matches('/')
        );
        let local = e.local_name().as_ref().to_owned();
        let name = e.name().as_ref().to_owned();
        let (ns, _) = reader.resolver().resolve_element(e.name());
        let qualified = match ns {
            ResolveResult::Bound(uri) => QUALIFIED
                .iter()
                .find(|(bound, _)| *bound == uri.as_ref())
                .map(|(_, prefix)| format!("{prefix}:{local}")),
            _ => None,
        };
        let xlink = xlink_prefix.unwrap_or_else(|| "xlink".to_owned());
        let tag = std::str::from_utf8(&part[span.clone()]).map_err(|_| malformed("a part"))?;
        let bare = envelope::set_attributes(
            tag.trim_end_matches('>').trim_end_matches('/'),
            &[
                (&format!("{xlink}:href"), None),
                (&format!("{xlink}:type"), None),
                (&format!("{xlink}:show"), None),
                (&format!("{xlink}:actuate"), None),
            ],
        );
        if let Some(data) = archive.file(&path) {
            if !qualified.as_deref().is_some_and(|q| BINARY.contains(&q)) {
                return Err(refuse(format!("{path} (the {name} pointing at it)")));
            }
            used.insert(path.clone());
            let mut replacement = format!(
                "{bare}><{office}:binary-data>{}</{office}:binary-data>",
                base64::engine::general_purpose::STANDARD.encode(data)
            );
            if empty {
                replacement.push_str(&format!("</{name}>"));
            }
            edits.push((span, replacement.into_bytes()));
        } else if archive.is_directory(&path) {
            if qualified.as_deref() != Some("draw:object") {
                return Err(refuse(format!("{path}/ (the {name} pointing at it)")));
            }
            let media = types.get(&format!("{path}/")).cloned().ok_or_else(|| {
                refuse(format!("{path}/ (an embedded document of no stated type)"))
            })?;
            let sub = flat_of(archive, &format!("{path}/"), &media, types, used)?;
            // The document first, and whatever the element already held (LibreOffice's empty
            // `loext:p`) after it, as a picture's bytes go first in [`BINARY`]'s elements.
            let mut replacement = format!("{bare}>").into_bytes();
            replacement.extend_from_slice(&sub);
            if empty {
                replacement.extend_from_slice(format!("</{name}>").as_bytes());
            }
            edits.push((span, replacement));
        }
        buf.clear();
    }
    let mut out = Vec::with_capacity(part.len());
    let mut at = 0;
    for (range, bytes) in edits {
        out.extend_from_slice(&part[at..range.start]);
        out.extend_from_slice(&bytes);
        at = range.end;
    }
    out.extend_from_slice(&part[at..]);
    Ok(out)
}
