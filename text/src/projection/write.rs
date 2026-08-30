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

/// The projection this document was read from, with the edited blocks put back in place.
///
/// R6 for the third form (D5). `None` means *not applicable, regenerate* and never *failed*, and
/// the two conditions are the same two `odf::write::splice` has, for the same reasons — the
/// sequence must not have moved, and every edited block must sit somewhere the file actually
/// spelled.
///
/// **A block is one node, so a block is one patch.** That is the whole of why the text side
/// needs no equivalent of the spreadsheet's shape check: a paragraph whose runs, level or style
/// changed is still a paragraph node, and re-emitting it is re-emitting one line. The one thing
/// a re-emitted node loses is an image, which this writer has no spelling for either way
/// (`doc/projection-text.md`), so splicing and regenerating drop exactly the same thing.
pub fn splice(doc: &Document) -> Option<String> {
    let source = doc.projection_source.as_deref()?;
    // The block sequence moved, so `p12` in the file and `p12` in the model are not the same
    // paragraph any more — and `p12` is the address every site is keyed by.
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
