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
//! supplies font metrics through [`Metrics`] and nothing else. Pagination is still gated, and
//! layout is left-to-right only by explicit decision.
//!
//! **What it does not have.** A session, so no `undo` across CLI invocations; tables,
//! footnotes and fields; style *definitions*, so a style name this build writes does not
//! survive LibreOffice, and every run therefore measures with the default character style;
//! pages; and any shell but the CLI.

pub mod action;
pub mod loc;
pub mod model;
pub mod odf;

pub use action::Action;
pub use grind_core::layout::{self, Fixed, Layout, Metrics};
pub use grind_core::{DocumentKind, Error, Form, Observer, Result, kind};
pub use loc::{Caret, Loc, Target};
pub use model::{Block, BlockId, BlockKind, Document, Run};

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
    ]
}

/// Read a `.odt` (package) or `.fodt` (flat) document from bytes.
///
/// Paired with [`read_file`] from the start because the browser has no filesystem, and this
/// is not retrofittable later (doc/plan.md, rule 5).
///
/// The form is sniffed from the bytes, so `name` is only ever a label for diagnostics.
pub fn read_bytes(_name: &str, bytes: &[u8]) -> Result<Document> {
    odf::read(bytes)
}

pub fn read_file(path: &Path) -> Result<Document> {
    read_bytes(&path.display().to_string(), &std::fs::read(path)?)
}

/// Serialise a document. See [`Form`].
pub fn write_bytes(doc: &Document, form: Form) -> Result<Vec<u8>> {
    odf::write(doc, form)
}

/// Write a document, choosing the form from the extension — `.fodt` flat, anything else the
/// package form ([`Form::from_path`]).
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
    /// Whether anything about this block is *directly* formatted rather than inherited from
    /// its named style — what `grind text formatting` lists.
    pub styled: bool,
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
                styled: block.is_styled(),
            })
            .collect();
        Viewport {
            blocks: start..end,
            items,
        }
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
    // disagree. The shell brings a width and a `Metrics`; nothing about a font is stored here.

    /// Break one block into lines at `width`, in whatever unit `metrics` answers in.
    ///
    /// The result is a plain value carrying the x of every caret position, so a shell paints
    /// from it and throws it away — the same contract [`App::get_viewport`] offers for content.
    /// A `width` of zero or less means do not wrap.
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
    pub fn caret_x(&self, at: Caret, width: f32, metrics: &dyn Metrics) -> Result<f32> {
        Ok(self.layout_block(at.block, width, metrics)?.x_at(at.offset))
    }

    /// Move a caret `delta` lines — **the Down and Up arrows**, and Page Down with a bigger
    /// number.
    ///
    /// Crosses block boundaries: down from the last line of a paragraph lands on the first line
    /// of the next, which is the behaviour that makes a document one flow rather than a list of
    /// boxes. At the very top or bottom it stops rather than erroring, because a caret that
    /// cannot move is not a failure — it is the end of the document.
    pub fn caret_line(
        &self,
        at: Caret,
        delta: isize,
        goal_x: f32,
        width: f32,
        metrics: &dyn Metrics,
    ) -> Result<Caret> {
        let state = self.state.read().unwrap();
        let blocks = state.doc.blocks.len();
        let mut block = at.block;
        let mut layout = lay_out(
            state
                .doc
                .block(block)
                .ok_or_else(|| Error::Xml(format!("no block {}", loc::format(block))))?,
            width,
            metrics,
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
            layout = lay_out(&state.doc.blocks[block], width, metrics);
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
    pub fn caret_line_bounds(
        &self,
        at: Caret,
        width: f32,
        metrics: &dyn Metrics,
    ) -> Result<(Caret, Caret)> {
        let layout = self.layout_block(at.block, width, metrics)?;
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
                block.runs.push(Run::Text {
                    text: text.to_owned(),
                    style: None,
                    href: None,
                });
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
            let (style, href) = caret_formatting(&runs, &tail);
            runs.push(Run::Text {
                text: text.to_owned(),
                style,
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
                block.runs.push(Run::Text {
                    text: text.to_owned(),
                    style: None,
                    href: None,
                });
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
fn caret_formatting(head: &[Run], tail: &[Run]) -> (Option<String>, Option<String>) {
    let neighbour = head.last().or_else(|| tail.first());
    match neighbour {
        Some(Run::Text { style, href, .. }) => (style.clone(), href.clone()),
        _ => (None, None),
    }
}

/// Break one block into lines.
///
/// The whole of this crate's contribution to layout: turn runs into [`layout::Fragment`]s and
/// hand them over. One fragment per run, so the character offsets a [`Caret`] counts and the
/// ones the layout reports are the same numbers — a bookmark contributes an empty fragment and
/// therefore no offset, exactly as it contributes no text.
///
/// **Every run measures with the default character style**, because a `Run`'s style is a *name*
/// and this build does not read style definitions (`doc/text-core.md`). The seam is right and
/// the lookup is missing; when definitions arrive, this function resolves them and nothing else
/// changes.
fn lay_out(block: &Block, width: f32, metrics: &dyn Metrics) -> Layout {
    let style = grind_core::style::TextStyle::default();
    let mut fragments: Vec<layout::Fragment<'_>> = block
        .runs
        .iter()
        .map(|run| layout::Fragment {
            text: run.text(),
            style: &style,
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
            style: &style,
        });
    }
    layout::wrap(&fragments, width, metrics)
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
