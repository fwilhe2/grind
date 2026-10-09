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
mod table;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use grind_core::style::TextStyle;
use grind_text::find::{self, Towards};
use grind_text::format::{self, Change, Landed};
use grind_text::look::Role;
use grind_text::markdown::Emphasis;
use grind_text::style::CharStyle;
use grind_text::{App, BlockKind, BlockView, Caret, Form, Metrics, loc};
use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, CompositionEvent, Document, Element, Event, HtmlCanvasElement,
    HtmlDialogElement, HtmlElement, HtmlInputElement, HtmlTextAreaElement, KeyboardEvent,
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
        // Every size and weight is `grind_text::look`'s, so a document has one shape in every
        // shell that draws a page. The code face is a pixel smaller than the body: a monospace
        // face at the same size reads larger, which is this page's own optical correction.
        let face =
            |role: Role| Face::new(ctx.clone(), (BODY_PX * role.scale()).round(), role.bold());
        Faces {
            body: face(Role::Body),
            headings: (1..=grind_text::look::HEADING_SCALE.len() as u8)
                .map(|level| face(Role::Heading(level)))
                .collect(),
            title: face(Role::Title),
            subtitle: face(Role::Subtitle),
            code: Face::in_family(ctx, BODY_PX - 1.0, false, MONO_FAMILY),
        }
    }

    /// The face a block is set in — [`Role::of`], which decides which name wins over which kind
    /// and what a heading deeper than six levels is set in, for every shell that draws a page.
    fn of(&self, kind: &BlockKind, style: Option<&str>) -> &Face {
        match Role::of(kind, style) {
            Role::Body => &self.body,
            Role::Heading(level) => &self.headings[usize::from(level) - 1],
            Role::Title => &self.title,
            Role::Subtitle => &self.subtitle,
            Role::Code => &self.code,
        }
    }
}

/// The measure and the face of **every** block — `grind_text::Faces`, for the motions that may
/// cross out of one block into another.
///
/// Down out of a heading lands in the paragraph below, which is set smaller; a list item's
/// indent comes out of the column, so it is also narrower. Both of those are this shell's own
/// arithmetic, and handing the core one width and one face for a whole motion is what used to
/// make Down-arrow out of a heading land a few characters from where a click would have.
struct Column<'a> {
    faces: &'a Faces,
    /// The flow's width, in the same CSS pixels [`Face`] answers in.
    width: f64,
    /// The width of every block that sits in a table cell ([`table::measures`]) — a cell's
    /// lines are broken at the cell's width, and a motion has to be answered at that width too.
    cells: HashMap<usize, f64>,
}

impl grind_text::Faces for Column<'_> {
    fn of(&self, index: usize, kind: &BlockKind, style: Option<&str>) -> (f32, &dyn Metrics) {
        let width = self.cells.get(&index).copied().unwrap_or(self.width);
        (
            (width - indent_of(kind)).max(1.0) as f32,
            self.faces.of(kind, style),
        )
    }
}

