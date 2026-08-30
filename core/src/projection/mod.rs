// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The projection — a document as plain text (`doc/dsl.md` layer 0). **\[GENERIC\]**
//!
//! What is here is the *container*: the KDL syntax, the kind header, and the two maps a code
//! view is made of. **No node name of either application appears in this module**, which is
//! R8 applied to a third serialisation — `grind_sheet::projection` spells `sheet`, `cell` and
//! `row`, `grind_text::projection` spells `p`, `h` and `li`, and neither knows about the
//! other. `doc/dsl.md` §3.2 is that decision and why a single `grind-projection` crate would
//! have been the wrong shape.
//!
//! Three things this module owns, in the order they matter:
//!
//! 1. **The header** (§3.3). A projection states its own kind in its first node — `grind
//!    spreadsheet` — for exactly the reason a `.fods` carries `office:mimetype`: reading is
//!    tolerant by construction, so a reader handed the wrong document type returns an empty
//!    document rather than an error. Inferring the kind from the first *body* node would put
//!    document vocabulary in this crate, so the file says it instead.
//! 2. **The token map** (§6.1). [`Emitter`] knows what every byte it writes *is*, so it
//!    records that as it goes and a shell colours the result. A highlighter would re-derive
//!    the same thing with regexes and be wrong at the edges — the argument
//!    [`crate::layout`] and `formula::display` already make, one layer up.
//! 3. **The span map** (§6.2). Each [`Anchor`] ties an address — an opaque string here,
//!    because an address is the *application's* vocabulary — to the byte range that projects
//!    it. Both directions: [`Projection::span_of`] for *select a cell, highlight its line*,
//!    and [`Projection::address_at`] for the reverse.
//!
//! 4. **The retained text** (§3.1, D5). [`Source`] is R6 for the third form: the bytes a
//!    document was read from, and where each address sits in them, so that saving edits the
//!    file instead of replacing it. The same retain-and-splice trick `odf/source.rs` uses, and
//!    the reason a `.grind` may hold comments and hand alignment at all.
//!
//! Reading is [`kdl`]'s, re-exported below so every crate in the workspace parses with one
//! version of it. What this module adds on the way in is [`parse`], which checks the header
//! and hands back the body — after which an application walks nodes it named itself.

use std::ops::Range;

use crate::{DocumentKind, Error, Result};

pub mod emit;
pub mod source;

pub use emit::Emitter;
pub use source::{Shape, Site, Source, entry_span, node_span};

/// The KDL implementation, re-exported.
///
/// An application's projection reader walks a [`kdl::KdlDocument`] directly — there is no
/// wrapper here, because a wrapper over a document model that is already the right shape is
/// just a second vocabulary to keep in sync. What it must not do is depend on `kdl` itself:
/// two versions in one workspace would be two `KdlValue` types that do not convert.
pub use kdl;

use kdl::{KdlDocument, KdlNode};

/// The first word of a projection: `grind spreadsheet`.
///
/// A magic number that reads as English, which is the whole ambition of the format. It is
/// also what makes [`is_projection`] cheap — no parse of the body is required to answer
/// "which document type is this", and `kind.rs` is a sniff rather than a reader for a reason.
pub const KEYWORD: &str = "grind";

/// The extension a projection takes on disk.
pub const EXTENSION: &str = "grind";

/// What a byte range of a projection *is*, for a shell that colours it.
///
/// Deliberately coarse. These are the distinctions a reader of the text makes — this is a
/// node, that is a value, this is prose — and not a Rust-shaped enumeration of KDL's grammar:
/// a shell picks a colour per variant, and a variant nobody would colour differently is a
/// variant that only costs a match arm in four shells.
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
    /// A `//` comment, which is the one thing in a projection that means nothing to the
    /// document (§3.1: they survive an edit because the writer never regenerates what nobody
    /// touched).
    Comment,
}

impl TokenKind {
    /// The one word each kind is called, for a CLI that prints the token map and a shell that
    /// keys a colour off it.
    ///
    /// In the core for `view::CellRole::marker`'s reason: four shells naming these separately
    /// is four vocabularies for one thing, and the first person to compare two of them finds
    /// they disagree.
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

/// One entry of the span map: an address, and the text that projects it.
///
/// The address is a `String` and not a type, and that is R8 rather than laziness — `Sheet1.B5`
/// and `#intro+5` are two applications' spellings of a place, and this crate is not allowed to
/// know either. What it can do is *carry* one, which is all a code view needs: it hands the
/// string straight back to the `App` that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub address: String,
    pub span: Range<usize>,
}

