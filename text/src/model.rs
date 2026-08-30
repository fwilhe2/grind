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

use crate::style::CharStyle;

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
// `Run::Text` is much bigger than `Run::Tab` since it started carrying a [`CharStyle`], and
// boxing something would fix that. Deliberately not done: a paragraph is text runs with the
// occasional tab in it, so the big variant *is* the common case, and the trade on offer is an
// allocation per run of prose in exchange for a smaller tab. `split_runs` and `coalesce` clone
// runs constantly, which is exactly where that allocation would land.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    /// Character data, with the styling that applies to it.
    Text {
        text: String,
        /// The composed **named** `text:style-name`s, outermost first, joined by a space.
        ///
        /// `text:span` nests, and this model is flat, so reading composes the stack down each
        /// branch (`doc/text-core.md`). Lossy for the *names* and lossless for the rendering;
        /// acceptable only because R6 never rewrites a paragraph nobody edited.
        ///
        /// Only names this build does not interpret reach here. A span whose style is a
        /// generated *automatic* one is resolved into [`Run::Text::props`] instead and its name
        /// forgotten, because that name is a serialisation detail rather than the document's
        /// own vocabulary — see [`crate::style`] for the whole of that argument.
        style: Option<String>,
        /// The direct character formatting that applies to this run, composed from every
        /// automatic style open over it.
        props: CharStyle,
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
    /// `draw:frame` holding a `draw:image` (rng:7932 inside `paragraph-content`, rng:5380) —
    /// anchored `char` or `paragraph` (`doc/suite.md`'s proposed scope for this crate; both
    /// anchor types are one child of a `text:p` regardless, so both are this one variant).
    /// One caret position, like a tab: it can be typed over, erased and backspaced across as
    /// a whole, but never edited a character at a time.
    Image {
        /// `draw:mime-type` on `draw:image` (rng:5397).
        mime: String,
        /// The decoded bytes.
        ///
        /// Only `office:binary-data` (rng:7681) is read — the flat form every `.fodt` uses,
        /// and what a package regenerates into on write (R3). A package's own
        /// `xlink:href` into its `Pictures/` entries (rng:5383's other choice) is a named
        /// gap: resolving it needs the archive's other files threaded through the reader,
        /// which nothing else here does yet, so such an image is inert content today —
        /// preserved by R6 whenever the paragraph around it is never edited, invisible to
        /// this model when it is.
        data: Vec<u8>,
        /// `svg:width` / `svg:height`, ODF lengths kept verbatim (`common-draw-size-attlist`,
        /// rng:1778) — off whichever of the frame and its image carries them, outermost
        /// preferred, since that is the size a person actually sees.
        width: Option<String>,
        height: Option<String>,
    },
}

impl Run {
    /// Unformatted character data — no style name, no direct formatting, no link.
    ///
    /// What a script's text is, and therefore what most of this crate builds: `set_text`,
    /// `insert`, and every test that does not care about styling.
    pub fn plain(text: impl Into<String>) -> Self {
        Run::Text {
            text: text.into(),
            style: None,
            props: CharStyle::default(),
            href: None,
        }
    }

    /// The direct character formatting on this run, if it can carry any. A tab, a break and a
    /// bookmark cannot, and answer with nothing rather than with a default nobody set.
    pub fn props(&self) -> Option<&CharStyle> {
        match self {
            Run::Text { props, .. } => Some(props),
            _ => None,
        }
    }

    /// The characters this run contributes to the paragraph's plain text.
    pub fn text(&self) -> &str {
        match self {
            Run::Text { text, .. } => text,
            Run::Tab => "\t",
            Run::Break => "\n",
            Run::Bookmark { .. } => "",
            // The object replacement character — Unicode's own placeholder for exactly this,
            // so a caret has one position to sit at and `Block::text()` has one character to
            // count, the same trade `Run::Tab` makes.
            Run::Image { .. } => "\u{fffc}",
        }
    }

    /// How many characters it contributes — what a `p12+40` offset counts against.
    pub fn len(&self) -> usize {
        self.text().chars().count()
    }

