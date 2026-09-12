// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Event routing and rendering. Holds no spreadsheet state of its own — cells come from
//! [`grind_sheet::App::get_viewport`] every frame, exactly as `ui_sheet_gtk/src/grid.rs` reads it.
//! The only fields here are presentation concerns: the active cell, the scroll offset, the
//! editing mode and a status line.
//!
//! Three modes, vi-style: **Normal** navigates (`keymap.rs`), **Insert** (`i`/`a`/`c`) edits
//! the active cell's text, **Command** (`:`) runs a line like `:w`, `:q`, `:recalc` or a bare
//! cell address to jump to. `Esc` always returns to Normal — cancelling Insert, since a
//! spreadsheet's staged edit (unlike vi's document-resident text) has somewhere honest to go
//! back to.
//!
//! **Six bands, top to bottom**: the title bar (`crate::chrome`), the column headers, the grid,
//! the formula line, the completion band (`super::assist`, only while one is up) and the status
//! bar. Two of those are chrome and one comes and goes, which is three rows out of twenty-four —
//! paid for by what they carry that nothing else could, and argued in `doc/tui-shell.md`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use grind_sheet::formula::{display, lex};
use grind_sheet::numfmt::{self, Kind};
use grind_sheet::style::CellStyle;
use grind_sheet::{App as CoreApp, CellValue, Pos, RecalcMode};

use crate::app::RedrawFlag;
use crate::chrome;

use super::assist::Assist;
use super::geom::{self, Align, Tracks};
use super::keymap::{self, Action, Dir, Motion};

const ROW_HEADER_WIDTH: u16 = 7;

enum Mode {
    Normal,
    /// Selecting a rectangle, from an anchor the active cell is being dragged away from —
    /// the terminal's answer to dragging one out with a pointer.
    Visual,
    Insert {
        buf: Vec<char>,
        cursor: usize,
    },
    Command {
        buf: String,
    },
}

/// Where the last `:find` matched, and which of those matches the cursor is on.
///
/// Held rather than re-derived per frame, for the reason [`crate::problems`] holds its report:
/// searching walks every used cell of every sheet, and a shell that did that once per keystroke
/// would make `j` the most expensive key it has. It is a **snapshot** — an edit that changes a
/// cell does not re-run it — and `n` says so by naming the count it was taken with.
#[derive(Clone, Debug, Default)]
struct Find {
    needle: String,
    /// Every hit, in reading order across the document: which sheet, and where in it. Ordered,
    /// because `n` and `N` are defined in terms of the order.
    hits: Vec<(usize, Pos)>,
    /// The same hits, as a set. Two shapes of one list rather than one, because the two questions
    /// asked of it are different: `n` wants *the next one* and the grid asks *is this cell one of
    /// them* once per visible cell per frame, and a linear scan for the second would be the whole
    /// window times every match a common word has.
    marked: HashSet<(usize, Pos)>,
    at: usize,
}

impl Find {
    /// Build from the hits, in the order they were found.
    fn new(needle: &str, hits: Vec<(usize, Pos)>, at: usize) -> Self {
        Find {
            needle: needle.to_owned(),
            marked: hits.iter().copied().collect(),
            hits,
            at,
        }
    }

    fn is_on(&self) -> bool {
        !self.needle.is_empty() && !self.hits.is_empty()
    }

    /// Whether this cell is one of the matches — what the grid draws a mark on.
    fn matched(&self, sheet: usize, pos: Pos) -> bool {
        !self.needle.is_empty() && self.marked.contains(&(sheet, pos))
    }
}

pub struct App {
    core: Arc<CoreApp>,
    redraw: Arc<RedrawFlag>,
    path: Option<PathBuf>,
    sheet: usize,
    active: Pos,
    top: Pos,
    visible_rows: u32,
    /// How many cells across the grid has for columns, once the row header has taken its share.
    /// The window's measure, in the unit [`super::geom`] does its arithmetic in.
    grid_width: u16,
    mode: Mode,
    /// The other corner of a Visual-mode rectangle. The active cell is this one's opposite.
    anchor: Option<Pos>,
    /// What `y` copied, as tab-separated text — the shape every spreadsheet reads, so a range
    /// yanked here is a range this build could paste anywhere. A register rather than the
    /// system clipboard: a terminal cannot reach one without a protocol the host may not
    /// speak, and vi's register is the convention a reader of this shell already has.
    register: String,
    status: String,
    /// Which of `doc/view-modes.md`'s overlays this pane is drawing — presentation state
    /// like everything else here, because a view mode is a reading of the document and
    /// never a change to it.
    overlays: grind_sheet::view::Overlays,
    /// The last `:find`, and where it matched.
    find: Find,
    /// The completion band, while a formula is being typed (`super::assist`).
    assist: Assist,
    /// The key list, when it is showing. Presentation state like everything else here.
    help: crate::help::Help,
    /// The code view, when it is showing, and the projection it is showing (`doc/dsl.md` §6).
    ///
    /// Projected **once, when the pane opens**, and dropped when it closes — which is §6.3's
    /// `ponytail` in its cheapest possible form and is exact rather than approximate here: the
    /// pane is read-only and every key that is not one of its motions closes it, so the document
    /// cannot change while a projection of it is on screen.
    code: crate::code::Code,
    source: Option<grind_sheet::projection::Projection>,
    /// `grind sheet lint`'s findings, when the pane is showing them (`doc/dsl.md` §4.3, D6).
    /// A snapshot, like the projection above and for the same reason: linting costs a
    /// recalculation, so it is taken when `:lint` is typed and not once per keystroke.
    problems: crate::problems::Problems,
    /// The window's height as of the last frame — what a page key in the help pane scrolls by.
    help_height: usize,
    quit: bool,
}

impl App {
    pub fn new(core: Arc<CoreApp>, redraw: Arc<RedrawFlag>, path: Option<PathBuf>) -> Self {
        App {
            core,
            redraw,
            path,
            sheet: 0,
            active: Pos::new(0, 0),
            top: Pos::new(0, 0),
            visible_rows: 20,
            grid_width: 60,
            mode: Mode::Normal,
            anchor: None,
            register: String::new(),
            status: String::new(),
            overlays: grind_sheet::view::Overlays::NONE,
            find: Find::default(),
            assist: Assist::default(),
            help: crate::help::Help::default(),
            code: crate::code::Code::default(),
            source: None,
            problems: crate::problems::Problems::default(),
            help_height: 20,
            quit: false,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        // Windows reports press *and* release; acting on both double-handles every key.
        if key.kind != KeyEventKind::Press {
            return;
        }
        self.redraw.raise();
        if self.help.is_open() {
            let text = crate::sheet::help();
            self.help
                .on_key(key.code, text.lines().count(), self.help_height());
            return;
        }
        if self.code.is_open() {
            self.on_code_key(key.code, key.modifiers);
            return;
        }
        if self.problems.is_open() {
            self.on_problems_key(key.code);
            return;
        }
        match self.mode {
            Mode::Normal | Mode::Visual => self.on_normal_key(key.code, key.modifiers),
            Mode::Insert { .. } => self.on_insert_key(key.code),
            Mode::Command { .. } => self.on_command_key(key.code),
        }
    }

    // --- Normal mode ---

    fn on_normal_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        let visual = matches!(self.mode, Mode::Visual);
        let Some(action) = keymap::normal_action(code, mods, visual) else {
            return;
        };
        match action {
            Action::Move(motion) => self.go(motion),
            Action::Insert => self.begin_edit(false),
            Action::Change => self.begin_edit(true),
            Action::Clear => self.clear_selection(),
            Action::Undo => self.report("nothing to undo", self.core.undo()),
            Action::Redo => self.report("nothing to redo", self.core.redo()),
            Action::Command => self.mode = Mode::Command { buf: String::new() },
            Action::Visual => self.toggle_visual(),
            Action::Yank => self.yank(),
            Action::Put => self.put(),
            Action::Bold => self.toggle_style(|style| toggle(&mut style.font_weight, "bold")),
            Action::Italic => self.toggle_style(|style| toggle(&mut style.font_style, "italic")),
            Action::Plain => self.write_style(None, "plain"),
            Action::Next(forward) => self.step_match(forward),
            Action::Escape => {
                self.anchor = None;
                // Escape puts the search away as well as the selection: a grid still marked with
                // yesterday's matches is a grid lying about what is in it.
                self.find = Find::default();
                self.status.clear();
                self.mode = Mode::Normal;
            }
        }
    }

    // --- find (`:find`, then `n` / `N`) ---

    /// Every cell whose **input text** holds `needle`, across every sheet, in reading order.
    ///
    /// The input text and not the displayed value, which is the same choice `yank` makes and for
    /// the same reason: searching for `SUM` should find `=SUM(B2:B9)`, and searching for `2026`
    /// should find the date somebody typed rather than only the sheets whose format spells the
    /// year out. Case is ignored, because nobody searching a spreadsheet means otherwise.
    ///
    /// Bounded by `used_extent`, which is the rectangle the document actually occupies — the ODF
    /// sheet limit is a million rows and none of them is worth walking.
    fn cmd_find(&mut self, needle: &str) {
        if needle.is_empty() {
            self.find = Find::default();
            self.status = "find cleared".to_owned();
            return;
        }
        let wanted = needle.to_lowercase();
        let mut hits = Vec::new();
        for sheet in 0..self.core.sheet_count() {
            let (rows, cols) = self.core.used_extent(sheet).unwrap_or((0, 0));
            for row in 0..rows {
                for col in 0..cols {
                    let pos = Pos::new(row, col);
                    let text = self.core.input_text(sheet, pos).unwrap_or_default();
                    if !text.is_empty() && text.to_lowercase().contains(&wanted) {
                        hits.push((sheet, pos));
                    }
                }
            }
        }
        // Start on the first hit at or after the cursor, so `:find` from halfway down a column
        // goes forwards like every other search anybody has ever used.
        let here = (self.sheet, self.active.row, self.active.col);
        let at = hits
            .iter()
            .position(|(s, p)| (*s, p.row, p.col) >= here)
            .unwrap_or(0);
        self.find = Find::new(needle, hits, at);
        match self.find.hits.is_empty() {
            true => self.status = format!("no cell holds {needle}"),
            false => {
                self.go_to_match();
                self.status = format!(
                    "{} of {} \u{00b7} n next, N previous",
                    self.find.at + 1,
                    self.find.hits.len()
                );
            }
        }
    }

    /// `n` / `N` — the next or previous match, wrapping round the ends the way vi's do.
    fn step_match(&mut self, forward: bool) {
        if !self.find.is_on() {
            self.status = match self.find.needle.is_empty() {
                true => "nothing to step \u{2014} :find <text> first".to_owned(),
                false => format!("no cell holds {}", self.find.needle),
            };
            return;
        }
        let count = self.find.hits.len();
        self.find.at = match forward {
            true => (self.find.at + 1) % count,
            false => (self.find.at + count - 1) % count,
        };
        self.go_to_match();
        self.status = format!("{} of {count}", self.find.at + 1);
    }

    /// Put the cursor on the match the search is currently on.
    fn go_to_match(&mut self) {
        if let Some((sheet, pos)) = self.find.hits.get(self.find.at).copied() {
            self.sheet = sheet;
            self.active = pos;
            self.anchor = None;
        }
    }

    /// Every row the document does not show.
    ///
    /// **Two calls, not one.** `App::hidden_rows` is the *filter's* answer alone and
    /// `App::manually_hidden_rows` is the other half; a shell that trusted either to be both would
    /// draw rows `:hide rows` had just taken away, or fold away rows nothing had hidden. The
    /// column axis has no filter, so `App::hidden_cols` really is the whole answer there.
    fn folded_rows(&self) -> HashSet<u32> {
        let filtered = self.core.hidden_rows(self.sheet).unwrap_or_default();
        let by_hand = self
            .core
            .manually_hidden_rows(self.sheet)
            .unwrap_or_default();
        filtered.into_iter().chain(by_hand).collect()
    }

