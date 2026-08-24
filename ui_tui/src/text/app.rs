// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Event routing and rendering for the word processor.
//!
//! Holds no document state of its own. Blocks come from [`grind_text::App::get_viewport`] and
//! lines from [`grind_text::App::layout_block`] every frame; the only fields here are
//! presentation concerns — the caret, the scroll offset, the mode and a status line.
//!
//! **Where the editing model is, and is not.** Every motion this shell offers is answered by
//! the core: `j` is `App::caret_line`, `0` and `$` are `App::caret_line_bounds`, typing is
//! `App::insert_text`, Enter is `App::split_block`, Backspace at the front of a block is
//! `App::join_block`. This file decides *which* question to ask and where to draw the answer.
//! That division is `doc/text-layout.md`'s whole point: the GTK shell will ask the same
//! questions of the same code and get the same answers, in different units.
//!
//! Three modes, vi-style. **Normal** navigates ([`super::keymap`]), **Insert** types at the
//! caret, **Command** (`:`) runs a line. One difference from the spreadsheet's shell is worth
//! naming: Insert here edits the *document*, not a staged buffer, so `Esc` returns to Normal
//! without undoing anything — `u` is how you take a sentence back, exactly as in vi. A cell's
//! edit is staged because a half-typed formula is not a value; a half-typed sentence is a
//! sentence.

use std::path::PathBuf;
use std::sync::Arc;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout as Rects};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use grind_text::{App as CoreApp, BlockKind, Caret};

use super::Cells;
use super::keymap::{self, Action, Motion};
use crate::app::RedrawFlag;

/// Room for `p12 h1 ` down the left, so a reader can see the structure the outline is made of.
const GUTTER: u16 = 8;

enum Mode {
    Normal,
    Insert,
    Command { buf: String },
}

