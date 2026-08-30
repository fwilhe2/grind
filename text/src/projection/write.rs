// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Projecting a text document. **\[ODT\]**
//!
//! One pass over the block sequence, one node per block. There is no tree to walk because
//! there is no tree: `model.rs`'s body is flat (rng:16938), and this file is the second place
//! that decision pays — the ODF writer folds depths back into `text:list` nesting, and this one
//! does not have to fold anything at all.
//!
//! What a line looks like is [`super::inline`]'s business; what a *block* looks like is here.

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
    for (index, block) in doc.blocks.iter().enumerate() {
        write_block(&mut out, doc, index, block);
    }
    out.finish()
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

    // An image has no spelling yet (`doc/projection-text.md` names the gap), so it is dropped
    // and the prose around it is not. Doing that here rather than inside `inline::write` keeps
    // that module's answer honest: it says "I cannot spell these runs", and the decision about
    // what to do next is the writer's.
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

    // The block's own `text:style-name`, kept as a name — `doc/text-core.md`'s line between a
    // style that is resolved and one that is carried.
    if let Some(style) = &block.style {
        out.prop("style", style.as_str());
    }

    // Every address this block answers to, so a code view's go-to box and the CLI's `--anchors`
    // speak `loc.rs`'s whole vocabulary rather than the one spelling this writer found easiest.
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
