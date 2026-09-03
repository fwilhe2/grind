// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ODF word processor core. **\[ODT\]**
//!
//! The second document type of the suite (`doc/suite.md`, phase 10). It builds on
//! `grind-core` exactly as `grind-sheet` does — same packaging, same namespace resolution,
//! same tolerant element-context stack — and adds the one thing that is its own: the
//! `office:text` content model.
//!
//! Two documents are normative here and are worth reading before the code:
//!
//! * **`doc/odt-format.md`** — the clean-room notes. Every structural claim cites
//!   `doc/OpenDocument-v1.4-schema.rng` by line, and §5 is a list of things about LibreOffice
//!   that are **not yet verified** and may not be implemented until they are.
//! * **`doc/text-core.md`** — the scope line. Unlike `doc/small-group.md` it was *invented*
//!   rather than extracted, because ODF defines no evaluator tier for text — which is exactly
//!   why [`implemented`] is checked against it by `tests/scope.rs` rather than trusted.
//!
//! **Where this build is.** S4–S7: the model, addressing, the reader, the writer in both forms
//! with R6 splicing (a `.fodt` lives in git the way a `.fods` does), loop C green both
//! directions, the `App` the CLI drives, and the caret-level edits a continuous-flow editor is
//! made of — [`App::insert_text`], [`App::erase`], [`App::split_block`], [`App::join_block`].
//!
//! **Layout lives in `grind_core::layout`** and this crate drives it — `doc/text-layout.md`,
//! decided on Path C. Line breaking, and every caret operation defined in terms of a line, are
//! in the core so that three shells cannot disagree about where Down-arrow goes; the shell
//! supplies font metrics through [`Metrics`] and nothing else — plus, through [`Faces`], which
//! of them each block is set in, because a heading and the paragraph under it are not the same
//! font and a motion by line crosses between them. Pagination is still gated, and layout is
//! left-to-right only by explicit decision.
//!
//! **Character formatting is direct formatting** ([`style`]). A run carries the properties an
//! `office:automatic-styles` entry set on it — bold, italic, a family, a size, a colour — and
//! [`App::set_char_style`] writes them back as a generated `style:style` that LibreOffice reads
//! (loop C compares it, in the "out" direction, character by character). What a run does *not*
//! carry is what a **named** style means: `Emphasis` stays a name, because the name is the
//! document's own vocabulary and resolving it would throw structure away to draw it.
//!
//! **What it does not have.** A session, so no `undo` across CLI invocations; tables,
//! footnotes and fields; style *definitions*, so a style **name** this build writes does not
//! survive LibreOffice, and a run carrying only one measures with the default; paragraph-level
//! properties, which is why LibreOffice hoisting a uniform paragraph's font out of its runs
//! reads here as formatting lost; pages.

pub mod action;
pub mod lint;
pub mod loc;
pub mod markdown;
pub mod model;
pub mod odf;
pub mod projection;
pub mod style;

pub use action::Action;
pub use grind_core::layout::{self, Fixed, Layout, Metrics};
pub use grind_core::{DocumentKind, Error, Form, Observer, Result, kind};
pub use loc::{Caret, Loc, Target};
pub use model::{Block, BlockId, BlockKind, Document, Run};
pub use style::CharStyle;

use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// Every element of the ODF text content model this build understands.
///
/// The mechanical half of `doc/text-core.md`, and the anti-bloat rule made checkable: adding
/// an element to the reader without adding it to that document fails the build, and so does
/// listing one there that nothing implements (`tests/scope.rs`).
///
/// This is `grind_sheet::formula::funcs::implemented()`'s counterpart, and it matters more:
/// the spreadsheet's scope line could be *extracted* from a normative tier, so drifting from
/// it would contradict a specification. Text has no such tier, so the only thing between this
/// list and a wish list is the test.
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
    ]
}

