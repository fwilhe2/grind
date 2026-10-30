// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ODF word processor core. **\[ODT\]**
//!
//! The `office:text` content model on top of `grind-core`'s packaging and tolerant reader.
//! `doc/odt-format.md` is the clean-room notes (its §5 is unverified and may not be
//! implemented) and `doc/text-core.md` the scope line, which [`implemented`] is checked
//! against by `tests/scope.rs`.
//!
//! Line layout is `grind_core::layout`'s (`doc/text-layout.md`): a shell supplies font metrics
//! through [`Metrics`], and through [`Faces`] which of them each block is set in. [`flow`]
//! stacks the laid-out blocks down a continuous page.
//!
//! A run carries **direct** formatting only ([`style`]): an automatic style is resolved onto
//! the run, a named style such as `Emphasis` stays a name.

mod app;

// The document and what changes it.
pub mod action;
pub mod loc;
pub mod marginal;
pub mod model;
pub mod numbering;
pub mod paragraph;
pub mod style;
pub mod table_look;

// In and out: ODF, Markdown, the projection, and lint.
pub mod commonmark;
pub mod lint;
pub mod odf;
pub mod projection;

// What every shell would otherwise work out for itself, decided here so that no two disagree.
pub mod blocks;
pub mod caret;
pub mod find;
pub mod flow;
pub mod format;
pub mod grammar;
pub mod look;
pub mod markdown;
pub mod page;
pub mod paint;
pub mod picture;
pub mod spell;
pub mod table;
pub mod word;

pub use action::Action;
pub use grind_core::layout::{self, Fixed, Layout, Metrics};
pub use grind_core::{DocumentKind, Error, Form, Observer, Result, kind};
pub use loc::{Caret, Loc, Target};
pub use model::{
    Block, BlockId, BlockKind, Cell, Document, MAX_LIST_DEPTH, NAMED_STYLES, ParagraphDefaults,
    Run, indent_kind, named_style_for,
};
pub use spell::{Lexicon, Misspelling};
pub use style::CharStyle;

use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// Every element of the ODF text content model this crate understands.
///
/// `tests/scope.rs` holds it to `doc/text-core.md` in both directions, so an element cannot be
/// read without being in the scope line, nor listed there without being read.
pub fn implemented() -> Vec<&'static str> {
    vec![
        // Block level
        "text:p",
        "text:h",
        "text:list",
        "text:list-item",
        // Inline
        "text:span",
        "text:s",
        "text:tab",
        "text:line-break",
        "text:a",
        "text:bookmark",
        "draw:frame",
        // Tables: rows and cells are spelled as a spreadsheet's, because `table:table` is one of
        // `text-content`'s own alternatives (rng:16938).
        "table:table",
        "table:table-column",
        "table:table-row",
        "table:table-cell",
        "table:covered-table-cell",
    ]
}

/// Read a `.odt` (package), `.fodt` (flat) or `.grind` (projection) document from bytes, sniffing
/// the form; `name` is only a label.
pub fn read_bytes(_name: &str, bytes: &[u8]) -> Result<Document> {
    // The projection is recognised by its first line.
    if grind_core::projection::is_projection(bytes).is_some() {
        let text = std::str::from_utf8(bytes).map_err(|e| Error::Projection(e.to_string()))?;
        return projection::read(text);
    }
    odf::read(bytes)
}

pub fn read_file(path: &Path) -> Result<Document> {
    read_bytes(&path.display().to_string(), &std::fs::read(path)?)
}

/// Serialise a document. See [`Form`].
pub fn write_bytes(doc: &Document, form: Form) -> Result<Vec<u8>> {
    odf::write(doc, form)
}

/// Write a document, choosing the form from the extension: `.odt` the package, anything else flat
/// ([`Form::from_path`], `doc/flat-first.md`).
pub fn write_file(doc: &Document, path: &Path) -> Result<()> {
    grind_core::atomic::write(path, &write_bytes(doc, Form::from_path(path))?)?;
    Ok(())
}

/// Read a document, refusing one that is not a text document: the tolerant reader would hand a
/// spreadsheet back as an empty text document, so [`kind`](fn@kind) is checked first and the error
/// names the app that opens it.
pub fn open_bytes(name: &str, bytes: &[u8]) -> Result<Document> {
    match kind(bytes) {
        Some(DocumentKind::Text) => read_bytes(name, bytes),
        other => Err(grind_core::Error::UnsupportedKind(other)),
    }
}

/// A window onto the document — what a renderer draws, and the only way a shell reads blocks. No
/// getter hands out the whole document, or a shell would keep its own copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub blocks: Range<usize>,
    items: Vec<BlockView>,
}

/// One block, as a reader sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockView {
    /// Where it is now. 0-based; `loc::format` is the only place this becomes a `p12`.
    pub index: usize,
    pub id: BlockId,
    pub kind: BlockKind,
    /// The named paragraph style, if any.
    pub style: Option<String>,
    /// The block's plain text.
    pub text: String,
    /// The block's content as runs of uniform formatting, in order — the pieces
    /// [`App::layout_block`] measures — carried here so text and formatting are read in one call.
    pub runs: Vec<RunView>,
    /// Whether anything about this block is directly formatted rather than inherited from its
    /// named style — what `grind text formatting` lists.
    pub styled: bool,
    /// The bookmarks anchored inside this block, as (character offset, name), in order
    /// (`doc/view-modes.md` §3.6). A bookmark contributes no characters, so this is what an
    /// overlay draws. Always filled: it costs a walk of the block's runs.
    pub marks: Vec<(usize, String)>,
    /// Whether this is an index's generated entry rather than the document's own text
    /// ([`Block::generated`]) — shown, not edited.
    pub generated: bool,
    /// Where this block sits in a table, if it is in one (`model::Cell`).
    pub cell: Option<model::Cell>,
    /// What a list item wears in front of it — `1.`, `2.a)`, the style's bullet — derived from its
    /// list's style and place (`numbering::labels`), never stored; a heading's number is
    /// [`BlockView::number`]. Empty for a list header or an item's second paragraph; `None` where
    /// the list states no style and a shell draws its own bullet ([`BlockView::mark`]).
    pub label: Option<String>,
}

impl BlockView {
    /// What to draw in front of this block: its list label, a shell's bullet for a list item whose
    /// list states no style, or nothing.
    pub fn mark(&self) -> Option<&str> {
        let BlockKind::ListItem { depth } = self.kind else {
            return None;
        };
        match self.label.as_deref() {
            Some("") => None,
            Some(label) => Some(label),
            None => Some(paint::bullet(depth)),
        }
    }

    /// A heading's number from the outline style — `1`, `2.1.3` — or `None`.
    pub fn number(&self) -> Option<&str> {
        match self.kind {
            BlockKind::Heading { .. } => self.label.as_deref().filter(|label| !label.is_empty()),
            _ => None,
        }
    }

    /// A heading the outline leaves unnumbered (`text:is-list-header`), at its level's left margin
    /// with no label.
    pub fn unnumbered(&self) -> bool {
        matches!(self.kind, BlockKind::Heading { .. }) && self.label.as_deref() == Some("")
    }
}

