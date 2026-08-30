// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ODF spreadsheet core.
//!
//! All state and all logic live here. Every front-end — CLI, TUI, desktop, browser — is a
//! renderer and an event forwarder that owns nothing (doc/plan.md). Reads go through
//! [`App::get_viewport`]; the column store exists so that this is the only sane way to
//! read, and a whole-document getter would quietly make it pointless.
//!
//! Phase 1: the model, the column store, actions and undo/redo. Phase 2 adds the reader,
//! which is why [`read_file`] exists and fails.
//!
//! See `doc/ods-format.md` for the format, `doc/plan.md` for phase order.

use std::fmt;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, RwLock};

pub mod a1;
pub mod action;
pub mod chart;
pub mod filter;
pub mod formula;
pub mod graph;
pub mod grid;
pub mod lint;
pub mod model;
pub mod numfmt;
pub mod odf;
pub mod projection;
pub mod style;
pub mod view;

/// What this crate takes from `grind-core` and hands on under its own name.
///
/// The crate boundary is the seam — `grind-core` compiles knowing nothing about spreadsheets,
/// and its `tests/generic.rs` fails the build if that stops being true (R8). These are
/// ergonomics on this side of it: a shell that only edits spreadsheets says `grind_sheet::` for
/// everything, and the split does not leak into it.
pub use grind_core::{DocumentKind, Form, Observer, build_info, kind, locale};

pub use action::Action;
pub use chart::{
    Axis as ChartAxis, Chart, ChartData, ChartKind, Series as ChartSeries, Ticks, axis_ticks,
    effective_color, series_color,
};
pub use filter::Filter;
pub use model::{CellValue, Document, Pos, Sheet};

