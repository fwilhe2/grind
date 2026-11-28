// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor half of the browser shell. **The DOM is a renderer, not the document**: no
//! `contenteditable`, which would be a second copy of the text with its own paragraphs and undo
//! stack. The document is drawn as its printed pages (`text/paged.rs`, `doc/page-model.md`),
//! set by the print module, which is loaded with the first text document; over them, the caret,
//! the selection and the marks, and beside them a reader layer with the words on screen
//! (`Ui::reader`) for a screen reader and the browser's find.

pub mod keymap;
mod paged;
mod runs;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use grind_text::find::{self, Towards};
use grind_text::format::{self, Change, Landed};
use grind_text::markdown::Emphasis;
use grind_text::style::CharStyle;
use grind_text::{App, BlockKind, BlockView, Caret, Form, loc};
use wasm_bindgen::prelude::*;
use web_sys::{
    CompositionEvent, Document, Element, Event, HtmlDialogElement, HtmlElement, HtmlInputElement,
    HtmlTextAreaElement, InputEvent, KeyboardEvent, MouseEvent, PointerEvent,
};

use crate::command::Entry;
use crate::{element, listen, request_repaint, set_pressed, set_select, set_swatch};
use grind_text::caret::{Editor, Intent, Motion};
use keymap::{Action, Chord};

/// How wide and tall the pane is taken to be before the browser has laid it out: jsdom reports
/// every rectangle as zero.
const UNMEASURED: f64 = 640.0;

/// CSS pixels per point at which the pages are shown: a page at its printed size.
const PAGE_SCALE: f64 = 96.0 / 72.0;

/// The elements this pane writes to. No document state — that is all in the core.
struct Dom {
    document: Document,
    /// The scrolling box, and the thing that holds the keyboard.
    pane: HtmlElement,
    /// The text column inside it: its width is the measure every line is broken at.
    flow: HtmlElement,
    /// What a touch screen's keyboard types into: a phone shows its keyboard only for an
    /// editable element, and the pane is a `<div>`.
    typing: HtmlTextAreaElement,
    message: HtmlElement,
    summary: HtmlElement,
}

impl Dom {
    fn find(document: &Document) -> Result<Self, JsValue> {
        Ok(Dom {
            document: document.clone(),
            pane: element(document, "page")?,
            flow: element(document, "flow")?,
            typing: element(document, "typing")?,
            message: element(document, "message")?,
            summary: element(document, "summary")?,
        })
    }
}

pub struct Ui {
    pub app: Arc<App>,
    dom: Dom,
    pending: Arc<AtomicBool>,
    /// How many times the document has changed, counted by its observer.
    changes: Arc<AtomicU64>,
    /// The printed pages, once the print module has loaded (`doc/page-model.md` decision 3).
    paged: RefCell<Option<paged::Paged>>,
    // Everything below is *presentation*. None of it is the document, which is why the core
    // neither knows nor keeps it.
    /// The caret, the selection, the goal column and the style pending for the next character,
    /// and every edit at them.
    editor: RefCell<Editor>,
    /// Whether the pointer is down and dragging out a selection.
    dragging: Cell<bool>,
    /// Whether the last press was a finger, so the keyboard goes to [`Dom::typing`].
    touch: Cell<bool>,
    /// Whether the bookmark anchors are drawn — `doc/view-modes.md` §3.6. Presentation
    /// state: it is a reading of the document rather than a change to it.
    names: Cell<bool>,
    /// Pictures already turned into a `data:` URL, by block.
    ///
    /// ponytail: keyed by index, so an insertion above an image draws the old index's picture for
    /// one frame. A `BlockId` would be exact, but is not on `RunView`.
    images: RefCell<HashMap<usize, String>>,
    message: RefCell<String>,
    /// The picture the alt text dialog is open on — where `picture::set_alt` writes when it
    /// closes on Save. Kept rather than re-asked, since the caret is not the dialog's to move.
    alt_at: Cell<Option<Caret>>,
    /// The word the palette last found, which F3 and Shift+F3 step through and Replace offers
    /// back.
    needle: RefCell<String>,
}

impl Ui {
    pub fn new(
        document: &Document,
        app: Arc<App>,
        pending: Arc<AtomicBool>,
        changes: Arc<AtomicU64>,
    ) -> Result<Rc<Self>, JsValue> {
        let ui = Rc::new(Ui {
            app,
            dom: Dom::find(document)?,
            pending,
            changes,
            paged: RefCell::new(None),
            editor: RefCell::default(),
            dragging: Cell::new(false),
            touch: Cell::new(false),
            names: Cell::new(false),
            images: RefCell::new(HashMap::new()),
            alt_at: Cell::new(None),
            message: RefCell::new(String::new()),
            needle: RefCell::new(String::new()),
        });
        ui.apply_grammar(&crate::grammar::load());
        wire(&ui)?;
        Ok(ui)
    }

    /// The pane takes the keyboard, or after a finger's press the field a phone's keyboard
    /// types into.
    pub fn focus(&self) -> Result<(), JsValue> {
        if self.touch.get() {
            self.dom.typing.focus()
        } else {
            self.dom.pane.focus()
        }
    }