/// One run of uniformly formatted characters, as a reader sees it. A tab and a line break are one
/// character each, so offsets here are [`Caret`] offsets, and a bookmark contributes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunView {
    /// Where this run starts, in characters from the beginning of the block.
    pub start: usize,
    /// Its characters. `\t` for a `text:tab` and `\n` for a `text:line-break`.
    pub text: String,
    /// The direct character formatting on it.
    pub props: CharStyle,
    /// What it looks like: its named character style's formatting down that style's chain, with
    /// [`RunView::props`] over it (`paragraph::shown`) — what a painter draws and a layout
    /// measures. `props` is what a toolbar reads and writes.
    pub shown: CharStyle,
    /// The named character style it carries ([`crate::style`]).
    pub style: Option<String>,
    /// `xlink:href`, when the run is inside a link.
    pub href: Option<String>,
    /// The image this run is — `Some` only for a [`Run::Image`] — for a shell that draws the
    /// picture rather than the placeholder character.
    ///
    /// ponytail: clones the bytes on every [`App::get_viewport`]; a cache keyed by identity, if a
    /// document with many large images ever needs it.
    pub image: Option<ImageView>,
}

impl RunView {
    /// One past its last character.
    pub fn end(&self) -> usize {
        self.start + self.text.chars().count()
    }
}

/// One embedded image, as a reader sees it — the data half of [`RunView::image`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageView {
    pub mime: String,
    pub data: Vec<u8>,
    /// `svg:width` / `svg:height`, ODF lengths kept verbatim (`"5cm"`), optional.
    pub width: Option<String>,
    pub height: Option<String>,
    /// Its alternative text, short and long (`svg:title`, `svg:desc`) — [`App::set_image_alt`].
    pub title: Option<String>,
    pub description: Option<String>,
}

/// Whether a block is a picture, optionally followed by its caption's text — the shape
/// [`App::insert_image`] makes in an empty paragraph, and how an ODF frame-inside-a-frame reads
/// (`doc/odt-format.md`). A shell draws it as a picture; an image mid-sentence still draws as
/// `\u{fffc}` (`doc/text-shell.md`).
pub fn picture_of(block: &BlockView) -> Option<(&ImageView, Option<&str>)> {
    match block.runs.as_slice() {
        [run] => run.image.as_ref().map(|image| (image, None)),
        [run, caption] if caption.image.is_none() => run
            .image
            .as_ref()
            .map(|image| (image, Some(caption.text.as_str()))),
        _ => None,
    }
}

impl Viewport {
    pub fn get(&self, index: usize) -> Option<&BlockView> {
        self.items.get(index.checked_sub(self.blocks.start)?)
    }

    pub fn iter(&self) -> impl Iterator<Item = &BlockView> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// One table, as a reader sees it — [`App::table`]'s answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// `table:name`, which is the table's identity in a flat sequence ([`model::Cell`]).
    pub name: String,
    /// The blocks it is made of: the maximal run of consecutive blocks naming it.
    pub blocks: Range<usize>,
    /// Counting every position a span covers, so a two-column table with a merged cell is still
    /// two columns wide.
    pub rows: u32,
    pub columns: u32,
}

/// One heading, as the outline lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub index: usize,
    pub level: u32,
    /// The outline path — `[2, 1, 3]` for §2.1.3 — computed, since the document stores no outline
    /// (`doc/odt-format.md` §2).
    pub path: Vec<u32>,
    pub text: String,
}

impl Heading {
    /// The address a user can type back — `§2.1.3`, which survives edits elsewhere where `p12`
    /// would not.
    pub fn address(&self) -> String {
        loc::format_outline(&self.path)
    }
}

/// One search hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub index: usize,
    /// Character offset within the block, so the address is `p12+40`.
    pub offset: usize,
    /// The line the hit is on, for context.
    pub text: String,
}

impl Match {
    pub fn address(&self) -> String {
        loc::format_offset(self.index, self.offset)
    }
}

/// A `table:name` no table here has: `Table1`, `Table2`, …, as LibreOffice names them. A table is
/// the run of blocks naming it, so two adjacent tables sharing a name would be one.
fn unused_table_name(doc: &Document) -> String {
    let taken: std::collections::BTreeSet<&str> = doc
        .blocks
        .iter()
        .filter_map(|b| b.cell.as_ref().map(|c| c.table.as_str()))
        .collect();
    (1..)
        .map(|n| format!("Table{n}"))
        .find(|name| !taken.contains(name.as_str()))
        .unwrap_or_else(|| "Table".to_owned())
}

/// What `words` counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub words: usize,
    pub characters: usize,
    pub blocks: usize,
    pub headings: usize,
}

impl Counts {
    /// How many minutes the document takes to read, rounded up, at 230 words a minute.
    pub fn reading_minutes(&self) -> usize {
        self.words.div_ceil(230)
    }
}

/// Which measure and which [`Metrics`] each block is set in, asked per block as a caret motion
/// reaches it — Down-arrow out of a heading lands in a block set in a different face.
///
/// Called while [`App`] holds its read lock, so an implementation must not call back into it;
/// that is why it is given the block's kind and style name rather than the block.
pub trait Faces {
    /// The measure and the metrics for the block at `index`. `width` is in the provider's unit;
    /// zero or less means do not wrap, as for [`App::layout_block`].
    fn of(&self, index: usize, kind: &BlockKind, style: Option<&str>) -> (f32, &dyn Metrics);

    /// The space above and below the block at `index` when its paragraph style decides it — added
    /// to its neighbours', and above the first block too ([`flow::Flow::push_spaced`]). `None`,
    /// the default, is a screen's own spacing; a printed page answers from the styles.
    fn spacing(&self, _index: usize) -> Option<flow::Space> {
        None
    }

    /// What the block at `index`'s style says about page breaks around it ([`page::Breaks`]).
    /// `None` leaves the paginator's rules: a heading keeps with what follows.
    fn breaks(&self, _index: usize) -> Option<page::Breaks> {
        None
    }

    /// How far the block's first line starts in from its others (`fo:text-indent`), in the face's
    /// unit; zero on screen.
    fn first_indent(&self, _index: usize) -> f32 {
        0.0
    }

    /// Where the block's tab characters go (`style:tab-stops`, `doc/odt-format.md` §5c fact 13),
    /// in the face's unit. No stops, the default, leaves a tab as wide as the metrics make it.
    fn tabs(&self, _index: usize) -> layout::Tabs {
        layout::Tabs::default()
    }

    /// The table named `table`'s column widths, one per column in the face's unit, `None` for a
    /// column stating none. `None` altogether, the default, is equal shares.
    fn columns(&self, _table: &str) -> Option<Vec<Option<f64>>> {
        None
    }

    /// Where the table named `table` sits across a text area `column` wide: its left edge from
    /// the area's, and its width. `None`, the default, is the whole area.
    fn table_span(&self, _table: &str, _column: f64) -> Option<(f64, f64)> {
        None
    }

    /// A cell's own padding — top, right, bottom, left — or `None` for the spacing's.
    fn cell_pad(&self, _table: &str, _row: u32, _column: u32) -> Option<[f64; 4]> {
        None
    }

    /// How far down the room its row leaves a cell's text sits: `0` at the top, `0.5` halfway,
    /// `1` at the bottom (`style:vertical-align`).
    fn cell_drop(&self, _table: &str, _row: u32, _column: u32) -> f64 {
        0.0
    }

    /// How much of the block's space above stays when it starts a page — one the text flowed
    /// onto, and one after a page break (`doc/odt-format.md` §5c fact 34); none on screen.
    fn kept_above(&self, _index: usize) -> (f64, f64) {
        (0.0, 0.0)
    }

