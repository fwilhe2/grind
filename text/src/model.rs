// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The text document model. **\[ODT\]**
//!
//! Shaped by one fact from the schema, recorded in `doc/odt-format.md` §2 and worth repeating
//! because it is not what anyone expects: **a text document's body is a flat sequence, not a
//! tree.** `office-text-content-main` is `zeroOrMore text-content` (rng:8352, rng:16938), and
//! a `text:h` does not contain the paragraphs beneath it. Outline structure is implied by
//! `text:outline-level` on each heading and by nothing else.
//!
//! So [`Document::blocks`] is a `Vec`, addressing is by index ([`crate::loc`]), and "the
//! section under heading 3.2" is a *range* computed from outline levels rather than a subtree.
//!
//! Positions are **0-based here** and 1-based only where a person types one — `loc.rs` is the
//! single conversion, exactly as `grind_sheet::a1` is for cells.

use std::collections::BTreeMap;

/// A block's identity, stable across insertions above it.
///
/// The one piece of machinery the spreadsheet did not need. A cell is addressed by where it
/// *is* and stays addressable when a neighbour changes; a block is addressed by its position
/// in a sequence, so inserting one two blocks up silently re-targets every index held
/// elsewhere. An undo entry, an observer notification and R6's splice registry all outlive the
/// edit that follows them, so they carry this instead of an index.
///
/// Cheaper to have from the first commit than to retrofit — the same argument that put
/// `get_viewport` in phase 1 rather than phase 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u64);

/// What kind of block this is.
///
/// Lists are **flattened into the block sequence** rather than nested: a `text:list-item`
/// becomes a block carrying its depth. Three reasons, in order of weight: addressing stays
/// uniform, so `p12` means the twelfth block whatever kind it is; the model matches the body's
/// own flatness rather than re-introducing a tree the schema does not have; and the writer
/// reconstructs `text:list` nesting from depth changes, which is a fold rather than a
/// traversal.
///
/// What it costs: the `text:list` element's own style name and its `text:continue-numbering`
/// are not carried, so re-emitting a list this build *edited* loses them.
///
/// ponytail: carrying a `List` block that owns its items is the alternative, and it costs a
/// recursive model plus recursive addressing. Worth it when a document turns up whose list
/// styles matter — R6 means an unedited list keeps everything, so the trigger is somebody
/// editing one and noticing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// `text:p`
    Paragraph,
    /// `text:h`, with its `text:outline-level`.
    ///
    /// Read at **any** level: the schema's `positiveInteger` has no ceiling (rng:6867), so a
    /// document with a level-9 heading loads. Authoring stops at 6 — see `doc/text-core.md`.
    Heading { level: u32 },
    /// One `text:list-item`, with its 1-based nesting depth.
    ListItem { depth: u32 },
}

/// A piece of a paragraph's content — `paragraph-content` in the schema (rng:8405).
///
/// `text:s` never appears here: it is ODF's run-length encoding of spaces and is **expanded on
/// read and re-encoded on write** (`doc/odt-format.md` §3.3), the same treatment
/// `table:number-columns-repeated` gets, and for the same reason — it is a correctness trap
/// rather than an optimisation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    /// Character data, with the styling that applies to it.
    Text {
        text: String,
        /// The composed `text:style-name`, outermost first, joined by a space.
        ///
        /// `text:span` nests, and this model is flat, so reading composes the stack down each
        /// branch (`doc/text-core.md`). Lossy for the *names* and lossless for the rendering;
        /// acceptable only because R6 never rewrites a paragraph nobody edited.
        style: Option<String>,
        /// `xlink:href`, when this run sits inside a `text:a`.
        href: Option<String>,
    },
    /// `text:tab` — not a tab character, which is why it is its own thing.
    Tab,
    /// `text:line-break` — a break *within* a paragraph, which is not a new paragraph.
    Break,
    /// `text:bookmark`. The named-range analogue: an anchor that moves with the text because
    /// it is *in* the text, which is what lets `loc.rs` offer `#intro` as an address that
    /// survives editing.
    Bookmark { name: String },
}

impl Run {
    /// The characters this run contributes to the paragraph's plain text.
    pub fn text(&self) -> &str {
        match self {
            Run::Text { text, .. } => text,
            Run::Tab => "\t",
            Run::Break => "\n",
            Run::Bookmark { .. } => "",
        }
    }
}

/// One block: a paragraph, a heading, or a list item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    /// `text:style-name` — the *named* style, kept verbatim.
    ///
    /// Not resolved through `style:parent-style-name`. That matters more here than it does for
    /// cells and is a named gap: see `doc/text-core.md`.
    pub style: Option<String>,
    pub runs: Vec<Run>,
}

impl Block {
    pub fn new(id: BlockId, kind: BlockKind) -> Self {
        Block {
            id,
            kind,
            style: None,
            runs: Vec::new(),
        }
    }

    /// The block's plain text, runs concatenated.
    pub fn text(&self) -> String {
        self.runs.iter().map(Run::text).collect()
    }

