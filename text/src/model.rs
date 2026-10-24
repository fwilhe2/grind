// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The text document model. **\[ODT\]**
//!
//! A text body is a flat sequence, not a tree (`doc/odt-format.md` §2): `office-text-content-main`
//! is `zeroOrMore text-content` (rng:8352, rng:16938), and a `text:h` does not contain the
//! paragraphs under it; the outline is implied by `text:outline-level` alone. So
//! [`Document::blocks`] is a `Vec`, addressing is by index ([`crate::loc`]), and "the section
//! under heading 3.2" is a range computed from levels. Positions are 0-based; `loc.rs` is the one
//! conversion.

use std::collections::BTreeMap;

use crate::style::CharStyle;

/// A block's identity, stable across insertions above it. A block's index changes whenever one is
/// inserted before it, and an undo entry, an observer notification and the splice registry all
/// outlive such edits, so they carry this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u64);

/// What kind of block this is. Lists are flattened into the sequence: a `text:list-item` becomes a
/// block with its depth, so `p12` is the twelfth block whatever it is, the model matches the
/// body's flatness, and the writer rebuilds `text:list` nesting from depth changes as a fold. A
/// list read from a file keeps its element's attributes ([`crate::odf::source::Source::lists`]); a
/// new one has none.
///
/// ponytail: a `List` block owning its items is the alternative, at the cost of recursive
/// addressing — worth it when list-level editing needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// `text:p`
    Paragraph,
    /// `text:h`, with its `text:outline-level`, read at any level (the schema sets no ceiling,
    /// rng:6867); authoring stops at 6 (`doc/text-core.md`).
    Heading { level: u32 },
    /// One `text:list-item`, with its 1-based nesting depth.
    ListItem { depth: u32 },
}

/// How deep a list may nest before Tab stops adding a level.
pub const MAX_LIST_DEPTH: u32 = 9;

/// What Tab (`by = 1`) or Shift+Tab (`by = -1`) does to the block at `kind`, given the caret's
/// offset in it: nest a list item deeper, un-nest one, or start a list from a block the caret is
/// at the front of. `None` when none applies, leaving the key free — a literal tab mid-sentence,
/// or nothing.
pub fn indent_kind(kind: &BlockKind, offset: usize, by: i32) -> Option<BlockKind> {
    match kind {
        BlockKind::ListItem { depth } => {
            let depth = (*depth as i32 + by).clamp(0, MAX_LIST_DEPTH as i32);
            Some(match depth {
                0 => BlockKind::Paragraph,
                depth => BlockKind::ListItem {
                    depth: depth as u32,
                },
            })
        }
        // Only the front of the block starts a list, so Tab after a word is still a tab.
        _ if by > 0 && offset == 0 => Some(BlockKind::ListItem { depth: 1 }),
        _ => None,
    }
}

/// The two named paragraph styles a shell may apply and draw, `Title` and `Subtitle`; any other
/// named style is kept and not drawn specially.
pub const NAMED_STYLES: [&str; 2] = ["Title", "Subtitle"];

/// What a block's `text:style-name` becomes after asking for `requested` — `Some("Title")` or
/// `Some("Subtitle")` from a menu, `None` from Paragraph and from any heading or list pick. A
/// style is only taken off when a shell put it on: Paragraph after Title clears it, but a block
/// wearing the document's own `Quotations` keeps it.
pub fn named_style_for(current: Option<&str>, requested: Option<&str>) -> Option<String> {
    let ours = current.is_some_and(|name| NAMED_STYLES.contains(&name));
    match (requested, ours) {
        (Some(name), _) => Some(name.to_owned()),
        (None, true) => None,
        (None, false) => current.map(str::to_owned),
    }
}

#[cfg(test)]
mod indent_tests {
    use super::*;

    #[test]
    fn tab_at_the_front_of_a_paragraph_starts_a_list() {
        assert_eq!(
            indent_kind(&BlockKind::Paragraph, 0, 1),
            Some(BlockKind::ListItem { depth: 1 })
        );
    }

    #[test]
    fn tab_mid_paragraph_does_nothing() {
        assert_eq!(indent_kind(&BlockKind::Paragraph, 3, 1), None);
    }

    #[test]
    fn shift_tab_out_of_the_first_level_ends_the_list() {
        assert_eq!(
            indent_kind(&BlockKind::ListItem { depth: 1 }, 0, -1),
            Some(BlockKind::Paragraph)
        );
    }