    /// How much room the footnotes cited in the block take at the foot of its page, in the face's
    /// unit; zero on screen.
    fn footnote_room(&self, _index: usize) -> f64 {
        0.0
    }

    /// Whether `row` of the table named `table` is a heading row, which a printed page repeats on
    /// every page the table continues on.
    fn header_row(&self, _table: &str, _row: u32) -> bool {
        false
    }

    /// The least height of `row` of the table named `table`, in the face's unit, where its style
    /// states one.
    fn row_height(&self, _table: &str, _row: u32) -> Option<f64> {
        None
    }

    /// Whether `row` of the table named `table` keeps together, moving whole to the next page
    /// rather than splitting between lines.
    fn row_kept(&self, _table: &str, _row: u32) -> bool {
        false
    }
}

/// Every block set alike: one width, one provider — what the CLI (`Fixed` at `--width`) and a
/// terminal want.
pub struct Uniform<'a> {
    width: f32,
    metrics: &'a dyn Metrics,
}

impl<'a> Uniform<'a> {
    pub fn new(width: f32, metrics: &'a dyn Metrics) -> Self {
        Uniform { width, metrics }
    }
}

impl Faces for Uniform<'_> {
    fn of(&self, _index: usize, _kind: &BlockKind, _style: Option<&str>) -> (f32, &dyn Metrics) {
        (self.width, self.metrics)
    }
}

#[derive(Default)]
struct State {
    doc: Document,
    undo: Vec<Action>,
    redo: Vec<Action>,
}

/// The word processor. Shells hold an `Arc<App>` and call these methods; every method takes
/// `&self` and the lock lives inside, as in `grind_sheet::App`.
#[derive(Default)]
pub struct App {
    state: RwLock<State>,
    observer: RwLock<Option<Arc<dyn Observer>>>,
    /// The dictionary spelling is checked against ([`App::set_lexicon`]), beside the document
    /// rather than in it.
    lexicon: RwLock<Option<Arc<dyn spell::Lexicon>>>,
    /// The grammar checker ([`App::set_checker`]), beside the lexicon.
    checker: RwLock<Option<Arc<dyn grammar::Checker>>>,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_observer(&self, observer: Arc<dyn Observer>) {
        *self.observer.write().unwrap() = Some(observer);
    }

