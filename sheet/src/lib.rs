// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ODF spreadsheet core.
//!
//! All state and all logic live here; every front-end is a renderer and an event forwarder
//! that owns nothing. Reads go through [`App::get_viewport`] — no getter hands out the whole
//! document. `doc/ods-format.md` is the clean-room notes on the format.

use std::fmt;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, RwLock};

mod app;

// The document and what changes it.
pub mod a1;
pub mod action;
pub mod chart;
pub mod filter;
pub mod formula;
pub mod graph;
pub mod grid;
pub mod model;
pub mod numfmt;
pub mod rule;
pub mod style;
pub mod table_format;

// In and out: ODF, CSV, the projection, and lint.
pub mod csv;
pub mod lint;
pub mod odf;
pub mod projection;

// What every shell would otherwise work out for itself, decided here so that no two disagree.
pub mod autoheight;
pub mod chart_frame;
pub mod chart_paint;
pub mod clip;
pub mod find;
pub mod fit;
pub mod format;
pub mod look;
pub mod nav;
pub mod place;
pub mod reading;
pub mod spell;
pub mod summary;
pub mod tracks;
pub mod verbs;
pub mod view;

/// What this crate takes from `grind-core` and hands on, so a spreadsheet-only shell says
/// `grind_sheet::` for everything.
pub use grind_core::{DocumentKind, Form, Observer, build_info, kind, locale};

pub use action::Action;
pub use chart::{
    Axis as ChartAxis, Chart, ChartData, ChartKind, Guess as ChartGuess, Legend as ChartLegend,
    Series as ChartSeries, Shape as ChartShape, Slice, Spec as ChartSpec, Ticks, axis_ticks,
    effective_color, pie_slice_at, pie_slices, series_color,
};
pub use filter::Filter;
pub use model::{CellValue, Checkbox, Document, Link, Pos, Sheet, Span};
pub use reading::Reading;
pub use spell::{Lexicon, Misspelling};
pub use table_format::{TableOptions, TotalsFunction};

/// What can go wrong with a spreadsheet. The failures any document can have — a zip, XML,
/// encryption, the filesystem — are [`grind_core::Error`], reached through [`Error::Odf`] (R8).
#[derive(Debug)]
pub enum Error {
    /// Something true of any ODF document, not just a spreadsheet.
    Odf(grind_core::Error),
    NoSuchSheet(usize),
    /// A request whose size is the problem — see [`MAX_FORMATTED_CELLS`].
    TooLarge(u64),
    /// A sheet operation ODF has no spelling for: a name another sheet already has, an
    /// empty one, or removing the document's last sheet.
    BadSheet(String),
    /// A formula or a name that is not what §5 says one is, rejected when defined
    /// ([`App::set_name`]).
    Formula(String),
    /// A correction aimed at a cell that no longer holds the word it names ([`App::correct`]).
    Stale(String),
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
            Error::Stale(e) => write!(f, "{e}"),
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
    /// Whether this is `Error::Odf(Encrypted)`, without the nesting at every call site.
    pub fn is_encrypted(&self) -> bool {
        matches!(self, Error::Odf(grind_core::Error::Encrypted))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// How big a sheet is: the bounds every other spreadsheet uses, and what the reader clamps a
/// repeat count to. Not a limit on what [`formula::lex`] parses — a foreign file may name a cell
/// past the end and must still load (R5) — but on where an address becomes a place, in
/// [`a1::resolve`].
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

/// How many cells one [`App::set_format`] may cover. Formats are stored per cell, so this bounds
/// the memory one command can ask for: well above any selection, well below a whole sheet.
pub const MAX_FORMATTED_CELLS: u64 = 1 << 16;

/// How many columns or rows one [`App::set_col_width`] may cover, and on reading how long a run of
/// equally sized tracks is still layout rather than background. A
/// `table:number-columns-repeated="16368"` column says "the rest of the sheet is the default
/// width", so the reader's cap and the writer's limit are one number.
///
/// ponytail: a document that really sizes 2000 columns loses them past the cap. A per-sheet
/// default size would keep them, when a file needs it.
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

    /// Whether a search finds this one — its sheet, address, formula or a function it calls,
    /// case-insensitively; everything for an empty needle.
    pub fn matches(&self, needle: &str) -> bool {
        let needle = needle.trim().to_uppercase();
        needle.is_empty()
            || self.address().to_uppercase().contains(&needle)
            || self.formula.to_uppercase().contains(&needle)
            || self.functions.iter().any(|f| f.contains(&needle))
    }
}

/// Which functions a set of calculations uses, commonest first, ties alphabetical.
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

/// A rectangle of values, already resolved — what a renderer draws. Rows and columns past the end
/// of the sheet come back empty, since scrolling into blank space is normal.
#[derive(Clone, Debug, PartialEq)]
pub struct Viewport {
    pub rows: Range<u32>,
    pub cols: Range<u32>,
    cells: Vec<CellValue>, // row-major
    /// What each cell displays, its number format applied (`doc/ods-format.md` §5), since a shell
    /// has neither the format nor the epoch.
    texts: Vec<String>,
    /// How each cell looks (`style.rs`), carried beside the text so a renderer takes the lock
    /// once.
    ///
    /// ponytail: a `CellStyle` of nine `Option<String>`s is cloned per visible cell; interning
    /// them is the upgrade once a profile says it matters.
    styles: Vec<Option<style::CellStyle>>,
    /// What each cell is — `doc/view-modes.md`'s role overlay, row-major. `None` unless the read
    /// asked for it ([`view::Overlays`]): roles need a document-wide analysis.
    roles: Option<Vec<view::CellRole>>,
    /// The name anchors intersecting this rectangle — a list, since a name binds a range as often
    /// as a cell.
    names: Vec<view::NameAnchor>,
    /// The merges intersecting this rectangle, each with its top-left cell's text and look, since
    /// a merge half scrolled out of view is drawn from a cell outside the rectangle.
    merges: Vec<Merged>,
    /// The checkboxes drawn in this rectangle and whether each is ticked — a linked one's state is
    /// another cell's value, perhaps off screen.
    checkboxes: Vec<(Pos, bool)>,
    /// Which cells hold a plain number in the General format, the only ones that narrow to fit a
    /// column ([`Viewport::narrower`]).
    general: Vec<bool>,
    locale: Option<locale::Locale>,
    /// The misspelt words of each cell drawn here that has any, as character ranges into what it
    /// displays — empty without a dictionary ([`App::set_lexicon`]).
    misspelt: Vec<(Pos, Vec<Range<usize>>)>,
}

/// One merged range as a renderer draws it: the whole area, with what its top-left cell shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Merged {
    /// The top-left cell, which holds the value.
    pub anchor: Pos,
    /// The bottom-right cell, inclusive.
    pub end: Pos,
    /// The top-left cell's value, which decides which side of the merge an unaligned text sits on.
    pub value: CellValue,
    /// What the top-left cell displays.
    pub text: String,
    /// How the top-left cell looks — the look of the whole area.
    pub style: Option<style::CellStyle>,
}

impl Merged {
    /// Whether this merge takes in `(row, col)`.
    pub fn contains(&self, row: u32, col: u32) -> bool {
        (self.anchor.row..=self.end.row).contains(&row)
            && (self.anchor.col..=self.end.col).contains(&col)
    }
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