    /// Whether it contributes no characters at all. True of every bookmark: an anchor is a
    /// position rather than content.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Split a run sequence at a character offset, cutting a run in half if the offset lands
/// inside one.
///
/// The primitive under every caret-level edit: typing, erasing and splitting a block are all
/// "cut here, put something between the halves". Only a [`Run::Text`] can be cut — [`Run::Tab`]
/// and [`Run::Break`] are one character each and atomic, so no integer offset lands inside one.
///
/// **A zero-width run exactly at the offset goes to the second half.** That is a real decision
/// and it only shows up for bookmarks: `#intro` at the front of a paragraph anchors the text
/// after it, so when that paragraph is split the anchor follows the words it names rather than
/// staying behind on an empty stub.
pub fn split_runs(runs: &[Run], offset: usize) -> (Vec<Run>, Vec<Run>) {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    let mut pos = 0;
    for run in runs {
        let len = run.len();
        if pos + len <= offset && !(len == 0 && pos == offset) {
            head.push(run.clone());
        } else if pos >= offset {
            tail.push(run.clone());
        } else if let Run::Text {
            text,
            style,
            props,
            href,
        } = run
        {
            let cut = text
                .char_indices()
                .nth(offset - pos)
                .map_or(text.len(), |(i, _)| i);
            head.push(Run::Text {
                text: text[..cut].to_owned(),
                style: style.clone(),
                props: props.clone(),
                href: href.clone(),
            });
            tail.push(Run::Text {
                text: text[cut..].to_owned(),
                style: style.clone(),
                props: props.clone(),
                href: href.clone(),
            });
        } else {
            // Unreachable for the reason above, and a `tail` push rather than a panic because a
            // model invariant is not worth taking a word processor down over.
            tail.push(run.clone());
        }
        pos += len;
    }
    (head, tail)
}

/// Merge adjacent text runs that agree about their formatting, and drop empty ones.
///
/// `grid.rs`'s `normalize()`, for prose. Every edit that splices runs together leaves
/// fragments — an empty half from a cut at a boundary, or two neighbours that were one run a
/// moment ago — and without this a paragraph typed one character at a time would accumulate
/// one `<text:span>` per keystroke. Canonical runs also keep R6's diffs small, because the
/// bytes spliced back into the file are the bytes the same text would have been written as if
/// it had never been edited.
pub fn coalesce(runs: &mut Vec<Run>) {
    runs.retain(|run| !matches!(run, Run::Text { text, .. } if text.is_empty()));
    let mut i = 1;
    while i < runs.len() {
        let joined = match (&runs[i - 1], &runs[i]) {
            (
                Run::Text {
                    text: a,
                    style: sa,
                    props: pa,
                    href: ha,
                },
                Run::Text {
                    text: b,
                    style: sb,
                    props: pb,
                    href: hb,
                },
            ) if sa == sb && pa == pb && ha == hb => Some(format!("{a}{b}")),
            _ => None,
        };
        match joined {
            Some(text) => {
                runs.remove(i);
                if let Run::Text { text: into, .. } = &mut runs[i - 1] {
                    *into = text;
                }
            }
            None => i += 1,
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
        self.runs.iter().map(Run::len).sum()
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
            || self.runs.iter().any(|r| match r {
                Run::Text { style, props, .. } => style.is_some() || !props.is_plain(),
                _ => false,
            })
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
    /// Every style name the file this document was read from **declares** — every
    /// `style:style`, of every family, in `office:styles` and `office:automatic-styles` alike.
    ///
    /// Not a style *definition*: `doc/text-core.md` gates resolving a named style's properties,
    /// and this changes nothing about that. It is the set of names the document said exist, and
    /// it exists so that one question can be answered — *is this `text:style-name` pointing at
    /// anything?* — which is `doc/dsl.md` §4.3's `undeclared-style` rule and nothing else.
    ///
    /// Empty for a document this build wrote or generated, and for one read from a projection,
    /// because neither carries style definitions. That is not a hole in this field: it is
    /// `doc/text-core.md`'s known loss, and `undeclared-style` reporting every style of a
    /// round-tripped document is the loss made visible rather than a false positive.
    pub styles: std::collections::BTreeSet<String>,
    /// The next id to hand out. Monotonic, never reused, so a stale [`BlockId`] is always
    /// stale rather than silently pointing at something new.
    next_id: u64,
    /// The file this document was read from, and where its blocks are in it (R6).
    ///
    /// `None` for a document this program authored — there is no diff to preserve — and for
    /// one read from a package, which is a zip.
    pub source: Option<Box<crate::odf::source::Source>>,
    /// The **projection** this document was read from, when it was read from one (`doc/dsl.md`
    /// §3.1, D5), and where each block sits in it.
    ///
    /// A second slot rather than a variant of the first, for the reason
    /// `grind_core::projection::source` gives: the two retain different things, and a document
    /// has at most one of them because it was read from one file.
    pub projection_source: Option<Box<grind_core::projection::Source>>,
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
                props: Default::default(),
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
            props: Default::default(),
            href: None,
        });
        block.runs.push(Run::Tab);
        assert_eq!(block.len(), 5, "four characters and a tab, not six bytes");
        assert_eq!(block.text(), "über\t");
    }

