// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writing a projection, and recording what was written. **\[GENERIC\]**
//!
//! An application's projection writer drives this and nothing else — it decides *what* nodes a
//! document is spelled as, and this decides how a node reaches the page. The split is the same
//! one `odf::write` and `odf::names` have, and it buys the two maps for free: the writer is
//! already saying "this is a node name, that is a cell's value", so [`Emitter`] writes both the
//! byte and the fact.
//!
//! **Values are formatted by [`kdl::KdlValue`]'s own `Display`, never by hand.** The reader is
//! `kdl`'s parser, so the writer has to be `kdl`'s printer or the two eventually disagree about
//! one string in one corpus document. That is also where the format's one pleasant surprise
//! comes from: a string that is already a bare identifier is emitted bare, so `row North 4200`
//! is what a sheet of names and numbers actually looks like, and it parses back as a string.

use std::ops::Range;

use kdl::KdlValue;

use super::{Anchor, KEYWORD, Projection, Token, TokenKind};
use crate::DocumentKind;

/// How far one level of nesting indents.
const INDENT: &str = "    ";

/// A node that has been opened and not yet closed: where it started, and the address it
/// anchors, so that [`Emitter::close`] can finish the span the whole block covers.
struct Frame {
    start: usize,
    anchor: Option<String>,
}

/// Builds a [`Projection`]: text, tokens and anchors in one pass.
///
/// The shape is a cursor rather than a tree builder, because a writer walking a document is
/// already a cursor: [`Emitter::begin`] starts a line, arguments and properties go on it, and
/// [`Emitter::end`] or [`Emitter::open`] finishes it. A tree would mean building a second copy
/// of the document in memory to serialise it, which is what `odf::write` deliberately does not
/// do either.
#[derive(Default)]
pub struct Emitter {
    text: String,
    tokens: Vec<Token>,
    anchors: Vec<Anchor>,
    /// Open blocks, innermost last.
    frames: Vec<Frame>,
    /// The node currently being written on this line, if any.
    line: Option<Frame>,
}