/// What can go wrong with a **spreadsheet**.
///
/// The generic failures — a zip that will not open, XML that will not parse, a document we
/// have no key for, the filesystem — belong to every document type and live in
/// [`grind_core::Error`], reached through [`Error::Odf`]. R8 is why the split exists; `?` is
/// why it costs one `From` impl rather than a rewrite of every call site.
#[derive(Debug)]
pub enum Error {
    /// Something true of any ODF document, not just a spreadsheet.
    Odf(grind_core::Error),
    NoSuchSheet(usize),
    /// A request whose size is the problem rather than its content — see
    /// [`MAX_FORMATTED_CELLS`].
    TooLarge(u64),
    /// A sheet operation ODF has no spelling for: a name another sheet already has, an
    /// empty one, or removing the document's last sheet.
    BadSheet(String),
    /// A formula or a name that is not what §5 says one is. Rejected at the point it is
    /// *defined* rather than stored and discovered later — see [`App::set_name`].
    Formula(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Odf(e) => write!(f, "{e}"),
            Error::NoSuchSheet(i) => write!(f, "no such sheet: {i}"),
            Error::TooLarge(n) => write!(
                f,
                "{n} is more than one command may cover at once \
                 ({MAX_FORMATTED_CELLS} cells, {MAX_TRACK_RUN} rows or columns)"
            ),
            Error::BadSheet(e) => write!(f, "{e}"),
            Error::Formula(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<grind_core::Error> for Error {
    fn from(e: grind_core::Error) -> Self {
        Error::Odf(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Odf(grind_core::Error::Io(e))
    }
}

impl Error {
    /// Whether this is a particular generic failure — `Error::Odf(Encrypted)` without the
    /// nesting at every call site. Loop A's one documented loosening is spelled with it.
    pub fn is_encrypted(&self) -> bool {
        matches!(self, Error::Odf(grind_core::Error::Encrypted))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// How big a sheet is: the bounds every other spreadsheet uses, and the ones the reader
/// clamps a repeat count to.
///
/// Public because a shell has to lay out a scrollbar over the same sheet the reader filled,
/// and two copies of these numbers is one copy too many. Note this is *not* a limit on what
/// [`formula::lex`] will parse — a foreign file may name a cell past the end and still has
/// to load (R5), so the bound is applied where an address becomes a place on *this* sheet,
/// in [`a1::resolve`].
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

/// How many cells one [`App::set_format`] may cover.
///
/// Formats are stored per cell, so this bounds the memory a single command can ask for.
/// Well above any rectangle a person selects, well below a whole sheet.
pub const MAX_FORMATTED_CELLS: u64 = 1 << 16;

/// How many columns or rows one [`App::set_col_width`] may cover — and, on the way in, how
/// long a run of equally sized tracks is still *layout* rather than background.
///
/// One constant for both ends on purpose: a `<table:table-column
/// table:number-columns-repeated="16368"/>` is a file saying "the rest of the sheet is the
/// default width", not a decision about sixteen thousand columns, and materialising it would
/// be that many equal strings per sheet. Setting a run this program then refuses to read
/// back would be the same bug from the other side, so the reader's cap and the writer's
/// limit are the same number.
///
/// ponytail: a document that really does size 2000 columns loses them past the cap. Storing
/// a per-sheet *default* size instead would keep it, and is two more fields on `Sheet` plus a
/// rule for which wins — worth it when a file turns up that needs it.
pub const MAX_TRACK_RUN: u32 = 1024;

/// One calculated cell, as an explorer lists it ([`App::calculations`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calculation {
    pub sheet: usize,
    pub sheet_name: String,
    pub pos: Pos,
    /// The formula in display form — `=SUM(B2:B4)`, what a formula bar shows.
    pub formula: String,
    /// What the cell shows now: the cached result with its number format applied.
    pub value: String,
    /// The functions it calls, outermost first. Empty for plain arithmetic, and for a
    /// formula that will not parse.
    pub functions: Vec<String>,
}

impl Calculation {
    /// The cell as an address a user can type back in: `Sheet1.B2`, ODF's own spelling.
    pub fn address(&self) -> String {
        format!("{}.{}", self.sheet_name, a1::format(None, self.pos))
    }

    /// Whether a search finds this one — its sheet, address, formula or any function it
    /// calls, case-insensitively, and everything when the needle is empty.
    ///
    /// Here rather than in each shell so that a dialog's search box and `--filter` cannot
    /// answer the same question differently.
    pub fn matches(&self, needle: &str) -> bool {
        let needle = needle.trim().to_uppercase();
        needle.is_empty()
            || self.address().to_uppercase().contains(&needle)
            || self.formula.to_uppercase().contains(&needle)
            || self.functions.iter().any(|f| f.contains(&needle))
    }
}

/// Which functions a set of calculations uses, commonest first and ties alphabetical —
/// "what is this spreadsheet actually doing", which the per-cell list only implies.
pub fn function_tally(calculations: &[Calculation]) -> Vec<(String, usize)> {
    let mut tally: Vec<(String, usize)> = Vec::new();
    for name in calculations.iter().flat_map(|calc| &calc.functions) {
        match tally.iter_mut().find(|(n, _)| n == name) {
            Some((_, count)) => *count += 1,
            None => tally.push((name.clone(), 1)),
        }
    }
    tally.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    tally
}

/// A rectangle of values, already resolved — what a renderer draws.
///
/// Requesting rows or columns past the end of the sheet is normal, not an error: a user
/// may scroll into blank space. Those cells come back empty. (This is where a grid parts
/// company with a text editor, whose viewport must clamp to the last line so that a stale
/// scroll offset cannot blank the view.)
#[derive(Clone, Debug, PartialEq)]
pub struct Viewport {
    pub rows: Range<u32>,
    pub cols: Range<u32>,
    cells: Vec<CellValue>, // row-major
    /// What each cell *displays* — its number format applied (doc/ods-format.md §5).
    ///
    /// Carried beside the values rather than left to the caller because rendering needs the
    /// cell's format and the document's epoch, and a shell has neither: handing it the value
    /// alone would make every renderer either wrong about dates or a second implementation
    /// of `numfmt`.
    texts: Vec<String>,
    /// How each cell *looks* — weight, colours, alignment, borders (`style.rs`).
    ///
    /// Beside the text for the same reason: a renderer that had to ask for a style per cell
    /// would either take the lock once per cell or keep its own copy of the document. One
    /// clone per styled cell, and the vector is viewport-sized rather than sheet-sized.
    ///
    /// ponytail: a `CellStyle` is nine `Option<String>`s and this clones them per visible
    /// cell. Interning them — the writer already pools styles — is the upgrade, once a
    /// profile says a screenful of styled cells costs anything.
    styles: Vec<Option<style::CellStyle>>,
    /// What each cell *is* — `doc/view-modes.md`'s role overlay, row-major and total.
    ///
    /// `None` when the read did not ask for it ([`view::Overlays`]), because deriving a role
    /// needs a document-wide analysis and a shell that draws no roles must not pay for one.
    /// A fourth parallel vector for the same reason the second and third are here: a
    /// renderer asking per cell would take the lock per cell.
    roles: Option<Vec<view::CellRole>>,
    /// The name anchors *intersecting* this rectangle — a list rather than a per-cell
    /// vector, because a name binds to a **range** as often as to a cell and `sales` over
    /// `A2:A50` is one anchor, not forty-nine.
    names: Vec<view::NameAnchor>,
}

impl Viewport {
    /// The index of one cell in the row-major vectors, or `None` if it is outside.
    fn at(&self, row: u32, col: u32) -> Option<usize> {
        if !self.rows.contains(&row) || !self.cols.contains(&col) {
            return None;
        }
        let width = (self.cols.end - self.cols.start) as usize;
        let r = (row - self.rows.start) as usize;
        let c = (col - self.cols.start) as usize;
        Some(r * width + c)
    }

    pub fn get(&self, row: u32, col: u32) -> Option<&CellValue> {
        self.cells.get(self.at(row, col)?)
    }

    /// The display text of one cell — what a renderer draws.
    pub fn text(&self, row: u32, col: u32) -> Option<&str> {
        self.texts.get(self.at(row, col)?).map(String::as_str)
    }

    /// How one cell looks, or `None` for a plain cell — and for one outside the viewport.
    ///
    /// A renderer wants both answers to be "draw nothing special", so the two `None`s are
    /// deliberately the same: an unstyled cell and a cell off screen need no distinction.
    pub fn style(&self, row: u32, col: u32) -> Option<&style::CellStyle> {
        self.styles.get(self.at(row, col)?)?.as_ref()
    }

    /// What one cell *is*, or `None` when the read did not ask for roles — and for a cell
    /// outside the viewport.
    ///
    /// Unlike [`Viewport::style`], the two `None`s here mean different things and a caller
    /// that cares knows which it is asking: a shell in role mode requested the overlay, so
    /// `None` can only be a cell it is not drawing.
    pub fn role(&self, row: u32, col: u32) -> Option<view::CellRole> {
        self.roles.as_ref()?.get(self.at(row, col)?).copied()
    }

    /// Every name anchor intersecting this rectangle. Empty when the read did not ask for
    /// them, and empty when there are none — a shell draws nothing either way.
    pub fn names(&self) -> &[view::NameAnchor] {
        &self.names
    }

    /// The name bound to one cell, if any — the per-cell question the CLI's `--names` asks
    /// and a formula bar asks again.
    pub fn name_at(&self, row: u32, col: u32) -> Option<&str> {
        self.names
            .iter()
            .find(|a| a.rows.contains(&row) && a.cols.contains(&col))
            .map(|a| a.name.as_str())
    }

    /// One row of the viewport, left to right. `None` if the row is outside it.
    pub fn row(&self, row: u32) -> Option<&[CellValue]> {
        if !self.rows.contains(&row) {
            return None;
        }
        let width = (self.cols.end - self.cols.start) as usize;
        let r = (row - self.rows.start) as usize;
        self.cells.get(r * width..(r + 1) * width)
    }
}

#[derive(Default)]
struct State {
    doc: Document,
    undo: Vec<Action>,
    redo: Vec<Action>,
}

/// The application. Shells hold an `Arc<App>` and call these methods.
///
/// Every method takes `&self`; the lock lives inside, so one `App` can be shared between a
/// UI thread and background work without the shell thinking about it.
#[derive(Default)]
pub struct App {
    state: RwLock<State>,
    observer: RwLock<Option<Arc<dyn Observer>>>,
    /// `doc/view-modes.md`'s document-wide analysis, computed on first use and dropped on
    /// every mutation.
    ///
    /// A cache with an invalidation rule is where correctness usually goes to die, so the
    /// rule is kept trivial: it is emptied in [`App::mutate`], the one place a document
    /// changes and observers are notified, and the first cut recomputes everything — which
    /// means the only bug available here is a stale cache, never a wrong one.
    ///
    /// Its own lock rather than a `State` field, so a read that wants an overlay does not
    /// have to take the write lock to fill it. The order is always state-then-analysis.
    analysis: RwLock<Option<Arc<view::Analysis>>>,
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
    /// An observer is expected to call straight back into the app to re-read what changed.
    /// Notifying while still holding the lock deadlocks it. There is a test
    /// (`an_observer_may_read_the_app_without_deadlocking`) and it is the reason this
    /// helper exists rather than each method locking and notifying for itself.
    fn mutate<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let result = {
            let mut state = self.state.write().unwrap();
            f(&mut state)
        };
        // Before the notification, because an observer reads straight back in and must not
        // be handed a reading of the document as it was.
        *self.analysis.write().unwrap() = None;
        self.notify();
        result
    }

    fn notify(&self) {
        // Clone the Arc out and drop the lock before calling, for the same reason.
        let observer = self.observer.read().unwrap().clone();
        if let Some(observer) = observer {
            observer.changed();
        }
    }

    // --- editing ---

    pub fn set_cell(&self, sheet: usize, pos: Pos, value: impl Into<CellValue>) -> Result<()> {
        let value = value.into();
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::SetCell { sheet, pos, value })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Set a formula and its cached value together (doc/ods-format.md §4).
    ///
    /// The value is computed here rather than left to a later `recalc`, because a formula
    /// saved without one renders blank in LibreOffice. Evaluating against the document as it
    /// stands is also what a spreadsheet does when you press Enter.
    pub fn set_formula(&self, sheet: usize, pos: Pos, formula: &str) -> Result<()> {
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let value = formula::eval::to_cell(
                formula::eval::Engine::new(&state.doc)
                    .eval(formula, formula::eval::Address::new(sheet, pos)),
            );
            let inverse = state
                .doc
                .apply(Action::SetFormula {
                    sheet,
                    pos,
                    formula: Some(formula.to_owned()),
                    value,
                })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// What pressing Enter means — the typing rule, in the core so no two shells disagree.
    ///
    /// `input` is what the user typed, and `typed` decides what it is: a leading `=`
    /// is a formula in **canonical** syntax (a shell converts display form first, with
    /// [`formula::display::from_display`]), a leading `'` forces the rest to be text, an
    /// empty string clears the cell, and anything else is a number, a logical or text.
    ///
    /// One [`Action`], so a cell that held a formula loses it when a number is typed over
    /// it — which is what every spreadsheet does and what two separate calls would get
    /// wrong.
    ///
    /// With [`RecalcMode::Document`] the dependents are recalculated inside the same lock
    /// and land in the **same undo entry**, so one Ctrl+Z takes back the edit and its
    /// ripple. Unless it would *spoil* a cell — a cached value this build's function set
    /// cannot reproduce — in which case the edit still commits, the recalculation is
    /// skipped, and [`EnterOutcome::recalc`] says so: refusing the edit would make a
    /// document that uses one unimplemented function read-only, which is worse than stale.
    pub fn enter(
        &self,
        sheet: usize,
        pos: Pos,
        input: &str,
        recalc: RecalcMode,
    ) -> Result<EnterOutcome> {
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let (kind, edit) = typed(&state.doc, sheet, pos, input);
            let recalc = commit(state, sheet, vec![edit], recalc)?;
            Ok(EnterOutcome {
                kind,
                cells: 1,
                recalc,
            })
        })
    }

    /// Fill a rectangle from `anchor`, one row per row — the core half of a paste.
    ///
    /// Every cell goes through the same rule [`App::enter`] uses, so a pasted `=A1+1` is a
    /// formula and a pasted `'123` is text; the whole thing plus its recalculation is one
    /// [`Action::Batch`], so undoing a paste is one step. Ragged rows are fine: a row's
    /// cells land where the row puts them.
    ///
    /// Bounded by [`MAX_FORMATTED_CELLS`], for the reason [`App::set_format`] is.
    pub fn enter_range(
        &self,
        sheet: usize,
        anchor: Pos,
        rows: &[Vec<String>],
        recalc: RecalcMode,
    ) -> Result<EnterOutcome> {
        let width = rows.iter().map(Vec::len).max().unwrap_or(0) as u32;
        let last = Pos::new(
            anchor.row + rows.len().saturating_sub(1) as u32,
            anchor.col + width.saturating_sub(1),
        );
        // For the size check only; the cells themselves come from `rows`.
        let _bounded = self.rectangle(anchor, last)?;
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let mut kind = Entered::Cleared;
            let mut edits = Vec::new();
            for (r, row) in rows.iter().enumerate() {
                for (c, input) in row.iter().enumerate() {
                    let pos = Pos::new(anchor.row + r as u32, anchor.col + c as u32);
                    let (this, edit) = typed(&state.doc, sheet, pos, input);
                    if (r, c) == (0, 0) {
                        kind = this;
                    }
                    edits.push(edit);
                }
            }
            let cells = edits.len();
            let recalc = commit(state, sheet, edits, recalc)?;
            Ok(EnterOutcome {
                kind,
                cells,
                recalc,
            })
        })
    }

    /// Replicate `source` across a rectangle — a fill, the way a drag handle or Ctrl+D/
    /// Ctrl+R work elsewhere. A formula's relative references shift by each target cell's
    /// offset from `source` ([`formula::shift`]); its absolute ones do not. A plain value is
    /// copied as is.
    ///
    /// One [`Action::Batch`], bounded by [`MAX_FORMATTED_CELLS`], for the same reasons
    /// [`App::enter_range`] is.
    pub fn fill(
        &self,
        sheet: usize,
        source: Pos,
        start: Pos,
        end: Pos,
        recalc: RecalcMode,
    ) -> Result<EnterOutcome> {
        let targets: Vec<Pos> = self.rectangle(start, end)?.collect();
        let (formula, value) = {
            let state = self.state.read().unwrap();
            let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            (s.formula(source).map(str::to_owned), s.get(source))
        };
        // A formula this build cannot parse is copied verbatim rather than refused — the
        // same honesty `App::input_text` shows for one.
        let parsed = formula
            .as_deref()
            .and_then(|f| formula::parse::parse(f).ok());
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let mut kind = Entered::Cleared;
            let mut edits = Vec::new();
            for (i, pos) in targets.iter().copied().enumerate() {
                // The text to store as this target's formula, `None` when `source` holds a
                // plain value instead.
                let text = match (&parsed, &formula) {
                    (Some(expr), _) => {
                        let shifted = formula::shift::shift(
                            expr,
                            i64::from(pos.row) - i64::from(source.row),
                            i64::from(pos.col) - i64::from(source.col),
                        );
                        Some(format!("={shifted}"))
                    }
                    (None, Some(text)) => Some(text.clone()),
                    (None, None) => None,
                };
                let (this, edit) = match text {
                    Some(text) => {
                        let value = formula::eval::to_cell(
                            formula::eval::Engine::new(&state.doc)
                                .eval(&text, formula::eval::Address::new(sheet, pos)),
                        );
                        (
                            Entered::Formula,
                            Action::SetFormula {
                                sheet,
                                pos,
                                formula: Some(text),
                                value,
                            },
                        )
                    }
                    None => (
                        entered_kind(&value),
                        Action::SetFormula {
                            sheet,
                            pos,
                            formula: None,
                            value: value.clone(),
                        },
                    ),
                };
                if i == 0 {
                    kind = this;
                }
                edits.push(edit);
            }
            let cells = edits.len();
            let recalc = commit(state, sheet, edits, recalc)?;
            Ok(EnterOutcome {
                kind,
                cells,
                recalc,
            })
        })
    }

    /// What a formula *would* evaluate to, without storing anything.
    ///
    /// Three properties a shell leans on, and there is a test for each: it takes only a
    /// **read** lock, so it is safe from a worker thread; it notifies **no observer**; and
    /// it creates **no undo entry**. That is what makes a live result chip and a status-bar
    /// aggregate cheap enough to recompute while someone is typing.
    pub fn preview(&self, sheet: usize, pos: Pos, formula: &str) -> Result<CellValue> {
        let state = self.state.read().unwrap();
        if state.doc.sheet(sheet).is_none() {
            return Err(Error::NoSuchSheet(sheet));
        }
        Ok(formula::eval::to_cell(
            formula::eval::Engine::new(&state.doc)
                .eval(formula, formula::eval::Address::new(sheet, pos)),
        ))
    }

    /// Empty every cell in a rectangle, formulas included. Returns how many were not
    /// already empty.
    ///
    /// [`App::set_format`]'s shape, for the same reasons: one [`Action::Batch`] so that
    /// Delete over a selection is one undo step, bounded by [`MAX_FORMATTED_CELLS`], and a
    /// cell that is already empty is left out rather than written with what it holds.
    pub fn clear_range(&self, sheet: usize, start: Pos, end: Pos) -> Result<usize> {
        let cells = self.rectangle(start, end)?;
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = cells
                .filter(|pos| {
                    state
                        .doc
                        .sheet(sheet)
                        .is_some_and(|s| !s.get(*pos).is_empty() || s.formula(*pos).is_some())
                })
                .map(|pos| Action::SetFormula {
                    sheet,
                    pos,
                    formula: None,
                    value: CellValue::Empty,
                })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Drop a cell's formula, keeping the value it last computed.
    pub fn clear_formula(&self, sheet: usize, pos: Pos) -> Result<()> {
        self.mutate(|state| {
            let value = state
                .doc
                .sheet(sheet)
                .ok_or(Error::NoSuchSheet(sheet))?
                .get(pos);
            let inverse = state
                .doc
                .apply(Action::SetFormula {
                    sheet,
                    pos,
                    formula: None,
                    value,
                })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    // --- sheets ---
    //
    // Adding and removing a sheet shifts every later index, which the undo stack survives
    // because it is strictly ordered — see `Document::apply`. Nothing here reorders sheets:
    // a new one is appended, because that is the button a shell has, and a move is a
    // capability to add when something can ask for it rather than a parameter to carry now.

    /// Append an empty sheet, returning its index.
    pub fn add_sheet(&self, name: &str) -> Result<usize> {
        self.mutate(|state| {
            check_sheet_name(&state.doc, name, None)?;
            let index = state.doc.sheets.len();
            let inverse = state
                .doc
                .apply(Action::InsertSheet {
                    index,
                    sheet: Box::new(Sheet::new(name)),
                })
                .expect("appending is always in range");
            state.undo.push(inverse);
            state.redo.clear();
            Ok(index)
        })
    }

    /// Rename a sheet, **and everything in the document that names it** — `doc/dsl.md` §6.5's
    /// first refactoring, D10. Returns how many references were rewritten.
    ///
    /// A formula naming the old sheet, a named expression defined in terms of it, and a chart
    /// range built from it all follow the rename. It used to be a documented loss
    /// (`doc/not-doing.md`: the cells went stale and recalculated to `#REF!`), and the fix is the
    /// shape §6.5 argues every refactoring should have: **one [`Action::Batch`]**, so three
    /// hundred rewritten formulas are one Ctrl+Z and a failure part-way is not a half-renamed
    /// document.
    ///
    /// The rewrite is an AST substitution re-serialised by the printer
    /// ([`formula::rename`]), never a textual one — `Sales` occurs inside `SalesTax` and inside
    /// `"Sales"`, and a name that needs quoting is not spelled the same before and after. A
    /// formula this build cannot *parse* is left exactly as it was and is not counted: rewriting
    /// text nobody understood is how a refactoring corrupts a document. `grind sheet lint`'s
    /// `missing-sheet` is what finds one afterwards.
    pub fn rename_sheet(&self, sheet: usize, name: &str) -> Result<usize> {
        self.mutate(|state| {
            check_sheet_name(&state.doc, name, Some(sheet))?;
            let from = state
                .doc
                .sheet(sheet)
                .ok_or(Error::NoSuchSheet(sheet))?
                .name
                .clone();
            let mut actions = vec![Action::RenameSheet {
                index: sheet,
                name: name.to_owned(),
            }];
            actions.extend(references_renamed(&state.doc, &from, name));
            let rewritten = actions.len() - 1;
            let inverse = state
                .doc
                .apply(Action::Batch(actions))
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(rewritten)
        })
    }

    /// Remove a sheet and everything on it.
    ///
    /// The last one is refused: a document with no sheet is a spreadsheet with nowhere to
    /// type, and every shell would need a special case for it. Undoing brings the cells back
    /// — the inverse carries the whole sheet.
    ///
    /// Formulas on *other* sheets that name this one are left alone, for the same reason and
    /// with the same consequence as [`App::rename_sheet`].
    pub fn remove_sheet(&self, sheet: usize) -> Result<()> {
        self.mutate(|state| {
            if state.doc.sheets.len() <= 1 {
                return Err(Error::BadSheet(
                    "a document needs a sheet; this is the last one".to_owned(),
                ));
            }
            let inverse = state
                .doc
                .apply(Action::RemoveSheet { index: sheet })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Set — or with `None`, clear — the number format of every cell in a rectangle.
    ///
    /// A rectangle rather than one cell because formatting a column is the normal case, and
    /// one [`Action::Batch`] because undoing it should be one step. Returns how many cells
    /// were touched.
    ///
    /// The rectangle is bounded by [`MAX_FORMATTED_CELLS`]: a format costs an entry per
    /// cell (see `Sheet::formats`), so `A1:Z1000000` is a memory-exhaustion request rather
    /// than an intent, and refusing it is more useful than serving it slowly. A *whole
    /// column* does not get here — a shell resolves `A:A` against the used extent first.
    pub fn set_format(
        &self,
        sheet: usize,
        start: Pos,
        end: Pos,
        format: Option<numfmt::Format>,
    ) -> Result<usize> {
        let cells = self.rectangle(start, end)?;
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            // A no-op cell is left out of the batch, so re-applying a format a cell already
            // has does not push an undo entry that restores nothing.
            let updates = cells
                .filter(|pos| {
                    state.doc.sheet(sheet).and_then(|s| s.format(*pos)) != format.as_ref()
                })
                .map(|pos| Action::SetFormat {
                    sheet,
                    pos,
                    format: format.clone().map(Box::new),
                })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Every position in an inclusive rectangle, once its size has been checked.
    fn rectangle(&self, start: Pos, end: Pos) -> Result<impl Iterator<Item = Pos> + use<>> {
        let rows = u64::from(end.row.saturating_sub(start.row)) + 1;
        let cols = u64::from(end.col.saturating_sub(start.col)) + 1;
        if rows.saturating_mul(cols) > MAX_FORMATTED_CELLS {
            return Err(Error::TooLarge(rows.saturating_mul(cols)));
        }
        Ok((start.row..=end.row)
            .flat_map(move |row| (start.col..=end.col).map(move |col| Pos::new(row, col))))
    }

    /// Set — or with `None`, clear — the styling of every cell in a rectangle.
    ///
    /// The twin of [`App::set_format`] in every respect: one [`Action::Batch`] so undo is
    /// one step, bounded by [`MAX_FORMATTED_CELLS`] for the same reason, and it *replaces*
    /// rather than merges — a shell that wants "make this bold as well" reads the style
    /// first, which is one call and keeps this one from growing a merge policy.
    pub fn set_style(
        &self,
        sheet: usize,
        start: Pos,
        end: Pos,
        style: Option<style::CellStyle>,
    ) -> Result<usize> {
        let cells = self.rectangle(start, end)?;
        // A style that sets nothing is no style, so the two spellings of "plain" cannot
        // produce different documents.
        let style = style.filter(|s| !s.is_plain());
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = cells
                .filter(|pos| state.doc.sheet(sheet).and_then(|s| s.style(*pos)) != style.as_ref())
                .map(|pos| Action::SetStyle {
                    sheet,
                    pos,
                    style: style.clone().map(Box::new),
                })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Set — or with `None`, clear — the width of a run of columns (§5.4).
    ///
    /// A range rather than one column because dragging three column headers wider is one
    /// gesture and should be one undo step, and bounded by [`MAX_TRACK_RUN`] because that is
    /// how long a run this program will read back.
    ///
    /// The width is an ODF length as the document stores it — `"2.5cm"`, `"64pt"`. It is
    /// checked here rather than at each shell: a length nothing can parse would be a column
    /// that silently vanishes from every renderer, and `style::mm_length` is what a shell
    /// that has measured something in millimetres spells it with.
    pub fn set_col_width(
        &self,
        sheet: usize,
        cols: Range<u32>,
        width: Option<String>,
    ) -> Result<usize> {
        let width = self.track_size(cols.len() as u64, width)?;
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = cols
                .filter(|col| {
                    state.doc.sheet(sheet).and_then(|s| s.col_width(*col)) != width.as_deref()
                })
                .map(|col| Action::SetColWidth {
                    sheet,
                    col,
                    width: width.clone(),
                })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// The row twin of [`App::set_col_width`], in every respect.
    pub fn set_row_height(
        &self,
        sheet: usize,
        rows: Range<u32>,
        height: Option<String>,
    ) -> Result<usize> {
        let height = self.track_size(rows.len() as u64, height)?;
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = rows
                .filter(|row| {
                    state.doc.sheet(sheet).and_then(|s| s.row_height(*row)) != height.as_deref()
                })
                .map(|row| Action::SetRowHeight {
                    sheet,
                    row,
                    height: height.clone(),
                })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Hide — or with `hidden: false`, show — a run of columns by hand (§5.4's
    /// `table:visibility="collapse"`). Same shape as [`App::set_col_width`]: a range because
    /// selecting several headers and hiding them is one gesture and one undo step, and
    /// bounded by [`MAX_TRACK_RUN`] for the same reason.
    pub fn set_col_hidden(&self, sheet: usize, cols: Range<u32>, hidden: bool) -> Result<usize> {
        if cols.len() as u64 > u64::from(MAX_TRACK_RUN) {
            return Err(Error::TooLarge(cols.len() as u64));
        }
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = cols
                .filter(|col| {
                    state
                        .doc
                        .sheet(sheet)
                        .is_some_and(|s| s.col_hidden(*col) != hidden)
                })
                .map(|col| Action::SetColHidden { sheet, col, hidden })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// The row twin of [`App::set_col_hidden`].
    pub fn set_row_hidden(&self, sheet: usize, rows: Range<u32>, hidden: bool) -> Result<usize> {
        if rows.len() as u64 > u64::from(MAX_TRACK_RUN) {
            return Err(Error::TooLarge(rows.len() as u64));
        }
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let updates = rows
                .filter(|row| {
                    state
                        .doc
                        .sheet(sheet)
                        .is_some_and(|s| s.row_manually_hidden(*row) != hidden)
                })
                .map(|row| Action::SetRowHidden { sheet, row, hidden })
                .collect::<Vec<_>>();
            self::apply_batch(state, sheet, updates)
        })
    }

    /// Every column hidden by hand, in order — what a shell draws at zero width and offers
    /// to unhide.
    pub fn hidden_cols(&self, sheet: usize) -> Result<Vec<u32>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.hidden_cols().collect())
    }

    /// The row twin of [`App::hidden_cols`] — distinct from [`App::hidden_rows`], which is
    /// what the *filter* hides rather than what a person hid directly.
    pub fn manually_hidden_rows(&self, sheet: usize) -> Result<Vec<u32>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.manually_hidden_rows().collect())
    }

    /// A track size as it will be stored: bounded, and a length something can measure.
    fn track_size(&self, tracks: u64, size: Option<String>) -> Result<Option<String>> {
        if tracks > u64::from(MAX_TRACK_RUN) {
            return Err(Error::TooLarge(tracks));
        }
        match size {
            Some(size) => match style::length_mm(&size) {
                Some(mm) if mm > 0.0 => Ok(Some(size)),
                _ => Err(Error::Formula(format!("not a positive ODF length: {size}"))),
            },
            None => Ok(None),
        }
    }

    /// Every sized column of a sheet, in order — what a renderer turns into prefix sums.
    ///
    /// The whole sheet rather than a viewport's worth, because an offset has to be counted
    /// from column zero: a caller asking only about what is on screen would have to add up
    /// the columns before it anyway. Sparse and capped by [`MAX_TRACK_RUN`] on the way in,
    /// so this is a handful of entries rather than 16384.
    pub fn col_widths(&self, sheet: usize) -> Result<Vec<(u32, String)>> {
        let state = self.state.read().unwrap();
        let sheet = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(sheet.col_widths().map(|(c, w)| (c, w.to_owned())).collect())
    }

    /// The row twin of [`App::col_widths`].
    pub fn row_heights(&self, sheet: usize) -> Result<Vec<(u32, String)>> {
        let state = self.state.read().unwrap();
        let sheet = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(sheet
            .row_heights()
            .map(|(r, h)| (r, h.to_owned()))
            .collect())
    }

    /// Recalculate every formula in the document, storing the results.
    ///
    /// Only cells whose value actually changed go into the undo entry, so recalculating an
    /// already-current document is a no-op that leaves the history alone — and the whole
    /// recalculation is one [`Action::Batch`], so undoing it is one step.
    ///
    /// [`Recalc::spoiled`] is the number that matters to a caller. This engine implements a
    /// subset of OpenFormula, so recalculating a document that uses a function it does not
    /// have replaces a perfectly good cached value with `#NAME?`. That is the honest result
    /// of *this* recalculation, but it is also data loss, and a shell has to be able to say
    /// so rather than quietly writing the file back.
    pub fn recalc(&self) -> Result<Recalc> {
        self.mutate(|state| {
            let (updates, spoiled) = recalculated(&state.doc);
            let changed = updates.len();
            if changed > 0 {
                let inverse = state
                    .doc
                    .apply(Action::Batch(updates))
                    .expect("every sheet index came from the document itself");
                state.undo.push(inverse);
                state.redo.clear();
            }
            Ok(Recalc { changed, spoiled })
        })
    }

    /// How many formula cells hold a value that recalculating would change — and, of those,
    /// how many would *break*.
    ///
    /// A cached value and a formula are two claims about the same cell, and editing a cell a
    /// formula reads makes them disagree without touching the formula's own cell. The
    /// document is then internally inconsistent: it says `SUM(B2:B4)` and it says `1500`,
    /// and only one of those can be right. ODF has no "dirty" bit to write, and every reader
    /// including LibreOffice shows the cached value until something recalculates — so a
    /// caller that has just edited a cell has to be able to *tell the user*, which is what
    /// this is for.
    ///
    /// The same walk [`App::recalc`] does, without writing anything: no dependency graph, no
    /// second implementation of what counts as stale, and no chance of the two disagreeing.
    /// The cost is a full recalculation, which is the cost of `recalc` itself.
    ///
    /// `spoiled` is carried for the same reason it is on [`Recalc`]: a document is free to
    /// use any of Part 4's other ~370 functions, and a cell that would recalculate to
    /// `#NAME?` is a cell this build must not be trusted to fix. Stale-and-spoiled means
    /// "this needs recalculating and I am not the one who can do it".
    pub fn stale(&self) -> Recalc {
        let state = self.state.read().unwrap();
        let (updates, spoiled) = recalculated(&state.doc);
        Recalc {
            changed: updates.len(),
            spoiled,
        }
    }

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

    // --- documents ---

    /// Replace the document. History is dropped: an undo across a file boundary would
    /// apply an action addressed to a document that no longer exists.
    pub fn open_bytes(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let doc = read_bytes(name, bytes)?;
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

    /// Serialise the current document. Paired with [`App::save_file`] because the browser
    /// has no filesystem (doc/plan.md, rule 5), and this is the whole reason the core
    /// never exposes its `Document`: saving is an operation, not a getter.
    pub fn save_bytes(&self, form: Form) -> Result<Vec<u8>> {
        write_bytes(&self.state.read().unwrap().doc, form)
    }

    pub fn save_file(&self, path: &Path) -> Result<()> {
        write_file(&self.state.read().unwrap().doc, path)
    }

    /// The document as its **projection** — plain text, with the token and span maps beside it
    /// (`doc/dsl.md` §3).
    ///
    /// Two things at once, and that is the point of the milestone order: it is what a `.grind`
    /// file holds, and it is what a code view shows of a document nobody has saved (§6). A
    /// shell reads it, colours it from [`projection::Projection::tokens`], and asks
    /// [`projection::Projection::address_at`] which cell the caret is in.
    ///
    /// Whole-document rather than range-taking, which is a real tension with rule 1 and is
    /// resolved rather than ignored: §6.3 makes a code view an `Observer` over *its own*
    /// scroll window, and the range-taking call is D9's. Until a shell has one, the only
    /// caller is `grind sheet project`, for which the whole document is the request.
    ///
    /// **Nothing here writes to the document** — the same promise `get_viewport_with`'s
    /// overlays make, and for the same reason: a projection is a view of a model, not a
    /// second copy of one.
    pub fn project(&self) -> projection::Projection {
        projection::project(&self.state.read().unwrap().doc)
    }

    // --- reading ---

    pub fn get(&self, sheet: usize, pos: Pos) -> Result<CellValue> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.get(pos))
    }

    /// Read a rectangle. The only way a shell reads cells.
    ///
    /// Plain values, texts and styles — `get_viewport_with(…, Overlays::NONE)`. The overlays
    /// are a separate entry point rather than an argument here because Rust has no default
    /// arguments and every ordinary paint in four shells asks for none of them.
    pub fn get_viewport(
        &self,
        sheet: usize,
        rows: Range<u32>,
        cols: Range<u32>,
    ) -> Result<Viewport> {
        self.get_viewport_with(sheet, rows, cols, view::Overlays::NONE)
    }

    /// Read a rectangle, with `doc/view-modes.md`'s derived overlays on it.
    ///
    /// Requested rather than always computed: roles and name anchors both need a
    /// document-wide analysis — *nothing references this cell* cannot be answered inside a
    /// viewport — and a shell that draws neither must not pay for one. The analysis is built
    /// once per document state and cached; this read is viewport-shaped,
    /// which is rule 1 unchanged.
    ///
    /// **Nothing about this writes to the document.** That is the feature's whole promise
    /// and `sheet/tests/view_modes.rs` asserts it on the bytes.
    pub fn get_viewport_with(
        &self,
        sheet: usize,
        rows: Range<u32>,
        cols: Range<u32>,
        overlays: view::Overlays,
    ) -> Result<Viewport> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        // An inverted range is a caller bug, not a panic: normalise it to empty.
        let rows = rows.start..rows.end.max(rows.start);
        let cols = cols.start..cols.end.max(cols.start);
        let size = ((rows.end - rows.start) as usize) * ((cols.end - cols.start) as usize);
        let mut cells = Vec::with_capacity(size);
        let mut texts = Vec::with_capacity(size);
        let mut styles = Vec::with_capacity(size);
        let analysis = overlays.any().then(|| self.analysis(&state.doc));
        let mut roles = overlays.roles.then(|| Vec::with_capacity(size));
        for row in rows.clone() {
            for col in cols.clone() {
                let pos = Pos::new(row, col);
                let value = s.get(pos);
                texts.push(render(s, pos, state.doc.null_date));
                styles.push(s.style(pos).cloned());
                cells.push(value);
                if let Some(roles) = roles.as_mut() {
                    let at = formula::eval::Address::new(sheet, pos);
                    roles.push(analysis.as_ref().expect("requested").role(s, at));
                }
            }
        }
        let names = match (overlays.names, &analysis) {
            (true, Some(analysis)) => analysis
                .anchors_in(sheet, rows.clone(), cols.clone())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        Ok(Viewport {
            rows,
            cols,
            cells,
            texts,
            styles,
            roles,
            names,
        })
    }

    /// The document-wide analysis, built if it is not already there.
    ///
    /// `doc` is passed in rather than read again because every caller already holds the read
    /// lock — and taking it twice is how a lock order gets invented by accident.
    fn analysis(&self, doc: &Document) -> Arc<view::Analysis> {
        if let Some(analysis) = self.analysis.read().unwrap().clone() {
            return analysis;
        }
        // Two threads may both build one here. That costs a duplicated walk and cannot give
        // a wrong answer — both read the same document under the same read lock — and it is
        // preferable to holding a write lock across the build.
        let fresh = Arc::new(view::Analysis::build(doc));
        *self.analysis.write().unwrap() = Some(fresh.clone());
        fresh
    }

    pub fn sheet_count(&self) -> usize {
        self.state.read().unwrap().doc.sheets.len()
    }

    pub fn sheet_name(&self, sheet: usize) -> Result<String> {
        let state = self.state.read().unwrap();
        state
            .doc
            .sheet(sheet)
            .map(|s| s.name.clone())
            .ok_or(Error::NoSuchSheet(sheet))
    }

    /// One past the last used row and column of a sheet.
    pub fn used_extent(&self, sheet: usize) -> Result<(u32, u32)> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok((s.used_rows(), s.used_cols()))
    }

    /// What an editor puts in front of a user for one cell: the text that, entered back
    /// unchanged, leaves the cell exactly as it is.
    ///
    /// [`App::enter`]'s inverse, and in the core for that reason — a shell deriving it
    /// would be deriving the typing rule a second time, and the two would drift on the
    /// cases that matter. A text cell that would otherwise read as a number, a logical or a
    /// formula comes back with the `'` that forces it to text: `007` typed back without one
    /// is the number 7.
    ///
    /// A formula comes back in **display form** (`=SUM(B2:B4)`), so a shell hands it to
    /// [`formula::display::from_display`] on the way back in, exactly as it does for a
    /// formula the user typed. That one step is the whole difference between what an editor
    /// holds and what [`App::enter`] takes.
    ///
    /// A date or time comes back in the ISO spelling [`numfmt::general`] already promises is
    /// always typeable back in — `date_kind` decides a cell counts as one whenever its
    /// format says so, or failing that its [`model::NumberKind`] does, so a cell LibreOffice
    /// wrote with `office:date-value` and no style of its own still round-trips.
    pub fn input_text(&self, sheet: usize, pos: Pos) -> Result<String> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        if let Some(formula) = s.formula(pos) {
            // A formula that will not parse is shown as it is stored, which is the honest
            // answer and still editable.
            return Ok(formula::display::to_display(formula).unwrap_or_else(|_| formula.to_owned()));
        }
        Ok(match s.get(pos) {
            CellValue::Empty => String::new(),
            CellValue::Number(n) => match date_kind(s, pos) {
                Some(kind) => {
                    numfmt::general(&CellValue::Number(n), Some(kind), state.doc.null_date)
                }
                None => formula::value::format_number(n),
            },
            CellValue::Bool(true) => "TRUE".to_owned(),
            CellValue::Bool(false) => "FALSE".to_owned(),
            // The typing rule itself decides whether the `'` is needed, so there is one
            // rule rather than a copy of it.
            CellValue::Text(text) => match typed(&state.doc, sheet, pos, &text).0 {
                Entered::Text => text,
                _ => format!("'{text}"),
            },
        })
    }

    /// What the cell *displays* — a formula's result, formatted, rather than its source.
    /// "Copy Value" is this instead of [`App::input_text`].
    pub fn value_text(&self, sheet: usize, pos: Pos) -> Result<String> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(render(s, pos, state.doc.null_date))
    }

    /// How one cell looks, or `None` for a plain one.
    ///
    /// [`App::set_style`] *replaces* rather than merges, deliberately — so "make this bold as
    /// well" is this call, a field set, and one `set_style`. That read-merge-write is what a
    /// toolbar's bold button is, and the reason the merge policy lives in the caller that
    /// knows which button was pressed rather than in the core.
    pub fn style_at(&self, sheet: usize, pos: Pos) -> Result<Option<style::CellStyle>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.style(pos).cloned())
    }

    /// How one cell's value is spelled, or `None` for the general format.
    ///
    /// The twin of [`App::style_at`], and what a format picker shows as its current state.
    /// Not carried in the [`Viewport`], because rendering needs the *text*, which is already
    /// there — this is for the one cell a user is asking about.
    pub fn format_at(&self, sheet: usize, pos: Pos) -> Result<Option<numfmt::Format>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.format(pos).cloned())
    }

    /// A cell's formula in display form, **with the document's names substituted** —
    /// `doc/view-modes.md` §3.3. `None` for a cell holding a plain value.
    ///
    /// [`App::input_text`]'s reading rather than its replacement: what comes back here is
    /// for *showing*, and typing it back in would store the names rather than the
    /// references. A formula bar that lets a name be edited is a shell decision and this is
    /// the answer it reads either way.
    ///
    /// A formula that will not parse comes back exactly as it is stored, which is what
    /// [`App::input_text`] does with one and for the same reason.
    pub fn named_formula(&self, sheet: usize, pos: Pos) -> Result<Option<String>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        let Some(formula) = s.formula(pos) else {
            return Ok(None);
        };
        let at = formula::eval::Address::new(sheet, pos);
        // `view::Names` rather than the cached [`view::Analysis`] on purpose: this needs the
        // name anchors and nothing else, and building those is one parse per declared name
        // where an analysis recalculates the document. A formula bar asks per selection
        // change, and it must not cost a recalculation to read one cell.
        Ok(Some(
            view::Names::build(&state.doc)
                .display(&state.doc, at, formula)
                .unwrap_or_else(|_| formula.to_owned()),
        ))
    }

    /// A cell's formula source, or `None` if it holds a plain value.
    pub fn formula(&self, sheet: usize, pos: Pos) -> Result<Option<String>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.formula(pos).map(str::to_owned))
    }

    /// Check the document against [`lint`]'s rules — `doc/dsl.md` §4.3, D6.
    ///
    /// A read: nothing is stored, nothing is marked, and linting a document leaves its bytes
    /// exactly as they were — the same promise view modes make, and for the same reason (a
    /// stored classification goes stale and a derived one cannot). `grind sheet lint` is the
    /// CLI twin, and a shell that wants squiggles turns each address into a byte range through
    /// the projection's span map (§6.2) rather than by inventing a second addressing.
    pub fn lint(&self, options: &grind_core::lint::Options) -> grind_core::lint::Report {
        lint::lint(&self.state.read().unwrap().doc, options)
    }

    /// Every calculated cell in the document, sheet by sheet and address by address.
    ///
    /// What a "where is this number coming from" question needs answered in one place: a
    /// spreadsheet hides its formulas behind their results, and the only way to see them all
    /// is a list of them. Plain arithmetic counts — `=[.A1]/2` calls nothing and is still a
    /// calculation — so this is every formula cell rather than every cell calling a function,
    /// and [`Calculation::functions`] is empty rather than absent for one.
    ///
    /// Filtering is the caller's: a dialog's search box and a `--filter` flag want different
    /// matching, and neither wants the whole document re-read per keystroke.
    pub fn calculations(&self) -> Vec<Calculation> {
        let state = self.state.read().unwrap();
        let null_date = state.doc.null_date;
        state
            .doc
            .sheets
            .iter()
            .enumerate()
            .flat_map(|(index, s)| {
                s.formulas().map(move |(pos, formula)| Calculation {
                    sheet: index,
                    sheet_name: s.name.clone(),
                    pos,
                    // Display form, because that is the spelling a user reads in the formula
                    // bar; one that will not parse is shown exactly as it is stored.
                    formula: formula::display::to_display(formula)
                        .unwrap_or_else(|_| formula.to_owned()),
                    value: render(s, pos, null_date),
                    functions: formula::funcs::used(formula).unwrap_or_default(),
                })
            })
            .collect()
    }

    pub fn formula_count(&self, sheet: usize) -> Result<usize> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.formula_count())
    }

    /// Define a named expression, or redefine one (§5.11).
    ///
    /// `expression` is OpenFormula text without the `of:=` prefix, exactly as a formula
    /// stores it — `[$Data.$A$1:.$B$9]` for a named range, `SUM([$Data.$A$1:.$A$9])` for a
    /// named expression. The two are the same thing to everything downstream, which is why
    /// the reader turns a `table:named-range` into the reference it stands for.
    ///
    /// It is **parsed before it is stored**, because a name is used from every formula in
    /// the document: a broken one is not one broken cell but `#NAME?` everywhere it is
    /// mentioned, and the address it was typed at is long gone by then.
    pub fn set_name(&self, name: &str, expression: &str) -> Result<()> {
        validate_name(name)?;
        formula::parse::parse(expression).map_err(|e| Error::Formula(e.to_string()))?;
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::SetName {
                    name: name.to_owned(),
                    expression: Some(expression.to_owned()),
                })
                .expect("a name addresses no sheet");
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Delete a named expression. `false` if there was no such name.
    ///
    /// Deleting one a formula still mentions is allowed and turns that formula into
    /// `#NAME?` at the next recalculation — the same answer LibreOffice gives, and refusing
    /// would mean a name could never be removed from a document that once used it.
    pub fn clear_name(&self, name: &str) -> bool {
        self.mutate(|state| {
            if !state.doc.names.contains_key(&name.to_lowercase()) {
                return false;
            }
            let inverse = state
                .doc
                .apply(Action::SetName {
                    name: name.to_owned(),
                    expression: None,
                })
                .expect("a name addresses no sheet");
            state.undo.push(inverse);
            state.redo.clear();
            true
        })
    }

    /// Every named expression, name and expression alike (§5.11).
    pub fn names(&self) -> Vec<(String, String)> {
        self.state
            .read()
            .unwrap()
            .doc
            .names
            .iter()
            .map(|(name, expr)| (name.clone(), expr.clone()))
            .collect()
    }

    /// Set — or with `None`, remove — a sheet's autofilter (§9.4).
    ///
    /// One filter per sheet, replaced rather than merged: a second filter over the same
    /// sheet is not a thing ODF's `table:database-range` can say twice about one range, and
    /// merging two would be inventing an answer.
    ///
    /// Which rows this hides is *not* stored — see [`crate::filter`] — so nothing else has
    /// to be kept in step, and a value edit changes the hidden rows immediately.
    pub fn set_filter(&self, sheet: usize, filter: Option<Filter>) -> Result<()> {
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::SetFilter {
                    sheet,
                    filter: filter.map(Box::new),
                })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// A sheet's autofilter, if it has one.
    pub fn filter(&self, sheet: usize) -> Result<Option<Filter>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.filter().cloned())
    }

    /// The rows the filter hides, in order — what a shell leaves undrawn, and empty when
    /// there is no filter.
    pub fn hidden_rows(&self, sheet: usize) -> Result<Vec<u32>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.hidden_rows(state.doc.null_date))
    }

    // --- charts ---

    /// Add a chart to a sheet, resolving `categories` and each series' `(values, label)` the
    /// way a formula's own reference resolves — see [`chart::parse_range`]. `label` names a
    /// range too (`chart:label-cell-address`, usually one cell); pass `None` for a series with
    /// no name of its own.
    #[allow(clippy::too_many_arguments)]
    pub fn add_chart(
        &self,
        sheet: usize,
        kind: ChartKind,
        categories: Option<&str>,
        series: &[(&str, Option<&str>)],
        x: &str,
        y: &str,
        width: &str,
        height: &str,
        x_axis: chart::Axis,
        y_axis: chart::Axis,
    ) -> Result<()> {
        let (categories, series) = self.resolve_chart_ranges(sheet, categories, series)?;
        let mut chart = Chart::new(
            kind,
            x.to_owned(),
            y.to_owned(),
            width.to_owned(),
            height.to_owned(),
        );
        chart.categories = categories;
        chart.series = series;
        chart.x_axis = x_axis;
        chart.y_axis = y_axis;

        self.mutate(|state| {
            let sheet_len = state
                .doc
                .sheet(sheet)
                .ok_or(Error::NoSuchSheet(sheet))?
                .charts()
                .len();
            let inverse = state
                .doc
                .apply(Action::InsertChart {
                    sheet,
                    index: sheet_len,
                    chart: Box::new(chart),
                })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Every chart on a sheet, in document order.
    pub fn charts(&self, sheet: usize) -> Result<Vec<Chart>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.charts().to_vec())
    }

    /// Remove the chart at `index` on `sheet`.
    pub fn remove_chart(&self, sheet: usize, index: usize) -> Result<()> {
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::RemoveChart { sheet, index })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Move or resize the chart at `index` — `x`/`y`/`width`/`height` are ODF lengths
    /// (`"2.5cm"`), the same as [`Self::add_chart`] takes. One undo step however many times a
    /// shell calls this mid-drag is `App`'s caller's job, not this method's — see
    /// `grind-sheet-gtk`'s own commit-on-release, which only calls this once the pointer is
    /// released rather than on every pixel of motion.
    pub fn reshape_chart(
        &self,
        sheet: usize,
        index: usize,
        x: &str,
        y: &str,
        width: &str,
        height: &str,
    ) -> Result<()> {
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::ReshapeChart {
                    sheet,
                    index,
                    x: x.to_owned(),
                    y: y.to_owned(),
                    width: width.to_owned(),
                    height: height.to_owned(),
                })
                .ok_or_else(|| Error::BadSheet(format!("sheet {sheet} has no chart {index}")))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Replace a chart's axes and its series (colours included) wholesale, one undo step —
    /// the same "send the whole mutable bundle" shape [`Self::reshape_chart`] uses. A caller
    /// reads the chart's current state through [`Self::charts`], mutates the one field it
    /// cares about (an axis' title, tick labels or gridlines, or one series'
    /// `color`/`point_colors`), and calls this with the rest unchanged.
    ///
    /// The series here are **already resolved** — full range-address strings carrying their
    /// colours, straight out of [`Self::charts`]. To change what a chart *points at*, in the
    /// vocabulary a user types, use [`Self::edit_chart`] instead.
    pub fn set_chart_style(
        &self,
        sheet: usize,
        index: usize,
        x_axis: chart::Axis,
        y_axis: chart::Axis,
        series: Vec<chart::Series>,
    ) -> Result<()> {
        let mut chart = self.chart(sheet, index)?;
        chart.x_axis = x_axis;
        chart.y_axis = y_axis;
        chart.series = series;
        self.replace_chart(sheet, index, chart)
    }

    /// Change what a chart *is*: its kind, the ranges it points at and its axes, in the same
    /// vocabulary [`Self::add_chart`] takes — one undo step. Its position and size are left
    /// alone (they are [`Self::reshape_chart`]'s, and a dialog that reopened would otherwise
    /// undo a drag).
    ///
    /// **A colour a user picked by hand survives an edit** that leaves the series it was
    /// picked on pointing at the same range: colours are matched back on by range address
    /// rather than by position, so adding a series above another one does not shuffle the
    /// colours down. A series whose range *changed* starts again from the default cycle,
    /// since a colour chosen for the fourth bar of one range means nothing on another.
    // Eight arguments, one over the limit, and the same eight `add_chart` takes minus the
    // geometry plus the index — an "options struct" here would only be this list with a name.
    #[allow(clippy::too_many_arguments)]
    pub fn edit_chart(
        &self,
        sheet: usize,
        index: usize,
        kind: ChartKind,
        categories: Option<&str>,
        series: &[(&str, Option<&str>)],
        x_axis: chart::Axis,
        y_axis: chart::Axis,
    ) -> Result<()> {
        let (categories, mut series) = self.resolve_chart_ranges(sheet, categories, series)?;
        let mut chart = self.chart(sheet, index)?;
        for new in &mut series {
            if let Some(old) = chart.series.iter().find(|old| old.values == new.values) {
                new.color = old.color.clone();
                new.point_colors = old.point_colors.clone();
            }
        }
        chart.kind = kind;
        chart.categories = categories;
        chart.series = series;
        chart.x_axis = x_axis;
        chart.y_axis = y_axis;
        self.replace_chart(sheet, index, chart)
    }

    /// One chart, by index — the read every chart edit starts from, since each of them
    /// replaces the whole value (`Action::ReplaceChart`).
    fn chart(&self, sheet: usize, index: usize) -> Result<Chart> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        s.charts()
            .get(index)
            .cloned()
            .ok_or_else(|| Error::BadSheet(format!("sheet {sheet} has no chart {index}")))
    }

    /// The write half of the same pair: one `Action::ReplaceChart`, one undo entry.
    fn replace_chart(&self, sheet: usize, index: usize, chart: Chart) -> Result<()> {
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::ReplaceChart {
                    sheet,
                    index,
                    chart: Box::new(chart),
                })
                .ok_or_else(|| Error::BadSheet(format!("sheet {sheet} has no chart {index}")))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// A chart's ranges as a user types them, turned into the ODF range addresses a chart
    /// stores — shared by [`Self::add_chart`] and [`Self::edit_chart`], and run **before** the
    /// mutation, because resolving a reference takes the same lock the mutation does.
    fn resolve_chart_ranges(
        &self,
        sheet: usize,
        categories: Option<&str>,
        series: &[(&str, Option<&str>)],
    ) -> Result<(Option<String>, Vec<chart::Series>)> {
        let categories = categories
            .map(|addr| chart::parse_range(self, sheet, addr))
            .transpose()?;
        let series = series
            .iter()
            .map(|(values, label)| {
                Ok(chart::Series {
                    values: chart::parse_range(self, sheet, values)?,
                    label: label
                        .map(|addr| chart::parse_range(self, sheet, addr))
                        .transpose()?,
                    color: None,
                    point_colors: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((categories, series))
    }

    /// A chart's data, resolved against the live sheet — what a shell draws from.
    pub fn chart_data(&self, sheet: usize, index: usize) -> Result<ChartData> {
        // Read and released before `ChartData::read` takes the lock again for the cells.
        let chart = self.chart(sheet, index)?;
        ChartData::read(self, &chart)
    }

    // --- history across processes ---

    /// The undo and redo stacks, for a shell that cannot stay running.
    pub fn session(&self) -> Session {
        let state = self.state.read().unwrap();
        Session {
            undo: state.undo.clone(),
            redo: state.redo.clone(),
        }
    }

    /// Restore stacks taken from [`App::session`].
    ///
    /// Safe against a document loaded separately because an inverse carries the value it
    /// restores — an action never has to consult the document it was recorded against. An
    /// entry naming a sheet that no longer exists simply fails to apply, which
    /// [`App::undo`] already reports as `false`.
    pub fn restore_session(&self, session: Session) {
        self.mutate(|state| {
            state.undo = session.undo;
            state.redo = session.redo;
        });
    }
}

/// Whether an edit recalculates the document behind it (doc/sheet-shell.md C3).
///
/// An enum rather than a bool because the useful third answer — recalculate only what
/// depends on the cell that changed — is what `eval.rs`'s `ponytail:` note is about, and it
/// arrives as a variant rather than as a second parameter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecalcMode {
    /// Leave the rest of the document alone; [`App::stale`] is how a caller finds out.
    #[default]
    No,
    /// Recalculate every formula, in the same undo entry as the edit.
    Document,
}

/// What [`App::enter`] made of the text it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entered {
    Formula,
    Number,
    Bool,
    Text,
    Cleared,
}

/// What one [`App::enter`] or [`App::enter_range`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnterOutcome {
    /// What the input became — the anchor cell's, for a range.
    pub kind: Entered,
    /// How many cells were written.
    pub cells: usize,
    /// The recalculation, when one was asked for. `spoiled > 0` means it was **skipped**
    /// and `changed` is what recalculating *would* have changed — the number a banner
    /// reports, and the reason it has to be offered rather than done.
    pub recalc: Option<Recalc>,
}

