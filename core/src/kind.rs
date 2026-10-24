// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which kind of document a pile of bytes is. **\[GENERIC\]**
//!
//! Decided before parsing, because the readers are tolerant by construction and handing a
//! spreadsheet to the text reader produces an empty document rather than an error. Sniffed from
//! content, never from the file name: `.fods` content turns up under `.xml`.

use std::io::Read;

use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

use crate::odf::names::Ns;
use crate::odf::package::is_package;

/// A document type this suite has a reader for — and `Presentation`, recognised so a shell can say
/// it does not open those rather than fail to parse one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DocumentKind {
    Spreadsheet,
    Text,
    Presentation,
}

/// The media type, byte for byte (`doc/ods-format.md` §10), as it appears inside a document. Not
/// the `-flat-xml` spellings, which are LibreOffice's type-detection labels and belong only in a
/// `.desktop` file (§1.2).
pub const SPREADSHEET: &str = "application/vnd.oasis.opendocument.spreadsheet";
pub const TEXT: &str = "application/vnd.oasis.opendocument.text";
pub const PRESENTATION: &str = "application/vnd.oasis.opendocument.presentation";

impl DocumentKind {
    /// The media type a document of this kind carries.
    pub fn media_type(self) -> &'static str {
        match self {
            DocumentKind::Spreadsheet => SPREADSHEET,
            DocumentKind::Text => TEXT,
            DocumentKind::Presentation => PRESENTATION,
        }
    }

    /// The `grind` subcommand that opens one, for an error message that tells a user what to
    /// do instead of only what went wrong. `None` for a kind no app handles.
    pub fn command(self) -> Option<&'static str> {
        match self {
            DocumentKind::Spreadsheet => Some("sheet"),
            DocumentKind::Text => Some("text"),
            DocumentKind::Presentation => None,
        }
    }

    /// The plain-language name, for a banner or a diagnostic.
    pub fn label(self) -> &'static str {
        match self {
            DocumentKind::Spreadsheet => "spreadsheet",
            DocumentKind::Text => "text document",
            DocumentKind::Presentation => "presentation",
        }
    }

    fn from_media_type(media: &str) -> Option<Self> {
        match media.trim() {
            SPREADSHEET => Some(DocumentKind::Spreadsheet),
            TEXT => Some(DocumentKind::Text),
            PRESENTATION => Some(DocumentKind::Presentation),
            _ => None,
        }
    }

    /// The body element that stands for this kind — `office:spreadsheet`, `office:text`,
    /// `office:presentation` (§10).
    fn from_body_element(local: &str) -> Option<Self> {
        match local {
            "spreadsheet" => Some(DocumentKind::Spreadsheet),
            "text" => Some(DocumentKind::Text),
            "presentation" => Some(DocumentKind::Presentation),
            _ => None,
        }
    }
}

/// What a shell opens when a person named a kind (`--sheet`, `--text`) and a file. The file
/// decides; a flag that disagrees is an error naming both kinds, with `shown` the file as the
/// person will recognise it.
pub fn reconcile(
    asked: Option<DocumentKind>,
    found: DocumentKind,
    shown: &str,
) -> Result<DocumentKind, String> {
    match asked {
        Some(asked) if asked != found => Err(format!(
            "{shown} is a {}, not a {}",
            found.label(),
            asked.label()
        )),
        _ => Ok(found),
    }
}

/// How many XML events to look at before giving up on the flat form. The root's `office:mimetype`
/// is the first event, but a bare `content.xml` has none and is identified by its body element,
/// behind the bounded prologue of meta, settings and styles.
const MAX_EVENTS: usize = 4096;

/// What kind of document these bytes are, or `None` if nothing here says — a truncated file, a zip
/// that is not ODF, or XML with no media type and no body element. Callers word their own
/// diagnostic.
pub fn kind(bytes: &[u8]) -> Option<DocumentKind> {
    if is_package(bytes) {
        return package_kind(bytes);
    }
    // The projection first: one line and no parse, and it cannot be confused with XML or a zip.
    if let Some(kind) = crate::projection::is_projection(bytes) {
        return Some(kind);
    }
    flat_kind(bytes)
}

/// The package form: the `mimetype` entry, read through the zip directory rather than at §1.1's
/// fixed offset so that a non-conforming package still works (R5). A missing or unreadable
/// `mimetype` falls back to `content.xml`'s root.
fn package_kind(bytes: &[u8]) -> Option<DocumentKind> {
    let mut archive = crate::odf::package::archive(bytes).ok()?;
    if let Ok(entry) = archive.by_name("mimetype") {
        // A media type is a few dozen bytes; a `mimetype` entry that is not is not one.
        let mut media = String::new();
        if entry.take(256).read_to_string(&mut media).is_ok()
            && let Some(kind) = DocumentKind::from_media_type(&media)
        {
            return Some(kind);
        }
    }
    // No usable `mimetype`. `content.xml` has no `office:mimetype` either, so this lands on
    // the body element — which is the more authoritative answer anyway.
    let mut content = archive.by_name("content.xml").ok()?;
    let buf = crate::odf::package::read_entry(&mut content).ok()?;
    flat_kind(&buf)
}

