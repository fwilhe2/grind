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

pub use action::Action;
pub use model::{CellValue, Document, Pos, Sheet};

#[derive(Debug)]
pub enum Error {
    /// Not built yet. Carries the phase that removes it.
    Unimplemented(&'static str),
    NoSuchSheet(usize),
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
        let mut cells = Vec::with_capacity(
            ((rows.end - rows.start) as usize) * ((cols.end - cols.start) as usize),
        );
        for row in rows.clone() {
            for col in cols.clone() {
                cells.push(s.get(Pos::new(row, col)));
            }
        }
        Ok(Viewport { rows, cols, cells })
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