/// What one cell displays: its own number format applied, or the general one (§5.2).
///
/// One function because a viewport and a list of calculations must not disagree about what
/// a cell reads as.
pub(crate) fn render(sheet: &Sheet, pos: Pos, null_date: i64) -> String {
    let value = sheet.get(pos);
    match sheet.format(pos) {
        Some(format) => format.render(&value, null_date),
        None => numfmt::general(&value, sheet.kind(pos), null_date),
    }
}

/// Whether a cell counts as a date or a time — its format if it has one and the format
/// says so, otherwise the [`model::NumberKind`] the reader left behind (§4.3.3's rule for
/// a date carrying no style of its own).
///
/// An explicit format wins because it is the one the user can see and change; a plain
/// number a user has *formatted* as a date is one they mean as a date even if the cell
/// never carried an `office:date-value`.
fn date_kind(sheet: &Sheet, pos: Pos) -> Option<model::NumberKind> {
    match sheet.format(pos).map(|f| f.kind) {
        Some(numfmt::Kind::Date) => Some(model::NumberKind::Date),
        Some(numfmt::Kind::Time) => Some(model::NumberKind::Time),
        _ => sheet.kind(pos),
    }
}

/// The typing rule: what a string a user typed means (doc/sheet-shell.md C3).
///
/// One [`Action::SetFormula`] whatever the answer, because every one of these outcomes also
/// has to *remove* whatever formula the cell held — typing `5` over `=SUM(A1:A4)` replaces
/// it, and two actions would leave the formula behind with a new cached value.
///
/// A formula's cached value is computed against the document as it stands, which is both
/// what [`App::set_formula`] does and what pressing Enter in a spreadsheet does.
/// What kind of thing a value already stored in a cell is — [`App::fill`]'s answer for a
/// source cell that holds no formula, where [`typed`] has no input text to classify.
fn entered_kind(value: &CellValue) -> Entered {
    match value {
        CellValue::Empty => Entered::Cleared,
        CellValue::Number(_) => Entered::Number,
        CellValue::Bool(_) => Entered::Bool,
        CellValue::Text(_) => Entered::Text,
    }
}

