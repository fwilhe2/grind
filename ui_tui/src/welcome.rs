// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The pane with no document in it — what `grind-tui` shows when given nothing to open and no kind
//! to start. Three choices, each a key: `s` a new spreadsheet, `t` a new text document, `o` a path
//! to open; then the recent documents, `1` to `9`. The arrows and Enter work too, and the open
//! prompt is the only text it takes. It owns no document, so it hands back an
//! [`crate::app::Switch`].

use std::path::PathBuf;
use std::sync::Arc;

use grind_core::DocumentKind;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{RedrawFlag, Switch};
use crate::chrome;

/// What the three cards are, in the order they are drawn.
const CARDS: [(&str, char, &str); 3] = [
    ("New Spreadsheet", 's', "an empty workbook"),
    ("New Text Document", 't', "an empty page"),
    (
        "Open a Document…",
        'o',
        "a path: .ods .fods .odt .fodt .grind .xlsx .csv .md",
    ),
];

/// How many recent documents are listed, each on a digit key.
const RECENT_MAX: usize = 9;

pub struct Welcome {
    redraw: Arc<RedrawFlag>,
    /// A card, or past the cards a recent document.
    selected: usize,
    recent: Vec<PathBuf>,
    /// `Some` while the open prompt is up: the path typed so far.
    typing: Option<String>,
    message: String,
    switch: Option<Switch>,
    quit: bool,
}

impl Welcome {
    pub fn new(redraw: Arc<RedrawFlag>) -> Self {
        let mut recent = crate::recent::list();
        recent.truncate(RECENT_MAX);
        Self::with_recent(redraw, recent)
    }

    fn with_recent(redraw: Arc<RedrawFlag>, recent: Vec<PathBuf>) -> Self {
        Self {
            redraw,
            selected: 0,
            recent,
            typing: None,
            message: String::new(),
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

    /// Nothing here takes text.
    pub fn on_paste(&mut self, _text: &str) {}

    pub fn on_key(&mut self, key: KeyEvent) {
        self.redraw.raise();
        if self.typing.is_some() {
            return self.on_path_key(key);
        }
        match key.code {
            KeyCode::Char('s') => self.choose(0),
            KeyCode::Char('t') => self.choose(1),
            KeyCode::Char('o') => self.choose(2),
            KeyCode::Char(digit @ '1'..='9') => {
                let n = digit as usize - '1' as usize;
                if n < self.recent.len() {
                    self.choose(CARDS.len() + n);
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.selected = (self.selected + 1) % self.targets()
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.selected = (self.selected + self.targets() - 1) % self.targets()
            }
            KeyCode::Enter => self.choose(self.selected),
            _ => {}
        }
    }

    fn targets(&self) -> usize {
        CARDS.len() + self.recent.len()
    }

    fn choose(&mut self, target: usize) {
        self.selected = target;
        self.message.clear();
        match target {
            0 => self.switch = Some(Switch::New(DocumentKind::Spreadsheet)),
            1 => self.switch = Some(Switch::New(DocumentKind::Text)),
            2 => self.typing = Some(String::new()),
            _ => {
                let path = self.recent[target - CARDS.len()].clone();
                self.open(path);
            }
        }
    }

    fn open(&mut self, path: PathBuf) {
        match crate::sniff(&path) {
            Ok(kind) => self.switch = Some(Switch::Open(path, kind)),
            Err(error) => self.message = format!("{}: {error}", path.display()),
        }
    }

    fn on_path_key(&mut self, key: KeyEvent) {
        let Some(typed) = self.typing.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.typing = None,
            KeyCode::Backspace => {
                typed.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => typed.push(c),
            KeyCode::Enter => {
                let path = PathBuf::from(typed.trim());
                if typed.trim().is_empty() {
                    self.typing = None;
                    return;
                }
                self.open(path);
            }
            _ => {}
        }
    }

    pub fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![chrome::badge("grind")])).style(chrome::title_style()),
            rows[0],
        );