/// Read a `.odt` (package), `.fodt` (flat) or `.grind` (projection) document from bytes.
///
/// Paired with [`read_file`] from the start because the browser has no filesystem, and this
/// is not retrofittable later (doc/plan.md, rule 5).
///
/// The form is sniffed from the bytes, so `name` is only ever a label for diagnostics.
pub fn read_bytes(_name: &str, bytes: &[u8]) -> Result<Document> {
    // The projection is the third physical form, and `grind_core::projection` decides whether
    // these bytes are one from their first line — never from the file's name, which is the
    // same rule `Form` and `kind` already follow.
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

/// Write a document, choosing the form from the extension — `.odt` the package,
/// **anything else flat** ([`Form::from_path`], `doc/flat-first.md`).
pub fn write_file(doc: &Document, path: &Path) -> Result<()> {
    std::fs::write(path, write_bytes(doc, Form::from_path(path))?)?;
    Ok(())
}

/// Read a document, refusing one that is not a text document.
///
/// The reader is tolerant by construction (§8), so handing it a spreadsheet produces an empty
/// text document rather than an error — which is exactly wrong for a user who opened the
/// wrong file. [`kind`](fn@kind) is checked first, and the error names the app that does
/// open it.
pub fn open_bytes(name: &str, bytes: &[u8]) -> Result<Document> {
    match kind(bytes) {
        Some(DocumentKind::Text) => read_bytes(name, bytes),
        other => Err(grind_core::Error::UnsupportedKind(other)),
    }
}

/// A window onto the document — what a renderer draws, and the only way a shell reads blocks.
///
/// `doc/plan.md` rule 1, applied to a second document type: **no getter hands out the whole
/// document.** The spreadsheet's reason was that a million rows do not fit; the reason here is
/// the same one underneath — a shell that could take the whole `Vec<Block>` would keep its own
/// copy, and then there would be two documents.
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
    /// The block's content split into runs of uniform formatting, in order — the same pieces
    /// [`App::layout_block`] measures, so a shell that draws bold text can walk both together.
    ///
    /// Present here rather than behind a getter of its own because **reads go through
    /// `get_viewport`** (`doc/plan.md` rule 1): a shell drawing a paragraph needs its text and
    /// how that text is formatted at the same moment, and two calls would be two moments.
    pub runs: Vec<RunView>,
    /// Whether anything about this block is *directly* formatted rather than inherited from
    /// its named style — what `grind text formatting` lists.
    pub styled: bool,
    /// The bookmarks anchored inside this block, as (character offset, name), in order —
    /// `doc/view-modes.md` §3.6, the word processor's half of inline names.
    ///
    /// A bookmark is the exact analogue of a named range and it is **invisible**: a
    /// zero-width `Run` that contributes no characters, so nothing a reader sees says it is
    /// there. This is what an overlay draws, and it is carried here for the reason every
    /// other field is — a shell drawing a paragraph needs its text and its anchors at the
    /// same moment, and two calls would be two moments.
    ///
    /// Always filled, with no flag to ask for it, and that is the difference from the
    /// spreadsheet's overlays rather than an inconsistency: a role costs a document-wide
    /// analysis and this costs a walk of the block's own runs, which the line above already
    /// does.
    pub marks: Vec<(usize, String)>,
}

/// One run of uniformly formatted characters, as a reader sees it.
///
/// A projection of [`model::Run`] and not the run itself: a tab and a line break are one
/// character each here, so the offsets a shell counts are the offsets a [`Caret`] counts, and a
/// bookmark contributes nothing at all because it is a position rather than content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunView {
    /// Where this run starts, in characters from the beginning of the block.
    pub start: usize,
    /// Its characters. `\t` for a `text:tab` and `\n` for a `text:line-break`.
    pub text: String,
    /// The direct character formatting on it.
    pub props: CharStyle,
    /// The named character style it carries, which this build does not interpret
    /// ([`crate::style`]).
    pub style: Option<String>,
    /// `xlink:href`, when the run is inside a link.
    pub href: Option<String>,
    /// The image this run is, if it is one — `Some` only for a [`Run::Image`], and what a
    /// shell that wants to draw an actual picture rather than the placeholder character reads.
    ///
    /// ponytail: clones the bytes on every [`App::get_viewport`] call, the same trade every
    /// other field here already makes for text. Worth a cache keyed by identity rather than
    /// content if a profile ever blames a document with many large images; nothing has yet.
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
    /// `svg:width` / `svg:height`, ODF lengths kept verbatim (`"5cm"`) and optional, exactly
    /// as a document's own frequently are.
    pub width: Option<String>,
    pub height: Option<String>,
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

/// One heading, as the outline lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub index: usize,
    pub level: u32,
    /// The outline path — `[2, 1, 3]` for §2.1.3. Computed, because the document stores no
    /// outline (`doc/odt-format.md` §2).
    pub path: Vec<u32>,
    pub text: String,
}

impl Heading {
    /// The address a user can type back in — `§2.1.3`, which survives edits elsewhere in the
    /// document where `p12` would not.
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

/// What `words` counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub words: usize,
    pub characters: usize,
    pub blocks: usize,
    pub headings: usize,
}

/// Which measure and which [`Metrics`] each block is set in — **the lookup a motion that
/// crosses a block boundary needs.**
///
/// Down-arrow out of a heading lands in the paragraph below it, and that paragraph is set in a
/// different face: a smaller font, and (for a list item) a narrower measure, because the indent
/// comes out of the column. A motion handed *one* width and *one* provider therefore measures
/// the block it arrives in with the face of the block it left — invisible in the middle of a
/// line and wrong by a few characters at either end. So the caret operations ask for each
/// block's face **as they reach it** rather than being told once, which is what makes them
/// right across a boundary rather than only within one block.
///
/// **The block is described rather than handed over**, and that is not a convenience: [`App`]
/// holds its read lock for the whole motion, so an implementation that called back into
/// [`App::get_viewport`] to find out what it was being asked about would be re-entering a lock
/// it is already inside. Kind and named style are what every shell keys a face off anyway —
/// `Title` and `Subtitle` are paragraphs whose only signal is the name, which is why the style
/// is here beside the kind — so they are what is passed.
///
/// This lives in `grind-text` rather than beside [`Metrics`] in `grind_core::layout` because a
/// *block* is the word processor's own vocabulary and R8 keeps that out of the core. The core's
/// half of the seam is unchanged: it still asks only how wide a piece of text is.
pub trait Faces {
    /// The measure and the metrics for the block at `index`.
    ///
    /// `width` is in whatever unit the returned provider answers in, and zero or less means do
    /// not wrap — the same contract [`App::layout_block`] has, because it is the same width.
    fn of(&self, index: usize, kind: &BlockKind, style: Option<&str>) -> (f32, &dyn Metrics);
}