fn typed(doc: &Document, sheet: usize, pos: Pos, input: &str) -> (Entered, Action) {
    let cell = |kind, value| {
        (
            kind,
            Action::SetFormula {
                sheet,
                pos,
                formula: None,
                value,
            },
        )
    };
    // The `'` rule is load-bearing rather than a convenience: without it the strings `=x`
    // and `123` cannot be typed into a cell at all.
    if let Some(text) = input.strip_prefix('\'') {
        return cell(Entered::Text, CellValue::Text(text.to_owned()));
    }
    if input.starts_with('=') {
        let value = formula::eval::to_cell(
            formula::eval::Engine::new(doc).eval(input, formula::eval::Address::new(sheet, pos)),
        );
        return (
            Entered::Formula,
            Action::SetFormula {
                sheet,
                pos,
                formula: Some(input.to_owned()),
                value,
            },
        );
    }
    // A cell already known to hold a date or a time (by format or by `NumberKind`, see
    // `date_kind`) accepts its own ISO spelling back — `input_text`'s exact inverse, not a
    // general "anything that looks like a date is a date" rule (doc/sheet-shell.md C3 defers
    // that on purpose).
    if let Some(kind) = doc.sheet(sheet).and_then(|s| date_kind(s, pos)) {
        let parsed = match kind {
            model::NumberKind::Date => formula::date::parse_date(input, doc.null_date),
            model::NumberKind::Time => formula::date::parse_time(input),
        };
        if let Some(n) = parsed {
            return cell(Entered::Number, CellValue::Number(n));
        }
    }
    if let Ok(n) = input.parse::<f64>() {
        return cell(Entered::Number, CellValue::Number(n));
    }
    match input {
        "TRUE" | "true" => cell(Entered::Bool, CellValue::Bool(true)),
        "FALSE" | "false" => cell(Entered::Bool, CellValue::Bool(false)),
        "" => cell(Entered::Cleared, CellValue::Empty),
        _ => cell(Entered::Text, CellValue::Text(input.to_owned())),
    }
}

