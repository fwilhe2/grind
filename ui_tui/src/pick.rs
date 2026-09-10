// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A **list to pick from**: rows that each carry an address, one of them highlighted, and `Enter`
//! handing that address back to the shell.
//!
//! The third pane of this shell's three, and the generic one. [`crate::help`] shows text,
//! [`crate::code`] shows a projection, [`crate::problems`] shows diagnostics — and each of those
//! knows what it is holding. This one does not: a row is a string to show and a string to go to,
//! which is every list a document can offer. The outline is the first caller; a list of defined
//! names or of bookmarks is the same pane with a different `Vec`.
//!
//! `ui_win32/src/dialog.rs`'s `choose` is the same idea in a different toolkit, and for the same
//! reason it gives there: a shell that grew a fourth list widget would have four answers to
//! "which key closes this".
//!
//! The keys are the pane vocabulary this shell already has: `j`/`k` move, `Enter` goes, any other
//! key closes. `doc/tui-shell.md`'s rule — vi's motions, no menu.

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// One row: what it says, where it goes, and how far in it is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// What `Enter` hands back — an address in the shell's own spelling, which is the only
    /// vocabulary this module has and it never looks inside it.
    pub address: String,
    pub label: String,
    /// Nesting, in levels. The outline's depth; zero for a flat list.
    pub depth: usize,
}

/// The pane's own state. Presentation, held by the shell for [`crate::help::Help`]'s reason.
#[derive(Clone, Debug, Default)]
pub struct Pick {
    open: bool,
    title: String,
    rows: Vec<Row>,
    selected: usize,
    scroll: usize,
}

/// What a key did, in the terms the shell cares about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nav {
    Stayed,
    /// `Enter` on a row: go to this address.
    Chose(String),
    Closed,
}

impl Pick {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open it on `rows`, with the cursor on whichever row is nearest `here` — *show me where I
    /// am in this list*, which is the direction of the map a shell always has an answer for.
    pub fn open(&mut self, title: &str, rows: Vec<Row>, here: Option<&str>) {
        self.selected = here
            .and_then(|address| rows.iter().position(|row| row.address == address))
            .unwrap_or(0);
        self.open = true;
        self.title = title.to_owned();
        self.rows = rows;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.rows = Vec::new();
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    pub fn on_key(&mut self, code: KeyCode, height: usize) -> Nav {
        let last = self.rows.len().saturating_sub(1);
        let page = height.saturating_sub(2).max(1);
        match code {
            KeyCode::Char('j') | KeyCode::Down => self.selected = (self.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::PageDown => self.selected = (self.selected + page).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => self.selected = 0,
            KeyCode::Char('G') | KeyCode::End => self.selected = last,
            KeyCode::Enter => {
                let Some(address) = self.selected().map(|row| row.address.clone()) else {
                    return Nav::Stayed;
                };
                self.close();
                return Nav::Chose(address);
            }
            _ => {
                self.close();
                return Nav::Closed;
            }
        }
        Nav::Stayed
    }

    /// Draw it over `area` — the whole of it, like every other pane here: a list the reader asked
    /// for is what they are reading.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, what: &str) {
        let height = usize::from(area.height).saturating_sub(2).max(1);
        self.scroll = self.scroll.min(self.selected);
        if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }

        let mut lines: Vec<Line> = Vec::with_capacity(height);
        if self.rows.is_empty() {
            lines.push(Line::from(Span::styled(
                format!(" This document has no {what}."),
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
        let end = (self.scroll + height).min(self.rows.len());
        for index in self.scroll..end {
            let row = &self.rows[index];
            let mark = match index == self.selected {
                true => Style::default().add_modifier(Modifier::REVERSED),
                false => Style::default(),
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:width$}", row.address, width = 9),
                    mark.fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{}{}", "  ".repeat(row.depth), row.label), mark),
            ]));
        }

        let footer = match self.rows.is_empty() {
            true => "  any key closes  ".to_owned(),
            false => format!(
                "  {} of {}  \u{00b7}  Enter goes there, j/k moves, any other key closes  ",
                self.selected + 1,
                self.rows.len()
            ),
        };
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", self.title))
                    .title_bottom(footer)
                    .style(Style::default().bg(Color::Reset)),
            ),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Row> {
        [
            ("\u{a7}1", "One", 0),
            ("\u{a7}1.1", "Under it", 1),
            ("\u{a7}2", "Two", 0),
        ]
        .into_iter()
        .map(|(address, label, depth)| Row {
            address: address.to_owned(),
            label: label.to_owned(),
            depth,
        })
        .collect()
    }

    #[test]
    fn it_opens_on_the_row_the_caret_is_already_in() {
        let mut pick = Pick::default();
        pick.open("Outline", rows(), Some("\u{a7}2"));
        assert_eq!(pick.selected().map(|row| row.label.as_str()), Some("Two"));

        // An address the list does not hold opens at the top rather than anywhere surprising.
        pick.open("Outline", rows(), Some("\u{a7}9"));
        assert_eq!(pick.selected().map(|row| row.label.as_str()), Some("One"));
    }

    #[test]
    fn enter_hands_back_the_address_and_closes() {
        let mut pick = Pick::default();
        pick.open("Outline", rows(), None);
        pick.on_key(KeyCode::Char('j'), 10);
        assert_eq!(
            pick.on_key(KeyCode::Enter, 10),
            Nav::Chose("\u{a7}1.1".to_owned())
        );
        assert!(!pick.is_open(), "going there is done looking at the list");
    }

    #[test]
    fn moving_stops_at_both_ends_and_any_other_key_closes_it() {
        let mut pick = Pick::default();
        pick.open("Outline", rows(), None);
        for _ in 0..20 {
            pick.on_key(KeyCode::Char('j'), 8);
        }
        assert_eq!(pick.selected().map(|row| row.label.as_str()), Some("Two"));
        for _ in 0..20 {
            pick.on_key(KeyCode::Char('k'), 8);
        }
        assert_eq!(pick.selected().map(|row| row.label.as_str()), Some("One"));
        assert!(pick.is_open(), "moving never closes it");
        assert_eq!(pick.on_key(KeyCode::Esc, 8), Nav::Closed);
        assert!(!pick.is_open());
    }

    /// An empty list is a sentence rather than a blank box, and it has nothing to go to.
    #[test]
    fn an_empty_list_says_so_and_enter_does_nothing() {
        let mut pick = Pick::default();
        pick.open("Outline", Vec::new(), None);
        assert_eq!(pick.on_key(KeyCode::Enter, 8), Nav::Stayed);
        assert!(pick.is_open());

        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(44, 6)).expect("a test terminal");
        terminal
            .draw(|frame| pick.draw(frame, frame.area(), "headings"))
            .expect("draws");
        let shown = terminal.backend().to_string();
        assert!(shown.contains("no headings"), "{shown}");
    }

    #[test]
    fn it_draws_the_rows_indented_by_depth_and_keeps_the_cursor_on_screen() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut pick = Pick::default();
        pick.open("Outline", rows(), Some("\u{a7}2"));
        let mut terminal = Terminal::new(TestBackend::new(44, 4)).expect("a test terminal");
        terminal
            .draw(|frame| pick.draw(frame, frame.area(), "headings"))
            .expect("draws");
        let shown = terminal.backend().to_string();
        assert!(shown.contains("Outline"), "{shown}");
        assert!(
            shown.contains("Two"),
            "a window of two rows scrolled to the cursor: {shown}"
        );
        assert!(shown.contains("3 of 3"), "{shown}");
    }
}