    /// The shorter spellings to try, in order, when a cell's [`Viewport::text`] does not fit its
    /// column ([`numfmt::narrower`]). Empty except for a plain number in the General format; any
    /// other number that does not fit is `###` (`doc/ods-format.md` §5.2).
    pub fn narrower(&self, row: u32, col: u32) -> Vec<String> {
        let Some(at) = self.at(row, col) else {
            return Vec::new();
        };
        match (self.general.get(at), self.cells.get(at)) {
            (Some(true), Some(CellValue::Number(n))) => numfmt::narrower(*n, self.locale.as_ref()),
            _ => Vec::new(),
        }
    }

    /// How one cell looks, or `None` for a plain cell and for one outside the viewport alike. Its
    /// font size is relative to the document's default, as 10pt is to a shell's own font.
    pub fn style(&self, row: u32, col: u32) -> Option<&style::CellStyle> {
        self.styles.get(self.at(row, col)?)?.as_ref()
    }

    /// What one cell is, or `None` when the read did not ask for roles or the cell is outside the
    /// viewport.
    pub fn role(&self, row: u32, col: u32) -> Option<view::CellRole> {
        self.roles.as_ref()?.get(self.at(row, col)?).copied()
    }

    /// Every name anchor intersecting this rectangle; empty when not asked for.
    pub fn names(&self) -> &[view::NameAnchor] {
        &self.names
    }

    /// The name bound to one cell, if any.
    pub fn name_at(&self, row: u32, col: u32) -> Option<&str> {
        self.names
            .iter()
            .find(|a| a.rows.contains(&row) && a.cols.contains(&col))
            .map(|a| a.name.as_str())
    }

    /// Every merge intersecting this rectangle, in address order of its top-left cell.
    pub fn merges(&self) -> &[Merged] {
        &self.merges
    }

    /// The merge one cell is part of, its top-left or a covered cell. A renderer draws the
    /// top-left cell over the whole area.
    pub fn merge_at(&self, row: u32, col: u32) -> Option<&Merged> {
        self.merges.iter().find(|m| m.contains(row, col))
    }

    /// Whether a checkbox is drawn in this cell, and if so whether it is ticked.
    pub fn checkbox(&self, row: u32, col: u32) -> Option<bool> {
        self.checkboxes
            .iter()
            .find(|(pos, _)| *pos == Pos::new(row, col))
            .map(|(_, ticked)| *ticked)
    }

    /// Every checkbox in this rectangle, with whether it is ticked.
    pub fn checkboxes(&self) -> &[(Pos, bool)] {
        &self.checkboxes
    }

    /// The misspelt words in what one cell displays, as character ranges into [`Viewport::text`].
    /// A merge's words are its top-left cell's.
    pub fn misspelt(&self, row: u32, col: u32) -> &[Range<usize>] {
        self.misspelt
            .iter()
            .find(|(pos, _)| *pos == Pos::new(row, col))
            .map_or(&[], |(_, ranges)| ranges.as_slice())
    }

    /// Whether one cell is hidden under a merge — inside one and not its top-left cell.
    pub fn covered(&self, row: u32, col: u32) -> bool {
        self.merge_at(row, col)
            .is_some_and(|m| m.anchor != Pos::new(row, col))
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
    /// Bumped by every [`App::mutate`], so work done under a read lock can tell the document moved
    /// before it writes.
    version: u64,
    /// [`App::defer_recalc`]: `RecalcMode::Document` records that a recalculation is owed
    /// instead of doing it.
    defer_recalc: bool,
    /// A recalculation was deferred and nobody has done it yet ([`App::take_recalc_owed`]).
    recalc_owed: bool,
}

/// The application. Shells hold an `Arc<App>` and call these methods; every method takes `&self`
/// and the lock lives inside. Editing, history, the document and reading it are below; charts,
/// names, formatting, a sheet's structure and spelling are in `app/`.
#[derive(Default)]
pub struct App {
    state: RwLock<State>,
    observer: RwLock<Option<Arc<dyn Observer>>>,
    /// `doc/view-modes.md`'s document-wide analysis, computed on first use and dropped in
    /// [`App::mutate`], the one place a document changes. Its own lock, so a read can fill it
    /// without the write lock; the order is always state, then analysis.
    analysis: RwLock<Option<Arc<view::Analysis>>>,
    /// The dictionary spelling is checked against ([`App::set_lexicon`]) — beside the document
    /// rather than in it, since it is never written (`doc/spelling.md`, decision 5). Locked after
    /// `state`.
    lexicon: RwLock<Option<Arc<dyn spell::Lexicon>>>,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_observer(&self, observer: Arc<dyn Observer>) {
        *self.observer.write().unwrap() = Some(observer);
    }

    /// Mutate, then notify with the write lock released: an observer calls straight back in to
    /// re-read (`an_observer_may_read_the_app_without_deadlocking`).
    fn mutate<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let result = {
            let mut state = self.state.write().unwrap();
            let result = f(&mut state);
            state.version += 1;
            result
        };
        // Before notifying, so an observer reading back in is not handed the old analysis.
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

    /// Set a formula and its cached value together (`doc/ods-format.md` §4): a formula saved
    /// without a value renders blank in LibreOffice.
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

