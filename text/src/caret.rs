// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a caret goes that is not a question about lines: one character on, one word on, the end
//! of the document, and which caret a click lands on. The vertical motions are [`App::caret_line`]
//! and [`App::caret_line_bounds`].
//!
//! Word motion differs by platform — Ctrl+Right on Windows stops at the start of the next word, ⌥→
//! on a Mac at the end of this one — so each shell keeps its platform's rule; [`word`] is the
//! GNOME window's and the Mac's, over [`crate::word`]'s definition of a word.

use grind_core::layout::Layout;

use crate::flow::Flow;
use crate::{App, Caret};

/// How many characters the block at `index` holds, or zero for a block that is not there.
pub fn block_len(app: &App, index: usize) -> usize {
    app.input_text(index)
        .map(|text| text.chars().count())
        .unwrap_or(0)
}

/// The first caret position in any document.
pub const START: Caret = Caret {
    block: 0,
    offset: 0,
};

/// The last caret position in the document: the end of its last block.
pub fn end(app: &App) -> Caret {
    let block = app.block_count().saturating_sub(1);
    Caret {
        block,
        offset: block_len(app, block),
    }
}

/// One character left (`delta` negative) or right from `at`, rolling onto the neighbouring block
/// at either end, and staying put at the ends of the document.
pub fn step(app: &App, at: Caret, delta: i32) -> Caret {
    let mut caret = at;
    if delta > 0 {
        if caret.offset < block_len(app, caret.block) {
            caret.offset += 1;
        } else if caret.block + 1 < app.block_count() {
            caret = Caret {
                block: caret.block + 1,
                offset: 0,
            };
        }
    } else if caret.offset > 0 {
        caret.offset -= 1;
    } else if caret.block > 0 {
        caret = Caret {
            block: caret.block - 1,
            offset: block_len(app, caret.block - 1),
        };
    }
    caret
}

/// One word right (`delta` positive) or left: the far edge of a word in this block — its end going
/// right, its start going left — or the end of this block, then the near end of the next.
pub fn word(app: &App, at: Caret, delta: i32) -> Caret {
    let text = app.input_text(at.block).unwrap_or_default();
    let len = text.chars().count();
    let within = match delta > 0 {
        true => crate::word::next_end(&text, at.offset),
        false => crate::word::previous_start(&text, at.offset),
    };
    let here = |offset| Caret {
        block: at.block,
        offset,
    };
    match within {
        Some(offset) => here(offset),
        None if delta > 0 && at.offset < len => here(len),
        None if delta < 0 && at.offset > 0 => here(0),
        None => step(app, at, delta),
    }
}

/// Which caret a point lands on: `x` from the text column's left edge, `y` from the document's
/// top, in the unit `flow` was stacked in. `layout_of` lays a block out as it was drawn, through
/// the shell's [`crate::Faces`], so a click and Down-arrow agree. The nearest, never nothing
/// ([`Flow::at`]): beside a line lands at its near end, below the last block on its last line.
/// `None` only when nothing is laid out.
pub fn hit(
    flow: &Flow,
    x: f64,
    y: f64,
    layout_of: impl Fn(usize) -> Option<Layout>,
) -> Option<Caret> {
    let block = flow.at(x, y)?;
    let slot = *flow.slot(block)?;
    let layout = layout_of(block)?;
    let line = layout.line_at_y((y - slot.top) as f32);
    Some(Caret {
        block,
        offset: layout.offset_at(line, (x - slot.indent) as f32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockKind, Fixed, Uniform};

    fn app(blocks: &[&str]) -> App {
        let app = App::new();
        for (index, text) in blocks.iter().enumerate() {
            app.insert(index, BlockKind::Paragraph, text).unwrap();
        }
        // `App::new` starts with one empty paragraph; the blocks above are before it.
        let last = app.block_count() - 1;
        app.delete(last..last + 1).unwrap();
        app
    }

    fn at(block: usize, offset: usize) -> Caret {
        Caret { block, offset }
    }

    #[test]
    fn a_character_step_rolls_across_a_block_boundary_and_stops_at_the_ends() {
        let app = app(&["ab", "", "c"]);
        assert_eq!(step(&app, at(0, 1), 1), at(0, 2));
        assert_eq!(
            step(&app, at(0, 2), 1),
            at(1, 0),
            "off the end, onto the next"
        );
        assert_eq!(
            step(&app, at(1, 0), 1),
            at(2, 0),
            "an empty block is one stop"
        );
        assert_eq!(step(&app, at(2, 1), 1), at(2, 1), "the document's end");
        assert_eq!(step(&app, at(2, 0), -1), at(1, 0));
        assert_eq!(
            step(&app, at(1, 0), -1),
            at(0, 2),
            "onto the previous block's end"
        );
        assert_eq!(step(&app, START, -1), START, "the document's start");
        assert_eq!(end(&app), at(2, 1));
    }

    #[test]
    fn a_step_counts_characters_and_not_bytes() {
        let app = app(&["héllo"]);
        assert_eq!(block_len(&app, 0), 5);
        assert_eq!(step(&app, at(0, 4), 1), at(0, 5));
        assert_eq!(block_len(&app, 9), 0, "no such block");
    }

    /// Right to a word's end, left to its start; then the block's end, then the next block.
    #[test]
    fn a_word_step_goes_to_the_far_edge_then_the_block_edge_then_across() {
        let app = app(&["one two.", "three"]);
        assert_eq!(word(&app, at(0, 0), 1), at(0, 3));
        assert_eq!(word(&app, at(0, 3), 1), at(0, 7));
        assert_eq!(
            word(&app, at(0, 7), 1),
            at(0, 8),
            "the full stop, then the end"
        );
        assert_eq!(word(&app, at(0, 8), 1), at(1, 0), "then the next block");
        assert_eq!(word(&app, at(1, 0), -1), at(0, 8), "and back");
        assert_eq!(word(&app, at(0, 8), -1), at(0, 4));
        assert_eq!(word(&app, at(0, 2), -1), at(0, 0));
        assert_eq!(word(&app, START, -1), START);
    }

    /// The flow's block, then the line, then the offset — and a point outside every block lands
    /// somewhere.
    #[test]
    fn a_click_lands_on_the_nearest_caret() {
        let app = app(&["the cat sat on the mat", "end"]);
        let spacing = crate::flow::Spacing {
            top: 2.0,
            gap: 1.0,
            heading: 0.0,
            indent: 0.0,
            cell_pad: 0.0,
        };
        let faces = Uniform::new(10.0, &Fixed);
        let flow = crate::flow::lay_out(&app, &faces, 10.0, &spacing, &|_, _| None);
        let layout_of = |index| app.layout_block(index, 10.0, &Fixed).ok();
        // The first block's lines are "the cat ", "sat on ", "the mat", from y = 2.
        assert_eq!(hit(&flow, 0.0, 2.0, layout_of), Some(at(0, 0)));
        assert_eq!(hit(&flow, 3.2, 3.5, layout_of), Some(at(0, 11)), "line 2");
        assert_eq!(
            hit(&flow, 99.0, 2.0, layout_of),
            Some(at(0, 8)),
            "past the end"
        );
        assert_eq!(
            hit(&flow, 0.0, -9.0, layout_of),
            Some(at(0, 0)),
            "above it all"
        );
        assert_eq!(
            hit(&flow, 9.0, 99.0, layout_of),
            Some(at(1, 3)),
            "below it all"
        );
        let empty = Flow::default();
        assert_eq!(hit(&empty, 0.0, 0.0, layout_of), None);
    }
}
