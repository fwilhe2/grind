// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tape pane: `grind_sheet::Tape` (`doc/calc.md`) with vi keys. Every decision is the tape's;
//! this holds the cursor, the line being typed and the command line.

use std::path::PathBuf;
use std::sync::Arc;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use grind_core::DocumentKind;
use grind_sheet::Tape;

use crate::app::{RedrawFlag, Switch};

pub const HELP: &str = "\
The tape — a scratch calculator (experimental):
  a line is a formula without its =      #3 reads line 3      vat = 0.19 names a line
  j k / arrows  move     g G  first / last     i Enter  edit the line     o O  new line below / above
  dd  delete the line    u / Ctrl+r  undo / redo    Esc  stop typing (a new line goes on to the next)
  :extract <number> <name> [n]   move a number in this line into a named line before it
  :promote <file>   :promote! <file>   write the tape as a spreadsheet (:w is the same)
  :q  :q!   :new [sheet|text]   :welcome
";

enum Mode {
    Normal,
    /// Typing a line: `at` is the line being replaced, or where a new one goes.
    Insert {
        at: usize,
        new: bool,
        buf: String,
        caret: usize,
    },
    Command(String),
    Help,
}

pub struct TapePane {
    tape: Tape,
    cursor: usize,
    mode: Mode,
    /// `d` waiting for its second `d`.
    pending_d: bool,
    status: String,
    saved: bool,
    redraw: Arc<RedrawFlag>,
    switch: Option<Switch>,
    quit: bool,
}

