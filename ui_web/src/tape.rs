// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tape dialog: `grind_sheet::Tape` (`doc/calc.md`) as a notebook over the page, one row per
//! line edited in place and a blank row after the last. Where the caret goes is
//! `grind_sheet::Notebook`'s, what to offer and how to colour a line the tape's; this maps keys
//! and keeps the rows in step. The page keeps one tape, so closing the dialog keeps its lines.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use grind_core::utf16;
use grind_sheet::formula::assist::{self, Assist};
use grind_sheet::locale::Locale;
use grind_sheet::{Notebook, Row, Step, Tape, TapeInk};
use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Element, Event, HtmlButtonElement, HtmlDialogElement, HtmlElement, HtmlInputElement,
    KeyboardEvent,
};

use crate::{element, listen};

pub struct TapeDialog {
    tape: RefCell<Option<Tape>>,
    book: RefCell<Notebook>,
    assist: RefCell<Assist>,
    /// Set while the rows are rebuilt, so the `change` an input fires as it goes is not a commit.
    drawing: Cell<bool>,
    document: Document,
    dialog: HtmlDialogElement,
    lines: HtmlElement,
    empty: HtmlElement,
    band: HtmlElement,
    note: HtmlElement,
    undo: HtmlButtonElement,
    redo: HtmlButtonElement,
}

impl TapeDialog {
    pub fn find(document: &Document) -> Result<Rc<Self>, JsValue> {
        let this = Rc::new(TapeDialog {
            tape: RefCell::new(None),
            book: RefCell::new(Notebook::default()),
            assist: RefCell::new(Assist::default()),
            drawing: Cell::new(false),
            document: document.clone(),
            dialog: element(document, "tape")?,
            lines: element(document, "tape-lines")?,
            empty: element(document, "tape-empty")?,
            band: element(document, "tape-assist")?,
            note: element(document, "tape-note")?,
            undo: element(document, "tape-undo")?,
            redo: element(document, "tape-redo")?,
        });
        this.wire()?;
        Ok(this)
    }

    /// Show the tape, made the first time in `locale`: the open spreadsheet's spelling of numbers.
    /// The caret starts in the blank row at the end.
    pub fn open(&self, locale: Option<Locale>) {
        let len = {
            let mut tape = self.tape.borrow_mut();
            tape.get_or_insert_with(|| Tape::new(locale)).len()
        };
        *self.book.borrow_mut() = Notebook::default();
        self.say("");
        self.refresh(true);
        if !self.dialog.open() {
            let _ = self.dialog.show_modal();
        }
        self.focus(Row::Blank(len));
    }

    pub fn close(&self) {
        self.dialog.close();
    }

    /// The tape as a spreadsheet, or why not, said in the dialog.
    pub fn promote(&self) -> Option<grind_sheet::Document> {
        let promoted = self.tape.borrow().as_ref()?.promote();
        promoted.map_err(|error| self.say(&error.to_string())).ok()
    }

    fn say(&self, text: &str) {
        self.note.set_text_content(Some(text));
    }

    /// The circle the tape is in, if any, else nothing.
    fn say_circles(&self) {
        let said = self.tape.borrow().as_ref().and_then(Tape::circles);
        self.say(&said.map_or_else(String::new, |circles| {
            format!("{circles}: #NUM! until the circle is broken, and no spreadsheet before then")
        }));
    }

    /// Run a notebook step from `input`'s row; the rows again and the caret where it lands, or
    /// the reason it was refused, with the caret left where it was.
    fn step(&self, input: &HtmlInputElement, step: Step) {
        let Some(row) = row_of(input) else { return };
        let text = input.value();
        let moved = {
            let mut tape = self.tape.borrow_mut();
            let Some(tape) = tape.as_mut() else { return };
            self.book.borrow_mut().step(tape, row, &text, step)
        };
        match moved {
            Ok(to) => {
                self.say_circles();
                self.refresh(true);
                self.focus(to);
            }
            Err(error) => self.say(&error.to_string()),
        }
    }

    /// The row an input lost the caret from, written: the `change` a click elsewhere fires.
    fn commit(&self, input: &HtmlInputElement) {
        let Some(row) = row_of(input) else { return };
        let text = input.value();
        let done = {
            let mut tape = self.tape.borrow_mut();
            let Some(tape) = tape.as_mut() else { return };
            self.book.borrow_mut().commit(tape, row, &text)
        };
        match done {
            Ok(grind_sheet::Done::Nothing) => {}
            Ok(_) => {
                self.say_circles();
                self.refresh(false);
            }
            Err(error) => self.say(&error.to_string()),
        }
    }

