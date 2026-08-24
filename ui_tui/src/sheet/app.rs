// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Event routing and rendering. Holds no spreadsheet state of its own — cells come from
//! [`grind_sheet::App::get_viewport`] every frame, exactly as `ui_gtk/src/grid.rs` reads it.
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
use grind_sheet::{App as CoreApp, Pos, RecalcMode};

use crate::app::RedrawFlag;

use super::keymap::{self, Action, Dir, Motion};

const ROW_HEADER_WIDTH: u16 = 7;
const COL_WIDTH: u16 = 10;

enum Mode {
    Normal,
    Insert { buf: Vec<char>, cursor: usize },
    Command { buf: String },
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
    status: String,
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
            status: String::new(),
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
        match self.mode {
            Mode::Normal => self.on_normal_key(key.code, key.modifiers),
            Mode::Insert { .. } => self.on_insert_key(key.code),
            Mode::Command { .. } => self.on_command_key(key.code),
        }
    }

    // --- Normal mode ---

    fn on_normal_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        let Some(action) = keymap::normal_action(code, mods) else {
            return;
        };
        match action {
            Action::Move(motion) => {
                let extent = self.core.used_extent(self.sheet).unwrap_or((0, 0));
                self.active = keymap::moved(self.active, motion, extent, self.visible_rows);
            }
            Action::Insert => self.begin_edit(false),
            Action::Change => self.begin_edit(true),
            Action::Clear => self.clear_active(),
            Action::Undo => self.report("nothing to undo", self.core.undo()),
            Action::Redo => self.report("nothing to redo", self.core.redo()),
            Action::Command => self.mode = Mode::Command { buf: String::new() },
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

    fn clear_active(&mut self) {
        match self
            .core
            .enter(self.sheet, self.active, "", RecalcMode::Document)
        {
            Ok(_) => self.status.clear(),
            Err(e) => self.status = e.to_string(),
        }
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

    /// Display form goes back to canonical here, exactly as `ui_gtk/src/grid.rs`'s `commit`
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

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
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

        let mut lines = Vec::with_capacity(visible_rows as usize);
        for r in self.top.row..self.top.row + visible_rows {
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
                let style = match (r, c) == (self.active.row, self.active.col) {
                    true => Style::default().add_modifier(Modifier::REVERSED),
                    false => Style::default(),
                };
                spans.push(Span::styled(padded(text, COL_WIDTH as usize), style));
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
            Mode::Normal if !self.status.is_empty() => self.status.clone(),
            Mode::Normal => {
                "h j k l move  i/a/c edit  x clear  u undo  ^r redo  : command  :q quit".to_string()
            }
        };
        frame.render_widget(
            Paragraph::new(status_text).style(Style::default().fg(Color::Black).bg(Color::Gray)),
            status_area,
        );
    }
}

/// Pad or truncate to exactly `width` columns, one trailing space as a column separator.
fn padded(text: &str, width: usize) -> String {
    let mut s: String = text.chars().take(width.saturating_sub(1)).collect();
    while s.chars().count() < width {
        s.push(' ');
    }
    s
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
}