/// Apply an edit and, if asked, the recalculation behind it — as **one** undo entry.
///
/// The order matters and is the whole point: the edit lands first, the recalculation is
/// computed against the document that results, and the inverses are pushed reversed so that
/// undoing runs the recalculation back before the edit that caused it.
fn commit(
    state: &mut State,
    sheet: usize,
    edits: Vec<Action>,
    mode: RecalcMode,
) -> Result<Option<Recalc>> {
    let mut inverses = Vec::new();
    if !edits.is_empty() {
        inverses.push(
            state
                .doc
                .apply(Action::Batch(edits))
                .ok_or(Error::NoSuchSheet(sheet))?,
        );
    }
    let recalc = match mode {
        RecalcMode::No => None,
        RecalcMode::Document => {
            let (updates, spoiled) = recalculated(&state.doc);
            let changed = updates.len();
            // Spoiling is the one case where the edit outlives its own recalculation.
            if spoiled == 0 && changed > 0 {
                inverses.push(
                    state
                        .doc
                        .apply(Action::Batch(updates))
                        .expect("every sheet index came from the document itself"),
                );
            }
            Some(Recalc { changed, spoiled })
        }
    };
    if let Some(last) = inverses.pop() {
        inverses.reverse();
        inverses.insert(0, last);
        state.undo.push(match inverses.len() {
            1 => inverses.pop().expect("just checked"),
            _ => Action::Batch(inverses),
        });
        state.redo.clear();
    }
    Ok(recalc)
}

