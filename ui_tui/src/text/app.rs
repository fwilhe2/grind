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

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout as Rects};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use grind_text::style::CharStyle;
use grind_text::{App as CoreApp, BlockKind, BlockView, Caret, Metrics};

use super::Cells;
use super::keymap::{self, Action, Motion};
use crate::app::RedrawFlag;
use crate::chrome;
use grind_text::markdown::{self, Emphasis};

/// Room for `p12 h1 ` down the left, so a reader can see the structure the outline is made of.
const GUTTER: u16 = 8;

/// How far one nesting level of a list indents its text, in terminal cells.
///
/// Two, which is exactly the width of the bullet that goes in it — so a depth-1 item's marker
/// occupies the indent rather than sitting beside it, and a depth-2 item's is two cells further
/// in. `ui_text_gtk/src/geom.rs`'s `INDENT` is the same idea in pixels.
const INDENT: u16 = 2;

/// How far a block's text starts from the left of the text column.
///
/// The only kind that has one is a list item, because it is the only kind whose *depth* is part
/// of the model. A heading is not indented: outline structure is implied by the level alone
/// (`text/src/model.rs`), and indenting it would be this shell inventing a hierarchy the
/// document does not have.
fn indent_of(kind: &BlockKind) -> u16 {
    match kind {
        // Bounded, because a document may legitimately carry a deeply nested list and a window
        // is only so wide — past this the text would have nowhere left to go.
        BlockKind::ListItem { depth } => INDENT * (*depth).clamp(1, 8) as u16,
        _ => 0,
    }
}

/// The bullet a list item's first line wears, sitting in the last two cells of its own indent.
///
/// One glyph per depth, cycling — the convention every word processor uses, and it is *drawn*
/// rather than inserted: a marker in the text would be a character the core never measured, which
/// puts every caret after it in the wrong column (`doc/tui-shell.md`, decision 2). This is
/// outside the block's measure altogether, which is why it is allowed where `**` is not.
fn bullet_of(depth: u32) -> &'static str {
    match depth.max(1) % 3 {
        1 => "\u{2022} ",
        2 => "\u{25e6} ",
        _ => "\u{2023} ",
    }
}

/// How wide each block is measured, and in what — this shell's [`grind_text::Faces`].
///
/// Two things make a block narrower than the window: a **list item**'s own depth, which
/// [`grind_text::Faces::of`] is handed directly as part of the kind, and a **table cell**, which
/// it is not. So the cells are a map, built once per frame from the blocks in and around the view
/// and read back here — which is the shape `ui_text_gtk/src/view.rs`'s `Column` has, and for the
/// reason `grind_text::Faces`' own documentation gives: this is called while `App` holds its read
/// lock, so it must not ask the document anything.
///
/// ponytail: the map covers the view and any table it touches, not the whole document. A caret
/// motion that leaps out of the view into a table it has never drawn measures that table's first
/// block at the full measure for one frame, and the frame after it is right. The upgrade is a
/// flow over the whole document, which is what the GNOME window builds and what a terminal
/// showing thirty lines of a thousand-block report should not.
#[derive(Clone, Debug, Default)]
struct Measures {
    /// The text column, in terminal cells.
    width: f32,
    /// Every block in a table the view touches, and the measure its own cell gives it.
    cells: HashMap<usize, f32>,
    /// The tables the view touches, in document order.
    tables: Vec<TableBox>,
}

/// One table, as this shell lays it out: which blocks it is made of, how big it is, and how wide
/// one of its columns is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TableBox {
    blocks: Range<usize>,
    rows: u32,
    columns: u32,
    /// The text inside one cell, in cells — the rules either side are not part of it.
    cell_width: u16,
}

impl Measures {
    fn measure(&self, index: usize, kind: &BlockKind) -> f32 {
        match self.cells.get(&index) {
            Some(width) => *width,
            None => (self.width - f32::from(indent_of(kind))).max(1.0),
        }
    }

    /// The table `index` is anywhere inside.
    fn table_of(&self, index: usize) -> Option<&TableBox> {
        self.tables
            .iter()
            .find(|table| table.blocks.contains(&index))
    }
}

impl grind_text::Faces for Measures {
    fn of(&self, index: usize, kind: &BlockKind, _style: Option<&str>) -> (f32, &dyn Metrics) {
        // One font at one size, so the *provider* never varies — only the measure does. That is
        // the whole of what a terminal contributes to layout, and the reason it was the sharpest
        // test of `doc/text-layout.md`'s decision.
        (self.measure(index, kind), &Cells)
    }
}

/// One screen row of the document.
///
/// Three kinds, because a terminal draws three things: a line of an ordinary block, one line
/// across a table's row, and a table's own rule. Everything else in this file is the first of
/// those; the other two exist because a table's cells are placed by *coordinate* and not by what
/// came before them, which is the one place a flat sequence of blocks stops being a stack of
/// lines.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    /// A line of an ordinary block: which block, which of its lines, the characters it covers,
    /// and how far in its text starts.
    Line {
        block: usize,
        line: usize,
        range: Range<usize>,
        indent: u16,
    },
    /// One line across a table's row: what each column contributes, if anything.
    Cells {
        pieces: Vec<Option<(usize, usize, Range<usize>)>>,
        width: u16,
    },
    /// A table's own horizontal rule, and the block its address is spelled from.
    Rule {
        kind: RuleKind,
        columns: usize,
        width: u16,
        at: usize,
    },
}

/// Which of a table's three rules this is — they differ only in the corner and junction glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuleKind {
    Top,
    Between,
    Bottom,
}

impl RuleKind {
    /// The left end, the junction and the right end, in that order.
    fn glyphs(self) -> (char, char, char) {
        match self {
            RuleKind::Top => ('\u{250c}', '\u{252c}', '\u{2510}'),
            RuleKind::Between => ('\u{251c}', '\u{253c}', '\u{2524}'),
            RuleKind::Bottom => ('\u{2514}', '\u{2534}', '\u{2518}'),
        }
    }
}

