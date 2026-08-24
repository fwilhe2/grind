// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor half of the browser shell — phase 10's S10.
//!
//! The same rule the grid follows: **the DOM is a renderer, not the document.** No
//! `contenteditable` anywhere. Every visible line is rebuilt from [`App::get_viewport`] and
//! [`App::layout_block`] on each repaint and thrown away, so the page cannot become a second
//! copy of the text — which is exactly what `contenteditable` would make it, with its own
//! idea of what a paragraph is and its own undo stack (doc/plan.md rule 1).
//!
//! **One `<div>` per laid-out line, not per paragraph.** The browser would happily wrap a
//! paragraph itself, and then its line breaks and the core's would disagree — Down-arrow
//! would land somewhere other than where the caret appears to be. So the core breaks the
//! lines, in *this* shell's units, and the page draws exactly what it was told
//! (`doc/text-layout.md`, Path C). `Face` below is the whole of what the browser
//! contributes: how wide is this text, in CSS pixels.

pub mod keymap;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use grind_core::style::TextStyle;
use grind_text::{App, BlockKind, Caret, Form, Metrics, loc};
use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, Document, Element, HtmlCanvasElement, HtmlElement, KeyboardEvent,
    MouseEvent,
};

use crate::{element, listen, request_repaint};
use keymap::{Action, Chord, Motion};

/// The document's typeface, owned **here** rather than in the stylesheet.
///
/// Two declarations that must agree are two declarations that will not — the same trap
/// `declare_cell_size` fixed for the grid. So the font is written into each block's inline
/// style as it is rendered, and the canvas measures with the identical string. A serif at
/// this size because this is a page of prose, not a user interface.
const FAMILY: &str = "Georgia, 'Times New Roman', serif";
const BODY_PX: f64 = 17.0;
/// Multiplied by the font size to give a line's height, and by the *body* size to give the
/// space under a paragraph.
const LINE: f64 = 1.55;
const GAP: f64 = 0.85;
/// The extra space above a heading, and how far one list level indents. Both in body sizes.
const HEADING_GAP: f64 = 1.4;
const INDENT: f64 = 1.6;

/// How much bigger than the body each heading level is — the same six numbers the GTK shell
/// uses, because a document should not change shape between two shells of one suite.
const HEADING_SCALE: [f64; 6] = [1.8, 1.5, 1.3, 1.15, 1.05, 1.0];

/// Where a paragraph wraps when the browser has not laid the page out yet.
///
/// jsdom reports every rectangle as zero (`ui_web/smoke.js`), and a width of zero means
/// "do not wrap" to the core — one line per paragraph, which is a perfectly good answer for
/// a headless test and a wrong one for a window that simply has not been measured yet.
const UNMEASURED: f64 = 640.0;

/// One block-level face: a font, its line height, and a canvas to measure with.
///
/// **The unit is the CSS pixel**, and the core neither knows nor converts — the terminal
/// answers in cells and GTK in Pango units over the same trait.
///
/// ponytail: a character's advance is measured **once and cached**, so `advance("ab")` is
/// `advance("a") + advance("b")` and kerning between two characters is lost. The trait's own
/// documentation warns about exactly this. The correct alternative is measuring every prefix
/// of every paragraph on every repaint, which is quadratic in the paragraph's length and
/// shows up as latency while typing. It is invisible where it would matter most — the caret
/// is an element *in* the line, so the browser places it against its own kerning, not against
/// this measurement — and it costs at most a pixel or two in where a line breaks.
struct Face {
    /// `None` where the page has no canvas at all — a headless run under jsdom, which is
    /// where `ui_web/smoke.js` drives this shell. Everything still works there; the widths
    /// are simply made up, and nothing in that test is about how wide a letter is.
    ctx: Option<Rc<CanvasRenderingContext2d>>,
    font: String,
    size: f64,
    height: f64,
    widths: RefCell<HashMap<char, f32>>,
}

impl Face {
    fn new(ctx: Option<Rc<CanvasRenderingContext2d>>, size: f64, bold: bool) -> Self {
        let weight = match bold {
            true => "bold ",
            false => "",
        };
        Face {
            ctx,
            font: format!("{weight}{size}px {FAMILY}"),
            size,
            height: (size * LINE).round(),
            widths: RefCell::new(HashMap::new()),
        }
    }

