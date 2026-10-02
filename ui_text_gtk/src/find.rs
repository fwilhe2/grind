// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The find bar — Ctrl+F, the `grind text find -i` of this window.
//!
//! A `gtk::SearchBar` under the format bar, the way every GNOME editor finds: it slides in, it
//! searches as it is typed, Enter and Shift+Enter (or Ctrl+G and Shift+Ctrl+G, which the entry
//! already answers) walk the hits, and Escape puts the keyboard back in the document with the
//! hit it stopped on still selected — so typing over a found word replaces it, which is what
//! most people opened the bar to do.
//!
//! **It ignores case**, which is `App::find_ignoring_case` and not a second matcher here: a
//! person typing `appendix` is looking for *Appendix*, and a bar that says "No results" for it
//! is a bar that has taught them it does not work.
//!
//! The hits are asked for again at every step rather than kept, because the document may have
//! been edited between two presses of Enter and a stale list selects the wrong characters.
//! [`step`] is the part that decides which hit is next, and it is pure.

use std::rc::Rc;
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_text::App;
use grind_text::find::{Towards, end_of, hits, step};
use gtk::glib;

use crate::view::Doc;

/// The bar and the two widgets in it anything outside needs.
pub struct Find {
    pub bar: gtk::SearchBar,
    entry: gtk::SearchEntry,
    count: gtk::Label,
    with: gtk::Entry,
    app: Arc<App>,
    doc: Doc,
}

impl Find {
    pub fn new(app: &Arc<App>, doc: &Doc) -> Rc<Self> {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Find in the document")
            .hexpand(true)
            .build();
        let count = gtk::Label::new(None);
        count.add_css_class("dim-label");
        count.add_css_class("numeric");
        let previous = gtk::Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text("Previous (Shift+Enter)")
            .build();
        let next = gtk::Button::builder()
            .icon_name("go-down-symbolic")
            .tooltip_text("Next (Enter)")
            .build();
        let arrows = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        arrows.add_css_class("linked");
        arrows.append(&previous);
        arrows.append(&next);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_width_request(420);
        row.append(&entry);
        row.append(&count);
        row.append(&arrows);
        // The second row: what to put there, and the one button that writes. `App::replace` is
        // every occurrence in one undo step and exact in case, so "Replace All" is its whole
        // vocabulary here — a single-hit replace would be a second code path for the one verb
        // every other client also has only as the whole-document one.
        let with = gtk::Entry::builder()
            .placeholder_text("Replace with")
            .hexpand(true)
            .build();
        let replace_all = gtk::Button::with_label("Replace All");
        replace_all.set_tooltip_text(Some(
            "Replace every occurrence, exactly as typed (Ctrl+Z undoes it)",
        ));
        let second = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        second.append(&with);
        second.append(&replace_all);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
        rows.append(&row);
        rows.append(&second);
        let bar = gtk::SearchBar::builder()
            .child(&rows)
            .show_close_button(true)
            .build();
        bar.connect_entry(&entry);

        let find = Rc::new(Self {
            bar,
            entry,
            count,
            with,
            app: app.clone(),
            doc: doc.clone(),
        });
        let go = |find: &Rc<Self>, towards| {
            let find = Rc::downgrade(find);
            move || {
                if let Some(find) = find.upgrade() {
                    find.go(towards);
                }
            }
        };
        let (here, forward, back) = (
            go(&find, Towards::Here),
            go(&find, Towards::Next),
            go(&find, Towards::Previous),
        );
        find.entry.connect_search_changed(move |_| here());
        let f = forward.clone();
        find.entry.connect_activate(move |_| f());
        let f = forward.clone();
        find.entry.connect_next_match(move |_| f());
        let b = back.clone();
        find.entry.connect_previous_match(move |_| b());
        next.connect_clicked(move |_| forward());
        previous.connect_clicked(move |_| back());

        let weak = Rc::downgrade(&find);
        let replace = move || {
            if let Some(find) = weak.upgrade() {
                find.replace_all();
            }
        };
        let r = replace.clone();
        find.with.connect_activate(move |_| r());
        replace_all.connect_clicked(move |_| replace());

        // Shift+Enter is not one of the entry's own bindings, and it is the key a hand already
        // holding Enter reaches for.
        let keys = gtk::EventControllerKey::new();
        let b = go(&find, Towards::Previous);
        keys.connect_key_pressed(move |_, key, _, state| {
            let enter = matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter);
            match enter && state.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                true => {
                    b();
                    glib::Propagation::Stop
                }
                false => glib::Propagation::Proceed,
            }
        });
        find.entry.add_controller(keys);

        // Escape, or the close button: the keyboard goes back to the page, the hit still
        // selected under it.
        let weak = Rc::downgrade(&find);
        find.bar.connect_search_mode_enabled_notify(move |bar| {
            if !bar.is_search_mode()
                && let Some(find) = weak.upgrade()
            {
                find.doc.grab_focus();
            }
        });
        find
    }

    /// Open the bar, seeded with the selection when it is a few words on one line — the
    /// thing somebody selected before pressing Ctrl+F is very often the thing they want found.
    pub fn open(&self) {
        if let Some((from, to)) = self.doc.selection()
            && from.block == to.block
            && to.offset - from.offset <= 64
        {
            let text: String = self
                .app
                .input_text(from.block)
                .unwrap_or_default()
                .chars()
                .skip(from.offset)
                .take(to.offset - from.offset)
                .collect();
            self.entry.set_text(&text);
        }
        self.bar.set_search_mode(true);
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    /// Search for `needle` as though it had been typed — for the widget test, which has no
    /// main loop to deliver `search-changed`'s debounce.
    #[cfg(test)]
    pub fn search(&self, needle: &str) {
        self.entry.set_text(needle);
        self.go(Towards::Here);
    }

    /// Type a replacement and press Replace All.
    #[cfg(test)]
    pub fn replace_with(&self, with: &str) {
        self.with.set_text(with);
        self.replace_all();
    }

    /// Enter.
    #[cfg(test)]
    pub fn next(&self) {
        self.go(Towards::Next);
    }

    /// Every occurrence of the search word becomes the replacement, in one undo step.
    fn replace_all(&self) {
        let needle = self.entry.text().to_string();
        if needle.is_empty() {
            return;
        }
        self.entry.remove_css_class("error");
        match self.app.replace(&needle, self.with.text().as_str()) {
            Ok(0) => {
                self.entry.add_css_class("error");
                self.count.set_text("No exact match");
            }
            Ok(n) => self.count.set_text(&format!(
                "Replaced in {n} paragraph{}",
                if n == 1 { "" } else { "s" }
            )),
            Err(error) => self.count.set_text(&error.to_string()),
        }
    }

    fn go(&self, towards: Towards) {
        let needle = self.entry.text().to_string();
        self.entry.remove_css_class("error");
        if needle.is_empty() {
            self.count.set_text("");
            return;
        }
        let hits = hits(&self.app, &needle);
        // From the selection's start when there is one — it is usually the last hit — and the
        // caret otherwise.
        let at = self
            .doc
            .selection()
            .map_or_else(|| self.doc.caret(), |(from, _)| from);
        let Some(index) = step(&hits, at, towards) else {
            self.entry.add_css_class("error");
            self.count.set_text("No results");
            return;
        };
        let from = hits[index];
        self.doc.select(from, end_of(from, &needle));
        self.count
            .set_text(&format!("{} of {}", index + 1, hits.len()));
    }
}