    fn undo_or_redo(&self, back: bool) {
        let stepped = self
            .tape
            .borrow_mut()
            .as_mut()
            .is_some_and(|tape| if back { tape.undo() } else { tape.redo() });
        if stepped {
            *self.book.borrow_mut() = Notebook::default();
            self.say_circles();
            self.refresh(true);
        }
    }

    /// The rows from the notebook, reusing the ones already drawn. Unless `rewrite`, the input
    /// with the caret keeps what it holds: somebody clicked into it to type.
    fn refresh(&self, rewrite: bool) {
        let borrowed = self.tape.borrow();
        let Some(tape) = borrowed.as_ref() else {
            return;
        };
        self.drawing.set(true);
        let rows = self.book.borrow().rows(tape);
        let lines = tape.lines();
        let drawn = self.lines.children();
        let focused = self.document.active_element();
        for (at, row) in rows.iter().enumerate() {
            let li = match drawn.item(at as u32) {
                Some(li) => li,
                None => match self.row() {
                    Ok(li) => {
                        let _ = self.lines.append_child(&li);
                        li
                    }
                    Err(_) => break,
                },
            };
            let Some(input) = part::<HtmlInputElement>(&li, ".tape-text") else {
                continue;
            };
            let keep = !rewrite && focused.as_ref() == Some(input.as_ref());
            let (value, error) = match row {
                Row::Line(n) => lines
                    .get(*n)
                    .map_or((String::new(), false), |l| (l.value.clone(), l.error)),
                Row::Blank(_) => (String::new(), false),
            };
            let label = Notebook::label(*row, tape);
            li.set_class_name(match (row, error) {
                (Row::Blank(_), _) => "blank",
                (_, true) => "error",
                _ => "",
            });
            if let Some(handle) = part::<HtmlElement>(&li, ".tape-handle") {
                handle.set_text_content(Some(&label));
            }
            if let Some(out) = part::<HtmlElement>(&li, ".tape-value") {
                out.set_text_content(Some(&value));
            }
            if let Some(delete) = part::<HtmlElement>(&li, ".tape-delete") {
                delete.set_hidden(matches!(row, Row::Blank(_)));
                let _ = delete.set_attribute("aria-label", &format!("Delete line {label}"));
            }
            let (kind, n) = match row {
                Row::Line(n) => ("line", n),
                Row::Blank(n) => ("blank", n),
            };
            let _ = input.set_attribute("data-row", kind);
            let _ = input.set_attribute("data-at", &n.to_string());
            let _ = input.set_attribute(
                "aria-label",
                &match row {
                    Row::Line(_) => format!("Line {label}"),
                    Row::Blank(_) => "New line".to_owned(),
                },
            );
            input.set_placeholder(match row {
                Row::Blank(n) if *n == 0 && tape.is_empty() => "1200 * 1.19",
                _ => "",
            });
            if !keep {
                input.set_value(&Notebook::text(*row, tape));
            }
            paint(&self.document, tape, &li, &input, None);
        }
        while drawn.length() as usize > rows.len() {
            match drawn.item(rows.len() as u32) {
                Some(extra) => extra.remove(),
                None => break,
            }
        }
        self.empty.set_hidden(!tape.is_empty());
        self.undo.set_disabled(!tape.can_undo());
        self.redo.set_disabled(!tape.can_redo());
        drop(borrowed);
        self.drawing.set(false);
        self.assist.borrow_mut().clear();
        self.draw_band();
    }

    /// One row's elements, empty: its handle, its text over the colours of its words, its value
    /// and a delete button.
    fn row(&self) -> Result<Element, JsValue> {
        let doc = &self.document;
        let li = doc.create_element("li")?;
        let handle = doc.create_element("span")?;
        handle.set_class_name("tape-handle");
        let edit = doc.create_element("div")?;
        edit.set_class_name("tape-edit");
        let ink = doc.create_element("div")?;
        ink.set_class_name("tape-ink");
        ink.set_attribute("aria-hidden", "true")?;
        let text: HtmlInputElement = doc.create_element("input")?.dyn_into()?;
        text.set_class_name("tape-text");
        text.set_attribute("autocomplete", "off")?;
        text.set_attribute("spellcheck", "false")?;
        edit.append_child(&ink)?;
        edit.append_child(&text)?;
        let value = doc.create_element("output")?;
        value.set_class_name("tape-value");
        let delete = doc.create_element("button")?;
        delete.set_class_name("tape-delete");
        delete.set_attribute("type", "button")?;
        delete.set_attribute("title", "Delete the line")?;
        delete.set_text_content(Some("\u{d7}"));
        for child in [&handle, &edit, &value, &delete] {
            li.append_child(child)?;
        }
        Ok(li)
    }

