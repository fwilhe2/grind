// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Editing by block: inserting, deleting, moving and retyping blocks, and the tables, pictures
//! and Markdown that arrive as blocks.

use crate::*;

impl App {
    /// Insert an image at a caret (`doc/text-core.md`'s `draw:frame` row): one [`Run::Image`]
    /// occupying one caret position, like a tab, so erasing, splitting and Backspace treat it as a
    /// whole. `mime` is a MIME type; `width` and `height` are optional ODF lengths (`"5cm"`),
    /// verbatim.
    pub fn insert_image(
        &self,
        at: Caret,
        mime: String,
        data: Vec<u8>,
        width: Option<String>,
        height: Option<String>,
    ) -> Result<()> {
        self.mutate(|state| {
            let mut block = block_at(state, at.block)?;
            let offset = at.offset.min(block.len());
            let (mut runs, tail) = model::split_runs(&block.runs, offset);
            runs.push(Run::Image {
                mime,
                data,
                width,
                height,
                // An inserted image is anchored to its paragraph; a document's own anchor is kept
                // verbatim (`Run::Image::anchor`).
                anchor: None,
                title: None,
                description: None,
            });
            runs.extend(tail);
            model::coalesce(&mut runs);
            block.runs = runs;
            Self::commit(
                state,
                Action::SetBlock {
                    index: at.block,
                    block: Box::new(block),
                },
            )
        })
    }

    /// Give the picture just after `at` its alternative text: `title` the short one a screen
    /// reader says and Markdown writes as `![title]`, `description` the long one (`svg:title`,
    /// `svg:desc`). `None` removes either. An error when the next character is not a picture.
    pub fn set_image_alt(
        &self,
        at: Caret,
        title: Option<String>,
        description: Option<String>,
    ) -> Result<()> {
        self.mutate(|state| {
            let mut block = block_at(state, at.block)?;
            let mut start = 0;
            let run = block.runs.iter_mut().find(|run| {
                let here = start;
                start += run.len();
                here == at.offset && matches!(run, Run::Image { .. })
            });
            let Some(Run::Image {
                title: old_title,
                description: old_description,
                ..
            }) = run
            else {
                return Err(Error::Xml(format!(
                    "{}+{} is not in front of a picture",
                    loc::format(at.block),
                    at.offset
                )));
            };
            let empty = |s: Option<String>| s.filter(|s| !s.is_empty());
            *old_title = empty(title);
            *old_description = empty(description);
            Self::commit(
                state,
                Action::SetBlock {
                    index: at.block,
                    block: Box::new(block),
                },
            )
        })
    }

    /// Insert a table before `index` — `rows` by `columns`, each cell holding one empty paragraph
    /// — as one undo step; `index == block_count()` appends. A cell always has a block once it has
    /// been through a file (`doc/odt-format.md` §5b), and a cell without one cannot hold a caret.
    /// `name` is the table's `table:name`, its identity ([`model::Cell`]), generated when `None`.
    pub fn insert_table(
        &self,
        index: usize,
        rows: u32,
        columns: u32,
        name: Option<String>,
    ) -> Result<()> {
        self.mutate(|state| {
            if rows == 0 || columns == 0 {
                return Err(Error::Xml(
                    "a table needs at least one row and one column".to_owned(),
                ));
            }
            if index > state.doc.blocks.len() {
                return Err(Error::Xml(format!(
                    "{} is past the end",
                    loc::format(index)
                )));
            }
            let name = name.unwrap_or_else(|| unused_table_name(&state.doc));
            let mut batch = Vec::with_capacity((rows * columns) as usize);
            let mut at = index;
            for row in 0..rows {
                for column in 0..columns {
                    let id = state.doc.next_id();
                    let mut block = Block::new(id, BlockKind::Paragraph);
                    block.cell = Some(model::Cell::new(name.clone(), row, column));
                    batch.push(Action::InsertBlock {
                        index: at,
                        block: Box::new(block),
                    });
                    at += 1;
                }
            }
            Self::commit(state, Action::Batch(batch))
        })
    }

    /// Insert Markdown's blocks before block `index` ([`commonmark`]), its pictures found through
    /// `resolve` ([`commonmark::beside`], [`commonmark::nowhere`]). One undo step; returns how
    /// many blocks it added.
    pub fn import_markdown(
        &self,
        index: usize,
        markdown: &str,
        resolve: &commonmark::Resolve<'_>,
    ) -> Result<usize> {
        self.mutate(|state| {
            if index > state.doc.blocks.len() {
                return Err(Error::Xml(format!(
                    "{} is past the end",
                    loc::format(index)
                )));
            }
            let blocks = commonmark::parse(markdown, &mut state.doc, resolve);
            let count = blocks.len();
            if count == 0 {
                return Ok(0);
            }
            let batch = blocks
                .into_iter()
                .enumerate()
                .map(|(n, block)| Action::InsertBlock {
                    index: index + n,
                    block: Box::new(block),
                })
                .collect();
            Self::commit(state, Action::Batch(batch))?;
            Ok(count)
        })
    }