        let mut lines = vec![
            Line::styled(
                "What would you like to do?",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
        ];
        for (i, (name, key, hint)) in CARDS.iter().enumerate() {
            let on = i == self.selected;
            let marker = if on { "▸" } else { " " };
            let style = match on {
                true => Style::default().add_modifier(Modifier::REVERSED),
                false => Style::default(),
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {marker} [{key}] {name} "), style),
                Span::raw("  "),
                Span::styled(
                    (*hint).to_owned(),
                    Style::default().fg(ratatui::style::Color::DarkGray),
                ),
            ]));
        }
        if !self.recent.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "Recent",
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        for (i, path) in self.recent.iter().enumerate() {
            let on = CARDS.len() + i == self.selected;
            let marker = if on { "▸" } else { " " };
            let style = match on {
                true => Style::default().add_modifier(Modifier::REVERSED),
                false => Style::default(),
            };
            let name = path.file_name().unwrap_or(path.as_os_str());
            let dir = path.parent().map(|dir| dir.display().to_string());
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {marker} [{}] {} ", i + 1, name.to_string_lossy()),
                    style,
                ),
                Span::raw("  "),
                Span::styled(
                    dir.unwrap_or_default(),
                    Style::default().fg(ratatui::style::Color::DarkGray),
                ),
            ]));
        }
        lines.push(Line::raw(""));
        match &self.typing {
            Some(typed) => lines.push(Line::raw(format!("Open: {typed}▏"))),
            None => lines.push(Line::raw(self.message.clone())),
        }
        let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let body = center(rows[1], 72, height);
        frame.render_widget(Paragraph::new(lines), body);

        let hint = match self.typing {
            Some(_) => " Enter opens · Esc goes back",
            None if !self.recent.is_empty() => {
                " s sheet · t text · o open · 1–9 recent · Enter chooses · q quits"
            }
            None => " s sheet · t text · o open · Enter chooses · q quits",
        };
        frame.render_widget(
            Paragraph::new(Span::styled(hint, chrome::status_style()))
                .alignment(Alignment::Left)
                .style(chrome::status_style()),
            rows[2],
        );
    }
}

fn center(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn press(w: &mut Welcome, code: KeyCode) {
        w.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn welcome() -> Welcome {
        Welcome::new(Arc::new(RedrawFlag::default()))
    }

    #[test]
    fn s_and_t_start_an_empty_document_of_that_kind() {
        let mut w = welcome();
        press(&mut w, KeyCode::Char('s'));
        assert_eq!(
            w.take_switch(),
            Some(Switch::New(DocumentKind::Spreadsheet))
        );
        press(&mut w, KeyCode::Char('t'));
        assert_eq!(w.take_switch(), Some(Switch::New(DocumentKind::Text)));
    }

    #[test]
    fn the_arrows_and_enter_choose_as_the_letters_do() {
        let mut w = welcome();
        press(&mut w, KeyCode::Down);
        press(&mut w, KeyCode::Enter);
        assert_eq!(w.take_switch(), Some(Switch::New(DocumentKind::Text)));
    }

    #[test]
    fn open_takes_a_path_and_says_why_one_will_not_open() {
        let mut w = welcome();
        press(&mut w, KeyCode::Char('o'));
        for c in "/no/such/file.fods".chars() {
            press(&mut w, KeyCode::Char(c));
        }
        press(&mut w, KeyCode::Enter);
        assert_eq!(w.take_switch(), None);
        assert!(w.message.contains("/no/such/file.fods"), "{}", w.message);
        press(&mut w, KeyCode::Esc);
        assert!(w.typing.is_none());
    }

    #[test]
    fn a_recent_document_opens_on_its_digit_and_by_the_arrows() {
        let dir = std::env::temp_dir().join("grind-tui-welcome-recent");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.fods");
        std::fs::write(
            &path,
            grind_sheet::App::new()
                .save_bytes(grind_core::Form::Flat)
                .unwrap(),
        )
        .unwrap();
        let mut w = Welcome::with_recent(Arc::new(RedrawFlag::default()), vec![path.clone()]);
        let opened = Some(Switch::Open(path, DocumentKind::Spreadsheet));
        press(&mut w, KeyCode::Char('2'));
        assert_eq!(w.take_switch(), None, "there is no second one");
        press(&mut w, KeyCode::Char('1'));
        assert_eq!(w.take_switch(), opened);
        w.selected = 0;
        press(&mut w, KeyCode::Up);
        press(&mut w, KeyCode::Enter);
        assert_eq!(
            w.take_switch(),
            opened,
            "Up from the first card wraps to it"
        );
    }

    #[test]
    fn it_draws_all_three_choices() {
        let mut w = welcome();
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|frame| w.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        for card in ["New Spreadsheet", "New Text Document", "Open a Document"] {
            assert!(text.contains(card), "{card} is on screen");
        }
    }
}
