// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every change to a text document, and its inverse. **\[ODT\]**
//!
//! `doc/plan.md` rule 2: **undo/redo lives in the core.** [`Document::apply`] returns the
//! action that undoes it, and that is the whole mechanism — shells never implement history,
//! and neither does a second document type invent a second one.
//!
//! The shape is `grind_sheet::action`'s, and deliberately so. What differs is that a block
//! action carries an **index**, and indices move: inserting a block shifts every later one. So
//! an inverse is only valid against the document that produced it, applied in order — which is
//! exactly the guarantee an undo stack already gives.

use crate::model::{Block, Document};

/// One change. Every variant carries what it needs to be undone without consulting the
/// document, so a stack of inverses survives being written to disk and read back.
#[derive(Clone, Debug)]
pub enum Action {
    /// Replace a block wholesale — its kind, its style and its runs.
    ///
    /// One action rather than separate content/style/kind edits, for the reason
    /// `grind_sheet`'s `SetFormula` is one: a paragraph that becomes a heading and gains text
    /// in the same keystroke must undo in one step, and two actions would get the order wrong.
    SetBlock { index: usize, block: Box<Block> },
    /// Insert a block, shifting everything at or after `index` down.
    InsertBlock { index: usize, block: Box<Block> },
    /// Remove the block at `index`.
    RemoveBlock { index: usize },
    /// Several changes that undo as one step.
    Batch(Vec<Action>),
}

impl Document {
    /// Apply an action, returning the action that undoes it.
    ///
    /// `None` means the action did not apply — an index past the end, usually because an
    /// inverse was replayed against a document it was not recorded from. Never a panic: a
    /// stale undo entry is a thing that happens, and `App::undo` reports it as "nothing
    /// happened" rather than taking the process down.
    pub fn apply(&mut self, action: Action) -> Option<Action> {
        let inverse = match action {
            Action::SetBlock { index, block } => {
                let old = self.blocks.get(index)?.clone();
                // R6: this block's element can be replaced in place. Both ids are recorded —
                // an edit that also changes the id has retired the old element and created a
                // new one, and the writer needs to know about both.
                self.edits.blocks.insert(old.id);
                self.edits.blocks.insert(block.id);
                self.blocks[index] = *block;
                Action::SetBlock {
                    index,
                    block: Box::new(old),
                }
            }
            Action::InsertBlock { index, block } => {
                if index > self.blocks.len() {
                    return None;
                }
                // The sequence moved, so the file's structure and the model's no longer
                // correspond — see `odf::source::Edits::structural`.
                self.edits.structural = true;
                self.blocks.insert(index, *block);
                Action::RemoveBlock { index }
            }
            Action::RemoveBlock { index } => {
                if index >= self.blocks.len() {
                    return None;
                }
                self.edits.structural = true;
                let old = self.blocks.remove(index);
                Action::InsertBlock {
                    index,
                    block: Box::new(old),
                }
            }
            Action::Batch(actions) => {
                // Undoing a batch means undoing its parts **in reverse**: the second edit was
                // made against the document the first produced. Getting this backwards is the
                // classic command-pattern bug and it only shows up when two edits overlap.
                let mut inverses = Vec::with_capacity(actions.len());
                for action in actions {
                    inverses.push(self.apply(action)?);
                }
                inverses.reverse();
                Action::Batch(inverses)
            }
        };
        // A bookmark may have moved, gone or arrived. Rebuilt rather than tracked, because two
        // copies of one fact disagree eventually.
        self.reindex_bookmarks();
        Some(inverse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockKind, Run};

    fn para(doc: &mut Document, text: &str) -> Block {
        let id = doc.next_id();
        let mut block = Block::new(id, BlockKind::Paragraph);
        block.runs.push(Run::Text {
            text: text.to_owned(),
            style: None,
            props: Default::default(),
            href: None,
        });
        block
    }

    fn doc(texts: &[&str]) -> Document {
        let mut doc = Document::new();
        for t in texts {
            let block = para(&mut doc, t);
            doc.blocks.push(block);
        }
        doc
    }

    /// The property the whole mechanism rests on: apply the inverse and you are back where you
    /// started, for every variant.
    #[test]
    fn every_action_round_trips_through_its_inverse() {
        let before = doc(&["a", "b", "c"]);

        let mut d = before.clone();
        let extra = para(&mut d, "new");
        let cases: Vec<Action> = vec![
            Action::SetBlock {
                index: 1,
                block: Box::new(extra.clone()),
            },
            Action::InsertBlock {
                index: 0,
                block: Box::new(extra.clone()),
            },
            Action::InsertBlock {
                index: 3,
                block: Box::new(extra.clone()),
            },
            Action::RemoveBlock { index: 2 },
            Action::Batch(vec![
                Action::RemoveBlock { index: 0 },
                Action::InsertBlock {
                    index: 2,
                    block: Box::new(extra),
                },
            ]),
        ];

        for action in cases {
            let mut d = before.clone();
            let inverse = d.apply(action.clone()).expect("applies");
            assert_ne!(d.text(), before.text(), "{action:?} changed nothing");
            d.apply(inverse).expect("undoes");
            assert_eq!(d.text(), before.text(), "{action:?} did not undo cleanly");
        }
    }

    #[test]
    fn a_batch_undoes_its_parts_in_reverse() {
        // Two removals of the *same* index: the second only means what it means because the
        // first already happened. Undoing them forwards would put them back swapped.
        let mut d = doc(&["a", "b", "c"]);
        let inverse = d
            .apply(Action::Batch(vec![
                Action::RemoveBlock { index: 0 },
                Action::RemoveBlock { index: 0 },
            ]))
            .expect("applies");
        assert_eq!(d.text(), "c");
        d.apply(inverse).expect("undoes");
        assert_eq!(d.text(), "a\nb\nc");
    }

    #[test]
    fn an_action_past_the_end_fails_rather_than_panicking() {
        let mut d = doc(&["a"]);
        assert!(d.apply(Action::RemoveBlock { index: 9 }).is_none());
        let block = para(&mut d, "x");
        assert!(
            d.apply(Action::InsertBlock {
                index: 9,
                block: Box::new(block)
            })
            .is_none()
        );
        assert_eq!(d.text(), "a", "a failed action changes nothing");
    }

    #[test]
    fn removing_a_block_drops_the_bookmark_it_held() {
        let mut d = doc(&["a", "b"]);
        d.blocks[1].runs.push(Run::Bookmark {
            name: "here".to_owned(),
        });
        d.reindex_bookmarks();
        assert_eq!(d.bookmarks.len(), 1);

        let inverse = d.apply(Action::RemoveBlock { index: 1 }).expect("applies");
        assert!(d.bookmarks.is_empty(), "the index followed the block out");
        d.apply(inverse).expect("undoes");
        assert_eq!(d.bookmarks.len(), 1, "and back in again");
    }
}