    /// Put the caret at the end of `row`'s input.
    fn focus(&self, row: Row) {
        let rows = match self.tape.borrow().as_ref() {
            Some(tape) => self.book.borrow().rows(tape),
            None => return,
        };
        let Some(at) = rows.iter().position(|r| *r == row) else {
            return;
        };
        if let Some(input) = self
            .lines
            .children()
            .item(at as u32)
            .and_then(|li| part::<HtmlInputElement>(&li, ".tape-text"))
        {
            let _ = input.focus();
            let end = utf16::units_before(&input.value(), input.value().len()) as u32;
            let _ = input.set_selection_range(end, end);
        }
    }

    /// The offers for the word at `input`'s caret, and the colours of what it holds.
    fn typed(&self, input: &HtmlInputElement) {
        {
            let tape = self.tape.borrow();
            let Some(tape) = tape.as_ref() else { return };
            let text = input.value();
            let units = input.selection_start().ok().flatten().unwrap_or(0) as usize;
            let caret = utf16::byte_of(&text, units);
            tape.assist(&mut self.assist.borrow_mut(), &text, caret);
            if let Some(li) = input.closest("li").ok().flatten() {
                paint(&self.document, tape, &li, input, Some(caret));
            }
        }
        self.draw_band();
    }

    fn draw_band(&self) {
        let pieces = assist::band(&self.assist.borrow(), true);
        self.band.set_text_content(None);
        // The band keeps its line, so that the rows do not jump as offers come and go.
        self.band.set_class_name(if pieces.is_empty() {
            "assist tape-assist idle"
        } else {
            "assist tape-assist"
        });
        if pieces.is_empty() {
            self.band.set_text_content(Some(
                "Enter: next line   \u{b7}   \u{2191}\u{2193}: move   \u{b7}   Tab: complete",
            ));
        }
        for piece in pieces {
            let Ok(span) = self.document.create_element("span") else {
                return;
            };
            span.set_class_name(match piece.ink {
                assist::Ink::Plain => "plain",
                assist::Ink::Muted => "muted",
                assist::Ink::Strong => "strong",
            });
            span.set_text_content(Some(&piece.text));
            let _ = self.band.append_child(&span);
        }
    }

    /// Tab: the chosen offer into the input, the caret after it.
    fn accept(&self, input: &HtmlInputElement) {
        let Some((span, replacement)) = self.assist.borrow_mut().accept() else {
            return;
        };
        let text = input.value();
        let (Some(head), Some(tail)) = (text.get(..span.start), text.get(span.end..)) else {
            return;
        };
        let next = format!("{head}{replacement}{tail}");
        let caret = utf16::units_before(&next, span.start + replacement.len()) as u32;
        input.set_value(&next);
        let _ = input.set_selection_range(caret, caret);
        self.typed(input);
    }

    fn key(&self, input: &HtmlInputElement, event: &KeyboardEvent) {
        if event.ctrl_key() || event.meta_key() || event.alt_key() {
            return;
        }
        let offering = self.assist.borrow().is_offering();
        let key = event.key();
        match key.as_str() {
            "Tab" if offering && !event.shift_key() => self.accept(input),
            "ArrowDown" if offering => self.assist.borrow_mut().step(1),
            "ArrowUp" if offering => self.assist.borrow_mut().step(-1),
            "Escape" if offering => self.assist.borrow_mut().dismiss(),
            "Enter" => return self.claimed(event, || self.step(input, Step::Enter)),
            "ArrowDown" => return self.claimed(event, || self.step(input, Step::Down)),
            "ArrowUp" => return self.claimed(event, || self.step(input, Step::Up)),
            "Backspace" if input.value().is_empty() => {
                return self.claimed(event, || self.step(input, Step::Erase));
            }
            _ => return,
        }
        event.prevent_default();
        event.stop_propagation();
        self.draw_band();
    }

    fn claimed(&self, event: &KeyboardEvent, then: impl FnOnce()) {
        event.prevent_default();
        event.stop_propagation();
        then();
    }