    /// What a block's element is styled with — the same font this face measures in, which is
    /// the whole reason the two are one object.
    fn css(&self, indent: f64) -> String {
        format!(
            "font:{};line-height:{}px;margin-left:{indent}px",
            self.font, self.height
        )
    }

    fn width_of(&self, c: char) -> f32 {
        if let Some(width) = self.widths.borrow().get(&c) {
            return *width;
        }
        if let Some(ctx) = &self.ctx {
            ctx.set_font(&self.font);
        }
        let width = match c {
            // A break ends the line; a caret sits at its end, and nothing is drawn.
            '\n' => 0.0,
            // ODF's `text:tab` is a tab *stop* the document does not record, so this shell
            // draws it as a fixed space rather than inventing a stop table.
            '\t' => 4.0 * self.measure(' '),
            c => self.measure(c),
        };
        self.widths.borrow_mut().insert(c, width);
        width
    }

    fn measure(&self, c: char) -> f32 {
        let mut buffer = [0u8; 4];
        self.ctx
            .as_ref()
            .and_then(|ctx| ctx.measure_text(c.encode_utf8(&mut buffer)).ok())
            .map(|metrics| metrics.width() as f32)
            // No canvas to ask. Half the font size is a plausible advance, which is all a
            // headless run needs and more than it checks.
            .unwrap_or((self.size / 2.0) as f32)
    }
}

impl Metrics for Face {
    fn advances(&self, text: &str, _style: &TextStyle, out: &mut Vec<f32>) {
        let mut x = 0.0;
        for c in text.chars() {
            x += self.width_of(c);
            out.push(x);
        }
    }

    fn line_height(&self, _style: &TextStyle) -> f32 {
        self.height as f32
    }
}

/// The faces a document is set in: body text, and one per heading level.
struct Faces {
    body: Face,
    headings: Vec<Face>,
}

impl Faces {
    fn new(ctx: Option<Rc<CanvasRenderingContext2d>>) -> Self {
        Faces {
            body: Face::new(ctx.clone(), BODY_PX, false),
            headings: HEADING_SCALE
                .iter()
                .map(|scale| Face::new(ctx.clone(), (BODY_PX * scale).round(), true))
                .collect(),
        }
    }

    /// The face a block is set in.
    ///
    /// A heading deeper than the six levels there are faces for is set as the last of them
    /// rather than refused: the reader is tolerant (R5), so a level-9 heading loads, and a
    /// shell that panicked on one would undo that.
    fn of(&self, kind: &BlockKind) -> &Face {
        match kind {
            BlockKind::Heading { level } => {
                let index = (*level).max(1) as usize - 1;
                self.headings.get(index).unwrap_or(&self.body)
            }
            _ => &self.body,
        }
    }
}

/// The elements this pane writes to. No document state — that is all in the core.
struct Dom {
    document: Document,
    /// The scrolling box, and the thing that holds the keyboard.
    pane: HtmlElement,
    /// The text column inside it: its width is the measure every line is broken at.
    flow: HtmlElement,
    message: HtmlElement,
    summary: HtmlElement,
}

impl Dom {
    fn find(document: &Document) -> Result<Self, JsValue> {
        Ok(Dom {
            document: document.clone(),
            pane: element(document, "page")?,
            flow: element(document, "flow")?,
            message: element(document, "message")?,
            summary: element(document, "summary")?,
        })
    }
}

pub struct Ui {
    pub app: Arc<App>,
    dom: Dom,
    faces: Faces,
    pending: Arc<AtomicBool>,
    // Everything below is *presentation*. None of it is the document, which is why the core
    // neither knows nor keeps it.
    caret: Cell<Caret>,
    /// The column the caret is trying to keep while moving by lines — see
    /// [`App::caret_line`]. Cleared by any horizontal move.
    goal_x: Cell<Option<f32>>,
    message: RefCell<String>,
}

