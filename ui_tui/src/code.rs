// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The **code view** — the document as its projection, shown (`doc/dsl.md` §6, D9).
//!
//! Delphi's form and its `.dfm`, in a terminal: the grid on one pane and its source on the
//! other, the same document either way. What it costs is this file, because the projection
//! writer is already D1 and D2 and the two maps it emits are already tested — a code view is
//! that output plus something that shows text, and this shell shows text for a living.
//!
//! **Shared by both halves of this shell, which nothing else here is.** [`crate::app`] says the
//! spreadsheet's grid and the document's flow have no rendering in common and does not invent a
//! widget for them; the projection is the one thing they *do* render identically, because it is
//! plain text in both cases and its colours come from a token map whose vocabulary is the core's.
//! So a spreadsheet's source and a text document's source are one widget with two callers, and
//! neither of them knows which it is holding.
//!
//! **Read-only, and §6.4 is the argument.** An editable code view is a text editor whose buffer
//! is a document, and reconciling the two needs an error-tolerant parser, a model diff and an
//! answer for the other view's caret — none of which exists. What is here instead is the half
//! that pays for itself immediately: *see the correspondence*. The cursor is a line, the line
//! reports the address it projects, and the shell selects it in the document. §6.2's point
//! exactly — the toggle is the cheap half and the span map is the artefact.
//!
//! Three colours and no more, from the terminal's own palette rather than from hex: a code view
//! in a terminal is read in whatever theme the reader chose, and a shell that picked its own
//! background would be the one window on their screen that ignored it.

use grind_core::projection::{Projection, TokenKind};
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// The pane's own state: whether it is showing, and where its cursor is.
///
/// Presentation state, held by each shell half for [`crate::help::Help`]'s reason — a module
/// that kept it in a global could not be tested twice in one process. Nothing here is ever
/// written to a document, which is the same thing `doc/view-modes.md` says about an overlay.
#[derive(Clone, Copy, Debug, Default)]
pub struct Code {
    open: bool,
    /// The cursor, as a line of the projection. Not a byte offset: this pane cannot edit, so a
    /// column would be a cursor nobody could use for anything.
    line: usize,
    scroll: usize,
}

/// What a key did, in the terms the shell cares about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    /// The pane handled it and the cursor did not move onto a new address.
    Stayed,
    /// The cursor moved; ask [`Code::address`] which address it is on now and select it.
    Moved,
    /// The pane closed.
    Closed,
}

impl Code {
    pub fn is_open(self) -> bool {
        self.open
    }