    #[test]
    fn depth_is_clamped_at_the_ceiling_and_the_floor() {
        assert_eq!(
            indent_kind(
                &BlockKind::ListItem {
                    depth: MAX_LIST_DEPTH
                },
                0,
                1
            ),
            Some(BlockKind::ListItem {
                depth: MAX_LIST_DEPTH
            })
        );
        assert_eq!(
            indent_kind(&BlockKind::Paragraph, 0, -1),
            None,
            "shift+tab on a paragraph is not a gesture"
        );
    }

    #[test]
    fn choosing_paragraph_after_title_clears_the_name_this_build_put_on() {
        assert_eq!(named_style_for(Some("Title"), None), None);
    }

    #[test]
    fn choosing_paragraph_on_a_documents_own_name_leaves_it_alone() {
        assert_eq!(
            named_style_for(Some("Quotations"), None),
            Some("Quotations".to_owned())
        );
    }

    #[test]
    fn asking_for_a_name_always_wins() {
        assert_eq!(
            named_style_for(Some("Quotations"), Some("Title")),
            Some("Title".to_owned())
        );
        assert_eq!(
            named_style_for(None, Some("Subtitle")),
            Some("Subtitle".to_owned())
        );
    }
}

/// A piece of a paragraph's content — `paragraph-content` (rng:8405). `text:s` never appears:
/// ODF's run-length encoding of spaces is expanded on reading and re-encoded on writing
/// (`doc/odt-format.md` §3.3).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    /// Character data, with the styling that applies to it.
    Text {
        text: String,
        /// The composed named `text:style-name`s, outermost first, joined by a space. `text:span`
        /// nests and this model is flat, so reading composes the stack down each branch
        /// (`doc/text-core.md`): lossy for the names, lossless for the rendering, and an unedited
        /// paragraph is never rewritten. An automatic style is resolved into [`Run::Text::props`]
        /// instead and its generated name forgotten ([`crate::style`]).
        style: Option<String>,
        /// The direct character formatting on this run, composed from every automatic style open
        /// over it.
        props: CharStyle,
        /// `xlink:href`, when this run sits inside a `text:a`.
        href: Option<String>,
    },
    /// `text:tab` — not a tab character, which is why it is its own thing.
    Tab,
    /// `text:line-break` — a break *within* a paragraph, which is not a new paragraph.
    Break,
    /// `text:bookmark`: an anchor that moves with the text because it is in the text, so `#intro`
    /// survives editing.
    Bookmark { name: String },
    /// `draw:frame` holding a `draw:image` (rng:7932 inside `paragraph-content`, rng:5380),
    /// anchored `char` or `paragraph` — both one child of a `text:p`. One caret position, like a
    /// tab: typed over, erased and backspaced across whole.
    Image {
        /// `draw:mime-type` on `draw:image` (rng:5397).
        mime: String,
        /// The decoded bytes — from `office:binary-data` (rng:7681), or from the package entry an
        /// `xlink:href` names (`Pictures/…`).
        data: Vec<u8>,
        /// `svg:width` / `svg:height`, verbatim (rng:1778), from the outermost of the frame and
        /// its image that carries them — the size a person sees.
        width: Option<String>,
        height: Option<String>,
        /// `text:anchor-type` (rng:6519) — `as-char`, `char`, `paragraph`, `page`, `frame` — kept
        /// and written back: without it a frame takes ODF's default `paragraph`, and an `as-char`
        /// image at the end of a list item moved to its front (loop C). `None` when the document
        /// did not say.
        anchor: Option<String>,
        /// `svg:title` and `svg:desc` on the frame (rng:1756): the short and long alternative text
        /// a screen reader says and Markdown's `![alt]` carries. Written after the `draw:image`,
        /// as the schema orders them.
        title: Option<String>,
        description: Option<String>,
    },
    /// An inline element this model does not read — a footnote (`text:note`, rng:8465) or a field
    /// (`text:date`, `text:page-number`, …, rng:8549 on) — kept as the file spelled it and shown
    /// as the text it shows (the citation mark, the field's cached value). Atomic like an image:
    /// it moves with the text, is erased whole and written back as its own bytes. Only a document
    /// read from a file holds one, spelled with that file's prefixes.
    Kept {
        /// The element, verbatim.
        xml: String,
        /// What it shows where it stands.
        text: String,
    },
}

impl Run {
    /// Unformatted text — no style, no formatting, no link.
    pub fn plain(text: impl Into<String>) -> Self {
        Run::Text {
            text: text.into(),
            style: None,
            props: CharStyle::default(),
            href: None,
        }
    }