impl Ui {
    pub fn new(
        document: &Document,
        app: Arc<App>,
        pending: Arc<AtomicBool>,
    ) -> Result<Rc<Self>, JsValue> {
        // A canvas that is never added to the page: it exists to be asked how wide a string
        // is, which is the one thing the DOM will not answer without laying it out first.
        //
        // Optional, and deliberately not an error: a page with no canvas is a page that
        // still opens documents, and refusing to start over a measurement device would make
        // the *whole* shell — the spreadsheet included — depend on one.
        let ctx = document
            .create_element("canvas")
            .ok()
            .and_then(|canvas| canvas.dyn_into::<HtmlCanvasElement>().ok())
            .and_then(|canvas| canvas.get_context("2d").ok().flatten())
            .and_then(|ctx| ctx.dyn_into::<CanvasRenderingContext2d>().ok())
            .map(Rc::new);

        let ui = Rc::new(Ui {
            app,
            dom: Dom::find(document)?,
            faces: Faces::new(ctx),
            pending,
            caret: Cell::new(Caret {
                block: 0,
                offset: 0,
            }),
            goal_x: Cell::new(None),
            message: RefCell::new(String::new()),
        });
        wire(&ui)?;
        Ok(ui)
    }

    pub fn focus(&self) -> Result<(), JsValue> {
        self.dom.pane.focus()
    }

    /// A document arrived: into the core, and the presentation state a new one resets.
    pub fn open(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.app
            .open_bytes(name, bytes)
            .map_err(|e| e.to_string())?;
        self.caret.set(Caret {
            block: 0,
            offset: 0,
        });
        self.goal_x.set(None);
        self.dom.pane.set_scroll_top(0);
        Ok(())
    }

    pub fn save_bytes(&self, form: Form) -> Result<Vec<u8>, String> {
        self.app.save_bytes(form).map_err(|e| e.to_string())
    }

    pub fn set_message(&self, message: String) {
        *self.message.borrow_mut() = message;
        self.request_repaint();
    }

    pub fn request_repaint(&self) {
        request_repaint(&self.pending);
    }

    pub fn refresh(&self) {
        if let Err(error) = self.render() {
            web_sys::console::error_1(&error);
        }
    }

    // --- rendering ---

    /// The measure: how wide a line may be, in the same pixels [`Face`] answers in.
    fn width(&self) -> f64 {
        match self.dom.flow.client_width() {
            0 => UNMEASURED,
            width => f64::from(width),
        }
    }

    fn render(&self) -> Result<(), JsValue> {
        let width = self.width();
        let caret = self.caret.get();
        // ponytail: the whole document is rendered, not the part on screen. A grid needs a
        // viewport because a sheet has a million rows; a document has as many blocks as
        // somebody typed, and windowing one costs a scroll-position-to-block map that only
        // pays for itself on documents nobody has written yet. Named in `doc/text-shell.md`.
        let viewport = self.app.get_viewport(0..self.app.block_count());

        self.dom.flow.set_text_content(None);
        for block in viewport.iter() {
            let face = self.faces.of(&block.kind);
            let indent = indent_of(&block.kind);
            let Ok(layout) = self
                .app
                .layout_block(block.index, (width - indent) as f32, face)
            else {
                continue;
            };

            let element = self.dom.document.create_element("div")?;
            element.set_class_name(&class_of(&block.kind));
            element.set_attribute("data-block", &block.index.to_string())?;
            element.set_attribute("style", &face.css(indent))?;

            let text: Vec<char> = block.text.chars().collect();
            for (number, line) in layout.lines().iter().enumerate() {
                let row = self.dom.document.create_element("div")?;
                row.set_class_name("line");
                row.set_attribute("data-line", &number.to_string())?;
                // An empty line still occupies one: a `<div>` with nothing in it is zero
                // pixels tall, however tall its line height says it is.
                row.set_attribute("style", &format!("height:{}px", face.height))?;

                let piece: String = text[line.start.min(text.len())..line.end.min(text.len())]
                    .iter()
                    .collect();
                let piece = piece.trim_end_matches('\n');
                // `line_at` rather than `Line::holds`: an offset at a soft break belongs to
                // both lines, and drawing the caret on both is drawing it twice.
                match block.index == caret.block && layout.line_at(caret.offset) == number {
                    // The caret is an element *between* two pieces of text rather than a
                    // rectangle drawn at a measured x — so the browser places it against its
                    // own kerning, which is the one place this shell's own measurements
                    // could be a pixel out.
                    true => {
                        let at = caret.offset.saturating_sub(line.start);
                        let head: String = piece.chars().take(at).collect();
                        let tail: String = piece.chars().skip(at).collect();
                        row.append_child(&self.dom.document.create_text_node(&head))?;
                        let caret_element = self.dom.document.create_element("span")?;
                        caret_element.set_class_name("caret");
                        caret_element.set_id("caret");
                        row.append_child(&caret_element)?;
                        row.append_child(&self.dom.document.create_text_node(&tail))?;
                    }
                    false => row.set_text_content(Some(piece)),
                }
                element.append_child(&row)?;
            }
            self.dom.flow.append_child(&element)?;
        }

        self.render_chrome();
        self.follow_caret();
        Ok(())
    }

