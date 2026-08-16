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

pub mod action;
pub mod formula;
pub mod grid;
pub mod model;
pub mod numfmt;
pub mod odf;
pub mod style;

pub use action::Action;
pub use model::{CellValue, Document, Pos, Sheet};
pub use odf::Form;

#[derive(Debug)]
pub enum Error {
    /// Not built yet. Carries the phase that removes it.
    Unimplemented(&'static str),
    NoSuchSheet(usize),
    /// A request whose size is the problem rather than its content — see
    /// [`MAX_FORMATTED_CELLS`].
    TooLarge(u64),
    /// The XML would not parse at all. Per doc/ods-format.md §8.2 this is the *structural*
    /// failure case — unrecognised content never reaches here, it is ignored instead.
    Xml(String),
    /// The zip container would not open, or holds no `content.xml`.
    Package(String),
    /// Password-protected. The document is fine; we have no key. Distinct from
    /// [`Error::Xml`] so callers can tell "cannot open" from "will not parse".
    Encrypted,
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unimplemented(what) => write!(f, "unimplemented: {what}"),
            Error::NoSuchSheet(i) => write!(f, "no such sheet: {i}"),
            Error::TooLarge(n) => write!(
                f,
                "{n} cells is more than the {MAX_FORMATTED_CELLS} a format may cover at once"
            ),
            Error::Xml(e) => write!(f, "xml: {e}"),
            Error::Package(e) => write!(f, "package: {e}"),
            Error::Encrypted => write!(f, "password-protected document"),
            Error::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// How many cells one [`App::set_format`] may cover.
///
/// Formats are stored per cell, so this bounds the memory a single command can ask for.
/// Well above any rectangle a person selects, well below a whole sheet.
pub const MAX_FORMATTED_CELLS: u64 = 1 << 16;

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
}

impl Viewport {
    pub fn get(&self, row: u32, col: u32) -> Option<&CellValue> {
        if !self.rows.contains(&row) || !self.cols.contains(&col) {
            return None;
        }
        let width = (self.cols.end - self.cols.start) as usize;
        let r = (row - self.rows.start) as usize;
        let c = (col - self.cols.start) as usize;
        self.cells.get(r * width + c)
    }

    /// The display text of one cell — what a renderer draws.
    pub fn text(&self, row: u32, col: u32) -> Option<&str> {
        if !self.rows.contains(&row) || !self.cols.contains(&col) {
            return None;
        }
        let width = (self.cols.end - self.cols.start) as usize;
        let r = (row - self.rows.start) as usize;
        let c = (col - self.cols.start) as usize;
        self.texts.get(r * width + c).map(String::as_str)
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

/// Notified after every change. Implemented by shells; the core calls it, shells never
/// poll (doc/plan.md, rule 3).
pub trait Observer: Send + Sync {
    fn changed(&self);
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
                .filter(|pos| state.doc.sheet(sheet).and_then(|s| s.format(*pos)) != format.as_ref())
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
            let mut updates = Vec::new();
            let mut spoiled = 0;
            {
                // The engine borrows the document immutably and memoises per cell, so this
                // is one pass whatever the dependency order.
                let mut engine = formula::eval::Engine::new(&state.doc);
                for (index, sheet) in state.doc.sheets.iter().enumerate() {
                    for (pos, _) in sheet.formulas() {
                        let at = formula::eval::Address::new(index, pos);
                        let value = formula::eval::to_cell(engine.value(at));
                        let previous = sheet.get(pos);
                        if value == previous {
                            continue;
                        }
                        if is_error(&value) && !is_error(&previous) && !previous.is_empty() {
                            spoiled += 1;
                        }
                        updates.push(Action::SetCell {
                            sheet: index,
                            pos,
                            value,
                        });
                    }
                }
            }
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
        self.open_bytes(
            &path.display().to_string(),
            &std::fs::read(path).map_err(Error::Io)?,
        )
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

    // --- reading ---

    pub fn get(&self, sheet: usize, pos: Pos) -> Result<CellValue> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.get(pos))
    }

    /// Read a rectangle. The only way a shell reads cells.
    pub fn get_viewport(
        &self,
        sheet: usize,
        rows: Range<u32>,
        cols: Range<u32>,
    ) -> Result<Viewport> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        // An inverted range is a caller bug, not a panic: normalise it to empty.
        let rows = rows.start..rows.end.max(rows.start);
        let cols = cols.start..cols.end.max(cols.start);
        let size = ((rows.end - rows.start) as usize) * ((cols.end - cols.start) as usize);
        let mut cells = Vec::with_capacity(size);
        let mut texts = Vec::with_capacity(size);
        for row in rows.clone() {
            for col in cols.clone() {
                let pos = Pos::new(row, col);
                let value = s.get(pos);
                texts.push(match s.format(pos) {
                    Some(format) => format.render(&value, state.doc.null_date),
                    None => numfmt::general(&value, s.kind(pos), state.doc.null_date),
                });
                cells.push(value);
            }
        }
        Ok(Viewport {
            rows,
            cols,
            cells,
            texts,
        })
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

    /// A cell's formula source, or `None` if it holds a plain value.
    pub fn formula(&self, sheet: usize, pos: Pos) -> Result<Option<String>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.formula(pos).map(str::to_owned))
    }

    pub fn formula_count(&self, sheet: usize) -> Result<usize> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.formula_count())
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
    read_bytes(
        &path.display().to_string(),
        &std::fs::read(path).map_err(Error::Io)?,
    )
}

/// Read a document from bytes. Paired with [`read_file`] from the start because the
/// browser has no filesystem, and this is not retrofittable later (doc/plan.md, rule 5).
///
/// The form — zip package or flat XML — is sniffed from the bytes, so `name` is only ever
/// a label for diagnostics.
pub fn read_bytes(_name: &str, bytes: &[u8]) -> Result<Document> {
    odf::read(bytes)
}

/// Serialise a document. See [`Form`].
pub fn write_bytes(doc: &Document, form: Form) -> Result<Vec<u8>> {
    odf::write(doc, form)
}

/// Write a document, choosing the form from the extension — `.fods` flat, anything else
/// the package form.
///
/// The only place in the codebase where a file extension decides anything: reading sniffs
/// the form from the bytes, but writing has to pick one, and the name the user typed is
/// the only statement of intent available.
pub fn write_file(doc: &Document, path: &Path) -> Result<()> {
    let form = match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("fods") || e.eq_ignore_ascii_case("xml") => Form::Flat,
        _ => Form::Package,
    };
    std::fs::write(path, write_bytes(doc, form)?).map_err(Error::Io)
}