    /// The direct character formatting on this run; a tab, a break or a bookmark has none.
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
            // The object replacement character, so a caret has one position and `Block::text()`
            // one character for the image.
            Run::Image { .. } => "\u{fffc}",
            Run::Kept { text, .. } => text,
        }
    }

    /// How many characters it contributes — what a `p12+40` offset counts against.
    pub fn len(&self) -> usize {
        self.text().chars().count()
    }

    /// Whether it contributes no characters — true of every bookmark.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Split a run sequence at a character offset, cutting a run in two if the offset lands inside one
/// — the primitive under every caret edit. Only a [`Run::Text`] can be cut; a tab or a break is
/// one atomic character. A zero-width run exactly at the offset goes to the second half, so
/// `#intro` at the front of a paragraph follows the words it names when the paragraph is split.
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
            // Unreachable; a push rather than a panic, since an invariant is not worth crashing
            // over.
            tail.push(run.clone());
        }
        pos += len;
    }
    (head, tail)
}

/// Merge adjacent text runs that agree about their formatting, and drop empty ones — `grid.rs`'s
/// `normalize()` for prose. Without it a paragraph typed one character at a time would gather a
/// `<text:span>` per keystroke, and canonical runs keep splices small.
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
            ) if sa == sb && pa.same(pb) && ha == hb => Some(format!("{a}{b}")),
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

/// Where a block sits inside a `table:table` — the second axis of the flat sequence.
///
/// A cell holds blocks: `table-table-cell-content` is `zeroOrMore text-content` (rng:16126), the
/// body's own production (rng:16938). So a cell is a coordinate beside the kind rather than a
/// kind: a block is a heading, and the heading in row 0, column 2 of table `Prices`. A table is a
/// maximal run of consecutive blocks naming it, a fold like a list's depth, so `p12`, every caret
/// edit, `#intro`, `§2.1.3` and the span map work inside tables with no table-specific code.
///
/// A table read from a file keeps its own styling through a regenerate
/// ([`crate::odf::source::TableSource`]); a new table has none. A table nested in a cell is read
/// as paragraphs of the cell holding it: its text survives, its structure does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// `table:name` (rng:15970) — the table's identity, the only thing marking where one table
    /// ends and the next begins. Kept verbatim, as LibreOffice keeps it (`doc/odt-format.md` §5b).
    pub table: String,
    /// 0-based; `loc.rs` converts.
    pub row: u32,
    pub column: u32,
    /// `table:number-columns-spanned` / `table:number-rows-spanned` (rng:16102), 1 for a cell
    /// spanning only itself; the positions it covers are written back as
    /// `table:covered-table-cell`.
    pub columns_spanned: u32,
    pub rows_spanned: u32,
}

impl Cell {
    /// A cell spanning only itself.
    pub fn new(table: impl Into<String>, row: u32, column: u32) -> Self {
        Cell {
            table: table.into(),
            row,
            column,
            columns_spanned: 1,
            rows_spanned: 1,
        }
    }

    /// Whether two blocks are in the same cell — the fold the writer performs; a cell with two
    /// paragraphs is two blocks.
    pub fn is_same(&self, other: &Cell) -> bool {
        self.table == other.table && self.row == other.row && self.column == other.column
    }
}

/// One block: a paragraph, a heading, or a list item — possibly inside a table cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    /// `text:style-name` — the named paragraph style, kept verbatim and resolved down its chain
    /// only for showing and printing ([`crate::paragraph::resolve`]).
    pub style: Option<String>,
    /// Where this block sits in a table, when it is in one. See [`Cell`].
    pub cell: Option<Cell>,
    pub runs: Vec<Run>,
    /// Whether it is a generated index entry — a table of contents' or list of figures'
    /// `text:index-body` paragraph. Shown like any block; a save puts the index element back as
    /// the file had it, so an edit to one is refused rather than lost.
    pub generated: bool,
    /// The innermost `text:section` it is in, an index into [`Document::sections`], so a
    /// regenerated body wraps it in its section again.
    pub section: Option<usize>,
}

impl Block {
    pub fn new(id: BlockId, kind: BlockKind) -> Self {
        Block {
            id,
            kind,
            style: None,
            cell: None,
            runs: Vec::new(),
            generated: false,
            section: None,
        }
    }

    /// The block's plain text, runs concatenated.
    pub fn text(&self) -> String {
        self.runs.iter().map(Run::text).collect()
    }

    /// How many characters [`Block::text`] would produce — what a `loc.rs` offset counts against.
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

    /// Whether any run carries direct formatting or a named style — what `grind text formatting`
    /// looks for.
    pub fn is_styled(&self) -> bool {
        self.style.is_some()
            || self.runs.iter().any(|r| match r {
                Run::Text { style, props, .. } => style.is_some() || !props.is_plain(),
                _ => false,
            })
    }
}