    fn styled(text: &str, style: Option<&str>) -> Run {
        Run::Text {
            text: text.to_owned(),
            style: style.map(str::to_owned),
            props: Default::default(),
            href: None,
        }
    }

    /// The offsets that matter are the ones at a seam: 0, the end, and exactly on a boundary
    /// between two runs. Everything between them is the same arithmetic.
    #[test]
    fn a_split_cuts_a_text_run_and_never_an_atomic_one() {
        let runs = vec![styled("ab", None), Run::Tab, styled("cd", None)];
        for (offset, head, tail) in [
            (0, "", "ab\tcd"),
            (1, "a", "b\tcd"),
            (2, "ab", "\tcd"),
            // 3 is *after* the tab: an offset cannot land inside a one-character run.
            (3, "ab\t", "cd"),
            (5, "ab\tcd", ""),
            // Past the end clamps rather than panicking — a caret outlives the text under it.
            (99, "ab\tcd", ""),
        ] {
            let (a, b) = split_runs(&runs, offset);
            let joined = |runs: Vec<Run>| runs.iter().map(Run::text).collect::<String>();
            assert_eq!(
                (joined(a), joined(b)),
                (head.to_owned(), tail.to_owned()),
                "at {offset}"
            );
        }
    }

    #[test]
    fn a_split_keeps_each_halfs_formatting() {
        let runs = vec![styled("plain", None), styled("bold", Some("B"))];
        let (head, tail) = split_runs(&runs, 7);
        assert_eq!(head, vec![styled("plain", None), styled("bo", Some("B"))]);
        assert_eq!(tail, vec![styled("ld", Some("B"))]);
    }

    #[test]
    fn a_bookmark_at_the_split_point_follows_the_text_it_anchors() {
        // `#intro` sits at offset 0 and names what comes after it, so splitting at 0 must not
        // leave it behind on the empty stub. Everything *before* the offset still stays.
        let mark = Run::Bookmark {
            name: "intro".to_owned(),
        };
        let runs = vec![mark.clone(), styled("hello", None)];
        assert_eq!(split_runs(&runs, 0).0, vec![]);
        assert_eq!(
            split_runs(&runs, 0).1,
            vec![mark.clone(), styled("hello", None)]
        );
        // At the end of the text it is behind the offset and stays in the first half.
        assert_eq!(split_runs(&runs, 5).0, vec![mark, styled("hello", None)]);
    }

    #[test]
    fn coalescing_merges_what_agrees_and_leaves_what_does_not() {
        let mut runs = vec![
            styled("", None),
            styled("a", None),
            styled("b", None),
            styled("c", Some("B")),
            styled("d", Some("B")),
            Run::Tab,
            styled("e", None),
        ];
        coalesce(&mut runs);
        assert_eq!(
            runs,
            vec![
                styled("ab", None),
                styled("cd", Some("B")),
                Run::Tab,
                styled("e", None),
            ],
            "empty dropped, like formatting merged, a tab still a boundary"
        );

        // A hyperlink is part of what "agrees": two runs with the same style and different
        // targets are two links, not one.
        let mut runs = vec![
            Run::Text {
                text: "a".to_owned(),
                style: None,
                props: Default::default(),
                href: Some("x".to_owned()),
            },
            Run::Text {
                text: "b".to_owned(),
                style: None,
                props: Default::default(),
                href: Some("y".to_owned()),
            },
        ];
        let before = runs.clone();
        coalesce(&mut runs);
        assert_eq!(runs, before);
    }

    #[test]
    fn coalescing_never_drops_a_bookmark() {
        // It is empty by every measure this function has, and it is the one empty thing that
        // must survive.
        let mut runs = vec![Run::Bookmark {
            name: "here".to_owned(),
        }];
        let before = runs.clone();
        coalesce(&mut runs);
        assert_eq!(runs, before);
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
            props: Default::default(),
            href: None,
        });
        assert_eq!(block.text(), "hello");
        assert_eq!(block.len(), 5, "an anchor is a position, not content");
    }
}
