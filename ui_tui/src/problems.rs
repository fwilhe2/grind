// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The **problems pane** — `grind lint`'s findings, in the terminal (`doc/dsl.md` §4.3, D6).
//!
//! **Shared by both halves of this shell**, like [`crate::code`] and for the same reason: a
//! diagnostic is document-type-neutral by construction (`grind_core::lint`), so a spreadsheet's
//! findings and a text document's are one list with two callers and neither knows which it is
//! holding. What differs is only what an address *means*, and that is the caller's business —
//! this pane hands back the string and the shell resolves it the way it resolves any other
//! address.
//!
//! The keys are the pane vocabulary this shell already has: `j`/`k` move, `Enter` goes to the
//! finding, any other key closes. `doc/tui-shell.md`'s rule — vi's motions, no menu.

use grind_core::lint::{Diagnostic, Report, Severity};
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// The pane's own state: the findings it is showing, and which one is selected.
///
/// The report is held rather than re-derived per frame, for [`crate::code`]'s reason: linting
/// costs a recalculation, and a pane that re-ran it every keystroke would make `j` the most
/// expensive key in the shell. It is a snapshot, and the status line says so by naming the verb
/// that takes a new one.
#[derive(Clone, Debug, Default)]
pub struct Problems {
    open: bool,
    report: Report,
    selected: usize,
    scroll: usize,
}

/// What a key did, in the terms the shell cares about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nav {
    /// Handled; nothing for the shell to do.
    Stayed,
    /// `Enter` on a finding: go to this address.
    Chose(String),
    Closed,
}

impl Problems {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, report: Report) {
        self.open = true;
        self.report = report;
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.report = Report::default();
    }

    /// The finding under the cursor.
    pub fn selected(&self) -> Option<&Diagnostic> {
        self.report.diagnostics.get(self.selected)
    }

    /// A key while the pane is open.
    pub fn on_key(&mut self, code: KeyCode, height: usize) -> Nav {
        let last = self.report.len().saturating_sub(1);
        let page = height.saturating_sub(2).max(1);
        match code {
            KeyCode::Char('j') | KeyCode::Down => self.selected = (self.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::PageDown => self.selected = (self.selected + page).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => self.selected = 0,
            KeyCode::Char('G') | KeyCode::End => self.selected = last,
            KeyCode::Enter => {
                // An address is what a finding is *for*: going to it closes the pane, because
                // the next thing the reader wants is the document. A finding about the whole
                // document has no address and stays put rather than jumping somewhere invented.
                let address = self.selected().map(|d| d.at.clone()).unwrap_or_default();
                if address.is_empty() {
                    return Nav::Stayed;
                }
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

    /// Draw it over `area` — the whole area, like the help and code panes: a list the reader
    /// asked for is what they are reading.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, title: &str) {
        let height = usize::from(area.height).saturating_sub(2).max(1);
        self.scroll = self.scroll.min(self.selected);
        if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }

        let mut lines: Vec<Line> = Vec::with_capacity(height);
        if self.report.is_empty() {
            lines.push(Line::from(Span::styled(
                " Nothing to report — the document does not contradict itself.",
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
        let end = (self.scroll + height).min(self.report.len());
        for index in self.scroll..end {
            let diagnostic = &self.report.diagnostics[index];
            let mut style = Style::default().fg(colour(diagnostic.severity));
            if index == self.selected {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let at = match diagnostic.at.is_empty() {
                true => String::new(),
                false => format!("{} ", diagnostic.at),
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", mark(diagnostic.severity)), style),
                Span::styled(at, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(diagnostic.message.clone()),
                Span::styled(
                    format!(" [{}]", diagnostic.rule),
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ]));
        }

        let footer = match self.report.is_empty() {
            true => "  any key closes  ".to_owned(),
            false => format!(
                "  {} of {}  ·  Enter goes there, j/k moves, any other key closes  ",
                self.selected + 1,
                self.report.len()
            ),
        };
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {title} — problems "))
                    .title_bottom(footer)
                    .style(Style::default().bg(Color::Reset)),
            ),
            area,
        );
    }
}

/// One character per severity, because a terminal has no icon theme — and the same three
/// `CellRole::marker` reaches for: a glyph is a shell's decision, and a *table* of them belongs
/// in one place rather than in each half of this shell.
pub fn mark(severity: Severity) -> char {
    match severity {
        Severity::Error => '!',
        Severity::Warning => '▲',
        Severity::Hint => '·',
    }
}

/// What each severity is painted. Named colours rather than hex, for `code.rs`'s reason: the
/// reader chose a palette and this is not the window that ignores it.
fn colour(severity: Severity) -> Color {
    match severity {
        Severity::Error => Color::Red,
        Severity::Warning => Color::Yellow,
        Severity::Hint => Color::Cyan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::lint::{Rule, Severity};

    const RULE: Rule = Rule {
        id: "stale-value",
        severity: Severity::Warning,
        what: "a cell whose cached value disagrees with its formula",
    };

    fn report(addresses: &[&str]) -> Report {
        let mut report = Report::default();
        for at in addresses {
            report.push(Diagnostic::new(&RULE, *at, "something"));
        }
        report
    }

    #[test]
    fn j_and_k_move_within_the_list_and_stop_at_its_ends() {
        let mut pane = Problems::default();
        pane.open(report(&["A1", "A2", "A3"]));
        assert!(pane.is_open());
        assert_eq!(pane.on_key(KeyCode::Char('k'), 10), Nav::Stayed);
        assert_eq!(pane.selected().unwrap().at, "A1", "already at the top");
        pane.on_key(KeyCode::Char('j'), 10);
        pane.on_key(KeyCode::Char('j'), 10);
        pane.on_key(KeyCode::Char('j'), 10);
        assert_eq!(pane.selected().unwrap().at, "A3", "and at the bottom");
    }

    #[test]
    fn enter_hands_back_the_address_and_closes() {
        let mut pane = Problems::default();
        pane.open(report(&["Sheet1.B12"]));
        assert_eq!(
            pane.on_key(KeyCode::Enter, 10),
            Nav::Chose("Sheet1.B12".to_owned())
        );
        assert!(!pane.is_open(), "going there is done looking at the list");
    }

    #[test]
    fn a_finding_with_no_address_has_nowhere_to_go() {
        let mut pane = Problems::default();
        pane.open(report(&[""]));
        assert_eq!(pane.on_key(KeyCode::Enter, 10), Nav::Stayed);
        assert!(pane.is_open(), "and it does not close on a no-op");
    }

    #[test]
    fn any_other_key_closes_it() {
        let mut pane = Problems::default();
        pane.open(report(&["A1"]));
        assert_eq!(pane.on_key(KeyCode::Char('x'), 10), Nav::Closed);
        assert!(!pane.is_open());
        assert!(
            pane.selected().is_none(),
            "and it lets the snapshot go rather than holding a document-sized list shut"
        );
    }

    #[test]
    fn every_severity_has_a_mark_of_its_own() {
        let marks: Vec<char> = [Severity::Error, Severity::Warning, Severity::Hint]
            .into_iter()
            .map(mark)
            .collect();
        assert!(marks[0] != marks[1] && marks[1] != marks[2] && marks[0] != marks[2]);
    }
}
