// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The projection — a document as plain text (`doc/dsl.md` layer 0). **\[GENERIC\]**
//!
//! This is the container; no node name of either application appears here (R8, `doc/dsl.md` §3.2).
//! It owns:
//!
//! 1. **The header** (§3.3): `grind spreadsheet`. A tolerant reader handed the wrong document type
//!    would return an empty document, so the file states its kind.
//! 2. **The token map** (§6.1): [`Emitter`] records what every byte it writes is, so a shell
//!    colours the result without a highlighter.
//! 3. **The span map** (§6.2): each [`Anchor`] ties an address — an opaque string here — to the
//!    byte range that projects it, both ways.
//! 4. **The retained text** (§3.1): [`Source`] keeps the bytes a document was read from, so saving
//!    edits the file instead of replacing it and comments and alignment survive.
//!
//! Reading is [`kdl`]'s; [`parse`] checks the header and hands back the body.

use std::ops::Range;

use crate::{DocumentKind, Error, Result};

pub mod emit;
pub mod source;

pub use emit::Emitter;
pub use source::{Location, Shape, Site, Source, entry_span, node_span};

/// The KDL implementation, re-exported so that every crate parses with one version of it: two
/// versions would be two `KdlValue` types that do not convert.
pub use kdl;

use kdl::{KdlDocument, KdlNode};

/// The first word of a projection: `grind spreadsheet`.
pub const KEYWORD: &str = "grind";

/// The extension a projection takes on disk.
pub const EXTENSION: &str = "grind";

/// What a byte range of a projection is, for a shell that colours it. Deliberately coarse: one
/// variant per colour a reader would want.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// A node name — `sheet`, `cell`, `p`. The structure of the document.
    Node,
    /// A property name, up to and including its `=`.
    Property,
    /// A string value, quotes included.
    Text,
    /// A numeric value.
    Number,
    /// A keyword value — `#true`, `#false`, `#null`.
    Keyword,
    /// A `//` comment, the one thing in a projection that means nothing to the document.
    Comment,
}

impl TokenKind {
    /// The one word each kind is called, for a CLI that prints the token map and a shell that keys
    /// a colour off it.
    pub fn name(self) -> &'static str {
        match self {
            TokenKind::Node => "node",
            TokenKind::Property => "property",
            TokenKind::Text => "text",
            TokenKind::Number => "number",
            TokenKind::Keyword => "keyword",
            TokenKind::Comment => "comment",
        }
    }
}

/// One coloured run of the projection's text, as a byte range into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Range<usize>,
}

/// One entry of the span map: an address, and the text that projects it. The address is a string
/// because `Sheet1.B5` and `#intro+5` are application vocabulary (R8); a code view hands it back
/// to the `App` that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub address: String,
    pub span: Range<usize>,
}

/// One stretch of a line of the projection, and what it is. `kind` is `None` for indentation,
/// braces and spaces, which a shell must still paint: the pieces of a line are the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece<'a> {
    pub kind: Option<TokenKind>,
    pub text: &'a str,
}

/// A projected document: the text, and the two maps beside it, recorded by [`Emitter`] while
/// writing.
#[derive(Clone, Debug, Default)]
pub struct Projection {
    text: String,
    tokens: Vec<Token>,
    anchors: Vec<Anchor>,
}

impl Projection {
    /// The projection itself — what is written to a `.grind` file and what a code view shows.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The token map, in the order the tokens appear.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// The span map, in the order the anchors were emitted.
    pub fn anchors(&self) -> &[Anchor] {
        &self.anchors
    }

    /// Where an address is projected — *select this cell, highlight that text*.
    ///
    /// The **narrowest** anchor for the address, so that asking for a sheet when a cell of it
    /// also anchors there still gets the sheet's whole block: an address is matched exactly,
    /// and the two are different strings.
    pub fn span_of(&self, address: &str) -> Option<Range<usize>> {
        self.anchors
            .iter()
            .filter(|anchor| anchor.address == address)
            .map(|anchor| anchor.span.clone())
            .min_by_key(|span| span.end - span.start)
    }

    /// What address a byte offset falls in — *put the caret there, select that cell*.
    ///
    /// The **narrowest** containing anchor, because anchors nest: a byte inside a cell's line
    /// is inside its sheet's block too, and the cell is the answer a person means.
    pub fn address_at(&self, byte: usize) -> Option<&str> {
        self.anchors
            .iter()
            .filter(|anchor| anchor.span.contains(&byte))
            .min_by_key(|anchor| anchor.span.end - anchor.span.start)
            .map(|anchor| anchor.address.as_str())
    }