    /// A document arrived: into the core, and the presentation state a new one resets.
    pub fn open(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.app
            .open_bytes(name, bytes)
            .map_err(|e| e.to_string())?;
        self.editor.replace(Editor::default());
        self.images.borrow_mut().clear();
        self.dom.pane.set_scroll_top(0);
        if let Some(paged) = self.paged.borrow_mut().as_mut() {
            paged.forget();
        }
        Ok(())
    }

    /// Show the document as its printed pages from here on, `pages` being the print module's
    /// `Pages` with the fonts already in it.
    pub fn attach(&self, pages: JsValue) {
        self.dom.flow.set_text_content(None);
        self.dom.flow.set_class_name("flow paged");
        *self.paged.borrow_mut() = Some(paged::Paged::new(pages));
        self.request_repaint();
    }

    /// Whether the document is shown as its pages.
    pub fn is_paged(&self) -> bool {
        self.paged.borrow().is_some()
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

    /// The pages once the print module has set them; until then, a line saying so.
    fn render(&self) -> Result<(), JsValue> {
        if self.is_paged() {
            return self.render_pages();
        }
        self.dom.flow.set_class_name("flow waiting");
        self.dom.flow.set_text_content(Some("Setting the pages…"));
        self.render_chrome();
        Ok(())
    }

    /// The document as its pages: each a canvas the print module drew, and over them what is
    /// not on paper — the selection, spelling and grammar underlines, bookmark names, the caret.
    fn render_pages(&self) -> Result<(), JsValue> {
        let mut slot = self.paged.borrow_mut();
        let Some(paged) = slot.as_mut() else {
            return Ok(());
        };
        paged.sync(self.changes.load(Ordering::SeqCst), || {
            self.app.save_bytes(Form::Flat).ok()
        });
        let pane = &self.dom.pane;
        let width = match pane.client_width() {
            0 => UNMEASURED,
            width => f64::from(width),
        };
        // Never wider than the window: a narrow one, a phone's, shows the page whole and smaller.
        let scale = paged.fitting(PAGE_SCALE, width);
        paged.place(scale, width);
        let (strip_width, strip_height) = paged.size();
        let flow = &self.dom.flow;
        let style = flow.style();
        style.set_property("width", &format!("{strip_width}px"))?;
        style.set_property("height", &format!("{strip_height}px"))?;
        let top = f64::from(pane.scroll_top());
        let bottom = top
            + match pane.client_height() {
                0 => UNMEASURED,
                height => f64::from(height),
            };
        let ratio = web_sys::window().map_or(1.0, |window| window.device_pixel_ratio());
        paged.draw(&self.dom.document, flow, (top, bottom), scale, ratio)?;

        for old in ["marks", "reader"] {
            if let Some(old) = self.dom.document.get_element_by_id(old) {
                old.remove();
            }
        }
        let marks = self.dom.document.create_element("div")?;
        marks.set_id("marks");
        marks.set_class_name("marks");
        let mark = |class: &str, [x, y, w, h]: paged::Rect| -> Result<Element, JsValue> {
            let element = self.dom.document.create_element("div")?;
            element.set_class_name(class);
            element.set_attribute(
                "style",
                &format!("left:{x}px;top:{y}px;width:{w}px;height:{h}px"),
            )?;
            marks.append_child(&element)?;
            Ok(element)
        };
        let caret = self.caret();
        let selection = self.selection();
        if let Some((from, to)) = selection {
            for band in paged.bands(from, to, top, bottom) {
                mark("band sel", band)?;
            }
        }
        let at = |block: usize, offset: usize| Caret { block, offset };
        if let Some(blocks) = paged.blocks(top, bottom) {
            for wrong in self.app.misspellings(blocks.clone()) {
                if selection.is_none() && wrong.being_typed(caret) {
                    continue;
                }
                let (from, to) = (
                    at(wrong.block, wrong.offset),
                    at(wrong.block, wrong.offset + wrong.len),
                );
                for band in paged.bands(from, to, top, bottom) {
                    mark("band misspelt-mark", band)?;
                }
            }
            for flag in self.grammar_flags(blocks.clone()) {
                let range = &flag.finding.range;
                if selection.is_none() && flag.block == caret.block && range.end == caret.offset {
                    continue;
                }
                let (start, end) = if range.is_empty() {
                    (range.start.saturating_sub(1), range.end + 1)
                } else {
                    (range.start, range.end)
                };
                let quiet = matches!(
                    flag.finding.kind,
                    grind_text::grammar::Kind::Typography | grind_text::grammar::Kind::Style
                );
                let class = if quiet {
                    "band quiet-mark"
                } else {
                    "band grammar-mark"
                };
                for band in paged.bands(at(flag.block, start), at(flag.block, end), top, bottom) {
                    mark(class, band)?;
                }
            }
            // A bookmark contributes no characters (`doc/view-modes.md` §3.6): a tick where it
            // anchors, and its name past the end of its line.
            if self.names.get() {
                let viewport = self.app.get_viewport(blocks.clone());
                for index in blocks {
                    let Some(block) = viewport.get(index) else {
                        continue;
                    };
                    for (offset, name) in &block.marks {
                        let anchor = at(index, *offset);
                        let Some([x, y, h, _]) = paged.spot(anchor) else {
                            continue;
                        };
                        mark("mark-tick paged", [x - 1.0, y, 2.0, h])?;
                        let end = paged
                            .bounds(anchor)
                            .and_then(|(_, end)| paged.spot(end))
                            .map_or(x, |[x, ..]| x);
                        let label = mark("mark-name paged", [end, y, 0.0, h])?;
                        label.set_text_content(Some(&format!("\u{2039}{name}\u{203a}")));
                    }
                }
            }
        }
        if selection.is_none()
            && let Some([x, y, h, _]) = paged.spot(caret)
        {
            mark("caret paged", [x, y, 0.0, h])?.set_id("caret");
            // The keyboard's field sits on the caret, so an input method's word is shown there.
            let keys = self.dom.typing.style();
            let (left, top) = (self.dom.flow.offset_left(), self.dom.flow.offset_top());
            keys.set_property("left", &format!("{}px", f64::from(left) + x))?;
            keys.set_property("top", &format!("{}px", f64::from(top) + y))?;
            keys.set_property("height", &format!("{h}px"))?;
        }
        if let Some(blocks) = paged.blocks(top, bottom) {
            // A picture's way to its alt text: a chip over its corner, quiet once it has some.
            for block in self.app.get_viewport(blocks.clone()).iter() {
                for run in &block.runs {
                    let Some(image) = &run.image else { continue };
                    let Some([x, y, _, _]) = paged.spot(at(block.index, run.start)) else {
                        continue;
                    };
                    let (class, label, says) = match image.title {
                        Some(_) => ("alt-badge", "ALT", "Edit the alt text"),
                        None => ("alt-badge missing", "+ Alt text", "Add alt text"),
                    };
                    let badge = self.dom.document.create_element("button")?;
                    badge.set_class_name(class);
                    badge.set_attribute("type", "button")?;
                    badge.set_attribute("data-alt", &format!("{}:{}", block.index, run.start))?;
                    badge
                        .set_attribute("style", &format!("left:{}px;top:{}px", x + 6.0, y + 6.0))?;
                    badge.set_text_content(Some(label));
                    badge.set_attribute("aria-label", says)?;
                    badge.set_attribute("title", says)?;
                    marks.append_child(&badge)?;
                }
            }
            let reader = self.reader(blocks, caret, selection.is_some())?;
            flow.append_child(&reader)?;
        }
        flow.append_child(&marks)?;
        drop(slot);
        self.render_chrome();
        self.follow_caret();
        Ok(())
    }

    /// The text of the blocks on screen, for whoever cannot see the pages — a screen reader, the
    /// browser's find, a test: each block's words with their formatting as classes, a misspelt
    /// word and a grammar finding marked as ARIA marks them. Laid out nowhere.
    fn reader(
        &self,
        blocks: std::ops::Range<usize>,
        caret: Caret,
        selecting: bool,
    ) -> Result<Element, JsValue> {
        let reader = self.dom.document.create_element("div")?;
        reader.set_id("reader");
        reader.set_class_name("reader");
        let wrong = self.app.misspellings(blocks.clone());
        let flagged = self.grammar_flags(blocks.clone());
        let dark = crate::ink::page_is_dark();
        for block in self.app.get_viewport(blocks).iter() {
            let tag = match &block.kind {
                BlockKind::Heading { level } => format!("h{}", (*level).clamp(1, 6)),
                _ => "p".to_owned(),
            };
            let element = self.dom.document.create_element(&tag)?;
            element.set_class_name(&class_of(block));
            element.set_attribute("data-block", &block.index.to_string())?;
            if let Some(image) = block.runs.iter().find_map(|run| run.image.as_ref()) {
                let picture = self.dom.document.create_element("img")?;
                picture.set_class_name("picture");
                picture.set_attribute("alt", image.title.as_deref().unwrap_or(""))?;
                element.append_child(&picture)?;
            }
            // The word still being typed is not marked: a caret at its end is a word not finished.
            let typing =
                |end: usize| !selecting && caret.block == block.index && caret.offset == end;
            let misspelt: Vec<std::ops::Range<usize>> = wrong
                .iter()
                .filter(|w| w.block == block.index && !typing(w.offset + w.len))
                .map(|w| w.offset..w.offset + w.len)
                .collect();
            let flags: Vec<(std::ops::Range<usize>, bool)> = flagged
                .iter()
                .filter(|f| f.block == block.index && !typing(f.finding.range.end))
                .map(|f| {
                    let range = &f.finding.range;
                    let quiet = matches!(
                        f.finding.kind,
                        grind_text::grammar::Kind::Typography | grind_text::grammar::Kind::Style
                    );
                    (
                        range.start.saturating_sub(usize::from(range.is_empty()))..range.end,
                        quiet,
                    )
                })
                .collect();
            let marks = runs::Marks {
                misspelt: &misspelt,
                flagged: &flags,
            };
            let text: Vec<char> = block.text.chars().collect();
            for piece in runs::cut(0..text.len(), &block.runs, &marks) {
                let span = self.dom.document.create_element("span")?;
                span.set_class_name(&runs::classes(&piece));
                let css = runs::css(&piece, dark);
                if !css.is_empty() {
                    span.set_attribute("style", &css)?;
                }
                if piece.wrong {
                    span.set_attribute("aria-invalid", "spelling")?;
                } else if piece.grammar.is_some() {
                    span.set_attribute("aria-invalid", "grammar")?;
                }
                let slice: String = text[piece.range.start..piece.range.end.min(text.len())]
                    .iter()
                    .collect();
                span.set_text_content(Some(&slice));
                element.append_child(&span)?;
            }
            reader.append_child(&element)?;
        }
        Ok(reader)
    }

    /// A picture as a `data:` URL, encoded once and remembered. A `blob:` URL would skip the
    /// base64 pass but must be revoked, a lifetime nothing here keeps when every frame discards
    /// the page.
    fn image_url(&self, block: usize, image: &grind_text::ImageView) -> String {
        if let Some(url) = self.images.borrow().get(&block) {
            return url.clone();
        }
        let url = format!("data:{};base64,{}", image.mime, base64(&image.data));
        self.images.borrow_mut().insert(block, url.clone());
        url
    }

    /// The status line, in words: what the caret is in and how long the document is — `Heading 1 ·
    /// 90 words · 1 min read`, `… · 12 characters selected`. The caret's address is the palette's
    /// tooltip.
    fn render_chrome(&self) {
        let caret = self.caret();
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
            .map_or_default(|block| describe(&block.kind, block.style.as_deref()));
        let words = match (counts.words, counts.reading_minutes()) {
            (_, 0) => "0 words".to_owned(),
            (1, minutes) => format!("1 word · {minutes} min read"),
            (n, minutes) => format!("{n} words · {minutes} min read"),
        };
        let line = if here.is_empty() {
            format!("{words}{selected}")
        } else {
            format!("{here} · {words}{selected}")
        };
        let line = match self.page_here() {
            Some((page, count)) => format!("Page {page} of {count} · {line}"),
            None => line,
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

    /// The printed page the caret is on, from 1, and how many there are — when the document
    /// is shown as its pages.
    fn page_here(&self) -> Option<(usize, usize)> {
        let paged = self.paged.borrow();
        let paged = paged.as_ref()?;
        let [_, y, h, _] = paged.spot(self.caret())?;
        let page = paged.visible(y + h / 2.0, y + h / 2.0).start;
        Some((page + 1, paged.count()))
    }

    // --- the selection ---

    /// The selection, in document order, or `None` when nothing is selected.
    pub fn selection(&self) -> Option<(Caret, Caret)> {
        self.editor.borrow().selection()
    }

    /// Erase whatever is selected, leaving the caret where it started — the clipboard's cut.
    pub fn erase_selection(&self) -> bool {
        self.edit(Editor::drop_selection).unwrap_or(false)
    }

    /// Select the word around the caret, or — with `block` — its whole paragraph.
    fn select_around(&self, block: bool) {
        let caret = self.caret();
        self.edit(|editor, app| {
            if block {
                editor.select_block(app, caret);
            } else {
                editor.select_word(app, caret);
            }
            Ok(())
        });
    }

    fn select_all(&self) {
        self.edit(|editor, app| {
            editor.select_all(app);
            Ok(())
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
            "doc.page-setup" => self.page_setup(),
            "view.names" => {
                let on = !self.names.get();
                self.names.set(on);
                self.set_message(if on {
                    "Bookmarks are shown where they anchor — nothing was written; \
                     run it again to stop"
                        .to_owned()
                } else {
                    "Bookmarks are invisible again".to_owned()
                });
            }
            id => match (id.strip_prefix("goto:"), id.strip_prefix("find:")) {
                (Some(where_), _) => self.go_to(where_),
                (_, Some(found)) => self.pick_found(found),
                _ => self.set_message(format!("No such command: {id}")),
            },
        }
    }

    /// What the palette offers for a query that is not a verb: a heading, a bookmark or an address
    /// to jump to — the outline and the go-to field in one box.
    pub fn targets(&self, query: &str) -> Vec<Entry> {
        let mut out = self.grammar_entries(query);
        out.extend(self.spelling_entries(query));
        out.extend(self.heading_targets(query));
        out
    }

    fn heading_targets(&self, query: &str) -> Vec<Entry> {
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
                Entry::target(format!("goto:{}", heading.index), heading.text, "Outline")
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

    // --- spelling (`crate::spelling`) ---

    /// The misspelt word the caret is in or at the end of, if the checker has found one.
    pub fn misspelt_here(&self) -> Option<grind_text::Misspelling> {
        let at = self.caret();
        self.app
            .misspellings(at.block..at.block + 1)
            .into_iter()
            .find(|m| m.offset <= at.offset && at.offset <= m.offset + m.len)
    }

    /// The palette's spelling rows for the word at the caret: what it might have been, and
    /// whether to leave it alone. Empty with the caret anywhere else.
    fn spelling_entries(&self, query: &str) -> Vec<Entry> {
        let Some(wrong) = self.misspelt_here() else {
            return Vec::new();
        };
        crate::spelling::offers(&wrong.word, &self.app.suggest(&wrong.word), query)
    }

    /// Replace the word at the caret with the `n`th suggestion — one undo step, the word's
    /// formatting kept (`App::correct`).
    pub fn fix_misspelt(&self, n: usize) {
        let Some(wrong) = self.misspelt_here() else {
            return self.set_message("No misspelt word here".to_owned());
        };
        let Some(with) = self.app.suggest(&wrong.word).into_iter().nth(n) else {
            return self.set_message(format!("No suggestion {} for {}", n + 1, wrong.word));
        };
        match self.app.correct(wrong.caret(), &wrong.word, &with) {
            Ok(()) => {
                self.move_to(Caret {
                    block: wrong.block,
                    offset: wrong.offset + with.chars().count(),
                });
                self.set_message(format!("{} \u{2192} {with}", wrong.word));
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// Put the caret on the next misspelt word, wrapping round the end — and say what it might
    /// have been.
    pub fn next_misspelling(&self) {
        let all = self.app.misspellings(0..self.app.block_count());
        let at = self.caret();
        let after = |m: &grind_text::Misspelling| (m.block, m.offset) > (at.block, at.offset);
        let Some(wrong) = all.iter().find(|m| after(m)).or_else(|| all.first()) else {
            return self.set_message("Nothing is misspelt".to_owned());
        };
        self.move_to(wrong.caret());
        let suggestions: Vec<String> = self.app.suggest(&wrong.word).into_iter().take(3).collect();
        self.set_message(if suggestions.is_empty() {
            format!("{} \u{2014} no suggestion", wrong.word)
        } else {
            format!("{} \u{2014} {}", wrong.word, suggestions.join(", "))
        });
    }

    // --- grammar (`crate::grammar`) ---

    /// Attach the built-in checker as `setting` says, its ignored rules included.
    pub fn apply_grammar(&self, setting: &grind_grammar::setting::Setting) {
        self.app.set_checker(grind_grammar::checker(setting));
    }

    fn grammar_flags(&self, blocks: std::ops::Range<usize>) -> Vec<grind_text::grammar::Flag> {
        self.app.grammar(blocks)
    }

    /// The grammar finding the caret is in, if any.
    pub fn grammar_here(&self) -> Option<grind_text::grammar::Flag> {
        let at = self.caret();
        self.grammar_flags(at.block..at.block + 1)
            .into_iter()
            .find(|f| f.finding.range.start <= at.offset && at.offset <= f.finding.range.end)
    }

    /// What a finding covers, as text.
    fn covered(&self, flag: &grind_text::grammar::Flag) -> String {
        let range = flag.finding.range.clone();
        self.app
            .get_viewport(flag.block..flag.block + 1)
            .get(flag.block)
            .map_or_default(|view| {
                view.text
                    .chars()
                    .skip(range.start)
                    .take(range.len())
                    .collect()
            })
    }

    fn grammar_entries(&self, query: &str) -> Vec<Entry> {
        let Some(flag) = self.grammar_here() else {
            return Vec::new();
        };
        crate::grammar::offers(&flag.finding, &self.covered(&flag), query)
    }

    /// Replace the finding at the caret with its `n`th replacement — one undo step.
    pub fn fix_grammar(&self, n: usize) {
        let Some(flag) = self.grammar_here() else {
            return self.set_message("No grammar finding here".to_owned());
        };
        let Some(with) = flag.finding.replacements.get(n).cloned() else {
            return self.set_message(format!("No replacement {} here", n + 1));
        };
        let covered = self.covered(&flag);
        match self.app.correct(flag.caret(), &covered, &with) {
            Ok(()) => {
                self.move_to(Caret {
                    block: flag.block,
                    offset: flag.finding.range.start + with.chars().count(),
                });
                self.set_message(format!(
                    "\u{201c}{covered}\u{201d} \u{2192} \u{201c}{with}\u{201d}"
                ));
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// Put the caret on the next grammar finding, wrapping round the end, and say what is wrong.
    pub fn next_grammar(&self) {
        let all = self.grammar_flags(0..self.app.block_count());
        let at = self.caret();
        let after = |f: &grind_text::grammar::Flag| {
            (f.block, f.finding.range.start) > (at.block, at.offset)
        };
        let Some(flag) = all.iter().find(|f| after(f)).or_else(|| all.first()) else {
            return self.set_message("No grammar findings".to_owned());
        };
        self.move_to(flag.caret());
        self.set_message(flag.finding.message.clone());
    }

    // --- whole paragraphs ---

    /// *Move paragraph up / down* — the caret and selection going along.
    fn move_blocks(&self, up: bool) {
        self.edit(|editor, app| editor.move_paragraphs(app, up));
    }

    /// *Delete paragraph* — `grind_text::blocks::remove`, the caret at the start of what followed.
    fn delete_blocks(&self) {
        self.edit(Editor::delete_paragraphs);
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
        match grind_text::picture::insert_below(&self.app, self.caret().block, mime, data) {
            Ok(block) => {
                self.move_to(Caret { block, offset: 1 });
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
        let caret = self.caret();
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
            if image.description.is_some() {
                more.set_attribute("open", "")
            } else {
                more.remove_attribute("open")
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
                Ok(()) => self.set_message(if title.value().trim().is_empty() {
                    "The picture has no alt text".to_owned()
                } else {
                    "Alt text saved".to_owned()
                }),
                Err(error) => self.set_message(error.to_string()),
            }
        }
        let _ = self.focus();
    }

    // --- markdown ---

    /// Markdown read in before the caret's block (`App::import_markdown`), one undo step.
    pub fn import_markdown(&self, markdown: &str) {
        let at = self.caret().block;
        // A page has no directory to read a picture from: only a `data:` URI comes in.
        match self
            .app
            .import_markdown(at, markdown, &grind_text::commonmark::nowhere)
        {
            Ok(n) => {
                self.move_to(Caret {
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
    /// (`grind_text::table::insert_below`). One prompt takes `3x4`, `3 4` or `3×4`.
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
        match grind_text::table::insert_below(&self.app, self.caret().block, rows, columns) {
            Ok(at) => {
                self.move_to(Caret {
                    block: at,
                    offset: 0,
                });
                self.set_message(format!("A {rows}×{columns} table — Ctrl+Z takes it back"));
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    // --- bookmarks and named styles ---

    /// *Bookmark this paragraph…* — a name, then `App::set_bookmark` at the caret's block; a name
    /// in use moves. The overlay goes on, since a bookmark contributes no characters and would
    /// leave nothing to show it was made.
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
        match self.app.set_bookmark(name, Some(self.caret().block)) {
            Ok(moved) => {
                self.names.set(true);
                self.set_message(if moved {
                    format!("#{name} moved here")
                } else {
                    format!("#{name} anchored here")
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
            .unwrap_or_else(|| (self.caret(), self.caret()));
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
            Ok(_) => self.set_message(if name.is_empty() {
                "Style name removed".to_owned()
            } else {
                format!("Style “{name}” set")
            }),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    // --- find and replace (`grind_text::find`) ---

    /// What the palette offers *after* the verbs: the paragraphs holding the query — after, so
    /// `bold` still puts *Bold* first. Two characters at least; past what fits, the last row is
    /// all of them, stepped through with F3. Ctrl+F opens this box, since the browser's own find
    /// sees only what is on screen.
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
        needle.clone_into(&mut self.needle.borrow_mut());
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
        self.select(from, find::end_of(from, needle));
        let _ = self.focus();
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
            .map_or_else(|| self.caret(), |(from, _)| from);
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
    /// Page setup… — the paper, how it lies and the margins, said in words as the terminal's
    /// `:page-setup` takes them (`PageGeometry::set_up`): the page as it is is offered to edit.
    fn page_setup(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let page = self.app.page().unwrap_or_default();
        let offered = format!(
            "{} {} {}mm",
            page.iso_name().unwrap_or_default().to_lowercase(),
            if page.width > page.height {
                "landscape"
            } else {
                "portrait"
            },
            page.top
        );
        let Ok(Some(words)) = window.prompt_with_message_and_default(
            "Page setup — a paper, landscape or portrait, and the margins, e.g. a5 landscape 15mm",
            offered.trim(),
        ) else {
            return;
        };
        match page.set_up(&words) {
            Ok(page) => match self.app.set_page(page) {
                Ok(()) => self.set_message(page.describe()),
                Err(error) => self.set_message(error.to_string()),
            },
            Err(why) => self.set_message(why),
        }
    }

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
        (*self.needle.borrow_mut()).clone_from(&what);
        match self.app.replace(&what, &with) {
            Ok(0) => self.set_message(format!("“{what}” is not in the document")),
            Ok(n) => {
                self.move_to(self.caret());
                self.set_message(format!(
                    "Replaced in {n} paragraph{} — Ctrl+Z takes it back",
                    if n == 1 { "" } else { "s" }
                ));
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    // --- the code view ---

    /// The document as its projection, for the code view.
    pub fn project(&self) -> grind_text::projection::Projection {
        self.app.project()
    }

    /// Which block the caret is in, spelled the way the span map spells it. `p12` — the address
    /// every block has, whatever else it also answers to.
    pub fn projection_address(&self) -> Option<String> {
        Some(grind_text::loc::format(self.caret().block))
    }

    /// What the document says about itself.
    pub fn lint(&self) -> grind_core::lint::Report {
        self.app.lint(&grind_core::lint::Options::default())
    }

    /// Put the caret in whatever block a code-view line projects: `p12`, `#intro` or `§2.1.3`,
    /// all of which `loc::parse` takes.
    pub fn select_projected(&self, address: &str) {
        let Ok(caret) = grind_text::loc::parse(address)
            .map_err(|e| e.to_string())
            .and_then(|loc| self.app.resolve_caret(&loc).map_err(|e| e.to_string()))
        else {
            return;
        };
        self.move_to(caret);
    }

    fn go_to(&self, where_: &str) {
        let Ok(index) = where_.parse::<usize>() else {
            return;
        };
        if index >= self.app.block_count() {
            return;
        }
        self.move_to(Caret {
            block: index,
            offset: 0,
        });
        let _ = self.focus();
    }

    // --- formatting ---

    /// Turn one character property on across the selection, or off when it is already on
    /// everywhere in it — [`App::char_style`] is what "already on everywhere" means, since it
    /// reports only what the whole span *agrees* about.
    fn toggle_char(&self, emphasis: Emphasis) {
        self.change_char(Change::toggle(emphasis, &self.style_here()));
    }

    /// One tool-row control over the selection — or, with nothing selected, held for the next
    /// character typed (`grind_text::format::apply`), as the `resume` [`App::type_markdown`]
    /// carries; moving the caret forgets it.
    fn change_char(&self, change: Change) {
        let pending = self.editor.borrow().resume.clone();
        let landed = format::apply(
            &self.app,
            self.selection(),
            self.caret(),
            pending.as_ref(),
            &change,
        );
        match landed {
            Ok(Landed::Written) => {}
            Ok(Landed::Pending(style)) => {
                self.editor.borrow_mut().resume = Some(style);
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
        let pending = self.editor.borrow().resume.clone();
        format::here(&self.app, self.selection(), self.caret(), pending.as_ref())
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
            None => (self.caret().block, self.caret().block),
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
        let index = self.caret().block;
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

        let block = self.block_at(self.caret().block);
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
            let start = if index == from.block { from.offset } else { 0 };
            let end = if index == to.block {
                to.offset
            } else {
                chars.len()
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
        self.edit(|editor, app| editor.paste(app, text));
    }

    fn block_at(&self, index: usize) -> Option<BlockView> {
        self.app.get_viewport(index..index + 1).get(index).cloned()
    }

    /// Scroll the least it takes to keep the caret on screen. Arithmetic rather than
    /// `scrollIntoView`, whose jumps are too big for a caret a line out of view and which a
    /// headless run would make throw. The caret's place is its client rectangle less the pane's
    /// plus the pane's scroll — not `offsetTop`, which counts from the nearest positioned
    /// ancestor, a `.block`.
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
        // A key this shell claimed must not also do its default — Tab moves focus, Ctrl+S opens
        // the browser's save dialog, Backspace can go back a page.
        event.prevent_default();
        if let Err(error) = self.apply(action) {
            web_sys::console::error_1(&error);
        }
    }

    /// What a composition finally produced, typed as one keystroke — **the only way a dead key's
    /// character gets in** (see `keymap`). Through [`Ui::type_text`], so a composed backtick is read
    /// by the notation like any other.
    fn on_composed(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.type_text(text);
    }

    fn apply(&self, action: Action<'_>) -> Result<(), JsValue> {
        match action {
            Action::Edit(Intent::Move { motion, extend }) => self.go(motion, extend),
            Action::Type(text) => self.type_text(text),
            Action::Edit(Intent::Split) => {
                self.edit(Editor::split);
            }
            Action::Edit(Intent::PageBreak) => {
                self.edit(Editor::break_page);
            }
            Action::Edit(Intent::Erase { unit, forward }) => {
                let paged = self.paged.borrow();
                let pages = paged::Pages(paged.as_ref());
                self.edit(|editor, app| editor.erase(app, &pages, unit, forward));
            }
            // A tab nests a list item, starts one at a block's front, and types a character
            // anywhere else.
            Action::Edit(Intent::Tab { back }) => {
                self.edit(|editor, app| editor.tab(app, back));
            }
            // Everything else is a command, and takes the same path a palette row does — the
            // chrome answers its own and hands the rest back to `Ui::run`.
            Action::Run(id) => crate::run_command(id),
        }
        Ok(())
    }

    /// Every motion, routed to the core's editor over the printed pages. `extend` keeps the
    /// anchor where it is, which is what makes Shift+arrow a selection rather than a move. A page
    /// is however many of the caret's lines fit, less one, so the line you were reading is still
    /// there afterwards.
    fn go(&self, motion: Motion, extend: bool) {
        let paged = self.paged.borrow();
        let page = paged
            .as_ref()
            .and_then(|paged| paged.spot(self.caret()))
            .map_or(1, |[_, _, height, _]| {
                let view = f64::from(self.dom.pane.client_height());
                ((view / height.max(1.0)) as isize - 1).max(1)
            });
        let pages = paged::Pages(paged.as_ref());
        self.edit(|editor, app| {
            editor.navigate(app, &pages, motion, extend, page);
            Ok(())
        });
    }

    /// The caret, where an edit lands.
    fn caret(&self) -> Caret {
        self.editor.borrow().caret
    }

    /// The caret put at `at` with nothing selected, and any style pending for the next character
    /// forgotten — a Bold pressed with nothing selected belongs to where it was pressed.
    fn move_to(&self, at: Caret) {
        self.edit(|editor, _| {
            editor.place(at, false);
            Ok(())
        });
    }

    /// `from..to` selected, the caret at `to`.
    fn select(&self, from: Caret, to: Caret) {
        self.edit(|editor, _| {
            editor.select(from, to);
            Ok(())
        });
    }

    /// One change to the editor, then a repaint — the core says nothing when only the caret
    /// moved; a refusal goes on the message line.
    fn edit<T>(&self, change: impl FnOnce(&mut Editor, &App) -> Result<T, String>) -> Option<T> {
        let result = change(&mut self.editor.borrow_mut(), &self.app);
        match result {
            Ok(value) => {
                self.request_repaint();
                Some(value)
            }
            Err(error) => {
                self.set_message(error);
                None
            }
        }
    }

    // --- editing ---

    /// A typed character, read as markdown-shaped notation as it lands (`grind_text::markdown`):
    /// `**bold**` becomes bold and its markers go, `` `code` `` monospace, `# ` makes a heading,
    /// three backticks fence a code paragraph. One `App::type_markdown` action, so one Ctrl+Z takes
    /// back `**bold**`. Pasting does not go through it ([`Ui::paste_text`]): clipboard text is text.
    fn type_text(&self, text: &str) {
        // A document with no blocks at all has nowhere to put a character, and the first
        // thing anybody does with an empty page is type into it.
        if self.app.block_count() == 0
            && let Err(error) = self.app.insert(0, BlockKind::Paragraph, "")
        {
            return self.set_message(error.to_string());
        }
        self.edit(|editor, app| editor.type_text(app, text));
    }

    /// Where a click landed: the line carries its address, and the core's layout answers the
    /// offset. `extend` is Shift held, or the pointer still down from a press.
    fn on_click(&self, event: &MouseEvent, extend: bool) -> Result<(), JsValue> {
        self.focus()?;
        let Some(caret) = self.caret_at(event)? else {
            return Ok(());
        };
        self.edit(|editor, _| {
            editor.place(caret, extend);
            Ok(())
        });
        Ok(())
    }

    /// The caret a pointer position names, or `None` when it landed on nothing — the gap
    /// between two blocks, or the chrome.
    fn caret_at(&self, event: &MouseEvent) -> Result<Option<Caret>, JsValue> {
        if let Some(paged) = self.paged.borrow().as_ref() {
            let flow = self.dom.flow.get_bounding_client_rect();
            let x = f64::from(event.client_x()) - flow.left();
            let y = f64::from(event.client_y()) - flow.top();
            return Ok(paged.hit(x, y));
        }
        Ok(None)
    }
}

/// Which CSS class a block is given in the reader layer: its structure.
fn class_of(block: &BlockView) -> String {
    let mut class = match &block.kind {
        BlockKind::Paragraph => "block p".to_owned(),
        BlockKind::Heading { level } => format!("block h h{}", level.clamp(&1, &6)),
        BlockKind::ListItem { .. } => "block li".to_owned(),
    };
    // A named style this shell gives a face to is a class too, so the stylesheet can space it
    // — the two that are not headings and would otherwise be indistinguishable paragraphs.
    if let Some(style @ ("Title" | "Subtitle")) = block.style.as_deref() {
        let _ = write!(class, " {}", style.to_lowercase());
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

/// Bytes as base64, for a picture's `data:` URL — fifteen lines rather than a dependency every
/// reader downloads; `window.btoa` would need the bytes as code points first.
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
            out.push(if pad { '=' } else { ALPHABET[sextet] as char });
        }
    }
    out
}

/// Where a picture's alt chip puts the caret: `data-alt` is `block:offset`, the picture's own.
fn chip_caret(at: &str) -> Option<Caret> {
    let (block, offset) = at.split_once(':')?;
    Some(Caret {
        block: block.parse().ok()?,
        offset: offset.parse().ok()?,
    })
}

/// A touch screen's keyboard, through [`Dom::typing`]. Its keys that have a name arrive as
/// `keydown` and reach the pane's keymap by bubbling; what it types is mostly only `beforeinput`,
/// which is answered here in the same keymap's words. A word an input method is composing stays
/// in the field, on the caret, until `compositionend` types it (the pane's listener) and it is
/// emptied.
fn wire_keyboard(ui: &Rc<Ui>) -> Result<(), JsValue> {
    let typed = ui.clone();
    listen(&ui.dom.typing, "beforeinput", move |event: InputEvent| {
        if event.is_composing() {
            return;
        }
        let kind = event.input_type();
        let (key, primary, shift) = match kind.as_str() {
            "insertText" | "insertReplacementText" => {
                event.prevent_default();
                if let Some(text) = event.data().filter(|text| !text.is_empty()) {
                    typed.type_text(&text);
                }
                return;
            }
            "insertParagraph" => ("Enter", false, false),
            "insertLineBreak" => ("Enter", false, true),
            "deleteContentBackward" => ("Backspace", false, false),
            "deleteContentForward" => ("Delete", false, false),
            "deleteWordBackward" => ("Backspace", true, false),
            "deleteWordForward" => ("Delete", true, false),
            _ => return,
        };
        event.prevent_default();
        let chord = Chord {
            key,
            primary,
            shift,
            composing: false,
        };
        if let Some(action) = keymap::action_for(&chord)
            && let Err(error) = typed.apply(action)
        {
            web_sys::console::error_1(&error);
        }
    })?;
    let keys = ui.dom.typing.clone();
    listen(
        &ui.dom.typing,
        "compositionend",
        move |_: CompositionEvent| {
            keys.set_value("");
        },
    )?;
    // Whatever got past `beforeinput` without a composition — a browser that cannot cancel it.
    let leftover = ui.clone();
    listen(&ui.dom.typing, "input", move |event: InputEvent| {
        if event.is_composing() {
            return;
        }
        let text = leftover.dom.typing.value();
        leftover.dom.typing.set_value("");
        if !text.is_empty() {
            leftover.type_text(&text);
        }
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
            keys.move_to(at);
            return keys.open_alt();
        }
        keys.on_key(&event);
    })?;

    // A composition — a dead key, the page's share of an input method — produces its character
    // here, and the keymap refuses every keystroke belonging to one.
    let composed = ui.clone();
    listen(
        &ui.dom.pane,
        "compositionend",
        move |event: CompositionEvent| {
            composed.on_composed(&event.data().unwrap_or_default());
        },
    )?;

    let pressed = ui.clone();
    listen(&ui.dom.pane, "pointerdown", move |event: PointerEvent| {
        pressed.touch.set(event.pointer_type() != "mouse");
    })?;
    wire_keyboard(ui)?;

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
            click.move_to(at);
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

    // Pages come into view as the pane scrolls, and a page that changed is drawn when it does.
    let scrolled = ui.clone();
    listen(&ui.dom.pane, "scroll", move |_: Event| {
        if scrolled.is_paged() {
            scrolled.request_repaint();
        }
    })?;

    // Dragging: every move with the button down extends the selection.
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
        dialog.close_with_return_value("cancel");
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
}
