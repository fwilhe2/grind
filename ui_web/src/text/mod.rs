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
mod runs;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use grind_core::style::TextStyle;
use grind_text::style::CharStyle;
use grind_text::{App, BlockKind, BlockView, Caret, Form, Metrics, loc};
use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, Document, Element, HtmlCanvasElement, HtmlElement, KeyboardEvent,
    MouseEvent,
};

use crate::command::Entry;
use crate::{element, listen, request_repaint, set_pressed, set_select, set_swatch};
use keymap::{Action, Chord, Motion};

/// The document's typeface, owned **here** rather than in the stylesheet.
///
/// Two declarations that must agree are two declarations that will not — the same trap
/// `declare_cell_size` fixed for the grid. So the font is written into each block's inline
/// style as it is rendered, and the canvas measures with the identical string. A serif at
/// this size because this is a page of prose, not a user interface.
const FAMILY: &str = "Georgia, 'Times New Roman', serif";
/// What a fenced block and a `` `code` `` run are set in. A generic first, so the reader's own
/// monospace face wins — which one that is, is theirs to know.
const MONO_FAMILY: &str = "ui-monospace, monospace";
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
        Self::in_family(ctx, size, bold, FAMILY)
    }

    fn in_family(
        ctx: Option<Rc<CanvasRenderingContext2d>>,
        size: f64,
        bold: bool,
        family: &str,
    ) -> Self {
        let weight = match bold {
            true => "bold ",
            false => "",
        };
        Face {
            ctx,
            font: format!("{weight}{size}px {family}"),
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

/// How much bigger than the body a `Title` and a `Subtitle` are — the two *named* paragraph
/// styles this shell gives a face of their own, matching `grind-text-gtk`'s.
const TITLE_SCALE: f64 = 2.2;
const SUBTITLE_SCALE: f64 = 1.3;

/// The faces a document is set in: body text, one per heading level, and the two named
/// paragraph styles that are not headings.
struct Faces {
    body: Face,
    headings: Vec<Face>,
    title: Face,
    subtitle: Face,
    /// A fenced code block (`grind_text::markdown::PREFORMATTED`). A *face* rather than a CSS
    /// rule, because the same object is what measures the block — a monospace paragraph drawn
    /// in one font and measured in another would break every line in the wrong place.
    code: Face,
}

impl Faces {
    fn new(ctx: Option<Rc<CanvasRenderingContext2d>>) -> Self {
        Faces {
            body: Face::new(ctx.clone(), BODY_PX, false),
            headings: HEADING_SCALE
                .iter()
                .map(|scale| Face::new(ctx.clone(), (BODY_PX * scale).round(), true))
                .collect(),
            title: Face::new(ctx.clone(), (BODY_PX * TITLE_SCALE).round(), true),
            subtitle: Face::new(ctx.clone(), (BODY_PX * SUBTITLE_SCALE).round(), false),
            code: Face::in_family(ctx, BODY_PX - 1.0, false, MONO_FAMILY),
        }
    }

    /// The face a block is set in.
    ///
    /// **The named style is checked before the kind**, because `Title` and `Subtitle` are both
    /// `BlockKind::Paragraph` with nothing else to key a face off — the same order
    /// `ui_text_gtk`'s `Faces::of` uses, so a document has one shape in both windows.
    ///
    /// A heading deeper than the six levels there are faces for is set as the last of them
    /// rather than refused: the reader is tolerant (R5), so a level-9 heading loads, and a
    /// shell that panicked on one would undo that.
    fn of(&self, block: &BlockView) -> &Face {
        match block.style.as_deref() {
            Some("Title") => return &self.title,
            Some("Subtitle") => return &self.subtitle,
            Some(grind_text::markdown::PREFORMATTED) => return &self.code,
            _ => {}
        }
        self.for_kind(&block.kind)
    }

    fn for_kind(&self, kind: &BlockKind) -> &Face {
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
    /// Where a selection started, if there is one. The caret is its other end, so extending
    /// with Shift is a move that leaves this alone — the same two-ends shape the grid's own
    /// `Selection` has, and for the same reason.
    anchor: Cell<Option<Caret>>,
    /// The column the caret is trying to keep while moving by lines — see
    /// [`App::caret_line`]. Cleared by any horizontal move.
    goal_x: Cell<Option<f32>>,
    /// What `App::type_markdown` said the next character must be set in, so a notation ends
    /// where its closing marker does. Handed straight back and never read here.
    resume: RefCell<Option<CharStyle>>,
    /// Whether the pointer is down and dragging out a selection.
    dragging: Cell<bool>,
    /// Whether the bookmark anchors are drawn — `doc/view-modes.md` §3.6. Presentation
    /// state: it is a reading of the document rather than a change to it.
    names: Cell<bool>,
    /// Pictures already turned into a `data:` URL, by the block they are in.
    ///
    /// ponytail: keyed by *index*, so an insertion above an image invalidates nothing and the
    /// picture under the old index is drawn for one frame before the next repaint corrects it.
    /// A `BlockId` would be exact; it is not on `RunView`, and the cost of being wrong is one
    /// frame of the wrong picture in a document with two images in it.
    images: RefCell<HashMap<usize, String>>,
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
            anchor: Cell::new(None),
            goal_x: Cell::new(None),
            resume: RefCell::new(None),
            dragging: Cell::new(false),
            names: Cell::new(false),
            images: RefCell::new(HashMap::new()),
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
        self.anchor.set(None);
        self.goal_x.set(None);
        *self.resume.borrow_mut() = None;
        self.images.borrow_mut().clear();
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
        let selection = self.selection();
        // ponytail: the whole document is rendered, not the part on screen. A grid needs a
        // viewport because a sheet has a million rows; a document has as many blocks as
        // somebody typed, and windowing one costs a scroll-position-to-block map that only
        // pays for itself on documents nobody has written yet. Named in `doc/text-shell.md`.
        let viewport = self.app.get_viewport(0..self.app.block_count());

        self.dom.flow.set_text_content(None);
        for block in viewport.iter() {
            let face = self.faces.of(block);
            let indent = indent_of(&block.kind);
            let Ok(layout) = self
                .app
                .layout_block(block.index, (width - indent) as f32, face)
            else {
                continue;
            };

            let element = self.dom.document.create_element("div")?;
            element.set_class_name(&class_of(block));
            element.set_attribute("data-block", &block.index.to_string())?;
            element.set_attribute("style", &face.css(indent))?;

            // A block that is a picture is drawn as one, above whatever text it also holds
            // (a caption reads as the paragraph's own text — `doc/odt-format.md`).
            if let Some(image) = block.runs.iter().find_map(|run| run.image.as_ref()) {
                let picture = self.dom.document.create_element("img")?;
                picture.set_class_name("picture");
                picture.set_attribute("alt", "")?;
                picture.set_attribute("src", &self.image_url(block.index, image))?;
                element.append_child(&picture)?;
            }

            for (number, line) in layout.lines().iter().enumerate() {
                let row = self.dom.document.create_element("div")?;
                row.set_class_name("line");
                row.set_attribute("data-line", &number.to_string())?;
                // An empty line still occupies one: a `<div>` with nothing in it is zero
                // pixels tall, however tall its line height says it is.
                row.set_attribute("style", &format!("height:{}px", face.height))?;

                // `line_at` rather than `Line::holds`: an offset at a soft break belongs to
                // both lines, and drawing the caret on both is drawing it twice.
                let here = (block.index == caret.block && layout.line_at(caret.offset) == number)
                    .then_some(caret.offset);
                self.draw_line(&row, block, line.start..line.end, &selection, here)?;
                // `doc/view-modes.md` §3.6: a bookmark contributes no characters, so it is
                // the one part of a text document a reader cannot see at all. With the mode
                // on, each one is named on the line it falls on — after the text rather than
                // inside it, because an offset inside the line is an offset the caret counts
                // and a mark drawn there would move it.
                if self.names.get() {
                    for (at, name) in &block.marks {
                        if !(line.start..line.end.max(line.start + 1)).contains(at) {
                            continue;
                        }
                        let mark = self.dom.document.create_element("span")?;
                        mark.set_class_name("mark-name");
                        mark.set_text_content(Some(&format!("\u{2039}{name}\u{203a}")));
                        row.append_child(&mark)?;
                    }
                }
                element.append_child(&row)?;
            }
            self.dom.flow.append_child(&element)?;
        }

        self.render_chrome();
        self.follow_caret();
        Ok(())
    }

    /// One line's worth of `<span>`s — the document's formatting, the selection and the caret,
    /// each of which cuts the line somewhere the others do not ([`runs::cut`]).
    fn draw_line(
        &self,
        row: &Element,
        block: &BlockView,
        line: std::ops::Range<usize>,
        selection: &Option<(Caret, Caret)>,
        caret: Option<usize>,
    ) -> Result<(), JsValue> {
        let text: Vec<char> = block.text.chars().collect();
        // The selection, clipped to *this block* — it may start pages above and end below.
        let within = selection.as_ref().and_then(|(from, to)| {
            (from.block <= block.index && block.index <= to.block).then(|| {
                let start = match from.block == block.index {
                    true => from.offset,
                    false => 0,
                };
                let end = match to.block == block.index {
                    true => to.offset,
                    false => text.len(),
                };
                start..end
            })
        });

        let mut drawn = false;
        for piece in runs::cut(line.clone(), &block.runs, within, caret) {
            if caret == Some(piece.range.start) {
                self.append_caret(row)?;
                drawn = true;
            }
            let slice: String = text
                [piece.range.start.min(text.len())..piece.range.end.min(text.len())]
                .iter()
                .collect();
            // A line break is where the line ends; it is not a character to draw.
            let slice = slice.trim_end_matches('\n');
            let span = self.dom.document.create_element("span")?;
            span.set_class_name(&runs::classes(&piece));
            let css = runs::css(&piece);
            if !css.is_empty() {
                span.set_attribute("style", &css)?;
            }
            span.set_text_content(Some(slice));
            row.append_child(&span)?;
        }
        // At the end of the line, and in an empty one — neither is the start of any piece.
        if caret.is_some() && !drawn {
            self.append_caret(row)?;
        }
        Ok(())
    }

    fn append_caret(&self, row: &Element) -> Result<(), JsValue> {
        let caret = self.dom.document.create_element("span")?;
        caret.set_class_name("caret");
        caret.set_id("caret");
        row.append_child(&caret)?;
        Ok(())
    }

    /// A picture as a `data:` URL, encoded once and remembered.
    ///
    /// A `blob:` URL would avoid the base64 pass, and would then have to be revoked — a
    /// lifetime this shell has nowhere to keep, since every frame throws the whole page away.
    /// A `data:` URL is owned by the element that carries it.
    fn image_url(&self, block: usize, image: &grind_text::ImageView) -> String {
        if let Some(url) = self.images.borrow().get(&block) {
            return url.clone();
        }
        let url = format!("data:{};base64,{}", image.mime, base64(&image.data));
        self.images.borrow_mut().insert(block, url.clone());
        url
    }

    fn render_chrome(&self) {
        let caret = self.caret.get();
        let counts = self.app.counts();
        let selected = match self.selection() {
            Some((from, to)) if from.block == to.block => {
                format!(" · {} selected", to.offset - from.offset)
            }
            Some((from, to)) => format!(" · {} blocks selected", to.block - from.block + 1),
            None => String::new(),
        };
        self.dom.summary.set_text_content(Some(&format!(
            "{} · {} words · {} blocks{selected}",
            loc::format_offset(caret.block, caret.offset),
            counts.words,
            counts.blocks
        )));
        self.dom
            .message
            .set_text_content(Some(&self.message.borrow()));
    }

    // --- the selection ---

    /// The selection, in document order, or `None` when the anchor is where the caret is —
    /// which is what "nothing is selected" *is*, rather than a second state to keep in step.
    pub fn selection(&self) -> Option<(Caret, Caret)> {
        let anchor = self.anchor.get()?;
        let caret = self.caret.get();
        if anchor == caret {
            return None;
        }
        Some(
            match (anchor.block, anchor.offset) <= (caret.block, caret.offset) {
                true => (anchor, caret),
                false => (caret, anchor),
            },
        )
    }

    /// Erase whatever is selected, leaving the caret where it started. Every edit that
    /// *replaces* a selection begins here — typing, Enter, and paste.
    pub fn erase_selection(&self) -> bool {
        let Some((from, to)) = self.selection() else {
            return false;
        };
        match self.app.erase(from, to) {
            Ok(_) => {
                self.anchor.set(None);
                self.set_caret(from);
                true
            }
            Err(error) => {
                self.set_message(error.to_string());
                false
            }
        }
    }

    fn select_all(&self) {
        let blocks = self.app.block_count();
        if blocks == 0 {
            return;
        }
        self.anchor.set(Some(Caret {
            block: 0,
            offset: 0,
        }));
        self.set_caret(Caret {
            block: blocks - 1,
            offset: self.block_len(blocks - 1),
        });
    }

    // --- commands ---

    /// Every verb this pane answers ([`crate::command::TEXT`]).
    pub fn run(&self, id: &str) {
        match id {
            "edit.select-all" => self.select_all(),
            "char.bold" => self.toggle_char(|s| &mut s.font_weight, "bold", "normal"),
            "char.italic" => self.toggle_char(|s| &mut s.font_style, "italic", "normal"),
            "char.underline" => self.toggle_char(|s| &mut s.underline, "solid", "none"),
            "char.strike" => self.toggle_char(|s| &mut s.line_through, "solid", "none"),
            "char.clear" => self.set_char_style(CharStyle::default()),
            "block.body" => self.set_kind(BlockKind::Paragraph, None),
            "block.title" => self.set_kind(BlockKind::Paragraph, Some("Title")),
            "block.subtitle" => self.set_kind(BlockKind::Paragraph, Some("Subtitle")),
            "block.h1" => self.set_kind(BlockKind::Heading { level: 1 }, None),
            "block.h2" => self.set_kind(BlockKind::Heading { level: 2 }, None),
            "block.h3" => self.set_kind(BlockKind::Heading { level: 3 }, None),
            "block.h4" => self.set_kind(BlockKind::Heading { level: 4 }, None),
            "block.list" => self.set_kind(BlockKind::ListItem { depth: 1 }, None),
            "block.indent" => self.renest(1),
            "block.outdent" => self.renest(-1),
            "view.names" => {
                let on = !self.names.get();
                self.names.set(on);
                self.set_message(match on {
                    true => "Bookmarks are shown where they anchor — nothing was written; \
                             run it again to stop"
                        .to_owned(),
                    false => "Bookmarks are invisible again".to_owned(),
                });
            }
            id => match id.strip_prefix("goto:") {
                Some(where_) => self.go_to(where_),
                None => self.set_message(format!("No such command: {id}")),
            },
        }
    }

    /// What the palette offers for a query that is not a verb: a heading to jump to, a
    /// bookmark, or an address.
    ///
    /// **This is the outline dialog and the go-to field, in the box that was already there.**
    /// `doc/text-shell.md` named both as the browser pane's next candidates; one palette
    /// answers both without a second dialog to open, style and keep accessible.
    pub fn targets(&self, query: &str) -> Vec<Entry> {
        let query = query.trim();
        if query.is_empty() {
            // With nothing typed, the outline *is* the useful list — a document's own table of
            // contents, which is what somebody who opened the palette in a long report wants.
            return self
                .app
                .outline()
                .into_iter()
                .take(8)
                .map(|heading| {
                    Entry::target(
                        format!("goto:{}", heading.index),
                        format!(
                            "{}{}",
                            "  ".repeat(heading.level.saturating_sub(1) as usize),
                            heading.text
                        ),
                        "Outline",
                    )
                })
                .collect();
        }
        let lower = query.to_lowercase();
        let mut out: Vec<Entry> = self
            .app
            .outline()
            .into_iter()
            .filter(|heading| heading.text.to_lowercase().contains(&lower))
            .take(6)
            .map(|heading| {
                Entry::target(
                    format!("goto:{}", heading.index),
                    heading.text.clone(),
                    "Outline",
                )
            })
            .collect();
        for (name, index) in self.app.bookmarks() {
            if name.to_lowercase().contains(&lower) {
                out.push(Entry::target(
                    format!("goto:{index}"),
                    format!("#{name}"),
                    "Bookmark",
                ));
            }
        }
        // `p12`, `#intro`, `§2.1.3` — `loc`'s own vocabulary, so what the CLI accepts the
        // palette accepts.
        if out.is_empty()
            && let Ok(at) = loc::parse(query)
            && let Ok(index) = self.app.resolve(&at)
        {
            out.push(Entry::target(
                format!("goto:{index}"),
                format!("Go to {query}"),
                "Go",
            ));
        }
        out.truncate(6);
        out
    }

    // --- the code view (doc/dsl.md §6, D9) ---

    /// The document as its projection, for the code view.
    pub fn project(&self) -> grind_text::projection::Projection {
        self.app.project()
    }

    /// Which block the caret is in, spelled the way the span map spells it. `p12` — the address
    /// every block has, whatever else it also answers to.
    pub fn projection_address(&self) -> Option<String> {
        Some(grind_text::loc::format(self.caret.get().block))
    }

    /// Put the caret in whatever block a code-view line projects.
    ///
    /// The span map may hand back `p12`, `#intro` or `§2.1.3`, and `loc::parse` takes all three,
    /// so this needs no vocabulary of its own — which is `loc.rs` earning its keep for the third
    /// time.
    /// What the document says about itself (`doc/dsl.md` §4.3, D6).
    pub fn lint(&self) -> grind_core::lint::Report {
        self.app.lint(&grind_core::lint::Options::default())
    }

    pub fn select_projected(&self, address: &str) {
        let Ok(caret) = grind_text::loc::parse(address)
            .map_err(|e| e.to_string())
            .and_then(|loc| self.app.resolve_caret(&loc).map_err(|e| e.to_string()))
        else {
            return;
        };
        self.anchor.set(None);
        self.set_caret(caret);
    }

    fn go_to(&self, where_: &str) {
        let Ok(index) = where_.parse::<usize>() else {
            return;
        };
        if index >= self.app.block_count() {
            return;
        }
        self.anchor.set(None);
        self.set_caret(Caret {
            block: index,
            offset: 0,
        });
        let _ = self.dom.pane.focus();
    }

    // --- formatting ---

    /// Turn one character property on across the selection, or off when it is already on
    /// everywhere in it — [`App::char_style`] is what "already on everywhere" means, since it
    /// reports only what the whole span *agrees* about.
    fn toggle_char(&self, field: fn(&mut CharStyle) -> &mut Option<String>, on: &str, off: &str) {
        let Some((from, to)) = self.selection() else {
            return self.set_message("Select some text first".to_owned());
        };
        let mut style = self.app.char_style(from, to).unwrap_or_default();
        let slot = field(&mut style);
        let already = slot.as_deref().is_some_and(|value| value != off);
        *slot = match already {
            true => Some(off.to_owned()),
            false => Some(on.to_owned()),
        };
        self.write_char_style(from, to, &style);
    }

    fn set_char_style(&self, style: CharStyle) {
        let Some((from, to)) = self.selection() else {
            return self.set_message("Select some text first".to_owned());
        };
        self.write_char_style(from, to, &style);
    }

    fn write_char_style(&self, from: Caret, to: Caret, style: &CharStyle) {
        if let Err(error) = self.app.set_char_style(from, to, style) {
            self.set_message(error.to_string());
        }
    }

    /// A colour from the swatch grid — `"color"` for the letters, `"highlight"` for behind
    /// them.
    pub fn set_color(&self, target: &str, hex: Option<String>) {
        let Some((from, to)) = self.selection() else {
            return self.set_message("Select some text first".to_owned());
        };
        let mut style = self.app.char_style(from, to).unwrap_or_default();
        match target {
            "color" => style.color = hex,
            "highlight" => style.background = hex,
            _ => return,
        }
        self.write_char_style(from, to, &style);
    }

    /// Change what the block under the caret *is* — every block the selection touches, since
    /// "make these three paragraphs headings" is one thing to ask for.
    fn set_kind(&self, kind: BlockKind, style: Option<&str>) {
        let (first, last) = match self.selection() {
            Some((from, to)) => (from.block, to.block),
            None => (self.caret.get().block, self.caret.get().block),
        };
        for index in first..=last {
            if let Err(error) = self.app.set_kind(index, kind.clone()) {
                return self.set_message(error.to_string());
            }
        }
        if let Err(error) = self
            .app
            .set_style(first..last + 1, style.map(str::to_owned))
        {
            self.set_message(error.to_string());
        }
    }

    /// One list level in or out. Only a list item has a depth to change, which is why Tab
    /// types a tab everywhere else.
    fn renest(&self, by: i32) {
        let index = self.caret.get().block;
        let Some(block) = self.block_at(index) else {
            return;
        };
        let BlockKind::ListItem { depth } = block.kind else {
            return self.set_message("Only a list item is nested".to_owned());
        };
        let depth = (depth as i32 + by).clamp(1, 9) as u32;
        if let Err(error) = self.app.set_kind(index, BlockKind::ListItem { depth }) {
            self.set_message(error.to_string());
        }
    }

    /// Show what the caret — or the selection — already is, on the tool row.
    pub fn refresh_tools(&self) -> Result<(), JsValue> {
        let document = &self.dom.document;
        // The toggles report what the *selection* agrees about, which is what
        // `App::char_style` answers. With nothing selected they read plain — honestly, since
        // the toggles are also disabled in that state: this shell formats a selection, and
        // there is no pending style a caret carries into the next keystroke.
        let style = match self.selection() {
            Some((from, to)) => self.app.char_style(from, to).unwrap_or_default(),
            None => CharStyle::default(),
        };
        let on =
            |value: &Option<String>, off: &str| value.as_deref().is_some_and(|value| value != off);
        set_pressed(document, "t-bold", on(&style.font_weight, "normal"));
        set_pressed(document, "t-italic", on(&style.font_style, "normal"));
        set_pressed(document, "t-underline", on(&style.underline, "none"));
        set_pressed(document, "t-strike", on(&style.line_through, "none"));
        set_swatch(document, "t-color-bar", style.color.as_deref());
        set_swatch(document, "t-highlight-bar", style.background.as_deref());

        let block = self.block_at(self.caret.get().block);
        set_select(document, "t-block", &named_block(block.as_ref()));
        Ok(())
    }

    // --- the clipboard ---

    /// The selected text, as plain text. Formatting is not carried: the clipboard this shell
    /// writes is the one every other application reads, and a run's own `CharStyle` has no
    /// spelling in `text/plain`.
    pub fn clipboard_text(&self) -> Option<String> {
        let (from, to) = self.selection()?;
        let mut out = String::new();
        for index in from.block..=to.block {
            let text = self.app.input_text(index).ok()?;
            let chars: Vec<char> = text.chars().collect();
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

    /// Text in, at the caret — replacing the selection if there is one. A newline splits a
    /// block, which is what pasting two paragraphs has to mean in a model that has no
    /// character for one.
    pub fn paste_text(&self, text: &str) {
        self.erase_selection();
        for (index, line) in text.replace("\r\n", "\n").split('\n').enumerate() {
            if index > 0 {
                self.split();
            }
            if !line.is_empty() {
                self.insert_plain(line);
            }
        }
    }

    fn block_at(&self, index: usize) -> Option<BlockView> {
        self.app.get_viewport(index..index + 1).get(index).cloned()
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
            Action::Move { motion, extend } => self.go(motion, extend),
            Action::Type(text) => {
                self.erase_selection();
                self.type_text(text);
            }
            Action::Split => {
                self.erase_selection();
                self.split();
            }
            Action::EraseBack => {
                if !self.erase_selection() {
                    self.erase_back();
                }
            }
            Action::EraseForward => {
                if !self.erase_selection() {
                    self.erase_forward();
                }
            }
            // A tab nests a list item and types a character anywhere else — the one key whose
            // meaning this pane decides rather than the keymap.
            Action::Tab { back } => match self.block_at(self.caret.get().block) {
                Some(block) if matches!(block.kind, BlockKind::ListItem { .. }) => {
                    self.renest(if back { -1 } else { 1 })
                }
                _ if !back => {
                    self.erase_selection();
                    self.type_text("\t");
                }
                _ => {}
            },
            // Everything else is a command, and takes the same path a palette row does — the
            // chrome answers its own and hands the rest back to `Ui::run`.
            Action::Run(id) => crate::run_command(id),
        }
        Ok(())
    }

    /// Every motion, routed to the core. `extend` keeps the anchor where it is, which is what
    /// makes Shift+arrow a selection rather than a move.
    fn go(&self, motion: Motion, extend: bool) {
        if self.app.block_count() == 0 {
            return;
        }
        match extend {
            // The anchor is set on the *first* extending move, from wherever the caret was.
            true => {
                if self.anchor.get().is_none() {
                    self.anchor.set(Some(self.caret.get()));
                }
            }
            false => self.anchor.set(None),
        }
        let caret = self.caret.get();
        let kind = self.kind_at(caret.block);
        let face = self.face_at(caret.block);
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
        self.block_at(index)
            .map(|block| block.kind)
            .unwrap_or(BlockKind::Paragraph)
    }

    /// The face a block is set in — the *whole* block, so a `Title` is measured as one.
    fn face_at(&self, index: usize) -> &Face {
        match self.block_at(index) {
            Some(block) => self.faces.of(&block),
            None => &self.faces.body,
        }
    }

    // --- editing ---

    /// A typed character, read as **markdown-shaped notation** as it lands
    /// (`grind_text::markdown`): `**bold**` becomes bold and its markers go, `` `code` ``
    /// becomes monospace, `# ` makes the block a heading, ``` fences a code paragraph.
    ///
    /// The reading is `App::type_markdown`'s, so this pane, the terminal and the CLI agree
    /// about what `**` means — and it is one action, so one Ctrl+Z takes back the whole of
    /// `**bold**`. Pasting deliberately does *not* go through it ([`Ui::paste_text`]): text
    /// arriving from a clipboard is text, and turning somebody's asterisks into formatting
    /// they did not ask for is not a favour.
    fn type_text(&self, text: &str) {
        // A document with no blocks at all has nowhere to put a character, and the first
        // thing anybody does with an empty page is type into it.
        if self.app.block_count() == 0 {
            match self.app.insert(0, BlockKind::Paragraph, "") {
                Ok(()) => self.set_caret(Caret {
                    block: 0,
                    offset: 0,
                }),
                Err(error) => return self.set_message(error.to_string()),
            }
        }
        let caret = self.caret.get();
        // Cloned out first: a `borrow()` in the scrutinee lives for the whole `match`, and the
        // arm below takes a `borrow_mut()` of the same cell.
        let resume = self.resume.borrow().clone();
        match self.app.type_markdown(caret, text, resume.as_ref()) {
            Ok(typed) => {
                self.goal_x.set(None);
                *self.resume.borrow_mut() = typed.resume;
                self.set_caret(typed.caret);
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// Text in at the caret with no notation read — what pasting and the register use.
    fn insert_plain(&self, text: &str) {
        let caret = self.caret.get();
        match self.app.insert_text(caret, text) {
            Ok(()) => {
                self.goal_x.set(None);
                *self.resume.borrow_mut() = None;
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
    ///
    /// `extend` is Shift held, or the pointer still down from a press — the two ways every
    /// editor grows a selection, and they end in the same place here.
    fn on_click(&self, event: &MouseEvent, extend: bool) -> Result<(), JsValue> {
        let Some(caret) = self.caret_at(event)? else {
            return Ok(());
        };
        match extend {
            true => {
                if self.anchor.get().is_none() {
                    self.anchor.set(Some(self.caret.get()));
                }
            }
            false => self.anchor.set(None),
        }
        self.goal_x.set(None);
        self.set_caret(caret);
        self.dom.pane.focus()
    }

    /// The caret a pointer position names, or `None` when it landed on nothing — the gap
    /// between two blocks, or the chrome.
    fn caret_at(&self, event: &MouseEvent) -> Result<Option<Caret>, JsValue> {
        let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return Ok(None);
        };
        let (Some(row), Some(block)) = (target.closest(".line")?, target.closest("[data-block]")?)
        else {
            return Ok(None);
        };
        let (Some(line), Some(index)) = (
            attribute(&row, "data-line"),
            attribute(&block, "data-block"),
        ) else {
            return Ok(None);
        };
        let kind = self.kind_at(index);
        let face = self.face_at(index);
        let width = (self.width() - indent_of(&kind)) as f32;
        let Ok(layout) = self.app.layout_block(index, width, face) else {
            return Ok(None);
        };
        let x = f64::from(event.client_x()) - row.get_bounding_client_rect().left();
        Ok(Some(Caret {
            block: index,
            offset: layout.offset_at(line, x as f32),
        }))
    }
}

/// Which CSS class a block is drawn with. Structure only — the font is inline, from the face
/// that measured it.
fn class_of(block: &BlockView) -> String {
    let mut class = match &block.kind {
        BlockKind::Paragraph => "block p".to_owned(),
        BlockKind::Heading { level } => format!("block h h{}", level.clamp(&1, &6)),
        BlockKind::ListItem { .. } => "block li".to_owned(),
    };
    // A named style this shell gives a face to is a class too, so the stylesheet can space it
    // — the two that are not headings and would otherwise be indistinguishable paragraphs.
    if let Some(style @ ("Title" | "Subtitle")) = block.style.as_deref() {
        class.push_str(&format!(" {}", style.to_lowercase()));
    }
    // ODF's own name for a code paragraph, which is what ``` fences (`grind_text::markdown`).
    if block.style.as_deref() == Some(grind_text::markdown::PREFORMATTED) {
        class.push_str(" pre");
    }
    class
}

/// Which option of the paragraph-style `<select>` a block *is* — the command id, so the
/// toolbar reports in the same vocabulary it commands in.
fn named_block(block: Option<&BlockView>) -> String {
    let Some(block) = block else {
        return "block.body".to_owned();
    };
    match block.style.as_deref() {
        Some("Title") => return "block.title".to_owned(),
        Some("Subtitle") => return "block.subtitle".to_owned(),
        _ => {}
    }
    match &block.kind {
        BlockKind::Heading { level } => format!("block.h{}", level.clamp(&1, &4)),
        BlockKind::ListItem { .. } => "block.list".to_owned(),
        BlockKind::Paragraph => "block.body".to_owned(),
    }
}

/// Bytes as base64, for a picture's `data:` URL.
///
/// Written out rather than pulled in: it is fifteen lines, the alternative is a dependency in
/// a `wasm` bundle that every reader downloads, and `window.btoa` would need the bytes turned
/// into a string of code points first — which is the same loop with an extra copy.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = |at: usize| u32::from(chunk.get(at).copied().unwrap_or(0));
        let triple = (b(0) << 16) | (b(1) << 8) | b(2);
        for shift in [18, 12, 6, 0] {
            let sextet = ((triple >> shift) & 0x3f) as usize;
            // The last group is padded rather than truncated: `=` is how a decoder is told
            // how many of the final bits are real.
            let pad = match shift {
                6 => chunk.len() < 2,
                0 => chunk.len() < 3,
                _ => false,
            };
            out.push(match pad {
                true => '=',
                false => ALPHABET[sextet] as char,
            });
        }
    }
    out
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
        // The browser's own text selection would otherwise start alongside this one, and the
        // two would disagree about where it is.
        event.prevent_default();
        click.dragging.set(true);
        if let Err(error) = click.on_click(&event, event.shift_key()) {
            web_sys::console::error_1(&error);
        }
    })?;

    // Dragging: every move with the button down extends the selection, which is the same
    // "press, move, release" gesture `ui_text_gtk` gets from `GestureDrag`.
    let drag = ui.clone();
    listen(&ui.dom.pane, "mousemove", move |event: MouseEvent| {
        if !drag.dragging.get() {
            return;
        }
        if let Err(error) = drag.on_click(&event, true) {
            web_sys::console::error_1(&error);
        }
    })?;

    // On the *window*, not the pane: a drag that ends outside it still ends.
    let Some(window) = web_sys::window() else {
        return Ok(());
    };
    let release = ui.clone();
    listen(&window, "mouseup", move |_: MouseEvent| {
        release.dragging.set(false);
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

    fn block(kind: BlockKind, style: Option<&str>) -> BlockView {
        BlockView {
            index: 0,
            id: grind_text::BlockId(0),
            kind,
            style: style.map(str::to_owned),
            text: String::new(),
            runs: Vec::new(),
            styled: false,
            marks: Vec::new(),
        }
    }

    #[test]
    fn a_block_carries_its_kind_as_a_class() {
        assert_eq!(class_of(&block(BlockKind::Paragraph, None)), "block p");
        assert_eq!(
            class_of(&block(BlockKind::Heading { level: 2 }, None)),
            "block h h2"
        );
        // A level the schema allows and this shell has no face for is still drawn.
        assert_eq!(
            class_of(&block(BlockKind::Heading { level: 9 }, None)),
            "block h h6"
        );
        assert_eq!(
            class_of(&block(BlockKind::ListItem { depth: 1 }, None)),
            "block li"
        );
        // The two named styles that are not headings carry their name as well.
        assert_eq!(
            class_of(&block(BlockKind::Paragraph, Some("Title"))),
            "block p title"
        );
        // A named style this shell has no face for is still an ordinary paragraph.
        assert_eq!(
            class_of(&block(BlockKind::Paragraph, Some("Quotations"))),
            "block p"
        );
    }

    /// The toolbar reports in the same vocabulary it commands in, so what it shows and what
    /// pressing it would do cannot drift apart.
    #[test]
    fn the_style_picker_names_the_command_that_would_produce_the_block() {
        assert_eq!(named_block(None), "block.body");
        assert_eq!(
            named_block(Some(&block(BlockKind::Heading { level: 3 }, None))),
            "block.h3"
        );
        assert_eq!(
            named_block(Some(&block(BlockKind::Paragraph, Some("Subtitle")))),
            "block.subtitle"
        );
        assert_eq!(
            named_block(Some(&block(BlockKind::ListItem { depth: 2 }, None))),
            "block.list"
        );
        // Deeper than the picker offers, clamped to the deepest it does.
        assert_eq!(
            named_block(Some(&block(BlockKind::Heading { level: 9 }, None))),
            "block.h4"
        );
    }

    /// Checked against a known vector rather than against itself.
    #[test]
    fn base64_pads_the_last_group() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(
            base64(b"any carnal pleasure"),
            "YW55IGNhcm5hbCBwbGVhc3VyZQ=="
        );
        // A PNG's own first bytes, which is what this is actually for.
        assert_eq!(base64(&[0x89, b'P', b'N', b'G']), "iVBORw==");
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