    /// Open it, with the cursor on the line that projects `address` — *show me this cell in the
    /// source*, which is the direction of §6.2's map that the shell always has an answer for.
    pub fn open(&mut self, projection: &Projection, address: Option<&str>) {
        self.open = true;
        self.line = address
            .and_then(|address| projection.line_of(address))
            .unwrap_or(0);
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Where the cursor is.
    ///
    /// Test-only: a shell asks [`Code::address`] what the cursor is *on*, which is the question
    /// it can act on. A line number is what a test checks, because a line number is what moving
    /// is defined in terms of.
    #[cfg(test)]
    pub fn line(self) -> usize {
        self.line
    }

    /// Which address the cursor is on, if the line projects one.
    ///
    /// A line that projects nothing — a blank, a closing brace, the header — answers `None`, and
    /// the shell leaves its selection where it was rather than jumping somewhere arbitrary.
    pub fn address(self, projection: &Projection) -> Option<&str> {
        projection.address_on_line(self.line)
    }

    /// A key while the pane is open.
    ///
    /// The same keys as the document underneath, because this is the same shell: `j`/`k` move,
    /// `Ctrl+f`/`Ctrl+b` page, `g`/`G` go to the ends. Anything else closes it, which is
    /// [`crate::help::Help`]'s rule and is right for the same reason — there is nothing here to
    /// lose by pressing the wrong key.
    pub fn on_key(&mut self, code: KeyCode, projection: &Projection, height: usize) -> Nav {
        let last = projection.line_count().saturating_sub(1);
        let page = height.saturating_sub(2).max(1);
        let was = self.line;
        match code {
            KeyCode::Char('j') | KeyCode::Down => self.line = (self.line + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.line = self.line.saturating_sub(1),
            KeyCode::PageDown => self.line = (self.line + page).min(last),
            KeyCode::PageUp => self.line = self.line.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => self.line = 0,
            KeyCode::Char('G') | KeyCode::End => self.line = last,
            _ => {
                self.close();
                return Nav::Closed;
            }
        }
        match self.line == was {
            true => Nav::Stayed,
            false => Nav::Moved,
        }
    }

    /// `Ctrl+f` / `Ctrl+b`, which arrive as a modifier rather than as their own `KeyCode`.
    pub fn page(&mut self, down: bool, projection: &Projection, height: usize) -> Nav {
        self.on_key(
            match down {
                true => KeyCode::PageDown,
                false => KeyCode::PageUp,
            },
            projection,
            height,
        )
    }

    /// Draw it over `area`.
    ///
    /// The whole area, like the help pane: a code view is what the reader asked to look at, and a
    /// third of one beside a grid is the worst of both. `title` is the shell's, because only the
    /// shell knows what the file is called.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, projection: &Projection, title: &str) {
        let height = usize::from(area.height).saturating_sub(2).max(1);
        // Keep the cursor on screen, exactly as the grid keeps the active cell on screen.
        self.scroll = self.scroll.min(self.line);
        if self.line >= self.scroll + height {
            self.scroll = self.line + 1 - height;
        }
        let width = format!("{}", projection.line_count().max(1)).len();

        let mut lines = Vec::with_capacity(height);
        for index in self.scroll..(self.scroll + height).min(projection.line_count()) {
            let mut spans = vec![Span::styled(
                format!("{:>width$} ", index + 1),
                Style::default().add_modifier(Modifier::DIM),
            )];
            let cursor = index == self.line;
            for piece in projection.line_pieces(index) {
                let mut style = match piece.kind {
                    Some(kind) => Style::default().fg(colour(kind)),
                    None => Style::default(),
                };
                if cursor {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                spans.push(Span::styled(piece.text.to_owned(), style));
            }
            lines.push(Line::from(spans));
        }

        // What this line *is*, in the document's own vocabulary. The whole point of the pane: a
        // reader looking at `row North 4200` is told it is `Sales.A2`, in the spelling they would
        // type into the go-to box.
        let at = self
            .address(projection)
            .map(|address| format!("  {address}  "))
            .unwrap_or_else(|| "  j/k moves, any other key closes  ".to_owned());
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {title} — source "))
                    .title_bottom(at)
                    .style(Style::default().bg(Color::Reset)),
            ),
            area,
        );
    }
}