impl Emitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// The kind header (`doc/dsl.md` §3.3) — always the first thing in the file.
    pub fn header(&mut self, kind: DocumentKind) {
        self.begin(KEYWORD);
        self.arg_word(kind.label_word());
        self.end();
    }

    /// A blank line, for the shape of the thing. Never two in a row.
    pub fn blank(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with("\n\n") {
            self.text.push('\n');
        }
    }

    /// A `//` comment on its own line.
    pub fn comment(&mut self, text: &str) {
        self.indent();
        let start = self.text.len();
        self.text.push_str("// ");
        self.text.push_str(text);
        self.token(TokenKind::Comment, start);
        self.text.push('\n');
    }

    /// Start a node. Everything until [`Emitter::end`] or [`Emitter::open`] belongs to it.
    pub fn begin(&mut self, name: &str) {
        debug_assert!(self.line.is_none(), "a node is already open");
        self.indent();
        let start = self.text.len();
        self.text
            .push_str(&KdlValue::String(name.to_owned()).to_string());
        self.token(TokenKind::Node, start);
        self.line = Some(Frame {
            start,
            anchor: None,
        });
    }

    /// Tie the node being written to an address (`doc/dsl.md` §6.2).
    ///
    /// The span is settled when the node ends: a leaf anchors its own line, and a node with
    /// children anchors the whole block, which is what makes *click the sheet, highlight the
    /// sheet* work at the same time as *click the cell, highlight the line*.
    pub fn anchor(&mut self, address: impl Into<String>) {
        if let Some(line) = self.line.as_mut() {
            line.anchor = Some(address.into());
        }
    }

    /// Tie the argument just written to an address of its own.
    ///
    /// [`Emitter::anchor`] covers a node; this covers one *value* on it, which is what a grid
    /// row needs — a row of twelve cells is one line and twelve places, and a code view that
    /// could only say "row 7" would be the coarser half of §6.2's promise.
    pub fn anchor_last(&mut self, address: impl Into<String>) {
        if let Some(token) = self.tokens.last() {
            self.anchors.push(Anchor {
                address: address.into(),
                span: token.span.clone(),
            });
        }
    }

    /// An argument, as whatever KDL value it is.
    pub fn arg(&mut self, value: impl Into<KdlValue>) {
        let value = value.into();
        self.space();
        let start = self.text.len();
        self.text.push_str(&value.to_string());
        self.token(kind_of(&value), start);
    }

    /// An argument that is a bare word rather than data — a kind name, a range, an address.
    ///
    /// The same bytes a string argument would produce when it happens to be a bare identifier,
    /// and a different *token*: `sheet Sales` and `format B2:C5 currency` both read as strings,
    /// but only one of them is a value somebody typed into a cell.
    pub fn arg_word(&mut self, word: &str) {
        self.space();
        let start = self.text.len();
        self.text
            .push_str(&KdlValue::String(word.to_owned()).to_string());
        self.token(TokenKind::Keyword, start);
    }

    /// A `name=value` property.
    pub fn prop(&mut self, name: &str, value: impl Into<KdlValue>) {
        let value = value.into();
        self.space();
        let start = self.text.len();
        self.text
            .push_str(&KdlValue::String(name.to_owned()).to_string());
        self.text.push('=');
        self.token(TokenKind::Property, start);
        let start = self.text.len();
        self.text.push_str(&value.to_string());
        self.token(kind_of(&value), start);
    }

    /// The same as [`Emitter::prop`], skipped when the value is absent — which is most of what
    /// a style or a format writer does, every field being an `Option`.
    pub fn prop_some(&mut self, name: &str, value: Option<impl Into<KdlValue>>) {
        if let Some(value) = value {
            self.prop(name, value);
        }
    }

    /// Finish a leaf node.
    pub fn end(&mut self) {
        let line = self.line.take().expect("a node was begun");
        self.text.push('\n');
        self.close_anchor(line, self.text.len());
    }

    /// Finish a node's own line and open its block of children.
    pub fn open(&mut self) {
        let line = self.line.take().expect("a node was begun");
        self.text.push_str(" {\n");
        self.frames.push(line);
    }

    /// Close the innermost block.
    pub fn close(&mut self) {
        let frame = self.frames.pop().expect("a block was opened");
        self.indent();
        self.text.push_str("}\n");
        self.close_anchor(frame, self.text.len());
    }

    /// The finished projection.
    pub fn finish(self) -> Projection {
        debug_assert!(
            self.line.is_none() && self.frames.is_empty(),
            "unclosed node"
        );
        Projection {
            text: self.text,
            tokens: self.tokens,
            anchors: self.anchors,
        }
    }

    fn close_anchor(&mut self, frame: Frame, end: usize) {
        if let Some(address) = frame.anchor {
            self.anchors.push(Anchor {
                address,
                span: frame.start..end,
            });
        }
    }

    fn indent(&mut self) {
        for _ in 0..self.frames.len() {
            self.text.push_str(INDENT);
        }
    }

    fn space(&mut self) {
        debug_assert!(self.line.is_some(), "an argument outside a node");
        self.text.push(' ');
    }

    fn token(&mut self, kind: TokenKind, start: usize) {
        self.tokens.push(Token {
            kind,
            span: Range {
                start,
                end: self.text.len(),
            },
        });
    }
}

/// Which token a value is. Strings that print bare are still text: how a value is *spelled* is
/// the printer's business, and what it *is* is what a shell colours.
fn kind_of(value: &KdlValue) -> TokenKind {
    match value {
        KdlValue::String(_) => TokenKind::Text,
        KdlValue::Integer(_) | KdlValue::Float(_) => TokenKind::Number,
        KdlValue::Bool(_) | KdlValue::Null => TokenKind::Keyword,
    }
}