    /// Mutate, then notify with the write lock released: an observer calls straight back in to
    /// re-read.
    fn mutate<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let result = {
            let mut state = self.state.write().unwrap();
            f(&mut state)
        };
        let observer = self.observer.read().unwrap().clone();
        if let Some(observer) = observer {
            observer.changed();
        }
        result
    }

    /// Record an action and its inverse in one step.
    fn commit(state: &mut State, action: Action) -> Result<()> {
        let inverse = state
            .doc
            .apply(action)
            .ok_or_else(|| Error::Xml("that block is not in the document".to_owned()))?;
        state.undo.push(inverse);
        state.redo.clear();
        Ok(())
    }

    // --- documents ---

    /// Replace the document. History is dropped: an undo would apply to a document that no longer
    /// exists.
    pub fn open_bytes(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let doc = open_bytes(name, bytes)?;
        self.mutate(|state| {
            state.doc = doc;
            state.undo.clear();
            state.redo.clear();
            Ok(())
        })
    }

    pub fn open_file(&self, path: &Path) -> Result<()> {
        self.open_bytes(&path.display().to_string(), &std::fs::read(path)?)
    }

    /// Serialise the current document. The browser has no filesystem (rule 5), and saving is an
    /// operation, never a getter for the document.
    pub fn save_bytes(&self, form: Form) -> Result<Vec<u8>> {
        write_bytes(&self.state.read().unwrap().doc, form)
    }

    pub fn save_file(&self, path: &Path) -> Result<()> {
        write_file(&self.state.read().unwrap().doc, path)
    }

    // --- reading ---

    /// Read a run of blocks — the only way a shell reads content. Asking past the end comes back
    /// short.
    pub fn get_viewport(&self, blocks: Range<usize>) -> Viewport {
        let state = self.state.read().unwrap();
        let end = blocks.end.min(state.doc.blocks.len());
        let start = blocks.start.min(end);
        let labels = numbering::labels(&state.doc, end);
        let items = state.doc.blocks[start..end]
            .iter()
            .enumerate()
            .map(|(offset, block)| BlockView {
                label: labels.get(start + offset).cloned().flatten(),
                index: start + offset,
                id: block.id,
                kind: block.kind.clone(),
                style: block.style.clone(),
                text: block.text(),
                runs: run_views(block, &state.doc.char_styles),
                styled: block.is_styled(),
                marks: marks(block),
                cell: block.cell.clone(),
                generated: block.generated,
            })
            .collect();
        Viewport {
            blocks: start..end,
            items,
        }
    }

    /// The document as its projection — plain text, with the token and span maps beside it
    /// (`doc/dsl.md` §3): what a `.grind` file holds and what a source view shows. A block is
    /// `p12`, `#intro` and `§2.1.3` at once, and the span map carries all three. Nothing is
    /// written.
    pub fn project(&self) -> projection::Projection {
        projection::project(&self.state.read().unwrap().doc)
    }

    /// Where each address is in the projection this document was read from — the file as written —
    /// in file order; empty unless it was read from a `.grind`. What a language server hovers
    /// over; after an unsaved edit it still describes the file on disk.
    pub fn source_anchors(&self) -> Vec<grind_core::projection::Anchor> {
        self.state
            .read()
            .unwrap()
            .doc
            .projection_source
            .as_deref()
            .map(grind_core::projection::Source::anchors)
            .unwrap_or_default()
    }

    pub fn block_count(&self) -> usize {
        self.state.read().unwrap().doc.blocks.len()
    }

    /// What an editor puts in front of a user for one block: the text that, entered back
    /// unchanged, leaves it as it is. [`App::set_text`]'s inverse.
    pub fn input_text(&self, index: usize) -> Result<String> {
        let state = self.state.read().unwrap();
        state
            .doc
            .block(index)
            .map(Block::text)
            .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(index))))
    }

    /// Resolve an address — `p12`, `#intro`, `§2.1.3` — against the document as it now is.
    pub fn resolve(&self, at: &Loc) -> Result<usize> {
        let state = self.state.read().unwrap();
        loc::resolve(&state.doc, at).map_err(|e| Error::Xml(e.to_string()))
    }

    /// Resolve a range of blocks, inclusive of both ends.
    pub fn resolve_range(&self, range: &loc::Range) -> Result<Range<usize>> {
        let state = self.state.read().unwrap();
        loc::resolve_range(&state.doc, range).map_err(|e| Error::Xml(e.to_string()))
    }

    /// Resolve an address to a [`Caret`] — a block and an offset in it, as `p12+40` names.
    pub fn resolve_caret(&self, at: &Loc) -> Result<Caret> {
        let state = self.state.read().unwrap();
        loc::resolve_caret(&state.doc, at).map_err(|e| Error::Xml(e.to_string()))
    }

    /// Resolve a range of characters, as [`App::erase`] takes it. An end without an offset means
    /// the end of its block, so `p3` is all of p3's text.
    pub fn resolve_caret_range(&self, range: &loc::Range) -> Result<(Caret, Caret)> {
        let state = self.state.read().unwrap();
        loc::resolve_caret_range(&state.doc, range).map_err(|e| Error::Xml(e.to_string()))
    }

    /// The blocks belonging to the heading at `index`: itself and everything up to the next
    /// heading at the same level or higher; `None` if it is not a heading. Computed, since the
    /// body has no such container (`doc/odt-format.md` §2), so `grind text move report.fodt §3.2
    /// §1` moves a section.
    pub fn section(&self, index: usize) -> Option<Range<usize>> {
        self.state.read().unwrap().doc.section(index)
    }

    /// Every heading, with its computed outline path.
    pub fn outline(&self) -> Vec<Heading> {
        let state = self.state.read().unwrap();
        state
            .doc
            .outline()
            .filter_map(|(index, level)| {
                Some(Heading {
                    index,
                    level,
                    path: loc::outline_path(&state.doc, index)?,
                    text: state.doc.block(index)?.text(),
                })
            })
            .collect()
    }

    /// Every block carrying direct formatting or a named style of its own — the answer to "why is
    /// this paragraph different?" (`doc/suite.md`).
    pub fn formatting(&self) -> Vec<BlockView> {
        self.get_viewport(0..self.block_count())
            .iter()
            .filter(|b| b.styled)
            .cloned()
            .collect()
    }

    /// Every occurrence of `needle`, case-sensitively, in document order.
    pub fn find(&self, needle: &str) -> Vec<Match> {
        if needle.is_empty() {
            return Vec::new();
        }
        let state = self.state.read().unwrap();
        let mut hits = Vec::new();
        for (index, block) in state.doc.blocks.iter().enumerate() {
            let text = block.text();
            // `match_indices` gives bytes; addresses count characters.
            for (at, _) in text.match_indices(needle) {
                hits.push(Match {
                    index,
                    offset: text[..at].chars().count(),
                    text: text.clone(),
                });
            }
        }
        hits
    }

    /// [`App::find`], ignoring case. Folded one character at a time to its first lowercase
    /// character, so a hit is as many characters as the needle; `ß` does not match `SS`.
    pub fn find_ignoring_case(&self, needle: &str) -> Vec<Match> {
        let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
        let needle: Vec<char> = needle.chars().map(fold).collect();
        if needle.is_empty() {
            return Vec::new();
        }
        let state = self.state.read().unwrap();
        let mut hits = Vec::new();
        for (index, block) in state.doc.blocks.iter().enumerate() {
            let text = block.text();
            let chars: Vec<char> = text.chars().map(fold).collect();
            let mut at = 0;
            // Non-overlapping, left to right — the same hits `match_indices` gives `find`.
            while at + needle.len() <= chars.len() {
                match chars[at..at + needle.len()] == needle[..] {
                    true => {
                        hits.push(Match {
                            index,
                            offset: at,
                            text: text.clone(),
                        });
                        at += needle.len();
                    }
                    false => at += 1,
                }
            }
        }
        hits
    }

    pub fn counts(&self) -> Counts {
        let state = self.state.read().unwrap();
        let mut counts = Counts {
            blocks: state.doc.blocks.len(),
            ..Counts::default()
        };
        for block in &state.doc.blocks {
            let text = block.text();
            counts.words += text.split_whitespace().count();
            counts.characters += text.chars().count();
            counts.headings += usize::from(block.outline_level().is_some());
        }
        counts
    }

    /// Check the document against [`lint`]'s rules (`doc/dsl.md` §4.3). A read: nothing is stored.
    /// With a [`spell::Lexicon`] attached the `misspelt` rule runs too. A document read from a
    /// `.grind` gets each finding's line and column in that file
    /// ([`grind_core::lint::Report::locate_in`]).
    pub fn lint(&self, options: &grind_core::lint::Options) -> grind_core::lint::Report {
        let lexicon = self.lexicon.read().unwrap().clone();
        let state = self.state.read().unwrap();
        let mut report = lint::lint_with(&state.doc, options, lexicon.as_deref());
        if let Some(source) = state.doc.projection_source.as_deref() {
            report.locate_in(source);
        }
        report
    }

    /// Every bookmark, name and the block it sits in.
    pub fn bookmarks(&self) -> Vec<(String, usize)> {
        let state = self.state.read().unwrap();
        state
            .doc
            .bookmarks
            .iter()
            .filter_map(|(name, id)| Some((name.clone(), state.doc.index_of(*id)?)))
            .collect()
    }

    // --- editing ---

    /// Replace a block's text, keeping its kind and style.
    pub fn set_text(&self, index: usize, text: &str) -> Result<()> {
        self.mutate(|state| {
            let mut block = block_at(state, index)?;
            // Bookmarks are anchors, not content, so retyping a paragraph keeps them.
            block.runs.retain(|r| matches!(r, Run::Bookmark { .. }));
            if !text.is_empty() {
                block.runs.push(Run::plain(text));
            }
            Self::commit(
                state,
                Action::SetBlock {
                    index,
                    block: Box::new(block),
                },
            )
        })
    }

    /// Insert text at a caret — what typing does. One of the caret-level edits a continuous-flow
    /// editor is built from ([`App::erase`], [`App::split_block`], [`App::join_block`]);
    /// [`App::set_text`] replaces a whole block, as a script does.
    ///
    /// The text takes the style of the run at the caret, preferring the one to its left, so typing
    /// at the end of a bold word continues bold. A hyperlink is carried only inside one: text
    /// typed at a link's edge is not part of it ([`App::char_style`] reads by the same rule).
    pub fn insert_text(&self, at: Caret, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        self.mutate(|state| {
            let mut block = block_at(state, at.block)?;
            let offset = at.offset.min(block.len());
            let (mut runs, tail) = model::split_runs(&block.runs, offset);
            let (style, props, href) = caret_formatting(&runs, &tail);
            runs.push(Run::Text {
                text: text.to_owned(),
                style,
                props,
                href,
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

    /// Type text at a caret, reading the markdown-shaped notation as each character lands
    /// ([`crate::markdown`]): `**bold**` becomes bold and its markers go, `` `code` `` becomes
    /// monospace, `# ` makes a heading, ``` fences a code block. Opt-in: [`App::insert_text`]
    /// keeps asterisks as asterisks. One action, so one undo takes back the whole of `**bold**`.
    ///
    /// `resume` ends the notation where its marker does: after a closing marker the next character
    /// would join the formatted run, so [`Typed::resume`] carries the formatting from before the
    /// span. Hand it back on the next call; a whole string typed in one call handles this between
    /// its characters.
    pub fn type_markdown(
        &self,
        at: Caret,
        text: &str,
        resume: Option<&CharStyle>,
    ) -> Result<Typed> {
        if text.is_empty() {
            return Ok(Typed {
                caret: at,
                resume: resume.cloned(),
            });
        }
        let mut resume = resume.cloned();
        self.mutate(|state| {
            let mut block = block_at(state, at.block)?;
            let mut caret = at.offset.min(block.len());
            for c in text.chars() {
                let mut buffer = [0u8; 4];
                let typed = c.encode_utf8(&mut buffer);
                // The character itself, in the caret's formatting or in what the last completed
                // span went back to.
                let (mut runs, tail) = model::split_runs(&block.runs, caret);
                let (style, props, href) = match resume.take() {
                    Some(props) => (None, props, None),
                    None => caret_formatting(&runs, &tail),
                };
                runs.push(Run::Text {
                    text: typed.to_owned(),
                    style,
                    props,
                    href,
                });
                runs.extend(tail);
                model::coalesce(&mut runs);
                block.runs = runs;
                caret += 1;

                // And now: did that character *finish* something?
                resume = apply_notation(&mut block, &mut caret);
            }
            Self::commit(
                state,
                Action::SetBlock {
                    index: at.block,
                    block: Box::new(block),
                },
            )?;
            Ok(Typed {
                caret: Caret {
                    block: at.block,
                    offset: caret,
                },
                resume,
            })
        })
    }

    /// Erase the characters between two carets — what Delete over a selection does. Erasing from
    /// the middle of p3 to the middle of p5 leaves one block with p3's kind and style, as one undo
    /// step.
    ///
    /// Bookmarks inside the span survive, collapsed to the caret, as [`App::set_text`] keeps them;
    /// a block removed entirely takes its anchors with it, as [`App::delete`] does. Returns the
    /// characters removed, each closed block boundary counting one, as [`Document::text`] counts
    /// it.
    pub fn erase(&self, from: Caret, to: Caret) -> Result<usize> {
        self.mutate(|state| {
            if to < from {
                return Err(Error::Xml("that range runs backwards".to_owned()));
            }
            let mut first = block_at(state, from.block)?;
            let last = block_at(state, to.block)?;
            let start = from.offset.min(first.len());
            let end = to.offset.min(last.len());
            if from.block == to.block && start == end {
                return Ok(0);
            }

            let anchors = |runs: Vec<Run>| {
                runs.into_iter()
                    .filter(|run| matches!(run, Run::Bookmark { .. }))
            };
            let (mut runs, rest) = model::split_runs(&first.runs, start);
            let removed;
            if from.block == to.block {
                let (cut, tail) = model::split_runs(&rest, end - start);
                removed = end - start;
                runs.extend(anchors(cut));
                runs.extend(tail);
            } else {
                let (cut, tail) = model::split_runs(&last.runs, end);
                // The first block's tail, everything between, the last block's head, and one per
                // closed boundary.
                removed = (first.len() - start)
                    + state.doc.blocks[from.block + 1..to.block]
                        .iter()
                        .map(Block::len)
                        .sum::<usize>()
                    + end
                    + (to.block - from.block);
                runs.extend(anchors(rest));
                runs.extend(anchors(cut));
                runs.extend(tail);
            }
            model::coalesce(&mut runs);
            first.runs = runs;

            let mut batch = vec![Action::SetBlock {
                index: from.block,
                block: Box::new(first),
            }];
            // Each removal shifts the rest down, so the same index removes the next one.
            batch.extend((from.block..to.block).map(|_| Action::RemoveBlock {
                index: from.block + 1,
            }));
            Self::commit(state, Action::Batch(batch))?;
            Ok(removed)
        })
    }

    /// Split a block at a caret — what Return does. The second half keeps the first's kind and
    /// style, so Return in a list item makes another item at the same depth. A heading split at
    /// its very end leaves a body paragraph behind it; split in the middle, it makes two headings.
    pub fn split_block(&self, at: Caret) -> Result<()> {
        self.mutate(|state| {
            let mut block = block_at(state, at.block)?;
            let offset = at.offset.min(block.len());
            let (mut head, mut tail) = model::split_runs(&block.runs, offset);
            model::coalesce(&mut head);
            model::coalesce(&mut tail);

            let id = state.doc.next_id();
            let mut second = Block::new(id, block.kind.clone());
            second.style = block.style.clone();
            // And its table cell: Return inside a cell makes a second block in that cell, or the
            // table would be cut in two at the caret (`model::Cell`).
            second.cell = block.cell.clone();
            second.runs = tail;
            if second.is_empty() && matches!(block.kind, BlockKind::Heading { .. }) {
                second.kind = BlockKind::Paragraph;
                // Only the kind and style change; the cell stays. A body paragraph wearing
                // `Heading_20_1` would look like a heading without being one.
                second.style = None;
            }
            block.runs = head;

            Self::commit(
                state,
                Action::Batch(vec![
                    Action::SetBlock {
                        index: at.block,
                        block: Box::new(block),
                    },
                    Action::InsertBlock {
                        index: at.block + 1,
                        block: Box::new(second),
                    },
                ]),
            )
        })
    }

    /// Join a block with the one after it — Backspace at the front of a block. The first block's
    /// kind and style survive, so backspacing at the front of a paragraph after a heading pulls
    /// the text into the heading.
    pub fn join_block(&self, index: usize) -> Result<()> {
        self.mutate(|state| {
            let mut block = block_at(state, index)?;
            let next = state
                .doc
                .block(index + 1)
                .cloned()
                .ok_or_else(|| Error::Xml(format!("nothing follows {}", loc::format(index))))?;
            // Never across a table cell's edge: the survivor's cell would swallow the other's text
            // and leave a cell with no block, which a save writes as an empty cell. Backspace at
            // the front of a cell does nothing in every word processor.
            let same_cell = match (&block.cell, &next.cell) {
                (None, None) => true,
                (Some(a), Some(b)) => a.is_same(b),
                _ => false,
            };
            if !same_cell {
                return Err(Error::Xml(format!(
                    "{} and the block after it are in different table cells",
                    loc::format(index)
                )));
            }
            block.runs.extend(next.runs);
            model::coalesce(&mut block.runs);
            Self::commit(
                state,
                Action::Batch(vec![
                    Action::SetBlock {
                        index,
                        block: Box::new(block),
                    },
                    Action::RemoveBlock { index: index + 1 },
                ]),
            )
        })
    }

    /// Replace every occurrence of `needle`. Returns how many blocks changed.
    pub fn replace(&self, needle: &str, with: &str) -> Result<usize> {
        if needle.is_empty() {
            return Ok(0);
        }
        self.mutate(|state| {
            let mut batch = Vec::new();
            for index in 0..state.doc.blocks.len() {
                let mut block = block_at(state, index)?;
                let mut hit = false;
                for run in &mut block.runs {
                    if let Run::Text { text, .. } = run
                        && text.contains(needle)
                    {
                        *text = text.replace(needle, with);
                        hit = true;
                    }
                }
                if hit {
                    batch.push(Action::SetBlock {
                        index,
                        block: Box::new(block),
                    });
                }
            }
            let changed = batch.len();
            if changed > 0 {
                Self::commit(state, Action::Batch(batch))?;
            }
            Ok(changed)
        })
    }

    /// Put a bookmark on a block, or with `None` remove one by name. It moves with the text, so
    /// `#intro` keeps naming the same place.
    pub fn set_bookmark(&self, name: &str, index: Option<usize>) -> Result<bool> {
        self.mutate(|state| {
            let mut batch = Vec::new();
            // Remove an existing anchor of this name first, so setting one twice moves it.
            for i in 0..state.doc.blocks.len() {
                let mut block = block_at(state, i)?;
                let before = block.runs.len();
                block
                    .runs
                    .retain(|r| !matches!(r, Run::Bookmark { name: n } if n == name));
                if block.runs.len() != before {
                    batch.push(Action::SetBlock {
                        index: i,
                        block: Box::new(block),
                    });
                }
            }
            let existed = !batch.is_empty();
            if let Some(index) = index {
                let mut block = block_at(state, index)?;
                // A block already in the batch is edited from the batch's version, or this
                // insertion would undo the removal above.
                if let Some(Action::SetBlock { block: pending, .. }) = batch
                    .iter()
                    .find(|a| matches!(a, Action::SetBlock { index: i, .. } if *i == index))
                {
                    block = (**pending).clone();
                }
                block.runs.insert(
                    0,
                    Run::Bookmark {
                        name: name.to_owned(),
                    },
                );
                batch.retain(|a| !matches!(a, Action::SetBlock { index: i, .. } if *i == index));
                batch.push(Action::SetBlock {
                    index,
                    block: Box::new(block),
                });
            }
            if batch.is_empty() {
                return Ok(false);
            }
            Self::commit(state, Action::Batch(batch))?;
            Ok(existed || index.is_some())
        })
    }

    // --- history ---

    /// Undo the last change. `false` if there was nothing to undo.
    pub fn undo(&self) -> bool {
        self.mutate(|state| {
            let Some(action) = state.undo.pop() else {
                return false;
            };
            // The inverse of an inverse is the original, so this is also the redo entry.
            match state.doc.apply(action) {
                Some(inverse) => {
                    state.redo.push(inverse);
                    true
                }
                None => false,
            }
        })
    }

    /// Redo the last undone change. `false` if there was nothing to redo.
    pub fn redo(&self) -> bool {
        self.mutate(|state| {
            let Some(action) = state.redo.pop() else {
                return false;
            };
            match state.doc.apply(action) {
                Some(inverse) => {
                    state.undo.push(inverse);
                    true
                }
                None => false,
            }
        })
    }

    pub fn can_undo(&self) -> bool {
        !self.state.read().unwrap().undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.state.read().unwrap().redo.is_empty()
    }
}

/// One hyperlink, whole — what [`App::link_at`] finds around a caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub from: Caret,
    pub to: Caret,
    /// `xlink:href`, verbatim: a URL, or `#name` for a bookmark in this document.
    pub href: String,
}

fn run_href(run: &Run) -> Option<&str> {
    match run {
        Run::Text { href, .. } => href.as_deref(),
        _ => None,
    }
}

/// What [`App::type_markdown`] did: where the caret ended up, and what the next character typed
/// there must be set in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typed {
    pub caret: Caret,
    /// `Some` when the last character closed an emphasis — see [`App::type_markdown`].
    pub resume: Option<CharStyle>,
}

/// Read the notation off a block just typed into and apply whatever it completed, in memory, so
/// `**bold**` is one `SetBlock` and one undo step. Returns the formatting the next character
/// needs, `None` when nothing completed.
fn apply_notation(block: &mut Block, caret: &mut usize) -> Option<CharStyle> {
    let text = block.text();

    // A fence toggles the block between preformatted and plain, and removes its three markers.
    if markdown::is_fence(&text, *caret) {
        block.runs = without(block, 0, *caret);
        *caret = 0;
        block.style = match block.style.as_deref() == Some(markdown::PREFORMATTED) {
            true => None,
            false => Some(markdown::PREFORMATTED.to_owned()),
        };
        return None;
    }

    // `# ` and `- ` — the block's own kind, and the prefix gone.
    if let Some((width, kind)) = markdown::block_prefix(&text, *caret) {
        block.runs = without(block, 0, width);
        *caret -= width;
        block.kind = kind;
        return None;
    }

    // `[label](target)`: the markers go and the label is linked, keeping its formatting. The next
    // character is typed outside the link by `link_around`'s rule; the label's formatting is
    // handed back regardless, which ends a span.
    if let Some(found) = markdown::linked(&text, *caret) {
        let before = props_at(block, found.open);
        block.runs = without(block, found.end, found.close);
        block.runs = without(block, found.open, found.start);
        let end = found.open + (found.end - found.start);
        if let Some(runs) = relinked(block, found.open, end, Some(&found.href)) {
            block.runs = runs;
        }
        *caret = end;
        return Some(before);
    }

    let found = markdown::emphasised(&text, *caret)?;
    // The span's formatting before emphasis, read from the opening marker, which was typed in the
    // surrounding formatting.
    let before = props_at(block, found.open);
    // The closing marker first: removing the opening one would shift these offsets.
    block.runs = without(block, found.end, found.close);
    block.runs = without(block, found.open, found.start);
    let end = found.open + (found.end - found.start);
    if let Some(runs) = restyled(block, found.open, end, &found.emphasis.style()) {
        block.runs = runs;
    }
    *caret = end;
    Some(before)
}

/// A block's runs with the characters in `from..to` removed.
fn without(block: &Block, from: usize, to: usize) -> Vec<Run> {
    let (mut head, rest) = model::split_runs(&block.runs, from);
    let (_, tail) = model::split_runs(&rest, to.saturating_sub(from));
    head.extend(tail);
    model::coalesce(&mut head);
    head
}

/// The direct formatting of the run covering `offset` — what text after a closing marker goes back
/// to.
fn props_at(block: &Block, offset: usize) -> CharStyle {
    let (head, tail) = model::split_runs(&block.runs, offset);
    caret_formatting(&head, &tail).1
}

/// The style and hyperlink newly typed text takes, from the runs either side of the caret: the
/// run before decides, or at the front of a block the run after. A tab, a break or a bookmark
/// carries no formatting, so the search is for the nearest text run, stopping at anything else.
fn caret_formatting(head: &[Run], tail: &[Run]) -> (Option<String>, CharStyle, Option<String>) {
    let neighbour = head.last().or_else(|| tail.first());
    match neighbour {
        Some(Run::Text { style, props, .. }) => (
            style.clone(),
            props.clone(),
            link_around(head, tail).map(str::to_owned),
        ),
        _ => (None, CharStyle::default(), None),
    }
}

/// The link a caret is strictly inside — the same target on both sides — or `None`. What typed
/// text joins ([`App::insert_text`]).
fn link_around<'a>(head: &'a [Run], tail: &[Run]) -> Option<&'a str> {
    let href = |run: Option<&Run>| match run {
        Some(Run::Text { href, .. }) => href.clone(),
        _ => None,
    };
    let left = match head.last() {
        Some(Run::Text { href, .. }) => href.as_deref()?,
        _ => return None,
    };
    (href(tail.first()).as_deref() == Some(left)).then_some(left)
}