/// `fo:widows` and `fo:orphans` of the default paragraph style (rng:12512, rng:12517), as counts:
/// the fewest lines a split paragraph leaves at the top of the next page and the foot of this one.
/// `None` when unstated, which Writer reads as no control (`doc/odt-format.md` §5c, fact 3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParagraphDefaults {
    pub widows: Option<u32>,
    pub orphans: Option<u32>,
}

/// One `text:section`: its start tag as the file spelled it, and the section it is inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub start: String,
    pub parent: Option<usize>,
    /// The element's bytes in the file it came from and the blocks it held, so an untouched
    /// section is written back as those bytes.
    pub range: Option<std::ops::Range<usize>>,
    pub blocks: Vec<crate::model::BlockId>,
}

/// One `style:master-page`, read for printing and never written: its page, header and footer, and
/// the master the next page is set on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Master {
    /// The page layout it names, or `None` when the file does not state one.
    pub geometry: Option<grind_core::page::PageGeometry>,
    pub header: Option<crate::marginal::Marginal>,
    pub footer: Option<crate::marginal::Marginal>,
    /// Lines drawn on every page set on it (`draw:line` in its header or footer).
    pub lines: Vec<crate::marginal::Line>,
    /// `style:next-style-name`; `None` is the master itself.
    pub next: Option<String>,
}

/// A text document.
#[derive(Clone, Debug)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// Every `text:bookmark`, name to the block holding it — an index over the runs, rebuilt
    /// whenever blocks change rather than stored twice.
    pub bookmarks: BTreeMap<String, BlockId>,
    /// Every style name the file declares — every `style:style` of every family, in
    /// `office:styles` and `office:automatic-styles` — for `undeclared-style` (`doc/dsl.md` §4.3)
    /// to ask whether a `text:style-name` points at anything. Empty for a document this build
    /// wrote or one read from a projection, which carry no style definitions.
    pub styles: std::collections::BTreeSet<String>,
    /// The page the document says it is set on: the `style:page-layout` of its `Standard` master
    /// page, or of its first. Read, never written (the envelope carries the page layouts byte for
    /// byte). `None` means no page was stated, and printing uses A4.
    pub page: Option<grind_core::page::PageGeometry>,
    /// The default paragraph style (`style:default-style style:family="paragraph"`), the root of
    /// every paragraph style chain, read and never written ([`crate::paragraph`]). Boxed, as a
    /// large value read once per print.
    pub default_paragraph: Box<crate::paragraph::ParagraphProps>,
    /// Every paragraph style declared, by name — the named ones of `office:styles` and the body
    /// part's automatic ones. Read, never written; [`crate::paragraph::resolve`] walks a block's
    /// chain.
    pub paragraph_styles: std::collections::HashMap<String, crate::paragraph::ParagraphStyle>,
    /// Every `text:list-style` declared, by name, and which list each item was read in — what a
    /// label is derived from (`crate::numbering`). Read for showing, never written.
    pub list_styles: std::collections::HashMap<String, crate::numbering::ListStyle>,
    pub list_marks: crate::numbering::Marks,
    /// The `text:outline-style` (rng:17530): how headings are numbered. Read, never written;
    /// `None` numbers none.
    pub outline_style: Option<crate::numbering::ListStyle>,
    /// Each declared font family's generic kind — `style:font-family-generic` (rng:10418: `roman`,
    /// `swiss`, `modern`, `decorative`, `script`, `system`) — for a printer's fallback. Read,
    /// never written.
    pub font_generics: std::collections::HashMap<String, String>,
    /// Every named character style declared, by name — what a run's style name stands for. Shown,
    /// never written (`crate::paragraph::shown`).
    pub char_styles: std::collections::HashMap<String, crate::paragraph::NamedChar>,
    /// Each table's look by table name — widths, cell styles, heading rows — read for showing and
    /// printing (`crate::table_look`).
    pub table_looks: std::collections::HashMap<String, crate::table_look::TableLook>,
    /// Every `text:section` in the body, in opening order: its start tag as the file spelled it
    /// and its parent section, so a regenerated body wraps its blocks again ([`Block::section`]).
    pub sections: Vec<crate::model::Section>,
    /// Every footnote and endnote, in the order they are cited (`crate::marginal::Note`).
    pub notes: Vec<crate::marginal::Note>,
    /// The page's header and footer, from [`Document::page`]'s master page; read for printing
    /// (`crate::marginal`).
    pub header: Option<crate::marginal::Marginal>,
    pub footer: Option<crate::marginal::Marginal>,
    /// The first page's own header and footer, when the first paragraph names a master page of its
    /// own and later pages are set on another (Writer's First Page). Read for printing; `None`
    /// when every page wears the same.
    pub first_page: Option<(
        Option<crate::marginal::Marginal>,
        Option<crate::marginal::Marginal>,
    )>,
    /// Every master page by name, and the body's master. A paragraph whose style names a master
    /// starts a page on it (`doc/pdf-export.md`). Read for printing.
    pub masters: BTreeMap<String, Master>,
    pub rest_master: Option<String>,
    /// The next id to hand out, never reused, so a stale [`BlockId`] stays stale.
    next_id: u64,
    /// The file this document was read from, and where its blocks are in it (R6); `None` for a
    /// document authored here.
    pub source: Option<Box<crate::odf::source::Source>>,
    /// The projection this document was read from, if it was one (`doc/dsl.md` §3.1), and where
    /// each block sits in it. A document has at most one of this and `source`.
    pub projection_source: Option<Box<grind_core::projection::Source>>,
    /// What has changed since it was read. See [`crate::odf::source::Edits`].
    pub edits: crate::odf::source::Edits,
}

