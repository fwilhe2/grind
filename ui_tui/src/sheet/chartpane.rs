// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **`:charts`** — the sheet's charts as a list to take hold of, the terminal's half of
//! `doc/chart-handling.md`.
//!
//! A terminal has no pointer to click a chart with and draws none on the grid, so the gesture
//! the windows have — click it, see it selected, act on it — is this pane: every chart a row, one
//! of them **selected** (reversed, as every list in this shell marks its cursor) and drawn in
//! characters below the list (`chartview`), and the keys acting on that one. It stays selected
//! when the pane closes, so `:chart <words>`, `:chart here` and `:chart!` mean it too.
//!
//! The keys are the same verbs every window gives a selected chart, in this shell's vocabulary:
//! `d`/`x`/Delete deletes it (and `u` brings it back), `H`/`J`/`K`/`L` nudge it a cell, `+`/`-`
//! make it bigger or smaller, `m` moves it to the cursor's cell, Enter changes it in words, Esc
//! lets go. Pure state and a reply; `app.rs` does what a reply asks.

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// The pane's own state: whether it is showing and which chart is selected in it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Charts {
    open: bool,
    selected: usize,
}

/// What a key asks the shell to do to the selected chart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reply {
    /// Nothing for the document; the pane redraws.
    Stay,
    /// The pane closed; the chart stays selected.
    Close,
    /// Esc: the pane closed and the chart let go of.
    Release,
    Delete(usize),
    /// Its corner to the cursor's cell.
    Move(usize),
    /// Enter: the command line, ready for `:chart <words>` on this chart.
    Change(usize),
    /// A cell's step, columns and rows.
    Nudge(usize, i8, i8),
    /// Bigger or smaller by this factor, its corner held.
    Scale(usize, f64),
}

/// How much `+` and `-` change a chart's size.
pub const STEP: f64 = 1.2;

pub const KEYS: &str =
    " j/k choose · Enter change · m move here · HJKL nudge · +/- size · d delete · Esc let go ";

impl Charts {
    pub fn is_open(self) -> bool {
        self.open
    }

    /// Open on chart `selected` — the one already selected, or the sheet's last.
    pub fn open(&mut self, selected: usize) {
        self.open = true;
        self.selected = selected;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn selected(self) -> usize {
        self.selected
    }

    /// A key with `count` charts on the sheet.
    pub fn on_key(&mut self, code: KeyCode, count: usize) -> Reply {
        if count == 0 {
            self.close();
            return Reply::Release;
        }
        self.selected = self.selected.min(count - 1);
        let at = self.selected;
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.selected = (at + 1).min(count - 1);
                Reply::Stay
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = at.saturating_sub(1);
                Reply::Stay
            }
            KeyCode::Char('d') | KeyCode::Char('x') | KeyCode::Delete | KeyCode::Backspace => {
                Reply::Delete(at)
            }
            KeyCode::Char('m') => Reply::Move(at),
            KeyCode::Char('H') => Reply::Nudge(at, -1, 0),
            KeyCode::Char('L') => Reply::Nudge(at, 1, 0),
            KeyCode::Char('K') => Reply::Nudge(at, 0, -1),
            KeyCode::Char('J') => Reply::Nudge(at, 0, 1),
            KeyCode::Char('+') | KeyCode::Char('=') => Reply::Scale(at, STEP),
            KeyCode::Char('-') => Reply::Scale(at, 1.0 / STEP),
            KeyCode::Enter | KeyCode::Char('c') => {
                self.close();
                Reply::Change(at)
            }
            KeyCode::Esc => {
                self.close();
                Reply::Release
            }
            _ => {
                self.close();
                Reply::Close
            }
        }
    }

    /// Draw it over `area`: one row per chart (`rows`), the selected one reversed, and under them
    /// the selected chart drawn in characters (`drawing`).
    pub fn draw(self, frame: &mut Frame, area: Rect, rows: &[String], drawing: &[String]) {
        let width = usize::from(area.width.saturating_sub(2));
        let mut lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| match index == self.selected {
                true => crate::pick::selected_row(vec![Span::raw(row.clone())], width),
                false => Line::from(Span::raw(row.clone())),
            })
            .collect();
        lines.push(Line::from(Span::styled(
            "─".repeat(width),
            Style::default().fg(Color::DarkGray),
        )));
        lines.extend(drawing.iter().map(|line| Line::from(line.clone())));
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" charts ")
                    .title_bottom(KEYS)
                    .style(Style::default().bg(Color::Reset)),
            ),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_act_on_the_selected_chart() {
        let mut pane = Charts::default();
        pane.open(0);
        assert_eq!(pane.on_key(KeyCode::Char('j'), 3), Reply::Stay);
        assert_eq!(pane.on_key(KeyCode::Char('j'), 3), Reply::Stay);
        assert_eq!(
            pane.on_key(KeyCode::Char('j'), 3),
            Reply::Stay,
            "stops at the last"
        );
        assert_eq!(pane.selected(), 2);
        assert_eq!(pane.on_key(KeyCode::Char('d'), 3), Reply::Delete(2));
        assert_eq!(
            pane.on_key(KeyCode::Char('L'), 2),
            Reply::Nudge(1, 1, 0),
            "clamped"
        );
        assert_eq!(pane.on_key(KeyCode::Char('+'), 2), Reply::Scale(1, STEP));
        assert!(pane.is_open());
        assert_eq!(pane.on_key(KeyCode::Enter, 2), Reply::Change(1));
        assert!(!pane.is_open());
        pane.open(0);
        assert_eq!(pane.on_key(KeyCode::Esc, 2), Reply::Release);
        pane.open(0);
        assert_eq!(pane.on_key(KeyCode::Char('q'), 2), Reply::Close);
    }
}