impl Row {
    /// Whether this row draws line `line` of block `block` — what "is the caret on screen" means.
    fn holds(&self, block: usize, line: usize) -> bool {
        match self {
            Row::Line {
                block: at,
                line: which,
                ..
            } => *at == block && *which == line,
            Row::Cells { pieces, .. } => pieces
                .iter()
                .flatten()
                .any(|(at, which, _)| *at == block && *which == line),
            Row::Rule { .. } => false,
        }
    }

    /// The first document line this row draws — where `top` goes when the view scrolls onto it.
    /// `None` for a rule, which belongs to the table rather than to any one line of it.
    fn first_line(&self) -> Option<(usize, usize)> {
        match self {
            Row::Line { block, line, .. } => Some((*block, *line)),
            Row::Cells { pieces, .. } => pieces
                .iter()
                .flatten()
                .map(|(block, line, _)| (*block, *line))
                .min(),
            Row::Rule { .. } => None,
        }
    }

    /// Every block this row reads from, so one viewport call can cover the whole window.
    fn blocks(&self) -> Vec<usize> {
        match self {
            Row::Line { block, .. } => vec![*block],
            Row::Cells { pieces, .. } => pieces.iter().flatten().map(|(at, _, _)| *at).collect(),
            Row::Rule { at, .. } => vec![*at],
        }
    }
}

/// Where the last `:find` matched, and which of those the caret is on.
///
/// A snapshot, like the spreadsheet half's — `App::find` walks every block, and re-running it per
/// keystroke would make `j` the most expensive key in the shell.
#[derive(Clone, Debug, Default)]
struct Find {
    needle: String,
    /// Each hit as the block it is in and the character offset of its first character. Ordered,
    /// because `n` and `N` are defined in terms of the order.
    hits: Vec<(usize, usize)>,
    /// The same hits, grouped by block. Two shapes of one list rather than one, because the two
    /// questions asked of it are different: `n` wants *the next one*, and the renderer asks *what
    /// does this block match at* once per drawn line — which a filter over every hit would make
    /// the window's height times every match a common word has.
    by_block: HashMap<usize, Vec<usize>>,
    at: usize,
}

impl Find {
    fn new(needle: &str, hits: Vec<(usize, usize)>, at: usize) -> Self {
        let mut by_block: HashMap<usize, Vec<usize>> = HashMap::new();
        for (block, offset) in &hits {
            by_block.entry(*block).or_default().push(*offset);
        }
        Find {
            needle: needle.to_owned(),
            hits,
            by_block,
            at,
        }
    }

    fn is_on(&self) -> bool {
        !self.needle.is_empty() && !self.hits.is_empty()
    }