/// Every block set alike: one width, one provider.
///
/// What the CLI measures with (`grind_text::Fixed` at `--width`) and what a shell whose unit is
/// a character cell wants, since a terminal has one font at one size. Named rather than
/// implied by an overload, so that a caller reaching for it is *saying* the document is set in
/// one face rather than forgetting that it might not be.
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

/// The word processor. Shells hold an `Arc<App>` and call these methods.
///
/// Every method takes `&self`; the lock lives inside, so one `App` can be shared between a UI
/// thread and background work without the shell thinking about it — the same contract
/// `grind_sheet::App` offers, because a shell should not have to learn two.
#[derive(Default)]
pub struct App {
    state: RwLock<State>,
    observer: RwLock<Option<Arc<dyn Observer>>>,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_observer(&self, observer: Arc<dyn Observer>) {
        *self.observer.write().unwrap() = Some(observer);
    }

    /// Mutate, then notify — **with the write lock already released**.
    ///
    /// An observer calls straight back in to re-read what changed, so notifying while still
    /// holding the lock deadlocks it. There is a test named for that deadlock, and this helper
    /// exists so no method has to remember.
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

    /// Replace the document. History is dropped: an undo across a file boundary would apply an
    /// action addressed to a document that no longer exists.
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

    /// Serialise the current document. Paired with [`App::save_file`] because the browser has
    /// no filesystem (doc/plan.md rule 5), and this is why the app never exposes its
    /// `Document`: saving is an operation, not a getter.
    pub fn save_bytes(&self, form: Form) -> Result<Vec<u8>> {
        write_bytes(&self.state.read().unwrap().doc, form)
    }

    pub fn save_file(&self, path: &Path) -> Result<()> {
        write_file(&self.state.read().unwrap().doc, path)
    }

    // --- reading ---

    /// Read a run of blocks. The only way a shell reads content.
    ///
    /// Asking past the end is normal rather than an error — a reader may scroll into blank
    /// space — and comes back short.
    pub fn get_viewport(&self, blocks: Range<usize>) -> Viewport {
        let state = self.state.read().unwrap();
        let end = blocks.end.min(state.doc.blocks.len());
        let start = blocks.start.min(end);
        let items = state.doc.blocks[start..end]
            .iter()
            .enumerate()
            .map(|(offset, block)| BlockView {
                index: start + offset,
                id: block.id,
                kind: block.kind.clone(),
                style: block.style.clone(),
                text: block.text(),
                runs: run_views(block),
                styled: block.is_styled(),
                marks: marks(block),
            })
            .collect();
        Viewport {
            blocks: start..end,
            items,
        }
    }

