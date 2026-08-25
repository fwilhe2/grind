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
use grind_sheet::{App as CoreApp, Pos, RecalcMode};

use crate::app::RedrawFlag;

use super::keymap::{self, Action, Dir, Motion};

const ROW_HEADER_WIDTH: u16 = 7;
const COL_WIDTH: u16 = 10;

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

pub struct App {
    core: Arc<CoreApp>,
    redraw: Arc<RedrawFlag>,
    path: Option<PathBuf>,
    sheet: usize,
    active: Pos,
    top: Pos,
    visible_rows: u32,
    visible_cols: u32,
    mode: Mode,
    /// The other corner of a Visual-mode rectangle. The active cell is this one's opposite.
    anchor: Option<Pos>,
    /// What `y` copied, as tab-separated text — the shape every spreadsheet reads, so a range
    /// yanked here is a range this build could paste anywhere. A register rather than the
    /// system clipboard: a terminal cannot reach one without a protocol the host may not
    /// speak, and vi's register is the convention a reader of this shell already has.
    register: String,
    status: String,
    /// The key list, when it is showing. Presentation state like everything else here.
    help: crate::help::Help,
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
            visible_cols: 6,
            mode: Mode::Normal,
            anchor: None,
            register: String::new(),
            status: String::new(),
            help: crate::help::Help::default(),
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
            Action::Move(motion) => {
                let extent = self.core.used_extent(self.sheet).unwrap_or((0, 0));
                self.active = keymap::moved(self.active, motion, extent, self.visible_rows);
            }
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
            Action::Escape => {
                self.anchor = None;
                self.status.clear();
                self.mode = Mode::Normal;
            }
        }
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
    }

    fn report(&mut self, when_nothing_happened: &str, changed: bool) {
        self.status = match changed {
            true => String::new(),
            false => when_nothing_happened.to_string(),
        };
    }

    // --- Insert mode ---

    fn on_insert_key(&mut self, code: KeyCode) {
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
        // Enter walks down, matching the habit typing into a spreadsheet already has.
        self.active = keymap::moved(self.active, Motion::By(Dir::Down), (0, 0), 1);
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
            _ if cmd.starts_with("align ") => self.cmd_align(cmd[6..].trim()),
            _ if cmd.starts_with("color ") => self.cmd_color(cmd[6..].trim(), false),
            _ if cmd.starts_with("fill ") => self.cmd_color(cmd[5..].trim(), true),
            _ if cmd.starts_with("format ") => self.cmd_format(cmd[7..].trim()),
            _ if cmd.starts_with("sheet-rename ") => self.cmd_sheet_rename(cmd[13..].trim()),
            _ if cmd.starts_with("w ") => self.cmd_write(Some(cmd[2..].trim())),
            _ if cmd.starts_with("sheet ") => self.cmd_sheet(cmd[6..].trim()),
            // Anything else is a cell or range address, vi's `:{line}` counterpart.
            _ => self.cmd_jump(cmd),
        }
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
        // A rename does not rewrite the formulas that name the old sheet — they go stale,
        // which `App::stale` counts and `:recalc` turns into errors. Saying so is what keeps
        // it from being a surprise.
        match self.core.rename_sheet(self.sheet, name) {
            Ok(()) => {
                self.status = format!("renamed to {name} — formulas naming the old name go stale")
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

    fn cmd_jump(&mut self, addr: &str) {
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
    fn follow_cursor(&mut self, rows: u32, cols: u32) {
        if self.active.row < self.top.row {
            self.top.row = self.active.row;
        } else if self.active.row >= self.top.row + rows {
            self.top.row = self.active.row + 1 - rows;
        }
        if self.active.col < self.top.col {
            self.top.col = self.active.col;
        } else if self.active.col >= self.top.col + cols {
            self.top.col = self.active.col + 1 - cols;
        }
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
        let [col_header_area, grid_area, formula_area, status_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

        let visible_cols = (u32::from(grid_area.width.saturating_sub(ROW_HEADER_WIDTH))
            / u32::from(COL_WIDTH))
        .max(1);
        let visible_rows = u32::from(grid_area.height).max(1);
        self.visible_rows = visible_rows;
        self.visible_cols = visible_cols;
        self.follow_cursor(visible_rows, visible_cols);

        let viewport = self
            .core
            .get_viewport(
                self.sheet,
                self.top.row..self.top.row + visible_rows,
                self.top.col..self.top.col + visible_cols,
            )
            .ok();

        let mut header = vec![Span::raw(" ".repeat(ROW_HEADER_WIDTH as usize))];
        for c in self.top.col..self.top.col + visible_cols {
            header.push(Span::styled(
                padded(&lex::column_name(c), COL_WIDTH as usize),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        frame.render_widget(Line::from(header), col_header_area);

        // Filtered *and* manually hidden, which is what `hidden_rows` already unions — a row
        // the document says is not there is drawn as a fold rather than as a gap.
        let hidden = self.core.hidden_rows(self.sheet).unwrap_or_default();

        let mut lines = Vec::with_capacity(visible_rows as usize);
        for r in self.top.row..self.top.row + visible_rows {
            if hidden.contains(&r) {
                continue;
            }
            let mut spans = vec![Span::styled(
                format!(
                    "{:>width$} ",
                    r + 1,
                    width = (ROW_HEADER_WIDTH - 1) as usize
                ),
                Style::default().add_modifier(Modifier::DIM),
            )];
            for c in self.top.col..self.top.col + visible_cols {
                let text = viewport.as_ref().and_then(|v| v.text(r, c)).unwrap_or("");
                let cell = viewport.as_ref().and_then(|v| v.style(r, c));
                let numeric = matches!(
                    viewport.as_ref().and_then(|v| v.get(r, c)),
                    Some(grind_sheet::CellValue::Number(_))
                );
                // The document's own styling, then the shell's own marks over it: the active
                // cell and the selection are *not* the document, so they are drawn last.
                let mut style = terminal_style(cell);
                let pos = Pos::new(r, c);
                if pos == self.active || self.selected(pos) {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                spans.push(Span::styled(
                    padded_as(text, COL_WIDTH as usize, alignment(cell, numeric)),
                    style,
                ));
            }
            lines.push(Line::from(spans));
        }
        frame.render_widget(Paragraph::new(lines), grid_area);

        let sheet_name = self.core.sheet_name(self.sheet).unwrap_or_default();
        let addr = grind_sheet::a1::format(None, self.active);
        let content = match &self.mode {
            Mode::Insert { buf, .. } => buf.iter().collect::<String>(),
            _ => self
                .core
                .input_text(self.sheet, self.active)
                .unwrap_or_default(),
        };
        frame.render_widget(
            Line::from(vec![
                Span::styled(
                    format!("{sheet_name}!{addr} "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(content),
            ]),
            formula_area,
        );

        let status_text = match &self.mode {
            Mode::Command { buf } => format!(":{buf}"),
            Mode::Insert { .. } => "-- INSERT --  Enter commit, Esc cancel".to_string(),
            Mode::Visual => {
                let (start, end) = self.rect();
                format!(
                    "-- VISUAL --  {}x{}  * bold  / italic  - plain  y yank  d clear  : command",
                    end.row - start.row + 1,
                    end.col - start.col + 1
                )
            }
            _ if !self.status.is_empty() => self.status.clone(),
            _ => "h j k l move  i/a/c edit  v select  x clear  y/p yank put  u undo  : command  :q quit"
                .to_string(),
        };
        frame.render_widget(
            Paragraph::new(status_text).style(Style::default().fg(Color::Black).bg(Color::Gray)),
            status_area,
        );
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

/// A cell's own styling, as the attributes a terminal has.
///
/// Four of `CellStyle`'s nine properties land here; a font size, a border and a wrap have no
/// meaning in a grid of one font at one size and one row per row, and that is a limit of the
/// medium rather than a gap in the shell — all of them are *stored*, and every other shell
/// draws them. Alignment is [`padded`]'s, which is where the width is known.
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Align {
    Left,
    Centre,
    Right,
}

/// Pad or truncate to exactly `width` columns, one trailing space as a column separator.
fn padded(text: &str, width: usize) -> String {
    padded_as(text, width, Align::Left)
}

/// The same, with the text pushed to one side of its column — a number to the right, which is
/// what a column of figures has to do to be read as one.
///
/// ponytail: measured in `char`s rather than in terminal cells, so a column of CJK text is
/// padded one cell per character and sits a little wide. `Cells` (the word processor's own
/// metrics) is the right answer and needs the grid to be laid out in cells throughout, which
/// is the same change as honouring the document's column widths — both named in
/// `doc/tui-shell.md`.
fn padded_as(text: &str, width: usize, align: Align) -> String {
    let room = width.saturating_sub(1);
    let text: String = text.chars().take(room).collect();
    let spare = room.saturating_sub(text.chars().count());
    let (before, after) = match align {
        Align::Left => (0, spare),
        Align::Right => (spare, 0),
        Align::Centre => (spare / 2, spare - spare / 2),
    };
    let mut out = " ".repeat(before);
    out.push_str(&text);
    out.push_str(&" ".repeat(after));
    // The column separator, which every alignment keeps.
    out.push(' ');
    out
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
        assert_eq!(status_line(&mut app, 40, 6), ":bogus");
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
        // A1 is the first cell of the first row, past the row header.
        let cell = buffer[(ROW_HEADER_WIDTH, 1)].style();
        assert!(
            cell.add_modifier.contains(Modifier::BOLD),
            "A1 draws bold: {cell:?}"
        );
        let plain = buffer[(ROW_HEADER_WIDTH, 2)].style();
        assert!(!plain.add_modifier.contains(Modifier::BOLD), "{plain:?}");
    }

    #[test]
    fn a_number_sits_to_the_right_of_its_column_and_text_to_the_left() {
        assert_eq!(padded_as("12", 6, Align::Right), "   12 ");
        assert_eq!(padded_as("ab", 6, Align::Left), "ab    ");
        assert_eq!(padded_as("ab", 7, Align::Centre), "  ab   ");
        // Always exactly the column's width, whatever the alignment.
        for align in [Align::Left, Align::Centre, Align::Right] {
            assert_eq!(padded_as("overlong text", 6, align).chars().count(), 6);
        }
        assert_eq!(alignment(None, true), Align::Right, "a number by default");
        assert_eq!(alignment(None, false), Align::Left);
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