/// Apply a batch and record its inverse, or do nothing at all when it is empty.
fn apply_batch(state: &mut State, sheet: usize, updates: Vec<Action>) -> Result<usize> {
    let changed = updates.len();
    if changed > 0 {
        let inverse = state
            .doc
            .apply(Action::Batch(updates))
            .ok_or(Error::NoSuchSheet(sheet))?;
        state.undo.push(inverse);
        state.redo.clear();
    }
    Ok(changed)
}

/// What one [`App::recalc`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recalc {
    /// Cells whose stored value the recalculation replaced.
    pub changed: usize,
    /// Of those, how many held a real value and now hold an error — almost always a
    /// function this build does not implement (`sheet functions` lists the ones it does).
    /// Undoing the recalculation is one step, which is the way back.
    pub spoiled: usize,
}

/// Whether `name` can be this document's sheet `except`'s name — or a new sheet's, with
/// `except` being `None`.
///
/// Two rules, and deliberately no third. A sheet name is `table:name`, whose schema type is
/// a plain string, so the *characters* are not ours to police: the serialiser already quotes
/// anything a reference cannot spell bare (`serialize.rs`, `'It''s a sheet'`).
///
/// * Empty is refused, because `[.A1]` and `['".A1]` would then be the same reference.
/// * A duplicate is refused, case-insensitively, because that is how a reference resolves it
///   (§5.8) — two sheets differing only in case would make `[$data.$A$1]` mean either.
fn check_sheet_name(doc: &Document, name: &str, except: Option<usize>) -> Result<()> {
    if name.trim().is_empty() {
        return Err(Error::BadSheet("a sheet needs a name".to_owned()));
    }
    let taken = doc
        .sheets
        .iter()
        .enumerate()
        .any(|(i, s)| Some(i) != except && s.name.eq_ignore_ascii_case(name));
    match taken {
        true => Err(Error::BadSheet(format!(
            "a sheet is already called {name:?}"
        ))),
        false => Ok(()),
    }
}

