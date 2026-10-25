// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layout, each operation defined in terms of a line so that no shell does its own. The engine is
//! `grind_core::layout` (`doc/text-layout.md`). The caret operations take [`Faces`], since a
//! motion by line may end in a block set differently from where it started.

use crate::*;

impl App {
    /// Break one block into lines at `width`, in `metrics`' unit; zero or less means do not wrap.
    /// A plain value with the x of every caret position, for painting and discarding.
    pub fn layout_block(&self, index: usize, width: f32, metrics: &dyn Metrics) -> Result<Layout> {
        let state = self.state.read().unwrap();
        let block = state
            .doc
            .block(index)
            .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(index))))?;
        Ok(lay_out(block, width, metrics, &state.doc.char_styles))
    }

    /// [`App::layout_block`] with the first line `first` further in and breaking that much shorter
    /// (`fo:text-indent`), for a printed page.
    pub fn layout_block_indented(
        &self,
        index: usize,
        width: f32,
        metrics: &dyn Metrics,
        first: f32,
    ) -> Result<Layout> {
        self.layout_block_tabbed(index, width, metrics, first, &layout::Tabs::default())
    }

    /// [`App::layout_block_indented`] with every tab advanced to its stop ([`Faces::tabs`]).
    pub fn layout_block_tabbed(
        &self,
        index: usize,
        width: f32,
        metrics: &dyn Metrics,
        first: f32,
        tabs: &layout::Tabs,
    ) -> Result<Layout> {
        let state = self.state.read().unwrap();
        let block = state
            .doc
            .block(index)
            .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(index))))?;
        Ok(lay_out_indented(
            block,
            width,
            metrics,
            first,
            tabs,
            &state.doc.char_styles,
        ))
    }

    /// One block laid out as if `text` had been typed at `at` — an input method's composition,
    /// before it commits. Nothing is written; on commit it becomes one `type_markdown`. The
    /// composed text takes the formatting typing there would (`resume`, else
    /// [`App::insert_text`]'s rule), and breaks where the committed text will. Offsets count the
    /// composed characters, so the caret inside the composition is `at.offset` plus its position
    /// in `text`.
    pub fn layout_composing(
        &self,
        at: Caret,
        text: &str,
        resume: Option<&CharStyle>,
        width: f32,
        metrics: &dyn Metrics,
    ) -> Result<Layout> {
        let state = self.state.read().unwrap();
        let mut block = block_at(&state, at.block)?;
        if !text.is_empty() {
            let (mut runs, tail) = model::split_runs(&block.runs, at.offset.min(block.len()));
            let (style, props, href) = match resume {
                Some(props) => (None, props.clone(), None),
                None => caret_formatting(&runs, &tail),
            };
            runs.push(Run::Text {
                text: text.to_owned(),
                style,
                props,
                href,
            });
            runs.extend(tail);
            model::coalesce(&mut runs);
            block.runs = runs;
        }
        Ok(lay_out(&block, width, metrics, &state.doc.char_styles))
    }

    /// The x of a caret within its line — the goal column a shell keeps while Down is held, so
    /// walking through a short line returns to the starting column. Passed back into
    /// [`App::caret_line`] rather than stored, and measured through the same [`Faces`], so it
    /// survives a change of face: x is the screen's.
    pub fn caret_x(&self, at: Caret, faces: &dyn Faces) -> Result<f32> {
        let state = self.state.read().unwrap();
        let block = state
            .doc
            .block(at.block)
            .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(at.block))))?;
        Ok(set_out(block, at.block, faces, &state.doc.char_styles).x_at(at.offset))
    }

    /// Move a caret `delta` lines — Down and Up, and Page Down with a bigger number. Crosses block
    /// boundaries, and stops rather than erroring at the top or bottom of the document. The block
    /// it arrives in is measured in its own face ([`Faces`]): Down out of a heading lands in a
    /// paragraph set in another font.
    pub fn caret_line(
        &self,
        at: Caret,
        delta: isize,
        goal_x: f32,
        faces: &dyn Faces,
    ) -> Result<Caret> {
        let state = self.state.read().unwrap();
        let blocks = state.doc.blocks.len();
        let mut block = at.block;
        let mut layout = set_out(
            state
                .doc
                .block(block)
                .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(block))))?,
            block,
            faces,
            &state.doc.char_styles,
        );
        let mut offset = at.offset.min(layout.len());
        let step = if delta < 0 { -1 } else { 1 };

        for _ in 0..delta.unsigned_abs() {
            if let Some(next) = layout.step(offset, step, goal_x) {
                offset = next;
                continue;
            }
            // Off this block's lines: carry into the neighbour, on the line nearest the one left.
            let next = match step > 0 {
                true if block + 1 < blocks => block + 1,
                false if block > 0 => block - 1,
                // The document's own top or bottom. Stay put.
                _ => break,
            };
            block = next;
            layout = set_out(
                &state.doc.blocks[block],
                block,
                faces,
                &state.doc.char_styles,
            );
            let line = match step > 0 {
                true => 0,
                false => layout.lines().len().saturating_sub(1),
            };
            offset = layout.offset_at(line, goal_x);
        }
        Ok(Caret { block, offset })
    }

    /// The two ends of the caret's own visual line — Home and End — as a pair.
    pub fn caret_line_bounds(&self, at: Caret, faces: &dyn Faces) -> Result<(Caret, Caret)> {
        let state = self.state.read().unwrap();
        let block = state
            .doc
            .block(at.block)
            .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(at.block))))?;
        let layout = set_out(block, at.block, faces, &state.doc.char_styles);
        let line = layout.lines()[layout.line_at(at.offset)];
        Ok((
            Caret {
                block: at.block,
                offset: line.start,
            },
            Caret {
                block: at.block,
                offset: line.end,
            },
        ))
    }
}
