// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading a projection back into a text document. **\[ODT\]**
//!
//! The twin of `odf::read` and, exactly as on the spreadsheet's side, **not** tolerant in the
//! same way: `odf::read` ignores what it does not recognise because R5 says other people's
//! files have to load, and a projection is a format this project writes, so an unknown node is
//! a typo in something a person hand-wrote and saying so with an offset is the kindness.
//!
//! Where it *is* lenient is where a human is typing:
//!
//! * `li "item"` with no depth is depth 1, which is what a list item usually is;
//! * a `list { … }` block is accepted and its items take their depth from how deep the nesting
//!   is — the second spelling of the same state, the way `at`/`row` and `cell` are two
//!   spellings on the sheet's side. The writer emits the flat form, because the model is flat
//!   and a depth is a number rather than a shape (`model.rs`, rng:16938).

use grind_core::projection::kdl::{KdlDocument, KdlNode, KdlValue};
use grind_core::projection::{Shape, Source, node_span};

use crate::model::{Block, BlockKind, Document};
use crate::{Error, Result, loc};

/// Read a projection.
///
/// The `Source` built alongside is R6 for this form (D5): the text as it came in, and where every
/// block sits in it, so that typing into one paragraph rewrites one line and leaves the comments,
/// the blank lines and the `list` nesting somebody wrote by hand alone.
pub fn read(text: &str) -> Result<Document> {
    let (kind, body) = grind_core::projection::parse(text)?;
    if kind != grind_core::DocumentKind::Text {
        return Err(grind_core::Error::UnsupportedKind(Some(kind)));
    }
    let mut doc = Document::new();
    let mut source = Source::new(text);
    blocks(&mut doc, body.nodes(), 0, &mut source)?;
    doc.reindex_bookmarks();
    doc.projection_source = Some(Box::new(source));
    Ok(doc)
}

/// One level of nodes. `depth` is how many `list` blocks are open around them, which is zero
/// everywhere except inside the authoring spelling.
fn blocks(doc: &mut Document, nodes: &[KdlNode], depth: u32, source: &mut Source) -> Result<()> {
    for node in nodes {
        let kind = match node.name().value() {
            "p" => BlockKind::Paragraph,
            "h" => BlockKind::Heading {
                level: number(node, 0)?,
            },
            "li" => BlockKind::ListItem {
                // Its own depth if it states one; otherwise how deep the nesting is, and 1 for a
                // `li` that is neither in a list nor numbered.
                depth: number(node, 0).unwrap_or(depth.max(1)),
            },
            "list" => {
                blocks(doc, children(node), depth + 1, source)?;
                continue;
            }
            other => return Err(unknown(node, other)),
        };

        // The whole node, not just its string: a block *is* one node, so everything an edit can
        // change about it — its text, its level, its style name — is inside this span, and the
        // `kdl` span stops before the indentation and the newline. One block is one line, so one
        // keystroke is one line of `git diff`, which is the property `text/src/odf/source.rs`
        // already gives a `.fodt` and D5 gives the third form.
        source.record(loc::format(doc.blocks.len()), node_span(node), Shape::Node);

        let id = doc.next_id();
        let mut block = Block::new(id, kind);
        if let Some((text, raw)) = string_of(node) {
            block.runs = super::inline::read(&text, raw);
        }
        if let Some(style) = node.get("style").and_then(KdlValue::as_string) {
            block.style = Some(style.to_owned());
        }
        doc.blocks.push(block);
    }
    Ok(())
}

/// The block's text: its last string argument, and **whether it was written raw**.
///
/// The one place in this project where how a value was *spelled* carries meaning rather than
/// only how it reads — `#"…"#` turns the inline notation off (`doc/dsl.md` §3.5). `kdl-rs`
/// keeps the original representation beside the decoded value, which is the same property R6
/// is built on and the reason that crate was chosen.
fn string_of(node: &KdlNode) -> Option<(String, bool)> {
    let entry = node.entries().iter().rfind(|e| e.name().is_none())?;
    let text = entry.value().as_string()?;
    let raw = entry
        .format()
        .is_some_and(|format| format.value_repr.trim_start().starts_with('#'));
    Some((text.to_owned(), raw))
}

/// A small non-negative integer argument — an outline level, a list depth.
fn number(node: &KdlNode, index: usize) -> Result<u32> {
    match node
        .entries()
        .iter()
        .filter(|e| e.name().is_none())
        .nth(index)
        .map(|e| e.value())
    {
        Some(KdlValue::Integer(n)) => u32::try_from(*n).map_err(|_| {
            at(
                node,
                format!("{n} is not a level or a depth this build has"),
            )
        }),
        _ => Err(at(
            node,
            format!("needs a number as argument {}", index + 1),
        )),
    }
}

fn children(node: &KdlNode) -> &[KdlNode] {
    node.children().map_or(&[], KdlDocument::nodes)
}

fn at(node: &KdlNode, message: String) -> Error {
    Error::Projection(format!(
        "`{}` at offset {}: {message}",
        node.name().value(),
        node.span().offset()
    ))
}

fn unknown(node: &KdlNode, name: &str) -> Error {
    at(
        node,
        format!("`{name}` is not something a text document holds"),
    )
}
