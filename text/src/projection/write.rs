// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Projecting a text document: one pass, one node per block, nothing to fold except tables. What a
//! line's text looks like is [`super::inline`]'s.

use grind_core::DocumentKind;
use grind_core::projection::{Emitter, Projection};

use super::inline;
use crate::loc;
use crate::model::{Block, BlockKind, Document, Run};

/// Project a document: the text, and the token and span maps beside it.
pub fn project(doc: &Document) -> Projection {
    let mut out = Emitter::new();
    out.header(DocumentKind::Text);
    out.blank();
    let mut index = 0;
    while index < doc.blocks.len() {
        match doc.blocks[index].cell {
            // An index's generated entries are left out, a named gap (`doc/projection-text.md`).
            None if doc.blocks[index].generated => index += 1,
            None => {
                write_block(&mut out, doc, index, &doc.blocks[index]);
                index += 1;
            }
            // A table folds the same run `odf::write` does — `Document::table`.
            Some(_) => {
                let range = doc.table(index).unwrap_or(index..index + 1);
                write_table(&mut out, doc, range.clone());
                index = range.end;
            }
        }
    }
    out.finish()
}

/// One `table` node: rows of cells, each holding its blocks. A row's position and a cell's
/// preceding columns give the coordinates, so a hand-written table needs no numbers, and a merged
/// cell moves later ones along with `span=`. Cells are written row by row.
fn write_table(out: &mut Emitter, doc: &Document, range: std::ops::Range<usize>) {
    let Some(name) = doc.blocks[range.start]
        .cell
        .as_ref()
        .map(|cell| cell.table.clone())
    else {
        return;
    };
    let (rows, columns) = doc.table_extent(range.clone());
    out.begin("table");
    out.arg_string(&name);
    out.open();
    for row in 0..rows.max(1) {
        out.begin("row");
        out.open();
        let mut column = 0;
        while column < columns.max(1) {
            let at = range.clone().find(|index| {
                doc.blocks[*index]
                    .cell
                    .as_ref()
                    .is_some_and(|c| c.row == row && c.column == column)
            });
            let Some(at) = at else {
                // A position no block names: covered by a span (whose `span=` moves the reader's
                // counter past it) or a hole, written as an empty cell either way.
                if !covered(doc, range.clone(), row, column) {
                    out.begin("cell");
                    out.end();
                }
                column += 1;
                continue;
            };
            let cell = doc.blocks[at].cell.clone().expect("found by its cell");
            out.begin("cell");
            if cell.columns_spanned > 1 {
                out.prop("span", i128::from(cell.columns_spanned));
            }
            if cell.rows_spanned > 1 {
                out.prop("rows", i128::from(cell.rows_spanned));
            }
            let content: Vec<usize> = range
                .clone()
                .filter(|index| {
                    doc.blocks[*index]
                        .cell
                        .as_ref()
                        .is_some_and(|c| c.is_same(&cell))
                })
                .collect();
            match content.is_empty() {
                true => out.end(),
                false => {
                    out.open();
                    for index in content {
                        write_block(out, doc, index, &doc.blocks[index]);
                    }
                    out.close();
                }
            }
            column += cell.columns_spanned.max(1);
        }
        out.close();
    }
    out.close();
}

/// Whether a cell's span reaches over this position, which then gets no node.
fn covered(doc: &Document, range: std::ops::Range<usize>, row: u32, column: u32) -> bool {
    doc.blocks[range]
        .iter()
        .filter_map(|b| b.cell.as_ref())
        .any(|cell| {
            (cell.columns_spanned > 1 || cell.rows_spanned > 1)
                && row >= cell.row
                && row < cell.row + cell.rows_spanned.max(1)
                && column >= cell.column
                && column < cell.column + cell.columns_spanned.max(1)
                && !(row == cell.row && column == cell.column)
        })
}

/// The projection this document was read from, with edited blocks put back in place (R6). `None`
/// means regenerate: the sequence moved, or an edited block sits nowhere the file spelled. A block
/// is one node, so an edit is one patch; an image is dropped either way, having no spelling.
pub fn splice(doc: &Document) -> Option<String> {
    let source = doc.projection_source.as_deref()?;
    // The sequence moved, so `p12` in the file is no longer `p12` in the model.
    if doc.edits.structural {
        return None;
    }
    let mut patches = Vec::with_capacity(doc.edits.blocks.len());
    for (index, block) in doc.blocks.iter().enumerate() {
        if !doc.edits.blocks.contains(&block.id) {
            continue;
        }
        let site = source.site(&loc::format(index))?;
        let mut out = Emitter::new();
        write_block(&mut out, doc, index, block);
        // The emitter ends a node with a newline; the span it is going into stops before the one
        // already in the file.
        patches.push((
            site.span.clone(),
            out.finish().into_text().trim_end().to_owned(),
        ));
    }
    source.splice(patches)
}

fn write_block(out: &mut Emitter, doc: &Document, index: usize, block: &Block) {
    match &block.kind {
        BlockKind::Paragraph => out.begin("p"),
        BlockKind::Heading { level } => {
            out.begin("h");
            out.arg(i128::from(*level));
        }
        BlockKind::ListItem { depth } => {
            out.begin("li");
            out.arg(i128::from(*depth));
        }
    }

    // An image has no spelling (`doc/projection-text.md`), so it is dropped and the prose around
    // it kept; deciding that is the writer's, not `inline::write`'s.
    let spelled = inline::write(&block.runs).or_else(|| {
        let without: Vec<Run> = block
            .runs
            .iter()
            .filter(|run| !matches!(run, Run::Image { .. }))
            .cloned()
            .collect();
        inline::write(&without)
    });
    if let Some(line) = spelled {
        match line.raw {
            true => out.arg_raw(&line.text),
            false => out.arg_string(&line.text),
        }
    }

    // The block's own `text:style-name`, kept as a name.
    if let Some(style) = &block.style {
        out.prop("style", style.as_str());
    }

    // Every address this block answers to, so a go-to box and `--anchors` speak `loc.rs`'s whole
    // vocabulary.
    out.anchor(loc::format(index));
    if let Some(path) = loc::outline_path(doc, index) {
        out.anchor(loc::format_outline(&path));
    }
    for run in &block.runs {
        if let Run::Bookmark { name } = run {
            out.anchor(format!("#{name}"));
        }
    }
    out.end();
}