    /// How many characters [`Block::text`] would produce — what an offset in `loc.rs` counts
    /// against.
    pub fn len(&self) -> usize {
        self.runs.iter().map(|r| r.text().chars().count()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The outline level, for a heading. `None` for anything else.
    pub fn outline_level(&self) -> Option<u32> {
        match self.kind {
            BlockKind::Heading { level } => Some(level),
            _ => None,
        }
    }

    /// Whether any run carries direct formatting or a named style — what
    /// `grind text formatting` looks for.
    pub fn is_styled(&self) -> bool {
        self.style.is_some()
            || self
                .runs
                .iter()
                .any(|r| matches!(r, Run::Text { style: Some(_), .. }))
    }
}

/// A text document.
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// Every `text:bookmark` in the document, name to the block holding it.
    ///
    /// Derived on read rather than stored a second time — the anchor itself lives in the runs,
    /// and this is the index over them. Rebuilt whenever blocks change, for the same reason
    /// `grind_sheet`'s filter does not store which rows it hides: two copies of one fact
    /// disagree eventually.
    pub bookmarks: BTreeMap<String, BlockId>,
    /// The next id to hand out. Monotonic, never reused, so a stale [`BlockId`] is always
    /// stale rather than silently pointing at something new.
    next_id: u64,
    /// The file this document was read from, and where its blocks are in it (R6).
    ///
    /// `None` for a document this program authored — there is no diff to preserve — and for
    /// one read from a package, which is a zip.
    pub source: Option<Box<crate::odf::source::Source>>,
    /// What has changed since it was read. See [`crate::odf::source::Edits`].
    pub edits: crate::odf::source::Edits,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint an id. The only place one is made.
    pub fn next_id(&mut self) -> BlockId {
        let id = BlockId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn block(&self, index: usize) -> Option<&Block> {
        self.blocks.get(index)
    }

    /// Where a block with this id currently sits.
    pub fn index_of(&self, id: BlockId) -> Option<usize> {
        self.blocks.iter().position(|b| b.id == id)
    }

    /// The whole document as plain text, one block per line.
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(Block::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Rebuild [`Document::bookmarks`] from the runs. Called after any change that could move
    /// or remove one.
    pub fn reindex_bookmarks(&mut self) {
        self.bookmarks.clear();
        for block in &self.blocks {
            for run in &block.runs {
                if let Run::Bookmark { name } = run {
                    self.bookmarks.insert(name.clone(), block.id);
                }
            }
        }
    }

    /// Every heading, as `(index, level)`, in document order — the outline.
    pub fn outline(&self) -> impl Iterator<Item = (usize, u32)> + '_ {
        self.blocks
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.outline_level().map(|level| (i, level)))
    }

    /// The blocks belonging to the heading at `index`: the heading itself, plus everything up
    /// to the next heading at the same level or higher.
    ///
    /// This is what "move section 3.2" operates on, and it is computed rather than stored
    /// because the body has no such container (`doc/odt-format.md` §2). `None` if `index` is
    /// not a heading.
    pub fn section(&self, index: usize) -> Option<std::ops::Range<usize>> {
        let level = self.block(index)?.outline_level()?;
        let end = self.blocks[index + 1..]
            .iter()
            .position(|b| b.outline_level().is_some_and(|l| l <= level))
            .map_or(self.blocks.len(), |offset| index + 1 + offset);
        Some(index..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(spec: &[(BlockKind, &str)]) -> Document {
        let mut d = Document::new();
        for (kind, text) in spec {
            let id = d.next_id();
            let mut block = Block::new(id, kind.clone());
            block.runs.push(Run::Text {
                text: (*text).to_owned(),
                style: None,
                href: None,
            });
            d.blocks.push(block);
        }
        d
    }

    #[test]
    fn ids_are_never_reused() {
        let mut d = Document::new();
        let a = d.next_id();
        let b = d.next_id();
        assert_ne!(a, b);
        // Even after the block goes, its id does not come back — a stale reference stays
        // stale rather than pointing at something new.
        let c = d.next_id();
        assert!(c.0 > b.0);
    }

    #[test]
    fn a_section_runs_to_the_next_heading_of_the_same_level_or_higher() {
        let d = doc(&[
            (BlockKind::Heading { level: 1 }, "One"),
            (BlockKind::Paragraph, "under one"),
            (BlockKind::Heading { level: 2 }, "One point one"),
            (BlockKind::Paragraph, "under one one"),
            (BlockKind::Heading { level: 1 }, "Two"),
            (BlockKind::Paragraph, "under two"),
        ]);
        // A level-1 section swallows the level-2 heading beneath it.
        assert_eq!(d.section(0), Some(0..4));
        // A level-2 section stops at the next level-1.
        assert_eq!(d.section(2), Some(2..4));
        // The last section runs to the end.
        assert_eq!(d.section(4), Some(4..6));
        // A paragraph is not a section.
        assert_eq!(d.section(1), None);
    }

    #[test]
    fn the_outline_is_every_heading_and_nothing_else() {
        let d = doc(&[
            (BlockKind::Heading { level: 1 }, "One"),
            (BlockKind::Paragraph, "text"),
            (BlockKind::ListItem { depth: 1 }, "item"),
            (BlockKind::Heading { level: 3 }, "Deep"),
        ]);
        assert_eq!(d.outline().collect::<Vec<_>>(), vec![(0, 1), (3, 3)]);
    }

    #[test]
    fn a_blocks_length_counts_characters_not_bytes() {
        let mut block = Block::new(BlockId(0), BlockKind::Paragraph);
        block.runs.push(Run::Text {
            text: "über".to_owned(),
            style: None,
            href: None,
        });
        block.runs.push(Run::Tab);
        assert_eq!(block.len(), 5, "four characters and a tab, not six bytes");
        assert_eq!(block.text(), "über\t");
    }

    #[test]
    fn a_bookmark_contributes_no_characters() {
        let mut block = Block::new(BlockId(0), BlockKind::Paragraph);
        block.runs.push(Run::Bookmark {
            name: "intro".to_owned(),
        });
        block.runs.push(Run::Text {
            text: "hello".to_owned(),
            style: None,
            href: None,
        });
        assert_eq!(block.text(), "hello");
        assert_eq!(block.len(), 5, "an anchor is a position, not content");
    }
}
