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
    /// Every address this node is. Usually one — a cell is a cell — and sometimes several: a
    /// heading is `p12` *and* `§2.1.3`, and both are addresses `loc.rs` resolves, so a span map
    /// that could only hold one would answer half the go-to box's questions.
    anchors: Vec<String>,
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
            anchors: Vec::new(),
        });
    }

    /// Tie the node being written to an address (`doc/dsl.md` §6.2).
    ///
    /// The span is settled when the node ends: a leaf anchors its own line, and a node with
    /// children anchors the whole block, which is what makes *click the sheet, highlight the
    /// sheet* work at the same time as *click the cell, highlight the line*.
    ///
    /// Called more than once, a node is more than one address. That is not a spreadsheet's
    /// shape — a cell is `B5` and nothing else — but it is a text document's: `p12`, `#intro`
    /// and `§2.1.3` can all name the same paragraph, and `loc.rs` resolves all three.
    pub fn anchor(&mut self, address: impl Into<String>) {
        if let Some(line) = self.line.as_mut() {
            line.anchors.push(address.into());
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
        self.text.push_str(&repr(&value));
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

    /// An argument that is always **quoted**, whatever is in it.
    ///
    /// [`Emitter::arg`] prints through `KdlValue`, which drops the quotes whenever a string
    /// happens to be a bare identifier — excellent for `row North 4200`, and wrong for a
    /// paragraph, since KDL's bare identifiers admit `*` and a paragraph beginning `**bold**`
    /// would go to the page naked. Prose is a string and looks like one.
    ///
    /// Still `KdlValue`'s printer, per this module's rule: a value it decides to quote is taken
    /// as it comes, and one it prints bare is *by definition* free of anything an escape covers,
    /// so putting quotes round it is the whole of the difference.
    pub fn arg_string(&mut self, text: &str) {
        self.space();
        let start = self.text.len();
        let printed = KdlValue::String(text.to_owned()).to_string();
        let printed = escape_disallowed(&printed);
        match printed.starts_with('"') {
            true => self.text.push_str(&printed),
            false => {
                self.text.push('"');
                self.text.push_str(&printed);
                self.text.push('"');
            }
        }
        self.token(TokenKind::Text, start);
    }

    /// An argument as one of KDL's **raw** strings — `#"…"#`, where no escape means anything.
    ///
    /// A container-level feature (it is KDL's, not any document type's), and one an application
    /// reaches for when its own notation would otherwise fill a line with backslashes:
    /// `doc/dsl.md` §3.5's paragraph *about* markdown. The caller is responsible for the string
    /// being one a raw form can hold — no `"#` in it — which is why this is `_raw` rather than
    /// something that decides for itself.
    pub fn arg_raw(&mut self, text: &str) {
        // A raw string has no escapes by definition, so a code point KDL will not accept
        // literally cannot be in one — the request falls back to the quoted form, which can
        // spell it. Deciding that here rather than at the call site is the point: which
        // characters KDL admits is the container's business and no document type's.
        if text.chars().any(is_disallowed) {
            return self.arg_string(text);
        }
        self.space();
        let start = self.text.len();
        self.text.push_str("#\"");
        self.text.push_str(text);
        self.text.push_str("\"#");
        self.token(TokenKind::Text, start);
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
        for address in frame.anchors {
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

/// How a value is spelled in a projection — the one function that answers it.
///
/// `KdlValue`'s own `Display`, per this module's rule, with the escaping the container has to add
/// on top — the code points KDL will not see literally. It is public because the *splice* needs
/// the same answer as the writer: `source.rs` replaces one value in the retained text, and a
/// value spelled two ways by two functions is a bijection that holds until the first file that
/// exercises the difference.
pub fn repr(value: &KdlValue) -> String {
    escape_disallowed(&value.to_string())
}

/// The code points KDL refuses to see *literally* in a document — the C0 controls it does not
/// have an escape for, the Unicode direction controls, and the byte-order mark
/// (`disallowed-literal-code-points`, and the reason for it is Trojan Source).
///
/// This exists because `kdl`'s own printer emits them raw, so its output does not always parse:
/// one document in LibreOffice's Writer corpus carries a U+200E, and projecting it produced a
/// file `kdl` itself rejected. Escaping them here is the narrowest possible fix and keeps this
/// module's rule intact — the printer still decides how a string is spelled, and this only
/// rewrites the characters it may not leave standing.
fn is_disallowed(c: char) -> bool {
    matches!(c,
        '\u{0000}'..='\u{0008}'
        | '\u{000E}'..='\u{001F}'
        | '\u{200E}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2066}'..='\u{2069}'
        | '\u{FEFF}'
    )
}

/// Rewrite each of those as the `\u{…}` escape KDL does accept.
fn escape_disallowed(printed: &str) -> String {
    if !printed.chars().any(is_disallowed) {
        return printed.to_owned();
    }
    printed
        .chars()
        .map(|c| match is_disallowed(c) {
            true => format!("\\u{{{:x}}}", c as u32),
            false => c.to_string(),
        })
        .collect()
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

    /// **D9's floor.** Whatever a shell does with the pieces of a line, concatenating them has to
    /// give the line back — otherwise a code view silently drops a brace or an indent and nobody
    /// notices until a screenshot.
    #[test]
    fn every_piece_of_every_line_puts_the_line_back_together() {
        let projection = sample();
        assert_eq!(
            projection.line_count(),
            projection.text().lines().count(),
            "the header, the blank, the comment, the block and its close"
        );
        for line in 0..projection.line_count() {
            let span = projection.line_span(line).expect("a line in range");
            let pieces = projection.line_pieces(line);
            assert_eq!(
                pieces.iter().map(|p| p.text).collect::<String>(),
                projection.text()[span],
                "line {line} does not survive being cut into pieces"
            );
        }
        assert_eq!(projection.line_span(99), None);
        assert!(projection.line_pieces(99).is_empty());
    }

    /// A blank line has one empty piece or none, and either way is still a line: an off-by-one
    /// here is a code view that scrolls out of step with the document it is showing.
    #[test]
    fn a_blank_line_is_a_line() {
        let projection = sample();
        let blank = projection
            .text()
            .lines()
            .position(str::is_empty)
            .expect("`sample` has one");
        assert_eq!(projection.line_span(blank), Some(18..18));
        assert!(
            projection
                .line_pieces(blank)
                .iter()
                .all(|p| p.text.is_empty()),
            "nothing on it"
        );
    }

    /// The uncoloured stretches are pieces too, and the coloured ones say what they are.
    #[test]
    fn a_line_says_which_of_its_pieces_are_what() {
        let projection = sample();
        let line = projection
            .text()
            .lines()
            .position(|l| l.contains("leaf"))
            .expect("`sample` has one");
        let pieces = projection.line_pieces(line);
        let named: Vec<_> = pieces
            .iter()
            .map(|p| (p.kind.map(TokenKind::name), p.text))
            .collect();
        assert_eq!(
            named,
            [
                (None, "    "),
                (Some("node"), "leaf"),
                (None, " "),
                (Some("keyword"), "A1"),
                (None, " "),
                (Some("text"), "Region"),
                (None, " "),
                (Some("number"), "4200.0"),
                (None, " "),
                (Some("property"), "bold="),
                (Some("keyword"), "#true"),
            ],
            "the indentation is a piece, and so is every space between values"
        );
    }

    /// The span map, asked line by line — which is how a code view whose cursor is a whole line
    /// answers *what am I looking at*.
    #[test]
    fn a_line_reports_the_address_it_belongs_to() {
        let projection = sample();
        let leaf = projection.line_of("Sales.A1").expect("anchored");
        assert_eq!(projection.address_on_line(leaf), Some("Sales.A1"));
        // The block's opening line belongs to the block: the leaf's anchor does not reach it.
        let block = projection.line_of("Sales").expect("anchored");
        assert_eq!(projection.address_on_line(block), Some("Sales"));
        assert_eq!(
            projection.address_on_line(0),
            None,
            "the header is nobody's"
        );

        // And the exact half, through a column on a line.
        let byte = projection.byte_at(leaf, 4).expect("a column on that line");
        assert_eq!(projection.address_at(byte), Some("Sales.A1"));
        let past = projection
            .byte_at(leaf, 9_999)
            .expect("clamped, not refused");
        assert_eq!(past, projection.line_span(leaf).expect("a line").end);
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