    /// The character ranges this block's matches cover — what a line is marked over.
    fn spans(&self, block: usize) -> Vec<Range<usize>> {
        let len = self.needle.chars().count();
        match self.by_block.get(&block) {
            Some(offsets) => offsets.iter().map(|at| *at..at + len).collect(),
            None => Vec::new(),
        }
    }
}

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
    /// How wide each block is measured, rebuilt every frame — this shell's `Faces`.
    measures: Measures,
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
    /// The last `:find`, and where it matched.
    find: Find,
    /// The outline, when it is showing — one row per heading, each a jump.
    outline: crate::pick::Pick,
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
            measures: Measures {
                width: 60.0,
                ..Measures::default()
            },
            height: 20,
            mode: Mode::Normal,
            resume: None,
            anchor: None,
            register: String::new(),
            status: String::new(),
            names: false,
            find: Find::default(),
            outline: crate::pick::Pick::default(),
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
        if self.outline.is_open() {
            let height = self.help_height();
            if let crate::pick::Nav::Chose(address) = self.outline.on_key(key.code, height) {
                self.cmd_jump(&address);
            }
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
            Action::Next(forward) => self.step_match(forward),
            Action::Escape => {
                self.anchor = None;
                // Escape puts the search away as well as the selection: a document still marked
                // with yesterday's matches is a document lying about what is in it.
                self.find = Find::default();
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

    /// How each block is set, for the motions that may cross out of one into another.
    ///
    /// One font at one size, because a terminal has one — but **not one measure any more**: a
    /// list item is indented and a table cell is a column of its own, so this is [`Measures`]
    /// rather than [`grind_text::Uniform`]. That is the seam earning its keep in the medium it
    /// was hardest to fake: the GUI shells vary the *face* per block and this varies only the
    /// width, and the engine above cannot tell which of the two it is being handed.
    fn faces(&self) -> &Measures {
        &self.measures
    }

    /// How wide the block at `index` is laid out — [`Measures::measure`] with the kind read for
    /// it, which is what every layout call in this file needs before it can ask.
    fn measure(&self, index: usize) -> f32 {
        let kind = self.kind_at(index).unwrap_or(BlockKind::Paragraph);
        self.measures.measure(index, &kind)
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
                    None => self.core.caret_x(self.caret, self.faces()).unwrap_or(0.0),
                };
                self.goal_x = Some(goal);
                if let Ok(moved) =
                    self.core
                        .caret_line(self.caret, delta as isize, goal, self.faces())
                {
                    self.caret = moved;
                }
            }
            Motion::LineStart | Motion::LineEnd => {
                self.goal_x = None;
                if let Ok((start, end)) = self.core.caret_line_bounds(self.caret, self.faces()) {
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
            "about" | "version" => self.status = crate::help::about(),
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
            "find" => self.cmd_find(""),
            "mark!" => self.cmd_unmark(),
            "table" => self.cmd_table("2 2"),
            _ if cmd.starts_with("mark ") => self.cmd_mark(cmd[5..].trim()),
            _ if cmd.starts_with("table ") => self.cmd_table(cmd[6..].trim()),
            _ if cmd.starts_with("move ") => self.cmd_move(cmd[5..].trim()),
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

    /// `:outline` — every heading, indented by its level, each row a jump.
    ///
    /// A **pane** rather than a line of status text, which is what it used to be: an outline is a
    /// list somebody reads and then acts on, and a list printed into a one-line bar is neither
    /// readable past the third heading nor clickable at all. [`crate::pick`] is the widget, and it
    /// opens on the section the caret is already in — the same courtesy the code view does.
    fn cmd_outline(&mut self) {
        let rows: Vec<crate::pick::Row> = self
            .core
            .outline()
            .into_iter()
            .map(|heading| crate::pick::Row {
                address: heading.address(),
                label: heading.text,
                depth: heading.path.len().saturating_sub(1),
            })
            .collect();
        // Which section the caret is in: the last heading at or before it.
        let here = self
            .core
            .outline()
            .into_iter()
            .rfind(|heading| heading.index <= self.caret.block)
            .map(|heading| heading.address());
        self.outline.open("Outline", rows, here.as_deref());
        self.status.clear();
    }

    /// `:mark <name>` — anchor a bookmark at the caret's block, which is the one thing `:names`
    /// could *show* and this shell could not make.
    ///
    /// A bookmark is what makes `#intro` an address that survives an edit above it where `p12`
    /// does not, and until now every client in the suite could read one and only the CLI could
    /// write one (`doc/feature-matrix.md` §7).
    fn cmd_mark(&mut self, name: &str) {
        if name.is_empty() {
            self.status = "usage: :mark <name>".to_string();
            return;
        }
        match self.core.set_bookmark(name, Some(self.caret.block)) {
            Ok(moved) => {
                self.names = true;
                self.status = match moved {
                    true => format!("#{name} moved here \u{2014} :names shows where"),
                    false => format!("#{name} anchored here \u{2014} :names shows where"),
                };
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// `:mark!` — drop whichever bookmark anchors in the caret's block.
    fn cmd_unmark(&mut self) {
        let here = self
            .core
            .bookmarks()
            .into_iter()
            .find(|(_, index)| *index == self.caret.block);
        match here {
            Some((name, _)) => match self.core.set_bookmark(&name, None) {
                Ok(_) => self.status = format!("dropped #{name} \u{2014} u brings it back"),
                Err(e) => self.status = e.to_string(),
            },
            None => self.status = "no bookmark anchors here".to_string(),
        }
    }

    /// `:table [rows cols]` — a table before the caret's block, and the grid this shell draws
    /// round one.
    ///
    /// A cell holds *blocks* (`doc/text-core.md`), so every key here already worked inside one
    /// before this verb existed; what it adds is the ability to make one at all.
    fn cmd_table(&mut self, size: &str) {
        let mut words = size.split_whitespace();
        let rows = words
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(2);
        let columns = words
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(2);
        if rows == 0 || columns == 0 {
            self.status = "usage: :table <rows> <columns>".to_string();
            return;
        }
        let at = self.caret.block;
        match self.core.insert_table(at, rows, columns, None) {
            Ok(()) => {
                self.caret = Caret {
                    block: at,
                    offset: 0,
                };
                self.goal_x = None;
                self.status = format!("a {rows}\u{00d7}{columns} table \u{2014} u takes it back");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// `:move <address>` — the caret's block, moved to sit before the block that address names.
    ///
    /// The one block operation this shell had no spelling for: `o` opens one and `X` deletes one,
    /// and reordering meant deleting and retyping. `App::move_blocks` is one action, so it is one
    /// press of `u`.
    fn cmd_move(&mut self, address: &str) {
        let to = match grind_text::loc::parse(address)
            .map_err(|e| e.to_string())
            .and_then(|loc| self.core.resolve(&loc).map_err(|e| e.to_string()))
        {
            Ok(index) => index,
            Err(e) => {
                self.status = format!("not an address: {e}");
                return;
            }
        };
        let from = self.caret.block;
        match self.core.move_blocks(from..from + 1, to) {
            Ok(_) => {
                // Follow the block rather than staying where it was: a move you cannot see the
                // result of is a move you have to go looking for.
                self.caret = Caret {
                    block: to.min(self.core.block_count().saturating_sub(1)),
                    offset: 0,
                };
                self.goal_x = None;
                self.status = format!("moved to {}", grind_text::loc::format(self.caret.block));
            }
            Err(e) => self.status = e.to_string(),
        }
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

    /// Where a piece of text is: **every** match marked on screen, and the caret on the first one
    /// at or after where it already was.
    ///
    /// `n` and `N` step from there, which is vi's own pair of keys and the spreadsheet half's
    /// (`crate::sheet::keymap`). It used to be one jump to the first match and a count — a search
    /// you could not walk.
    fn cmd_find(&mut self, needle: &str) {
        if needle.is_empty() {
            self.find = Find::default();
            self.status = "find cleared".to_string();
            return;
        }
        let hits: Vec<(usize, usize)> = self
            .core
            .find(needle)
            .into_iter()
            .map(|found| (found.index, found.offset))
            .collect();
        let here = (self.caret.block, self.caret.offset);
        let at = hits.iter().position(|hit| *hit >= here).unwrap_or(0);
        self.find = Find::new(needle, hits, at);
        match self.find.hits.is_empty() {
            true => self.status = format!("no match for {needle}"),
            false => {
                self.go_to_match();
                self.status = format!(
                    "{} of {} \u{00b7} n next, N previous",
                    self.find.at + 1,
                    self.find.hits.len()
                );
            }
        }
    }

    /// `n` / `N` — the next or previous match, wrapping round the ends the way vi's do.
    fn step_match(&mut self, forward: bool) {
        if !self.find.is_on() {
            self.status = match self.find.needle.is_empty() {
                true => "nothing to step \u{2014} :find <text> first".to_string(),
                false => format!("no match for {}", self.find.needle),
            };
            return;
        }
        let count = self.find.hits.len();
        self.find.at = match forward {
            true => (self.find.at + 1) % count,
            false => (self.find.at + count - 1) % count,
        };
        self.go_to_match();
        self.status = format!("{} of {count}", self.find.at + 1);
    }

    fn go_to_match(&mut self) {
        if let Some((block, offset)) = self.find.hits.get(self.find.at).copied() {
            self.caret = Caret { block, offset };
            self.anchor = None;
            self.goal_x = None;
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

    /// Every screen row from `top`, as far as the window needs.
    ///
    /// Offsets rather than a `String`, because what is drawn is not one piece of text: a run of it
    /// may be bold, part of it may be selected, part of it may be a search match, and the caret
    /// sits between two characters. [`App::draw`] cuts it up; this only says where each line goes.
    ///
    /// **A table is laid out here, and it is why this returns a [`Row`] rather than a triple.** A
    /// document is a flat sequence of blocks and a terminal is a stack of lines, so everything
    /// else in this file is one block per line and one line under the last. A table's cells are
    /// side by side — its blocks are placed by coordinate, not by what came before them — and that
    /// is the whole of the difference, expressed as one variant.
    fn visible(&self, height: usize) -> Vec<Row> {
        let blocks = self.core.block_count();
        let mut out: Vec<Row> = Vec::with_capacity(height);
        let mut block = self.top.0.min(blocks.saturating_sub(1));
        let mut skip = self.top.1;
        // One viewport read for the whole walk rather than one per block: a block is at least one
        // line tall, so `height` of them is the most a window of `height` rows can reach. The
        // only thing wanted from it is each block's *kind*, which is what says how far its text
        // is indented — a table's cells get their measure from `Measures` instead.
        let kinds = self
            .core
            .get_viewport(block..(block + height + 1).min(blocks));
        while block < blocks && out.len() < height {
            // A block in a table draws as part of the whole table, from its first cell — a grid
            // cannot be entered halfway across.
            if let Some(table) = self.measures.table_of(block) {
                let rows = self.table_rows(table);
                // Opening mid-table (scrolled into it from above) starts on the row the top of
                // the window is actually on rather than at the table's own first rule.
                let from = match out.is_empty() {
                    true => rows
                        .iter()
                        .position(|row| row.holds(block, skip))
                        .unwrap_or(0),
                    // Reached from above, so the whole table is drawn — rule first.
                    false => 0,
                };
                for row in rows.into_iter().skip(from) {
                    if out.len() == height {
                        break;
                    }
                    out.push(row);
                }
                block = table.blocks.end;
                skip = 0;
                continue;
            }
            let kind = kinds
                .get(block)
                .map(|view| view.kind.clone())
                .unwrap_or(BlockKind::Paragraph);
            let indent = indent_of(&kind);
            let measure = self.measures.measure(block, &kind);
            if let Ok(layout) = self.core.layout_block(block, measure, &Cells) {
                for (n, line) in layout.lines().iter().enumerate().skip(skip) {
                    if out.len() == height {
                        break;
                    }
                    out.push(Row::Line {
                        block,
                        line: n,
                        range: line.start..line.end,
                        indent,
                    });
                }
            }
            skip = 0;
            block += 1;
        }
        out
    }

    /// One table, as the rows a terminal draws it in: a rule, then each of its rows' lines side by
    /// side, a rule between rows, and a rule under the last.
    ///
    /// Every cell is laid out at its **own** measure, which is what makes the caret land where the
    /// ink is: `Measures` gave each of its blocks that width, so `App::layout_block`,
    /// `App::caret_line` and this all break the same text at the same places.
    fn table_rows(&self, table: &TableBox) -> Vec<Row> {
        let views = self.core.get_viewport(table.blocks.clone());
        let mut out = Vec::new();
        for row in 0..table.rows {
            // What each column contributes, as the lines of the blocks in that cell one after
            // another — a cell holds blocks, and several of them stack inside it.
            let mut columns: Vec<Vec<(usize, usize, Range<usize>)>> =
                vec![Vec::new(); table.columns as usize];
            for index in table.blocks.clone() {
                let Some(view) = views.get(index) else {
                    continue;
                };
                let Some(cell) = &view.cell else { continue };
                if cell.row != row {
                    continue;
                }
                let Some(column) = columns.get_mut(cell.column as usize) else {
                    continue;
                };
                if let Ok(layout) =
                    self.core
                        .layout_block(index, f32::from(table.cell_width), &Cells)
                {
                    for (n, line) in layout.lines().iter().enumerate() {
                        column.push((index, n, line.start..line.end));
                    }
                }
            }
            let tall = columns.iter().map(Vec::len).max().unwrap_or(1).max(1);
            out.push(Row::Rule {
                kind: match row {
                    0 => RuleKind::Top,
                    _ => RuleKind::Between,
                },
                columns: table.columns as usize,
                width: table.cell_width,
                at: table.blocks.start,
            });
            for line in 0..tall {
                out.push(Row::Cells {
                    pieces: columns
                        .iter()
                        .map(|column| column.get(line).cloned())
                        .collect(),
                    width: table.cell_width,
                });
            }
        }
        out.push(Row::Rule {
            kind: RuleKind::Bottom,
            columns: table.columns as usize,
            width: table.cell_width,
            at: table.blocks.start,
        });
        out
    }

    /// Where each block in and around the view is measured, rebuilt every frame.
    ///
    /// One viewport read for the window, and one `App::table` call per *table* rather than per
    /// block — a table's extent is derived from its cells (`Document::table_extent`), and the core
    /// is the one place that derivation lives.
    fn measures_for(&self, width: f32, height: usize) -> Measures {
        let blocks = self.core.block_count();
        // A page either side of the view, so a `Ctrl+f` that lands in a table has already
        // measured it. See the `ponytail` on `Measures` for what the bound costs.
        let lo = self.top.0.saturating_sub(height);
        let hi = (self.top.0 + 2 * height + 1).min(blocks);
        let mut measures = Measures {
            width,
            ..Measures::default()
        };
        if lo >= hi {
            return measures;
        }
        let views = self.core.get_viewport(lo..hi);
        let mut index = lo;
        while index < hi {
            let in_a_table = views.get(index).is_some_and(|view| view.cell.is_some());
            if !in_a_table {
                index += 1;
                continue;
            }
            let Some(table) = self.core.table(index) else {
                index += 1;
                continue;
            };
            // Every column the same width, which is what this build's model can say: a table
            // carries no style of its own, so there are no column widths to honour
            // (`doc/text-core.md`). One rule down each side of each column comes out first.
            let columns = table.columns.max(1) as u16;
            let cell_width = ((width as u16).saturating_sub(columns + 1) / columns).max(1);
            for block in table.blocks.clone() {
                measures.cells.insert(block, f32::from(cell_width));
            }
            let end = table.blocks.end;
            measures.tables.push(TableBox {
                blocks: table.blocks,
                rows: table.rows,
                columns: table.columns,
                cell_width,
            });
            index = end.max(index + 1);
        }
        measures
    }

    /// Slide `top` just far enough to keep the caret's line on screen.
    fn follow_caret(&mut self, height: usize) {
        let line = self
            .core
            .layout_block(self.caret.block, self.measure(self.caret.block), &Cells)
            .map(|l| l.line_at(self.caret.offset))
            .unwrap_or(0);
        let here = (self.caret.block, line);
        if here < self.top {
            self.top = here;
            return;
        }
        // Walk the window forward one row at a time until the caret is inside it. Bounded by the
        // document, and only ever a few steps in practice because the caret moves by one.
        while !self
            .visible(height)
            .iter()
            .any(|row| row.holds(here.0, here.1))
        {
            let Some(next) = self
                .visible(height)
                .iter()
                .skip(1)
                .find_map(Row::first_line)
            else {
                return;
            };
            if next <= self.top {
                return;
            }
            self.top = next;
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
        line: Range<usize>,
        selection: &Option<(Caret, Caret)>,
        caret: Option<usize>,
    ) -> Vec<Span<'static>> {
        let marks = self.find.spans(view.index);
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
        for span in &marks {
            mark(span.start);
            mark(span.end);
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
            // A `:find` match, and it **replaces** the run's own colours rather than adding to
            // them — the same rule the spreadsheet half's marks follow, for the same reason: a
            // match drawn over a run that was already yellow would be a mark nobody could tell
            // from the document. It is transient, it is only drawn while a search is live, and
            // `Esc` puts it away.
            if marks
                .iter()
                .any(|span| span.start <= start && end <= span.end)
            {
                style = MATCH;
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
        if self.outline.is_open() {
            self.outline.draw(frame, area, "headings");
            return;
        }
        if let Some(projection) = self.source.take() {
            let title = self.document_name();
            self.code.draw(frame, area, &projection, &title);
            self.source = Some(projection);
            return;
        }
        let [title_area, body, status_area] = Rects::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);

        let width = f32::from(body.width.saturating_sub(GUTTER)).max(1.0);
        self.height = usize::from(body.height).max(1);
        self.clamp_caret();
        // Twice, either side of following the caret: the first pass measures the window the view
        // is on, the second the window it moved to — a `G` into a table would otherwise be drawn
        // for one frame at the full measure. Both are one viewport read of three screenfuls.
        self.measures = self.measures_for(width, self.height);
        self.follow_caret(self.height);
        self.measures = self.measures_for(width, self.height);

        let caret_line = self
            .core
            .layout_block(self.caret.block, self.measure(self.caret.block), &Cells)
            .map(|layout| layout.line_at(self.caret.offset))
            .unwrap_or(0);

        // --- the title bar: which document, and which section of it the caret is in ---
        let name = chrome::file_name(self.path.as_deref());
        let mut left = vec![chrome::badge("TEXT")];
        left.extend(chrome::document(&name, self.core.can_undo()));
        let right = match self.section_here() {
            Some(section) => vec![Span::styled(
                format!("{section}  "),
                chrome::title_style().fg(Color::Gray),
            )],
            None => Vec::new(),
        };
        frame.render_widget(
            chrome::bar(title_area.width, chrome::title_style(), left, right),
            title_area,
        );

        let rows = self.visible(self.height);
        let selection = self.selection();
        // **One viewport read for the whole window**, rather than one per block: every row says
        // which blocks it draws, and the range between the first and the last is contiguous
        // because the document is a sequence.
        let touched: Vec<usize> = rows.iter().flat_map(Row::blocks).collect();
        let views = match (touched.iter().min(), touched.iter().max()) {
            (Some(lo), Some(hi)) => self.core.get_viewport(*lo..hi + 1),
            _ => self.core.get_viewport(0..0),
        };

        let mut lines = Vec::with_capacity(self.height);
        for row in &rows {
            lines.push(match row {
                Row::Line {
                    block,
                    line,
                    range,
                    indent,
                } => {
                    let Some(view) = views.get(*block) else {
                        continue;
                    };
                    // Only the first line of a block carries its mark and its bullet, so a
                    // wrapped paragraph reads as one paragraph.
                    let first = *line == 0;
                    let mut spans = vec![Span::styled(
                        match first {
                            true => format!(
                                "{:<4}{:<3} ",
                                grind_text::loc::format(*block),
                                describe_block(&view.kind, view.style.as_deref())
                            ),
                            false => " ".repeat(GUTTER as usize),
                        },
                        gutter_style(&view.kind, view.style.as_deref()),
                    )];
                    if *indent > 0 {
                        spans.push(Span::styled(
                            indent_text(&view.kind, *indent, first),
                            Style::default().add_modifier(Modifier::DIM),
                        ));
                    }
                    let caret = ((*block, *line) == (self.caret.block, caret_line))
                        .then_some(self.caret.offset);
                    spans.extend(self.line_spans(view, range.clone(), &selection, caret));
                    // `doc/view-modes.md` §3.6: a bookmark is the named-range analogue and it is
                    // the one part of a text document a reader cannot see at all — it contributes
                    // no characters. With `:names` on, the block that holds one says so, after
                    // its text rather than inside it, because an offset inside the line is an
                    // offset the caret counts and a mark drawn there would move it.
                    if self.names && first && !view.marks.is_empty() {
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
                    Line::from(spans)
                }
                Row::Cells { pieces, width } => {
                    let mut spans = vec![Span::raw(" ".repeat(GUTTER as usize))];
                    for piece in pieces {
                        spans.push(Span::styled("\u{2502}", RULE));
                        let cell = match piece {
                            Some((block, line, range)) => match views.get(*block) {
                                Some(view) => {
                                    let caret = ((*block, *line) == (self.caret.block, caret_line))
                                        .then_some(self.caret.offset);
                                    self.line_spans(view, range.clone(), &selection, caret)
                                }
                                None => Vec::new(),
                            },
                            None => Vec::new(),
                        };
                        spans.extend(fit(cell, usize::from(*width)));
                    }
                    spans.push(Span::styled("\u{2502}", RULE));
                    Line::from(spans)
                }
                Row::Rule {
                    kind,
                    columns,
                    width,
                    at,
                } => {
                    let (start, join, end) = kind.glyphs();
                    let mut drawn = String::from(start);
                    for column in 0..*columns {
                        drawn.push_str(&"\u{2500}".repeat(usize::from(*width)));
                        drawn.push(match column + 1 == *columns {
                            true => end,
                            false => join,
                        });
                    }
                    Line::from(vec![
                        Span::styled(
                            match kind {
                                RuleKind::Top => {
                                    format!("{:<4}{:<3} ", grind_text::loc::format(*at), "tbl")
                                }
                                _ => " ".repeat(GUTTER as usize),
                            },
                            gutter_style(&BlockKind::Paragraph, None),
                        ),
                        Span::styled(drawn, RULE),
                    ])
                }
            });
        }
        frame.render_widget(Paragraph::new(lines), body);

        let where_ = grind_text::loc::format_offset(self.caret.block, self.caret.offset);
        let mode = match &self.mode {
            Mode::Normal => chrome::Mode::Normal,
            Mode::Visual => chrome::Mode::Visual,
            Mode::Insert => chrome::Mode::Insert,
            Mode::Command { .. } => chrome::Mode::Command,
        };
        let says = match &self.mode {
            Mode::Command { buf } => format!(":{buf}"),
            Mode::Insert => {
                "**bold** *italic* __under__ ~~struck~~  `code`  # heading  - list".to_string()
            }
            Mode::Visual => {
                "* bold  / italic  _ under  ~ struck  - plain  y yank  d delete".to_string()
            }
            _ if !self.status.is_empty() => self.status.clone(),
            _ => "hjkl move  i/a/o insert  v select  x erase  X delete  J join  u undo  :help"
                .to_string(),
        };
        // The caret's own address at the far end, and how much is selected when something is —
        // the two numbers a writer glances at, in the place they stay put.
        let at = match self.selection() {
            Some((from, to)) => format!("{} selected \u{00b7} {where_}", span_len(from, to)),
            None => where_,
        };
        frame.render_widget(
            Paragraph::new(chrome::bar(
                status_area.width,
                chrome::status_style(),
                vec![
                    mode.chip(),
                    Span::styled(format!(" {says}"), chrome::status_style()),
                ],
                vec![Span::styled(format!("{at} "), chrome::muted())],
            ))
            .style(chrome::status_style()),
            status_area,
        );
    }

    /// What the title bar calls this document.
    fn document_name(&self) -> String {
        self.path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "untitled".to_owned())
    }

    /// Which section the caret is in — the last heading at or before it, as `\u{a7}2.1 Costs`.
    ///
    /// The one thing on screen that says *where in a long document* the caret is, which is why it
    /// earns the title bar's right-hand end. It walks the outline, which walks every block; that
    /// is one pass per frame over a document a person is reading, and the same walk `:outline`
    /// makes.
    fn section_here(&self) -> Option<String> {
        let heading = self
            .core
            .outline()
            .into_iter()
            .rfind(|heading| heading.index <= self.caret.block)?;
        Some(format!("{} {}", heading.address(), heading.text))
    }
}

/// A table's rules, and a `:find` match — the two grounds this pane paints that are not the
/// document's own.
///
/// Named colours for `crate::chrome`'s reason. A rule is drawn quietly on purpose: it is the
/// shape of the table, and a grid whose lines shouted would be a grid you read instead of the
/// text in it.
const RULE: Style = Style::new().fg(Color::DarkGray);
const MATCH: Style = Style::new().bg(Color::LightYellow).fg(Color::Black);

/// The indent a block's line starts with, with the list bullet in the last two cells of the
/// first one.
fn indent_text(kind: &BlockKind, indent: u16, first: bool) -> String {
    match (first, kind) {
        (true, BlockKind::ListItem { depth }) => {
            let bullet = bullet_of(*depth);
            format!(
                "{}{bullet}",
                " ".repeat(usize::from(indent).saturating_sub(bullet.chars().count()))
            )
        }
        _ => " ".repeat(usize::from(indent)),
    }
}

/// What colour the gutter is drawn in — the shell's own space, so this is the one place a
/// *structural* colour is allowed.
///
/// A run's own `fo:color` is the document's and is drawn as itself (`terminal_style`); a colour
/// chosen here would be indistinguishable from one. The gutter has no document content in it at
/// all — it holds an address and three letters this shell wrote — so the kind may be a hue there
/// without ever being one in the text.
fn gutter_style(kind: &BlockKind, style: Option<&str>) -> Style {
    let color = match kind {
        _ if style == Some(markdown::PREFORMATTED) => Color::Magenta,
        BlockKind::Heading { .. } => Color::Cyan,
        BlockKind::ListItem { .. } => Color::Green,
        BlockKind::Paragraph => Color::DarkGray,
    };
    Style::default().fg(color)
}

/// Cut or pad a row of spans to exactly `width` terminal cells.
///
/// What a table cell needs and nothing else does: the rule after a cell has to land in the same
/// column on every line of the table, so a cell that came out one cell wide — the caret sitting
/// past the end of a full line does exactly that — would bend the grid.
fn fit(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let mut out = Vec::with_capacity(spans.len() + 1);
    let mut used = 0usize;
    for span in spans {
        let span_width = span.width();
        if used + span_width <= width {
            used += span_width;
            out.push(span);
            continue;
        }
        let room = width - used;
        let style = span.style;
        let mut kept = String::new();
        for c in span.content.chars() {
            // Whole characters only: a wide one half-landing in the last cell would put the rule
            // after it a column out.
            if kept.width() + c.width().unwrap_or(0) > room {
                break;
            }
            kept.push(c);
        }
        used += kept.width();
        if !kept.is_empty() {
            out.push(Span::styled(kept, style));
        }
        break;
    }
    if used < width {
        out.push(Span::raw(" ".repeat(width - used)));
    }
    out
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
        // Row 0 is the title bar, so the document starts at 1.
        let plain = render(&mut app, 40, 6);
        assert!(!plain[1].contains("intro"), "{:?}", plain[1]);

        app.run_command("names");
        let shown = render(&mut app, 40, 6);
        assert!(shown[1].contains("intro"), "{:?}", shown[1]);
        assert!(shown[1].contains("Introduction"), "the text yielded");
        assert_eq!(text(&app), "Introduction", "a reading changed the document");

        app.run_command("names");
        assert_eq!(render(&mut app, 40, 6)[1], plain[1]);
    }

    /// **The gap `doc/tui-shell.md` named, closed.** A table is drawn as a grid, with box rules
    /// round cells that were already editable — and every cell is laid out at its *own* measure,
    /// which is what makes the caret land where the ink is.
    #[test]
    fn a_table_is_drawn_as_a_grid_and_its_cells_are_measured_narrower() {
        let mut app = app(&["before", "after"]);
        app.core.insert_table(1, 2, 3, None).expect("a table");
        for (index, text) in [(1, "Region"), (2, "Q1"), (3, "Q2"), (4, "North")] {
            app.core.set_text(index, text).expect("fills a cell");
        }
        let lines = render(&mut app, 76, 14);
        let shown = lines.join("\n");
        assert!(
            shown.contains('\u{250c}') && shown.contains('\u{252c}'),
            "a top rule: {shown}"
        );
        assert!(
            shown.contains('\u{251c}'),
            "a rule between the rows: {shown}"
        );
        assert!(shown.contains('\u{2514}'), "a rule under it: {shown}");
        // Three columns side by side on one line, which is the whole point.
        let row = lines
            .iter()
            .find(|line| line.contains("Region"))
            .expect("the header row");
        assert!(row.contains("Q1") && row.contains("Q2"), "{row:?}");
        assert!(
            row.matches('\u{2502}').count() == 4,
            "a rule either side of each: {row:?}"
        );
        // The table's own address is in the gutter, and the blocks after it are back to normal.
        assert!(shown.contains("p2  tbl"), "{shown}");
        assert!(
            lines.iter().any(|line| line.starts_with("p8  p   after")),
            "{lines:?}"
        );

        // A cell's blocks are measured at the cell's width, not the window's: this one wraps.
        let measure = app.measure(1);
        assert!(
            measure < 30.0,
            "a cell is far narrower than the window: {measure}"
        );
        assert_eq!(app.measure(0), 68.0, "and a block outside it is not");
    }

    /// Every caret motion already worked inside a cell before the grid was drawn — this is the
    /// half that was missing, and it must keep working with the narrower measure.
    #[test]
    fn the_caret_moves_inside_a_table_cell_by_its_own_lines() {
        let mut app = app(&["before"]);
        app.core.insert_table(1, 1, 2, None).expect("a table");
        app.core
            .set_text(1, "the cat sat on the mat and then it slept again")
            .expect("a long cell");
        render(&mut app, 60, 12);
        let lines = app
            .core
            .layout_block(1, app.measure(1), &Cells)
            .expect("laid out")
            .lines()
            .len();
        assert!(lines > 1, "the cell has to wrap or this proves nothing");

        app.caret = Caret {
            block: 1,
            offset: 0,
        };
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.caret.block, 1, "still in the same cell");
        assert!(app.caret.offset > 0, "and further down it");
        // And typing lands in the cell, not beside it.
        press(&mut app, KeyCode::Char('i'));
        type_str(&mut app, "X");
        assert!(app.core.input_text(1).unwrap().contains('X'));
    }

    /// `:table` — the verb that makes one. Until now this shell could edit a cell and not create
    /// a table to put one in.
    #[test]
    fn the_command_line_inserts_a_table_and_u_takes_it_back() {
        let mut app = app(&["only"]);
        app.run_command("table 2 3");
        assert_eq!(app.core.block_count(), 7, "{}", app.status);
        assert!(app.core.table(0).is_some(), "the caret's block is in it");
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.core.block_count(), 1, "one action, one undo");

        app.run_command("table 0 3");
        assert!(app.status.starts_with("usage:"), "{}", app.status);
    }

    /// A list item is indented and wears a bullet, and the bullet is **drawn** rather than typed
    /// — the text is untouched, so no caret after it moves.
    #[test]
    fn a_list_item_is_indented_and_wears_a_drawn_bullet() {
        let mut app = app(&["shallow", "deeper"]);
        app.core
            .set_kind(0, BlockKind::ListItem { depth: 1 })
            .expect("a list item");
        app.core
            .set_kind(1, BlockKind::ListItem { depth: 2 })
            .expect("a nested one");
        let lines = render(&mut app, 40, 8);
        assert!(lines[1].contains("\u{2022} shallow"), "{lines:?}");
        assert!(
            lines[2].ends_with("  \u{25e6} deeper"),
            "one level further in, with its own bullet: {:?}",
            lines[2]
        );
        assert_eq!(text(&app), "shallow\ndeeper", "nothing was inserted");
        // And the measure shrank by the indent, which is what puts the wrap in the right place.
        assert_eq!(app.measure(0), 30.0);
        assert_eq!(app.measure(1), 28.0);
    }

    /// `:find` marks every match and `n`/`N` walk them — vi's own two keys, and the same pair
    /// the spreadsheet half binds.
    #[test]
    fn find_marks_every_match_and_n_steps_through_them() {
        let mut app = app(&["one two", "two three", "and two more"]);
        render(&mut app, 40, 8);
        app.run_command("find two");
        assert_eq!(app.find.hits.len(), 3, "{}", app.status);
        assert_eq!(app.caret.block, 0);

        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.caret.block, 1);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.caret.block, 2);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.caret.block, 0, "and it wraps");
        press(&mut app, KeyCode::Char('N'));
        assert_eq!(app.caret.block, 2, "backwards too");

        // The matches are marked where they are, not just counted.
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let marked = buffer[(GUTTER + 4, 1)].style();
        assert_eq!(
            marked.bg,
            Some(Color::LightYellow),
            "the `two` in `one two`: {marked:?}"
        );
        let plain = buffer[(GUTTER, 1)].style();
        assert_ne!(plain.bg, Some(Color::LightYellow), "and only the match");

        press(&mut app, KeyCode::Esc);
        assert!(!app.find.is_on(), "Esc puts the search away");
    }

    /// `:mark` — the verb that makes a bookmark, which is what turns `#intro` into an address
    /// that survives an edit above it.
    #[test]
    fn the_command_line_anchors_and_drops_a_bookmark() {
        let mut app = app(&["Introduction", "body"]);
        app.run_command("mark intro");
        assert_eq!(app.core.bookmarks(), vec![("intro".to_owned(), 0)]);
        // It turned the overlay on, so the reader can see what they just made.
        assert!(render(&mut app, 40, 8)[1].contains("intro"));

        // And it is an address like any other, straight away.
        app.run_command("#intro");
        assert_eq!(app.caret.block, 0);

        app.run_command("mark!");
        assert!(app.core.bookmarks().is_empty(), "{}", app.status);
    }

    /// `:move` — a block put somewhere else, in one action and so one press of `u`.
    #[test]
    fn the_command_line_moves_a_block() {
        let mut app = app(&["first", "second", "third"]);
        app.caret = Caret {
            block: 2,
            offset: 0,
        };
        app.run_command("move p1");
        assert_eq!(text(&app), "third\nfirst\nsecond", "{}", app.status);
        assert_eq!(app.caret.block, 0, "the caret followed it");
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(text(&app), "first\nsecond\nthird");
    }

    /// `:outline` opens a pane rather than printing into the status bar, and every row is a jump.
    #[test]
    fn the_outline_is_a_pane_and_every_row_is_a_jump() {
        let mut app = app(&["One", "under it", "Two"]);
        app.core
            .set_kind(0, BlockKind::Heading { level: 1 })
            .unwrap();
        app.core
            .set_kind(2, BlockKind::Heading { level: 1 })
            .unwrap();
        app.run_command("outline");
        assert!(app.outline.is_open());
        let shown = render(&mut app, 44, 8).join("\n");
        assert!(shown.contains("Outline"), "{shown}");
        assert!(shown.contains("One") && shown.contains("Two"), "{shown}");

        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert!(!app.outline.is_open(), "going there closes it");
        assert_eq!(app.caret.block, 2, "and it went there");
    }

    /// The title bar says which document is open, whether it has unsaved changes, and which
    /// section of it the caret is in — the last of which nothing else on screen could say.
    #[test]
    fn the_title_bar_names_the_document_and_the_section_the_caret_is_in() {
        let mut app = app(&["Costs", "some prose"]);
        app.path = Some(PathBuf::from("report.fodt"));
        app.core
            .set_kind(0, BlockKind::Heading { level: 1 })
            .unwrap();
        app.caret = Caret {
            block: 1,
            offset: 0,
        };
        let title = render(&mut app, 60, 8).remove(0);
        assert!(title.contains("TEXT"), "{title:?}");
        assert!(title.contains("report.fodt"), "{title:?}");
        assert!(title.contains("\u{a7}1 Costs"), "{title:?}");
        assert!(title.contains('\u{25cf}'), "unsaved: {title:?}");
    }

    /// Exactly `width` cells, whatever is in them — what keeps a table's rules in one column.
    #[test]
    fn a_table_cell_is_cut_and_padded_to_its_own_width() {
        use unicode_width::UnicodeWidthStr;
        let width_of = |spans: &[Span<'static>]| -> usize {
            spans.iter().map(|span| span.content.as_ref().width()).sum()
        };
        for text in [
            "",
            "short",
            "far too long to fit in here",
            "\u{4e16}\u{754c}\u{4e16}",
        ] {
            let fitted = fit(vec![Span::raw(text.to_owned())], 6);
            assert_eq!(width_of(&fitted), 6, "{text:?}");
        }
        // A wide character is never left half in the last cell.
        let fitted = fit(vec![Span::raw("\u{4e16}\u{754c}".to_owned())], 3);
        assert_eq!(width_of(&fitted), 3);
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
                .layout_block(0, app.measure(0), &Cells)
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
        assert!(lines[1].starts_with("p1  p"), "{lines:?}");
        assert!(
            lines[2].starts_with("        "),
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
            .map(|c| buffer[(c, 1)].symbol().to_string())
            .collect();
        assert!(row.contains("hello world"), "{row:?}");
        assert!(!row.contains('*'), "no markers on screen: {row:?}");

        // The cell under "e" — past the title bar, the gutter and the caret's reversed "h".
        let bold = buffer[(GUTTER + 1, 1)].style();
        assert!(
            bold.add_modifier.contains(Modifier::BOLD),
            "the run draws bold: {bold:?}"
        );
        let plain = buffer[(GUTTER + 7, 1)].style();
        assert!(!plain.add_modifier.contains(Modifier::BOLD), "{plain:?}");
    }

    #[test]
    fn the_command_line_finds_and_substitutes() {
        let mut app = app(&["one two", "two three"]);
        press(&mut app, KeyCode::Char(':'));
        type_str(&mut app, "find two");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.caret.block, 0);
        assert!(app.status.starts_with("1 of 2"), "{}", app.status);

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
