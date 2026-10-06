// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Insert ▸ Link… (Ctrl+K) — one entry in a popover at the caret.
//!
//! What it links is decided when it opens, in the order a word processor decides it: the
//! selection if there is one; otherwise the whole link the caret is on, so the same key edits
//! an existing link; otherwise nothing, and the target typed is inserted as its own text and
//! linked. Every write is `App::set_link` — this file has no idea of what a link is that the
//! document does not share, and `grind text link` is the same edit from the command line.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_text::{App, Caret};
use gtk::glib;

use crate::view::Doc;

pub struct Editor {
    app: Arc<App>,
    doc: Doc,
    popover: gtk::Popover,
    entry: gtk::Entry,
    remove: gtk::Button,
    /// The characters the entry's target goes on, fixed when the popover opened — `None` when
    /// there were none and the target is inserted as its own text.
    span: Cell<Option<(Caret, Caret)>>,
}

impl Editor {
    pub fn new(app: &Arc<App>, doc: &Doc) -> Rc<Self> {
        let entry = gtk::Entry::builder()
            .placeholder_text("https://\u{2026} or #bookmark")
            .width_chars(32)
            .build();
        let apply = gtk::Button::with_label("Link");
        apply.add_css_class("suggested-action");
        let remove = gtk::Button::with_label("Remove");
        remove.add_css_class("destructive-action");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&entry);
        row.append(&apply);
        row.append(&remove);

        let popover = gtk::Popover::builder().child(&row).build();
        popover.set_parent(doc);
        popover.set_position(gtk::PositionType::Bottom);

        let editor = Rc::new(Self {
            app: app.clone(),
            doc: doc.clone(),
            popover,
            entry,
            remove,
            span: Cell::new(None),
        });
        entry_activates(&editor, &apply);
        editor.remove.connect_clicked(glib::clone!(
            #[weak]
            editor,
            move |_| editor.unlink()
        ));
        editor.entry.connect_changed(|entry| {
            entry.remove_css_class("error");
            entry.set_tooltip_text(None);
        });
        editor.popover.connect_closed(glib::clone!(
            #[weak]
            editor,
            move |_| {
                editor.doc.grab_focus();
            }
        ));
        editor
    }

    /// Open on the selection, the link at the caret, or nothing.
    pub fn open(&self) {
        self.prepare();
        if let Some(rect) = self.doc.caret_rect() {
            self.popover.set_pointing_to(Some(&rect));
        }
        self.popover.popup();
        self.entry.grab_focus();
    }

    /// Decide what the popover is about, and fill the entry with the target already there —
    /// everything [`Editor::open`] does short of showing it.
    pub(crate) fn prepare(&self) {
        let caret = self.doc.caret();
        let (span, href) = match self.doc.selection() {
            Some((from, to)) => (Some((from, to)), self.app.link(from, to).ok().flatten()),
            None => match self.app.link_at(caret).ok().flatten() {
                Some(link) => (Some((link.from, link.to)), Some(link.href)),
                None => (None, None),
            },
        };
        self.span.set(span);
        self.entry.set_text(href.as_deref().unwrap_or(""));
        self.entry.select_region(0, -1);
        self.remove.set_visible(href.is_some());
    }

    fn apply(&self) {
        let href = self.entry.text().trim().to_owned();
        match self.link_to(&href) {
            Ok(()) => self.popover.popdown(),
            Err(why) => self.refuse(&why),
        }
    }

    /// Link what the popover opened on to `href` — the entry's Enter, without the entry.
    pub(crate) fn link_to(&self, href: &str) -> Result<(), String> {
        if href.is_empty() {
            return Err("A link needs somewhere to point".to_owned());
        }
        let (from, to) = match self.span.get() {
            Some(span) => span,
            // Nothing to put it on: the target becomes its own text, the way pasting a URL
            // into a word processor's link box with nothing selected does.
            None => {
                let at = self.doc.caret();
                self.app
                    .insert_text(at, href)
                    .map_err(|error| error.to_string())?;
                let end = Caret {
                    block: at.block,
                    offset: at.offset + href.chars().count(),
                };
                (at, end)
            }
        };
        self.app
            .set_link(from, to, Some(href))
            .map_err(|error| error.to_string())?;
        self.doc.go_to(to);
        Ok(())
    }

    pub(crate) fn unlink(&self) {
        if let Some((from, to)) = self.span.get() {
            let _ = self.app.set_link(from, to, None);
        }
        self.popover.popdown();
    }

    /// Said in the box, which is where the reader is looking; the popover stays to be fixed.
    fn refuse(&self, why: &str) {
        self.entry.add_css_class("error");
        self.entry.set_tooltip_text(Some(why));
    }
}

fn entry_activates(editor: &Rc<Editor>, apply: &gtk::Button) {
    editor.entry.connect_activate(glib::clone!(
        #[weak]
        editor,
        move |_| editor.apply()
    ));
    apply.connect_clicked(glib::clone!(
        #[weak]
        editor,
        move |_| editor.apply()
    ));
}