    fn wire(self: &Rc<Self>) -> Result<(), JsValue> {
        let keys = self.clone();
        listen(&self.lines, "keydown", move |event: KeyboardEvent| {
            if let Some(input) = input_of(&event) {
                keys.key(&input, &event);
            }
        })?;
        for name in ["input", "keyup", "click", "focusin"] {
            let typed = self.clone();
            listen(&self.lines, name, move |event: Event| {
                let navigating = event
                    .dyn_ref::<KeyboardEvent>()
                    .is_some_and(|k| matches!(k.key().as_str(), "Enter" | "Tab" | "Escape"));
                if let Some(input) = input_of(&event)
                    && !navigating
                {
                    typed.typed(&input);
                }
            })?;
        }
        // A row the caret leaves is coloured whole, the word it was in included.
        let left = self.clone();
        listen(&self.lines, "focusout", move |event: Event| {
            let tape = left.tape.borrow();
            if let (Some(tape), Some(input)) = (tape.as_ref(), input_of(&event))
                && let Some(li) = input.closest("li").ok().flatten()
            {
                paint(&left.document, tape, &li, &input, None);
            }
        })?;
        // A click elsewhere writes the row the caret left: `change` is that. A row a key already
        // wrote reads as its line, and is not written again.
        let edited = self.clone();
        listen(&self.lines, "change", move |event: Event| {
            if edited.drawing.get() {
                return;
            }
            if let Some(input) = input_of(&event) {
                edited.commit(&input);
            }
        })?;
        let deleted = self.clone();
        listen(&self.lines, "click", move |event: Event| {
            let Some(button) = event
                .target()
                .and_then(|t| t.dyn_into::<HtmlButtonElement>().ok())
            else {
                return;
            };
            let Some(input) = button
                .closest("li")
                .ok()
                .flatten()
                .and_then(|li| part::<HtmlInputElement>(&li, ".tape-text"))
            else {
                return;
            };
            input.set_value("");
            deleted.step(&input, Step::Erase);
        })?;

        let back = self.clone();
        listen(&self.undo, "click", move |_: Event| back.undo_or_redo(true))?;
        let forward = self.clone();
        listen(&self.redo, "click", move |_: Event| {
            forward.undo_or_redo(false);
        })?;
        let closing: HtmlButtonElement = element(&self.document, "tape-close")?;
        let closed = self.clone();
        listen(&closing, "click", move |_: Event| closed.close())?;

        // Ctrl+Z inside the dialog is the tape's, never the browser's undo of one input.
        let keys = self.clone();
        listen(&self.dialog, "keydown", move |event: KeyboardEvent| {
            if !(event.ctrl_key() || event.meta_key()) {
                return;
            }
            let back = match event.key().as_str() {
                "z" => true,
                "Z" | "y" | "Y" => false,
                _ => return,
            };
            event.prevent_default();
            event.stop_propagation();
            keys.undo_or_redo(back);
        })
    }
}

/// The colours of the words `input` holds, drawn into the layer under it and scrolled with it.
fn paint(
    document: &Document,
    tape: &Tape,
    li: &Element,
    input: &HtmlInputElement,
    caret: Option<usize>,
) {
    let Some(ink) = part::<HtmlElement>(li, ".tape-ink") else {
        return;
    };
    let text = input.value();
    ink.set_text_content(None);
    let mut at = 0;
    for span in tape.spans(&text, caret) {
        let (Some(gap), Some(word)) =
            (text.get(at..span.range.start), text.get(span.range.clone()))
        else {
            continue;
        };
        let _ = ink.append_with_str_1(gap);
        if let Ok(run) = document.create_element("span") {
            run.set_class_name(match span.ink {
                TapeInk::Definition => "ink-def",
                TapeInk::Name => "ink-name",
                TapeInk::Handle => "ink-handle",
                TapeInk::Function => "ink-func",
                TapeInk::Number => "ink-num",
                TapeInk::Text => "ink-text",
                TapeInk::Error => "ink-error",
                TapeInk::Unknown => "ink-unknown",
            });
            run.set_text_content(Some(word));
            let _ = ink.append_child(&run);
        }
        at = span.range.end;
    }
    let _ = ink.append_with_str_1(text.get(at..).unwrap_or_default());
    ink.set_scroll_left(input.scroll_left());
}

fn part<T: JsCast>(li: &Element, selector: &str) -> Option<T> {
    li.query_selector(selector).ok().flatten()?.dyn_into().ok()
}

fn input_of(event: &Event) -> Option<HtmlInputElement> {
    event
        .target()?
        .dyn_into::<HtmlInputElement>()
        .ok()
        .filter(|input| input.class_name() == "tape-text")
}

fn row_of(input: &HtmlInputElement) -> Option<Row> {
    let at = input.get_attribute("data-at")?.parse().ok()?;
    match input.get_attribute("data-row")?.as_str() {
        "line" => Some(Row::Line(at)),
        "blank" => Some(Row::Blank(at)),
        _ => None,
    }
}