    /// The line an address is projected on, 0-based — what a shell scrolls to.
    pub fn line_of(&self, address: &str) -> Option<usize> {
        let span = self.span_of(address)?;
        Some(self.text[..span.start].matches('\n').count())
    }

    pub fn into_text(self) -> String {
        self.text
    }

    // --- the code view (§6): the line-shaped questions every shell's code view asks ---

    /// How many lines the projection has.
    ///
    /// The text always ends in a newline, and the empty stretch after it is not a line — so this
    /// is the number of `\n`s, and a projection of nothing at all has none.
    pub fn line_count(&self) -> usize {
        self.text.matches('\n').count()
    }

    /// A line's byte range, **without** its newline.
    pub fn line_span(&self, line: usize) -> Option<Range<usize>> {
        let mut start = 0usize;
        for (index, piece) in self.text.split_inclusive('\n').enumerate() {
            if index == line {
                let end = start + piece.trim_end_matches('\n').len();
                return Some(start..end);
            }
            start += piece.len();
        }
        None
    }

    /// A line cut into pieces, in order, covering every byte of it exactly once — uncoloured
    /// stretches included, so concatenating the pieces gives the line back.
    pub fn line_pieces(&self, line: usize) -> Vec<Piece<'_>> {
        let Some(span) = self.line_span(line) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut at = span.start;
        for token in &self.tokens {
            if token.span.end <= span.start {
                continue;
            }
            if token.span.start >= span.end {
                break;
            }
            // A token that runs past either end of the line is clipped to it: a multi-line
            // string is one token and several lines, and each of them shows its own part.
            let start = token.span.start.max(span.start);
            let end = token.span.end.min(span.end);
            if start > at {
                out.push(Piece {
                    kind: None,
                    text: &self.text[at..start],
                });
            }
            out.push(Piece {
                kind: Some(token.kind),
                text: &self.text[start..end],
            });
            at = end;
        }
        if at < span.end {
            out.push(Piece {
                kind: None,
                text: &self.text[at..span.end],
            });
        }
        out
    }

    /// What address a whole line belongs to, for a shell whose code cursor is a line:
    ///
    /// 1. An anchor that fits inside the line, leftmost first — a grid row's first cell.
    /// 2. Otherwise the narrowest anchor overlapping it — the block a line is inside, or a node's
    ///    own line (whose anchor includes the newline, so never fits).
    ///
    /// Ties go to whichever was emitted first, which for a text block is `p12`. A shell with a
    /// column should use [`Projection::address_at`] on [`Projection::byte_at`], which is exact.
    pub fn address_on_line(&self, line: usize) -> Option<&str> {
        let span = self.line_span(line)?;
        let overlapping = || {
            self.anchors
                .iter()
                .filter(move |anchor| anchor.span.start < span.end && span.start < anchor.span.end)
        };
        overlapping()
            .filter(|anchor| anchor.span.start >= span.start && anchor.span.end <= span.end)
            .min_by_key(|anchor| anchor.span.start)
            .or_else(|| overlapping().min_by_key(|anchor| anchor.span.end - anchor.span.start))
            .map(|anchor| anchor.address.as_str())
    }

    /// The byte offset of a byte `column` on a line, clamped to the line's end: a caret past the
    /// end of a line is ordinary.
    pub fn byte_at(&self, line: usize, column: usize) -> Option<usize> {
        let span = self.line_span(line)?;
        Some((span.start + column).min(span.end))
    }
}

/// The kind a projection declares, from its header alone. A scan rather than a parse, since
/// [`crate::kind()`] must answer for any bytes; blank lines and `//` comments before the header
/// are skipped.
pub fn header_kind(text: &str) -> Option<DocumentKind> {
    let header = text.lines().find(|line| {
        let line = line.trim();
        !line.is_empty() && !line.starts_with("//")
    })?;
    let rest = header.trim().strip_prefix(KEYWORD)?;
    // `grindstone spreadsheet` is not a header; the keyword has to be a whole word.
    let word = rest.strip_prefix(|c: char| c.is_whitespace())?.trim();
    match word {
        "spreadsheet" => Some(DocumentKind::Spreadsheet),
        "text" => Some(DocumentKind::Text),
        "presentation" => Some(DocumentKind::Presentation),
        _ => None,
    }
}