/// One stretch of a line of the projection, and what it is.
///
/// `kind` is `None` for what the writer never called anything — the indentation, the braces, the
/// spaces between values. A shell paints those in its ordinary text colour, and it must paint
/// them, because the pieces of a line are the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece<'a> {
    pub kind: Option<TokenKind>,
    pub text: &'a str,
}

/// A projected document: the text, and the two maps beside it.
///
/// Produced by [`Emitter`], which is to say by an application's projection *writer* — the maps
/// are bookkeeping done while emitting rather than a second pass over the result, because the
/// writer is the only thing that ever knows what a byte it just wrote meant.
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

    // --- the code view (D9, §6) ---
    //
    // A code view is a widget that shows lines, so what it needs from here is *lines*: how many,
    // what is on one, and which address a place on one belongs to. All four of these could be
    // written in a shell out of `text()` and `tokens()`; the reason they are not is that they
    // would then be written four times, and the first person to compare two of them would find
    // they disagree about a tab or an empty line. Same argument as `CellRole::marker` and
    // `layout`: what a shell contributes is the drawing.

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

    /// A line cut into pieces, in order, covering every byte of it exactly once.
    ///
    /// The uncoloured stretches between tokens — indentation, the `{` and `}` of a block — come
    /// back as pieces with no kind rather than being dropped, because a shell concatenating what
    /// it is given has to get the line back. That is the property `every_piece_of_every_line`
    /// asserts, and it is what makes this safe to render blindly.
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

    /// What address a whole line belongs to — the coarse half of *put the caret there, select
    /// that cell*, for a shell whose code cursor is a line rather than a character.
    ///
    /// Two rules, in order, and the second is the fallback rather than the answer:
    ///
    /// 1. **An anchor that fits inside the line wins, leftmost first.** A grid row is one line
    ///    and a dozen anchors — one per cell — and the useful answer for *this line* is the cell
    ///    it starts with. Picking the narrowest instead would answer with whichever cell happened
    ///    to have the shortest value on it, which is arbitrary and looks like a bug.
    /// 2. **Otherwise the narrowest anchor overlapping it**, which is how a line inside a block
    ///    reports the block, and how a node's own line reports that node — a node's anchor runs
    ///    to the end of its line *including* the newline, so it never "fits".
    ///
    /// Ties go to whichever was emitted first, which for a text block is `p12` rather than
    /// `#intro` or `§2.1.3`: all three name it and one of them has to be the one shown, so it is
    /// the one every block has.
    ///
    /// A shell with a column to offer should use [`Projection::address_at`] on
    /// [`Projection::byte_at`] instead, which is exact rather than per line.
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

    /// The byte offset of a column on a line, clamped to the line's end.
    ///
    /// `column` is a **byte** offset into the line, which is what a shell that has already cut
    /// the line into pieces is holding. Clamped rather than optional because a caret past the end
    /// of a line is an ordinary thing for an editor to have.
    pub fn byte_at(&self, line: usize, column: usize) -> Option<usize> {
        let span = self.line_span(line)?;
        Some((span.start + column).min(span.end))
    }
}

/// The kind a projection declares, from its header alone.
///
/// A scan rather than a parse, and on purpose: [`crate::kind()`] has to answer for *any* pile of
/// bytes, including bytes that are not a projection and would fail to parse. Blank lines and
/// `//` comments are skipped, so a file may open with a copyright header the way every source
/// file in this repository does.
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

/// Whether these bytes are a projection, and of what.
///
/// UTF-8 is required rather than sniffed for: the projection is a format this project writes,
/// and it writes UTF-8. A file that is not valid UTF-8 is not one of ours.
pub fn is_projection(bytes: &[u8]) -> Option<DocumentKind> {
    header_kind(std::str::from_utf8(bytes).ok()?)
}

/// Parse a projection: check the header, hand back the kind and the body nodes.
///
/// The header node is *removed* from the document that comes back, so an application's reader
/// walks body nodes only and never has to know the header exists. That is the same seam
/// [`crate::odf::package::content_xml`] draws — the container is this crate's problem, and what
/// is inside it is not.
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

/// A `kdl` parse failure as one line with a line and column in it.
///
/// `KdlError` carries `miette` diagnostics whose rendering wants a terminal; this project's
/// errors are one `Display` line that a CLI, a GUI banner and a test assertion all read. So
/// the position is recovered from the span and the message kept short.
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
