// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tape dialog: `grind_sheet::Tape` (`doc/calc.md`), a scratch calculator over the page.
//! `index.html`'s `#tape`; every decision is the tape's, and this redraws the list after each call.
//! The page keeps one tape, so closing the dialog keeps its lines for the next time.

use std::cell::RefCell;
use std::rc::Rc;

use grind_sheet::Tape;
use grind_sheet::locale::Locale;
use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Event, HtmlButtonElement, HtmlDialogElement, HtmlElement, HtmlInputElement,
    KeyboardEvent,
};

use crate::{element, listen};

pub struct TapeDialog {
    tape: RefCell<Option<Tape>>,
    document: Document,
    dialog: HtmlDialogElement,
    lines: HtmlElement,
    empty: HtmlElement,
    note: HtmlElement,
    input: HtmlInputElement,
    undo: HtmlButtonElement,
    redo: HtmlButtonElement,
}

impl TapeDialog {
    pub fn find(document: &Document) -> Result<Rc<Self>, JsValue> {
        let this = Rc::new(TapeDialog {
            tape: RefCell::new(None),
            document: document.clone(),
            dialog: element(document, "tape")?,
            lines: element(document, "tape-lines")?,
            empty: element(document, "tape-empty")?,
            note: element(document, "tape-note")?,
            input: element(document, "tape-input")?,
            undo: element(document, "tape-undo")?,
            redo: element(document, "tape-redo")?,
        });
        this.wire()?;
        Ok(this)
    }

    /// Show the tape, made the first time in `locale`: the open spreadsheet's spelling of numbers.
    pub fn open(&self, locale: Option<Locale>) {
        self.tape
            .borrow_mut()
            .get_or_insert_with(|| Tape::new(locale));
        self.refresh();
        if !self.dialog.open() {
            let _ = self.dialog.show_modal();
        }
        let _ = self.input.focus();
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

    /// Run `f` on the tape; redrawn when it took, said when it did not. Answers whether it took.
    fn change(&self, f: impl FnOnce(&mut Tape) -> grind_sheet::Result<()>) -> bool {
        let done = match self.tape.borrow_mut().as_mut() {
            Some(tape) => f(tape),
            None => return false,
        };
        match done {
            Ok(()) => {
                self.refresh();
                true
            }
            Err(error) => {
                self.say(&error.to_string());
                false
            }
        }
    }

    fn step(&self, back: bool) {
        let stepped = self
            .tape
            .borrow_mut()
            .as_mut()
            .is_some_and(|tape| if back { tape.undo() } else { tape.redo() });
        if stepped {
            self.refresh();
        }
    }

    /// Every row again from the tape: a tape is short, and a row per line cannot then disagree
    /// with it.
    fn refresh(&self) {
        let borrowed = self.tape.borrow();
        let Some(tape) = borrowed.as_ref() else {
            return;
        };
        self.lines.set_inner_html("");
        for (at, line) in tape.lines().into_iter().enumerate() {
            if let Ok(row) = self.row(at, &line) {
                let _ = self.lines.append_child(&row);
            }
        }
        self.lines.set_hidden(tape.is_empty());
        self.empty.set_hidden(!tape.is_empty());
        self.say(&tape.circles().map_or_else(String::new, |circles| {
            format!("{circles}: #NUM! until the circle is broken, and no spreadsheet before then")
        }));
        self.undo.set_disabled(!tape.can_undo());
        self.redo.set_disabled(!tape.can_redo());
    }

    /// One line: its handle, its text to edit in place, its value, and a delete button. `data-at`
    /// is what the list's one listener reads back.
    fn row(&self, at: usize, line: &grind_sheet::TapeLine) -> Result<web_sys::Element, JsValue> {
        let doc = &self.document;
        let row = doc.create_element("li")?;
        row.set_class_name(if line.error { "error" } else { "" });
        let handle = doc.create_element("span")?;
        handle.set_class_name("tape-handle");
        handle.set_text_content(Some(&format!("#{}", line.handle)));
        let text: HtmlInputElement = doc.create_element("input")?.dyn_into()?;
        text.set_class_name("tape-text");
        text.set_value(&line.text);
        text.set_attribute("data-at", &at.to_string())?;
        text.set_attribute("aria-label", &format!("Line {}", line.handle))?;
        text.set_attribute("autocomplete", "off")?;
        text.set_attribute("spellcheck", "false")?;
        let value = doc.create_element("output")?;
        value.set_class_name("tape-value");
        value.set_text_content(Some(&line.value));
        let delete = doc.create_element("button")?;
        delete.set_class_name("tape-delete");
        delete.set_attribute("type", "button")?;
        delete.set_attribute("data-at", &at.to_string())?;
        delete.set_attribute("title", "Delete the line")?;
        delete.set_attribute("aria-label", &format!("Delete line {}", line.handle))?;
        delete.set_text_content(Some("\u{d7}"));
        for child in [&handle, &text, &value, &delete] {
            row.append_child(child)?;
        }
        Ok(row)
    }

    fn wire(self: &Rc<Self>) -> Result<(), JsValue> {
        let typed = self.clone();
        listen(&self.input, "keydown", move |event: KeyboardEvent| {
            if event.key() != "Enter" {
                return;
            }
            event.prevent_default();
            let text = typed.input.value();
            if text.trim().is_empty() {
                return;
            }
            // A line that does not read stays in the entry, to be fixed rather than retyped.
            if typed.change(|tape| tape.push(&text).map(|_| ())) {
                typed.input.set_value("");
            }
        })?;

        // An edited line is written when it is left or Enter is pressed: `change` is both.
        let edited = self.clone();
        listen(&self.lines, "change", move |event: Event| {
            let Some(input) = event
                .target()
                .and_then(|t| t.dyn_into::<HtmlInputElement>().ok())
            else {
                return;
            };
            if let Some(at) = at_of(&input) {
                let text = input.value();
                edited.change(|tape| tape.edit(at, &text));
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
            if let Some(at) = at_of(&button) {
                deleted.change(|tape| tape.delete(at));
                let _ = deleted.input.focus();
            }
        })?;

        let back = self.clone();
        listen(&self.undo, "click", move |_: Event| back.step(true))?;
        let forward = self.clone();
        listen(&self.redo, "click", move |_: Event| forward.step(false))?;
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
            keys.step(back);
        })
    }
}

fn at_of(element: &web_sys::Element) -> Option<usize> {
    element.get_attribute("data-at")?.parse().ok()
}