    fn render_chrome(&self) {
        let caret = self.caret.get();
        let counts = self.app.counts();
        self.dom.summary.set_text_content(Some(&format!(
            "{} · {} words · {} blocks",
            loc::format_offset(caret.block, caret.offset),
            counts.words,
            counts.blocks
        )));
        self.dom
            .message
            .set_text_content(Some(&self.message.borrow()));
    }

    /// Scroll the least it takes to keep the caret on screen.
    ///
    /// Arithmetic against `offsetTop` rather than `scrollIntoView`, because the caret is only
    /// ever a *line* out of view and the browser's own version jumps the page around — and
    /// because a headless run reports every offset as zero, where this does nothing at all
    /// rather than throwing.
    fn follow_caret(&self) {
        let Some(caret) = self
            .dom
            .document
            .get_element_by_id("caret")
            .and_then(|element| element.dyn_into::<HtmlElement>().ok())
        else {
            return;
        };
        let top = f64::from(caret.offset_top());
        let height = f64::from(caret.offset_height()).max(1.0);
        let view = f64::from(self.dom.pane.client_height());
        let scroll = f64::from(self.dom.pane.scroll_top());
        let wanted = match scroll {
            _ if top < scroll => top,
            _ if top + height > scroll + view => top + height - view,
            _ => return,
        };
        self.dom.pane.set_scroll_top(wanted as i32);
    }

    // --- input ---

    fn on_key(&self, event: &KeyboardEvent) {
        let key = event.key();
        let chord = Chord {
            key: &key,
            primary: event.ctrl_key() || event.meta_key(),
            shift: event.shift_key(),
        };
        let Some(action) = keymap::action_for(&chord) else {
            return;
        };
        // A key this shell claimed must not also do its default — Tab moves focus, Ctrl+S
        // opens the browser's own save dialog, and Backspace used to go back a page.
        event.prevent_default();
        if let Err(error) = self.apply(action) {
            web_sys::console::error_1(&error);
        }
    }

    fn apply(&self, action: Action<'_>) -> Result<(), JsValue> {
        match action {
            Action::Move(motion) => self.go(motion),
            Action::Type(text) => self.type_text(text),
            Action::Split => self.split(),
            Action::EraseBack => self.erase_back(),
            Action::EraseForward => self.erase_forward(),
            // The chrome's, and the same code path its buttons take.
            Action::Undo => crate::with_shell(|shell| shell.undo()),
            Action::Redo => crate::with_shell(|shell| shell.redo()),
            Action::Open => crate::with_shell(|shell| shell.open_picker()),
            Action::Save => crate::with_shell(|shell| shell.save()),
        }
        Ok(())
    }