    /// The document as its **projection** — plain text, with the token and span maps beside it
    /// (`doc/dsl.md` §3, D2).
    ///
    /// Two things at once, exactly as the spreadsheet's twin is: it is what a `.grind` file
    /// holds, and it is what a code view shows of a document nobody has saved (§6). A shell
    /// reads it, colours it from [`projection::Projection::tokens`], and asks
    /// [`projection::Projection::address_at`] which block the caret is in — and here that
    /// answers in `loc.rs`'s whole vocabulary, because a block is `p12` *and* `#intro` *and*
    /// `§2.1.3` and the span map carries all three.
    ///
    /// **Nothing here writes to the document.** A projection is a view of a model, not a second
    /// copy of one.
    pub fn project(&self) -> projection::Projection {
        projection::project(&self.state.read().unwrap().doc)
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

    /// Resolve an address to a [`Caret`] — a block *and* an offset within it, which is what
    /// `p12+40` names and what the caret-level edits take.
    pub fn resolve_caret(&self, at: &Loc) -> Result<Caret> {
        let state = self.state.read().unwrap();
        loc::resolve_caret(&state.doc, at).map_err(|e| Error::Xml(e.to_string()))
    }

    /// Resolve a range of *characters*, as [`App::erase`] takes it. An end with no offset of
    /// its own means the end of its block, so a bare `p3` is all of p3's text.
    pub fn resolve_caret_range(&self, range: &loc::Range) -> Result<(Caret, Caret)> {
        let state = self.state.read().unwrap();
        loc::resolve_caret_range(&state.doc, range).map_err(|e| Error::Xml(e.to_string()))
    }

    /// The blocks belonging to the heading at `index` — itself, plus everything up to the next
    /// heading at the same level or higher. `None` if it is not a heading.
    ///
    /// Computed rather than stored, because the body has no such container
    /// (`doc/odt-format.md` §2). This is what lets `grind text move report.fodt §3.2 §1` mean
    /// "move that section" rather than "move that one line".
    pub fn section(&self, index: usize) -> Option<Range<usize>> {
        self.state.read().unwrap().doc.section(index)
    }

    /// Every heading, with its computed outline path — the document's structure.
    ///
    /// `grind_sheet::App::calculations`'s counterpart: a spreadsheet hides its formulas behind
    /// their results, and a long document hides its shape behind its prose. The only way to
    /// see either is a list.
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

    /// Every block carrying direct formatting or a named style of its own.
    ///
    /// **The differentiator** (`doc/suite.md`). Every shared document raises the question "why
    /// is this paragraph different?", and no mainstream word processor answers it in one place.
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
            // Byte offsets from `match_indices`, characters in the address — a `p12+40` counts
            // what a person counts, and `loc` is 0-based inside.
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

    /// Check the document against [`lint`]'s rules — `doc/dsl.md` §4.3, D6.
    ///
    /// A read, so it takes the read lock and hands back a plain value like every other one
    /// here: nothing is stored, nothing is marked, and linting a document leaves its bytes
    /// exactly as they were. `grind text lint` is the CLI twin, and a shell that wants
    /// squiggles turns each address into a byte range through the projection's span map (§6.2)
    /// rather than by inventing a second addressing.
    pub fn lint(&self, options: &grind_core::lint::Options) -> grind_core::lint::Report {
        lint::lint(&self.state.read().unwrap().doc, options)
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

    // --- layout ---
    //
    // `doc/text-layout.md`, decided: the engine is `grind_core::layout` and these four methods
    // are how a shell reaches it. Every one of them exists because the operation it names is
    // defined in terms of a *line* — and a line is an output of layout, not a thing in the
    // document, so leaving them to the shells would have been three implementations that
    // disagree. The shell brings the faces and nothing about a font is stored here.
    //
    // The three caret operations take a [`Faces`] rather than one width and one provider,
    // because a motion by line may end in a block set differently from the one it started in.
    // [`App::layout_block`] keeps the pair: its caller has already named the one block it
    // wants, and answering "how wide is *this* block" is the question the CLI's `--width` asks.

    /// Break one block into lines at `width`, in whatever unit `metrics` answers in.
    ///
    /// The result is a plain value carrying the x of every caret position, so a shell paints
    /// from it and throws it away — the same contract [`App::get_viewport`] offers for content.
    /// A `width` of zero or less means do not wrap.
    ///
    /// One named block, so the caller has already chosen its face; anything that may *move*
    /// between blocks takes a [`Faces`] instead and looks each one up as it arrives.
    pub fn layout_block(&self, index: usize, width: f32, metrics: &dyn Metrics) -> Result<Layout> {
        let state = self.state.read().unwrap();
        let block = state
            .doc
            .block(index)
            .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(index))))?;
        Ok(lay_out(block, width, metrics))
    }

    /// The x of a caret within its line — what a shell remembers as the **goal column** while
    /// the user holds Down, so that walking through a short line and out the other side comes
    /// back to the column it started in.
    ///
    /// Passed back into [`App::caret_line`] rather than stored, because it is a property of a
    /// *run of keystrokes* and not of the document.
    ///
    /// Measured through the same [`Faces`] the move itself uses, so the goal column and the
    /// line it is applied to are in one unit. That is what makes a goal column survive a change
    /// of face: a heading and the paragraph under it are set in different fonts but on the same
    /// screen, and x is the screen's.
    pub fn caret_x(&self, at: Caret, faces: &dyn Faces) -> Result<f32> {
        let state = self.state.read().unwrap();
        let block = state
            .doc
            .block(at.block)
            .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(at.block))))?;
        Ok(set_out(block, at.block, faces).x_at(at.offset))
    }

    /// Move a caret `delta` lines — **the Down and Up arrows**, and Page Down with a bigger
    /// number.
    ///
    /// Crosses block boundaries: down from the last line of a paragraph lands on the first line
    /// of the next, which is the behaviour that makes a document one flow rather than a list of
    /// boxes. At the very top or bottom it stops rather than erroring, because a caret that
    /// cannot move is not a failure — it is the end of the document.
    ///
    /// **The block it arrives in is measured in its own face**, which is why this takes a
    /// [`Faces`] rather than a width and a provider: Down out of a heading is the one motion
    /// where the two blocks are set differently by construction, and measuring the paragraph
    /// below with the heading's font put the caret several characters from where the click
    /// would have.
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
        );
        let mut offset = at.offset.min(layout.len());
        let step = if delta < 0 { -1 } else { 1 };

        for _ in 0..delta.unsigned_abs() {
            if let Some(next) = layout.step(offset, step, goal_x) {
                offset = next;
                continue;
            }
            // Off the end of this block's lines: carry into the neighbour, landing on the line
            // nearest the one we left.
            let next = match step > 0 {
                true if block + 1 < blocks => block + 1,
                false if block > 0 => block - 1,
                // The document's own top or bottom. Stay put.
                _ => break,
            };
            block = next;
            layout = set_out(&state.doc.blocks[block], block, faces);
            let line = match step > 0 {
                true => 0,
                false => layout.lines().len().saturating_sub(1),
            };
            offset = layout.offset_at(line, goal_x);
        }
        Ok(Caret { block, offset })
    }

    /// The two ends of the caret's own line — **Home and End**.
    ///
    /// On a wrapped line these are the *visual* ends, not the paragraph's, which is the whole
    /// difference between a text editor and a `set_text` front end. Returned as a pair because
    /// they are one layout apart and a shell asking for one usually wants the other.
    pub fn caret_line_bounds(&self, at: Caret, faces: &dyn Faces) -> Result<(Caret, Caret)> {
        let state = self.state.read().unwrap();
        let block = state
            .doc
            .block(at.block)
            .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(at.block))))?;
        let layout = set_out(block, at.block, faces);
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

    // --- editing ---

    /// Replace a block's text, keeping its kind and style.
    pub fn set_text(&self, index: usize, text: &str) -> Result<()> {
        self.mutate(|state| {
            let mut block = block_at(state, index)?;
            // Bookmarks are anchors rather than content, so retyping a paragraph keeps them —
            // losing an anchor because its sentence was rewritten would make `#intro` useless.
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

    /// Insert text at a caret — **what typing does.**
    ///
    /// The first of the four caret-level edits (`insert_text`, [`App::erase`],
    /// [`App::split_block`], [`App::join_block`]) that a continuous-flow editor is built out of.
    /// [`App::set_text`] replaces a whole block, which is what a *script* does; these four are
    /// what a *cursor* does, and they are in the core rather than in a shell because the
    /// terminal, GTK and web shells would otherwise each write their own and disagree
    /// (`doc/suite.md` S7).
    ///
    /// The inserted text takes the style and hyperlink of **the run at the caret, preferring
    /// the one to its left** — typing at the end of a bold word continues in bold, which is
    /// what every editor does and what a person expects. At the front of a block there is
    /// nothing to the left, so the run to the right decides.
    ///
    /// ponytail: that rule carries a `text:a` too, so typing at the end of a link extends the
    /// link. Right often enough to be the default and wrong often enough to want an override;
    /// the override is a shell-level decision and there is no shell yet to make it.
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

    /// Type text at a caret, reading the **markdown-shaped notation** as each character lands
    /// ([`crate::markdown`]) — `**bold**` becomes bold and its four markers go, `` `code` ``
    /// becomes monospace, `# ` makes the block a heading, ``` fences a code block.
    ///
    /// [`App::insert_text`]'s opinionated sibling, and opt-in for exactly that reason: a shell
    /// that wants asterisks to stay asterisks calls the other one. It is here rather than in a
    /// shell for the reason `doc/text-layout.md` gives about line breaking — three shells
    /// recognising `**` three ways would be three editors — and it is *one* action, so one
    /// press of undo takes back the whole of `**bold**` rather than the style, then the
    /// markers, then the characters.
    ///
    /// **`resume` is what makes the notation end where it says it does.** After a closing
    /// marker the caret sits at the end of the run just formatted, and the next character
    /// typed there would join it — `say **this** and` would carry on bold. The returned
    /// [`Typed::resume`] carries the formatting the span had *before* it was emphasised; hand
    /// it back on the next call and the notation stops where its marker did. A shell typing a
    /// whole string in one call never sees this: it is handled between the characters.
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
                // The character itself, in whatever the caret is set in — or in what the last
                // completed span said to go back to.
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

    /// Insert an image at a caret — a fifth caret-level edit, alongside `insert_text`, for the
    /// one kind of content that is not text (`doc/text-core.md`'s `draw:frame` row).
    ///
    /// One [`Run::Image`], occupying exactly one caret position the way a tab does, so
    /// `erase`, `split_block` and Backspace already handle it correctly with no change to any
    /// of them: it is removed, carried across a split, or backspaced over as a whole, never a
    /// character at a time. `mime` is a MIME type (`image/jpeg`, `image/png`, …); `width` and
    /// `height` are ODF lengths (`"5cm"`), kept verbatim and optional, exactly as a document's
    /// own would be.
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

    /// Erase the characters between two carets — **what a selection and a Delete key do.**
    ///
    /// Crossing a block boundary is the interesting case and the reason this is one method
    /// rather than three: erasing from the middle of p3 to the middle of p5 leaves *one* block,
    /// keeping p3's kind and style, and it is one [`Action::Batch`] and therefore one undo step.
    /// A shell doing it as "erase, erase, delete, join" would get four, and the fourth Ctrl+Z
    /// would be the one that surprised somebody.
    ///
    /// **Bookmarks inside the erased span survive, collapsed to the caret** — the same stance
    /// [`App::set_text`] takes, because an anchor is a position rather than content and losing
    /// `#intro` by rewriting the sentence around it would make the address useless. A block that
    /// ceases to exist entirely does take its anchors with it, exactly as [`App::delete`] does:
    /// there is no longer a position for them to be at.
    ///
    /// Returns the number of characters removed, counting each block boundary that closed up as
    /// one — the same arithmetic [`Document::text`] uses when it joins blocks with a newline.
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
                // The tail of the first block, the whole of everything between, the head of the
                // last, and one for each newline that closed up.
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

    /// Split a block at a caret — **what the Return key does.**
    ///
    /// The second half keeps the first's kind and style, so Return inside a list item makes
    /// another list item at the same depth and Return inside a paragraph makes another
    /// paragraph. **One exception: a heading split at its very end becomes a body paragraph**,
    /// because a heading followed by an empty heading is never what anybody meant, and every
    /// word processor there has ever been does this. Splitting a heading in the *middle* does
    /// make two headings, which is how a title gets divided.
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
            second.runs = tail;
            if second.is_empty() && matches!(block.kind, BlockKind::Heading { .. }) {
                second.kind = BlockKind::Paragraph;
                // The heading's style went with the heading; a body paragraph wearing
                // `Heading_20_1` would look like a heading and not be one.
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

    /// Join a block with the one after it — **what Backspace at the front of a block does.**
    ///
    /// [`App::split_block`]'s inverse in effect though not in mechanism, and the survivor is
    /// the *first* block: its kind and style win, which is why backspacing at the front of a
    /// paragraph that follows a heading pulls the text up into the heading rather than
    /// demoting the heading.
    pub fn join_block(&self, index: usize) -> Result<()> {
        self.mutate(|state| {
            let mut block = block_at(state, index)?;
            let next = state
                .doc
                .block(index + 1)
                .cloned()
                .ok_or_else(|| Error::Xml(format!("nothing follows {}", loc::format(index))))?;
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

    /// Insert a block before `index`. `index == block_count()` appends.
    pub fn insert(&self, index: usize, kind: BlockKind, text: &str) -> Result<()> {
        self.mutate(|state| {
            let id = state.doc.next_id();
            let mut block = Block::new(id, kind);
            if !text.is_empty() {
                block.runs.push(Run::plain(text));
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
            // Removing from the front repeatedly: each removal shifts the rest down, so the
            // same index removes the next one.
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

    /// Change what kind of block this is — paragraph to heading and back.
    ///
    /// The outline is implied by nothing else (`doc/odt-format.md` §2), so this *is* how a
    /// document gets its structure.
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

    /// The character formatting over a span — **what a toolbar shows.**
    ///
    /// Whatever every character between the two carets agrees about, and nothing else: a range
    /// that is bold throughout reads as bold, and one that is half bold reads as neither. That
    /// is what makes a toggle button predictable over a mixed selection, and it is decided here
    /// rather than in a shell so that three shells cannot decide it three ways.
    ///
    /// An **empty** span — the two carets equal, which is what a bare cursor is — reports the
    /// formatting of the run at the caret, **preferring the one to its left**. The same rule
    /// [`App::insert_text`] uses to decide what typed text looks like, and necessarily so: the
    /// toolbar has to show what the next keystroke will produce.
    pub fn char_style(&self, from: Caret, to: Caret) -> Result<CharStyle> {
        let state = self.state.read().unwrap();
        if to < from {
            return Err(Error::Xml("that range runs backwards".to_owned()));
        }
        let mut common: Option<CharStyle> = None;
        for index in from.block..=to.block {
            let block = state
                .doc
                .block(index)
                .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(index))))?;
            let start = if index == from.block { from.offset } else { 0 };
            let end = if index == to.block {
                to.offset
            } else {
                block.len()
            };
            for props in spanned(block, start, end) {
                common = Some(match common {
                    Some(so_far) => so_far.common(&props),
                    None => props,
                });
            }
        }
        Ok(common.unwrap_or_default())
    }

    /// Replace the character formatting of every character between two carets — **bold,
    /// italic, a font, a size.**
    ///
    /// **Replaces rather than adds**, which is `grind_sheet::App::set_style`'s contract and is
    /// what makes a toolbar a read, one field and a write: an empty [`CharStyle`] is "plain
    /// again". Named character styles are untouched, because they are the document's own
    /// vocabulary and this method is about direct formatting ([`crate::style`]).
    ///
    /// One [`Action::Batch`], so formatting a section is one Ctrl+Z. Returns how many blocks
    /// changed.
    pub fn set_char_style(&self, from: Caret, to: Caret, style: &CharStyle) -> Result<usize> {
        self.mutate(|state| {
            if to < from {
                return Err(Error::Xml("that range runs backwards".to_owned()));
            }
            let mut batch = Vec::new();
            for index in from.block..=to.block {
                let mut block = block_at(state, index)?;
                let start = if index == from.block { from.offset } else { 0 };
                let end = if index == to.block {
                    to.offset
                } else {
                    block.len()
                };
                let Some(runs) = restyled(&block, start, end, style) else {
                    continue;
                };
                block.runs = runs;
                batch.push(Action::SetBlock {
                    index,
                    block: Box::new(block),
                });
            }
            let changed = batch.len();
            if changed > 0 {
                Self::commit(state, Action::Batch(batch))?;
            }
            Ok(changed)
        })
    }

    /// Set — or with `None`, clear — the named paragraph style of a run of blocks.
    pub fn set_style(&self, blocks: Range<usize>, style: Option<String>) -> Result<usize> {
        self.mutate(|state| {
            if blocks.end > state.doc.blocks.len() {
                return Err(Error::Xml("that range runs past the end".to_owned()));
            }
            let mut batch = Vec::new();
            for index in blocks {
                let mut block = block_at(state, index)?;
                if block.style == style {
                    continue;
                }
                block.style = style.clone();
                batch.push(Action::SetBlock {
                    index,
                    block: Box::new(block),
                });
            }
            let changed = batch.len();
            if changed > 0 {
                Self::commit(state, Action::Batch(batch))?;
            }
            Ok(changed)
        })
    }

    /// Move a run of blocks so that it starts at `to`. What "move section 3.2" is.
    ///
    /// `to` is an index in the document **as it is now**, before the move. Moving a range into
    /// itself is refused rather than silently doing nothing, because it is always a mistake at
    /// the call site.
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
            // After the removals every index at or past the range has shifted down by its
            // length, so the destination has too.
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

    /// Put a bookmark on a block, or with `None` remove one by name.
    ///
    /// The named-range analogue: an anchor that moves with the text, so `#intro` keeps meaning
    /// the same place after an edit somewhere else.
    pub fn set_bookmark(&self, name: &str, index: Option<usize>) -> Result<bool> {
        self.mutate(|state| {
            let mut batch = Vec::new();
            // Remove any existing anchor of this name first, so setting one twice moves it
            // rather than leaving two.
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
                // A block already in the batch has to be edited from the batch's version, not
                // the document's, or the removal above would be undone by this insertion.
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

/// The style and hyperlink newly typed text should take, given the runs on either side of the
/// caret.
///
/// **Prefer the left.** Typing continues what you just typed, so the run before the caret
/// decides; at the front of a block there is nothing before it and the run after decides
/// instead. A caret next to a tab, a break or a bookmark carries no formatting of its own, so
/// the search is for the nearest *text* run and stops at the first thing that is not one.
/// What [`App::type_markdown`] did: where the caret ended up, and what the *next* character
/// typed there must be set in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typed {
    pub caret: Caret,
    /// `Some` when the last character closed an emphasis — see [`App::type_markdown`].
    pub resume: Option<CharStyle>,
}

/// Read the notation off a block that has just had a character typed into it, and apply
/// whatever it completed — in memory, so the whole of `**bold**` is one `SetBlock` and one
/// undo step.
///
/// Returns the formatting the next character typed at the caret must be given, `None` when
/// nothing completed.
fn apply_notation(block: &mut Block, caret: &mut usize) -> Option<CharStyle> {
    let text = block.text();

    // A fence toggles the block between preformatted and plain, and takes its own three
    // markers back out.
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

    let found = markdown::emphasised(&text, *caret)?;
    // What the span was set in before it was emphasised — read from the opening marker, which
    // was typed in the formatting around it and is about to be deleted.
    let before = props_at(block, found.open);
    // The closing marker first: taking the opening one out would move everything after it, and
    // these offsets were measured before either went.
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

/// The direct formatting of the run covering `offset` — what a character typed there would
/// have inherited, which is what the text after a closing marker has to go back to.
fn props_at(block: &Block, offset: usize) -> CharStyle {
    let (head, tail) = model::split_runs(&block.runs, offset);
    caret_formatting(&head, &tail).1
}

fn caret_formatting(head: &[Run], tail: &[Run]) -> (Option<String>, CharStyle, Option<String>) {
    let neighbour = head.last().or_else(|| tail.first());
    match neighbour {
        Some(Run::Text {
            style, props, href, ..
        }) => (style.clone(), props.clone(), href.clone()),
        _ => (None, CharStyle::default(), None),
    }
}

/// Break one block into lines **in the face that block is set in** — [`lay_out`] with the
/// width and the provider looked up rather than passed in.
///
/// One function so that every caret operation asks the same question the same way. The
/// alternative — each of them doing its own lookup — is how a motion ends up measuring one
/// block with another's font, which is exactly the bug [`Faces`] exists to close.
fn set_out(block: &Block, index: usize, faces: &dyn Faces) -> Layout {
    let (width, metrics) = faces.of(index, &block.kind, block.style.as_deref());
    lay_out(block, width, metrics)
}

/// Break one block into lines.
///
/// The whole of this crate's contribution to layout: turn runs into [`layout::Fragment`]s and
/// hand them over. One fragment per run, so the character offsets a [`Caret`] counts and the
/// ones the layout reports are the same numbers — a bookmark contributes an empty fragment and
/// therefore no offset, exactly as it contributes no text.
///
/// **Each run measures with its own direct formatting**, projected into the four properties
/// that change how wide text is (`crate::style::CharStyle::metrics`). So a bold word is
/// measured bold and the caret lands where the ink does — which only became true when runs
/// started carrying properties rather than a style *name*, and is the reason they do.
///
/// A run carrying only a **named** character style still measures with the default, because
/// this build does not read style definitions (`doc/text-core.md`). The seam is unchanged: when
/// definitions arrive, they are resolved into the same `TextStyle` and nothing here moves.
fn lay_out(block: &Block, width: f32, metrics: &dyn Metrics) -> Layout {
    let default = grind_core::style::TextStyle::default();
    let styles: Vec<grind_core::style::TextStyle> = block
        .runs
        .iter()
        .map(|run| run.props().map(CharStyle::metrics).unwrap_or_default())
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
    // An empty paragraph is still one line tall, and the only way to say how tall that is
    // is to hand the provider a fragment to answer about: `wrap` takes its height from the
    // fragments it was given, so with none it has nothing to ask and falls back to one unit.
    // Invisible where a unit *is* a line — the CLI's `Fixed`, a terminal cell — and the
    // difference between an empty paragraph and a one-pixel gap in a shell that draws
    // pixels.
    if fragments.is_empty() {
        fragments.push(layout::Fragment {
            text: "",
            style: &default,
        });
    }
    layout::wrap(&fragments, width, metrics)
}

/// The formatting of every run that `start..end` touches, in order.
///
/// An **empty** range reports one formatting rather than none: the run at the caret, preferring
/// the one to its left, which is [`caret_formatting`]'s rule and has to be the same rule.
fn spanned(block: &Block, start: usize, end: usize) -> Vec<CharStyle> {
    if start == end {
        // Cut where the caret is and ask the same question `insert_text` asks, through the
        // same primitive — two spellings of "the run at the caret" would eventually disagree,
        // and the disagreement would be a toolbar lying about the next keystroke.
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

/// This block's runs with `start..end` restyled, or `None` when that changes nothing.
///
/// Built on [`model::split_runs`] twice, which is the same surgery every caret edit is: cut at
/// both ends, rewrite the middle, put the three back together. A run half inside the span is
/// split by construction, so a bold word inside a plain sentence is exactly the characters that
/// were asked for.
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
            *props = style.clone();
        }
    }
    let mut runs = head;
    runs.extend(middle);
    runs.extend(tail);
    model::coalesce(&mut runs);
    Some(runs)
}

/// One block's runs, as a reader sees them.
///
/// Bookmarks are dropped rather than carried as empty runs: they contribute no characters, and
/// a shell walking runs alongside a [`Layout`] would otherwise have to know to skip them.
/// Where each bookmark in a block sits, in characters from its start — [`BlockView::marks`].
///
/// The offsets are counted the way [`run_views`] counts them and the way a [`Caret`] does, so
/// a mark's offset is an offset a shell can already put a caret at.
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

fn run_views(block: &Block) -> Vec<RunView> {
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
            } => Some(ImageView {
                mime: mime.clone(),
                data: data.clone(),
                width: width.clone(),
                height: height.clone(),
            }),
            _ => None,
        };
        out.push(RunView {
            start,
            text: run.text().to_owned(),
            props: run.props().cloned().unwrap_or_default(),
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
        // rng:8408 — ODF's run-length encoding of spaces. The trap that
        // table:number-columns-repeated is, wearing different clothes.
        let doc = read(r#"<text:p>a<text:s text:c="4"/>b<text:s/>c</text:p>"#);
        assert_eq!(doc.blocks[0].text(), "a    b c");
    }

    #[test]
    fn a_repeat_count_is_clamped_rather_than_believed() {
        // §9: never trust the file's number. Four billion spaces is a memory-exhaustion
        // vector, not an intent.
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
        // §8's whole design, arriving unchanged in a second document type. None of these cost
        // a line of code in the reader, and the paragraphs around them still read.
        let doc = read(
            r#"<text:p>before</text:p>
               <text:section text:name="s"><text:p>inside a section</text:p></text:section>
               <text:table-of-content><text:index-body><text:p>TOC</text:p></text:index-body></text:table-of-content>
               <text:bibliography/>
               <office:annotation><text:p>a comment</text:p></office:annotation>
               <text:p>after</text:p>"#,
        );
        assert_eq!(
            doc.text(),
            "before\nafter",
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
        // §8.1: dispatch is on the URI. The same document under different prefixes reads the
        // same, and this is the second document type proving it.
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