impl TapePane {
    pub fn new(redraw: Arc<RedrawFlag>) -> Self {
        Self {
            tape: Tape::new(None),
            cursor: 0,
            mode: Mode::Insert {
                at: 0,
                new: true,
                buf: String::new(),
                caret: 0,
            },
            pending_d: false,
            status: String::new(),
            saved: true,
            redraw,
            switch: None,
            quit: false,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn take_switch(&mut self) -> Option<Switch> {
        self.switch.take()
    }

    pub fn redraw_flag(&self) -> Arc<RedrawFlag> {
        self.redraw.clone()
    }

    pub fn on_paste(&mut self, text: &str) {
        self.redraw.raise();
        match &mut self.mode {
            Mode::Insert { buf, caret, .. } => {
                let text = text.lines().next().unwrap_or_default();
                buf.insert_str(*caret, text);
                *caret += text.len();
            }
            Mode::Command(buf) => buf.push_str(text.lines().next().unwrap_or_default()),
            Mode::Normal | Mode::Help => {
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    if let Err(e) = self.tape.push(line) {
                        self.status = e.to_string();
                        break;
                    }
                    self.saved = false;
                }
                self.cursor = self.tape.len().saturating_sub(1);
            }
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        self.redraw.raise();
        match self.mode {
            Mode::Normal => self.normal(key),
            Mode::Insert { .. } => self.insert(key),
            Mode::Command(_) => self.command(key),
            Mode::Help => self.mode = Mode::Normal,
        }
    }

    fn normal(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let last = self.tape.len().saturating_sub(1);
        let d = std::mem::take(&mut self.pending_d);
        self.status.clear();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.cursor = (self.cursor + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => self.cursor = 0,
            KeyCode::Char('G') | KeyCode::End => self.cursor = last,
            KeyCode::Char('i' | 'A' | 'a') | KeyCode::Enter if !self.tape.is_empty() => {
                let buf = self.tape.lines()[self.cursor].text.clone();
                let caret = buf.len();
                self.mode = Mode::Insert {
                    at: self.cursor,
                    new: false,
                    buf,
                    caret,
                };
            }
            KeyCode::Char('o' | 'i' | 'a' | 'A') | KeyCode::Enter => self.open(self.below()),
            KeyCode::Char('O') => self.open(self.cursor),
            KeyCode::Char('d') if d => {
                if let Err(e) = self.tape.delete(self.cursor) {
                    self.status = e.to_string();
                } else {
                    self.saved = false;
                    self.cursor = self.cursor.min(self.tape.len().saturating_sub(1));
                }
            }
            KeyCode::Char('d') => self.pending_d = true,
            KeyCode::Char('u') => self.undo(true),
            KeyCode::Char('r') if ctrl => self.undo(false),
            KeyCode::Char(':') => self.mode = Mode::Command(String::new()),
            _ => {}
        }
    }

    fn below(&self) -> usize {
        if self.tape.is_empty() {
            0
        } else {
            self.cursor + 1
        }
    }

    fn open(&mut self, at: usize) {
        self.mode = Mode::Insert {
            at,
            new: true,
            buf: String::new(),
            caret: 0,
        };
    }

    fn undo(&mut self, back: bool) {
        let done = if back {
            self.tape.undo()
        } else {
            self.tape.redo()
        };
        if done {
            self.saved = false;
            self.cursor = self.cursor.min(self.tape.len().saturating_sub(1));
        } else {
            self.status = format!("nothing to {}", if back { "undo" } else { "redo" });
        }
    }

    fn insert(&mut self, key: KeyEvent) {
        let Mode::Insert {
            at,
            new,
            buf,
            caret,
        } = &mut self.mode
        else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let (at, new, text) = (*at, *new, buf.clone());
                if text.trim().is_empty() {
                    self.mode = Mode::Normal;
                    return;
                }
                let result = if new {
                    self.tape.insert(at, &text)
                } else {
                    self.tape.edit(at, &text)
                };
                match result {
                    Ok(()) => {
                        self.saved = false;
                        self.status.clear();
                        self.cursor = at;
                        // A new line goes on to the next, as on a paper tape.
                        if new {
                            self.open(at + 1);
                        } else {
                            self.mode = Mode::Normal;
                        }
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
            KeyCode::Backspace if *caret > 0 => {
                let before = buf[..*caret].chars().next_back().map_or(0, char::len_utf8);
                *caret -= before;
                buf.remove(*caret);
            }
            KeyCode::Left => {
                *caret -= buf[..*caret].chars().next_back().map_or(0, char::len_utf8);
            }
            KeyCode::Right => {
                *caret += buf[*caret..].chars().next().map_or(0, char::len_utf8);
            }
            KeyCode::Home => *caret = 0,
            KeyCode::End => *caret = buf.len(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                buf.insert(*caret, c);
                *caret += c.len_utf8();
            }
            _ => {}
        }
    }

    fn command(&mut self, key: KeyEvent) {
        let Mode::Command(buf) = &mut self.mode else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                if buf.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Char(c) => buf.push(c),
            KeyCode::Enter => {
                let line = std::mem::take(buf);
                self.mode = Mode::Normal;
                self.run(line.trim());
            }
            _ => {}
        }
    }

    fn run(&mut self, line: &str) {
        let (verb, args) = line.split_once(' ').unwrap_or((line, ""));
        let args = args.trim();
        let unsaved = !self.saved && !self.tape.is_empty();
        let refuse = |this: &mut Self, verb: &str| {
            this.status =
                format!("the tape is not saved: :promote <file>, or :{verb}! to leave it");
        };
        match verb {
            "q" | "quit" if unsaved => refuse(self, "q"),
            "q" | "quit" | "q!" => self.quit = true,
            "help" | "h" => self.mode = Mode::Help,
            "welcome" if unsaved => refuse(self, "welcome"),
            "welcome" | "welcome!" => self.switch = Some(Switch::Welcome),
            "new" | "new!" if unsaved && verb == "new" => refuse(self, "new"),
            "new" | "new!" => {
                let kind = if args.starts_with('t') {
                    DocumentKind::Text
                } else {
                    DocumentKind::Spreadsheet
                };
                self.switch = Some(Switch::New(kind));
            }
            "promote" | "promote!" | "w" | "w!" => {
                if args.is_empty() {
                    self.status = format!(":{verb} <file>");
                    return;
                }
                let path = PathBuf::from(args);
                if path.exists() && !verb.ends_with('!') {
                    self.status = format!("{args} exists; :{verb}! {args} overwrites it");
                    return;
                }
                match self.tape.save(&path) {
                    Ok(()) => {
                        self.saved = true;
                        self.status = format!("written {args}");
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
            "extract" | "x" => {
                let words: Vec<&str> = args.split_whitespace().collect();
                let (number, name, nth) = match words.as_slice() {
                    [number, name] => (*number, *name, None),
                    [number, name, k] => (*number, *name, k.parse().ok()),
                    _ => {
                        self.status = ":extract <number> <name> [n]".into();
                        return;
                    }
                };
                match self.tape.extract(self.cursor, number, nth, name) {
                    Ok(()) => {
                        self.saved = false;
                        self.cursor += 1;
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
            "" => {}
            _ => self.status = format!("no command :{verb}; :help lists them"),
        }
    }

    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let [body, foot] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
        if let Mode::Help = self.mode {
            frame.render_widget(
                Paragraph::new(HELP).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" tape — any key "),
                ),
                body,
            );
            return;
        }
        let lines = self.tape.lines();
        let text_width = lines
            .iter()
            .map(|l| l.text.chars().count())
            .max()
            .unwrap_or(0);
        let mut shown: Vec<Line> = Vec::new();
        let typing = match &self.mode {
            Mode::Insert { at, new, buf, .. } => Some((*at, *new, buf.clone())),
            _ => None,
        };
        let typed_line = |handle: usize, buf: &str| {
            Line::from(vec![
                Span::styled(
                    format!("#{handle:<4}"),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    buf.to_owned(),
                    Style::default().add_modifier(Modifier::UNDERLINED),
                ),
            ])
        };
        for (i, line) in lines.iter().enumerate() {
            if let Some((at, true, buf)) = &typing
                && *at == i
            {
                shown.push(typed_line(i + 1, buf));
            }
            if let Some((at, false, buf)) = &typing
                && *at == i
            {
                shown.push(typed_line(line.handle, buf));
                continue;
            }
            let value_style = if line.error {
                Style::default().fg(Color::Red)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            let mut row = Line::from(vec![
                Span::styled(
                    format!("#{:<4}", line.handle),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(format!("{:<text_width$}   ", line.text)),
                Span::styled(line.value.clone(), value_style),
            ]);
            if typing.is_none() && i == self.cursor {
                row = row.style(Style::default().bg(Color::Indexed(236)));
            }
            shown.push(row);
        }
        if let Some((at, true, buf)) = &typing
            && *at >= lines.len()
        {
            shown.push(typed_line(at + 1, buf));
        }
        let scroll = u16::try_from(
            self.cursor
                .saturating_sub(usize::from(body.height.saturating_sub(3))),
        )
        .unwrap_or(0);
        frame.render_widget(
            Paragraph::new(shown).scroll((scroll, 0)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" tape — :help "),
            ),
            body,
        );
        let foot_text = match &self.mode {
            Mode::Command(buf) => format!(":{buf}"),
            _ if !self.status.is_empty() => self.status.clone(),
            _ if let Some(circles) = self.tape.circles() => circles,
            Mode::Insert { .. } => "-- INSERT --  Enter commits, Esc stops".into(),
            _ => String::new(),
        };
        frame.render_widget(Paragraph::new(foot_text), foot);
        // The caret, where text is typed.
        if let Mode::Insert { at, buf, caret, .. } = &self.mode {
            let row = (*at).min(lines.len());
            let x = body.x + 1 + 5 + u16::try_from(buf[..*caret].chars().count()).unwrap_or(0);
            let y = body.y + 1 + u16::try_from(row).unwrap_or(0).saturating_sub(scroll);
            frame.set_cursor_position((x, y));
        } else if let Mode::Command(buf) = &self.mode {
            let x = foot.x + 1 + u16::try_from(buf.chars().count()).unwrap_or(0);
            frame.set_cursor_position((x, foot.y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(p: &mut TapePane, code: KeyCode) {
        p.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn typed(p: &mut TapePane, text: &str) {
        for c in text.chars() {
            press(p, KeyCode::Char(c));
        }
        press(p, KeyCode::Enter);
    }

    fn pane() -> TapePane {
        TapePane::new(Arc::new(RedrawFlag::default()))
    }

    fn values(p: &TapePane) -> Vec<String> {
        p.tape.lines().into_iter().map(|l| l.value).collect()
    }

    #[test]
    fn lines_typed_one_after_another_compute_as_they_go() {
        let mut p = pane();
        typed(&mut p, "vat = 0.19");
        typed(&mut p, "1200 * (1 + vat)");
        press(&mut p, KeyCode::Esc);
        assert_eq!(values(&p), ["0.19", "1428"]);
        assert_eq!(p.cursor, 1);
    }

    #[test]
    fn a_line_that_does_not_read_stays_open_with_the_reason() {
        let mut p = pane();
        typed(&mut p, "1 +");
        assert!(matches!(p.mode, Mode::Insert { .. }));
        assert_ne!(p.status, "");
        assert!(p.tape.is_empty());
    }

    #[test]
    fn dd_deletes_and_u_brings_it_back() {
        let mut p = pane();
        typed(&mut p, "2");
        typed(&mut p, "#1 * 3");
        press(&mut p, KeyCode::Esc);
        press(&mut p, KeyCode::Char('g'));
        press(&mut p, KeyCode::Char('d'));
        press(&mut p, KeyCode::Char('d'));
        assert_eq!(values(&p), ["#REF!"]);
        press(&mut p, KeyCode::Char('u'));
        assert_eq!(values(&p), ["2", "6"]);
    }

    #[test]
    fn i_edits_the_line_under_the_cursor() {
        let mut p = pane();
        typed(&mut p, "2");
        typed(&mut p, "#1 * 3");
        press(&mut p, KeyCode::Esc);
        press(&mut p, KeyCode::Char('g'));
        press(&mut p, KeyCode::Char('i'));
        press(&mut p, KeyCode::Backspace);
        typed(&mut p, "5");
        assert_eq!(values(&p), ["5", "15"]);
        assert!(matches!(p.mode, Mode::Normal));
    }

    #[test]
    fn extract_and_promote_from_the_command_line() {
        let mut p = pane();
        typed(&mut p, "100 * 1.19");
        press(&mut p, KeyCode::Esc);
        press(&mut p, KeyCode::Char(':'));
        typed(&mut p, "extract 1.19 rate");
        assert_eq!(p.tape.lines()[0].text, "rate = 1.19");
        assert_eq!(p.cursor, 1);

        press(&mut p, KeyCode::Char(':'));
        typed(&mut p, "q");
        assert!(!p.should_quit(), "unsaved work is not left behind");

        let dir = std::env::temp_dir().join(format!("tui-tape-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tape.fods");
        press(&mut p, KeyCode::Char(':'));
        typed(&mut p, &format!("promote {}", file.display()));
        assert!(file.exists(), "{}", p.status);
        press(&mut p, KeyCode::Char(':'));
        typed(&mut p, "q");
        assert!(p.should_quit());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn it_draws_the_lines_their_values_and_a_cycle() {
        let mut p = pane();
        typed(&mut p, "#2");
        typed(&mut p, "#1");
        press(&mut p, KeyCode::Esc);
        let backend = ratatui::backend::TestBackend::new(60, 8);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| p.draw(f)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(screen.contains("#NUM!"), "{screen}");
        assert!(screen.contains("read each other in a circle"), "{screen}");
    }
}