/// The flat form, and any loose ODF XML fragment: the root's `office:mimetype`, or failing
/// that the `office:` body element.
fn flat_kind(bytes: &[u8]) -> Option<DocumentKind> {
    let mut reader = NsReader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    for _ in 0..MAX_EVENTS {
        let event = match reader.read_event_into(&mut buf) {
            Ok(event) => event,
            // Malformed XML is not this function's problem to report; the reader that runs
            // next will say so properly.
            Err(_) => return None,
        };
        let start = match event {
            Event::Start(ref e) | Event::Empty(ref e) => e,
            Event::Eof => return None,
            _ => {
                buf.clear();
                continue;
            }
        };

        // `office:mimetype` on the root settles it outright (§1.2).
        for attr in start.attributes().flatten() {
            let (rr, local) = reader.resolver().resolve_attribute(attr.key);
            if !matches!(rr, ResolveResult::Bound(n) if Ns::from_uri(n.as_ref()) == Ns::Office) {
                continue;
            }
            if local.as_ref() != "mimetype" {
                continue;
            }
            if let Ok(value) = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                && let Some(kind) = DocumentKind::from_media_type(&value)
            {
                return Some(kind);
            }
        }

        // Otherwise the body element, once we reach one.
        let (rr, local) = reader.resolver().resolve_element(start.name());
        if matches!(rr, ResolveResult::Bound(n) if Ns::from_uri(n.as_ref()) == Ns::Office)
            && let Some(kind) = DocumentKind::from_body_element(local.as_ref())
        {
            return Some(kind);
        }

        buf.clear();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_agrees_with_the_flag_is_fine() {
        let found = DocumentKind::Text;
        assert_eq!(
            reconcile(Some(DocumentKind::Text), found, "a.fodt"),
            Ok(found)
        );
        assert_eq!(reconcile(None, found, "a.fodt"), Ok(found), "no flag");
    }

    /// A file decides, and disagreeing is an error rather than an override, because opening a
    /// spreadsheet as a document shows an empty one.
    #[test]
    fn a_file_that_contradicts_the_flag_is_an_error_naming_both() {
        let error = reconcile(
            Some(DocumentKind::Text),
            DocumentKind::Spreadsheet,
            "book.fods",
        )
        .unwrap_err();
        assert_eq!(error, "book.fods is a spreadsheet, not a text document");
        let error = reconcile(
            Some(DocumentKind::Spreadsheet),
            DocumentKind::Presentation,
            "deck.odp",
        )
        .unwrap_err();
        assert_eq!(error, "deck.odp is a presentation, not a spreadsheet");
    }
    /// The `office:` namespace, for a test that builds documents by hand.
    use crate::odf::names::OFFICE as OFFICE_NS;

    fn flat(mimetype: &str, body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="{OFFICE_NS}" office:mimetype="{mimetype}" office:version="1.4">
  <office:body><office:{body}/></office:body>
</office:document>"#
        )
    }

    #[test]
    fn the_flat_root_mimetype_decides() {
        assert_eq!(
            kind(flat(SPREADSHEET, "spreadsheet").as_bytes()),
            Some(DocumentKind::Spreadsheet)
        );
        assert_eq!(
            kind(flat(TEXT, "text").as_bytes()),
            Some(DocumentKind::Text)
        );
        assert_eq!(
            kind(flat(PRESENTATION, "presentation").as_bytes()),
            Some(DocumentKind::Presentation)
        );
    }

    #[test]
    fn a_fragment_without_a_mimetype_falls_back_to_the_body_element() {
        // What `content.xml` looks like: no `office:mimetype` anywhere in it.
        let content = format!(
            r#"<office:document-content xmlns:office="{OFFICE_NS}" office:version="1.4">
  <office:body><office:text/></office:body>
</office:document-content>"#
        );
        assert_eq!(kind(content.as_bytes()), Some(DocumentKind::Text));
    }

    #[test]
    fn the_prefix_carries_no_meaning() {
        // §8.1: dispatch is on the URI. A document using `ns0:` must read the same.
        let content =
            format!(r#"<ns0:document xmlns:ns0="{OFFICE_NS}" ns0:mimetype="{SPREADSHEET}"/>"#);
        assert_eq!(kind(content.as_bytes()), Some(DocumentKind::Spreadsheet));
    }

    #[test]
    fn an_unknown_media_type_is_none_rather_than_a_guess() {
        assert_eq!(kind(flat("application/pdf", "chart").as_bytes()), None);
        assert_eq!(kind(b"not xml at all"), None);
        assert_eq!(kind(b""), None);
        // A zip that is not an ODF package.
        assert_eq!(kind(b"PK\x03\x04nonsense"), None);
    }

    #[test]
    fn a_kind_knows_the_command_that_opens_it() {
        assert_eq!(DocumentKind::Spreadsheet.command(), Some("sheet"));
        assert_eq!(DocumentKind::Text.command(), Some("text"));
        assert_eq!(
            DocumentKind::Presentation.command(),
            None,
            "naming a kind is not the same as handling it"
        );
        assert_eq!(DocumentKind::Text.media_type(), TEXT);
    }
}
