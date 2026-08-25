// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The command palette, as a widget.
//!
//! It knows nothing about what any command *means* — it shows [`Entry`] values, remembers
//! which row is picked, and hands back the id of the one that was chosen. What goes in the
//! list is [`crate::command`]'s and the pane's; what happens next is the shell's own `run`.
//!
//! Not a `<dialog>`: the top layer is more than a list needs, and a plain element behaves the
//! same in every browser and in jsdom, where the smoke test drives it.

use std::cell::{Cell, RefCell};

use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, HtmlElement, HtmlInputElement};

use crate::command::Entry;
use crate::element;

pub struct Palette {
    document: Document,
    root: HtmlElement,
    pub input: HtmlInputElement,
    list: HtmlElement,
    empty: HtmlElement,
    /// What is on screen, in the order it is drawn — the index [`Palette::active`] points into.
    shown: RefCell<Vec<Entry>>,
    active: Cell<usize>,
}

impl Palette {
    pub fn find(document: &Document) -> Result<Self, JsValue> {
        Ok(Palette {
            document: document.clone(),
            root: element(document, "palette")?,
            input: element(document, "palette-input")?,
            list: element(document, "palette-list")?,
            empty: element(document, "palette-empty")?,
            shown: RefCell::new(Vec::new()),
            active: Cell::new(0),
        })
    }

    pub fn is_open(&self) -> bool {
        !self.root.hidden()
    }

    pub fn query(&self) -> String {
        self.input.value()
    }

    /// Open it with a fresh query. The caller supplies the entries, because only the shell
    /// knows which pane is showing.
    pub fn open(&self, entries: Vec<Entry>) -> Result<(), JsValue> {
        self.input.set_value("");
        self.root.set_hidden(false);
        self.show(entries)?;
        self.input.focus()
    }

    pub fn close(&self) {
        self.root.set_hidden(true);
        self.shown.borrow_mut().clear();
    }

    /// Replace the list — what every keystroke in the input does. The pick goes back to the
    /// first row, because after a new letter the old row is a different command.
    pub fn show(&self, entries: Vec<Entry>) -> Result<(), JsValue> {
        self.list.set_text_content(None);
        self.empty.set_hidden(!entries.is_empty());
        for (index, entry) in entries.iter().enumerate() {
            let row = self.document.create_element("li")?;
            row.set_attribute("role", "option")?;
            row.set_attribute("data-index", &index.to_string())?;
            row.set_attribute("data-id", &entry.id)?;

            let what = self.document.create_element("span")?;
            what.set_class_name("what");
            emphasise(&self.document, &what, &entry.title, &entry.hits)?;
            row.append_child(&what)?;

            let where_ = self.document.create_element("span")?;
            where_.set_class_name("where");
            where_.set_text_content(Some(&entry.group));
            row.append_child(&where_)?;

            if !entry.keys.is_empty() {
                let keys = self.document.create_element("kbd")?;
                keys.set_text_content(Some(&entry.keys));
                row.append_child(&keys)?;
            }
            self.list.append_child(&row)?;
        }
        *self.shown.borrow_mut() = entries;
        self.active.set(0);
        self.mark()
    }

    /// Move the pick, wrapping — a list that stops at the bottom makes the last row hard to
    /// reach and the first one harder.
    pub fn step(&self, delta: isize) -> Result<(), JsValue> {
        let count = self.shown.borrow().len();
        if count == 0 {
            return Ok(());
        }
        let at = self.active.get() as isize + delta;
        self.active
            .set(at.rem_euclid(count as isize).unsigned_abs());
        self.mark()
    }

    pub fn pick(&self, index: usize) -> Result<(), JsValue> {
        if index < self.shown.borrow().len() {
            self.active.set(index);
            self.mark()?;
        }
        Ok(())
    }

    /// The id of the picked row, if there is one.
    pub fn chosen(&self) -> Option<String> {
        self.shown
            .borrow()
            .get(self.active.get())
            .map(|entry| entry.id.clone())
    }

    /// Paint the pick, and keep it in view — a palette that scrolls its own selection out of
    /// sight is one where Down-arrow appears to do nothing.
    fn mark(&self) -> Result<(), JsValue> {
        let active = self.active.get();
        let rows = self.list.children();
        for index in 0..rows.length() {
            let Some(row) = rows.item(index) else {
                continue;
            };
            let picked = index as usize == active;
            row.set_attribute("aria-selected", if picked { "true" } else { "false" })?;
            if picked && let Some(row) = row.dyn_ref::<HtmlElement>() {
                scroll_into_view(&self.list, row);
            }
        }
        Ok(())
    }
}

/// The least scrolling that puts `row` inside `list` — the same "move as little as possible"
/// rule the grid and the document pane both follow, and for the same reason: `scrollIntoView`
/// jumps, and in a headless run every offset is zero and this does nothing at all.
fn scroll_into_view(list: &HtmlElement, row: &HtmlElement) {
    let top = f64::from(row.offset_top());
    let height = f64::from(row.offset_height());
    let view = f64::from(list.client_height());
    let scroll = f64::from(list.scroll_top());
    let wanted = match () {
        _ if top < scroll => top,
        _ if top + height > scroll + view => top + height - view,
        _ => return,
    };
    list.set_scroll_top(wanted as i32);
}

/// The title with the matched characters wrapped in `<b>` — the feedback that makes a fuzzy
/// match legible rather than mysterious.
fn emphasise(
    document: &Document,
    into: &Element,
    title: &str,
    hits: &[usize],
) -> Result<(), JsValue> {
    if hits.is_empty() {
        into.set_text_content(Some(title));
        return Ok(());
    }
    let mut plain = String::new();
    for (at, c) in title.chars().enumerate() {
        if hits.contains(&at) {
            if !plain.is_empty() {
                into.append_child(&document.create_text_node(&plain))?;
                plain.clear();
            }
            let hit = document.create_element("b")?;
            hit.set_text_content(Some(&c.to_string()));
            into.append_child(&hit)?;
        } else {
            plain.push(c);
        }
    }
    if !plain.is_empty() {
        into.append_child(&document.create_text_node(&plain))?;
    }
    Ok(())
}