pub struct App {
    core: Arc<CoreApp>,
    redraw: Arc<RedrawFlag>,
    path: Option<PathBuf>,
    caret: Caret,
    /// The first document line on screen, as a block and a line within it. Scrolling by line
    /// rather than by block, because one paragraph can be taller than the window.
    top: (usize, usize),
    /// The column the caret is trying to keep while moving by lines — see
    /// `grind_text::App::caret_line`. Cleared by any horizontal move, which is what makes
    /// walking down through a short line and out the other side come back to where it started.
    goal_x: Option<f32>,
    width: f32,
    height: usize,
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
            caret: Caret {
                block: 0,
                offset: 0,
            },
            top: (0, 0),
            goal_x: None,
            width: 60.0,
            height: 20,
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
            Mode::Insert => self.on_insert_key(key.code),
            Mode::Command { .. } => self.on_command_key(key.code),
        }
    }

    // --- Normal mode ---

    fn on_normal_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        let Some(action) = keymap::normal_action(code, mods) else {
            return;
        };
        match action {
            Action::Move(motion) => self.go(motion),
            Action::Insert => self.mode = Mode::Insert,
            Action::Append => {
                self.go(Motion::Char(1));
                self.mode = Mode::Insert;
            }
            Action::OpenBelow => self.open_below(),
            Action::EraseChar => self.erase_forward(),
            Action::DeleteBlock => self.delete_block(),
            Action::Join => self.join(),
            Action::Undo => self.history(self.core.undo(), "nothing to undo"),
            Action::Redo => self.history(self.core.redo(), "nothing to redo"),
            Action::Command => self.mode = Mode::Command { buf: String::new() },
        }
    }

    /// Every motion, routed to the core.
    ///
    /// The horizontal ones are the only arithmetic in this file, and they are arithmetic over
    /// *characters* rather than over layout — walking off the end of a block onto the next is a
    /// document fact, not a line one, so the shell may do it.
    fn go(&mut self, motion: Motion) {
        let blocks = self.core.block_count();
        if blocks == 0 {
            return;
        }
        match motion {
            Motion::Char(delta) => {
                self.goal_x = None;
                self.caret = self.stepped(delta);
            }
            Motion::Line(delta) => {
                // Remembered across a run of j/k, which is what `goal_x` is for.
                let goal = match self.goal_x {
                    Some(x) => x,
                    None => self
                        .core
                        .caret_x(self.caret, self.width, &Cells)
                        .unwrap_or(0.0),
                };
                self.goal_x = Some(goal);
                if let Ok(moved) =
                    self.core
                        .caret_line(self.caret, delta as isize, goal, self.width, &Cells)
                {
                    self.caret = moved;
                }
            }
            Motion::LineStart | Motion::LineEnd => {
                self.goal_x = None;
                if let Ok((start, end)) =
                    self.core.caret_line_bounds(self.caret, self.width, &Cells)
                {
                    self.caret = match motion {
                        Motion::LineStart => start,
                        _ => end,
                    };
                }
            }
            Motion::DocStart => {
                self.goal_x = None;
                self.caret = Caret {
                    block: 0,
                    offset: 0,
                };
            }
            Motion::DocEnd => {
                self.goal_x = None;
                let block = blocks - 1;
                self.caret = Caret {
                    block,
                    offset: self.block_len(block),
                };
            }
        }
    }

    /// One character left or right, rolling onto the neighbouring block at either end.
    fn stepped(&self, delta: i32) -> Caret {
        let mut caret = self.caret;
        if delta > 0 {
            if caret.offset < self.block_len(caret.block) {
                caret.offset += 1;
            } else if caret.block + 1 < self.core.block_count() {
                caret = Caret {
                    block: caret.block + 1,
                    offset: 0,
                };
            }
        } else if caret.offset > 0 {
            caret.offset -= 1;
        } else if caret.block > 0 {
            caret = Caret {
                block: caret.block - 1,
                offset: self.block_len(caret.block - 1),
            };
        }
        caret
    }

    fn block_len(&self, index: usize) -> usize {
        self.core
            .input_text(index)
            .map(|t| t.chars().count())
            .unwrap_or(0)
    }

    fn open_below(&mut self) {
        let kind = match self.kind_at(self.caret.block) {
            // A new paragraph under a heading, not another heading — the same rule
            // `App::split_block` follows at the end of one.
            Some(BlockKind::ListItem { depth }) => BlockKind::ListItem { depth },
            _ => BlockKind::Paragraph,
        };
        let at = self.caret.block + 1;
        match self.core.insert(at, kind, "") {
            Ok(()) => {
                self.caret = Caret {
                    block: at,
                    offset: 0,
                };
                self.mode = Mode::Insert;
                self.status.clear();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn kind_at(&self, index: usize) -> Option<BlockKind> {
        self.core
            .get_viewport(index..index + 1)
            .get(index)
            .map(|b| b.kind.clone())
    }

    /// `x`, and Delete in Insert mode: erase the character *at* the caret, joining the next
    /// block when there is no character left to erase.
    fn erase_forward(&mut self) {
        let to = match self.caret.offset < self.block_len(self.caret.block) {
            true => Caret {
                block: self.caret.block,
                offset: self.caret.offset + 1,
            },
            false if self.caret.block + 1 < self.core.block_count() => Caret {
                block: self.caret.block + 1,
                offset: 0,
            },
            false => return,
        };
        if let Err(e) = self.core.erase(self.caret, to) {
            self.status = e.to_string();
        }
    }

    /// Backspace: erase the character *before* the caret, and at the front of a block join it
    /// onto the one above — which is what `App::erase` across a boundary already does.
    fn erase_back(&mut self) {
        let from = self.stepped(-1);
        if from == self.caret {
            return;
        }
        match self.core.erase(from, self.caret) {
            Ok(_) => self.caret = from,
            Err(e) => self.status = e.to_string(),
        }
    }

    fn delete_block(&mut self) {
        let at = self.caret.block;
        match self.core.delete(at..at + 1) {
            Ok(_) => {
                let blocks = self.core.block_count();
                self.caret = Caret {
                    block: at.min(blocks.saturating_sub(1)),
                    offset: 0,
                };
                self.status.clear();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn join(&mut self) {
        let at = self.caret.block;
        // The caret lands where the seam was, which is where a person expects to carry on.
        let seam = self.block_len(at);
        match self.core.join_block(at) {
            Ok(()) => {
                self.caret = Caret {
                    block: at,
                    offset: seam,
                };
                self.status.clear();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn history(&mut self, changed: bool, when_nothing_happened: &str) {
        self.status = match changed {
            true => String::new(),
            false => when_nothing_happened.to_string(),
        };
        // History moves blocks around underneath the caret, so put it somewhere that exists.
        self.clamp_caret();
    }

    fn clamp_caret(&mut self) {
        let blocks = self.core.block_count();
        self.caret.block = self.caret.block.min(blocks.saturating_sub(1));
        self.caret.offset = self.caret.offset.min(self.block_len(self.caret.block));
    }

    // --- Insert mode ---

    fn on_insert_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char(c) => self.type_char(c),
            KeyCode::Tab => self.type_char('\t'),
            KeyCode::Backspace => self.erase_back(),
            KeyCode::Delete => self.erase_forward(),
            KeyCode::Enter => self.split(),
            KeyCode::Left => self.go(Motion::Char(-1)),
            KeyCode::Right => self.go(Motion::Char(1)),
            KeyCode::Up => self.go(Motion::Line(-1)),
            KeyCode::Down => self.go(Motion::Line(1)),
            KeyCode::Home => self.go(Motion::LineStart),
            KeyCode::End => self.go(Motion::LineEnd),
            // Esc leaves Insert and changes nothing: the text is already in the document, and
            // `u` is how it comes back out. Unlike a cell's staged edit, which has an honest
            // "never happened" to return to.
            KeyCode::Esc => {
                self.status.clear();
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    fn type_char(&mut self, c: char) {
        let mut text = [0u8; 4];
        match self.core.insert_text(self.caret, c.encode_utf8(&mut text)) {
            Ok(()) => {
                self.caret.offset += 1;
                self.goal_x = None;
                self.status.clear();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn split(&mut self) {
        match self.core.split_block(self.caret) {
            Ok(()) => {
                self.caret = Caret {
                    block: self.caret.block + 1,
                    offset: 0,
                };
                self.goal_x = None;
                self.status.clear();
            }
            Err(e) => self.status = e.to_string(),
        }
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
                if self.status.starts_with("wrote") {
                    self.quit = true;
                }
            }
            "outline" => self.cmd_outline(),
            "words" => self.cmd_words(),
            _ if cmd.starts_with("w ") => self.cmd_write(Some(cmd[2..].trim())),
            _ if cmd.starts_with("style ") => self.cmd_style(Some(cmd[6..].trim())),
            "style" => self.cmd_style(None),
            _ if cmd.starts_with("h ") => self.cmd_kind(cmd[2..].trim()),
            // Anything else is an address, vi's `:{line}` counterpart — and here it may be
            // `p12`, `#intro` or `§2.1`, which is the thing no word processor's UI offers.
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

    fn cmd_outline(&mut self) {
        let outline = self.core.outline();
        self.status = match outline.is_empty() {
            true => "no headings".to_string(),
            false => outline
                .iter()
                .map(|h| format!("{} {}", h.address(), h.text))
                .collect::<Vec<_>>()
                .join("   "),
        };
    }

    fn cmd_words(&mut self) {
        let c = self.core.counts();
        self.status = format!(
            "{} blocks  {} headings  {} words  {} characters",
            c.blocks, c.headings, c.words, c.characters
        );
    }

    fn cmd_style(&mut self, name: Option<&str>) {
        let at = self.caret.block;
        match self.core.set_style(
            at..at + 1,
            name.map(str::to_owned).filter(|s| !s.is_empty()),
        ) {
            Ok(_) => self.status.clear(),
            Err(e) => self.status = e.to_string(),
        }
    }

    /// `:h 2` makes the caret's block a level-2 heading; `:h 0` makes it a paragraph again.
    fn cmd_kind(&mut self, level: &str) {
        let kind = match level.parse::<u32>() {
            Ok(0) => BlockKind::Paragraph,
            Ok(level) => BlockKind::Heading { level },
            Err(_) => {
                self.status = format!("not an outline level: {level}");
                return;
            }
        };
        match self.core.set_kind(self.caret.block, kind) {
            Ok(()) => self.status.clear(),
            Err(e) => self.status = e.to_string(),
        }
    }

    fn cmd_jump(&mut self, addr: &str) {
        match grind_text::loc::parse(addr).map_err(|e| e.to_string()) {
            Ok(loc) => match self.core.resolve_caret(&loc) {
                Ok(caret) => {
                    self.caret = caret;
                    self.goal_x = None;
                    self.status.clear();
                }
                Err(e) => self.status = e.to_string(),
            },
            Err(e) => self.status = format!("not a command or address: {e}"),
        }
    }

    // --- Rendering ---

    /// Every document line from `top`, as far as the window needs — with the block each came
    /// from, so the gutter can mark where one starts.
    fn visible(&self, height: usize) -> Vec<(usize, usize, String)> {
        let blocks = self.core.block_count();
        let mut out = Vec::with_capacity(height);
        let mut block = self.top.0.min(blocks.saturating_sub(1));
        let mut skip = self.top.1;
        while block < blocks && out.len() < height {
            let text: Vec<char> = self
                .core
                .input_text(block)
                .unwrap_or_default()
                .chars()
                .collect();
            if let Ok(layout) = self.core.layout_block(block, self.width, &Cells) {
                for (n, line) in layout.lines().iter().enumerate().skip(skip) {
                    if out.len() == height {
                        break;
                    }
                    let piece: String = text[line.start..line.end].iter().collect();
                    out.push((block, n, piece.trim_end().to_string()));
                }
            }
            skip = 0;
            block += 1;
        }
        out
    }

    /// Slide `top` just far enough to keep the caret's line on screen.
    fn follow_caret(&mut self, height: usize) {
        let line = self
            .core
            .layout_block(self.caret.block, self.width, &Cells)
            .map(|l| l.line_at(self.caret.offset))
            .unwrap_or(0);
        let here = (self.caret.block, line);
        if here < self.top {
            self.top = here;
            return;
        }
        // Walk the window forward one line at a time until the caret is inside it. Bounded by
        // the document, and only ever a few steps in practice because the caret moves by one.
        while !self
            .visible(height)
            .iter()
            .any(|(b, n, _)| (*b, *n) == here)
        {
            let blocks = self.core.block_count();
            let lines = self
                .core
                .layout_block(self.top.0, self.width, &Cells)
                .map(|l| l.lines().len())
                .unwrap_or(1);
            self.top = match self.top.1 + 1 < lines {
                true => (self.top.0, self.top.1 + 1),
                false if self.top.0 + 1 < blocks => (self.top.0 + 1, 0),
                false => return,
            };
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let [body, status_area] =
            Rects::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

        self.width = f32::from(body.width.saturating_sub(GUTTER)).max(1.0);
        self.height = usize::from(body.height).max(1);
        self.clamp_caret();
        self.follow_caret(self.height);

        let caret_line = self
            .core
            .layout_block(self.caret.block, self.width, &Cells)
            .map(|l| (l.line_at(self.caret.offset), l.x_at(self.caret.offset)))
            .unwrap_or((0, 0.0));

        let mut lines = Vec::with_capacity(self.height);
        for (block, n, text) in self.visible(self.height) {
            // Only the first line of a block carries its mark, so a wrapped paragraph reads as
            // one paragraph.
            let mark = match n {
                0 => format!(
                    "{:<4}{:<3} ",
                    grind_text::loc::format(block),
                    self.kind_at(block)
                        .as_ref()
                        .map(describe_kind)
                        .unwrap_or_default()
                ),
                _ => " ".repeat(GUTTER as usize),
            };
            let mut spans = vec![Span::styled(
                mark,
                Style::default().add_modifier(Modifier::DIM),
            )];
            if (block, n) == (self.caret.block, caret_line.0) {
                // Split the line at the caret so the cell under it can be reversed — a real
                // cursor rather than a highlighted row.
                let at = (caret_line.1 as usize).min(text.chars().count());
                let before: String = text.chars().take(at).collect();
                let under: String = text.chars().skip(at).take(1).collect();
                let after: String = text.chars().skip(at + 1).collect();
                spans.push(Span::raw(before));
                spans.push(Span::styled(
                    match under.is_empty() {
                        true => " ".to_string(),
                        false => under,
                    },
                    Style::default().add_modifier(Modifier::REVERSED),
                ));
                spans.push(Span::raw(after));
            } else {
                spans.push(Span::raw(text));
            }
            lines.push(Line::from(spans));
        }
        frame.render_widget(Paragraph::new(lines), body);

        let status_text = match &self.mode {
            Mode::Command { buf } => format!(":{buf}"),
            Mode::Insert => format!(
                "-- INSERT --  {}  Esc normal",
                grind_text::loc::format_offset(self.caret.block, self.caret.offset)
            ),
            Mode::Normal if !self.status.is_empty() => self.status.clone(),
            Mode::Normal => format!(
                "{}  h j k l move  i/a/o insert  x erase  X delete  J join  u undo  : command",
                grind_text::loc::format_offset(self.caret.block, self.caret.offset)
            ),
        };
        frame.render_widget(
            Paragraph::new(status_text).style(Style::default().fg(Color::Black).bg(Color::Gray)),
            status_area,
        );
    }
}

fn describe_kind(kind: &BlockKind) -> String {
    match kind {
        BlockKind::Paragraph => "p".to_owned(),
        BlockKind::Heading { level } => format!("h{level}"),
        BlockKind::ListItem { depth } => format!("li{depth}"),
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

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// A shell over a document of `paragraphs`, with a window already measured — `draw` sets
    /// the width, and a test that never draws would wrap at the default.
    fn app(paragraphs: &[&str]) -> App {
        let core = Arc::new(CoreApp::new());
        for (i, text) in paragraphs.iter().enumerate() {
            core.insert(i, BlockKind::Paragraph, text).expect("inserts");
        }
        App::new(core, Arc::new(RedrawFlag::default()), None)
    }

    fn render(app: &mut App, width: u16, height: u16) -> Vec<String> {
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

    fn text(app: &App) -> String {
        app.core
            .get_viewport(0..app.core.block_count())
            .iter()
            .map(|b| b.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn typing_reaches_the_document_immediately() {
        let mut app = app(&["hello world"]);
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "oh ");
        assert_eq!(text(&app), "oh hello world");
        // Esc changes nothing — the text is already in, and `u` is how it comes out.
        press(&mut app, KeyCode::Esc);
        assert_eq!(text(&app), "oh hello world");
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('u'));
        }
        assert_eq!(text(&app), "hello world");
    }

    #[test]
    fn enter_splits_a_block_and_backspace_at_the_front_joins_it_back() {
        let mut app = app(&["one two"]);
        press(&mut app, KeyCode::Char('i'));
        for _ in 0..3 {
            press(&mut app, KeyCode::Right);
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(text(&app), "one\n two");
        assert_eq!(
            app.caret,
            Caret {
                block: 1,
                offset: 0
            }
        );

        press(&mut app, KeyCode::Backspace);
        assert_eq!(text(&app), "one two", "backspace at the front joins");
        assert_eq!(
            app.caret,
            Caret {
                block: 0,
                offset: 3
            }
        );
    }

    /// The test S8 exists for. `j` is not "the next block" — it is the next *line*, and the
    /// answer comes from the core, measured in terminal cells.
    #[test]
    fn j_moves_by_a_wrapped_line_not_by_a_block() {
        let mut app = app(&["the cat sat on the mat and then it slept"]);
        // A 28-cell window minus the gutter wraps this paragraph into several lines.
        render(&mut app, 28, 10);
        assert!(
            app.core
                .layout_block(0, app.width, &Cells)
                .unwrap()
                .lines()
                .len()
                > 1,
            "the fixture has to actually wrap or this proves nothing"
        );

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.caret.block, 0, "still inside the same paragraph");
        assert!(app.caret.offset > 0, "but further down it");
    }

    #[test]
    fn j_and_k_keep_the_goal_column_across_a_short_line() {
        let mut app = app(&["aaaaaaaa", "bb", "cccccccc"]);
        render(&mut app, 40, 10);
        press(&mut app, KeyCode::Char('$')); // end of the first line
        let want = app.caret.offset;
        press(&mut app, KeyCode::Char('j')); // onto "bb", which is shorter
        press(&mut app, KeyCode::Char('j')); // and on again
        assert_eq!(app.caret.block, 2);
        assert_eq!(
            app.caret.offset, want,
            "the column survived the short line in between"
        );
    }

    #[test]
    fn home_and_end_are_the_visual_line() {
        let mut app = app(&["the cat sat on the mat and then it slept"]);
        render(&mut app, 28, 10);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('0'));
        let start = app.caret.offset;
        assert!(start > 0, "the start of line 2, not of the paragraph");
        press(&mut app, KeyCode::Char('$'));
        assert!(app.caret.offset > start);
    }

    #[test]
    fn x_erases_a_character_and_capital_x_deletes_the_block() {
        let mut app = app(&["abc", "def"]);
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(text(&app), "bc\ndef");
        press(&mut app, KeyCode::Char('X'));
        assert_eq!(text(&app), "def");
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(text(&app), "bc\ndef");
    }

    #[test]
    fn o_opens_a_paragraph_below_and_starts_typing() {
        let mut app = app(&["first"]);
        press(&mut app, KeyCode::Char('o'));
        assert!(matches!(app.mode, Mode::Insert));
        type_str(&mut app, "second");
        assert_eq!(text(&app), "first\nsecond");
    }

    #[test]
    fn shift_j_joins_and_leaves_the_caret_at_the_seam() {
        let mut app = app(&["one", "two"]);
        press(&mut app, KeyCode::Char('J'));
        assert_eq!(text(&app), "onetwo");
        assert_eq!(
            app.caret,
            Caret {
                block: 0,
                offset: 3
            }
        );
    }

    /// The addressing no word processor's UI offers: `#intro` and `§2.1` survive edits above
    /// them, so `:` here is vi's `:{line}` with a memory.
    #[test]
    fn the_command_line_jumps_by_every_kind_of_address() {
        let mut app = app(&["Title", "body", "more"]);
        app.core
            .set_kind(0, BlockKind::Heading { level: 1 })
            .expect("heading");
        app.core.set_bookmark("here", Some(2)).expect("bookmark");

        for (address, block) in [("p2", 1), ("#here", 2), ("\u{a7}1", 0)] {
            press(&mut app, KeyCode::Char(':'));
            type_str(&mut app, address);
            press(&mut app, KeyCode::Enter);
            assert_eq!(app.caret.block, block, "{address}");
            assert!(matches!(app.mode, Mode::Normal));
        }
    }

    #[test]
    fn the_command_line_sets_a_heading_level_and_a_style() {
        let mut app = app(&["Title"]);
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "h 2");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.kind_at(0), Some(BlockKind::Heading { level: 2 }));

        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "style Quote");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.core.formatting().len(), 1);
    }

    #[test]
    fn quit_with_unsaved_changes_needs_a_bang() {
        let mut app = app(&["a"]);
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "q");
        press(&mut app, KeyCode::Enter);
        assert!(!app.should_quit(), "the insert that built it is unsaved");

        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "q!");
        press(&mut app, KeyCode::Enter);
        assert!(app.should_quit());
    }

    #[test]
    fn the_gutter_marks_where_a_block_starts_and_a_wrapped_line_is_not_one() {
        let mut app = app(&["the cat sat on the mat and then it slept", "next"]);
        let lines = render(&mut app, 28, 8);
        assert!(lines[0].starts_with("p1  p"), "{lines:?}");
        assert!(
            lines[1].starts_with("        "),
            "a continuation line carries no mark: {lines:?}"
        );
    }

    #[test]
    fn an_empty_document_draws_and_navigates_without_panicking() {
        let mut app = app(&[]);
        let _ = render(&mut app, 40, 6);
        for key in ['j', 'k', 'h', 'l', 'x', 'X', 'J', '0', '$', 'G', 'g'] {
            press(&mut app, KeyCode::Char(key));
        }
        let _ = render(&mut app, 40, 6);
    }

    #[test]
    fn the_caret_scrolls_into_view_in_a_short_window() {
        let paragraphs: Vec<String> = (0..40).map(|i| format!("line {i}")).collect();
        let refs: Vec<&str> = paragraphs.iter().map(String::as_str).collect();
        let mut app = app(&refs);
        render(&mut app, 40, 6);
        press(&mut app, KeyCode::Char('G'));
        let lines = render(&mut app, 40, 6);
        assert!(
            lines.iter().any(|l| l.contains("line 39")),
            "the last block should be on screen: {lines:?}"
        );
    }
}