/// Whether a string is a name a formula could actually mention (§5.11 `Identifier`).
///
/// Checked when a name is *defined*, in the core, because a name that cannot be lexed is
/// unreachable: the document would carry it and every formula naming it would still say
/// `#NAME?`. Storing one is therefore never useful, whoever asked.
///
/// Two rules, both from the grammar rather than from taste:
///
/// * it must lex as a single [`formula::lex::Token::Name`] — which rules out spaces,
///   punctuation, a leading digit, and the empty string, and admits any Unicode letter
///   because §5.6's `LetterXML` is [XML1.0]'s Letter production;
/// * it must not *also* read as a **cell** address. `A1` and `$B$7` are names the grammar
///   allows and addresses every user and every other spreadsheet's name box reads first, so
///   one would mean two things at once. LibreOffice refuses them, and refuses `Q1` for the
///   same reason, which is worth knowing before naming a quarter.
///
/// A column or a row alone — `SALES` lexes as `[.SALES]`, a whole column — is *not* refused,
/// and that is the difference between this and a first guess. OpenFormula always brackets a
/// reference, so `SUM(SALES)` is unambiguous in the file whatever `SALES` would mean inside
/// brackets; refusing it would rule out most of the words anyone wants to name a range.
fn validate_name(name: &str) -> Result<()> {
    let refused = |why: &str| {
        Err(Error::Formula(format!(
            "{name:?} is not a valid name: {why}"
        )))
    };
    match formula::lex::lex(name) {
        Ok(tokens) => match tokens.as_slice() {
            [formula::lex::Token::Name(n)] if n == name => {}
            _ => {
                return refused(
                    "a name is one identifier — a letter or _, then letters, digits or _",
                );
            }
        },
        Err(_) => {
            return refused("a name is one identifier — a letter or _, then letters, digits or _");
        }
    }
    // `[.A1]` is how a reference is spelled, so lexing the name inside brackets asks the
    // same question a reader would, without a second address parser to disagree with the
    // first. Both axes present is what makes it a *cell* rather than a whole column.
    if let Ok(tokens) = formula::lex::lex(&format!("[.{name}]"))
        && let [formula::lex::Token::Ref(r)] = tokens.as_slice()
        && r.start.row.is_some()
        && r.start.col.is_some()
    {
        return refused("it is also a cell address, and everything reads that first");
    }
    Ok(())
}

