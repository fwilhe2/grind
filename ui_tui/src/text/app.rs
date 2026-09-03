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

use grind_text::style::CharStyle;
use grind_text::{App as CoreApp, BlockKind, BlockView, Caret};

use super::Cells;
use super::keymap::{self, Action, Motion};
use crate::app::RedrawFlag;
use grind_text::markdown::{self, Emphasis};

/// Room for `p12 h1 ` down the left, so a reader can see the structure the outline is made of.
const GUTTER: u16 = 8;

enum Mode {
    Normal,
    /// Selecting, from an anchor the caret is being dragged away from — vi's own Visual, and
    /// this shell's answer to Shift+arrow.
    Visual,
    Insert,
    Command {
        buf: String,
    },
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
    /// What [`grind_text::App::type_markdown`] said the next character must be set in — see
    /// there for why a notation needs it to end where its marker does. Handed straight back on
    /// the next keystroke and never read here.
    resume: Option<CharStyle>,
    /// Where a Visual-mode selection started. The caret is its other end.
    anchor: Option<Caret>,
    /// What `y` copied, as plain text. A register rather than the system clipboard: a
    /// terminal cannot reach one without a protocol the host may not speak, and vi's own
    /// register is the convention every reader of this shell already has.
    register: String,
    status: String,
    /// Whether the bookmark anchors are being drawn — `doc/view-modes.md` §3.6, and
    /// presentation state like everything else here, since it is a reading of the document
    /// rather than a change to it.
    names: bool,
    /// The key list, when it is showing. Presentation state like everything else here.
    help: crate::help::Help,
    /// The code view, when it is showing, and the projection it is showing (`doc/dsl.md` §6).
    ///
    /// Projected once when the pane opens and dropped when it closes, which §6.3's `ponytail`
    /// allows and which is exact here rather than approximate: the pane is read-only and any key
    /// that is not one of its motions closes it, so the document cannot change underneath it.
    code: crate::code::Code,
    source: Option<grind_text::projection::Projection>,
    /// `grind text lint`'s findings, when the pane is showing them (`doc/dsl.md` §4.3, D6) —
    /// the same pane the spreadsheet half uses, because a diagnostic is document-type-neutral.
    problems: crate::problems::Problems,
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
            caret: Caret {
                block: 0,
                offset: 0,
            },
            top: (0, 0),
            goal_x: None,
            width: 60.0,
            height: 20,
            mode: Mode::Normal,
            resume: None,
            anchor: None,
            register: String::new(),
            status: String::new(),
            names: false,
            help: crate::help::Help::default(),
            code: crate::code::Code::default(),
            source: None,
            problems: crate::problems::Problems::default(),
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
            let text = crate::text::help();
            self.help
                .on_key(key.code, text.lines().count(), self.help_height());
            return;
        }
        if self.code.is_open() {
            self.on_code_key(key.code, key.modifiers);
            return;
        }
        if self.problems.is_open() {
            self.on_problems_key(key.code);
            return;
        }
        match self.mode {
            Mode::Normal | Mode::Visual => self.on_normal_key(key.code, key.modifiers),
            Mode::Insert => self.on_insert_key(key.code),
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
            Action::Move(motion) => self.go(motion),
            Action::Insert => self.begin_insert(),
            Action::Append => {
                self.go(Motion::Char(1));
                self.begin_insert();
            }
            Action::OpenBelow => self.open_below(),
            // Over a selection the erase keys mean the selection, which is what makes `v`
            // worth having: `d` is "delete this", not "delete one character".
            Action::EraseChar => {
                if !self.erase_selection() {
                    self.erase_forward();
                }
            }
            Action::DeleteBlock => self.delete_block(),
            Action::Join => self.join(),
            Action::Undo => self.history(self.core.undo(), "nothing to undo"),
            Action::Redo => self.history(self.core.redo(), "nothing to redo"),
            Action::Command => self.mode = Mode::Command { buf: String::new() },
            Action::Visual => self.toggle_visual(),
            Action::Yank => self.yank(),
            Action::Put => self.put(),
            Action::Emphasise(emphasis) => self.emphasise_selection(emphasis),
            Action::Plain => self.set_selection_style(&CharStyle::default(), "plain"),
            Action::Escape => {
                self.anchor = None;
                self.status.clear();
                self.mode = Mode::Normal;
            }
        }
    }

    fn begin_insert(&mut self) {
        self.anchor = None;
        self.mode = Mode::Insert;
    }

    // --- Visual mode ---

    fn toggle_visual(&mut self) {
        match self.mode {
            Mode::Visual => {
                self.anchor = None;
                self.mode = Mode::Normal;
            }
            _ => {
                self.anchor = Some(self.caret);
                self.mode = Mode::Visual;
            }
        }
        self.status.clear();
    }

    /// The selection, in document order — `None` when the anchor is where the caret is, which
    /// is what "nothing selected" *is* rather than a second state to keep in step.
    pub fn selection(&self) -> Option<(Caret, Caret)> {
        let anchor = self.anchor?;
        if anchor == self.caret {
            return None;
        }
        Some(
            match (anchor.block, anchor.offset) <= (self.caret.block, self.caret.offset) {
                true => (anchor, self.caret),
                false => (self.caret, anchor),
            },
        )
    }

    /// Erase whatever is selected, leaving the caret where the selection started. `false` when
    /// there was nothing selected, so the caller can fall back to its one-character meaning.
    fn erase_selection(&mut self) -> bool {
        let Some((from, to)) = self.selection() else {
            return false;
        };
        match self.core.erase(from, to) {
            Ok(_) => {
                self.caret = from;
                self.anchor = None;
                self.mode = Mode::Normal;
                self.status.clear();
            }
            Err(e) => self.status = e.to_string(),
        }
        true
    }

    /// The selected text, as plain text — formatting is not carried, because a register that
    /// held it would be a second model of a run and this shell has none.
    fn selected_text(&self) -> Option<String> {
        let (from, to) = self.selection()?;
        let mut out = String::new();
        for index in from.block..=to.block {
            let chars: Vec<char> = self.core.input_text(index).ok()?.chars().collect();
            let start = match index == from.block {
                true => from.offset,
                false => 0,
            };
            let end = match index == to.block {
                true => to.offset,
                false => chars.len(),
            };
            if index > from.block {
                out.push('\n');
            }
            out.extend(&chars[start.min(chars.len())..end.min(chars.len())]);
        }
        Some(out)
    }

    fn yank(&mut self) {
        let Some((from, _)) = self.selection() else {
            self.status = "nothing selected — v starts a selection".to_string();
            return;
        };
        match self.selected_text() {
            Some(text) => {
                let count = text.chars().count();
                self.register = text;
                self.anchor = None;
                self.mode = Mode::Normal;
                // Where the selection started, which is vi's own answer and the place a `p`
                // straight afterwards would want to be.
                self.caret = from;
                self.goal_x = None;
                self.status = format!("yanked {count} character(s)");
            }
            None => self.status = "nothing selected — v starts a selection".to_string(),
        }
    }

    /// Put the register back at the caret, replacing a selection if there is one. A newline in
    /// it splits a block, since a block *is* the paragraph and there is no character for one.
    fn put(&mut self) {
        if self.register.is_empty() {
            self.status = "nothing yanked".to_string();
            return;
        }
        self.erase_selection();
        let register = std::mem::take(&mut self.register);
        for (index, piece) in register.split('\n').enumerate() {
            if index > 0 {
                self.split();
            }
            if !piece.is_empty() {
                self.insert_at_caret(piece);
            }
        }
        self.register = register;
        self.status.clear();
    }

    fn insert_at_caret(&mut self, text: &str) {
        match self.core.insert_text(self.caret, text) {
            Ok(()) => self.caret.offset += text.chars().count(),
            Err(e) => self.status = e.to_string(),
        }
    }

    // --- formatting ---

    /// Turn one emphasis on across the selection, or off when the whole of it already has it —
    /// `App::char_style` reports only what a span *agrees* about, which is exactly the question
    /// a toggle asks.
    fn emphasise_selection(&mut self, emphasis: Emphasis) {
        let Some((from, to)) = self.selection() else {
            self.status = "nothing selected — v starts a selection".to_string();
            return;
        };
        let mut style = self.core.char_style(from, to).unwrap_or_default();
        let wanted = emphasis.style();
        let field = |style: &CharStyle| match emphasis {
            Emphasis::Bold => style.font_weight.clone(),
            Emphasis::Italic => style.font_style.clone(),
            Emphasis::Underline => style.underline.clone(),
            Emphasis::Strike => style.line_through.clone(),
            Emphasis::Code => style.font_family.clone(),
        };
        // The four switches have an explicit "off" the document can hold; a *family* does not
        // — the way to have none is to have none, which is `None`.
        let off = match emphasis {
            Emphasis::Bold | Emphasis::Italic => Some("normal"),
            Emphasis::Underline | Emphasis::Strike => Some("none"),
            Emphasis::Code => None,
        };
        let already = match off {
            Some(off) => field(&style).as_deref().is_some_and(|v| v != off),
            None => field(&style).is_some(),
        };
        let value = match already {
            true => off.map(str::to_owned),
            false => field(&wanted),
        };
        match emphasis {
            Emphasis::Bold => style.font_weight = value,
            Emphasis::Italic => style.font_style = value,
            Emphasis::Underline => style.underline = value,
            Emphasis::Strike => style.line_through = value,
            Emphasis::Code => style.font_family = value,
        }
        self.set_selection_style(&style, emphasis.markers());
    }

    fn set_selection_style(&mut self, style: &CharStyle, what: &str) {
        let Some((from, to)) = self.selection() else {
            self.status = "nothing selected — v starts a selection".to_string();
            return;
        };
        match self.core.set_char_style(from, to, style) {
            Ok(_) => {
                self.status = format!("{what} over {} character(s)", span_len(from, to));
                self.anchor = None;
                self.mode = Mode::Normal;
            }
            Err(e) => self.status = e.to_string(),
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
        // **One core call, one undo step.** The notation, the erasing of the markers and the
        // formatting are `App::type_markdown`'s (`grind_text::markdown`), so this shell and
        // the two windows read `**` the same way and none of them has its own idea of it.
        match self
            .core
            .type_markdown(self.caret, &c.to_string(), self.resume.as_ref())
        {
            Ok(typed) => {
                self.caret = typed.caret;
                self.resume = typed.resume;
                self.goal_x = None;
                self.status.clear();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn split(&mut self) {
        // A code block continues over Enter with nothing done here: `App::split_block` already
        // carries a named paragraph style to the second block, so a run of preformatted
        // paragraphs is what a fence opens and `` ``` `` is what ends it.
        match self.core.split_block(self.caret) {
            Ok(()) => {
                self.caret = Caret {
                    block: self.caret.block + 1,
                    offset: 0,
                };
                self.resume = None;
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
            "help" | "h?" => self.help.open(),
            "q" => self.cmd_quit(false),
            "q!" => self.cmd_quit(true),
            "w" => self.cmd_write(None),
            "wq" | "x" => {
                self.cmd_write(None);
                if self.status.starts_with("wrote") {
                    self.quit = true;
                }
            }
            "source" => self.cmd_source(),
            "lint" => self.cmd_lint(false),
            "lint hints" | "lint!" => self.cmd_lint(true),
            "outline" => self.cmd_outline(),
            // The word processor's half of inline names (§3.6). A verb rather than a key,
            // and the same word turns it off: nothing is written either way.
            "names" => {
                self.names = !self.names;
                self.status = match self.names {
                    true => "names on — :names to turn it off".to_string(),
                    false => "names off".to_string(),
                };
            }
            "words" => self.cmd_words(),
            "plain" => self.set_selection_style(&CharStyle::default(), "plain"),
            _ if cmd.starts_with("find ") => self.cmd_find(cmd[5..].trim()),
            // vi's own substitution, and the one command here that is a *document* edit rather
            // than a caret move: `App::replace` changes every match, which is what `/g` means
            // and the only thing this build's core offers.
            _ if cmd.starts_with("s/") => self.cmd_substitute(&cmd[2..]),
            _ if cmd.starts_with("color ") => self.cmd_color(cmd[6..].trim(), false),
            _ if cmd.starts_with("highlight ") => self.cmd_color(cmd[10..].trim(), true),
            _ if cmd.starts_with("li") => self.cmd_list(cmd[2..].trim()),
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

    /// Where a piece of text is — the count, and the caret on the first one.
    fn cmd_find(&mut self, needle: &str) {
        let found = self.core.find(needle);
        match found.first() {
            Some(first) => {
                self.caret = Caret {
                    block: first.index,
                    offset: first.offset,
                };
                self.goal_x = None;
                self.status = format!("{} match(es) — {}", found.len(), first.address());
            }
            None => self.status = format!("no match for {needle}"),
        }
    }

    /// `:s/old/new/` — every occurrence, one undo step, because that is what `App::replace` is.
    fn cmd_substitute(&mut self, rest: &str) {
        let mut parts = rest.splitn(2, '/');
        let (Some(needle), Some(with)) = (parts.next(), parts.next()) else {
            self.status = "usage: :s/old/new/".to_string();
            return;
        };
        let with = with.strip_suffix('/').unwrap_or(with);
        if needle.is_empty() {
            self.status = "usage: :s/old/new/".to_string();
            return;
        }
        match self.core.replace(needle, with) {
            Ok(0) => self.status = format!("no match for {needle}"),
            Ok(n) => {
                self.clamp_caret();
                self.status = format!("replaced {n}");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// A colour over the selection, by the core's own palette name or an `#rrggbb` — the same
    /// vocabulary `grind text format --color` takes, so a swatch in a window and a word here
    /// are the same attribute.
    fn cmd_color(&mut self, name: &str, background: bool) {
        let Some((from, to)) = self.selection() else {
            self.status = "nothing selected — v starts a selection".to_string();
            return;
        };
        let value = match name {
            "" | "none" | "default" => None,
            name => match grind_core::style::palette(name) {
                Some(hex) => Some(hex.to_owned()),
                None if name.starts_with('#') => Some(name.to_owned()),
                None => {
                    self.status = format!("not a colour: {name}");
                    return;
                }
            },
        };
        let mut style = self.core.char_style(from, to).unwrap_or_default();
        match background {
            true => style.background = value,
            false => style.color = value,
        }
        self.set_selection_style(&style, name);
    }

    /// `:li` makes the caret's block a list item, `:li 2` nests it one level deeper.
    fn cmd_list(&mut self, depth: &str) {
        let depth = match depth.is_empty() {
            true => 1,
            false => match depth.parse::<u32>() {
                Ok(depth) if depth >= 1 => depth,
                _ => {
                    self.status = format!("not a list depth: {depth}");
                    return;
                }
            },
        };
        match self
            .core
            .set_kind(self.caret.block, BlockKind::ListItem { depth })
        {
            Ok(()) => self.status.clear(),
            Err(e) => self.status = e.to_string(),
        }
    }

    // --- the code view (doc/dsl.md §6, D9) ---

    /// `:source` — the document as its projection, with the cursor on the block the caret is in.
    ///
    /// A `:` command for the reason `:names` is one: it is a *mode*, and this shell's keys are
    /// vi's motions. `doc/tui-shell.md`'s second decision rules out drawing markdown markers
    /// *inline*, and this is not that — it is a separate pane showing a different notation, which
    /// is exactly how a source view avoids the problem that rules the inline one out.
    fn cmd_source(&mut self) {
        let projection = self.core.project();
        // `p12` — the address every block has, whatever else it answers to.
        self.code.open(
            &projection,
            Some(&grind_text::loc::format(self.caret.block)),
        );
        self.source = Some(projection);
    }

    /// A key while the code view is open. Moving puts the caret in the block that line projects,
    /// which is §6.2's map in the direction that makes the pane worth having.
    fn on_code_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        let Some(projection) = self.source.take() else {
            self.code.close();
            return;
        };
        let height = self.help_height();
        let nav = match (code, mods.contains(KeyModifiers::CONTROL)) {
            (KeyCode::Char('f'), true) => self.code.page(true, &projection, height),
            (KeyCode::Char('b'), true) => self.code.page(false, &projection, height),
            _ => self.code.on_key(code, &projection, height),
        };
        if nav == crate::code::Nav::Closed {
            self.status.clear();
            return;
        }
        // A block answers to `p12`, `#intro` and `§2.1.3` alike, and `loc::parse` takes all
        // three — so whichever spelling the span map hands back resolves, and this needs no
        // vocabulary of its own.
        if nav == crate::code::Nav::Moved
            && let Some(address) = self.code.address(&projection)
            && let Ok(caret) = grind_text::loc::parse(address)
                .map_err(|e| e.to_string())
                .and_then(|loc| self.core.resolve_caret(&loc).map_err(|e| e.to_string()))
        {
            self.caret = caret;
            self.anchor = None;
            self.goal_x = None;
        }
        self.source = Some(projection);
    }

    /// `:lint` — check the document and show what it says about itself (`doc/dsl.md` §4.3).
    fn cmd_lint(&mut self, hints: bool) {
        let report = self.core.lint(&grind_text::lint::Options {
            hints,
            off: Vec::new(),
        });
        self.status = match report.is_empty() {
            true => "no problems found".to_owned(),
            false => format!("{} finding(s) — Enter goes to one", report.len()),
        };
        self.problems.open(report);
    }

    /// A key while the problems pane is open. Enter puts the caret where the finding is,
    /// through `cmd_jump` — the same one `:p12` and the go-to box use, so a diagnostic's
    /// address is an address like any other.
    fn on_problems_key(&mut self, code: KeyCode) {
        let height = self.help_height();
        if let crate::problems::Nav::Chose(address) = self.problems.on_key(code, height) {
            self.cmd_jump(&address);
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

    /// Every document line from `top`, as far as the window needs — the block it came from,
    /// which line of that block it is, and the characters it covers.
    ///
    /// Offsets rather than a `String`, because what is drawn is not one piece of text: a run
    /// of it may be bold, part of it may be selected, and the caret sits between two
    /// characters. [`App::draw`] cuts it up; this only says where the line is.
    fn visible(&self, height: usize) -> Vec<(usize, usize, std::ops::Range<usize>)> {
        let blocks = self.core.block_count();
        let mut out = Vec::with_capacity(height);
        let mut block = self.top.0.min(blocks.saturating_sub(1));
        let mut skip = self.top.1;
        while block < blocks && out.len() < height {
            if let Ok(layout) = self.core.layout_block(block, self.width, &Cells) {
                for (n, line) in layout.lines().iter().enumerate().skip(skip) {
                    if out.len() == height {
                        break;
                    }
                    out.push((block, n, line.start..line.end));
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

    /// One laid-out line, cut into the pieces the terminal can draw it as.
    ///
    /// Three things change part-way along a line and none of them lines up with the others:
    /// the document's own **formatting**, the **selection**, and the **caret**. Each is a set
    /// of boundaries in the block's character offsets, and a piece is what falls between two
    /// adjacent ones — the same cut `ui_web/src/text/runs.rs` makes for the same reason, in a
    /// different toolkit.
    ///
    /// **A terminal draws formatting, it does not spell it.** Bold is bold, not `**bold**`:
    /// markers on screen would be characters the core never measured, and every caret after
    /// one would sit in the wrong column. The markers are for *typing* (`markdown.rs`).
    fn line_spans(
        &self,
        view: &BlockView,
        line: std::ops::Range<usize>,
        selection: &Option<(Caret, Caret)>,
        caret: Option<usize>,
    ) -> Vec<Span<'static>> {
        let chars: Vec<char> = view.text.chars().collect();
        // The selection, clipped to this block — it may start pages above and end below.
        let within = selection.as_ref().and_then(|(from, to)| {
            (from.block <= view.index && view.index <= to.block).then(|| {
                let start = match from.block == view.index {
                    true => from.offset,
                    false => 0,
                };
                let end = match to.block == view.index {
                    true => to.offset,
                    false => chars.len(),
                };
                start..end
            })
        });

        let mut bounds = vec![line.start, line.end];
        let mut mark = |at: usize| {
            if at > line.start && at < line.end {
                bounds.push(at);
            }
        };
        for run in &view.runs {
            mark(run.start);
            mark(run.start + run.text.chars().count());
        }
        if let Some(range) = &within {
            mark(range.start);
            mark(range.end);
        }
        if let Some(caret) = caret {
            mark(caret);
            mark(caret + 1);
        }
        bounds.sort_unstable();
        bounds.dedup();

        let heading = matches!(view.kind, BlockKind::Heading { .. })
            || matches!(view.style.as_deref(), Some("Title" | "Subtitle"));
        // A whole block of code is drawn the way a run of it is — the fence set a paragraph
        // style, and what it means is "all of this is code".
        let block_code = view.style.as_deref() == Some(markdown::PREFORMATTED);
        let mut spans = Vec::new();
        let mut drawn_caret = false;
        for pair in bounds.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            let run = view.runs.iter().find(|run| {
                (run.start..run.start + run.text.chars().count().max(1)).contains(&start)
            });
            let piece: String = chars[start.min(chars.len())..end.min(chars.len())]
                .iter()
                .collect();
            // A line break ends the line; it is not a character to draw.
            let piece = piece.trim_end_matches('\n').to_string();
            let selected = within
                .as_ref()
                .is_some_and(|sel| sel.start <= start && end <= sel.end);
            let under_caret = caret == Some(start);
            drawn_caret |= under_caret;
            let mut style = run
                .map(|run| terminal_style(&run.props))
                .unwrap_or_default();
            if heading {
                style = style.add_modifier(Modifier::BOLD);
            }
            if block_code {
                style = style.add_modifier(Modifier::DIM);
            }
            if selected || under_caret {
                style = style.add_modifier(Modifier::REVERSED);
            }
            spans.push(Span::styled(piece, style));
        }
        // At the end of a line, and in an empty one, the caret has no character to sit on.
        if caret.is_some() && !drawn_caret {
            spans.push(Span::styled(
                " ".to_string(),
                Style::default().add_modifier(Modifier::REVERSED),
            ));
        }
        spans
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
            self.help.draw(frame, area, &crate::text::help());
            return;
        }
        if self.problems.is_open() {
            let title = self
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "untitled".to_owned());
            self.problems.draw(frame, area, &title);
            return;
        }
        if let Some(projection) = self.source.take() {
            let title = self
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "untitled".to_owned());
            self.code.draw(frame, area, &projection, &title);
            self.source = Some(projection);
            return;
        }
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

        let selection = self.selection();
        let mut lines = Vec::with_capacity(self.height);
        // One viewport read per *block* rather than per line: a wrapped paragraph is several
        // rows and they all draw from the same runs.
        let mut current: Option<(usize, BlockView)> = None;
        for (block, n, range) in self.visible(self.height) {
            if current.as_ref().is_none_or(|(at, _)| *at != block) {
                current = self
                    .core
                    .get_viewport(block..block + 1)
                    .get(block)
                    .cloned()
                    .map(|view| (block, view));
            }
            let Some((_, view)) = &current else { continue };

            // Only the first line of a block carries its mark, so a wrapped paragraph reads as
            // one paragraph.
            let mark = match n {
                0 => format!(
                    "{:<4}{:<3} ",
                    grind_text::loc::format(block),
                    describe_block(&view.kind, view.style.as_deref())
                ),
                _ => " ".repeat(GUTTER as usize),
            };
            let mut spans = vec![Span::styled(
                mark,
                Style::default().add_modifier(Modifier::DIM),
            )];
            let caret =
                ((block, n) == (self.caret.block, caret_line.0)).then_some(self.caret.offset);
            spans.extend(self.line_spans(view, range, &selection, caret));
            // `doc/view-modes.md` §3.6: a bookmark is the named-range analogue and it is the
            // one part of a text document a reader cannot see at all — it contributes no
            // characters. With `:names` on, the block that holds one says so, after its
            // text rather than inside it, because an offset inside the line is an offset the
            // caret counts and a mark drawn there would move it.
            if self.names && n == 0 && !view.marks.is_empty() {
                let marks: Vec<String> = view
                    .marks
                    .iter()
                    .map(|(at, name)| format!("\u{2039}{name}\u{203a}+{at}"))
                    .collect();
                spans.push(Span::styled(
                    format!("  {}", marks.join(" ")),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
            lines.push(Line::from(spans));
        }
        frame.render_widget(Paragraph::new(lines), body);

        let where_ = grind_text::loc::format_offset(self.caret.block, self.caret.offset);
        let status_text = match &self.mode {
            Mode::Command { buf } => format!(":{buf}"),
            Mode::Insert => format!(
                "-- INSERT --  {where_}  **bold** *italic* __under__ ~~struck~~  # heading  - list"
            ),
            Mode::Visual => format!(
                "-- VISUAL --  {} selected  * bold  / italic  _ under  ~ struck  - plain  y yank  d delete",
                self.selection()
                    .map(|(from, to)| span_len(from, to))
                    .unwrap_or(0)
            ),
            _ if !self.status.is_empty() => self.status.clone(),
            _ => format!(
                "{where_}  h j k l move  i/a/o insert  v select  x erase  X delete  J join  u undo  : command"
            ),
        };
        frame.render_widget(
            Paragraph::new(status_text).style(Style::default().fg(Color::Black).bg(Color::Gray)),
            status_area,
        );
    }
}

/// How many characters a selection covers, for the status line. Across blocks it counts the
/// boundaries as one character each, which is what erasing the same span would take out.
fn span_len(from: Caret, to: Caret) -> usize {
    match from.block == to.block {
        true => to.offset.saturating_sub(from.offset),
        // Only the two ends are known here without reading every block between them; a
        // rough count is what a status line is for.
        false => to.block - from.block + to.offset,
    }
}

/// A run's own formatting, as the attributes a terminal has.
///
/// Four of the eight `CharStyle` properties land here; family and size have no meaning in a
/// grid of one font at one size (`Cells`' own note), and that is a limit of the medium rather
/// than a gap in the shell. A colour is offered as one of the sixteen the terminal names,
/// nearest by hue — a document's `#ff4136` is red here, which is the honest answer.
fn terminal_style(props: &CharStyle) -> Style {
    let on = |value: &Option<String>, off: &str| value.as_deref().is_some_and(|v| v != off);
    let mut style = Style::default();
    if on(&props.font_weight, "normal") {
        style = style.add_modifier(Modifier::BOLD);
    }
    if on(&props.font_style, "normal") {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if on(&props.underline, "none") {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if on(&props.line_through, "none") {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    // **A terminal has one font**, so a monospace run cannot be drawn as a different one —
    // everything here already is monospace. It is dimmed instead, and SGR 2 is optional, so on
    // a terminal or a theme that ignores it a `` `code` `` run is *invisible*. That was left
    // open once; it is decided now, and the decision is to keep DIM and say what it costs,
    // because every alternative is worse:
    //
    // * a colour or a background would be the *document's* — three lines below, a run's own
    //   `fo:color` becomes exactly that, so a shell-chosen one would be indistinguishable from
    //   a document-chosen one and would overwrite it where a run had both;
    // * bold, italic, underline and strikethrough are each already a property of the run;
    // * reverse video is the selection and the caret;
    // * a marker in the line (`` ` ``) is a character the core never measured, which puts every
    //   caret after it in the wrong column — `doc/tui-shell.md`'s decision 2, and the reason
    //   markdown is for typing and never for showing.
    //
    // The *block* half does not depend on SGR 2 at all: a fenced block says `pre` in the
    // gutter (`describe_block`), which is plain text every terminal draws. There is no such
    // place for a run inside a line, and `doc/tui-shell.md` says so rather than implying that
    // dimming always shows.
    if props.font_family.is_some() {
        style = style.add_modifier(Modifier::DIM);
    }
    if let Some(color) = props.color.as_deref().and_then(nearest_color) {
        style = style.fg(color);
    }
    if let Some(color) = props.background.as_deref().and_then(nearest_color) {
        style = style.bg(color);
    }
    style
}

/// The terminal colour nearest an ODF `#rrggbb`, by squared distance in RGB.
///
/// Eight hues and their bright halves — the palette every terminal has, rather than the 256
/// some do and the true colour others do: a shell that assumed more would look wrong on the
/// terminals that have less, and the point of a colour here is that it is *distinguishable*.
pub fn nearest_color(hex: &str) -> Option<Color> {
    let hex = hex.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| {
        u8::from_str_radix(hex.get(at..at + 2)?, 16)
            .ok()
            .map(i32::from)
    };
    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    const TERMINAL: [(Color, (i32, i32, i32)); 16] = [
        (Color::Black, (0, 0, 0)),
        (Color::Red, (170, 0, 0)),
        (Color::Green, (0, 170, 0)),
        (Color::Yellow, (170, 85, 0)),
        (Color::Blue, (0, 0, 170)),
        (Color::Magenta, (170, 0, 170)),
        (Color::Cyan, (0, 170, 170)),
        (Color::Gray, (170, 170, 170)),
        (Color::DarkGray, (85, 85, 85)),
        (Color::LightRed, (255, 85, 85)),
        (Color::LightGreen, (85, 255, 85)),
        (Color::LightYellow, (255, 255, 85)),
        (Color::LightBlue, (85, 85, 255)),
        (Color::LightMagenta, (255, 85, 255)),
        (Color::LightCyan, (85, 255, 255)),
        (Color::White, (255, 255, 255)),
    ];
    TERMINAL
        .iter()
        .min_by_key(|(_, (tr, tg, tb))| (r - tr).pow(2) + (g - tg).pow(2) + (b - tb).pow(2))
        .map(|(color, _)| *color)
}

/// What the gutter calls a block: its kind, or the one named paragraph style this shell has a
/// spelling for.
///
/// **The gutter is where a code *block* is visible in a terminal**, and deliberately so. A
/// fence (```) sets a paragraph style and changes no character of the text, so the only two
/// places to show it are inside the line — where a marker would be a character the core never
/// measured, putting every caret after it in the wrong column (`doc/tui-shell.md`, decision 2)
/// — and outside it, where this shell already writes each block's address and kind. Three
/// letters in the gutter are plain characters that every terminal draws, which is more than
/// can be said for the `Modifier::DIM` the block also carries (`terminal_style`).
///
/// `Title` and `Subtitle` get no spelling of their own: they are drawn bold, which is a
/// distinction the reader can already see in the text, and the gutter has three columns.
fn describe_block(kind: &BlockKind, style: Option<&str>) -> String {
    if style == Some(markdown::PREFORMATTED) {
        return "pre".to_owned();
    }
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

    /// **D9 in this shell.** `:source` shows the projection, opens on the block the caret is in,
    /// and moving in it puts the caret in the block that line projects.
    #[test]
    fn the_code_view_shows_the_source_and_moving_in_it_moves_the_caret() {
        let mut app = app(&["first", "second", "third"]);
        app.caret = Caret {
            block: 2,
            offset: 0,
        };
        let before = render(&mut app, 46, 10);

        app.run_command("source");
        let shown = render(&mut app, 46, 10).join("\n");
        assert!(shown.contains("— source"), "{shown}");
        assert!(shown.contains("p \"third\""), "the projection: {shown}");
        assert!(
            shown.contains("p3"),
            "opened on the caret's own block, named the way every block is: {shown}"
        );

        press(&mut app, KeyCode::Char('k'));
        assert!(
            render(&mut app, 46, 10).join("\n").contains("p2"),
            "the pane says which block this line is"
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.caret.block, 1, "and the caret went there");
        assert_eq!(
            render(&mut app, 46, 10)[0],
            before[0],
            "closing puts the document back"
        );
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

    /// `doc/view-modes.md` §3.6 in this shell. A bookmark contributes no characters, so
    /// without the mode there is nothing on screen to say it exists — and with it on, the
    /// text is untouched, because the mark goes after the line rather than into it.
    #[test]
    fn a_bookmark_is_invisible_until_the_name_mode_says_where_it_is() {
        let mut app = app(&["Introduction"]);
        app.core.set_bookmark("intro", Some(0)).unwrap();
        let plain = render(&mut app, 40, 6);
        assert!(!plain[0].contains("intro"), "{:?}", plain[0]);

        app.run_command("names");
        let shown = render(&mut app, 40, 6);
        assert!(shown[0].contains("intro"), "{:?}", shown[0]);
        assert!(shown[0].contains("Introduction"), "the text yielded");
        assert_eq!(text(&app), "Introduction", "a reading changed the document");

        app.run_command("names");
        assert_eq!(render(&mut app, 40, 6)[0], plain[0]);
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

    /// The markdown ask, end to end: the markers are typed as ordinary characters and are
    /// gone by the time the closing one lands, leaving the *document* formatted.
    #[test]
    fn typing_markdown_formats_the_span_and_takes_the_markers_back_out() {
        for (typed, want, check) in [
            ("**bold**", "bold", "font_weight"),
            ("*slant*", "slant", "font_style"),
            ("__under__", "under", "underline"),
            ("~~struck~~", "struck", "line_through"),
        ] {
            let mut app = app(&[""]);
            press(&mut app, KeyCode::Char('i'));
            type_str(&mut app, typed);
            assert_eq!(text(&app), want, "{typed}: the markers are gone");

            let view = app.core.get_viewport(0..1);
            let block = view.get(0).expect("the block");
            let props = &block.runs.first().expect("one run").props;
            let set = match check {
                "font_weight" => props.font_weight.is_some(),
                "font_style" => props.font_style.is_some(),
                "underline" => props.underline.is_some(),
                _ => props.line_through.is_some(),
            };
            assert!(set, "{typed}: the span carries {check} — {props:?}");
        }
    }

    /// Backticks, both halves: `` `code` `` is a monospace run, and ``` is a code *block* —
    /// a named paragraph style, which is what ODF has for one.
    #[test]
    fn backticks_make_code_inline_and_a_fence_makes_a_block() {
        let mut inline = app(&[""]);
        press(&mut inline, KeyCode::Char('i'));
        type_str(&mut inline, "run `ls -l` first");
        assert_eq!(text(&inline), "run ls -l first", "the backticks are gone");
        let view = inline.core.get_viewport(0..1);
        let code = view
            .get(0)
            .expect("the block")
            .runs
            .iter()
            .find(|run| run.props.font_family.is_some())
            .expect("a monospace run");
        assert_eq!(code.text, "ls -l");
        assert_eq!(code.props.font_family.as_deref(), Some(markdown::MONOSPACE));

        // A fence turns the block into a code paragraph, and Enter keeps it that way.
        let mut fenced = app(&[""]);
        press(&mut fenced, KeyCode::Char('i'));
        type_str(&mut fenced, "```");
        let preformatted = |app: &App, index: usize| {
            app.core
                .get_viewport(index..index + 1)
                .get(index)
                .and_then(|block| block.style.clone())
                .as_deref()
                == Some(markdown::PREFORMATTED)
        };
        assert!(preformatted(&fenced, 0), "the fence opened a code block");
        assert_eq!(text(&fenced), "", "and took its own markers back out");
        type_str(&mut fenced, "one");
        press(&mut fenced, KeyCode::Enter);
        type_str(&mut fenced, "two");
        assert!(preformatted(&fenced, 1), "Enter continues the block");

        // And a second fence ends it.
        press(&mut fenced, KeyCode::Enter);
        type_str(&mut fenced, "```");
        assert!(!preformatted(&fenced, 2), "the closing fence ends it");
    }

    /// The bug the backtick test found, pinned for all five notations: a character typed
    /// after a closing marker joins the run that was just emphasised unless something stops
    /// it, so `say **this** and` would carry on bold past the marker that ended it.
    #[test]
    fn typing_after_a_closing_marker_is_not_emphasised() {
        for typed in [
            "**b** tail",
            "*i* tail",
            "__u__ tail",
            "~~s~~ tail",
            "`c` tail",
        ] {
            let mut app = app(&[""]);
            press(&mut app, KeyCode::Char('i'));
            type_str(&mut app, typed);
            let view = app.core.get_viewport(0..1);
            let block = view.get(0).expect("the block");
            let last = block.runs.last().expect("a run");
            assert!(
                last.props.is_plain(),
                "{typed}: the tail is plain — {:?}",
                last.props
            );
            assert!(
                last.text.ends_with("tail"),
                "{typed}: and it is the tail — {:?}",
                last.text
            );
        }
    }

    /// And the rules that keep prose out of it hold through the shell, not just in the
    /// notation: `2*3*4` is arithmetic and stays as it was typed.
    #[test]
    fn typing_arithmetic_is_not_formatting() {
        let mut app = app(&[""]);
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "2*3*4");
        assert_eq!(text(&app), "2*3*4");
        let view = app.core.get_viewport(0..1);
        assert!(!view.get(0).expect("the block").styled);
    }

    /// The block half of the same notation.
    #[test]
    fn typing_a_hash_makes_the_block_a_heading_and_a_dash_a_list_item() {
        let mut heading = app(&["title"]);
        press(&mut heading, KeyCode::Char('i'));
        type_str(&mut heading, "## ");
        assert_eq!(text(&heading), "title", "the prefix is gone");
        assert_eq!(heading.kind_at(0), Some(BlockKind::Heading { level: 2 }));

        let mut item = app(&["item"]);
        press(&mut item, KeyCode::Char('i'));
        type_str(&mut item, "- ");
        assert_eq!(item.kind_at(0), Some(BlockKind::ListItem { depth: 1 }));
    }

    /// Visual mode is the terminal's Shift+arrow: an anchor, a caret, and every verb that
    /// needs a range.
    #[test]
    fn visual_mode_selects_and_the_marker_keys_format_what_is_selected() {
        let mut app = app(&["hello world"]);
        render(&mut app, 40, 6);
        press(&mut app, KeyCode::Char('v'));
        for _ in 0..5 {
            press(&mut app, KeyCode::Char('l'));
        }
        assert_eq!(
            app.selection(),
            Some((
                Caret {
                    block: 0,
                    offset: 0
                },
                Caret {
                    block: 0,
                    offset: 5
                }
            ))
        );

        press(&mut app, KeyCode::Char('*'));
        assert!(matches!(app.mode, Mode::Normal), "the selection is spent");
        let view = app.core.get_viewport(0..1);
        let runs = &view.get(0).expect("the block").runs;
        assert_eq!(runs[0].text, "hello");
        assert_eq!(runs[0].props.font_weight.as_deref(), Some("bold"));
        assert!(runs[1].props.font_weight.is_none(), "and only that far");
    }

    #[test]
    fn a_selection_yanks_and_puts_and_deletes() {
        let mut app = app(&["hello world"]);
        render(&mut app, 40, 6);
        press(&mut app, KeyCode::Char('v'));
        for _ in 0..5 {
            press(&mut app, KeyCode::Char('l'));
        }
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.register, "hello");

        // `d` over a selection deletes the selection, not one character.
        press(&mut app, KeyCode::Char('v'));
        for _ in 0..5 {
            press(&mut app, KeyCode::Char('l'));
        }
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(text(&app), " world");

        press(&mut app, KeyCode::Char('p'));
        assert_eq!(text(&app), "hello world", "the register puts it back");
    }

    /// Formatting is *drawn*, not spelled: the terminal's own bold, with no markers on screen
    /// to shift every caret after them.
    #[test]
    fn a_bold_run_is_drawn_bold_and_no_markers_are_shown() {
        let mut app = app(&["hello world"]);
        render(&mut app, 40, 6);
        app.core
            .set_char_style(
                Caret {
                    block: 0,
                    offset: 0,
                },
                Caret {
                    block: 0,
                    offset: 5,
                },
                &Emphasis::Bold.style(),
            )
            .expect("bold");

        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..40)
            .map(|c| buffer[(c, 0)].symbol().to_string())
            .collect();
        assert!(row.contains("hello world"), "{row:?}");
        assert!(!row.contains('*'), "no markers on screen: {row:?}");

        // The cell under "e" — past the gutter, past the caret's own reversed "h".
        let bold = buffer[(GUTTER + 1, 0)].style();
        assert!(
            bold.add_modifier.contains(Modifier::BOLD),
            "the run draws bold: {bold:?}"
        );
        let plain = buffer[(GUTTER + 7, 0)].style();
        assert!(!plain.add_modifier.contains(Modifier::BOLD), "{plain:?}");
    }

    #[test]
    fn the_command_line_finds_and_substitutes() {
        let mut app = app(&["one two", "two three"]);
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "find two");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.caret.block, 0);
        assert!(app.status.starts_with("2 match"), "{}", app.status);

        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "s/two/2/");
        press(&mut app, KeyCode::Enter);
        assert_eq!(text(&app), "one 2\n2 three");
    }

    /// A colour goes through the core's own palette, so `red` here and a swatch in a window
    /// are the same attribute.
    #[test]
    fn the_command_line_colours_a_selection() {
        let mut app = app(&["hello"]);
        render(&mut app, 40, 6);
        press(&mut app, KeyCode::Char('v'));
        for _ in 0..5 {
            press(&mut app, KeyCode::Char('l'));
        }
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "color red");
        press(&mut app, KeyCode::Enter);
        let view = app.core.get_viewport(0..1);
        assert_eq!(
            view.get(0).expect("the block").runs[0]
                .props
                .color
                .as_deref(),
            grind_core::style::palette("red")
        );
    }

    /// The sixteen a terminal has, nearest by hue — a document's own hex has to land on one.
    #[test]
    fn a_document_colour_lands_on_a_colour_the_terminal_has() {
        assert_eq!(nearest_color("#ff4136"), Some(Color::LightRed));
        assert_eq!(nearest_color("#001f3f"), Some(Color::Black));
        assert_eq!(nearest_color("#ffffff"), Some(Color::White));
        assert_eq!(nearest_color("not a colour"), None);
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