/// This block's runs with `start..end` linked to `href` (or unlinked), or `None` when nothing
/// changes — [`restyled`]'s twin.
fn relinked(block: &Block, start: usize, end: usize, href: Option<&str>) -> Option<Vec<Run>> {
    let len = block.len();
    let (start, end) = (start.min(len), end.min(len));
    if start >= end {
        return None;
    }
    let (head, rest) = model::split_runs(&block.runs, start);
    let (mut middle, tail) = model::split_runs(&rest, end - start);
    let mut changed = false;
    for run in &mut middle {
        if let Run::Text { href: own, .. } = run
            && own.as_deref() != href
        {
            *own = href.map(str::to_owned);
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    let mut runs = head;
    runs.extend(middle);
    runs.extend(tail);
    model::coalesce(&mut runs);
    Some(runs)
}

/// Break one block into lines in the face that block is set in — [`lay_out`] with the width and
/// provider looked up in `faces`, so every caret operation asks the same way.
fn set_out(block: &Block, index: usize, faces: &dyn Faces, named: &Named) -> Layout {
    let (width, metrics) = faces.of(index, &block.kind, block.style.as_deref());
    lay_out(block, width, metrics, named)
}

/// The named character styles a layout measures runs by ([`paragraph::shown`]).
type Named = std::collections::HashMap<String, paragraph::NamedChar>;

/// Break one block into lines: runs become [`layout::Fragment`]s, one per run, so caret offsets
/// and layout offsets are the same numbers (a bookmark is an empty fragment). Each run measures
/// with what it looks like — its named character style under its direct formatting
/// ([`paragraph::shown`]) — so a bold word is measured bold.
fn lay_out(block: &Block, width: f32, metrics: &dyn Metrics, named: &Named) -> Layout {
    lay_out_indented(block, width, metrics, 0.0, &layout::Tabs::default(), named)
}

fn lay_out_indented(
    block: &Block,
    width: f32,
    metrics: &dyn Metrics,
    first: f32,
    tabs: &layout::Tabs,
    named: &Named,
) -> Layout {
    let default = grind_core::style::TextStyle::default();
    // Measured as shown: a run's named style is part of how wide it is.
    let styles: Vec<grind_core::style::TextStyle> = block
        .runs
        .iter()
        .map(|run| match run {
            Run::Text { style, props, .. } => {
                paragraph::shown(named, style.as_deref(), props).metrics()
            }
            _ => run.props().map(CharStyle::metrics).unwrap_or_default(),
        })
        .collect();
    let mut fragments: Vec<layout::Fragment<'_>> = block
        .runs
        .iter()
        .zip(&styles)
        .map(|(run, style)| layout::Fragment {
            text: run.text(),
            style,
        })
        .collect();
    // An empty paragraph is still one line tall, and `wrap` takes its height from its fragments,
    // so it gets an empty one to ask about.
    if fragments.is_empty() {
        fragments.push(layout::Fragment {
            text: "",
            style: &default,
        });
    }
    layout::wrap_tabbed(&fragments, width, metrics, first, tabs)
}

/// The formatting of every run `start..end` touches, in order. An empty range reports one: the run
/// at the caret, preferring the left, as [`caret_formatting`] decides.
fn spanned(block: &Block, start: usize, end: usize) -> Vec<CharStyle> {
    if start == end {
        // Cut at the caret and ask what `insert_text` asks, through the same primitive, so the
        // toolbar never disagrees with the next keystroke.
        let (head, tail) = model::split_runs(&block.runs, start);
        let (_, props, _) = caret_formatting(&head, &tail);
        return vec![props];
    }
    let mut out = Vec::new();
    let mut at = 0usize;
    for run in &block.runs {
        let len = run.len();
        if let Some(props) = run.props()
            && at < end
            && at + len > start
        {
            out.push(props.clone());
        }
        at += len;
    }
    out
}

/// This block's runs with `start..end` restyled, or `None` when nothing changes:
/// [`model::split_runs`] at both ends, the middle rewritten, so a run half inside the span is
/// split.
fn restyled(block: &Block, start: usize, end: usize, style: &CharStyle) -> Option<Vec<Run>> {
    let len = block.len();
    let (start, end) = (start.min(len), end.min(len));
    if start >= end {
        return None;
    }
    let (head, rest) = model::split_runs(&block.runs, start);
    let (mut middle, tail) = model::split_runs(&rest, end - start);
    if middle
        .iter()
        .all(|run| run.props().is_none_or(|props| props == style))
    {
        return None;
    }
    for run in &mut middle {
        if let Run::Text { props, .. } = run {
            // What the model does not read stays where it was read from.
            let origin = std::mem::take(&mut props.origin);
            *props = style.clone();
            props.origin = origin;
        }
    }
    let mut runs = head;
    runs.extend(middle);
    runs.extend(tail);
    model::coalesce(&mut runs);
    Some(runs)
}

/// Where each bookmark in a block sits, in characters from its start ([`BlockView::marks`]),
/// counted as [`run_views`] and a [`Caret`] count.
fn marks(block: &Block) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    for run in &block.runs {
        match run {
            Run::Bookmark { name } => out.push((at, name.clone())),
            _ => at += run.len(),
        }
    }
    out
}

/// One block's runs as a reader sees them. Bookmarks are dropped: they contribute no characters.
fn run_views(
    block: &Block,
    named: &std::collections::HashMap<String, paragraph::NamedChar>,
) -> Vec<RunView> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for run in &block.runs {
        let len = run.len();
        if len == 0 {
            continue;
        }
        let (style, href) = match run {
            Run::Text { style, href, .. } => (style.clone(), href.clone()),
            _ => (None, None),
        };
        let image = match run {
            Run::Image {
                mime,
                data,
                width,
                height,
                title,
                description,
                ..
            } => Some(ImageView {
                mime: mime.clone(),
                data: data.clone(),
                width: width.clone(),
                height: height.clone(),
                title: title.clone(),
                description: description.clone(),
            }),
            _ => None,
        };
        let props = run.props().cloned().unwrap_or_default();
        out.push(RunView {
            start,
            text: run.text().to_owned(),
            shown: paragraph::shown(named, style.as_deref(), &props),
            props,
            style,
            href,
            image,
        });
        start += len;
    }
    out
}