/// A new document: one empty paragraph, since a caret lives in a block (`loc.rs`) and a document
/// with no blocks has nowhere to type. What `<office:text><text:p/></office:text>` says. Readers
/// use [`Document::empty`].
impl Default for Document {
    fn default() -> Self {
        let mut doc = Document::empty();
        let id = doc.next_id();
        doc.blocks.push(Block::new(id, BlockKind::Paragraph));
        doc
    }
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    /// A document with no blocks, for a reader to fill from a file, which may hold none. A
    /// constructor because `next_id` is private.
    pub fn empty() -> Self {
        Document {
            blocks: Vec::new(),
            bookmarks: BTreeMap::new(),
            styles: std::collections::BTreeSet::new(),
            page: None,
            default_paragraph: Box::default(),
            paragraph_styles: std::collections::HashMap::new(),
            list_styles: std::collections::HashMap::new(),
            list_marks: std::collections::HashMap::new(),
            outline_style: None,
            font_generics: std::collections::HashMap::new(),
            char_styles: std::collections::HashMap::new(),
            table_looks: std::collections::HashMap::new(),
            sections: Vec::new(),
            notes: Vec::new(),
            header: None,
            first_page: None,
            masters: BTreeMap::new(),
            rest_master: None,
            footer: None,
            next_id: 0,
            source: None,
            projection_source: None,
            edits: crate::odf::source::Edits::default(),
        }
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

    /// Rebuild [`Document::bookmarks`] from the runs, after any change that could move or remove
    /// one.
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

    /// The blocks of the table the block at `index` is in — the maximal run of consecutive blocks
    /// naming it — or `None`, for the writer, a shell's grid and the projection alike. A paragraph
    /// between two tables of the same name makes them two.
    pub fn table(&self, index: usize) -> Option<std::ops::Range<usize>> {
        let name = &self.block(index)?.cell.as_ref()?.table;
        let named = |block: &Block| block.cell.as_ref().is_some_and(|c| &c.table == name);
        let start = self.blocks[..index]
            .iter()
            .rposition(|block| !named(block))
            .map_or(0, |before| before + 1);
        let end = self.blocks[index + 1..]
            .iter()
            .position(|block| !named(block))
            .map_or(self.blocks.len(), |offset| index + 1 + offset);
        Some(start..end)
    }

    /// How many rows and columns the table occupying `blocks` has, counting a spanned cell for
    /// every position it covers; derived, not stored.
    pub fn table_extent(&self, blocks: std::ops::Range<usize>) -> (u32, u32) {
        let mut rows = 0;
        let mut columns = 0;
        for cell in self.blocks[blocks].iter().filter_map(|b| b.cell.as_ref()) {
            rows = rows.max(cell.row + cell.rows_spanned.max(1));
            columns = columns.max(cell.column + cell.columns_spanned.max(1));
        }
        (rows, columns)
    }

    /// The blocks belonging to the heading at `index` — itself and everything up to the next
    /// heading at the same level or higher — computed, since the body has no such container
    /// (`doc/odt-format.md` §2). `None` if not a heading.
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
        let mut d = Document::empty();
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
        let mut d = Document::empty();
        let a = d.next_id();
        let b = d.next_id();
        assert_ne!(a, b);
        // A removed block's id never comes back.
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

    /// The offsets that matter are at a seam: 0, the end, and exactly on a run boundary.
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
        // `#intro` at offset 0 names what comes after it, so splitting at 0 must not strand it.
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

        // Two runs with the same style and different targets are two links.
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
        // Empty by every measure here, and the one empty thing that must survive.
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