impl DocumentKind {
    /// The one word a projection header spells this kind as (`doc/dsl.md` §3.3).
    ///
    /// Not [`DocumentKind::label`], which is prose for a banner and says "text document". This
    /// is an identifier in a file format and has to stay exactly as short as it is.
    pub fn label_word(self) -> &'static str {
        match self {
            DocumentKind::Spreadsheet => "spreadsheet",
            DocumentKind::Text => "text",
            DocumentKind::Presentation => "presentation",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::{header_kind, parse};

    fn sample() -> Projection {
        let mut out = Emitter::new();
        out.header(DocumentKind::Spreadsheet);
        out.blank();
        out.comment("Q3 forecast.");
        out.begin("block");
        out.arg_word("Sales");
        out.anchor("Sales");
        out.open();
        out.begin("leaf");
        out.arg_word("A1");
        out.arg("Region");
        out.arg(4200.0);
        out.prop("bold", true);
        out.anchor("Sales.A1");
        out.end();
        out.close();
        out.finish()
    }

    #[test]
    fn what_it_writes_is_what_kdl_reads_back() {
        let projection = sample();
        let (kind, body) = parse(projection.text()).expect("its own output parses");
        assert_eq!(kind, DocumentKind::Spreadsheet);
        assert_eq!(body.nodes().len(), 1);
        let leaf = &body.nodes()[0].children().expect("a block").nodes()[0];
        assert_eq!(leaf.name().value(), "leaf");
        assert_eq!(leaf.entries()[1].value().as_string(), Some("Region"));
        assert_eq!(leaf.entries()[2].value().as_float(), Some(4200.0));
        assert_eq!(leaf.get("bold").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn the_header_is_the_first_line() {
        assert_eq!(
            header_kind(sample().text()),
            Some(DocumentKind::Spreadsheet)
        );
    }

    #[test]
    fn the_span_map_goes_both_ways_and_the_narrower_answer_wins() {
        let projection = sample();
        let cell = projection.span_of("Sales.A1").expect("anchored");
        let block = projection.span_of("Sales").expect("anchored");
        assert!(
            projection.text()[cell.clone()].starts_with("leaf A1"),
            "{:?}",
            &projection.text()[cell.clone()]
        );
        assert!(
            block.start < cell.start && block.end >= cell.end,
            "a block contains the lines under it"
        );
        // A byte inside the cell's line is inside the block too; the cell is the answer.
        assert_eq!(projection.address_at(cell.start + 1), Some("Sales.A1"));
        assert_eq!(projection.address_at(block.start + 1), Some("Sales"));
        assert_eq!(projection.address_at(0), None, "the header anchors nothing");
    }

    #[test]
    fn every_token_spans_text_that_is_really_there() {
        let projection = sample();
        let mut previous = 0;
        for token in projection.tokens() {
            assert!(token.span.start >= previous, "tokens are in order");
            assert!(token.span.end <= projection.text().len());
            assert!(!projection.text()[token.span.clone()].is_empty());
            previous = token.span.end;
        }
        // The one comment is tokenised as one, prose and all.
        let comments: Vec<_> = projection
            .tokens()
            .iter()
            .filter(|t| t.kind == TokenKind::Comment)
            .map(|t| &projection.text()[t.span.clone()])
            .collect();
        assert_eq!(comments, ["// Q3 forecast."]);
    }

    #[test]
    fn a_value_that_would_be_mistaken_for_something_else_is_quoted() {
        let mut out = Emitter::new();
        out.header(DocumentKind::Spreadsheet);
        out.begin("leaf");
        for text in ["true", "4200", "", "two words", "a\"quote", "a\ttab"] {
            out.arg(text);
        }
        out.end();
        let projection = out.finish();
        let (_, body) = parse(projection.text()).expect("parses");
        let read: Vec<_> = body.nodes()[0]
            .entries()
            .iter()
            .map(|e| e.value().as_string().expect("a string").to_owned())
            .collect();
        assert_eq!(
            read,
            ["true", "4200", "", "two words", "a\"quote", "a\ttab"]
        );
    }
}
