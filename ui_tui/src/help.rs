// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The key list, and the pane that shows it.
//!
//! **Written once, shown twice.** `grind-tui --help` prints it before the terminal is taken
//! over; `:help` shows it inside, over the document. A list that lived in two places would be
//! wrong in one of them, and the one nobody reads is the one that rots — so each shell owns
//! its own section (`sheet::HELP`, `text::HELP`), this file owns what they share, and both
//! entry points compose the same strings.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Everything that means the same thing in both shells.
pub const COMMON: &str = "\
Normal mode, both document types (vi-style):
  h j k l / arrows   move
  Ctrl+f / Ctrl+b    page down / up
  0 / $              start / end of the line
  g / G              start / end of the document
  v                  select — Visual mode, a rectangle or a run of text
  y / p              yank the selection / put it back
  u / Ctrl+r         undo / redo
  :                  command line          :help  this page

Visual mode — one notation for emphasis, whichever document it is:
  *  bold       /  italic       -  no formatting
  d / x  delete or clear what is selected   Esc  stop selecting

Saving and leaving, both:
  :w [file]   :q   :q!   :wq or :x
";

/// A help pane's scroll position, and whether it is showing at all.
///
/// Held by each shell rather than by this module: it is presentation state, and a shell that
/// asked a global for it would be a shell that could not be tested twice in one process.
#[derive(Clone, Copy, Debug, Default)]
pub struct Help {
    open: bool,
    scroll: usize,
}

impl Help {
    pub fn is_open(self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// A key while the pane is open. Every key closes it except the ones that scroll, which is
    /// what a reader expects of something they opened to *read* — there is nothing to lose by
    /// pressing the wrong one.
    pub fn on_key(
        &mut self,
        code: ratatui::crossterm::event::KeyCode,
        lines: usize,
        height: usize,
    ) {
        use ratatui::crossterm::event::KeyCode;
        let page = height.saturating_sub(2).max(1);
        let last = lines.saturating_sub(height.saturating_sub(2));
        match code {
            KeyCode::Char('j') | KeyCode::Down => self.scroll = (self.scroll + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char('f') => self.scroll = (self.scroll + page).min(last),
            KeyCode::PageUp | KeyCode::Char('b') => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => self.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => self.scroll = last,
            _ => self.close(),
        }
    }

    /// Draw it over `area`, from `text` — the whole area, because a key list is what the
    /// reader asked to look at and half of one is worse than none.
    pub fn draw(self, frame: &mut Frame, area: Rect, text: &str) {
        let height = usize::from(area.height);
        let lines: Vec<Line> = text
            .lines()
            .skip(self.scroll)
            .take(height.saturating_sub(2))
            .map(|line| match line.starts_with(' ') || line.is_empty() {
                // A section's own heading is the line that does not start indented.
                true => Line::from(Span::raw(line.to_string())),
                false => Line::from(Span::styled(
                    line.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
            })
            .collect();
        let more = match self.scroll + height.saturating_sub(2) < text.lines().count() {
            true => "  j/k scroll  ",
            false => "  any key closes  ",
        };
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" grind-tui — keys and commands ")
                    .title_bottom(more)
                    .style(Style::default().bg(Color::Reset)),
            ),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyCode;

    /// Every section is composed from the same strings `--help` prints, so a key documented in
    /// one place is documented in both. This checks the *shape* that guarantee relies on: a
    /// shell's own help holds its own section and the shared one.
    #[test]
    fn each_shell_shows_the_shared_keys_and_its_own() {
        let sheet = crate::sheet::help();
        let text = crate::text::help();
        for shared in ["h j k l / arrows", ":help  this page", ":wq or :x"] {
            assert!(sheet.contains(shared), "{shared} missing from the sheet's");
            assert!(
                text.contains(shared),
                "{shared} missing from the document's"
            );
        }
        assert!(sheet.contains(":recalc"), "the sheet's own");
        assert!(text.contains("**bold**"), "the document's own");
        assert!(!sheet.contains("**bold**"), "and not each other's");
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut help = Help::default();
        help.open();
        assert!(help.is_open());
        // Ten lines in a window of six leaves four to scroll through.
        for _ in 0..20 {
            help.on_key(KeyCode::Char('j'), 10, 6);
        }
        assert_eq!(help.scroll, 6);
        for _ in 0..20 {
            help.on_key(KeyCode::Char('k'), 10, 6);
        }
        assert_eq!(help.scroll, 0);
        assert!(help.is_open(), "scrolling never closes it");
    }

    #[test]
    fn any_other_key_closes_it() {
        let mut help = Help::default();
        help.open();
        help.on_key(KeyCode::Esc, 10, 6);
        assert!(!help.is_open());
    }
}