    /// Every motion, routed to the core.
    fn go(&self, motion: Motion) {
        if self.app.block_count() == 0 {
            return;
        }
        let caret = self.caret.get();
        let kind = self.kind_at(caret.block);
        let face = self.faces.of(&kind);
        let width = (self.width() - indent_of(&kind)) as f32;
        let moved = match motion {
            Motion::Char(delta) => Some(self.stepped(delta)),
            Motion::Line(steps) | Motion::Page(steps) => {
                let lines = match motion {
                    // A page is however many body lines fit, less one so the line you were
                    // reading is still there afterwards.
                    Motion::Page(_) => {
                        let fit = f64::from(self.dom.pane.client_height()) / self.faces.body.height;
                        (fit as isize - 1).max(1)
                    }
                    _ => 1,
                };
                // Remembered across a run of Down presses, which is what `goal_x` is for.
                let goal = match self.goal_x.get() {
                    Some(x) => x,
                    None => self.app.caret_x(caret, width, face).unwrap_or(0.0),
                };
                self.goal_x.set(Some(goal));
                self.app
                    .caret_line(caret, steps as isize * lines, goal, width, face)
                    .ok()
            }
            Motion::LineStart | Motion::LineEnd => self
                .app
                .caret_line_bounds(caret, width, face)
                .ok()
                .map(|(start, end)| match motion {
                    Motion::LineStart => start,
                    _ => end,
                }),
            Motion::DocStart => Some(Caret {
                block: 0,
                offset: 0,
            }),
            Motion::DocEnd => {
                let block = self.app.block_count() - 1;
                Some(Caret {
                    block,
                    offset: self.block_len(block),
                })
            }
        };
        let Some(moved) = moved else { return };
        if !matches!(motion, Motion::Line(_) | Motion::Page(_)) {
            self.goal_x.set(None);
        }
        self.set_caret(moved);
    }