    /// What pressing Enter means — the typing rule. A leading `=` is a formula in canonical syntax
    /// (a shell converts display form first, [`formula::display::from_display`]), a leading `'`
    /// forces text, an empty string clears the cell, and anything else is a number, a logical or
    /// text. One [`Action`], so typing a number over a formula removes the formula.
    ///
    /// With [`RecalcMode::Document`] the dependents are recalculated in the same undo entry —
    /// unless that would spoil a cell (a cached value this build cannot reproduce): then the edit
    /// commits, the recalculation is skipped, and [`EnterOutcome::recalc`] says so.
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
            let (kind, edit, reading) = typed(&state.doc, sheet, pos, input);
            let recalc = commit(state, sheet, vec![edit], recalc)?;
            Ok(EnterOutcome {
                kind,
                cells: 1,
                recalc,
                readings: usize::from(reading.is_some()),
                reading,
            })
        })
    }

    /// Fill a rectangle from `anchor`, one row per row — the core half of a paste. Every cell goes
    /// through [`App::enter`]'s rule, and the whole paste and its recalculation are one undo step.
    /// Ragged rows are fine. Bounded by [`MAX_FORMATTED_CELLS`].
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
            let (mut reading, mut readings) = (None, 0);
            for (r, row) in rows.iter().enumerate() {
                for (c, input) in row.iter().enumerate() {
                    let pos = Pos::new(anchor.row + r as u32, anchor.col + c as u32);
                    let (this, edit, read) = typed(&state.doc, sheet, pos, input);
                    if (r, c) == (0, 0) {
                        kind = this;
                    }
                    if read.is_some() {
                        readings += 1;
                        reading = reading.or(read);
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
                reading,
                readings,
            })
        })
    }

    /// Read a CSV or TSV file into the sheet, starting at `anchor`: a paste with a parser in
    /// front. [`csv::parse`] cuts the fields, [`csv::input`] says what a person would have typed
    /// to mean each one, and the typing rule decides the rest.
    ///
    /// [`csv::Import::dates`] formats the target cells as dates first, because the typing rule
    /// reads an ISO date only into a cell formatted as one; then the fields are read, and both
    /// passes are one undo entry. Bounded by [`MAX_FORMATTED_CELLS`] like any paste; bulk data
    /// wants a generator.
    pub fn import_csv(
        &self,
        sheet: usize,
        anchor: Pos,
        text: &str,
        options: &csv::Import,
        recalc: RecalcMode,
    ) -> Result<EnterOutcome> {
        let fields = csv::parse(text, &options.dialect);
        let width = fields.iter().map(Vec::len).max().unwrap_or(0) as u32;
        let last = Pos::new(
            anchor.row + fields.len().saturating_sub(1) as u32,
            anchor.col + width.saturating_sub(1),
        );
        // For the size check only, exactly as `enter_range` does it.
        let _bounded = self.rectangle(anchor, last)?;
        self.mutate(|state| {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let at =
                |row: usize, col: usize| Pos::new(anchor.row + row as u32, anchor.col + col as u32);

            // Pass one: the formats date fields need, applied now, since `typed` reads the
            // document rather than the batch. `--text` outranks it.
            let mut inverses = Vec::new();
            if options.dates && !options.text {
                let null_date = state.doc.null_date;
                let formats: Vec<Action> = fields
                    .iter()
                    .enumerate()
                    .flat_map(|(row, line)| {
                        line.iter().enumerate().filter_map(move |(col, field)| {
                            csv::dated(field, null_date).map(|format| Action::SetFormat {
                                sheet,
                                pos: at(row, col),
                                format: Some(Box::new(format)),
                            })
                        })
                    })
                    .collect();
                if !formats.is_empty() {
                    inverses.push(
                        state
                            .doc
                            .apply(Action::Batch(formats))
                            .ok_or(Error::NoSuchSheet(sheet))?,
                    );
                }
            }

            // Pass two: the values, through the rule a keystroke uses.
            let mut kind = Entered::Cleared;
            let mut edits = Vec::new();
            for (row, line) in fields.iter().enumerate() {
                for (col, field) in line.iter().enumerate() {
                    let pos = at(row, col);
                    // `csv::input` spells a number the ISO way, whatever the file or the
                    // document spell one, so it is read the ISO way too.
                    let (this, edit, _) = typed_in(
                        &state.doc,
                        sheet,
                        pos,
                        &csv::input(field, options),
                        None,
                        true,
                    );
                    if (row, col) == (0, 0) {
                        kind = this;
                    }
                    edits.push(edit);
                }
            }
            let cells = edits.len();
            let recalc = commit_after(state, sheet, inverses, edits, recalc)?;
            Ok(EnterOutcome {
                kind,
                cells,
                recalc,
                reading: None,
                readings: 0,
            })
        })
    }

    /// Write a rectangle out as CSV or TSV: what each cell shows, so a date leaves as a date — the
    /// format is the only place a document says which a number is. [`csv::Export::formulas`]
    /// writes formulas instead, in display form. Hands back text, not cells (rule 1).
    pub fn export_csv(
        &self,
        sheet: usize,
        start: Pos,
        end: Pos,
        options: &csv::Export,
    ) -> Result<String> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        let mut rows = Vec::with_capacity((end.row.saturating_sub(start.row) + 1) as usize);
        for row in start.row..=end.row {
            let mut fields = Vec::with_capacity((end.col.saturating_sub(start.col) + 1) as usize);
            for col in start.col..=end.col {
                let pos = Pos::new(row, col);
                let formula = options.formulas.then(|| s.formula(pos)).flatten();
                fields.push(match formula {
                    // A formula that will not parse goes out as stored.
                    Some(formula) => {
                        formula::display::to_display(formula).unwrap_or_else(|_| formula.to_owned())
                    }
                    None => render_in(s, pos, state.doc.null_date, state.doc.locale.as_ref()),
                });
            }
            rows.push(fields);
        }
        Ok(csv::write(&rows, options))
    }

    /// Replicate `source` across a rectangle, as a drag handle or Ctrl+D/Ctrl+R does: relative
    /// references shift by each target's offset ([`formula::shift`]), absolute ones do not, and a
    /// plain value is copied. One undo step, bounded by [`MAX_FORMATTED_CELLS`].
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
        // A formula that will not parse is copied verbatim.
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
                reading: None,
                readings: 0,
            })
        })
    }

    /// What a formula would evaluate to, without storing anything: a read lock only, no observer
    /// notified and no undo entry, so a live result or a status-bar sum can be recomputed while
    /// someone types.
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

    /// A formula taken apart: every sub-expression of `formula` (display syntax, `=` optional) and
    /// what it comes to at `pos`, nothing stored (`doc/formula-workbench.md` §3.2).
    /// [`App::preview`]'s guarantees, as the workbench asks on every keystroke.
    ///
    /// `overlay` holds cells that read differently for this trace only ([`App::overlay`] builds
    /// one); `&Overlay::default()` traces the document as it is.
    pub fn trace(
        &self,
        sheet: usize,
        pos: Pos,
        formula: &str,
        overlay: &formula::trace::Overlay,
    ) -> Result<formula::trace::Trace> {
        let state = self.state.read().unwrap();
        if state.doc.sheet(sheet).is_none() {
            return Err(Error::NoSuchSheet(sheet));
        }
        formula::trace::trace(&state.doc, sheet, pos, formula, overlay)
            .map_err(|e| Error::Formula(e.to_string()))
    }

    /// An [`formula::trace::Overlay`] from what a person typed for each cell, each read by
    /// [`App::enter`]'s rule for that cell. A formula is evaluated to its value. Nothing is
    /// stored.
    pub fn overlay(&self, entries: &[(usize, Pos, &str)]) -> Result<formula::trace::Overlay> {
        let state = self.state.read().unwrap();
        let mut overlay = formula::trace::Overlay::default();
        for &(sheet, pos, input) in entries {
            if state.doc.sheet(sheet).is_none() {
                return Err(Error::NoSuchSheet(sheet));
            }
            let Action::SetFormula { value, .. } = typed(&state.doc, sheet, pos, input).1 else {
                unreachable!("the typing rule only ever sets a cell");
            };
            overlay.set(sheet, pos, formula::eval::from_cell(value));
        }
        Ok(overlay)
    }

    /// Empty every cell in a rectangle, formulas included, as one undo step bounded by
    /// [`MAX_FORMATTED_CELLS`]; already empty cells are left out. Returns how many were not
    /// already empty.
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

    // --- find and replace ---

    /// Every cell a [`find::Search`] matches, in reading order — sheet by sheet, row by row —
    /// searching each cell's [`App::input_text`] within its sheet's used extent.
    pub fn find(&self, search: &find::Search) -> Result<Vec<find::Hit>> {
        let state = self.state.read().unwrap();
        hits(&state.doc, search)
    }

    /// Replace every occurrence [`App::find`] would report, as one undo step. Each rewritten cell
    /// goes back through the typing rule: a formula stays a formula, a date cell reads its ISO
    /// spelling, and `12` replaced into `123` is a number. A cell whose result is a formula that
    /// will not parse is left alone and listed in [`find::Replaced::refused`]. Bounded by
    /// [`MAX_FORMATTED_CELLS`] changed cells; with [`RecalcMode::Document`] the recalculation
    /// lands in the same entry.
    pub fn replace(
        &self,
        search: &find::Search,
        with: &str,
        recalc: RecalcMode,
    ) -> Result<find::Replaced> {
        self.mutate(|state| {
            let mut refused = Vec::new();
            let mut edits = Vec::new();
            for hit in hits(&state.doc, search)? {
                let Some(input) = search.replaced(&hit.text, with) else {
                    continue;
                };
                let input = match formula::display::to_input(&input) {
                    Ok(input) => input,
                    Err(e) => {
                        refused.push((hit, e.to_string()));
                        continue;
                    }
                };
                edits.push(typed(&state.doc, hit.sheet, hit.pos, &input).1);
            }
            if edits.len() as u64 > MAX_FORMATTED_CELLS {
                return Err(Error::TooLarge(edits.len() as u64));
            }
            let cells = edits.len();
            let recalc = match cells {
                0 => None,
                _ => commit(state, search.sheet.unwrap_or(0), edits, recalc)?,
            };
            Ok(find::Replaced {
                cells,
                refused,
                recalc,
            })
        })
    }

    // --- sheets ---
    //
    // Adding or removing a sheet shifts every later index; the undo stack survives because it is
    // strictly ordered (`Document::apply`). A new sheet is appended; there is no move yet.

    /// The name a new sheet is offered: the first `SheetN` no sheet has, compared without regard
    /// to case.
    pub fn fresh_sheet_name(&self) -> String {
        let taken: Vec<String> = (0..self.sheet_count())
            .filter_map(|i| self.sheet_name(i).ok())
            .collect();
        (1..)
            .map(|n| format!("Sheet{n}"))
            .find(|name| !taken.iter().any(|t| t.eq_ignore_ascii_case(name)))
            .expect("there is always a free number")
    }

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

    /// Rename a sheet, and every formula, named expression and chart range that names it, as one
    /// undo step (`doc/dsl.md` §6.5). Returns how many references were rewritten.
    ///
    /// The rewrite is an AST substitution ([`formula::rename`]), never a textual one. A formula
    /// that will not parse is left as it was and not counted; `grind sheet lint`'s `missing-sheet`
    /// finds it.
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

    /// Remove a sheet and everything on it; undo brings it back. The last sheet is refused.
    /// Formulas on other sheets naming this one are left as they are, and recalculate to `#REF!`.
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

    /// Recalculate every formula in the document, storing the results as one undo step; only
    /// changed cells go in, so recalculating a current document leaves the history alone.
    ///
    /// [`Recalc::spoiled`] counts cached values replaced by an error — usually `#NAME?` for a
    /// function this build lacks. That is data loss, which a shell must be able to tell the user
    /// about.
    pub fn recalc(&self) -> Result<Recalc> {
        self.mutate(|state| {
            state.recalc_owed = false;
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

    /// Make `RecalcMode::Document` owe a recalculation rather than perform one. A full
    /// recalculation takes as long as the document is big — a minute for 600,000 formulas — and
    /// [`App::enter`] holds the write lock while it runs. A shell that cannot wait learns one is
    /// owed from [`App::take_recalc_owed`] and runs [`App::recalc_in_place`] off its UI thread.
    pub fn defer_recalc(&self, on: bool) {
        let mut state = self.state.write().unwrap();
        state.defer_recalc = on;
        if !on {
            state.recalc_owed = false;
        }
    }

    /// Whether a recalculation was deferred ([`App::defer_recalc`]) since the last one, clearing
    /// the answer.
    pub fn take_recalc_owed(&self) -> bool {
        std::mem::take(&mut self.state.write().unwrap().recalc_owed)
    }

    /// [`App::recalc`] computed under a read lock, so the document can still be read and drawn
    /// meanwhile, and written only if nothing changed in between. `progress` gets `(formulas done,
    /// formulas in all)`.
    ///
    /// `merge` adds the result to the top undo entry, so a deferred recalculation belongs to the
    /// edit that owed it; `force` writes even when it would spoil cached values. `Ok(None)` means
    /// the document changed meanwhile and nothing was written. The caller must not write to the
    /// document while this runs.
    pub fn recalc_in_place(
        &self,
        merge: bool,
        force: bool,
        progress: impl FnMut(usize, usize),
    ) -> Result<Option<Recalc>> {
        let (version, (updates, spoiled)) = {
            let state = self.state.read().unwrap();
            (state.version, recalculated_with(&state.doc, progress))
        };
        self.mutate(|state| {
            if state.version != version {
                return Ok(None);
            }
            state.recalc_owed = false;
            let changed = updates.len();
            if changed > 0 && (spoiled == 0 || force) {
                let inverse = state
                    .doc
                    .apply(Action::Batch(updates))
                    .expect("every sheet index came from the document itself");
                match state.undo.pop() {
                    Some(edit) if merge => state.undo.push(Action::Batch(vec![inverse, edit])),
                    Some(edit) => {
                        state.undo.push(edit);
                        state.undo.push(inverse);
                    }
                    None => state.undo.push(inverse),
                }
                state.redo.clear();
            }
            Ok(Some(Recalc { changed, spoiled }))
        })
    }

    /// How many formula cells hold a value that recalculating would change, and how many of those
    /// would break. Editing a cell a formula reads leaves the formula's cached value wrong, and
    /// ODF has no dirty bit, so a shell needs this to tell the user. The same walk [`App::recalc`]
    /// does, without writing; it costs a full recalculation. `spoiled` is as on [`Recalc`].
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

    /// Replace the document. History is dropped: an undo would apply to a document that no longer
    /// exists.
    pub fn open_bytes(&self, name: &str, bytes: &[u8]) -> Result<()> {
        self.open_document(read_bytes(name, bytes)?)
    }

    /// Hold a document already read — [`read_bytes`] is the slow half and takes no `App`, so a
    /// shell can run it on a worker thread.
    pub fn open_document(&self, doc: Document) -> Result<()> {
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

    /// The document as its projection — plain text, with the token and span maps beside it
    /// (`doc/dsl.md` §3): what a `.grind` file holds and what every shell's source view shows.
    /// Whole-document, so a source view reads all of it. Nothing is written.
    pub fn project(&self) -> projection::Projection {
        projection::project(&self.state.read().unwrap().doc)
    }

    /// Where each address is in the projection this document was read from — the file as written,
    /// comments and alignment included — in file order; empty unless it was read from a `.grind`.
    /// What a language server hovers over. After an unsaved edit it still describes the file on
    /// disk.
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

    // --- reading ---

    pub fn get(&self, sheet: usize, pos: Pos) -> Result<CellValue> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.get(pos))
    }

    /// Read a rectangle — the only way a shell reads cells: `get_viewport_with(…,
    /// Overlays::NONE)`.
    pub fn get_viewport(
        &self,
        sheet: usize,
        rows: Range<u32>,
        cols: Range<u32>,
    ) -> Result<Viewport> {
        self.get_viewport_with(sheet, rows, cols, view::Overlays::NONE)
    }

    /// Read a rectangle with `doc/view-modes.md`'s derived overlays. Roles and name anchors need a
    /// document-wide analysis, built once per document state and cached, so a shell that draws
    /// neither does not pay. Nothing is written (`sheet/tests/view_modes.rs` asserts it on the
    /// bytes).
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
        let mut general = Vec::with_capacity(size);
        let analysis = overlays.any().then(|| self.analysis(&state.doc));
        let mut roles = overlays.roles.then(|| Vec::with_capacity(size));
        // Conditional formats are evaluated here, against what the cells show, so no shell
        // evaluates a rule (`doc/conditional-format.md` §2).
        let mut engine =
            (!s.rules().is_empty()).then(|| formula::eval::Engine::over_cached(&state.doc));
        let default_pt = default_font_pt(&state.doc);
        let mut look = |pos: Pos| -> Option<style::CellStyle> {
            let own = s.style(pos);
            let applied = engine
                .as_mut()
                .and_then(|engine| rule::applied(&state.doc, engine, sheet, pos));
            let mut look = match applied {
                Some(rule) => Some(rule.over(own)),
                None => own.cloned(),
            };
            // A number format's own colour (a red negative's red) is drawn over the cell's
            // (`doc/ods-format.md` §5.2).
            if let Some(color) = s.format(pos).and_then(|f| f.color_of(&s.get(pos))) {
                look.get_or_insert_with(Default::default).color = Some(color.to_owned());
            }
            // A shell sets a cell naming no size in its own font, and a size relative to 10pt
            // in it (`look::font_scale`), so a document whose default is not 10pt has its sizes
            // handed out relative to its own default (`doc/ods-format.md` §5.2).
            if default_pt != look::DEFAULT_FONT_PT
                && let Some(style) = look.as_mut()
                && let Some(scale) = look::font_scale(style.font_size.as_deref())
            {
                style.font_size = Some(format!(
                    "{}pt",
                    scale * look::DEFAULT_FONT_PT * look::DEFAULT_FONT_PT / default_pt
                ));
            }
            look
        };
        for row in rows.clone() {
            for col in cols.clone() {
                let pos = Pos::new(row, col);
                let value = s.get(pos);
                texts.push(render_in(
                    s,
                    pos,
                    state.doc.null_date,
                    state.doc.locale.as_ref(),
                ));
                styles.push(look(pos));
                general.push(
                    matches!(value, CellValue::Number(_))
                        && s.format(pos).is_none()
                        && s.kind(pos).is_none(),
                );
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
        let merges = if rows.is_empty() || cols.is_empty() {
            Vec::new()
        } else {
            let (first, last) = (
                Pos::new(rows.start, cols.start),
                Pos::new(rows.end - 1, cols.end - 1),
            );
            s.merges_within(first, last)
                .into_iter()
                .map(|(anchor, span)| Merged {
                    anchor,
                    end: span.end(anchor),
                    value: s.get(anchor),
                    text: render_in(s, anchor, state.doc.null_date, state.doc.locale.as_ref()),
                    style: look(anchor),
                })
                .collect()
        };
        let checkboxes = s
            .checkboxes()
            .filter(|(pos, _)| rows.contains(&pos.row) && cols.contains(&pos.col))
            .map(|(pos, c)| (pos, checkbox_state(&state.doc, sheet, c)))
            .collect();
        let misspelt = match self.lexicon.read().unwrap().clone() {
            None => Vec::new(),
            Some(lexicon) => {
                let anchors = merges.iter().map(|m: &Merged| m.anchor);
                let cells = rows
                    .clone()
                    .flat_map(|row| cols.clone().map(move |col| Pos::new(row, col)));
                cells
                    .chain(anchors)
                    .filter_map(|pos| {
                        let ranges = misspelt_shown(&state.doc, s, pos, lexicon.as_ref());
                        (!ranges.is_empty()).then_some((pos, ranges))
                    })
                    .collect()
            }
        };
        Ok(Viewport {
            rows,
            cols,
            cells,
            texts,
            styles,
            roles,
            names,
            merges,
            checkboxes,
            general,
            locale: state.doc.locale.clone(),
            misspelt,
        })
    }

    /// The document-wide analysis, built if missing. `doc` is passed in because every caller
    /// already holds the read lock.
    fn analysis(&self, doc: &Document) -> Arc<view::Analysis> {
        if let Some(analysis) = self.analysis.read().unwrap().clone() {
            return analysis;
        }
        // Two threads may both build one; that duplicates a walk but cannot give a wrong answer,
        // and beats holding a write lock across the build.
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

    /// What an editor puts in front of a user for one cell: the text that, entered back unchanged,
    /// leaves the cell as it is — [`App::enter`]'s inverse. Text that would otherwise read as a
    /// number, a logical or a formula gets a leading `'`. A formula comes back in display form
    /// (`=SUM(B2:B4)`), which a shell converts with [`formula::display::from_display`] on the way
    /// back in. A date or time comes back in ISO, which reads back in: `date_kind` decides a cell
    /// is one by its format, else by its [`model::NumberKind`].
    pub fn input_text(&self, sheet: usize, pos: Pos) -> Result<String> {
        let state = self.state.read().unwrap();
        input_of(&state.doc, sheet, pos)
    }

    /// What the cell displays — a formula's result, formatted; what "Copy Value" copies.
    pub fn value_text(&self, sheet: usize, pos: Pos) -> Result<String> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(render_in(
            s,
            pos,
            state.doc.null_date,
            state.doc.locale.as_ref(),
        ))
    }

    /// What a cell naming no font size is set in, in points (`Document::default_font_size`).
    pub fn default_font_pt(&self) -> f64 {
        default_font_pt(&self.state.read().unwrap().doc)
    }

    /// How one cell looks, or `None` for a plain one — what a toolbar reads before
    /// [`App::set_style`], which replaces rather than merges.
    pub fn style_at(&self, sheet: usize, pos: Pos) -> Result<Option<style::CellStyle>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.style(pos).cloned())
    }

    /// How one cell's value is spelled, or `None` for the general format — what a format picker
    /// shows.
    pub fn format_at(&self, sheet: usize, pos: Pos) -> Result<Option<numfmt::Format>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.format(pos).cloned())
    }

    /// What one cell would show under `format` (`None` being general), in the document's locale,
    /// with nothing written — a format picker's live sample.
    pub fn shown_as(
        &self,
        sheet: usize,
        pos: Pos,
        format: Option<&numfmt::Format>,
    ) -> Result<String> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        let value = s.get(pos);
        let locale = state.doc.locale.as_ref();
        Ok(match format {
            Some(format) => format.render_in(&value, state.doc.null_date, locale),
            None => numfmt::general_in(&value, s.kind(pos), state.doc.null_date, locale),
        })
    }

    /// A cell's formula in display form with the document's names substituted (`doc/view-modes.md`
    /// §3.3); `None` for a plain value. For showing only: typing it back would store the names. A
    /// formula that will not parse comes back as stored.
    pub fn named_formula(&self, sheet: usize, pos: Pos) -> Result<Option<String>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        let Some(formula) = s.formula(pos) else {
            return Ok(None);
        };
        let at = formula::eval::Address::new(sheet, pos);
        // `view::Names` rather than the cached analysis: this needs only the name anchors, one
        // parse per name, where an analysis recalculates the document.
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

    /// Check the document against [`lint`]'s rules (`doc/dsl.md` §4.3). A read: nothing is stored.
    /// With a [`spell::Lexicon`] attached the `misspelt` rule runs too. A document read from a
    /// `.grind` gets each finding's line and column in that file
    /// ([`grind_core::lint::Report::locate_in`]).
    pub fn lint(&self, options: &grind_core::lint::Options) -> grind_core::lint::Report {
        let state = self.state.read().unwrap();
        let lexicon = self.lexicon.read().unwrap().clone();
        let mut report = lint::lint_with(&state.doc, options, lexicon.as_deref());
        if let Some(source) = state.doc.projection_source.as_deref() {
            report.locate_in(source);
        }
        report
    }

    /// Every calculated cell in the document, sheet by sheet and address by address — plain
    /// arithmetic included (`=[.A1]/2`), with an empty [`Calculation::functions`]. Filtering is
    /// the caller's.
    pub fn calculations(&self) -> Vec<Calculation> {
        let state = self.state.read().unwrap();
        let null_date = state.doc.null_date;
        let locale = state.doc.locale.clone();
        let locale = locale.as_ref();
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
                    // Display form, as the formula bar shows it; one that will not parse as
                    // stored.
                    formula: formula::display::to_display(formula)
                        .unwrap_or_else(|_| formula.to_owned()),
                    value: render_in(s, pos, null_date, locale),
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

    /// The document's own locale ([`Document::locale`]) — how it spells its numbers.
    pub fn locale(&self) -> Option<locale::Locale> {
        self.state.read().unwrap().doc.locale.clone()
    }

    /// Set the document's own locale, or with `None` remove it, as one undo step. Every number
    /// shown through a format without a locale of its own is spelled the new way at once, and
    /// typing is read the new way. No cell changes.
    pub fn set_locale(&self, locale: Option<locale::Locale>) -> Result<()> {
        self.mutate(|state| {
            if state.doc.locale == locale {
                return Ok(());
            }
            let inverse = state
                .doc
                .apply(Action::SetLocale { locale })
                .expect("a document-level action names no sheet");
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// A number spelled the way this document spells an unformatted one ([`numfmt::spell_number`])
    /// — for a status bar's sum.
    pub fn display_number(&self, n: f64) -> String {
        numfmt::spell_number(n, self.state.read().unwrap().doc.locale.as_ref())
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

    /// Restore stacks taken from [`App::session`]. Safe against a separately loaded document: an
    /// inverse carries the value it restores, and one naming a missing sheet fails to apply
    /// ([`App::undo`] returns `false`).
    pub fn restore_session(&self, session: Session) {
        self.mutate(|state| {
            state.undo = session.undo;
            state.redo = session.redo;
        });
    }
}

/// Whether an edit recalculates the document behind it (`doc/sheet-shell.md` C3). An enum so that
/// recalculating only the dependents can arrive as a variant.
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
#[derive(Clone, Debug, PartialEq)]
pub struct EnterOutcome {
    /// What the input became — the anchor cell's, for a range.
    pub kind: Entered,
    /// How many cells were written.
    pub cells: usize,
    /// The recalculation, when asked for. `spoiled > 0` means it was skipped, and `changed` is
    /// what it would have changed — what a banner offers.
    pub recalc: Option<Recalc>,
    /// The first number read in a way worth telling the person about — the other decimal sign, or
    /// an ambiguous thousands separator ([`reading`]). A shell shows its [`Reading::sentence`]
    /// once.
    pub reading: Option<Reading>,
    /// How many cells were read that way, of which `reading` is the first.
    pub readings: usize,
}

impl EnterOutcome {
    /// What a shell says after the entry in a document spelled the `locale` way, or `None` when
    /// every number was read the document's way.
    pub fn note(&self, locale: Option<&locale::Locale>) -> Option<String> {
        let sentence = self.reading.as_ref()?.sentence(locale);
        Some(match self.readings {
            0 | 1 => sentence,
            2 => format!("{sentence} (One more cell was read the same way.)"),
            n => format!("{sentence} ({} more cells were read the same way.)", n - 1),
        })
    }
}

/// What one cell displays: its number format applied, or the general one (§5.2). Public so
/// `grind-xlsx`'s tests ask this rather than answering themselves.
pub fn render(sheet: &Sheet, pos: Pos, null_date: i64) -> String {
    render_in(sheet, pos, null_date, None)
}

/// [`render`] in a document whose own locale is `locale` ([`Document::locale`]).
pub fn render_in(
    sheet: &Sheet,
    pos: Pos,
    null_date: i64,
    locale: Option<&locale::Locale>,
) -> String {
    let value = sheet.get(pos);
    match sheet.format(pos) {
        Some(format) => format.render_in(&value, null_date, locale),
        None => numfmt::general_in(&value, sheet.kind(pos), null_date, locale),
    }
}

/// Whether a cell counts as a date or a time: its format, if it has one, else the
/// [`model::NumberKind`] the reader left (§4.3.3). The format wins because the user can see it.
pub(crate) fn date_kind(sheet: &Sheet, pos: Pos) -> Option<model::NumberKind> {
    match sheet.format(pos).map(|f| f.kind) {
        Some(numfmt::Kind::Date) => Some(model::NumberKind::Date),
        Some(numfmt::Kind::Time) => Some(model::NumberKind::Time),
        _ => sheet.kind(pos),
    }
}

/// What kind of thing a value already stored in a cell is — [`App::fill`]'s answer for a source
/// cell without a formula, which has no typed text to classify.
fn entered_kind(value: &CellValue) -> Entered {
    match value {
        CellValue::Empty => Entered::Cleared,
        CellValue::Number(_) => Entered::Number,
        CellValue::Bool(_) => Entered::Bool,
        CellValue::Text(_) => Entered::Text,
    }
}

/// The misspelt words of one cell, as character ranges into what it displays — its own text found
/// inside a text format's surrounding words (`"Item: "@`). Empty for a cell that is not typed text
/// ([`Sheet::text_cells`]), and for one whose display does not contain its text.
fn misspelt_shown(
    doc: &Document,
    s: &Sheet,
    pos: Pos,
    lexicon: &dyn spell::Lexicon,
) -> Vec<Range<usize>> {
    let CellValue::Text(text) = s.get(pos) else {
        return Vec::new();
    };
    if s.formula(pos).is_some() {
        return Vec::new();
    }
    let unknown = spell::unknown(&text, lexicon);
    if unknown.is_empty() {
        return Vec::new();
    }
    let shown = render_in(s, pos, doc.null_date, doc.locale.as_ref());
    let Some(byte) = shown.find(text.as_str()) else {
        return Vec::new();
    };
    let shift = shown[..byte].chars().count();
    unknown
        .into_iter()
        .map(|(range, _)| range.start + shift..range.end + shift)
        .collect()
}

/// [`App::find`] under a lock the caller already holds.
fn hits(doc: &Document, search: &find::Search) -> Result<Vec<find::Hit>> {
    let sheets = match search.sheet {
        Some(sheet) => {
            doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            sheet..sheet + 1
        }
        None => 0..doc.sheets.len(),
    };
    let mut hits = Vec::new();
    if search.needle.is_empty() {
        return Ok(hits);
    }
    for sheet in sheets {
        let s = &doc.sheets[sheet];
        // The used extent, narrowed to the search's range when it has one — never widened, so
        // a range past the end of the sheet walks nothing that is not there.
        let (mut rows, mut cols) = (0..s.used_rows(), 0..s.used_cols());
        if let Some((start, end)) = search.range {
            rows = rows.start.max(start.row)..rows.end.min(end.row.saturating_add(1));
            cols = cols.start.max(start.col)..cols.end.min(end.col.saturating_add(1));
        }
        for row in rows {
            for col in cols.clone() {
                let pos = Pos::new(row, col);
                let text = input_of(doc, sheet, pos)?;
                if !search.occurrences(&text).is_empty() {
                    hits.push(find::Hit {
                        sheet,
                        sheet_name: s.name.clone(),
                        pos,
                        text,
                    });
                }
            }
        }
    }
    Ok(hits)
}

/// [`App::input_text`] under a lock the caller already holds, for [`App::find`] and
/// [`App::replace`], which ask it of every cell.
fn input_of(doc: &Document, sheet: usize, pos: Pos) -> Result<String> {
    let s = doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
    if let Some(formula) = s.formula(pos) {
        return Ok(formula::display::to_display(formula).unwrap_or_else(|_| formula.to_owned()));
    }
    Ok(match s.get(pos) {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => match date_kind(s, pos) {
            Some(kind) => numfmt::general(&CellValue::Number(n), Some(kind), doc.null_date),
            // In the document's own spelling, which the typing rule reads back as the same number.
            None => numfmt::spell_number(n, doc.locale.as_ref()),
        },
        CellValue::Bool(true) => "TRUE".to_owned(),
        CellValue::Bool(false) => "FALSE".to_owned(),
        // The typing rule itself decides whether the `'` is needed.
        CellValue::Text(text) => match typed(doc, sheet, pos, &text).0 {
            Entered::Text => text,
            _ => format!("'{text}"),
        },
    })
}

/// The typing rule: what a string a user typed means (`doc/sheet-shell.md` C3). One
/// [`Action::SetFormula`] whatever the answer, so typing `5` over `=SUM(A1:A4)` removes the formula.
/// A formula's cached value is computed against the document as it stands.
fn typed(
    doc: &Document,
    sheet: usize,
    pos: Pos,
    input: &str,
) -> (Entered, Action, Option<Reading>) {
    typed_in(doc, sheet, pos, input, doc.locale.as_ref(), false)
}

fn default_font_pt(doc: &Document) -> f64 {
    look::font_scale(doc.default_font_size.as_deref())
        .map_or(look::DEFAULT_FONT_PT, |scale| scale * look::DEFAULT_FONT_PT)
}

/// A typed number, read the way the document spells one — `reading` is its locale, or `None`
/// for the ISO spelling — and where that cannot be meant, the other way ([`reading::read`]);
/// `strict` reads only the first, for a value nobody typed.
fn typed_in(
    doc: &Document,
    sheet: usize,
    pos: Pos,
    input: &str,
    reading: Option<&locale::Locale>,
    strict: bool,
) -> (Entered, Action, Option<Reading>) {
    let cell = |kind, value| {
        (
            kind,
            Action::SetFormula {
                sheet,
                pos,
                formula: None,
                value,
            },
            None,
        )
    };
    // Without the `'` rule, `=x` and `123` could not be typed as text at all.
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
            None,
        );
    }
    // A cell known to hold a date or time (`date_kind`) accepts its own ISO spelling back —
    // `input_text`'s inverse, not "anything that looks like a date" (`doc/sheet-shell.md` C3).
    if let Some(kind) = doc.sheet(sheet).and_then(|s| date_kind(s, pos)) {
        let parsed = match kind {
            model::NumberKind::Date => formula::date::parse_date(input, doc.null_date),
            model::NumberKind::Time => formula::date::parse_time(input),
        };
        if let Some(n) = parsed {
            return cell(Entered::Number, CellValue::Number(n));
        }
    }
    if let Some((n, note)) = reading::read(input, reading, strict) {
        let (kind, action, _) = cell(Entered::Number, CellValue::Number(n));
        return (kind, action, note);
    }
    match input {
        "TRUE" | "true" => cell(Entered::Bool, CellValue::Bool(true)),
        "FALSE" | "false" => cell(Entered::Bool, CellValue::Bool(false)),
        "" => cell(Entered::Cleared, CellValue::Empty),
        _ => cell(Entered::Text, CellValue::Text(input.to_owned())),
    }
}

/// Apply an edit and, if asked, the recalculation behind it, as one undo entry: the edit first,
/// the recalculation against the result, and the inverses reversed so undo runs the recalculation
/// back first.
fn commit(
    state: &mut State,
    sheet: usize,
    edits: Vec<Action>,
    mode: RecalcMode,
) -> Result<Option<Recalc>> {
    commit_after(state, sheet, Vec::new(), edits, mode)
}

/// [`commit`] with edits already applied in front of it, for [`App::import_csv`], which must
/// format date columns before the typing rule reads the fields. `prior` is their inverses in
/// application order; everything is reversed so undo takes the values back before the formats.
fn commit_after(
    state: &mut State,
    sheet: usize,
    prior: Vec<Action>,
    edits: Vec<Action>,
    mode: RecalcMode,
) -> Result<Option<Recalc>> {
    let mut inverses = prior;
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
        RecalcMode::Document if state.defer_recalc => {
            state.recalc_owed = true;
            None
        }
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

/// Whether a checkbox is ticked: its linked cell's value (`TRUE`, or a non-zero number), else its
/// own state.
pub fn checkbox_state(doc: &Document, sheet: usize, checkbox: &Checkbox) -> bool {
    let Some(link) = &checkbox.link else {
        return checkbox.checked;
    };
    let target = match &link.sheet {
        Some(name) => doc.sheets.iter().find(|s| &s.name == name),
        None => doc.sheet(sheet),
    };
    match target.map(|s| s.get(link.pos)) {
        Some(CellValue::Bool(b)) => b,
        Some(CellValue::Number(n)) => n != 0.0,
        _ => false,
    }
}

/// What one [`App::recalc`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recalc {
    /// Cells whose stored value the recalculation replaced.
    pub changed: usize,
    /// Of those, how many held a real value and now hold an error — almost always a function this
    /// build lacks. Undoing the recalculation is one step.
    pub spoiled: usize,
}

/// Whether `name` can be sheet `except`'s name, or a new sheet's with `None`. Any characters are
/// allowed (`table:name` is a plain string, and the serialiser quotes as needed); empty is
/// refused, and so is a case-insensitive duplicate, since that is how a reference resolves (§5.8).
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

/// Whether a string is a name a formula could mention (§5.11 `Identifier`), checked when the name
/// is defined:
///
/// * it must lex as one [`formula::lex::Token::Name`], which rules out spaces, punctuation, a
///   leading digit and the empty string, and admits any Unicode letter;
/// * it must not also read as a cell address (`A1`, `$B$7`, `Q1`), as LibreOffice refuses.
///
/// A column or row alone (`SALES` lexes as a whole column) is allowed: OpenFormula brackets every
/// reference, so `SUM(SALES)` is unambiguous.
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
    // Lexing the name as `[.name]` asks a reader's question with the reader's own lexer; both axes
    // present makes it a cell.
    if let Ok(tokens) = formula::lex::lex(&format!("[.{name}]"))
        && let [formula::lex::Token::Ref(r)] = tokens.as_slice()
        && r.start.row.is_some()
        && r.start.col.is_some()
    {
        return refused("it is also a cell address, and everything reads that first");
    }
    Ok(())
}

/// Every formula cell whose stored value disagrees with a fresh evaluation, as the actions that
/// fix them, and how many would replace a good value with an error — one walk for [`App::recalc`]
/// and [`App::stale`].
fn recalculated(doc: &Document) -> (Vec<Action>, usize) {
    recalculated_with(doc, |_, _| {})
}

/// [`recalculated`], reporting `(done, total)` formulas as it walks them.
fn recalculated_with(doc: &Document, progress: impl FnMut(usize, usize)) -> (Vec<Action>, usize) {
    let differences = differences(doc, progress);
    let spoiled = differences.iter().filter(|(.., spoiled)| *spoiled).count();
    let updates = differences
        .into_iter()
        .map(|(sheet, pos, value, _)| Action::SetCell { sheet, pos, value })
        .collect();
    (updates, spoiled)
}

/// Every formula cell whose computed value differs from its cached one: where, what it computes,
/// and whether it is this build's gap (a good value replaced by an error, [`Recalc::spoiled`]).
/// Shared by recalculation and [`lint::STALE_VALUE`], so the two cannot disagree.
fn differences(
    doc: &Document,
    mut progress: impl FnMut(usize, usize),
) -> Vec<(usize, Pos, CellValue, bool)> {
    let mut out = Vec::new();
    let total: usize = doc.sheets.iter().map(|s| s.formulas().count()).sum();
    let mut done = 0;
    // The engine borrows the document immutably and memoises per cell, so this is one pass
    // whatever the dependency order.
    let mut engine = formula::eval::Engine::new(doc);
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, _) in sheet.formulas() {
            done += 1;
            if done % 2048 == 0 {
                progress(done, total);
            }
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

/// Every edit a sheet rename implies besides the rename: each cell's formula, each named
/// expression's definition, and each chart's ranges (address strings, through
/// `chart::rename_sheet_in_range`). Takes the document rather than the lock, so it is testable.
fn references_renamed(doc: &Document, from: &str, to: &str) -> Vec<Action> {
    let mut actions = Vec::new();
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, text) in sheet.formulas() {
            if let Some(formula) = formula::rename::rename_in_formula(text, from, to) {
                actions.push(Action::SetFormula {
                    sheet: index,
                    pos,
                    formula: Some(formula),
                    // The cached value stands: a renamed reference reads the same cells.
                    value: sheet.get(pos),
                });
            }
        }
    }
    // A conditional-format rule's condition is a formula, and follows the same way.
    for (index, sheet) in doc.sheets.iter().enumerate() {
        if let Some(rules) = rules_rewritten(sheet, |condition| {
            formula::rename::rename_in_formula(condition, from, to)
        }) {
            actions.push(Action::SetRules {
                sheet: index,
                rules,
            });
        }
    }
    // A checkbox linked to a cell on the renamed sheet follows it.
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, checkbox) in sheet.checkboxes() {
            if checkbox
                .link
                .as_ref()
                .is_some_and(|link| link.sheet.as_deref() == Some(from))
            {
                let mut renamed = checkbox.clone();
                if let Some(link) = renamed.link.as_mut() {
                    link.sheet = Some(to.to_owned());
                }
                actions.push(Action::SetCheckbox {
                    sheet: index,
                    pos,
                    checkbox: Some(Box::new(renamed)),
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

/// A sheet's rules with every condition `rewrite` changes changed — `None` when it changes none.
/// How a rename reaches a rule: its condition is a formula, rewritten the way a cell's is.
fn rules_rewritten(
    sheet: &Sheet,
    rewrite: impl Fn(&str) -> Option<String>,
) -> Option<Vec<rule::Rule>> {
    let mut changed = false;
    let rules = sheet
        .rules()
        .iter()
        .map(|r| {
            let mut r = r.clone();
            if let Some(condition) = rewrite(&r.condition) {
                r.condition = condition;
                changed = true;
            }
            r
        })
        .collect();
    changed.then_some(rules)
}

/// The cells [`lint::STALE_VALUE`] reports: the differences this build can settle, the spoiled
/// ones dropped.
fn stale_cells(doc: &Document) -> Vec<(usize, Pos, CellValue)> {
    differences(doc, |_, _| {})
        .into_iter()
        .filter(|(.., spoiled)| !spoiled)
        .map(|(sheet, pos, value, _)| (sheet, pos, value))
        .collect()
}

/// Whether a cell holds one of §4.6's error names, which is how an error is stored
/// ([`formula::eval::to_cell`]).
fn is_error(value: &CellValue) -> bool {
    match value {
        CellValue::Text(s) => formula::value::FormulaError::from_name(s).is_some(),
        _ => false,
    }
}

/// Undo history, serialised to outlive the process. The document is not part of it: a stateless
/// shell re-reads the file.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Session {
    /// `#[serde(default)]` on both, so an older session file still loads.
    #[serde(default)]
    pub undo: Vec<Action>,
    #[serde(default)]
    pub redo: Vec<Action>,
}

/// Read an `.ods` (package) or `.fods` (flat) document.
pub fn read_file(path: &Path) -> Result<Document> {
    read_bytes(&path.display().to_string(), &std::fs::read(path)?)
}

/// Read a document from bytes, in any form — package, flat XML or projection — sniffed from the
/// bytes; `name` is only a label.
pub fn read_bytes(_name: &str, bytes: &[u8]) -> Result<Document> {
    // The projection is recognised by its first line.
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

/// Write a document, choosing the form from the extension: `.ods` the package, anything else flat
/// ([`Form::from_path`], `doc/flat-first.md`). Reading never looks at the name; writing has to
/// choose.
pub fn write_file(doc: &Document, path: &Path) -> Result<()> {
    grind_core::atomic::write(path, &write_bytes(doc, Form::from_path(path))?)?;
    Ok(())
}

/// What `grind-spell` chooses a language through: a spreadsheet states its language in its locale
/// (`Document::locale`).
impl grind_core::spell::Spelled for App {
    fn set_lexicon(&self, lexicon: Option<Arc<dyn spell::Lexicon>>) {
        App::set_lexicon(self, lexicon);
    }

    fn spelling_language(&self) -> Option<String> {
        App::spelling_language(self)
    }

    fn stated_language(&self) -> Option<String> {
        self.state
            .read()
            .unwrap()
            .doc
            .locale
            .as_ref()
            .map(locale::Locale::tag)
    }

    fn sample(&self) -> Vec<String> {
        spell::sample(&self.state.read().unwrap().doc)
    }
}