/// A table being drawn: its `table:name`, its grid element, and the cell elements made so far
/// by (row, column).
type OpenTable = (String, Element, HashMap<(u32, u32), Element>);

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
    /// The picture the alt text dialog is open on — where `picture::set_alt` writes when it
    /// closes on Save. Kept rather than re-asked, since the caret is not the dialog's to move.
    alt_at: Cell<Option<Caret>>,
    /// The word the palette last found, which F3 and Shift+F3 step through and Replace offers
    /// back — `sheet::Ui::needle`'s twin.
    needle: RefCell<String>,
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
            alt_at: Cell::new(None),
            message: RefCell::new(String::new()),
            needle: RefCell::new(String::new()),
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
        let caret = self.caret.get();
        let selection = self.selection();
        // ponytail: the whole document is rendered, not the part on screen. A grid needs a
        // viewport because a sheet has a million rows; a document has as many blocks as
        // somebody typed, and windowing one costs a scroll-position-to-block map that only
        // pays for itself on documents nobody has written yet. Named in `doc/text-shell.md`.
        let viewport = self.app.get_viewport(0..self.app.block_count());
        // The same lookup a click and a motion ask, so a cell's lines are drawn broken where
        // the caret will think they are.
        let column = self.column_over(&viewport);
        // The table being drawn, if the last block was in one: its name, its grid element, and
        // the cell elements made so far — a cell holding two paragraphs is two blocks, and both
        // go in the one cell.
        let mut open: Option<OpenTable> = None;
        let columns = table::columns(viewport.iter().filter_map(|block| block.cell.as_ref()));

        self.dom.flow.set_text_content(None);
        for block in viewport.iter() {
            let face = self.faces.of(&block.kind, block.style.as_deref());
            let indent = indent_of(&block.kind);
            let (measure, _) =
                grind_text::Faces::of(&column, block.index, &block.kind, block.style.as_deref());
            let Ok(layout) = self.app.layout_block(block.index, measure, face) else {
                continue;
            };

            let element = self.dom.document.create_element("div")?;
            element.set_class_name(&class_of(block));
            element.set_attribute("data-block", &block.index.to_string())?;
            element.set_attribute("style", &face.css(indent))?;
            // A list item's number or its style's bullet (`BlockView::mark`), drawn by the
            // stylesheet in front of it — never a character of the document.
            if let Some(mark) = block.mark() {
                element.set_attribute("data-label", mark)?;
            }

            // A block that is a picture is drawn as one, above whatever text it also holds
            // (a caption reads as the paragraph's own text — `doc/odt-format.md`).
            if let Some((at, image)) = block
                .runs
                .iter()
                .find_map(|run| Some((run.start, run.image.as_ref()?)))
            {
                let figure = self.dom.document.create_element("div")?;
                figure.set_class_name("figure");
                let picture = self.dom.document.create_element("img")?;
                picture.set_class_name("picture");
                // The document's own alternative text is the page's, too.
                picture.set_attribute("alt", image.title.as_deref().unwrap_or(""))?;
                picture.set_attribute("src", &self.image_url(block.index, image))?;
                figure.append_child(&picture)?;
                let badge = self.dom.document.create_element("button")?;
                badge.set_attribute("type", "button")?;
                badge.set_attribute("data-alt", &format!("{}:{at}", block.index))?;
                let (class, label, says) = match image.title {
                    Some(_) => ("alt-badge", "ALT", "Edit the alt text"),
                    None => ("alt-badge missing", "+ Alt text", "Add alt text"),
                };
                badge.set_class_name(class);
                badge.set_text_content(Some(label));
                badge.set_attribute("aria-label", says)?;
                badge.set_attribute("title", says)?;
                figure.append_child(&badge)?;
                element.append_child(&figure)?;
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
            let parent: Element = match &block.cell {
                None => {
                    open = None;
                    self.dom.flow.clone().into()
                }
                Some(cell) => {
                    if open.as_ref().is_none_or(|(name, ..)| *name != cell.table) {
                        let grid = self.dom.document.create_element("div")?;
                        grid.set_class_name("table");
                        let count = columns.get(&cell.table).copied().unwrap_or(1);
                        grid.set_attribute(
                            "style",
                            &format!("grid-template-columns:repeat({count}, minmax(0, 1fr))"),
                        )?;
                        self.dom.flow.append_child(&grid)?;
                        open = Some((cell.table.clone(), grid, HashMap::new()));
                    }
                    let Some((_, grid, cells)) = open.as_mut() else {
                        continue;
                    };
                    match cells.get(&(cell.row, cell.column)) {
                        Some(existing) => existing.clone(),
                        None => {
                            let box_ = self.dom.document.create_element("div")?;
                            box_.set_class_name("tcell");
                            box_.set_attribute("style", &table::placement(cell))?;
                            grid.append_child(&box_)?;
                            cells.insert((cell.row, cell.column), box_.clone());
                            box_
                        }
                    }
                }
            };
            parent.append_child(&element)?;
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

        // `doc/view-modes.md` §3.6: a bookmark contributes no characters, so a tick at its
        // own offset is the only way to say *exactly* where one is — the name at the end of
        // the line (below) says only which line. `ui_text_gtk`'s `x_at` twin: a boundary
        // between two pieces places it against its own kerning rather than in the middle of
        // whichever run it falls in.
        let anchors_here: Vec<usize> = match self.names.get() {
            true => block
                .marks
                .iter()
                .map(|(at, _)| *at)
                .filter(|at| line.contains(at))
                .collect(),
            false => Vec::new(),
        };

        let mut drawn = false;
        for piece in runs::cut(line.clone(), &block.runs, within, caret, &anchors_here) {
            if anchors_here.contains(&piece.range.start) {
                let tick = self.dom.document.create_element("span")?;
                tick.set_class_name("mark-tick");
                row.append_child(&tick)?;
            }
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
            let css = runs::css(&piece, crate::ink::page_is_dark());
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

    /// The status line, in words: what the caret is in and how long the document is —
    /// `Heading 1 · 90 words · 1 min read`, `Body text · 90 words · 1 min read · 12 characters
    /// selected`. It used to lead with the caret's address (`p1+0 · 90 words · 22 blocks`), which
    /// is the palette's to take and now its tooltip; `ui_text_gtk`'s status bar made the same
    /// change.
    fn render_chrome(&self) {
        let caret = self.caret.get();
        let counts = self.app.counts();
        let selected = match self.selection() {
            Some((from, to)) if from.block == to.block => match to.offset - from.offset {
                1 => " · 1 character selected".to_owned(),
                n => format!(" · {n} characters selected"),
            },
            Some((from, to)) => format!(" · {} paragraphs selected", to.block - from.block + 1),
            None => String::new(),
        };
        let here = self
            .block_at(caret.block)
            .map(|block| describe(&block.kind, block.style.as_deref()))
            .unwrap_or_default();
        let words = match (counts.words, counts.reading_minutes()) {
            (_, 0) => "0 words".to_owned(),
            (1, minutes) => format!("1 word · {minutes} min read"),
            (n, minutes) => format!("{n} words · {minutes} min read"),
        };
        let line = match here.is_empty() {
            true => format!("{words}{selected}"),
            false => format!("{here} · {words}{selected}"),
        };
        self.dom.summary.set_text_content(Some(&line));
        let _ = self.dom.summary.set_attribute(
            "title",
            &format!(
                "At {} — {} blocks",
                loc::format_offset(caret.block, caret.offset),
                counts.blocks
            ),
        );
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

    /// Select the word around the caret, or — with `block` — its whole paragraph.
    fn select_around(&self, block: bool) {
        let caret = self.caret.get();
        let text = self.app.input_text(caret.block).unwrap_or_default();
        let (start, end) = match block {
            true => (0, text.chars().count()),
            false => grind_text::word::around(&text, caret.offset),
        };
        self.anchor.set(Some(Caret {
            block: caret.block,
            offset: start,
        }));
        self.set_caret(Caret {
            block: caret.block,
            offset: end,
        });
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
            "char.bold" => self.toggle_char(Emphasis::Bold),
            "char.italic" => self.toggle_char(Emphasis::Italic),
            "char.underline" => self.toggle_char(Emphasis::Underline),
            "char.strike" => self.toggle_char(Emphasis::Strike),
            "char.code" => self.toggle_char(Emphasis::Code),
            "char.family" => self.ask_char("Font family — empty for the document's own", |name| {
                Change::Family(name)
            }),
            "char.size" => self
                .ask_char("Font size — 14pt, 1.2em; empty for the default", |size| {
                    Change::Size(size)
                }),
            "char.clear" => self.change_char(Change::Clear),
            "block.body" => self.set_kind(BlockKind::Paragraph, None),
            "block.title" => self.set_kind(BlockKind::Paragraph, Some("Title")),
            "block.subtitle" => self.set_kind(BlockKind::Paragraph, Some("Subtitle")),
            "block.h1" => self.set_kind(BlockKind::Heading { level: 1 }, None),
            "block.h2" => self.set_kind(BlockKind::Heading { level: 2 }, None),
            "block.h3" => self.set_kind(BlockKind::Heading { level: 3 }, None),
            "block.h4" => self.set_kind(BlockKind::Heading { level: 4 }, None),
            "block.h5" => self.set_kind(BlockKind::Heading { level: 5 }, None),
            "block.h6" => self.set_kind(BlockKind::Heading { level: 6 }, None),
            "block.list" => self.set_kind(BlockKind::ListItem { depth: 1 }, None),
            "block.up" => self.move_blocks(true),
            "block.down" => self.move_blocks(false),
            "block.delete" => self.delete_blocks(),
            "block.table" => self.insert_table(),
            "block.alt" => self.open_alt(),
            "block.bookmark" => self.bookmark(),
            "block.style" => self.name_style(),
            "block.indent" => self.renest(1),
            "block.outdent" => self.renest(-1),
            "edit.find-next" => self.find_step(Towards::Next),
            "edit.find-previous" => self.find_step(Towards::Previous),
            "edit.replace" => self.replace(),
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
            id => match (id.strip_prefix("goto:"), id.strip_prefix("find:")) {
                (Some(where_), _) => self.go_to(where_),
                (_, Some(found)) => self.pick_found(found),
                _ => self.set_message(format!("No such command: {id}")),
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

    // --- whole paragraphs ---

    /// The blocks a paragraph verb acts on: every one the selection touches, or the caret's.
    fn touched(&self) -> std::ops::RangeInclusive<usize> {
        match self.selection() {
            Some((from, to)) => from.block..=to.block,
            None => self.caret.get().block..=self.caret.get().block,
        }
    }

    /// *Move paragraph up / down* — `grind_text::blocks::shift`, caret and selection going along.
    fn move_blocks(&self, up: bool) {
        match grind_text::blocks::shift(&self.app, self.touched(), up) {
            Ok(()) => {
                let step = |caret: Caret| Caret {
                    block: if up { caret.block - 1 } else { caret.block + 1 },
                    ..caret
                };
                self.anchor.set(self.anchor.get().map(step));
                self.set_caret(step(self.caret.get()));
            }
            Err(why) => self.set_message(why),
        }
    }

    /// *Delete paragraph* — `grind_text::blocks::remove`, the caret at the start of what followed.
    fn delete_blocks(&self) {
        match grind_text::blocks::remove(&self.app, self.touched()) {
            Ok(block) => {
                self.anchor.set(None);
                self.set_caret(Caret { block, offset: 0 });
            }
            Err(why) => self.set_message(why),
        }
    }

    // --- pictures ---

    /// A picture's bytes below the caret's paragraph, the caret past it
    /// (`grind_text::picture::insert_below`). Anything the signature does not name as a picture is
    /// said so rather than embedded.
    pub fn insert_picture(&self, data: Vec<u8>) {
        let Some(mime) = grind_text::picture::mime(&data) else {
            return self.set_message(
                "That file is not a picture — a PNG, JPEG, GIF, WebP, BMP or SVG can go in"
                    .to_owned(),
            );
        };
        match grind_text::picture::insert_below(&self.app, self.caret.get().block, mime, data) {
            Ok(block) => {
                self.anchor.set(None);
                self.set_caret(Caret { block, offset: 1 });
                self.set_message(
                    "Picture inserted — its “+ Alt text” chip says what a screen reader will"
                        .to_owned(),
                );
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    // --- alt text ---

    /// *Alt text for the picture…* — `index.html`'s `#alt` dialog, opened on the picture beside
    /// the caret (`grind_text::picture::at`) with what it already says filled in.
    fn open_alt(&self) {
        let caret = self.caret.get();
        let Some(block) = self.block_at(caret.block) else {
            return;
        };
        let Some((offset, image)) = grind_text::picture::at(&block, caret.offset) else {
            return self.set_message("Put the caret beside a picture first".to_owned());
        };
        let Ok(parts) = self.alt_parts() else {
            return;
        };
        let (dialog, title, description) = parts;
        title.set_value(image.title.as_deref().unwrap_or(""));
        description.set_value(image.description.as_deref().unwrap_or(""));
        if let Some(more) = self.dom.document.get_element_by_id("alt-more") {
            match image.description.is_some() {
                true => more.set_attribute("open", ""),
                false => more.remove_attribute("open"),
            }
            .ok();
        }
        if let Some(shown) = self.dom.document.get_element_by_id("alt-picture") {
            let _ = shown.set_attribute("src", &self.image_url(caret.block, image));
        }
        self.alt_at.set(Some(Caret {
            block: caret.block,
            offset,
        }));
        self.advise_alt();
        dialog.set_return_value("");
        let _ = dialog.show_modal();
        let _ = title.focus();
        title.select();
    }

    /// The dialog and its two fields.
    fn alt_parts(
        &self,
    ) -> Result<(HtmlDialogElement, HtmlInputElement, HtmlTextAreaElement), JsValue> {
        let document = &self.dom.document;
        Ok((
            element(document, "alt")?,
            element(document, "alt-title")?,
            element(document, "alt-description")?,
        ))
    }

    /// The advice line and the count under the short field, as it is typed.
    fn advise_alt(&self) {
        let Ok((_, title, _)) = self.alt_parts() else {
            return;
        };
        let text = title.value();
        let document = &self.dom.document;
        if let Some(advice) = document.get_element_by_id("alt-advice") {
            advice.set_text_content(Some(grind_text::picture::advice(&text).unwrap_or("")));
        }
        if let Some(count) = document.get_element_by_id("alt-count") {
            let n = text.trim().chars().count();
            count.set_text_content(Some(&format!("{n} / {}", grind_text::picture::SHORT)));
            let _ = count
                .class_list()
                .toggle_with_force("over", n > grind_text::picture::SHORT);
        }
    }

    /// The dialog closed: written when it closed on Save, and nothing otherwise.
    fn close_alt(&self) {
        let Some(at) = self.alt_at.take() else {
            return;
        };
        let Ok((dialog, title, description)) = self.alt_parts() else {
            return;
        };
        if dialog.return_value() == "save" {
            match grind_text::picture::set_alt(&self.app, at, &title.value(), &description.value())
            {
                Ok(()) => self.set_message(match title.value().trim().is_empty() {
                    true => "The picture has no alt text".to_owned(),
                    false => "Alt text saved".to_owned(),
                }),
                Err(error) => self.set_message(error.to_string()),
            }
        }
        let _ = self.focus();
    }

    // --- markdown ---

    /// Markdown read in before the caret's block (`App::import_markdown`), one undo step.
    pub fn import_markdown(&self, markdown: &str) {
        let at = self.caret.get().block;
        // A page has no directory to read a picture from: only a `data:` URI comes in.
        match self
            .app
            .import_markdown(at, markdown, &grind_text::commonmark::nowhere)
        {
            Ok(n) => {
                self.anchor.set(None);
                self.set_caret(Caret {
                    block: at,
                    offset: 0,
                });
                self.set_message(format!("Imported {n} block(s) from Markdown"));
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// The selection's blocks, or the whole document, as CommonMark — its pictures inside it as
    /// `data:` URIs, since a download is one file and there is nowhere to put a second.
    pub fn export_markdown(&self) -> Option<String> {
        let blocks = match self.selection() {
            Some((from, to)) => from.block..to.block + 1,
            None => 0..self.app.block_count(),
        };
        match self
            .app
            .export_markdown(blocks, &grind_text::commonmark::Pictures::Inline)
        {
            Ok(exported) => Some(exported.markdown),
            Err(error) => {
                self.set_message(error.to_string());
                None
            }
        }
    }

    // --- tables ---

    /// *Insert a table…* — a size, then a table below the caret's block
    /// (`grind_text::table::insert_below`, the one answer to where it goes). One prompt that
    /// takes `3x4`, `3 4` or `3×4`: this page has no dialog surface, and two numbers are one
    /// question.
    fn insert_table(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(Some(answer)) =
            window.prompt_with_message_and_default("Table size — rows × columns", "3x3")
        else {
            return;
        };
        let Some((rows, columns)) = grind_text::table::parse_size(&answer) else {
            return self.set_message("A table size looks like 3x4 — rows, then columns".to_owned());
        };
        match grind_text::table::insert_below(&self.app, self.caret.get().block, rows, columns) {
            Ok(at) => {
                self.anchor.set(None);
                self.set_caret(Caret {
                    block: at,
                    offset: 0,
                });
                self.set_message(format!("A {rows}×{columns} table — Ctrl+Z takes it back"));
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    // --- bookmarks and named styles ---

    /// *Bookmark this paragraph…* — a name, then `App::set_bookmark` at the caret's block. A
    /// name already in use *moves* (the core's rule), so asking twice relocates rather than
    /// duplicates. The overlay goes on, because a bookmark contributes no characters and would
    /// otherwise leave nothing on the page to say it was made.
    fn bookmark(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(Some(name)) = window.prompt_with_message_and_default("Bookmark name", "") else {
            return;
        };
        let name = name.trim().trim_start_matches('#');
        if name.is_empty() {
            return;
        }
        match self.app.set_bookmark(name, Some(self.caret.get().block)) {
            Ok(moved) => {
                self.names.set(true);
                self.set_message(match moved {
                    true => format!("#{name} moved here"),
                    false => format!("#{name} anchored here"),
                });
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// *Name this paragraph's style…* — `App::set_style` over the selected blocks, an empty
    /// answer taking the name away. A named style is kept and never interpreted
    /// (`doc/text-core.md`), so this is how a document says *Quote* to a reader that has one.
    fn name_style(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let (from, to) = self
            .selection()
            .unwrap_or_else(|| (self.caret.get(), self.caret.get()));
        let current = self
            .block_at(from.block)
            .and_then(|block| block.style)
            .unwrap_or_default();
        let Ok(Some(name)) = window.prompt_with_message_and_default("Paragraph style", &current)
        else {
            return;
        };
        let name = name.trim();
        let style = (!name.is_empty()).then(|| name.to_owned());
        match self.app.set_style(from.block..to.block + 1, style) {
            Ok(_) => self.set_message(match name.is_empty() {
                true => "Style name removed".to_owned(),
                false => format!("Style “{name}” set"),
            }),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    // --- find and replace (`grind_text::find`) ---

    /// What the palette offers *after* the verbs for a query: the paragraphs holding it.
    ///
    /// After, not before like [`Ui::targets`] — a word somebody types is far more often a verb
    /// than a phrase of the text, and `bold` must still put *Bold* first. Two characters at
    /// least, since one letter is in every paragraph. When there are more hits than fit, the
    /// last row is all of them, stepped through with F3. The browser's own find would only see
    /// what is on screen, which is why Ctrl+F opens this box.
    pub fn found(&self, query: &str) -> Vec<Entry> {
        const SHOWN: usize = 5;
        let query = query.trim();
        if query.chars().count() < 2 {
            return Vec::new();
        }
        let hits = self.app.find_ignoring_case(query);
        let mut out: Vec<Entry> = hits
            .iter()
            .take(SHOWN)
            .map(|hit| {
                let text: String = hit.text.chars().take(60).collect();
                Entry::target(
                    format!("find:{}:{}\n{query}", hit.index, hit.offset),
                    format!("{} — {text}", hit.address()),
                    "Text",
                )
            })
            .collect();
        if hits.len() > SHOWN {
            out.push(Entry::target(
                format!("find:\n{query}"),
                format!(
                    "All {} places holding “{query}” — F3 steps through them",
                    hits.len()
                ),
                "Find",
            ));
        }
        out
    }

    /// A hit picked from [`Ui::found`]: select it and remember the word, so F3 carries on. No
    /// address means "the first hit from here", which is the *All N places* row.
    fn pick_found(&self, found: &str) {
        let Some((address, needle)) = found.split_once('\n') else {
            return;
        };
        *self.needle.borrow_mut() = needle.to_owned();
        let at = address.split_once(':').and_then(|(block, offset)| {
            Some(Caret {
                block: block.parse().ok()?,
                offset: offset.parse().ok()?,
            })
        });
        match at {
            Some(from) => self.select_hit(from, needle),
            None => self.find_step(Towards::Here),
        }
    }

    /// Select the hit that starts at `from`, so typing over it replaces it.
    fn select_hit(&self, from: Caret, needle: &str) {
        self.anchor.set(Some(from));
        self.set_caret(find::end_of(from, needle));
        let _ = self.dom.pane.focus();
    }

    /// F3 and Shift+F3: the next or previous occurrence of the remembered word, wrapping at
    /// either end. The hits are asked for again each time, since the document may have changed.
    fn find_step(&self, towards: Towards) {
        let needle = self.needle.borrow().clone();
        if needle.is_empty() {
            return self.set_message("Nothing to find yet — Ctrl+F, then type".to_owned());
        }
        let hits = find::hits(&self.app, &needle);
        // From the selection's start when there is one — usually the last hit — and the caret
        // otherwise.
        let at = self
            .selection()
            .map_or_else(|| self.caret.get(), |(from, _)| from);
        let Some(index) = find::step(&hits, at, towards) else {
            return self.set_message(format!("“{needle}” is not in the document"));
        };
        self.select_hit(hits[index], &needle);
        self.set_message(format!(
            "{} of {} · F3 next, Shift+F3 previous",
            index + 1,
            hits.len()
        ));
    }

    /// *Replace in the document…* — two questions, then `App::replace` over every paragraph in
    /// one undo step. Prompts rather than a form, as the spreadsheet's does: this page has no
    /// dialog surface, and a replace is two words. Exact in case, since it writes.
    fn replace(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let offered = self.needle.borrow().clone();
        let Ok(Some(what)) = window.prompt_with_message_and_default("Replace what?", &offered)
        else {
            return;
        };
        if what.is_empty() {
            return;
        }
        let Ok(Some(with)) =
            window.prompt_with_message_and_default(&format!("Replace “{what}” with"), "")
        else {
            return;
        };
        *self.needle.borrow_mut() = what.clone();
        match self.app.replace(&what, &with) {
            Ok(0) => self.set_message(format!("“{what}” is not in the document")),
            Ok(n) => {
                self.anchor.set(None);
                self.set_message(format!(
                    "Replaced in {n} paragraph{} — Ctrl+Z takes it back",
                    if n == 1 { "" } else { "s" }
                ));
            }
            Err(error) => self.set_message(error.to_string()),
        }
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
    fn toggle_char(&self, emphasis: Emphasis) {
        self.change_char(Change::toggle(emphasis, &self.style_here()));
    }

    /// One control of the tool row over the selection — or, with nothing selected, held for
    /// the next character typed at the caret, which is what Bold-then-type means in every word
    /// processor (`grind_text::format::apply`). The pending style is the `resume`
    /// [`App::type_markdown`] already carries, and moving the caret forgets it.
    fn change_char(&self, change: Change) {
        let pending = self.resume.borrow().clone();
        let landed = format::apply(
            &self.app,
            self.selection(),
            self.caret.get(),
            pending.as_ref(),
            &change,
        );
        match landed {
            Ok(Landed::Written) => {}
            Ok(Landed::Pending(style)) => {
                *self.resume.borrow_mut() = Some(style);
                let _ = self.refresh_tools();
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// Ask for a value in a prompt and apply it over the selection (or the next typed character):
    /// the family or the size, as the document stores them — `fo:font-family`'s name, an ODF
    /// length. Empty puts the document's own back.
    fn ask_char(&self, message: &str, change: impl Fn(Option<String>) -> Change) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(Some(answer)) = window.prompt_with_message_and_default(message, "") else {
            return;
        };
        let answer = answer.trim();
        self.change_char(change((!answer.is_empty()).then(|| answer.to_owned())));
    }

    /// What the tool row shows and what a toggle changes (`grind_text::format::here`).
    fn style_here(&self) -> CharStyle {
        let pending = self.resume.borrow().clone();
        format::here(
            &self.app,
            self.selection(),
            self.caret.get(),
            pending.as_ref(),
        )
    }

    /// A colour from the swatch grid — `"color"` for the letters, `"highlight"` for behind
    /// them.
    pub fn set_color(&self, target: &str, hex: Option<String>) {
        match target {
            "color" => self.change_char(Change::Color(hex)),
            "highlight" => self.change_char(Change::Highlight(hex)),
            _ => {}
        }
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
        // The toggles report what the selection agrees about, or — with nothing selected —
        // what the next character typed will carry, which is what pressing one there changes.
        let style = self.style_here();
        set_pressed(document, "t-bold", format::has(&style, Emphasis::Bold));
        set_pressed(document, "t-italic", format::has(&style, Emphasis::Italic));
        set_pressed(
            document,
            "t-underline",
            format::has(&style, Emphasis::Underline),
        );
        set_pressed(document, "t-strike", format::has(&style, Emphasis::Strike));
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
    /// Arithmetic rather than `scrollIntoView`, because the caret is only ever a *line* out of
    /// view and the browser's own version jumps the page around — and because a headless run
    /// reports every rectangle as zero, where this does nothing at all rather than throwing.
    ///
    /// The caret's place on the page is its client rectangle less the pane's, plus how far the
    /// pane is already scrolled. It used to be `offsetTop`, which is measured from the nearest
    /// *positioned* ancestor — a block (`.block` is `position: relative` for its bullet), not
    /// the pane — so every caret sat a few pixels from "the top", and PageDown walked the caret
    /// off the bottom of the screen with the page never following it.
    fn follow_caret(&self) {
        let Some(caret) = self
            .dom
            .document
            .get_element_by_id("caret")
            .and_then(|element| element.dyn_into::<HtmlElement>().ok())
        else {
            return;
        };
        let scroll = f64::from(self.dom.pane.scroll_top());
        let top = caret.get_bounding_client_rect().top()
            - self.dom.pane.get_bounding_client_rect().top()
            + scroll;
        let height = f64::from(caret.offset_height()).max(1.0);
        let view = f64::from(self.dom.pane.client_height());
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
            composing: event.is_composing(),
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

    /// What a composition finally produced, typed as if it had been one keystroke.
    ///
    /// **The only path a dead key has into this document.** `` ` ``, `´`, `^` and `~` are dead
    /// keys on German, French, Spanish and many other layouts, and while one is composing the
    /// browser reports `key == "Dead"` and then `isComposing` — never the character. The
    /// keymap refuses both (`keymap`'s module documentation) and the text arrives here
    /// instead, which is why `` `code` `` can be typed on those keyboards at all.
    ///
    /// It goes through [`Ui::type_text`] rather than [`Ui::insert_plain`], so the notation
    /// reads a composed backtick exactly as it reads one that came from a key — a composition
    /// is a way of *typing* a character, not a different kind of character.
    fn on_composed(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.erase_selection();
        self.type_text(text);
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
        let faces = self.column();
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
                    None => self.app.caret_x(caret, &faces).unwrap_or(0.0),
                };
                self.goal_x.set(Some(goal));
                self.app
                    .caret_line(caret, steps as isize * lines, goal, &faces)
                    .ok()
            }
            Motion::LineStart | Motion::LineEnd => self
                .app
                .caret_line_bounds(caret, &faces)
                .ok()
                .map(|(start, end)| match motion {
                    Motion::LineStart => start,
                    _ => end,
                }),
            Motion::DocStart => Some(grind_text::caret::START),
            Motion::DocEnd => Some(grind_text::caret::end(&self.app)),
        };
        let Some(moved) = moved else { return };
        if !matches!(motion, Motion::Line(_) | Motion::Page(_)) {
            self.goal_x.set(None);
        }
        self.set_caret(moved);
    }

    /// One character left or right, rolling onto the neighbouring block at either end —
    /// `grind_text::caret::step`, which every shell's page shares.
    fn stepped(&self, delta: i32) -> Caret {
        grind_text::caret::step(&self.app, self.caret.get(), delta)
    }

    /// Move the caret, forgetting any style pending for the next character — a Bold pressed
    /// with nothing selected belongs to where it was pressed. Typing is the one move that keeps
    /// it ([`Ui::place_caret`]).
    fn set_caret(&self, caret: Caret) {
        *self.resume.borrow_mut() = None;
        self.place_caret(caret);
    }

    fn place_caret(&self, caret: Caret) {
        self.caret.set(caret);
        // Nothing in the core changed, so nothing will tell the page to repaint.
        self.request_repaint();
    }

    fn block_len(&self, index: usize) -> usize {
        grind_text::caret::block_len(&self.app, index)
    }

    /// How each block is set — this pane's [`grind_text::Faces`], rebuilt per question because
    /// the flow's width is read from the DOM and can change under a resize between two of them.
    fn column(&self) -> Column<'_> {
        let viewport = self.app.get_viewport(0..self.app.block_count());
        self.column_over(&viewport)
    }

    /// [`Ui::column`] over a viewport the caller already read — the renderer's, which reads the
    /// whole document anyway.
    fn column_over(&self, viewport: &grind_text::Viewport) -> Column<'_> {
        let width = self.width();
        Column {
            faces: &self.faces,
            width,
            cells: table::measures(
                &viewport
                    .iter()
                    .map(|block| (block.index, block.cell.as_ref()))
                    .collect::<Vec<_>>(),
                width,
            ),
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
                self.place_caret(typed.caret);
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
        // Through the same lookup a motion uses, so a click and a Down-arrow cannot disagree
        // about which face this block is set in.
        let column = self.column();
        let block = self.block_at(index);
        let kind = block
            .as_ref()
            .map_or(BlockKind::Paragraph, |block| block.kind.clone());
        let style = block.as_ref().and_then(|block| block.style.clone());
        let (width, face) = grind_text::Faces::of(&column, index, &kind, style.as_deref());
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
        BlockKind::Heading { level } => format!("block.h{}", level.clamp(&1, &6)),
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

/// Where a picture's alt chip puts the caret: `data-alt` is `block:offset`, the picture's own.
fn chip_caret(at: &str) -> Option<Caret> {
    let (block, offset) = at.split_once(':')?;
    Some(Caret {
        block: block.parse().ok()?,
        offset: offset.parse().ok()?,
    })
}

fn wire(ui: &Rc<Ui>) -> Result<(), JsValue> {
    let keys = ui.clone();
    listen(&ui.dom.pane, "keydown", move |event: KeyboardEvent| {
        // Enter or Space on a picture's alt chip is the chip's, not a new paragraph.
        let chip = event
            .target()
            .and_then(|t| t.dyn_into::<Element>().ok())
            .and_then(|t| t.get_attribute("data-alt"))
            .and_then(|at| chip_caret(&at));
        if let (Some(at), "Enter" | " ") = (chip, event.key().as_str()) {
            event.prevent_default();
            keys.set_caret(at);
            return keys.open_alt();
        }
        keys.on_key(&event);
    })?;

    // The other half of the keyboard. A composition — a dead key, and as much of an input
    // method as a page with no `contenteditable` gets — produces its character here rather
    // than in a `keydown`, and the keymap turns away every keystroke belonging to one so this
    // is the only place it is typed.
    let composed = ui.clone();
    listen(
        &ui.dom.pane,
        "compositionend",
        move |event: CompositionEvent| {
            composed.on_composed(&event.data().unwrap_or_default());
        },
    )?;

    // One listener for the whole flow rather than one per line: the lines are rebuilt every
    // frame, and a listener each would be a listener each frame.
    let click = ui.clone();
    listen(&ui.dom.pane, "mousedown", move |event: MouseEvent| {
        // The browser's own text selection would otherwise start alongside this one, and the
        // two would disagree about where it is.
        event.prevent_default();
        // A picture's alt chip: the caret goes beside that picture and the dialog opens on it.
        if let Some(at) = event
            .target()
            .and_then(|t| t.dyn_into::<Element>().ok())
            .and_then(|t| t.closest("[data-alt]").ok().flatten())
            .and_then(|chip| chip.get_attribute("data-alt"))
            .and_then(|at| chip_caret(&at))
        {
            click.anchor.set(None);
            click.set_caret(at);
            click.open_alt();
            return;
        }
        click.dragging.set(true);
        if let Err(error) = click.on_click(&event, event.shift_key()) {
            web_sys::console::error_1(&error);
        }
        // A second press selects the word, a third the paragraph — `detail` is the browser's
        // own count, so the double-click speed is the reader's. The word is
        // `grind_text::word::around`'s, so this pane and `grind-text-gtk` agree where one ends.
        if event.detail() >= 2 && !event.shift_key() {
            click.dragging.set(false);
            click.select_around(event.detail() >= 3);
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

    // The alt text dialog: advice as it is typed, Enter on a focused chip, Cancel, and the
    // write when it closes — on Save, which is the form's one submit button, or not at all.
    let (dialog, title, _) = ui.alt_parts()?;
    let typed = ui.clone();
    listen(&title, "input", move |_: Event| typed.advise_alt())?;
    let closed = ui.clone();
    listen(&dialog, "close", move |_: Event| closed.close_alt())?;
    let cancel: HtmlElement = element(&ui.dom.document, "alt-cancel")?;
    listen(&cancel, "click", move |_: Event| {
        dialog.close_with_return_value("cancel")
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

/// What the status line calls a block — the tool row's own labels (`index.html`'s `#t-block`),
/// so the two never name one thing twice. A named style that is not one of the two this pane
/// draws is shown after it, with ODF's `_20_` escaping undone.
fn describe(kind: &BlockKind, style: Option<&str>) -> String {
    let name = match (kind, style) {
        (BlockKind::Paragraph, Some(named @ ("Title" | "Subtitle"))) => return named.to_owned(),
        (BlockKind::Paragraph, _) => "Body text".to_owned(),
        (BlockKind::Heading { level }, _) => format!("Heading {level}"),
        (BlockKind::ListItem { depth: 1 }, _) => "List item".to_owned(),
        (BlockKind::ListItem { depth }, _) => format!("List item, level {depth}"),
    };
    match style {
        Some(style) => format!("{name} — {}", grind_text::style::readable_name(style)),
        None => name,
    }
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
    fn the_status_line_names_a_block_the_way_the_tool_row_does() {
        assert_eq!(describe(&BlockKind::Paragraph, None), "Body text");
        assert_eq!(
            describe(&BlockKind::Heading { level: 2 }, None),
            "Heading 2"
        );
        assert_eq!(describe(&BlockKind::Paragraph, Some("Title")), "Title");
        assert_eq!(
            describe(&BlockKind::Paragraph, Some("Text_20_body")),
            "Body text — Text body"
        );
    }

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
            // This pane draws no tables yet (`doc/web-shell.md`), so a fixture never is in one.
            cell: None,
            generated: false,
            label: None,
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
            "block.h6"
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
}