    fn folded_cols(&self) -> HashSet<u32> {
        self.core
            .hidden_cols(self.sheet)
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    /// Apply a motion, counted in the tracks the document actually shows.
    ///
    /// The hidden sets are read here, per keystroke, rather than kept: a filter can change under
    /// the cursor (`:show`, an undo), and a remembered set would move the cursor by yesterday's
    /// idea of what is on screen. This is the same "asked for fresh, never stored" rule every
    /// paint in this file follows.
    fn go(&mut self, motion: Motion) {
        let extent = self.core.used_extent(self.sheet).unwrap_or((0, 0));
        let (rows, cols) = (self.folded_rows(), self.folded_cols());
        let folded = keymap::Folded {
            rows: &rows,
            cols: &cols,
        };
        self.active = keymap::moved(self.active, motion, extent, self.visible_rows, folded);
    }

    // --- the code view (doc/dsl.md §6, D9) ---

    /// `:source` — show the document as its projection, with the cursor on the active cell's own
    /// line.
    ///
    /// A `:` command rather than a key, which is this shell's rule for a *mode* (`:roles`,
    /// `:names`): keys here are vi's motions, and a pane that came and went under one of them
    /// would be a motion that sometimes moved the cursor and sometimes did not. Turning it off is
    /// any other key, which is what the help pane already does.
    fn cmd_source(&mut self) {
        let projection = self.core.project();
        // The address the *projection* spells this cell as: sheet-qualified, because two sheets
        // have an `A1` and the span map has to tell them apart. `a1::format` is the one place
        // that spelling is made, here as everywhere else.
        let here = self
            .core
            .sheet_name(self.sheet)
            .ok()
            .map(|name| grind_sheet::a1::format(Some(&name), self.active));
        self.code.open(&projection, here.as_deref());
        self.source = Some(projection);
    }

    /// `:lint` — check the document and show what it says about itself.
    fn cmd_lint(&mut self, hints: bool) {
        let report = self.core.lint(&grind_sheet::lint::Options {
            hints,
            off: Vec::new(),
        });
        self.status = match report.is_empty() {
            true => "no problems found".to_owned(),
            false => format!("{} finding(s) — Enter goes to one", report.len()),
        };
        self.problems.open(report);
    }

    /// A key while the problems pane is open. Enter selects the cell the finding is about,
    /// through the same `locate` the code view uses — one answer to "what is this address".
    fn on_problems_key(&mut self, code: KeyCode) {
        let height = self.help_height();
        if let crate::problems::Nav::Chose(address) = self.problems.on_key(code, height) {
            match self.locate(&address) {
                Some((sheet, pos)) => {
                    self.sheet = sheet;
                    self.active = pos;
                    self.status = address;
                }
                None => self.status = format!("{address}: nothing here can show that"),
            }
        }
    }

    /// A key while the code view is open. Moving the cursor **selects the cell that line
    /// projects**, which is §6.2's map in the direction that makes the pane worth having.
    fn on_code_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        let Some(projection) = self.source.take() else {
            self.code.close();
            return;
        };
        let height = self.help_height();
        let nav = match (code, mods.contains(KeyModifiers::CONTROL)) {
            (KeyCode::Char('f'), true) => self.code.page(true, &projection, height),
            (KeyCode::Char('b'), true) => self.code.page(false, &projection, height),
            _ => self.code.on_key(code, &projection, height),
        };
        if nav == crate::code::Nav::Closed {
            self.status.clear();
            return;
        }
        if nav == crate::code::Nav::Moved
            && let Some(address) = self.code.address(&projection)
            && let Some((sheet, pos)) = self.locate(address)
        {
            self.sheet = sheet;
            self.active = pos;
            self.anchor = None;
        }
        self.source = Some(projection);
    }

    /// An address the span map handed back, resolved to a place in this document.
    ///
    /// Two spellings, and the order matters. A **sheet's own name** is checked first, because a
    /// `sheet` node anchors one and a bare name is also a perfectly good cell address — `Sheet1`
    /// parses as column `SHEET`, row 1, and a code view that answered a click on `sheet Sales {`
    /// by jumping to a cell nobody has ever used would be worse than useless. Naming a sheet
    /// means its top-left, which is where opening that sheet puts you anyway.
    ///
    /// Otherwise it is `a1::parse` and `a1::resolve`, the same pair `:{address}` already uses —
    /// the projection spells a cell the way `a1.rs` does, which is the point of `a1.rs`.
    fn locate(&self, address: &str) -> Option<(usize, Pos)> {
        if let Ok(sheet) = grind_sheet::a1::sheet(&self.core, address) {
            return Some((sheet, Pos::new(0, 0)));
        }
        let reference = grind_sheet::a1::parse(address).ok()?;
        let (sheet, start, _end) = grind_sheet::a1::resolve(&self.core, &reference).ok()?;
        Some((sheet, start))
    }

    // --- Visual mode ---

    fn toggle_visual(&mut self) {
        match self.mode {
            Mode::Visual => {
                self.anchor = None;
                self.mode = Mode::Normal;
            }
            _ => {
                self.anchor = Some(self.active);
                self.mode = Mode::Visual;
            }
        }
        self.status.clear();
    }

    /// The selected rectangle — the active cell alone when nothing is being dragged out, so
    /// every verb over a range works in both modes and `x` means the same thing in each.
    fn rect(&self) -> (Pos, Pos) {
        let other = self.anchor.unwrap_or(self.active);
        (
            Pos::new(
                self.active.row.min(other.row),
                self.active.col.min(other.col),
            ),
            Pos::new(
                self.active.row.max(other.row),
                self.active.col.max(other.col),
            ),
        )
    }

    fn selected(&self, pos: Pos) -> bool {
        let (start, end) = self.rect();
        self.anchor.is_some()
            && (start.row..=end.row).contains(&pos.row)
            && (start.col..=end.col).contains(&pos.col)
    }

    fn leave_visual(&mut self) {
        self.anchor = None;
        self.mode = Mode::Normal;
    }

    // --- the register ---

    /// The selection as tab-separated text, cell by cell in reading order.
    ///
    /// The cells' *input* text, not their displayed text: a formula yanks as a formula, which
    /// is what somebody copying `=SUM(A1:A9)` means, and it is what `put` feeds back to
    /// `App::enter_range` — so a round trip through the register is lossless.
    fn yank(&mut self) {
        let (start, end) = self.rect();
        let mut out = String::new();
        for row in start.row..=end.row {
            if row > start.row {
                out.push('\n');
            }
            for col in start.col..=end.col {
                if col > start.col {
                    out.push('\t');
                }
                let text = self
                    .core
                    .input_text(self.sheet, Pos::new(row, col))
                    .unwrap_or_default();
                // A tab or a newline inside a cell would read back as a cell boundary.
                out.push_str(&text.replace(['\t', '\n'], " "));
            }
        }
        let cells = (end.row - start.row + 1) * (end.col - start.col + 1);
        self.register = out;
        self.leave_visual();
        self.status = format!("yanked {cells} cell(s)");
    }

