// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which kind of document a pile of bytes is. **\[GENERIC\]**
//!
//! One ODF reader per document type, chosen explicitly — so something has to choose, and it
//! has to do so *before* parsing, because handing a spreadsheet to the text reader produces
//! an empty document rather than an error (§8's default-ignore architecture is tolerant by
//! construction, which is exactly wrong for this one question).
//!
//! Every caller of this is a place a user meets the wrong file: `grind info`, a shell's Open
//! dialog, the terminal shell picking a mode, and the cross-app handoff banner that offers to
//! launch the sibling application. None of them can afford to guess.
//!
//! **Sniffed from content, never from the file name** — the same rule
//! [`crate::odf::package::is_package`] follows, and for the same reason: `.fods` content turns
//! up under `.xml`, and an extension is a hint from a filesystem rather than a fact about the
//! data.

use std::io::{Cursor, Read};

use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

use crate::odf::names::Ns;
use crate::odf::package::is_package;

/// A document type this suite has a reader for.
///
/// `Presentation` is deliberately present and deliberately unhandled elsewhere: recognising
/// one is how a shell says *"that is a presentation, and this build does not open those"*
/// rather than failing to parse it. Naming it here costs a variant and buys an honest error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DocumentKind {
    Spreadsheet,
    Text,
    Presentation,
}

/// The media type, byte for byte (doc/ods-format.md §10).
///
/// These are the strings that appear **inside** a document — the `mimetype` package entry and
/// the flat root's `office:mimetype`. They are *not* the `-flat-xml` suffixed spellings: §1.2
/// records that those are LibreOffice's internal type-detection labels and are never written
/// into a file. The suffixed forms belong in a `.desktop` file's `MimeType=` line and nowhere
/// near this module.
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

/// How many XML events to look at before giving up on the flat form.
///
/// The root element carries `office:mimetype` and is the first event, so one would nearly
/// always do. The budget exists for the fragment case — a bare `content.xml`, which has no
/// `office:mimetype` at all and must be identified by its body element instead, sitting behind
/// `office:meta`, `office:settings`, `office:styles` and `office:automatic-styles`. Those are
/// bounded prologue, not content, so a small budget covers every conforming document while
/// keeping this a sniff rather than a parse of a 50 MB file.
const MAX_EVENTS: usize = 4096;

/// What kind of document these bytes are, or `None` if nothing here says.
///
/// `None` is a real answer and not a failure: a truncated file, a zip that is not ODF, or XML
/// that declares no media type and reaches no body element. Callers turn it into their own
/// diagnostic, because "this is not an ODF document" reads differently in an Open dialog than
/// it does on stderr.
pub fn kind(bytes: &[u8]) -> Option<DocumentKind> {
    if is_package(bytes) {
        return package_kind(bytes);
    }
    flat_kind(bytes)
}

/// The package form: the `mimetype` entry, which §1.1 requires to be first, stored, and the
/// media type byte for byte.
///
/// Read through the zip directory rather than at the fixed offset every reader sniffs. The
/// offset is faster and is what §1.1 describes, but it is only correct for a *conforming*
/// package, and R5 says other people's files have to work. A document whose `mimetype` entry
/// is missing or unreadable falls back to `content.xml`'s root, which is what actually decides
/// how the document is parsed.
fn package_kind(bytes: &[u8]) -> Option<DocumentKind> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    if let Ok(mut entry) = archive.by_name("mimetype") {
        let mut media = String::new();
        if entry.read_to_string(&mut media).is_ok()
            && let Some(kind) = DocumentKind::from_media_type(&media)
        {
            return Some(kind);
        }
    }
    // No usable `mimetype`. `content.xml` has no `office:mimetype` either, so this lands on
    // the body element — which is the more authoritative answer anyway.
    let mut content = archive.by_name("content.xml").ok()?;
    let mut buf = Vec::with_capacity(content.size() as usize);
    content.read_to_end(&mut buf).ok()?;
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
            let (rr, local) = reader.resolve_attribute(attr.key);
            if !matches!(rr, ResolveResult::Bound(n) if Ns::from_uri(n.as_ref()) == Ns::Office) {
                continue;
            }
            if local.as_ref() != b"mimetype" {
                continue;
            }
            if let Ok(value) = attr.unescape_value()
                && let Some(kind) = DocumentKind::from_media_type(&value)
            {
                return Some(kind);
            }
        }

        // Otherwise the body element, once we reach one.
        let (rr, local) = reader.resolve_element(start.name());
        if matches!(rr, ResolveResult::Bound(n) if Ns::from_uri(n.as_ref()) == Ns::Office)
            && let Ok(local) = std::str::from_utf8(local.as_ref())
            && let Some(kind) = DocumentKind::from_body_element(local)
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
