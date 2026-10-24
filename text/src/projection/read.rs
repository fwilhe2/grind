// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading a projection back into a text document. **\[ODT\]** Not tolerant the way `odf::read`
//! is: an unknown node is a typo, reported with its offset. Lenient where a person types:
//!
//! * `li "item"` with no depth is depth 1;
//! * a `list { … }` block is accepted, its items' depth taken from the nesting — a second spelling
//!   of the same state; the writer emits the flat form.

use grind_core::projection::kdl::{KdlDocument, KdlNode, KdlValue};
use grind_core::projection::{Shape, Source, node_span};

use crate::model::{Block, BlockKind, Cell, Document};
use crate::{Error, Result, loc};

/// Read a projection, with a `Source` beside it: the text, and where every block sits, so typing
/// into a paragraph rewrites one line and keeps comments, blank lines and hand-written `list`
/// nesting.
pub fn read(text: &str) -> Result<Document> {
    let (kind, body) = grind_core::projection::parse(text)?;
    if kind != grind_core::DocumentKind::Text {
        return Err(grind_core::Error::UnsupportedKind(Some(kind)));
    }
    // Every block comes from the projection, including none (`Document::empty`).
    let mut doc = Document::empty();
    let mut source = Source::new(text);
    blocks(&mut doc, body.nodes(), 0, None, &mut source)?;
    doc.reindex_bookmarks();
    doc.projection_source = Some(Box::new(source));
    Ok(doc)
}

/// One level of nodes. `depth` is how many `list` blocks are open around them; `cell` is the table
/// cell they are in, if any.
fn blocks(
    doc: &mut Document,
    nodes: &[KdlNode],
    depth: u32,
    cell: Option<&Cell>,
    source: &mut Source,
) -> Result<()> {
    for node in nodes {
        let kind = match node.name().value() {
            "p" => BlockKind::Paragraph,
            "h" => BlockKind::Heading {
                level: number(node, 0)?,
            },
            "li" => BlockKind::ListItem {
                // Its own depth if stated, else the nesting depth, and 1 for a bare `li`.
                depth: number(node, 0).unwrap_or(depth.max(1)),
            },
            "list" => {
                blocks(doc, children(node), depth + 1, cell, source)?;
                continue;
            }
            "table" => {
                table(doc, node, source)?;
                continue;
            }
            other => return Err(unknown(node, other)),
        };

        // The whole node: everything an edit can change about a block is in it, and `kdl`'s span
        // excludes indentation and newline, so one keystroke is one line of diff.
        source.record(loc::format(doc.blocks.len()), node_span(node), Shape::Node);

        let id = doc.next_id();
        let mut block = Block::new(id, kind);
        block.cell = cell.cloned();
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

/// One `table` node: rows of cells whose coordinates come from where they sit — a row by position,
/// a cell's column by how many came before it. A `span=` moves the counter on, so covered
/// positions need no node. A table with no name gets one from its position, since a name is its
/// identity (`model::Cell`).
fn table(doc: &mut Document, node: &KdlNode, source: &mut Source) -> Result<()> {
    let name = match node
        .entries()
        .iter()
        .find(|e| e.name().is_none())
        .and_then(|e| e.value().as_string())
    {
        Some(name) => name.to_owned(),
        None => format!("Table{}", doc.blocks.len() + 1),
    };
    // The positions a `rows=` span reaches into, remembered until stepped over — otherwise every
    // cell after a vertically merged one shifts left a column.
    let mut covered: std::collections::BTreeSet<(u32, u32)> = std::collections::BTreeSet::new();
    for (row, child) in children(node).iter().enumerate() {
        let row = row as u32;
        if child.name().value() != "row" {
            return Err(at(
                child,
                format!(
                    "`{}` is not something a table holds — a table holds rows",
                    child.name().value()
                ),
            ));
        }
        let mut column = 0u32;
        for entry in children(child) {
            if entry.name().value() != "cell" {
                return Err(at(
                    entry,
                    format!(
                        "`{}` is not something a row holds — a row holds cells",
                        entry.name().value()
                    ),
                ));
            }
            while covered.contains(&(row, column)) {
                column += 1;
            }
            let cell = Cell {
                table: name.clone(),
                row,
                column,
                columns_spanned: span(entry, "span")?,
                rows_spanned: span(entry, "rows")?,
            };
            for r in cell.row..cell.row + cell.rows_spanned.max(1) {
                for c in cell.column..cell.column + cell.columns_spanned.max(1) {
                    if (r, c) != (cell.row, cell.column) {
                        covered.insert((r, c));
                    }
                }
            }
            column += cell.columns_spanned.max(1);
            blocks(doc, children(entry), 0, Some(&cell), source)?;
        }
    }
    Ok(())
}

/// A `span=` or `rows=` property: how many columns or rows this cell covers; one when unstated.
fn span(node: &KdlNode, name: &str) -> Result<u32> {
    match node.get(name) {
        None => Ok(1),
        Some(KdlValue::Integer(n)) => u32::try_from(*n)
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| at(node, format!("{n} is not a span"))),
        Some(_) => Err(at(node, format!("`{name}` needs a number"))),
    }
}

/// The block's text — its last string argument — and whether it was written raw: `#"…"#` turns the
/// notation off (`doc/dsl.md` §3.5), and `kdl-rs` keeps each value's original spelling.
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