    /// The register back, as a rectangle from the active cell — one undo step, because
    /// `App::enter_range` is one action.
    fn put(&mut self) {
        if self.register.is_empty() {
            self.status = "nothing yanked".to_string();
            return;
        }
        let rows: Vec<Vec<String>> = self
            .register
            .split('\n')
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect();
        match self
            .core
            .enter_range(self.sheet, self.active, &rows, RecalcMode::Document)
        {
            Ok(outcome) => {
                self.leave_visual();
                self.status = format!("put {} cell(s)", outcome.cells);
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    // --- styling ---

    /// Read the active cell's style, change one field, write the whole rectangle.
    ///
    /// `App::set_style` *replaces* rather than merges, deliberately (`sheet/src/lib.rs`), so
    /// the merge policy is here — where "make this bold as well" is a sentence about the cell
    /// under the cursor rather than about every cell in the range.
    fn toggle_style(&mut self, change: impl Fn(&mut CellStyle)) {
        let mut style = self
            .core
            .style_at(self.sheet, self.active)
            .ok()
            .flatten()
            .unwrap_or_default();
        change(&mut style);
        self.write_style(Some(style), "styled");
    }

    fn write_style(&mut self, style: Option<CellStyle>, what: &str) {
        let (start, end) = self.rect();
        match self.core.set_style(self.sheet, start, end, style) {
            Ok(cells) => {
                self.leave_visual();
                self.status = format!("{what} {cells} cell(s)");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn write_format(&mut self, format: Option<numfmt::Format>, what: &str) {
        let (start, end) = self.rect();
        match self.core.set_format(self.sheet, start, end, format) {
            Ok(cells) => {
                self.leave_visual();
                self.status = format!("{what} over {cells} cell(s)");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// Empty every cell in the selection — `App::enter` with nothing in it, which is what
    /// clearing *is*, over a rectangle so it lands as one undo step.
    fn clear_selection(&mut self) {
        let (start, end) = self.rect();
        match self.core.clear_range(self.sheet, start, end) {
            Ok(cells) => {
                self.leave_visual();
                self.status = match cells {
                    0 => String::new(),
                    n => format!("cleared {n} cell(s)"),
                };
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn begin_edit(&mut self, from_empty: bool) {
        let text = match from_empty {
            true => String::new(),
            false => self
                .core
                .input_text(self.sheet, self.active)
                .unwrap_or_default(),
        };
        let buf: Vec<char> = text.chars().collect();
        let cursor = buf.len();
        self.status.clear();
        self.mode = Mode::Insert { buf, cursor };
        self.refresh_assist();
    }

    fn report(&mut self, when_nothing_happened: &str, changed: bool) {
        self.status = match changed {
            true => String::new(),
            false => when_nothing_happened.to_string(),
        };
    }

    // --- Insert mode ---

    fn on_insert_key(&mut self, code: KeyCode) {
        // A list of offers claims four keys, and only while it is up: Tab would otherwise do
        // nothing at all, Up/Down nothing, and Escape would throw the whole edit away when what
        // was meant was "not that completion". **Enter is deliberately not claimed** — a formula
        // finished by pressing Enter is the common case (`super::assist`).
        if self.assist.is_offering() {
            match code {
                KeyCode::Tab => return self.accept_offer(),
                KeyCode::Down => return self.assist.step(1),
                KeyCode::Up => return self.assist.step(-1),
                KeyCode::Esc => return self.assist.dismiss(),
                _ => {}
            }
        }
        let Mode::Insert { buf, cursor } = &mut self.mode else {
            return;
        };
        match code {
            KeyCode::Char(c) => {
                buf.insert(*cursor, c);
                *cursor += 1;
            }
            KeyCode::Backspace if *cursor > 0 => {
                *cursor -= 1;
                buf.remove(*cursor);
            }
            KeyCode::Delete if *cursor < buf.len() => {
                buf.remove(*cursor);
            }
            KeyCode::Left => *cursor = cursor.saturating_sub(1),
            KeyCode::Right => *cursor = (*cursor + 1).min(buf.len()),
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = buf.len(),
            // Esc cancels: the buffer is a staged edit, not the document itself, so there is
            // an honest "never happened" to go back to — unlike real vi.
            KeyCode::Esc => {
                self.status.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => self.commit_edit(),
            _ => {}
        }
        self.refresh_assist();
    }

    /// Ask [`Assist`] what to offer for the buffer as it now stands.
    ///
    /// Recomputed after every keystroke rather than updated in place: a remembered completion goes
    /// stale the moment the caret moves, which is the same reason nothing else in this file caches
    /// a read of the document.
    fn refresh_assist(&mut self) {
        let Mode::Insert { buf, cursor } = &self.mode else {
            self.assist.clear();
            return;
        };
        let text: String = buf.iter().collect();
        // `assist` counts bytes and the buffer counts characters — one conversion, here.
        let caret = text
            .char_indices()
            .nth(*cursor)
            .map(|(at, _)| at)
            .unwrap_or(text.len());
        let names: Vec<String> = self
            .core
            .names()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        self.assist.refresh(&text, caret, &names);
    }

    /// Tab: put the highlighted offer into the buffer, with the caret where its first argument
    /// goes.
    fn accept_offer(&mut self) {
        let Some((span, insert)) = self.assist.accept() else {
            return;
        };
        if let Mode::Insert { buf, cursor } = &mut self.mode {
            let text: String = buf.iter().collect();
            if span.end > text.len()
                || !text.is_char_boundary(span.start)
                || !text.is_char_boundary(span.end)
            {
                return;
            }
            *cursor = text[..span.start].chars().count() + insert.chars().count();
            *buf = format!("{}{}{}", &text[..span.start], insert, &text[span.end..])
                .chars()
                .collect();
        }
        self.refresh_assist();
    }

    /// Display form goes back to canonical here, exactly as `ui_sheet_gtk/src/grid.rs`'s `commit`
    /// does — the one step between what an editor holds and what `App::enter` takes. A
    /// formula that will not parse, or a value the core rejects, does **not** commit: Insert
    /// mode stays open with the typed text intact, because silently storing `=SUM(B2` as text
    /// is how a spreadsheet loses a user's work.
    fn commit_edit(&mut self) {
        let Mode::Insert { buf, .. } = &self.mode else {
            return;
        };
        let text: String = buf.iter().collect();
        let before = self
            .core
            .input_text(self.sheet, self.active)
            .unwrap_or_default();
        if before == text {
            self.status.clear();
        } else {
            let input = match text.starts_with('=') {
                true => match display::from_display(&text) {
                    Ok(canonical) => canonical,
                    Err(e) => {
                        self.status = format!("{} (at {})", e.message, e.at);
                        return;
                    }
                },
                false => text,
            };
            match self
                .core
                .enter(self.sheet, self.active, &input, RecalcMode::Document)
            {
                Ok(outcome) => {
                    self.status = match outcome.recalc.filter(|r| r.spoiled > 0) {
                        Some(r) => {
                            format!("{} cell(s) skipped recalculating — run :recalc", r.spoiled)
                        }
                        None => String::new(),
                    };
                }
                Err(e) => {
                    self.status = e.to_string();
                    return;
                }
            }
        }
        self.mode = Mode::Normal;
        // Enter walks down, matching the habit typing into a spreadsheet already has — down to
        // the next row that is *drawn*, like every other downward motion here.
        self.go(Motion::By(Dir::Down));
    }

    // --- Command mode ---

    fn on_command_key(&mut self, code: KeyCode) {
        let Mode::Command { buf } = &mut self.mode else {
            return;
        };
        match code {
            KeyCode::Char(c) => {
                buf.push(c);
                return;
            }
            KeyCode::Backspace => {
                match buf.is_empty() {
                    true => self.mode = Mode::Normal,
                    false => {
                        buf.pop();
                    }
                }
                return;
            }
            KeyCode::Esc => {
                self.status.clear();
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {}
            _ => return,
        }
        let Mode::Command { buf } = std::mem::replace(&mut self.mode, Mode::Normal) else {
            unreachable!("checked above");
        };
        self.run_command(buf.trim());
    }

    fn run_command(&mut self, cmd: &str) {
        if cmd.is_empty() {
            return;
        }
        match cmd {
            "help" | "h?" => self.help.open(),
            "about" | "version" => self.status = crate::help::about(),
            "q" => self.cmd_quit(false),
            "q!" => self.cmd_quit(true),
            "w" => self.cmd_write(None),
            "wq" | "x" => {
                self.cmd_write(None);
                if self.status.is_empty() {
                    self.quit = true;
                }
            }
            "recalc" => self.cmd_recalc(),
            // `doc/view-modes.md`'s two overlays. Verbs rather than keys because they are
            // *modes*, and this shell's keys are vi's motions; `:roles` off is `:roles`
            // again, which is the whole of turning one off — nothing was written.
            "source" => self.cmd_source(),
            // `doc/dsl.md` §4.3, D6 — the rules are the core's and this is a list in front of
            // them. `:lint hints` is `--hints`, off by default here as everywhere.
            "lint" => self.cmd_lint(false),
            "lint hints" | "lint!" => self.cmd_lint(true),
            "roles" => self.cmd_overlay(true),
            "names" => self.cmd_overlay(false),
            "bold" => self.toggle_style(|style| toggle(&mut style.font_weight, "bold")),
            "italic" => self.toggle_style(|style| toggle(&mut style.font_style, "italic")),
            "wrap" => self.toggle_style(|style| toggle(&mut style.wrap, "wrap")),
            "border" => self.toggle_style(|style| {
                style.set_border(Some(BORDER.to_owned()));
            }),
            "plain" => self.write_style(None, "plain"),
            "general" => self.write_format(None, "general"),
            "sheet-new" | "sheet-add" => self.cmd_sheet_add(),
            "sheet-delete" => self.cmd_sheet_delete(),
            // A fill in the two directions anybody means one in. `App::fill` replicates *one*
            // cell, so a selection several lines across is one call per line.
            "down" => self.cmd_fill(true),
            "right" => self.cmd_fill(false),
            "find" => self.cmd_find(""),
            "hide" => self.cmd_hide(true, false),
            "hide rows" => self.cmd_hide(true, true),
            "show" => self.cmd_hide(false, false),
            "show rows" => self.cmd_hide(false, true),
            "width" | "width auto" => self.cmd_width(None),
            "height" | "height auto" => self.cmd_height(None),
            "name!" => self.cmd_unname(),
            "format-table" => self.cmd_table(""),
            _ if cmd.starts_with("align ") => self.cmd_align(cmd[6..].trim()),
            _ if cmd.starts_with("color ") => self.cmd_color(cmd[6..].trim(), false),
            _ if cmd.starts_with("fill ") => self.cmd_color(cmd[5..].trim(), true),
            _ if cmd.starts_with("find ") => self.cmd_find(cmd[5..].trim()),
            _ if cmd.starts_with("format ") => self.cmd_format(cmd[7..].trim()),
            _ if cmd.starts_with("eval ") => self.cmd_eval(cmd[5..].trim()),
            _ if cmd.starts_with("width ") => self.cmd_width(Some(cmd[6..].trim())),
            _ if cmd.starts_with("height ") => self.cmd_height(Some(cmd[7..].trim())),
            _ if cmd.starts_with("name ") => self.cmd_name(cmd[5..].trim()),
            _ if cmd.starts_with("format-table ") => self.cmd_table(cmd[13..].trim()),
            _ if cmd.starts_with("csv-in ") => self.cmd_csv_in(cmd[7..].trim()),
            _ if cmd.starts_with("csv-out ") => self.cmd_csv_out(cmd[8..].trim()),
            _ if cmd.starts_with("sheet-rename ") => self.cmd_sheet_rename(cmd[13..].trim()),
            _ if cmd.starts_with("w ") => self.cmd_write(Some(cmd[2..].trim())),
            _ if cmd.starts_with("sheet ") => self.cmd_sheet(cmd[6..].trim()),
            // Anything else is a cell, a range or a defined name — vi's `:{line}` counterpart.
            _ => self.cmd_jump(cmd),
        }
    }

    /// `:down` / `:right` — replicate the selection's leading line across the rest of it, with
    /// every relative reference shifted (`App::fill`).
    ///
    /// ponytail: one `App::fill` call, and so one undo entry, **per line** — a fill down across
    /// five columns is five undo steps rather than one. The same ceiling `ui_sheet_gtk/src/grid.rs`
    /// records, and the same upgrade: a multi-source fill in the core. Nothing needs it until that
    /// selection shape is a common one.
    fn cmd_fill(&mut self, down: bool) {
        let (start, end) = self.rect();
        let (from, to) = match down {
            true => (start.row, end.row),
            false => (start.col, end.col),
        };
        if to <= from {
            self.status = match down {
                true => "select more than one row first".to_owned(),
                false => "select more than one column first".to_owned(),
            };
            return;
        }
        let mut cells = 0;
        let mut failed = None;
        // The source is the leading line; the targets are everything after it.
        let lines = match down {
            true => start.col..=end.col,
            false => start.row..=end.row,
        };
        for line in lines {
            let (source, first, last) = match down {
                true => (
                    Pos::new(from, line),
                    Pos::new(from + 1, line),
                    Pos::new(to, line),
                ),
                false => (
                    Pos::new(line, from),
                    Pos::new(line, from + 1),
                    Pos::new(line, to),
                ),
            };
            match self
                .core
                .fill(self.sheet, source, first, last, RecalcMode::Document)
            {
                Ok(outcome) => cells += outcome.cells,
                Err(e) => failed = Some(e.to_string()),
            }
        }
        self.leave_visual();
        self.status = match failed {
            Some(e) => e,
            None => format!("filled {cells} cell(s)"),
        };
    }

    /// `:eval <formula>` — what it would come to, storing nothing and creating no undo entry.
    ///
    /// Typed in display syntax like every other formula in this shell, and evaluated **at the
    /// active cell**, which is what its relative references are relative to — the same two
    /// decisions `grind sheet eval` makes.
    fn cmd_eval(&mut self, formula: &str) {
        let canonical = match formula.starts_with('=') {
            true => match display::from_display(formula) {
                Ok(canonical) => canonical,
                Err(e) => {
                    self.status = format!("{} (at {})", e.message, e.at);
                    return;
                }
            },
            false => format!("={formula}"),
        };
        self.status = match self.core.preview(self.sheet, self.active, &canonical) {
            Ok(value) => format!("{formula} \u{2192} {}", show_value(&value)),
            Err(e) => e.to_string(),
        };
    }

    /// `:width [n|auto]` — how many terminal cells wide the selection's columns are drawn.
    ///
    /// Written into the document as an **ODF length** (`super::geom::length`), not as a count of
    /// cells: a width set here is a width every other shell honours, because it goes in in the
    /// document's own unit rather than in this terminal's idea of one. `auto` takes the width away
    /// again, leaving the column at whatever a renderer's default is.
    fn cmd_width(&mut self, cells: Option<&str>) {
        let width = match cells {
            None => None,
            Some("auto" | "default") => None,
            Some(n) => match n.parse::<u16>() {
                Ok(n) if n > 0 => Some(geom::length(n)),
                _ => {
                    self.status = format!("not a width in cells: {n}");
                    return;
                }
            },
        };
        let (start, end) = self.rect();
        match self
            .core
            .set_col_width(self.sheet, start.col..end.col + 1, width.clone())
        {
            Ok(changed) => {
                self.leave_visual();
                self.status = match width {
                    Some(length) => format!("{changed} column(s) at {length}"),
                    None => format!("{changed} column(s) back to the default width"),
                };
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// `:height [n]` — the row twin of [`App::cmd_width`], in ODF's own unit.
    ///
    /// **Stored and not drawn**, and that is the medium rather than a gap: a row here is one line
    /// of a terminal, so there is nothing for a height to change. Every other shell draws it, the
    /// CLI reads it back, and a document that arrives with row heights keeps them.
    fn cmd_height(&mut self, cells: Option<&str>) {
        let height = match cells {
            None | Some("auto" | "default") => None,
            Some(n) => match n.parse::<u16>() {
                Ok(n) if n > 0 => Some(geom::length(n)),
                _ => {
                    self.status = format!("not a height in cells: {n}");
                    return;
                }
            },
        };
        let (start, end) = self.rect();
        match self
            .core
            .set_row_height(self.sheet, start.row..end.row + 1, height.clone())
        {
            Ok(changed) => {
                self.leave_visual();
                self.status = match height {
                    Some(length) => {
                        format!("{changed} row(s) at {length} \u{2014} stored, not drawn here")
                    }
                    None => format!("{changed} row(s) back to the default height"),
                };
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// `:hide` / `:show`, over the selection's columns — or its rows, with `rows` after the verb.
    ///
    /// Explicit rather than guessed from the shape of the selection: a rectangle covers both axes,
    /// and a verb that hid a column when you meant a row is a verb you have to undo to find out
    /// what it did.
    fn cmd_hide(&mut self, hidden: bool, rows: bool) {
        let (start, end) = self.rect();
        let done = match rows {
            true => self
                .core
                .set_row_hidden(self.sheet, start.row..end.row + 1, hidden),
            false => self
                .core
                .set_col_hidden(self.sheet, start.col..end.col + 1, hidden),
        };
        let what = match rows {
            true => "row",
            false => "column",
        };
        let how = match hidden {
            true => "hid",
            false => "showed",
        };
        match done {
            Ok(changed) => {
                self.leave_visual();
                // Hiding the track the cursor is on would leave it somewhere invisible, so it
                // steps off — the same courtesy every spreadsheet's own Hide does, and onto the
                // next one that is *shown* rather than onto whatever is one past the run, which
                // may itself have been hidden earlier.
                if hidden && !rows {
                    let folded = self.folded_cols();
                    let past = self.active.col.max(end.col + 1).min(super::MAX_COLS - 1);
                    self.active.col = keymap::shown(past, &folded, super::MAX_COLS - 1);
                }
                if hidden && rows {
                    let folded = self.folded_rows();
                    let past = self.active.row.max(end.row + 1).min(super::MAX_ROWS - 1);
                    self.active.row = keymap::shown(past, &folded, super::MAX_ROWS - 1);
                }
                self.status = format!("{how} {changed} {what}(s) \u{2014} u brings them back");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// `:name <name>` — define a name over the selection, which is what makes `:tax_rate` a place
    /// to go and `=tax_rate*subtotal` a formula somebody can read.
    ///
    /// The definition is `a1::as_definition`'s, so the expression stored is the absolute,
    /// sheet-qualified form ODF wants rather than whatever the cursor happened to be spelled as.
    fn cmd_name(&mut self, name: &str) {
        if name.is_empty() {
            self.status = "usage: :name <name>".to_owned();
            return;
        }
        let (start, end) = self.rect();
        let sheet = self.core.sheet_name(self.sheet).unwrap_or_default();
        let reference = grind_sheet::a1::reference(Some(&sheet), start, end);
        match grind_sheet::a1::as_definition(&self.core, &reference)
            .and_then(|definition| self.core.set_name(name, &definition).map(|()| definition))
        {
            Ok(definition) => {
                self.leave_visual();
                self.status = format!("{name} = {definition}");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// `:name!` — drop whichever name covers the selection exactly.
    fn cmd_unname(&mut self) {
        let (start, end) = self.rect();
        let sheet = self.core.sheet_name(self.sheet).unwrap_or_default();
        let reference = grind_sheet::a1::reference(Some(&sheet), start, end);
        let Ok(wanted) = grind_sheet::a1::as_definition(&self.core, &reference) else {
            self.status = "no name here".to_owned();
            return;
        };
        // Exactly, not overlapping: a name is a handle on one range, and dropping one because the
        // cursor happened to sit inside it would delete something nobody pointed at.
        let found = self
            .core
            .names()
            .into_iter()
            .find(|(_, expression)| *expression == wanted);
        match found {
            Some((name, _)) => {
                self.core.clear_name(&name);
                self.status = format!("dropped {name} \u{2014} u brings it back");
            }
            None => self.status = "no name covers exactly this".to_owned(),
        }
    }

    /// `:format-table [--no-header] [--totals [FUNC]] [--name NAME]` — format the selection as a
    /// table: autofilter, alternating row shading, an optional totals row and a named range, all
    /// in one undo step (`App::format_table`). `--totals` alone is a sum; `--totals average` (or
    /// any other `TotalsFunction` id) picks the aggregate, and the word offered when somebody
    /// mistypes one is `TotalsFunction::ids()` rather than a second list kept here.
    /// A single cell expands to the sheet's used extent,
    /// the same rule `:csv-out` and the GTK shell's own filter toggle use. There is no
    /// "un-table": ODF has nothing resembling a persisted table object to remove as a unit,
    /// so clearing the effects means `:name!` on the name and restyling the cells by hand,
    /// same as after LibreOffice's own AutoFormat.
    fn cmd_table(&mut self, what: &str) {
        const USAGE: &str = "usage: :format-table [--no-header] [--totals [FUNC]] [--name NAME]";
        let mut header = true;
        let mut totals = None;
        let mut name = None;
        // Peekable because `--totals` takes an *optional* argument: the next word is its
        // function only when it is not another option, so `--totals --name x` still means a
        // sum, the same reading `grind sheet format-table --totals` has.
        let mut words = what.split_whitespace().peekable();
        while let Some(word) = words.next() {
            match word {
                "--no-header" => header = false,
                "--totals" => {
                    let named = words.next_if(|w| !w.starts_with("--"));
                    totals = match named {
                        Some(w) => match w.parse::<grind_sheet::TotalsFunction>() {
                            Ok(function) => Some(function),
                            Err(say) => {
                                self.status = say;
                                return;
                            }
                        },
                        None => Some(grind_sheet::TotalsFunction::Sum),
                    };
                }
                "--name" => match words.next() {
                    Some(n) => name = Some(n.to_owned()),
                    None => {
                        self.status = USAGE.to_owned();
                        return;
                    }
                },
                _ => {
                    self.status = format!("{word}: unknown option \u{2014} {USAGE}");
                    return;
                }
            }
        }
        let (start, mut end) = self.rect();
        if start == end
            && let Ok((rows, cols)) = self.core.used_extent(self.sheet)
        {
            end = Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
        }
        if end.row <= start.row {
            self.status = "select the rows to format, including their headings".to_owned();
            return;
        }
        let options = grind_sheet::TableOptions {
            header,
            totals,
            name,
        };
        match self.core.format_table(self.sheet, start, end, options) {
            Ok(settled) => {
                self.leave_visual();
                self.status = format!("formatted as table {settled:?}");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// `:csv-in <file>` — a CSV or TSV read in at the cursor, delimiter sniffed from the file's
    /// own content (`grind_sheet::csv`).
    fn cmd_csv_in(&mut self, path: &str) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                self.status = format!("{path}: {e}");
                return;
            }
        };
        let options = grind_sheet::csv::Import::default();
        match self.core.import_csv(
            self.sheet,
            self.active,
            &text,
            &options,
            RecalcMode::Document,
        ) {
            Ok(outcome) => self.status = format!("read {} cell(s) from {path}", outcome.cells),
            Err(e) => self.status = e.to_string(),
        }
    }

    /// `:csv-out <file>` — the selection, or the whole used sheet when nothing is selected.
    fn cmd_csv_out(&mut self, path: &str) {
        let (start, end) = match self.anchor {
            Some(_) => self.rect(),
            None => {
                let (rows, cols) = self.core.used_extent(self.sheet).unwrap_or((0, 0));
                match rows == 0 || cols == 0 {
                    true => (Pos::new(0, 0), Pos::new(0, 0)),
                    false => (Pos::new(0, 0), Pos::new(rows - 1, cols - 1)),
                }
            }
        };
        let options = grind_sheet::csv::Export::default();
        match self
            .core
            .export_csv(self.sheet, start, end, &options)
            .map_err(|e| e.to_string())
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()))
        {
            Ok(()) => {
                self.leave_visual();
                self.status = format!("wrote {path}");
            }
            Err(e) => self.status = e,
        }
    }

    /// Turn one of `doc/view-modes.md`'s overlays on or off, and say which — a terminal has
    /// no toolbar to show a mode is on, so the status line does it (§9 asks for an
    /// indication that is always visible, and here that is the only place there is).
    fn cmd_overlay(&mut self, roles: bool) {
        let overlays = &mut self.overlays;
        let on = match roles {
            true => {
                overlays.roles = !overlays.roles;
                overlays.roles
            }
            false => {
                overlays.names = !overlays.names;
                overlays.names
            }
        };
        let what = match roles {
            true => "roles",
            false => "names",
        };
        self.status = match on {
            true => format!("{what} on — :{what} to turn it off"),
            false => format!("{what} off"),
        };
    }

    fn cmd_write(&mut self, path: Option<&str>) {
        let Some(target) = path.map(PathBuf::from).or_else(|| self.path.clone()) else {
            self.status = "no file name".to_string();
            return;
        };
        match self.core.save_file(&target) {
            Ok(()) => {
                self.status = format!("wrote {}", target.display());
                self.path = Some(target);
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn cmd_quit(&mut self, force: bool) {
        if !force && self.core.can_undo() {
            self.status = "unsaved changes — :q! to discard, :w to save".to_string();
            return;
        }
        self.quit = true;
    }

    fn cmd_recalc(&mut self) {
        match self.core.recalc() {
            Ok(r) if r.spoiled > 0 => {
                self.status = format!("recalculated — {} cell(s) spoiled", r.spoiled)
            }
            Ok(r) => self.status = format!("recalculated {} cell(s)", r.changed),
            Err(e) => self.status = e.to_string(),
        }
    }

    /// The alignments a cell has, in ODF's own words — `start`/`end` rather than left/right,
    /// because that is what the document stores and this build keeps values verbatim.
    fn cmd_align(&mut self, how: &str) {
        let value = match how {
            "l" | "left" | "start" => Some("start"),
            "c" | "centre" | "center" => Some("center"),
            "r" | "right" | "end" => Some("end"),
            "" | "auto" | "default" => None,
            _ => {
                self.status = format!("not an alignment: {how}");
                return;
            }
        };
        self.toggle_style(|style| style.align = value.map(str::to_owned));
    }

    /// A colour by the core's own palette name or an `#rrggbb` — the same vocabulary
    /// `sheet style --color` takes, so a word here and a swatch in a window are one attribute.
    fn cmd_color(&mut self, name: &str, background: bool) {
        let value = match name {
            "" | "none" | "default" => None,
            name => match grind_sheet::style::palette(name) {
                Some(hex) => Some(hex.to_owned()),
                None if name.starts_with('#') => Some(name.to_owned()),
                None => {
                    self.status = format!("not a colour: {name}");
                    return;
                }
            },
        };
        self.toggle_style(|style| match background {
            true => style.background = value.clone(),
            false => style.color = value.clone(),
        });
    }

    /// One of the number-format presets, in the core's own vocabulary
    /// (`grind_sheet::numfmt::preset`) rather than a format-code string — this build has no
    /// such thing, which is `doc/ods-format.md` §5.2's decision and not this shell's.
    fn cmd_format(&mut self, what: &str) {
        let mut words = what.split_whitespace();
        let kind = words.next().unwrap_or_default();
        // `:format number 3` — the decimals, where a preset takes any.
        let decimals: u8 = words.next().and_then(|n| n.parse().ok()).unwrap_or(2);
        let format = match kind {
            "general" | "" => None,
            "int" | "integer" => Some(numfmt::preset(Kind::Number, 0, true, "")),
            "number" => Some(numfmt::preset(Kind::Number, decimals, true, "")),
            "percent" => Some(numfmt::preset(Kind::Percentage, 0, false, "")),
            "currency" => Some(numfmt::preset(Kind::Currency, 2, true, CURRENCY)),
            "date" => Some(numfmt::preset(Kind::Date, 0, false, "")),
            "time" => Some(numfmt::preset(Kind::Time, 0, false, "")),
            "datetime" => Some(numfmt::datetime_preset()),
            other => {
                self.status = format!(
                    "not a format: {other} — general int number percent currency date time datetime"
                );
                return;
            }
        };
        self.write_format(format, kind);
    }

    fn cmd_sheet_add(&mut self) {
        let taken: Vec<String> = (0..self.core.sheet_count())
            .filter_map(|i| self.core.sheet_name(i).ok())
            .collect();
        let name = (1..)
            .map(|n| format!("Sheet{n}"))
            .find(|name| !taken.iter().any(|t| t.eq_ignore_ascii_case(name)))
            .expect("there is always a free number");
        match self.core.add_sheet(&name) {
            Ok(index) => {
                self.sheet = index;
                self.active = Pos::new(0, 0);
                self.top = Pos::new(0, 0);
                self.status = format!("added {name}");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn cmd_sheet_rename(&mut self, name: &str) {
        // A rename now rewrites everything that named the old sheet — formulas, named
        // expressions, chart ranges — in one undo step (`doc/dsl.md` §6.5, D10). Saying how many
        // is what tells a user the document-wide edit really happened, and that `u` undoes all
        // of it.
        match self.core.rename_sheet(self.sheet, name) {
            Ok(0) => self.status = format!("renamed to {name}"),
            Ok(rewritten) => {
                self.status =
                    format!("renamed to {name} — {rewritten} reference(s) rewritten, u undoes all")
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn cmd_sheet_delete(&mut self) {
        let name = self.core.sheet_name(self.sheet).unwrap_or_default();
        match self.core.remove_sheet(self.sheet) {
            Ok(()) => {
                self.sheet = self.sheet.saturating_sub(1);
                self.active = Pos::new(0, 0);
                self.top = Pos::new(0, 0);
                self.status = format!("deleted {name} — u brings it back");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn cmd_sheet(&mut self, target: &str) {
        let found = match target.parse::<usize>() {
            Ok(n) if n >= 1 && n <= self.core.sheet_count() => Some(n - 1),
            Ok(_) => None,
            Err(_) => grind_sheet::a1::sheet(&self.core, target).ok(),
        };
        match found {
            Some(i) => {
                self.sheet = i;
                self.active = Pos::new(0, 0);
                self.top = Pos::new(0, 0);
                self.status.clear();
            }
            None => self.status = format!("no such sheet: {target}"),
        }
    }

    /// `:{address}` — vi's `:{line}`, over the three things a place in a spreadsheet can be
    /// called.
    ///
    /// **A defined name is tried first.** `tax_rate` parses perfectly well as a cell address
    /// (column `TAX_RATE` does not exist, but `Sheet1` parses as column `SHEET`, row 1 — the same
    /// trap `locate` documents for the code view), so a name has to win or naming a range would
    /// make it unreachable by the name somebody gave it. The name box in every other shell offers
    /// them for the same reason.
    fn cmd_jump(&mut self, addr: &str) {
        if let Some((_, expression)) = self
            .core
            .names()
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(addr))
            && let Ok((sheet, start, _end)) = grind_sheet::a1::parse_bracketed(&expression)
                .and_then(|reference| grind_sheet::a1::resolve(&self.core, &reference))
        {
            self.sheet = sheet;
            self.active = start;
            self.status = format!("{addr} \u{2014} {expression}");
            return;
        }
        match grind_sheet::a1::parse(addr).and_then(|r| grind_sheet::a1::resolve(&self.core, &r)) {
            Ok((sheet, start, _end)) => {
                self.sheet = sheet;
                self.active = start;
                self.status.clear();
            }
            Err(e) => self.status = format!("not a command or address: {e}"),
        }
    }

    // --- Rendering ---

    /// Slide the scroll offset just far enough to keep the active cell on screen — the same
    /// rule `editor`'s `Editor::follow_cursor` applies to a line, one axis at a time here.
    ///
    /// Both axes are [`super::geom`]'s arithmetic now rather than a division: columns are not all
    /// the same width any more, and a hidden row is not a row, so neither question is "how many
    /// fit" times "how big is one".
    fn follow_cursor(&mut self, tracks: &Tracks, hidden: &HashSet<u32>, rows: u32, room: u16) {
        self.top.row =
            geom::follow_row(hidden, self.top.row, self.active.row, rows, super::MAX_ROWS);
        self.top.col = geom::follow(tracks, self.top.col, self.active.col, room, super::MAX_COLS);
    }

    /// What the title bar calls this document.
    fn document_name(&self) -> String {
        self.path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "untitled".to_owned())
    }

    /// Where the selection is, and what it adds up to — the right-hand end of the status bar.
    ///
    /// The aggregates go through [`grind_sheet::App::preview`] over generated formulas rather than
    /// through a summing loop of this shell's own, so what the bar says and what a cell holding
    /// `=SUM(...)` would say cannot differ. A single cell says only where it is, deliberately:
    /// it has nothing to add up, and every other spreadsheet stays quiet about it.
    ///
    /// ponytail: the third copy of a shape `ui_sheet_gtk/src/chrome.rs` and
    /// `ui_win32/src/sheet/status.rs` already have. It is small enough that three of it is
    /// cheaper than a fourth spelling in `grind-sheet`; a fourth caller is where it gets hoisted,
    /// the way `formula::assist` and `grind_core::search::score` were.
    fn selection_summary(&self) -> String {
        use grind_sheet::a1::format as spell;
        let (start, end) = self.rect();
        if start == end {
            return spell(None, start);
        }
        let address = format!("{}:{}", spell(None, start), spell(None, end));
        let Ok((rows, cols)) = self.core.used_extent(self.sheet) else {
            return address;
        };
        // Clamped to the used extent first: a selection reaching past what the sheet holds must
        // not ask the evaluator to walk a million empty rows.
        if rows == 0 || cols == 0 || start.row >= rows || start.col >= cols {
            return address;
        }
        let end = Pos::new(end.row.min(rows - 1), end.col.min(cols - 1));
        if end.row < start.row || end.col < start.col {
            return address;
        }
        let range = format!("[.{}:.{}]", spell(None, start), spell(None, end));
        // Evaluated one row past the used extent: a formula is evaluated *as if* it sat somewhere,
        // and anywhere inside the range would be a circular reference.
        let at = Pos::new(rows, 0);
        let of = |formula: String| match self.core.preview(self.sheet, at, &formula) {
            Ok(CellValue::Number(n)) => Some(n),
            _ => None,
        };
        // A status bar's Count is non-empty rather than numeric, which is `COUNTA`.
        let count = of(format!("=COUNTA({range})")).unwrap_or(0.0);
        if count == 0.0 {
            return address;
        }
        let mut parts = vec![address, format!("Count {}", show(count))];
        // Sum and Average of no numbers are not zero, they are nothing — `AVERAGE` says so with
        // `#DIV/0!`, which is why both are read back as an optional number and offered together.
        if let Some(sum) = of(format!("=SUM({range})"))
            && let Some(average) = of(format!("=AVERAGE({range})"))
        {
            parts.insert(1, format!("Sum {}", show(sum)));
            parts.push(format!("Avg {}", show(average)));
        }
        parts.join("  \u{00b7}  ")
    }

    /// How tall the help pane is, for the page keys — the whole window, which is what it
    /// takes when it is open.
    fn help_height(&self) -> usize {
        self.help_height.max(1)
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.help_height = usize::from(area.height);
        if self.help.is_open() {
            self.help.draw(frame, area, &crate::sheet::help());
            return;
        }
        if self.problems.is_open() {
            let title = self.document_name();
            self.problems.draw(frame, area, &title);
            return;
        }
        if let Some(projection) = self.source.take() {
            let title = self.document_name();
            self.code.draw(frame, area, &projection, &title);
            self.source = Some(projection);
            return;
        }
        // The completion band takes a row only while it has something to say, so a document
        // being read is never a row shorter than one being edited.
        let band = matches!(self.mode, Mode::Insert { .. }) && self.assist.is_showing();
        let [
            title_area,
            col_header_area,
            grid_area,
            formula_area,
            assist_area,
            status_area,
        ] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(u16::from(band)),
            Constraint::Length(1),
        ])
        .areas(area);

        // The document's own geometry, read fresh every frame like everything else here.
        let tracks = Tracks::new(
            self.core.col_widths(self.sheet).unwrap_or_default(),
            self.core.hidden_cols(self.sheet).unwrap_or_default(),
        );
        // Filtered *and* manually hidden — `folded_rows` unions the two, because the core does
        // not. A row the document says is not there is drawn as a fold rather than as a gap, and
        // it is the same set the motions step over, so the cursor cannot land on one.
        let hidden = self.folded_rows();

        let room = grid_area.width.saturating_sub(ROW_HEADER_WIDTH);
        let visible_rows = u32::from(grid_area.height).max(1);
        self.visible_rows = visible_rows;
        self.grid_width = room;
        self.follow_cursor(&tracks, &hidden, visible_rows, room);

        let cols = geom::columns(&tracks, self.top.col, room, super::MAX_COLS);
        let rows = geom::rows(&hidden, self.top.row, visible_rows, super::MAX_ROWS);
        let last_col = cols.last().map_or(self.top.col + 1, |(col, _)| col + 1);
        let last_row = rows.last().map_or(self.top.row + 1, |row| row + 1);

        let viewport = self
            .core
            .get_viewport_with(
                self.sheet,
                self.top.row..last_row,
                self.top.col..last_col,
                self.overlays,
            )
            .ok();

        // --- the title bar: which document, and which of its sheets ---
        let name = chrome::file_name(self.path.as_deref());
        let mut left = vec![chrome::badge("SHEET")];
        left.extend(chrome::document(&name, self.core.can_undo()));
        let sheets: Vec<String> = (0..self.core.sheet_count())
            .map(|index| self.core.sheet_name(index).unwrap_or_default())
            .collect();
        let taken: usize = left.iter().map(Span::width).sum();
        let right = chrome::tabs(
            &sheets,
            self.sheet,
            usize::from(title_area.width).saturating_sub(taken + 2),
        );
        frame.render_widget(
            chrome::bar(title_area.width, chrome::title_style(), left, right),
            title_area,
        );

        // --- the column headers, with the cursor's own column picked out ---
        let mut header = vec![Span::styled(" ".repeat(ROW_HEADER_WIDTH as usize), HEADER)];
        let mut painted = 0u16;
        for (col, width) in &cols {
            header.push(Span::styled(
                geom::pad(&lex::column_name(*col), usize::from(*width), Align::Centre),
                match *col == self.active.col {
                    true => HEADER_ACTIVE,
                    false => HEADER,
                },
            ));
            painted += width;
        }
        // A sheet narrower than the window leaves the band short; it is a band, so it is filled.
        header.push(Span::styled(
            " ".repeat(usize::from(room.saturating_sub(painted))),
            HEADER,
        ));
        frame.render_widget(Line::from(header), col_header_area);

        let mut lines = Vec::with_capacity(rows.len());
        for r in rows.iter().copied() {
            let mut spans = vec![Span::styled(
                format!(
                    "{:>width$} ",
                    r + 1,
                    width = (ROW_HEADER_WIDTH - 1) as usize
                ),
                match r == self.active.row {
                    true => HEADER_ACTIVE,
                    false => HEADER_ROW,
                },
            )];
            for (c, width) in &cols {
                let (r, c) = (r, *c);
                let text = viewport.as_ref().and_then(|v| v.text(r, c)).unwrap_or("");
                let cell = viewport.as_ref().and_then(|v| v.style(r, c));
                let numeric = matches!(
                    viewport.as_ref().and_then(|v| v.get(r, c)),
                    Some(CellValue::Number(_))
                );
                let role = viewport.as_ref().and_then(|v| v.role(r, c));
                // The document's own styling, then the shell's own marks over it: the active
                // cell and the selection are *not* the document, so they are drawn last. In
                // role mode the document's colours are suppressed altogether (§4.5): colour
                // means role, exclusively, or a cell's colour has two causes and no way to
                // tell them apart.
                let mut style = match role {
                    Some(role) => Style::default().fg(role_color(role)),
                    None => terminal_style(cell),
                };
                // The name overlay, in the one channel a cell has to spare: *underlined* means
                // this cell has a name, and the name itself is spelled out for the active cell
                // on the formula line below. Drawing `sales` inside a ten-character column
                // would be the value yielding to the hint, which §3.2 forbids in every shell.
                if viewport.as_ref().and_then(|v| v.name_at(r, c)).is_some() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                let pos = Pos::new(r, c);
                // A `:find` match, and it **replaces** the document's own colours rather than
                // adding to them — the same rule the role overlay follows, for the same reason:
                // a marked cell whose own background was already yellow would be a mark nobody
                // could tell from the document. It is a transient reading, it is drawn only
                // while a search is live, and `Esc` puts it away.
                if self.find.matched(self.sheet, pos) {
                    style = MATCH;
                }
                if pos == self.active || self.selected(pos) {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                // §4.6's second channel, and a terminal needs it more than a window does:
                // eight ANSI colours are what some of them have, and the glyph is what says
                // the role where the colour cannot. It takes a column of the cell, which is
                // the mode's price and is paid only while the mode is on — and a column
                // clipped at the window's edge has none to give, so it keeps its text.
                let marked = role.is_some() && *width >= 2;
                if let (true, Some(role)) = (marked, role) {
                    spans.push(Span::styled(role.marker().to_string(), style));
                }
                let width = match marked {
                    true => usize::from(*width) - 1,
                    false => usize::from(*width),
                };
                spans.push(Span::styled(
                    geom::pad(text, width, alignment(cell, numeric)),
                    style,
                ));
            }
            lines.push(Line::from(spans));
        }
        frame.render_widget(Paragraph::new(lines), grid_area);

        let addr = grind_sheet::a1::format(None, self.active);
        let content = match &self.mode {
            Mode::Insert { buf, .. } => buf.iter().collect::<String>(),
            _ => self
                .core
                .input_text(self.sheet, self.active)
                .unwrap_or_default(),
        };
        // What the modes have to say about the cell the cursor is on, spelled out rather
        // than coloured — §4.6's floor, and in a terminal it is also the only place a long
        // name fits. `named_formula` is §3.3: the formula read through the names it uses.
        let mut reading = String::new();
        if self.overlays.names {
            if let Some(name) = viewport
                .as_ref()
                .and_then(|v| v.name_at(self.active.row, self.active.col))
            {
                reading.push_str(&format!("  \u{2039}{name}\u{203a}"));
            }
            if let Ok(Some(formula)) = self.core.named_formula(self.sheet, self.active)
                && formula != content
            {
                reading.push_str(&format!("  {formula}"));
            }
        }
        if let Some(role) = viewport
            .as_ref()
            .and_then(|v| v.role(self.active.row, self.active.col))
        {
            reading.push_str(&format!("  [{}]", role.name()));
        }
        // The name box, then what the cell holds — an `fx` badge when that is a formula, which is
        // the one thing a reader needs to know before reading the rest of the line.
        let mut formula_line = vec![Span::styled(format!(" {addr} "), NAME_BOX)];
        if content.starts_with('=') {
            formula_line.push(Span::styled(
                " fx ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            formula_line.push(Span::raw(" "));
        }
        formula_line.push(Span::raw(content));
        formula_line.push(Span::styled(
            reading,
            Style::default().add_modifier(Modifier::DIM),
        ));
        frame.render_widget(Line::from(formula_line), formula_area);

        if band {
            frame.render_widget(Line::from(self.assist.line(false)), assist_area);
        }

        let mode = match &self.mode {
            Mode::Normal => chrome::Mode::Normal,
            Mode::Visual => chrome::Mode::Visual,
            Mode::Insert { .. } => chrome::Mode::Insert,
            Mode::Command { .. } => chrome::Mode::Command,
        };
        let says = match &self.mode {
            Mode::Command { buf } => format!(":{buf}"),
            Mode::Insert { .. } => "Enter commits, Esc cancels".to_string(),
            // Short on purpose: the right-hand end of this bar is carrying the arithmetic, which
            // is what a reader with a range selected is actually looking at. The rest of the
            // Visual-mode keys are in `:help` and in `--help`, written once (`crate::help`).
            Mode::Visual => {
                let (start, end) = self.rect();
                format!(
                    "{}\u{00d7}{}  * bold  / italic  - plain",
                    end.row - start.row + 1,
                    end.col - start.col + 1
                )
            }
            _ if !self.status.is_empty() => self.status.clone(),
            _ => "hjkl move  i edit  v select  x clear  y/p yank put  u undo  :help".to_string(),
        };
        frame.render_widget(
            Paragraph::new(chrome::bar(
                status_area.width,
                chrome::status_style(),
                vec![
                    mode.chip(),
                    Span::styled(format!(" {says}"), chrome::status_style()),
                ],
                vec![Span::styled(
                    format!("{} ", self.selection_summary()),
                    chrome::muted(),
                )],
            ))
            .style(chrome::status_style()),
            status_area,
        );
    }
}

/// The header bands, the name box and a search mark — the four places this shell paints a ground
/// of its own inside the document area.
///
/// Named colours for `crate::chrome`'s reason, and picked out rather than merely bold because a
/// grid's own crosshair is the read-out a reader uses most: which column am I in, which row.
const HEADER: Style = Style::new().bg(Color::DarkGray).fg(Color::Gray);
const HEADER_ROW: Style = Style::new().fg(Color::DarkGray);
const HEADER_ACTIVE: Style = Style::new()
    .bg(Color::Cyan)
    .fg(Color::Black)
    .add_modifier(Modifier::BOLD);
const NAME_BOX: Style = Style::new()
    .bg(Color::Gray)
    .fg(Color::Black)
    .add_modifier(Modifier::BOLD);
const MATCH: Style = Style::new().bg(Color::LightYellow).fg(Color::Black);

/// A number as a status bar says it: no trailing zeroes, and no exponent for anything a
/// spreadsheet is likely to hold.
fn show(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{n:.0}");
    }
    let text = format!("{n:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// One evaluated value, spelled for a status line.
fn show_value(value: &CellValue) -> String {
    match value {
        CellValue::Empty => "(empty)".to_owned(),
        CellValue::Number(n) => show(*n),
        CellValue::Text(text) => text.clone(),
        CellValue::Bool(true) => "TRUE".to_owned(),
        CellValue::Bool(false) => "FALSE".to_owned(),
    }
}

/// The border this shell draws when asked for one — LibreOffice's own hairline, in the
/// three-part form ODF stores (`doc/ods-format.md` §5.4), so a box drawn here is the box a
/// document already full of them has.
const BORDER: &str = "0.06pt solid #000000";

/// What `:format currency` spells. A gap, and a named one: the core carries the symbol a
/// document chose and this shell has no locale to pick one from, so it offers the one that is
/// unambiguous rather than guessing at the reader's.
const CURRENCY: &str = "\u{a4}";

/// Turn a property on, or — when it is already that value — off. What a *toggle* means, as
/// opposed to a value a picker sets.
fn toggle(field: &mut Option<String>, value: &str) {
    *field = match field.as_deref() == Some(value) {
        true => None,
        false => Some(value.to_owned()),
    };
}

/// The colour a role is drawn in — `doc/view-modes.md` §4.5's convention, in the sixteen
/// colours a terminal is guaranteed to have.
///
/// Named `ratatui` colours rather than an RGB one on purpose: an RGB escape is not what
/// every terminal reads, and a role mode that is invisible in `screen` over `ssh` is a role
/// mode half the people who would use this shell cannot use. The hues are the same
/// convention `ui_sheet_gtk` draws from `style::PALETTE` — inputs blue, formulas the default
/// foreground, another sheet green — quantised to what the medium has, which is the same
/// trade [`terminal_style`] already makes for a document's own colours.
fn role_color(role: grind_sheet::view::CellRole) -> Color {
    use grind_sheet::view::CellRole as R;
    match role {
        R::InputNamed => Color::Blue,
        R::InputUnnamed => Color::Cyan,
        R::ConstantUnnamed => Color::Yellow,
        // Not `Color::Reset`: the cell may be drawn reversed, and a role that is "whatever
        // the terminal was doing" reads as a role that is missing.
        R::ComputedLocal | R::Empty => Color::White,
        R::ComputedCrossSheet => Color::Green,
        R::Label => Color::DarkGray,
        R::Error => Color::Red,
        R::Stale => Color::Magenta,
    }
}

/// A cell's own styling, as the attributes a terminal has.
///
/// Four of `CellStyle`'s nine properties land here; a font size, a border and a wrap have no
/// meaning in a grid of one font at one size and one row per row, and that is a limit of the
/// medium rather than a gap in the shell — all of them are *stored*, and every other shell
/// draws them. Alignment is [`geom::pad`]'s, which is where the width is known.
fn terminal_style(style: Option<&CellStyle>) -> Style {
    let Some(style) = style else {
        return Style::default();
    };
    let on = |value: &Option<String>, off: &str| value.as_deref().is_some_and(|v| v != off);
    let mut out = Style::default();
    if on(&style.font_weight, "normal") {
        out = out.add_modifier(Modifier::BOLD);
    }
    if on(&style.font_style, "normal") {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if let Some(color) = style
        .color
        .as_deref()
        .and_then(crate::text::app::nearest_color)
    {
        out = out.fg(color);
    }
    if let Some(color) = style
        .background
        .as_deref()
        .and_then(crate::text::app::nearest_color)
    {
        out = out.bg(color);
    }
    out
}

/// Which way a cell's text sits in its column: what the document said, or — for a number with
/// nothing said about it — the right, which is the convention every spreadsheet has and a
/// rendering default rather than a property of the cell.
fn alignment(style: Option<&CellStyle>, numeric: bool) -> Align {
    match style.and_then(|style| style.align.as_deref()) {
        Some("center") => Align::Centre,
        Some("end" | "right") => Align::Right,
        Some(_) => Align::Left,
        None if numeric => Align::Right,
        None => Align::Left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn ctrl(app: &mut App, ch: char) {
        app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL));
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn app() -> App {
        App::new(
            Arc::new(CoreApp::new()),
            Arc::new(RedrawFlag::default()),
            None,
        )
    }

    fn status_line(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..width)
            .map(|c| buffer[(c, height - 1)].symbol().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// The whole screen as lines, for the checks that are about what is *drawn*.
    fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|r| {
                (0..width)
                    .map(|c| buffer[(c, r)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// **D9 in this shell.** `:source` shows the projection, the cursor lands on the active
    /// cell's line, moving it selects the cell that line projects, and any other key puts the
    /// grid back exactly as it was — a code view is a reading, so there is nothing to undo.
    #[test]
    fn the_code_view_shows_the_source_and_moving_in_it_selects_the_cell() {
        let mut app = app();
        for (pos, value) in [
            (Pos::new(0, 0), "North"),
            (Pos::new(0, 1), "4200"),
            (Pos::new(2, 1), "=[.B1]*2"),
        ] {
            app.core
                .enter(0, pos, value, grind_sheet::RecalcMode::Document)
                .unwrap();
        }
        app.active = Pos::new(2, 1);
        let before = screen(&mut app, 46, 10);

        app.run_command("source");
        let shown = screen(&mut app, 46, 10).join("\n");
        assert!(shown.contains("— source"), "{shown}");
        assert!(
            shown.contains("cell B3"),
            "the projection is drawn: {shown}"
        );
        assert!(
            shown.contains("Sheet1.B3"),
            "and it opened on the active cell's own line: {shown}"
        );

        // Up to the grid row, and the cell it projects becomes the selection — the half of
        // §6.2's map that a shell cannot fake.
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('k'));
        }
        assert!(
            screen(&mut app, 46, 10).join("\n").contains("Sheet1.A1"),
            "the pane says which cell this line is"
        );

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.active, Pos::new(0, 0), "and the grid went there");
        assert_eq!(
            screen(&mut app, 46, 10)[1],
            before[1],
            "closing puts the grid back"
        );
    }

    /// `doc/view-modes.md` V7 in this shell: `:roles` draws the marker channel, and turning
    /// it off leaves the screen as it was — a mode is a reading, so there is nothing to undo
    /// and nothing to write.
    #[test]
    fn the_role_mode_marks_every_cell_and_puts_the_screen_back() {
        let mut app = app();
        app.core
            .enter(0, Pos::new(0, 0), "Total", grind_sheet::RecalcMode::No)
            .unwrap();
        app.core
            .enter(0, Pos::new(0, 1), "2", grind_sheet::RecalcMode::No)
            .unwrap();
        app.core
            .enter(
                0,
                Pos::new(0, 2),
                "=[.B1]*2",
                grind_sheet::RecalcMode::Document,
            )
            .unwrap();
        let before = screen(&mut app, 40, 8);

        app.run_command("roles");
        let with = screen(&mut app, 40, 8);
        // Row 0 is the title bar and row 1 the column headers, so the grid starts at 2.
        let row = &with[2];
        // One glyph per cell, and the glyphs are the core's — a label, an input, a formula.
        assert!(row.contains('T'), "no label marker in {row:?}");
        assert!(row.contains('\u{25c7}'), "no input marker in {row:?}");
        assert!(row.contains('='), "no formula marker in {row:?}");
        // And it says what the cell under the cursor is, for a reader who cannot see colour
        // and is not reading the grid a glyph at a time.
        assert!(with[6].contains("[label]"), "no role said: {:?}", with[6]);

        app.run_command("roles");
        assert_eq!(screen(&mut app, 40, 8)[2], before[2]);
    }

    /// A named cell says so, and says *what* — the name in the formula line, since a
    /// ten-column cell has no room for one and §3.2 does not let the value yield.
    #[test]
    fn the_name_mode_spells_out_the_name_of_the_cell_under_the_cursor() {
        let mut app = app();
        app.core
            .enter(0, Pos::new(0, 0), "0.2", grind_sheet::RecalcMode::No)
            .unwrap();
        app.core.set_name("tax_rate", "[$Sheet1.$A$1]").unwrap();
        app.run_command("names");
        let screen = screen(&mut app, 40, 8);
        assert!(
            screen[6].contains("tax_rate"),
            "the name is not said: {:?}",
            screen[6]
        );
    }

    #[test]
    fn typing_into_a_cell_reaches_the_core() {
        let mut app = app();
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "42");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.core.get(0, Pos::new(0, 0)).unwrap(), 42.0.into());
        // Enter walked down after committing.
        assert_eq!(app.active, Pos::new(1, 0));
    }

    #[test]
    fn a_bad_formula_stays_in_insert_mode_with_the_text_intact() {
        let mut app = app();
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "=SUM(");
        press(&mut app, KeyCode::Enter);
        assert!(
            matches!(app.mode, Mode::Insert { .. }),
            "commit must not have succeeded"
        );
        assert_eq!(
            app.core.get(0, Pos::new(0, 0)).unwrap(),
            grind_sheet::CellValue::Empty
        );
    }

    #[test]
    fn escape_cancels_an_edit_without_touching_the_document() {
        let mut app = app();
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "hello");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(
            app.core.get(0, Pos::new(0, 0)).unwrap(),
            grind_sheet::CellValue::Empty
        );
    }

    #[test]
    fn hjkl_moves_the_active_cell() {
        let mut app = app();
        press(&mut app, KeyCode::Char('l'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.active, Pos::new(1, 1));
        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.active, Pos::new(0, 0));
    }

    #[test]
    fn x_clears_a_cell_and_u_undoes_it() {
        let mut app = app();
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "5");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('k')); // back onto A1
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(
            app.core.get(0, Pos::new(0, 0)).unwrap(),
            grind_sheet::CellValue::Empty
        );
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.core.get(0, Pos::new(0, 0)).unwrap(), 5.0.into());
    }

    #[test]
    fn command_mode_jumps_to_an_address() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "C4");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.active, Pos::new(3, 2));
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn quit_without_unsaved_changes_is_immediate() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "q");
        press(&mut app, KeyCode::Enter);
        assert!(app.should_quit());
    }

    #[test]
    fn quit_with_unsaved_changes_needs_a_bang() {
        let mut app = app();
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "x");
        press(&mut app, KeyCode::Enter);

        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "q");
        press(&mut app, KeyCode::Enter);
        assert!(!app.should_quit(), "unsaved changes must block a plain :q");

        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "q!");
        press(&mut app, KeyCode::Enter);
        assert!(app.should_quit());
    }

    #[test]
    fn ctrl_f_and_b_page_and_the_status_line_shows_the_mode() {
        let mut app = app();
        ctrl(&mut app, 'f');
        assert!(app.active.row > 0);
        ctrl(&mut app, 'b');
        assert_eq!(app.active.row, 0);

        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "bogus");
        // The chip says which mode the keyboard is in, then what is being typed into it.
        let status = status_line(&mut app, 40, 6);
        assert!(status.starts_with(" COMMAND "), "{status:?}");
        assert!(status.contains(":bogus"), "{status:?}");
    }

    /// Fill a few cells the way a reader would, so the styling tests have something to look at.
    fn filled() -> App {
        let app = app();
        for (address, text) in [
            ("A1", "Party"),
            ("B1", "Votes"),
            ("A2", "CDU"),
            ("B2", "1200"),
        ] {
            let pos = grind_sheet::a1::parse(address)
                .and_then(|r| grind_sheet::a1::resolve(&app.core, &r))
                .expect("an address")
                .1;
            app.core
                .enter(0, pos, text, RecalcMode::No)
                .expect("enters");
        }
        app
    }

    /// Visual mode is the terminal's answer to dragging a rectangle out, and every verb over a
    /// range needs it first.
    #[test]
    fn visual_mode_selects_a_rectangle_and_the_marker_keys_style_it() {
        let mut app = filled();
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('l'));
        let (start, end) = app.rect();
        assert_eq!((start, end), (Pos::new(0, 0), Pos::new(0, 1)));

        press(&mut app, KeyCode::Char('*'));
        assert!(matches!(app.mode, Mode::Normal), "the selection is spent");
        for col in 0..2 {
            let style = app
                .core
                .style_at(0, Pos::new(0, col))
                .expect("reads")
                .expect("styled");
            assert_eq!(style.font_weight.as_deref(), Some("bold"), "column {col}");
        }
        // And the cell below, outside the rectangle, is untouched.
        assert!(app.core.style_at(0, Pos::new(1, 0)).unwrap().is_none());
    }

    /// The register is tab-separated, which is the shape every other spreadsheet reads — and
    /// what `put` feeds back to `App::enter_range`, so a round trip is lossless.
    #[test]
    fn a_range_yanks_as_tab_separated_text_and_puts_back() {
        let mut app = filled();
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('l'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.register, "Party\tVotes\nCDU\t1200");

        // Put it three rows down: the rectangle lands from the active cell.
        app.active = Pos::new(4, 0);
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(
            app.core.input_text(0, Pos::new(5, 1)).unwrap(),
            "1200",
            "the bottom-right of the rectangle"
        );
    }

    /// `x` over a selection clears the selection, not one cell — which is the whole reason a
    /// range mode is worth having.
    #[test]
    fn clearing_covers_the_selection_and_undoes_in_one_step() {
        let mut app = filled();
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('l'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('x'));
        for pos in [Pos::new(0, 0), Pos::new(1, 1)] {
            assert_eq!(app.core.input_text(0, pos).unwrap(), "");
        }
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.core.input_text(0, Pos::new(0, 0)).unwrap(), "Party");
    }

    /// The command line carries the styling vocabulary the GTK shell's toolbar has.
    #[test]
    fn the_command_line_styles_and_formats() {
        let mut app = filled();
        for (command, check) in [
            ("bold", "bold"),
            ("italic", "italic"),
            ("color red", "color"),
            ("fill yellow", "fill"),
            ("align center", "align"),
        ] {
            press(&mut app, KeyCode::Char(':'));
            type_str(&mut app, command);
            press(&mut app, KeyCode::Enter);
            let style = app
                .core
                .style_at(0, Pos::new(0, 0))
                .expect("reads")
                .expect("styled");
            let set = match check {
                "bold" => style.font_weight.is_some(),
                "italic" => style.font_style.is_some(),
                "color" => style.color.as_deref() == grind_sheet::style::palette("red"),
                "fill" => style.background.as_deref() == grind_sheet::style::palette("yellow"),
                _ => style.align.as_deref() == Some("center"),
            };
            assert!(set, ":{command} — {style:?}");
        }

        // A number format is the core's own preset, not a format-code string.
        app.active = Pos::new(1, 1);
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "format percent");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.core.value_text(0, Pos::new(1, 1)).unwrap(), "120000%");

        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "general");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.core.value_text(0, Pos::new(1, 1)).unwrap(), "1200");
    }

    /// Styling is *drawn*: bold is bold, and a number sits to the right of its column.
    #[test]
    fn a_styled_cell_is_drawn_with_the_terminals_own_attributes() {
        let mut app = filled();
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "bold");
        press(&mut app, KeyCode::Enter);

        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        // A1 is the first cell of the first grid row, past the title bar, the column headers
        // and the row header.
        let cell = buffer[(ROW_HEADER_WIDTH, 2)].style();
        assert!(
            cell.add_modifier.contains(Modifier::BOLD),
            "A1 draws bold: {cell:?}"
        );
        let plain = buffer[(ROW_HEADER_WIDTH, 3)].style();
        assert!(!plain.add_modifier.contains(Modifier::BOLD), "{plain:?}");
    }

    /// Which side of its column a cell's text sits on. The padding itself is
    /// [`super::geom::pad`]'s and is tested there, in cells rather than in characters.
    #[test]
    fn a_number_sits_to_the_right_of_its_column_and_text_to_the_left() {
        assert_eq!(alignment(None, true), Align::Right, "a number by default");
        assert_eq!(alignment(None, false), Align::Left);
        let centred = CellStyle {
            align: Some("center".to_owned()),
            ..CellStyle::default()
        };
        assert_eq!(
            alignment(Some(&centred), true),
            Align::Centre,
            "the document wins"
        );
    }

    /// **The `ponytail` this shell carried since S8, gone.** A column is as wide as the document
    /// says it is, so opening somebody's spreadsheet is opening theirs rather than an
    /// approximation of it laid out ten cells at a time.
    #[test]
    fn the_document_s_own_column_widths_are_drawn() {
        let mut app = filled();
        app.core
            .set_col_width(0, 0..1, Some("2in".to_owned()))
            .expect("a width");
        let header = screen(&mut app, 60, 8).remove(1);
        // A is twenty cells wide now, so B's header sits at 7 + 20 rather than at 7 + 10.
        assert_eq!(
            header
                .char_indices()
                .find(|(_, c)| *c == 'B')
                .map(|(i, _)| i),
            Some(usize::from(ROW_HEADER_WIDTH) + 20 + 4),
            "B is centred in its own column, past a twenty-cell A: {header:?}"
        );

        app.core
            .set_col_width(0, 0..1, None)
            .expect("back to the default");
        let header = screen(&mut app, 60, 8).remove(1);
        assert_eq!(
            header
                .char_indices()
                .find(|(_, c)| *c == 'B')
                .map(|(i, _)| i),
            Some(usize::from(ROW_HEADER_WIDTH) + 10 + 4)
        );
    }

    /// A hidden column is *absent*, exactly as a filtered row already was — the other axis of
    /// the same rule.
    #[test]
    fn a_hidden_column_is_folded_away_and_show_brings_it_back() {
        let mut app = filled();
        press(&mut app, KeyCode::Char('l')); // onto B
        app.run_command("hide");
        let header = screen(&mut app, 60, 8).remove(1);
        assert!(!header.contains('B'), "B is gone: {header:?}");
        assert!(header.contains('A') && header.contains('C'), "{header:?}");
        assert_eq!(
            app.active.col, 2,
            "the cursor stepped off the hidden column"
        );

        // `:show` needs the hidden column selected, which `:B1` is how you say.
        app.run_command("B1");
        app.run_command("show");
        assert!(screen(&mut app, 60, 8).remove(1).contains('B'));
    }

    /// The title bar is this shell's only answer to "which sheets are there", which is why it
    /// costs a row.
    #[test]
    fn the_title_bar_names_the_document_and_lists_its_sheets() {
        let mut app = filled();
        app.path = Some(PathBuf::from("book.fods"));
        app.core.add_sheet("Data").expect("a second sheet");
        let title = screen(&mut app, 60, 8).remove(0);
        assert!(title.contains("SHEET"), "{title:?}");
        assert!(title.contains("book.fods"), "{title:?}");
        assert!(
            title.contains("Sheet1") && title.contains("Data"),
            "{title:?}"
        );
        assert!(
            title.contains('\u{25cf}'),
            "the fixture has unsaved changes and says so: {title:?}"
        );
    }

    /// `:find`, then vi's own two keys — the first client in the suite that can search cells at
    /// all (`doc/feature-matrix.md` §4 had a row of ○).
    #[test]
    fn find_marks_every_match_and_n_steps_through_them() {
        let mut app = filled();
        app.run_command("find 12");
        // B2 holds 1200; the cursor went to the first match at or after A1.
        assert_eq!(app.active, Pos::new(1, 1), "{}", app.status);
        assert!(app.status.starts_with("1 of 1"), "{}", app.status);

        app.core
            .enter(0, Pos::new(3, 0), "12 apples", RecalcMode::No)
            .expect("a second match");
        app.run_command("find 12");
        assert_eq!(app.find.hits.len(), 2);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.active, Pos::new(3, 0));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.active, Pos::new(1, 1), "and it wraps");
        press(&mut app, KeyCode::Char('N'));
        assert_eq!(app.active, Pos::new(3, 0), "backwards too");

        // The matches are marked in the grid, and Esc puts the marks away.
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let marked = terminal.backend().buffer()[(ROW_HEADER_WIDTH, 5)].style();
        assert_eq!(marked.bg, Some(Color::LightYellow), "{marked:?}");
        press(&mut app, KeyCode::Esc);
        assert!(
            !app.find.is_on(),
            "Esc puts the search away with the selection"
        );
    }

    /// A search finds a formula by what was *typed*, not only by what it came to.
    #[test]
    fn a_formula_is_found_by_its_own_text() {
        let mut app = app();
        app.core
            .enter(0, Pos::new(0, 0), "=SUM([.B1:.B4])", RecalcMode::Document)
            .expect("a formula");
        app.run_command("find sum");
        assert_eq!(app.find.hits.len(), 1, "{}", app.status);
        assert_eq!(app.active, Pos::new(0, 0));
    }

    /// Fill down, with the references shifting — `App::fill`, one call per line.
    #[test]
    fn filling_down_shifts_every_relative_reference() {
        let mut app = app();
        for (pos, text) in [
            (Pos::new(0, 0), "2"),
            (Pos::new(1, 0), "3"),
            (Pos::new(2, 0), "4"),
            (Pos::new(0, 1), "=[.A1]*10"),
        ] {
            app.core
                .enter(0, pos, text, RecalcMode::Document)
                .expect("enters");
        }
        // Select B1:B3 and fill down from B1.
        app.active = Pos::new(0, 1);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        app.run_command("down");
        assert_eq!(app.core.get(0, Pos::new(1, 1)).unwrap(), 30.0.into());
        assert_eq!(app.core.get(0, Pos::new(2, 1)).unwrap(), 40.0.into());
        assert!(app.status.starts_with("filled"), "{}", app.status);
    }

    /// The status bar adds the selection up, through the evaluator rather than through a
    /// summing loop of this shell's own.
    #[test]
    fn the_status_bar_says_what_the_selection_comes_to() {
        let mut app = app();
        for (row, value) in [(0u32, "10"), (1, "20"), (2, "30")] {
            app.core
                .enter(0, Pos::new(row, 0), value, RecalcMode::No)
                .expect("enters");
        }
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        let status = status_line(&mut app, 80, 8);
        assert!(status.contains("A1:A3"), "{status:?}");
        assert!(status.contains("Sum 60"), "{status:?}");
        assert!(status.contains("Count 3"), "{status:?}");
        assert!(status.contains("Avg 20"), "{status:?}");

        // A single cell says only where it is: it has nothing to add up.
        press(&mut app, KeyCode::Esc);
        let status = status_line(&mut app, 80, 8);
        assert!(status.ends_with("A3"), "{status:?}");
        assert!(!status.contains("Sum"), "{status:?}");
    }

    /// Autocomplete while typing a formula, over `grind_sheet::formula::assist` — so an offer
    /// here is a function the evaluator really has.
    #[test]
    fn typing_a_formula_offers_completions_and_tab_takes_one() {
        let mut app = app();
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "=SU");
        assert!(app.assist.is_offering(), "a half-typed name offers");
        let band = screen(&mut app, 60, 10);
        assert!(
            band.iter().any(|line| line.contains("SUM")),
            "the band is drawn: {band:?}"
        );

        let sum = app
            .assist
            .offers
            .iter()
            .position(|offer| offer.name == "SUM")
            .expect("SUM is offered");
        for _ in 0..sum {
            press(&mut app, KeyCode::Down);
        }
        press(&mut app, KeyCode::Tab);
        let Mode::Insert { buf, cursor } = &app.mode else {
            panic!("still editing");
        };
        assert_eq!(buf.iter().collect::<String>(), "=SUM(");
        assert_eq!(*cursor, 5, "the caret is where the first argument goes");

        // Enter is not the list's key: it commits, as it always does.
        type_str(&mut app, "1;2)");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.core.get(0, Pos::new(0, 0)).unwrap(), 3.0.into());
    }

    /// Escape with a list up is "not that completion", not "throw the edit away".
    #[test]
    fn escape_dismisses_the_offers_before_it_cancels_the_edit() {
        let mut app = app();
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "=SU");
        press(&mut app, KeyCode::Esc);
        assert!(
            matches!(app.mode, Mode::Insert { .. }),
            "the edit is still open"
        );
        assert!(!app.assist.is_offering());
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal), "the second one cancels");
    }

    /// A name defined here is a name every other shell reads, and `:{name}` is a place to go.
    #[test]
    fn a_name_can_be_defined_over_the_selection_and_gone_to() {
        let mut app = filled();
        app.active = Pos::new(1, 1);
        app.run_command("name votes");
        assert_eq!(
            app.core.names(),
            vec![("votes".to_owned(), "[$Sheet1.$B$2]".to_owned())],
            "{}",
            app.status
        );

        app.active = Pos::new(0, 0);
        app.run_command("votes");
        assert_eq!(app.active, Pos::new(1, 1), "{}", app.status);

        app.run_command("name!");
        assert!(app.core.names().is_empty(), "{}", app.status);
    }

    /// `:format-table` over a selection applies the filter, the banding, the totals row and
    /// the name in one command — the same composite `App::format_table` builds for every
    /// shell.
    #[test]
    fn format_table_applies_filter_banding_totals_and_a_name() {
        let mut app = filled();
        app.active = Pos::new(0, 0);
        app.anchor = Some(Pos::new(1, 1));
        app.run_command("format-table --totals");
        assert!(app.core.filter(0).unwrap().is_some(), "{}", app.status);
        assert_eq!(app.core.names().len(), 1, "{}", app.status);
        assert_eq!(
            app.core.get(0, Pos::new(2, 0)).unwrap(),
            CellValue::Text("Total".to_owned()),
            "the totals row lands right after the selection"
        );
        assert_eq!(
            app.core.get(0, Pos::new(2, 1)).unwrap(),
            CellValue::Number(1200.0),
            "and sums the one numeric column"
        );
    }

    /// `--totals` takes the aggregate's own name, and a word that is not one of them says so
    /// rather than quietly summing.
    #[test]
    fn format_table_takes_the_aggregate_by_name() {
        let mut app = filled();
        app.active = Pos::new(0, 0);
        app.anchor = Some(Pos::new(1, 1));
        app.run_command("format-table --totals max");
        assert_eq!(
            app.core.get(0, Pos::new(2, 0)).unwrap(),
            CellValue::Text("Maximum".to_owned()),
            "{}",
            app.status
        );
        assert_eq!(
            app.core.input_text(0, Pos::new(2, 1)).unwrap(),
            "=MAX(B2)",
            "{}",
            app.status
        );

        let mut app = filled();
        app.active = Pos::new(0, 0);
        app.anchor = Some(Pos::new(1, 1));
        app.run_command("format-table --totals median");
        assert!(app.status.contains("expected one of"), "{}", app.status);
        assert!(app.core.filter(0).unwrap().is_none(), "and applies nothing");
    }

    /// The track verbs: a width in cells goes into the document as an ODF length, so a column
    /// sized here is a column every other shell draws the same.
    #[test]
    fn the_width_verb_writes_an_odf_length_the_rest_of_the_suite_reads() {
        let mut app = filled();
        app.run_command("width 20");
        let widths = app.core.col_widths(0).expect("widths");
        assert_eq!(widths.len(), 1);
        assert_eq!(geom::cells(&widths[0].1), Some(20));

        app.run_command("width auto");
        assert!(app.core.col_widths(0).expect("widths").is_empty());

        app.run_command("width nonsense");
        assert!(app.status.starts_with("not a width"), "{}", app.status);
    }

    /// A row height is *stored and not drawn* — the medium, not a gap, and the shell says so.
    #[test]
    fn a_row_height_is_stored_and_the_status_line_is_honest_about_it() {
        let mut app = filled();
        app.run_command("height 3");
        assert_eq!(app.core.row_heights(0).expect("heights").len(), 1);
        assert!(app.status.contains("not drawn here"), "{}", app.status);
    }

    /// `:eval` evaluates against the document and stores nothing — no cell changes, and there
    /// is nothing to undo afterwards.
    #[test]
    fn eval_says_what_a_formula_would_come_to_and_writes_nothing() {
        let mut app = filled();
        app.run_command("eval =SUM(B2:B2)*2");
        assert!(app.status.contains("2400"), "{}", app.status);
        assert_eq!(app.core.input_text(0, Pos::new(0, 0)).unwrap(), "Party");

        app.run_command("eval =SUM(");
        assert!(
            !app.status.contains("2400"),
            "a bad formula says so instead"
        );
    }

    /// CSV out and back in again, through the command line — the one non-ODF format
    /// (`doc/not-doing.md` §2), and until now the CLI's alone.
    #[test]
    fn csv_goes_out_and_comes_back_in() {
        let dir = std::env::temp_dir().join(format!("grind-tui-csv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let file = dir.join("out.csv");
        let path = file.display().to_string();

        let mut app = filled();
        app.run_command(&format!("csv-out {path}"));
        assert!(app.status.starts_with("wrote"), "{}", app.status);
        let written = std::fs::read_to_string(&file).expect("the file");
        assert!(written.contains("Party"), "{written:?}");

        let mut back = App::new(
            Arc::new(CoreApp::new()),
            Arc::new(RedrawFlag::default()),
            None,
        );
        back.active = Pos::new(0, 0);
        back.run_command(&format!("csv-in {path}"));
        assert_eq!(back.core.input_text(0, Pos::new(1, 1)).unwrap(), "1200");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The bug this test exists for.** A filter folds rows away; `j` used to step onto them
    /// anyway, so pressing it five times moved the cursor one row on screen and left it invisible
    /// for the other four — which does not read as a cursor on a hidden row, it reads as a
    /// terminal dropping keystrokes.
    #[test]
    fn every_press_of_j_lands_on_a_row_that_is_actually_drawn() {
        let mut app = filled();
        for row in 2..7u32 {
            app.core
                .enter(0, Pos::new(row, 0), "SPD", RecalcMode::No)
                .expect("enters");
        }
        // A filter keeping only the CDU row, exactly as `examples/sample-sheet.sh` builds one.
        let mut filter =
            grind_sheet::Filter::new("__Anonymous_Sheet_DB__0", Pos::new(0, 0), Pos::new(6, 1));
        filter.contains_header = true;
        filter.keep.entry(0).or_default().insert("CDU".to_owned());
        app.core.set_filter(0, Some(filter)).expect("a filter");

        let mut seen = Vec::new();
        for _ in 0..4 {
            press(&mut app, KeyCode::Char('j'));
            seen.push(app.active.row);
            // Whatever row it landed on has to be one the frame really draws. The header is
            // right-aligned, so the prefix cannot match a different row by accident.
            let drawn = screen(&mut app, 40, 12);
            let header = format!(
                "{:>width$}",
                app.active.row + 1,
                width = usize::from(ROW_HEADER_WIDTH) - 1
            );
            assert!(
                drawn.iter().any(|line| line.starts_with(&header)),
                "row {} is not on screen: {drawn:?}",
                app.active.row + 1
            );
        }
        // One *drawn* row per press: 1, then straight over the folded run to 7.
        assert_eq!(seen, vec![1, 7, 8, 9], "a folded run is one step, not five");

        // And back up again, over the same run.
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('k'));
        }
        assert_eq!(app.active.row, 1);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.active.row, 0);
    }

    /// The other axis, which `:hide` made reachable: a folded column is stepped over too.
    #[test]
    fn a_hidden_column_is_stepped_over_rather_than_landed_on() {
        let mut app = filled();
        app.core
            .set_col_hidden(0, 1..4, true)
            .expect("three columns away");
        app.active = Pos::new(0, 0);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.active.col, 4, "over B, C and D in one step");
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.active.col, 0);
    }

    /// `App::hidden_rows` is the **filter's** answer alone, so a shell that trusted it to be both
    /// halves drew rows `:hide rows` had just taken away — and stepped onto them.
    #[test]
    fn a_row_hidden_by_hand_folds_away_like_a_filtered_one() {
        let mut app = filled();
        press(&mut app, KeyCode::Char('j')); // onto row 2
        app.run_command("hide rows");
        let drawn = screen(&mut app, 40, 10);
        assert!(
            !drawn.iter().any(|line| line.contains("CDU")),
            "row 2 is hidden and must not be drawn: {drawn:?}"
        );
        assert_eq!(app.active.row, 2, "the cursor stepped off it");

        app.run_command("A2");
        app.run_command("show rows");
        assert!(screen(&mut app, 40, 10).iter().any(|l| l.contains("CDU")));
    }

    /// A sheet can be added, renamed and deleted without leaving the shell — the three verbs
    /// the tab bar in a window has.
    #[test]
    fn the_command_line_manages_sheets() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "sheet-new");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.core.sheet_count(), 2);
        assert_eq!(app.sheet, 1);

        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "sheet-rename Data");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.core.sheet_name(1).unwrap(), "Data");

        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "sheet-delete");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.core.sheet_count(), 1);
        assert_eq!(app.sheet, 0);
    }
}