/// Every formula cell whose stored value disagrees with a fresh evaluation, as the actions
/// that would fix them, and how many of those would replace a good value with an error.
///
/// One walk, shared by [`App::recalc`] (which applies it) and [`App::stale`] (which counts
/// it), so "what recalculating would do" and "what recalculating does" cannot drift apart.
fn recalculated(doc: &Document) -> (Vec<Action>, usize) {
    let differences = differences(doc);
    let spoiled = differences.iter().filter(|(.., spoiled)| *spoiled).count();
    let updates = differences
        .into_iter()
        .map(|(sheet, pos, value, _)| Action::SetCell { sheet, pos, value })
        .collect();
    (updates, spoiled)
}

/// Every formula cell whose computed value differs from its cached one: where it is, what the
/// formula computes, and whether the difference is *this build's* gap rather than the
/// document's — `true` when a good value would be replaced by an error, which is what
/// [`Recalc::spoiled`] counts.
///
/// One walk, because two callers ask the same question for different reasons: recalculating
/// writes the new values, and [`lint::STALE_VALUE`] reports that the old ones are wrong. A
/// second implementation of "what counts as stale" is exactly the kind of disagreement
/// `doc/dsl.md` §4.3 exists to catch in *documents*, and it would be worse in the code.
fn differences(doc: &Document) -> Vec<(usize, Pos, CellValue, bool)> {
    let mut out = Vec::new();
    // The engine borrows the document immutably and memoises per cell, so this is one pass
    // whatever the dependency order.
    let mut engine = formula::eval::Engine::new(doc);
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, _) in sheet.formulas() {
            let at = formula::eval::Address::new(index, pos);
            let value = formula::eval::to_cell(engine.value(at));
            let previous = sheet.get(pos);
            if value == previous {
                continue;
            }
            let spoiled = is_error(&value) && !is_error(&previous) && !previous.is_empty();
            out.push((index, pos, value, spoiled));
        }
    }
    out
}

/// Every edit a sheet rename implies, besides the rename itself — `doc/dsl.md` §6.5, D10.
///
/// Three kinds of thing in a document name a sheet, and a refactoring that reached only the
/// first would be worse than none at all: a cell's **formula**, a **named expression**'s
/// definition (which every formula using that name reads through), and a **chart**'s ranges,
/// which are address strings rather than formulas and so come through `chart::rename_sheet_in_range`.
///
/// A separate function from [`App::rename_sheet`] because it takes the document rather than the
/// lock, which is what makes it testable and what keeps the mutation itself three lines.
fn references_renamed(doc: &Document, from: &str, to: &str) -> Vec<Action> {
    let mut actions = Vec::new();
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, text) in sheet.formulas() {
            if let Some(formula) = formula::rename::rename_in_formula(text, from, to) {
                actions.push(Action::SetFormula {
                    sheet: index,
                    pos,
                    formula: Some(formula),
                    // The cached value is untouched: pointing a reference at a renamed sheet
                    // changes what it is *called* and not what it reads, so nothing about the
                    // answer has changed and marking the cell stale would be a lie.
                    value: sheet.get(pos),
                });
            }
        }
    }
    for (name, expression) in &doc.names {
        if let Some(expression) = formula::rename::rename_in_formula(expression, from, to) {
            actions.push(Action::SetName {
                name: name.clone(),
                expression: Some(expression),
            });
        }
    }
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (position, chart) in sheet.charts().iter().enumerate() {
            let mut renamed = chart.clone();
            let mut changed = false;
            let mut retarget = |addr: &mut String| {
                if let Some(new) = chart::rename_sheet_in_range(addr, from, to) {
                    *addr = new;
                    changed = true;
                }
            };
            if let Some(categories) = renamed.categories.as_mut() {
                retarget(categories);
            }
            for series in &mut renamed.series {
                retarget(&mut series.values);
                if let Some(label) = series.label.as_mut() {
                    retarget(label);
                }
            }
            if changed {
                actions.push(Action::ReplaceChart {
                    sheet: index,
                    index: position,
                    chart: Box::new(renamed),
                });
            }
        }
    }
    actions
}

/// The cells [`lint::STALE_VALUE`] reports: a disagreement this build can actually settle.
///
/// The spoiled ones are dropped here rather than in the rule, because *which* differences are
/// this engine's fault is a fact about the evaluator and belongs beside it.
fn stale_cells(doc: &Document) -> Vec<(usize, Pos, CellValue)> {
    differences(doc)
        .into_iter()
        .filter(|(.., spoiled)| !spoiled)
        .map(|(sheet, pos, value, _)| (sheet, pos, value))
        .collect()
}

/// Whether a cell holds one of §4.6's error names. Errors live in a cell as their name in
/// text, because that is the only shape [`CellValue`] has for one — see
/// [`formula::eval::to_cell`].
fn is_error(value: &CellValue) -> bool {
    match value {
        CellValue::Text(s) => formula::value::FormulaError::from_name(s).is_some(),
        _ => false,
    }
}

/// Undo history, serialised so it can outlive the process that built it.
///
/// The document is *not* part of it: a stateless shell re-reads the file each time, and the
/// stacks are the only thing that would otherwise be lost.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Session {
    /// `#[serde(default)]` on both, so a session file written by an older build still loads
    /// rather than failing a user's next command.
    #[serde(default)]
    pub undo: Vec<Action>,
    #[serde(default)]
    pub redo: Vec<Action>,
}

/// Read an `.ods` (package) or `.fods` (flat) document.
pub fn read_file(path: &Path) -> Result<Document> {
    read_bytes(&path.display().to_string(), &std::fs::read(path)?)
}

/// Read a document from bytes. Paired with [`read_file`] from the start because the
/// browser has no filesystem, and this is not retrofittable later (doc/plan.md, rule 5).
///
/// The form — zip package, flat XML, or the projection (`doc/dsl.md`) — is sniffed from the
/// bytes, so `name` is only ever a label for diagnostics.
pub fn read_bytes(_name: &str, bytes: &[u8]) -> Result<Document> {
    // The projection is the third physical form, and `grind_core::projection` decides whether
    // these bytes are one from their first line — never from the file's name, which is the
    // same rule `Form` and `kind` already follow.
    if grind_core::projection::is_projection(bytes).is_some() {
        let text =
            std::str::from_utf8(bytes).map_err(|e| grind_core::Error::Projection(e.to_string()))?;
        return projection::read(text);
    }
    odf::read(bytes)
}

/// Serialise a document. See [`Form`].
pub fn write_bytes(doc: &Document, form: Form) -> Result<Vec<u8>> {
    odf::write(doc, form)
}

/// Write a document, choosing the form from the extension — `.ods` the package,
/// **anything else flat** ([`Form::from_path`], `doc/flat-first.md`).
///
/// The only place in the codebase where a file extension decides anything: reading sniffs
/// the form from the bytes, but writing has to pick one, and the name the user typed is
/// the only statement of intent available.
pub fn write_file(doc: &Document, path: &Path) -> Result<()> {
    std::fs::write(path, write_bytes(doc, Form::from_path(path))?)?;
    Ok(())
}