/// Whether these bytes are a projection, and of what. A projection is always UTF-8.
pub fn is_projection(bytes: &[u8]) -> Option<DocumentKind> {
    header_kind(std::str::from_utf8(bytes).ok()?)
}

/// Parse a projection: check the header, hand back the kind and the body nodes, header removed.
pub fn parse(text: &str) -> Result<(DocumentKind, KdlDocument)> {
    let mut document =
        KdlDocument::parse(text).map_err(|e| Error::Projection(diagnostic(text, &e)))?;
    let kind = match document.nodes().first() {
        Some(node) if node.name().value() == KEYWORD => header(node)?,
        _ => {
            return Err(Error::Projection(format!(
                "a projection starts with `{KEYWORD} <kind>`; this one does not"
            )));
        }
    };
    document.nodes_mut().remove(0);
    Ok((kind, document))
}

/// The kind named by a parsed header node.
fn header(node: &KdlNode) -> Result<DocumentKind> {
    let word = node
        .entries()
        .iter()
        .find(|entry| entry.name().is_none())
        .and_then(|entry| entry.value().as_string())
        .ok_or_else(|| Error::Projection(format!("`{KEYWORD}` needs a document kind after it")))?;
    match word {
        "spreadsheet" => Ok(DocumentKind::Spreadsheet),
        "text" => Ok(DocumentKind::Text),
        "presentation" => Ok(DocumentKind::Presentation),
        other => Err(Error::Projection(format!(
            "`{other}` is not a document kind — spreadsheet, text or presentation"
        ))),
    }
}

/// A `kdl` parse failure as one line with a line and column in it, rather than `miette`'s terminal
/// rendering.
fn diagnostic(text: &str, error: &kdl::KdlError) -> String {
    let Some(first) = error.diagnostics.first() else {
        return "not a projection".to_owned();
    };
    let offset = first.span.offset().min(text.len());
    let line = text[..offset].matches('\n').count() + 1;
    let column = offset - text[..offset].rfind('\n').map_or(0, |i| i + 1) + 1;
    let what = first
        .message
        .as_deref()
        .or(first.help.as_deref())
        .unwrap_or("cannot parse");
    format!("line {line}, column {column}: {what}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_names_the_kind_without_a_parse() {
        assert_eq!(
            header_kind("grind spreadsheet\n\nblock Sales\n"),
            Some(DocumentKind::Spreadsheet)
        );
        assert_eq!(header_kind("grind text\n"), Some(DocumentKind::Text));
        assert_eq!(
            header_kind("grind presentation\n"),
            Some(DocumentKind::Presentation)
        );
    }

    #[test]
    fn a_licence_header_may_come_first() {
        // REUSE-IgnoreStart — the licence tag below is a fixture, not this file's own.
        let source = "// SPDX-License-Identifier: AGPL-3.0-or-later\n\ngrind spreadsheet\n";
        // REUSE-IgnoreEnd
        assert_eq!(header_kind(source), Some(DocumentKind::Spreadsheet));
    }

    #[test]
    fn nothing_else_is_a_projection() {
        for not in [
            "",
            "block Sales\n",
            "grindstone spreadsheet\n",
            "grind ledger\n",
            "<?xml version=\"1.0\"?>",
            "grind\n",
        ] {
            assert_eq!(header_kind(not), None, "{not:?}");
        }
        assert_eq!(is_projection(&[0xff, 0xfe]), None, "not even UTF-8");
    }

    #[test]
    fn parsing_checks_the_header_and_hands_back_the_body() {
        let (kind, body) = parse("grind spreadsheet\n\nblock Sales\n").expect("parses");
        assert_eq!(kind, DocumentKind::Spreadsheet);
        assert_eq!(body.nodes().len(), 1, "the header is not a body node");
        assert_eq!(body.nodes()[0].name().value(), "block");
    }

    #[test]
    fn a_missing_header_is_an_error_rather_than_an_empty_document() {
        // The whole point of §3.3: a tolerant reader handed the wrong bytes must not quietly
        // succeed with nothing in it.
        let error = parse("block Sales\n").expect_err("no header");
        assert!(format!("{error}").contains("grind"), "{error}");
        let error = parse("grind ledger\n").expect_err("no such kind");
        assert!(format!("{error}").contains("ledger"), "{error}");
    }

    #[test]
    fn a_syntax_error_says_where() {
        let error = parse("grind spreadsheet\nblock \"unterminated\n").expect_err("bad syntax");
        assert!(format!("{error}").contains("line 2"), "{error}");
    }
}