    /// One character left or right, rolling onto the neighbouring block at either end.
    ///
    /// The only arithmetic here, and it is over *characters* rather than over layout —
    /// walking off the end of a block is a document fact, not a line one.
    fn stepped(&self, delta: i32) -> Caret {
        let mut caret = self.caret.get();
        if delta > 0 {
            if caret.offset < self.block_len(caret.block) {
                caret.offset += 1;
            } else if caret.block + 1 < self.app.block_count() {
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

    fn set_caret(&self, caret: Caret) {
        self.caret.set(caret);
        // Nothing in the core changed, so nothing will tell the page to repaint.
        self.request_repaint();
    }

    fn block_len(&self, index: usize) -> usize {
        self.app
            .input_text(index)
            .map(|text| text.chars().count())
            .unwrap_or(0)
    }

    fn kind_at(&self, index: usize) -> BlockKind {
        self.app
            .get_viewport(index..index + 1)
            .get(index)
            .map(|block| block.kind.clone())
            .unwrap_or(BlockKind::Paragraph)
    }

    // --- editing ---

    fn type_text(&self, text: &str) {
        // A document with no blocks at all has nowhere to put a character, and the first
        // thing anybody does with an empty page is type into it.
        if self.app.block_count() == 0 {
            match self.app.insert(0, BlockKind::Paragraph, text) {
                Ok(()) => self.set_caret(Caret {
                    block: 0,
                    offset: text.chars().count(),
                }),
                Err(error) => self.set_message(error.to_string()),
            }
            return;
        }
        let caret = self.caret.get();
        match self.app.insert_text(caret, text) {
            Ok(()) => {
                self.goal_x.set(None);
                self.set_caret(Caret {
                    block: caret.block,
                    offset: caret.offset + text.chars().count(),
                });
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    fn split(&self) {
        let caret = self.caret.get();
        match self.app.split_block(caret) {
            Ok(()) => {
                self.goal_x.set(None);
                self.set_caret(Caret {
                    block: caret.block + 1,
                    offset: 0,
                });
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// Backspace: the character before the caret, and at the front of a block the boundary
    /// itself — which is what [`App::erase`] across one already does.
    fn erase_back(&self) {
        let caret = self.caret.get();
        let from = self.stepped(-1);
        if from == caret {
            return;
        }
        match self.app.erase(from, caret) {
            Ok(_) => self.set_caret(from),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    fn erase_forward(&self) {
        let caret = self.caret.get();
        let to = match caret.offset < self.block_len(caret.block) {
            true => Caret {
                block: caret.block,
                offset: caret.offset + 1,
            },
            false if caret.block + 1 < self.app.block_count() => Caret {
                block: caret.block + 1,
                offset: 0,
            },
            false => return,
        };
        if let Err(error) = self.app.erase(caret, to) {
            self.set_message(error.to_string());
        }
    }

    /// Where a click landed. The line carries its own address, so the DOM answers the first
    /// half and the core's layout answers the second.
    fn on_click(&self, event: &MouseEvent) -> Result<(), JsValue> {
        let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return Ok(());
        };
        let (Some(row), Some(block)) = (target.closest(".line")?, target.closest("[data-block]")?)
        else {
            return Ok(());
        };
        let (Some(line), Some(index)) = (
            attribute(&row, "data-line"),
            attribute(&block, "data-block"),
        ) else {
            return Ok(());
        };
        let kind = self.kind_at(index);
        let face = self.faces.of(&kind);
        let width = (self.width() - indent_of(&kind)) as f32;
        let Ok(layout) = self.app.layout_block(index, width, face) else {
            return Ok(());
        };
        let x = f64::from(event.client_x()) - row.get_bounding_client_rect().left();
        self.goal_x.set(None);
        self.set_caret(Caret {
            block: index,
            offset: layout.offset_at(line, x as f32),
        });
        self.dom.pane.focus()
    }
}

/// Which CSS class a block is drawn with. Structure only — the font is inline, from the face
/// that measured it.
fn class_of(kind: &BlockKind) -> String {
    match kind {
        BlockKind::Paragraph => "block p".to_owned(),
        BlockKind::Heading { level } => format!("block h h{}", level.clamp(&1, &6)),
        BlockKind::ListItem { .. } => "block li".to_owned(),
    }
}

/// How far a block's text is indented — a list's nesting, and nothing else.
fn indent_of(kind: &BlockKind) -> f64 {
    match kind {
        BlockKind::ListItem { depth } => f64::from(*depth) * INDENT * BODY_PX,
        _ => 0.0,
    }
}

fn attribute(element: &Element, name: &str) -> Option<usize> {
    element.get_attribute(name)?.parse().ok()
}

fn wire(ui: &Rc<Ui>) -> Result<(), JsValue> {
    let keys = ui.clone();
    listen(&ui.dom.pane, "keydown", move |event: KeyboardEvent| {
        keys.on_key(&event);
    })?;

    // One listener for the whole flow rather than one per line: the lines are rebuilt every
    // frame, and a listener each would be a listener each frame.
    let click = ui.clone();
    listen(&ui.dom.pane, "mousedown", move |event: MouseEvent| {
        if let Err(error) = click.on_click(&event) {
            web_sys::console::error_1(&error);
        }
    })
}

/// The space under a block, and the extra above a heading — written into the page once, so
/// the stylesheet and this file cannot disagree about it.
pub fn declare_spacing(document: &Document) -> Result<(), JsValue> {
    let Some(root) = document
        .document_element()
        .and_then(|e| e.dyn_into::<HtmlElement>().ok())
    else {
        return Ok(());
    };
    let style = root.style();
    style.set_property("--block-gap", &format!("{}px", (GAP * BODY_PX).round()))?;
    style.set_property(
        "--heading-gap",
        &format!("{}px", (HEADING_GAP * BODY_PX).round()),
    )?;
    style.set_property("--measure", &format!("{}px", (BODY_PX * 38.0).round()))
}

/// Nothing here is reachable without a browser, so what is testable on the host is the
/// vocabulary: which class a block gets, and how far it is indented.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_carries_its_kind_as_a_class() {
        assert_eq!(class_of(&BlockKind::Paragraph), "block p");
        assert_eq!(class_of(&BlockKind::Heading { level: 2 }), "block h h2");
        // A level the schema allows and this shell has no face for is still drawn.
        assert_eq!(class_of(&BlockKind::Heading { level: 9 }), "block h h6");
        assert_eq!(class_of(&BlockKind::ListItem { depth: 1 }), "block li");
    }

    #[test]
    fn only_a_list_item_is_indented_and_it_is_by_its_depth() {
        assert_eq!(indent_of(&BlockKind::Paragraph), 0.0);
        assert_eq!(indent_of(&BlockKind::Heading { level: 1 }), 0.0);
        assert_eq!(
            indent_of(&BlockKind::ListItem { depth: 2 }),
            2.0 * INDENT * BODY_PX
        );
    }

    /// The two shells that draw pixels must agree about the shape of a document, or the
    /// same file looks like two different ones.
    #[test]
    fn the_heading_scale_matches_the_gtk_shell() {
        assert_eq!(HEADING_SCALE, [1.8, 1.5, 1.3, 1.15, 1.05, 1.0]);
    }
}