/// What each kind of token is painted.
///
/// The shell's decision and not the core's — `TokenKind` says what a byte *is*, and what colour
/// that gets is exactly the part a shell owns (`doc/dsl.md` §6.6). Named colours rather than hex
/// for the reason every colour in `ui_sheet_gtk` comes from the theme: the reader chose a
/// palette, and a code view that ignored it would be the one window on their screen that did.
fn colour(kind: TokenKind) -> Color {
    match kind {
        TokenKind::Node => Color::Cyan,
        TokenKind::Property => Color::Magenta,
        TokenKind::Text => Color::Green,
        TokenKind::Number => Color::Yellow,
        TokenKind::Keyword => Color::Blue,
        TokenKind::Comment => Color::DarkGray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spreadsheet() -> Projection {
        let doc = grind_sheet::projection::read(
            "grind spreadsheet\n\nsheet Sales {\n    at A1 {\n        row North 4200\n    }\n\
             \n    cell B3 \"=[.B1]*2\"\n}\n",
        )
        .expect("parses");
        grind_sheet::projection::project(&doc)
    }

    fn document() -> Projection {
        let doc = grind_text::projection::read("grind text\n\nh 1 \"One\"\np \"{#intro}text\"\n")
            .expect("parses");
        grind_text::projection::project(&doc)
    }

    /// One widget, two document types, and it cannot tell which it has — the claim this module's
    /// existence rests on.
    #[test]
    fn the_same_pane_shows_either_document_type() {
        for projection in [spreadsheet(), document()] {
            let mut code = Code::default();
            code.open(&projection, None);
            assert!(code.is_open());
            assert!(projection.line_count() > 2);
            code.on_key(KeyCode::Char('G'), &projection, 10);
            assert_eq!(code.line(), projection.line_count() - 1);
        }
    }

    /// Opening on an address puts the cursor where that address is, and the cursor reports it
    /// back — §6.2's map, both directions, which is the milestone's actual output.
    #[test]
    fn the_cursor_lands_on_the_address_it_was_opened_with_and_reports_it() {
        let projection = spreadsheet();
        let mut code = Code::default();
        code.open(&projection, Some("Sales.B3"));
        assert_eq!(code.address(&projection), Some("Sales.B3"));
        assert!(
            projection
                .line_pieces(code.line())
                .iter()
                .any(|p| p.text == "cell"),
            "and it really is that line"
        );

        // Moving reports a different one, or none, and never lies about which.
        let mut seen = Vec::new();
        code.open(&projection, None);
        for _ in 0..projection.line_count() {
            seen.push(code.address(&projection).map(str::to_owned));
            code.on_key(KeyCode::Char('j'), &projection, 20);
        }
        assert!(seen.iter().flatten().any(|a| a == "Sales.B3"));
        // A grid row is one line and two cells; the tie goes to the leftmost, which is the one
        // documented answer rather than whichever the anchor list happened to hold first.
        assert!(seen.iter().flatten().any(|a| a == "Sales.A1"));
        assert!(seen.contains(&None), "a blank line projects nothing");
    }

    /// An address the projection does not spell leaves the cursor at the top rather than
    /// anywhere surprising — a cell that is empty is not in the file, and asking for it is an
    /// ordinary thing for a grid cursor to do.
    #[test]
    fn an_address_the_file_does_not_spell_opens_at_the_top() {
        let projection = spreadsheet();
        let mut code = Code::default();
        code.open(&projection, Some("Sales.Z99"));
        assert_eq!(code.line(), 0);
    }

    #[test]
    fn moving_stops_at_both_ends_and_any_other_key_closes_it() {
        let projection = spreadsheet();
        let mut code = Code::default();
        code.open(&projection, None);
        for _ in 0..100 {
            code.on_key(KeyCode::Char('j'), &projection, 8);
        }
        assert_eq!(code.line(), projection.line_count() - 1);
        assert_eq!(
            code.on_key(KeyCode::Char('j'), &projection, 8),
            Nav::Stayed,
            "at the end, `j` is not a move"
        );
        for _ in 0..100 {
            code.on_key(KeyCode::Char('k'), &projection, 8);
        }
        assert_eq!(code.line(), 0);
        assert!(code.is_open(), "moving never closes it");
        assert_eq!(code.on_key(KeyCode::Esc, &projection, 8), Nav::Closed);
        assert!(!code.is_open());
    }

    /// What is drawn is the projection, coloured, with a gutter — and the scroll follows the
    /// cursor rather than the other way round.
    #[test]
    fn it_draws_the_source_and_keeps_the_cursor_on_screen() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let projection = spreadsheet();
        let mut terminal = Terminal::new(TestBackend::new(44, 6)).expect("a test terminal");
        let mut code = Code::default();
        code.open(&projection, Some("Sales.B3"));
        terminal
            .draw(|frame| code.draw(frame, frame.area(), &projection, "book.fods"))
            .expect("draws");

        let shown = terminal.backend().to_string();
        assert!(shown.contains("book.fods — source"), "{shown}");
        assert!(shown.contains("Sales.B3"), "the line's address: {shown}");
        assert!(
            shown.contains("cell B3"),
            "a window of four lines scrolled to the cursor: {shown}"
        );
    }
}