/// A clone of one block, or an error naming the address a user typed.
fn block_at(state: &State, index: usize) -> Result<Block> {
    state
        .doc
        .block(index)
        .cloned()
        .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(index))))
}

/// What `grind-spell` chooses a language through: each method the inherent one, and
/// [`spell::sample`] for the guess.
impl grind_core::spell::Spelled for App {
    fn set_lexicon(&self, lexicon: Option<Arc<dyn spell::Lexicon>>) {
        App::set_lexicon(self, lexicon);
    }

    fn spelling_language(&self) -> Option<String> {
        App::spelling_language(self)
    }

    fn stated_language(&self) -> Option<String> {
        self.language()
    }

    fn sample(&self) -> Vec<String> {
        spell::sample(&self.state.read().unwrap().doc.blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat text document with `body` as its `office:text` content.
    fn fodt(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:xlink="http://www.w3.org/1999/xlink"
  office:mimetype="application/vnd.oasis.opendocument.text"
  office:version="1.4">
  <office:body><office:text>{body}</office:text></office:body>
</office:document>"#
        )
    }

    fn read(body: &str) -> Document {
        odf::read(fodt(body).as_bytes()).expect("the document parses")
    }

    fn image_run(mime: &str) -> RunView {
        RunView {
            start: 0,
            text: "\u{fffc}".to_owned(),
            props: CharStyle::default(),
            shown: CharStyle::default(),
            style: None,
            href: None,
            image: Some(ImageView {
                mime: mime.to_owned(),
                data: Vec::new(),
                width: None,
                height: None,
                title: None,
                description: None,
            }),
        }
    }

    fn text_run(start: usize, text: &str) -> RunView {
        RunView {
            start,
            text: text.to_owned(),
            props: CharStyle::default(),
            shown: CharStyle::default(),
            style: None,
            href: None,
            image: None,
        }
    }

    fn block(runs: Vec<RunView>) -> BlockView {
        BlockView {
            index: 0,
            id: BlockId(0),
            kind: BlockKind::Paragraph,
            style: None,
            text: String::new(),
            runs,
            styled: false,
            marks: Vec::new(),
            cell: None,
            generated: false,
            label: None,
        }
    }

    #[test]
    fn a_lone_image_run_is_a_picture_with_no_caption() {
        let view = block(vec![image_run("image/png")]);
        let (image, caption) = picture_of(&view).expect("reads as a picture");
        assert_eq!(image.mime, "image/png");
        assert_eq!(caption, None);
    }

    #[test]
    fn an_image_then_text_is_a_picture_with_a_caption() {
        let view = block(vec![image_run("image/png"), text_run(1, "A caption")]);
        let (_, caption) = picture_of(&view).expect("reads as a picture");
        assert_eq!(caption, Some("A caption"));
    }

    #[test]
    fn text_before_the_image_is_not_a_picture() {
        let view = block(vec![text_run(0, "Some words "), image_run("image/png")]);
        assert_eq!(
            picture_of(&view),
            None,
            "an image mid-sentence is not a figure"
        );
    }

    #[test]
    fn a_second_image_is_not_a_caption() {
        let view = block(vec![image_run("image/png"), image_run("image/png")]);
        assert_eq!(picture_of(&view), None);
    }

    #[test]
    fn paragraphs_and_headings_come_back_in_order() {
        let doc = read(
            r#"<text:h text:outline-level="1">Title</text:h>
               <text:p>First.</text:p>
               <text:p>Second.</text:p>"#,
        );
        assert_eq!(doc.blocks.len(), 3);
        assert_eq!(doc.blocks[0].outline_level(), Some(1));
        assert_eq!(doc.blocks[0].text(), "Title");
        assert_eq!(doc.blocks[1].text(), "First.");
        assert_eq!(doc.text(), "Title\nFirst.\nSecond.");
    }

    #[test]
    fn text_s_is_expanded_because_xml_would_otherwise_lose_the_spaces() {
        // rng:8408 — ODF's run-length encoding of spaces.
        let doc = read(r#"<text:p>a<text:s text:c="4"/>b<text:s/>c</text:p>"#);
        assert_eq!(doc.blocks[0].text(), "a    b c");
    }

    #[test]
    fn a_repeat_count_is_clamped_rather_than_believed() {
        // §9: four billion spaces is a memory-exhaustion vector, not an intent.
        let doc = read(r#"<text:p><text:s text:c="4000000000"/></text:p>"#);
        assert!(doc.blocks[0].len() <= 4096);
    }

    #[test]
    fn nested_spans_compose_their_style_names() {
        // doc/text-core.md: the model is flat, so reading composes the stack down each branch.
        let doc = read(
            r#"<text:p>plain <text:span text:style-name="B">bold \
<text:span text:style-name="I">both</text:span></text:span></text:p>"#,
        );
        let runs = &doc.blocks[0].runs;
        let styled: Vec<_> = runs
            .iter()
            .filter_map(|r| match r {
                Run::Text { text, style, .. } => Some((text.as_str(), style.as_deref())),
                _ => None,
            })
            .collect();
        assert_eq!(styled[0].1, None, "text outside any span carries no style");
        assert_eq!(styled[1].1, Some("B"));
        assert_eq!(styled[2].1, Some("B I"), "outermost first, both kept");
    }

    #[test]
    fn a_list_flattens_into_blocks_that_know_their_depth() {
        let doc = read(
            r#"<text:list><text:list-item><text:p>one</text:p></text:list-item>
                 <text:list-item><text:p>two</text:p>
                   <text:list><text:list-item><text:p>two a</text:p></text:list-item></text:list>
                 </text:list-item></text:list>"#,
        );
        let kinds: Vec<_> = doc.blocks.iter().map(|b| b.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::ListItem { depth: 1 },
                BlockKind::ListItem { depth: 1 },
                BlockKind::ListItem { depth: 2 },
            ]
        );
        assert_eq!(doc.blocks[2].text(), "two a");
    }

    #[test]
    fn tabs_and_breaks_are_elements_rather_than_characters() {
        let doc = read(r#"<text:p>a<text:tab/>b<text:line-break/>c</text:p>"#);
        assert!(matches!(doc.blocks[0].runs[1], Run::Tab));
        assert_eq!(doc.blocks[0].text(), "a\tb\nc");
    }

    #[test]
    fn a_hyperlink_carries_its_target_onto_the_text_inside_it() {
        let doc = read(r#"<text:p>see <text:a xlink:href="https://x/">here</text:a>.</text:p>"#);
        let hrefs: Vec<_> = doc.blocks[0]
            .runs
            .iter()
            .filter_map(|r| match r {
                Run::Text { text, href, .. } => Some((text.as_str(), href.as_deref())),
                _ => None,
            })
            .collect();
        assert_eq!(hrefs[0], ("see ", None));
        assert_eq!(hrefs[1], ("here", Some("https://x/")));
        assert_eq!(hrefs[2], (".", None), "the href closes with the element");
    }

    #[test]
    fn a_bookmark_is_indexed_and_contributes_no_text() {
        let doc = read(r#"<text:p><text:bookmark text:name="intro"/>Hello</text:p>"#);
        assert_eq!(doc.blocks[0].text(), "Hello");
        assert_eq!(doc.bookmarks.get("intro"), Some(&doc.blocks[0].id));
    }

    #[test]
    fn everything_outside_the_scope_line_is_inert_rather_than_an_error() {
        // §8's design in a second document type: none of these cost reader code, and the
        // paragraphs around them still read.
        let doc = read(
            r#"<text:p>before</text:p>
               <text:section text:name="s"><text:p>inside a section</text:p></text:section>
               <text:table-of-content><text:index-body><text:p>TOC</text:p></text:index-body></text:table-of-content>
               <text:bibliography/>
               <office:annotation><text:p>a comment</text:p></office:annotation>
               <text:p>after</text:p>"#,
        );
        // A section's text and a table of contents' generated entries are read (sections are
        // the document's own text; an index's entries are shown, never written); a comment and
        // an empty bibliography are still inert.
        assert_eq!(
            doc.text(),
            "before\ninside a section\nTOC\nafter",
            "an unrecognised subtree is swallowed whole, contents included"
        );
    }

    #[test]
    fn an_unbounded_outline_level_loads_because_the_schema_permits_one() {
        // rng:6867 — positiveInteger, no ceiling. Tolerance on the way in (R5).
        let doc = read(r#"<text:h text:outline-level="9">deep</text:h>"#);
        assert_eq!(doc.blocks[0].outline_level(), Some(9));
        // A heading that says it is a heading is one, whatever else it fails to say.
        let doc = read(r#"<text:h>no level</text:h>"#);
        assert_eq!(doc.blocks[0].outline_level(), Some(1));
    }

    #[test]
    fn the_prefix_carries_no_meaning() {
        // §8.1: dispatch is on the URI, so different prefixes read the same.
        let odd = r#"<?xml version="1.0" encoding="UTF-8"?>
<ns0:document xmlns:ns0="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:zz="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  ns0:mimetype="application/vnd.oasis.opendocument.text">
  <ns0:body><ns0:text><zz:p>hello</zz:p></ns0:text></ns0:body>
</ns0:document>"#;
        let doc = odf::read(odd.as_bytes()).expect("parses");
        assert_eq!(doc.text(), "hello");
    }

    #[test]
    fn a_spreadsheet_is_refused_rather_than_read_as_an_empty_document() {
        let ods = br#"<?xml version="1.0"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  office:mimetype="application/vnd.oasis.opendocument.spreadsheet"/>"#;
        let err = open_bytes("book.ods", ods).expect_err("not a text document");
        assert!(
            err.to_string().contains("grind sheet"),
            "the error names the app that does open it: {err}"
        );
    }
}