    /// The blocks in `range` as CommonMark ([`commonmark::export`]), and the pictures it names,
    /// for the caller to write where `pictures` said.
    pub fn export_markdown(
        &self,
        range: Range<usize>,
        pictures: &commonmark::Pictures,
    ) -> Result<commonmark::Exported> {
        let state = self.state.read().unwrap();
        let blocks = state
            .doc
            .blocks
            .get(range)
            .ok_or_else(|| Error::Xml("that range runs past the end".to_owned()))?;
        Ok(commonmark::export(blocks, pictures))
    }

    /// The table the block at `index` is in — where it starts and ends, its size, its name — or
    /// `None`, derived from the cells (`Document::table_extent`).
    pub fn table(&self, index: usize) -> Option<Table> {
        let state = self.state.read().unwrap();
        let blocks = state.doc.table(index)?;
        let (rows, columns) = state.doc.table_extent(blocks.clone());
        Some(Table {
            name: state.doc.block(blocks.start)?.cell.as_ref()?.table.clone(),
            blocks,
            rows,
            columns,
        })
    }

    /// Insert a block before `index`. `index == block_count()` appends.
    pub fn insert(&self, index: usize, kind: BlockKind, text: &str) -> Result<()> {
        self.mutate(|state| {
            let id = state.doc.next_id();
            let mut block = Block::new(id, kind);
            if !text.is_empty() {
                block.runs.push(Run::plain(text));
            }
            // Inside a section when both neighbours are, or a regenerate would split the section
            // around it.
            let section = |at: Option<usize>| at.and_then(|i| state.doc.blocks.get(i)?.section);
            let before = section(index.checked_sub(1));
            if before.is_some() && before == section(Some(index)) {
                block.section = before;
            }
            Self::commit(
                state,
                Action::InsertBlock {
                    index,
                    block: Box::new(block),
                },
            )
        })
    }

    /// Delete a run of blocks. One [`Action::Batch`], so it is one undo step.
    pub fn delete(&self, blocks: Range<usize>) -> Result<usize> {
        self.mutate(|state| {
            if blocks.end > state.doc.blocks.len() {
                return Err(Error::Xml("that range runs past the end".to_owned()));
            }
            // Removing at the same index repeatedly: each removal shifts the rest down.
            let removed = blocks.len();
            let batch = (0..removed)
                .map(|_| Action::RemoveBlock {
                    index: blocks.start,
                })
                .collect();
            Self::commit(state, Action::Batch(batch))?;
            Ok(removed)
        })
    }

    /// Change what kind of block this is — paragraph, heading or list item. The outline is implied
    /// by nothing else (`doc/odt-format.md` §2).
    pub fn set_kind(&self, index: usize, kind: BlockKind) -> Result<()> {
        self.mutate(|state| {
            let mut block = block_at(state, index)?;
            block.kind = kind;
            Self::commit(
                state,
                Action::SetBlock {
                    index,
                    block: Box::new(block),
                },
            )
        })
    }

    /// Move a run of blocks so it starts at `to`, an index in the document before the move. Moving
    /// a range into itself is refused.
    pub fn move_blocks(&self, blocks: Range<usize>, to: usize) -> Result<usize> {
        self.mutate(|state| {
            if blocks.end > state.doc.blocks.len() || to > state.doc.blocks.len() {
                return Err(Error::Xml("that range runs past the end".to_owned()));
            }
            if to > blocks.start && to < blocks.end {
                return Err(Error::Xml(
                    "a range cannot be moved into the middle of itself".to_owned(),
                ));
            }
            let moved: Vec<Block> = state.doc.blocks[blocks.clone()].to_vec();
            let count = moved.len();
            let mut batch: Vec<Action> = (0..count)
                .map(|_| Action::RemoveBlock {
                    index: blocks.start,
                })
                .collect();
            // After the removals, indices past the range have shifted down by its length.
            let landing = match to > blocks.start {
                true => to - count,
                false => to,
            };
            for (offset, block) in moved.into_iter().enumerate() {
                batch.push(Action::InsertBlock {
                    index: landing + offset,
                    block: Box::new(block),
                });
            }
            Self::commit(state, Action::Batch(batch))?;
            Ok(count)
        })
    }
}
