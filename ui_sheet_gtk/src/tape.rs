// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tape dialog: `grind_sheet::Tape` (`doc/calc.md`), a scratch calculator over the sheet. An
//! `adw::Dialog`, so it opens centred on the window it belongs to; the window keeps it, so closing
//! it keeps the tape for the next time. Every decision is the tape's; this holds the widgets and
//! redraws them after each call.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_sheet::locale::Locale;
use grind_sheet::{Document, Tape};
use gtk::{gio, glib};

pub struct TapeDialog {
    tape: RefCell<Tape>,
    dialog: adw::Dialog,
    /// Between `present` and the dialog's `closed`.
    open: Cell<bool>,
    list: gtk::ListBox,
    empty: gtk::Label,
    banner: adw::Banner,
    toasts: adw::ToastOverlay,
    input: gtk::Entry,
    undo: gtk::Button,
    redo: gtk::Button,
    /// Hands a promoted tape to the sheet window.
    open_as_sheet: Rc<dyn Fn(Document)>,
}

/// A tape reading numbers in `locale`, not yet shown. Whoever holds it keeps the tape: every
/// handler holds it weakly.
pub fn new(locale: Option<Locale>, open_as_sheet: Rc<dyn Fn(Document)>) -> Rc<TapeDialog> {
    let input = gtk::Entry::builder()
        .placeholder_text("1200 * 1.19")
        .hexpand(true)
        .build();
    input.add_css_class("monospace");
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    let hint = match locale.as_ref() {
        Some(locale) => format!(
            "Type a formula without its =, as in a cell, and press Enter.\n\
             #2 reads line 2; vat = 0.19 names a line so others can write vat.\n\
             Numbers are written as {} spells them: {}.",
            locale.tag(),
            grind_sheet::locale::sample(Some(locale))
        ),
        None => "Type a formula without its =, as in a cell, and press Enter.\n\
                 #2 reads line 2; vat = 0.19 names a line so others can write vat."
            .to_owned(),
    };
    let empty = gtk::Label::builder()
        .label(hint)
        .wrap(true)
        .xalign(0.0)
        .build();
    empty.add_css_class("dim-label");
    let banner = adw::Banner::new("");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&empty);
    content.append(&list);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&content)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&banner);
    body.append(&scroller);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&body));

    let bottom = gtk::Box::builder()
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    bottom.append(&input);

    let undo = gtk::Button::from_icon_name("edit-undo-symbolic");
    undo.set_tooltip_text(Some("Undo (Ctrl+Z)"));
    let redo = gtk::Button::from_icon_name("edit-redo-symbolic");
    redo.set_tooltip_text(Some("Redo (Ctrl+Shift+Z)"));
    let open = gtk::Button::with_label("Open as Spreadsheet");
    open.add_css_class("suggested-action");
    open.set_tooltip_text(Some(
        "One row per line in the window: its name in A, its number or formula in B",
    ));
    let save = gtk::Button::from_icon_name("document-save-as-symbolic");
    save.set_tooltip_text(Some("Save as Spreadsheet…"));

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Scratch Calculator",
        "Experimental",
    )));
    header.pack_start(&undo);
    header.pack_start(&redo);
    header.pack_end(&open);
    header.pack_end(&save);
    let view = adw::ToolbarView::builder().content(&toasts).build();
    view.add_top_bar(&header);
    view.add_bottom_bar(&bottom);
    let dialog = adw::Dialog::builder()
        .title("Scratch Calculator")
        .content_width(560)
        .content_height(560)
        .child(&view)
        .build();

    let this = Rc::new(TapeDialog {
        tape: RefCell::new(Tape::new(locale)),
        dialog,
        open: Cell::new(false),
        list,
        empty,
        banner,
        toasts,
        input,
        undo,
        redo,
        open_as_sheet,
    });

    this.input.connect_activate(glib::clone!(
        #[weak]
        this,
        move |input| {
            let text = input.text();
            if text.trim().is_empty() {
                return;
            }
            let pushed = this.tape.borrow_mut().push(&text);
            match pushed {
                Ok(_) => {
                    input.set_text("");
                    this.refresh();
                }
                // The line stays, so the slip can be fixed rather than retyped.
                Err(error) => this.say(&error.to_string()),
            }
        }
    ));
    this.undo.connect_clicked(glib::clone!(
        #[weak]
        this,
        move |_| this.step(true)
    ));
    this.redo.connect_clicked(glib::clone!(
        #[weak]
        this,
        move |_| this.step(false)
    ));
    open.connect_clicked(glib::clone!(
        #[weak]
        this,
        move |_| {
            let promoted = this.tape.borrow().promote();
            match promoted {
                Ok(doc) => {
                    // The sheet is what to look at now; the tape stays for the next time.
                    this.dialog.close();
                    (this.open_as_sheet)(doc);
                }
                Err(error) => this.say(&error.to_string()),
            }
        }
    ));
    save.connect_clicked(glib::clone!(
        #[weak]
        this,
        move |_| this.save()
    ));

    let keys = gtk::ShortcutController::new();
    keys.set_scope(gtk::ShortcutScope::Managed);
    for (accel, back) in [
        ("<Control>z", true),
        ("<Control><Shift>z", false),
        ("<Control>y", false),
    ] {
        let weak = Rc::downgrade(&this);
        keys.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(accel),
            Some(gtk::CallbackAction::new(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    this.step(back);
                }
                glib::Propagation::Stop
            })),
        ));
    }
    this.dialog.add_controller(keys);
    this.dialog.connect_closed(glib::clone!(
        #[weak]
        this,
        move |_| this.open.set(false)
    ));

    this.refresh();
    this
}

impl TapeDialog {
    /// Show the tape over `parent`, centred on its window.
    pub fn present(&self, parent: &impl IsA<gtk::Widget>) {
        self.open.set(true);
        self.dialog.present(Some(parent));
        self.input.grab_focus();
    }

    /// Whether the dialog is showing — while it is, the window's Undo and Redo are the tape's.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    fn say(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Undo, or redo with `back` false.
    pub fn step(self: &Rc<Self>, back: bool) {
        let done = if back {
            self.tape.borrow_mut().undo()
        } else {
            self.tape.borrow_mut().redo()
        };
        if done {
            self.refresh();
        }
    }

    /// Every row again from the tape: a tape is short, and a row per line cannot then disagree
    /// with it.
    fn refresh(self: &Rc<Self>) {
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        let tape = self.tape.borrow();
        for (at, line) in tape.lines().into_iter().enumerate() {
            self.list.append(&self.row(at, &line));
        }
        let any = !tape.is_empty();
        self.list.set_visible(any);
        self.empty.set_visible(!any);
        let circles = tape.circles();
        self.banner.set_revealed(circles.is_some());
        if let Some(circles) = circles {
            self.banner.set_title(&format!(
                "{circles}: #NUM! until the circle is broken, and no spreadsheet before then"
            ));
        }
        self.undo.set_sensitive(tape.can_undo());
        self.redo.set_sensitive(tape.can_redo());
    }

    /// One line: its handle, its text editable in place, its value, and what can be done to it.
    fn row(self: &Rc<Self>, at: usize, line: &grind_sheet::TapeLine) -> adw::EntryRow {
        let row = adw::EntryRow::builder()
            .title(format!("#{}", line.handle))
            .text(&line.text)
            .show_apply_button(true)
            .build();
        row.add_css_class("monospace");
        row.connect_apply(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |row| {
                let edited = this.tape.borrow_mut().edit(at, &row.text());
                match edited {
                    Ok(()) => this.refresh(),
                    Err(error) => this.say(&error.to_string()),
                }
            }
        ));

        let value = gtk::Label::builder()
            .label(&line.value)
            .selectable(true)
            .valign(gtk::Align::Center)
            .build();
        value.add_css_class("numeric");
        value.add_css_class(if line.error { "error" } else { "heading" });
        row.add_suffix(&value);

        let extract = gtk::Button::with_label("Name…");
        extract.set_tooltip_text(Some("Name a number in this line"));
        extract.set_valign(gtk::Align::Center);
        extract.add_css_class("flat");
        extract.connect_clicked(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_| this.extract(at)
        ));
        row.add_suffix(&extract);

        let delete = gtk::Button::from_icon_name("user-trash-symbolic");
        delete.set_tooltip_text(Some("Delete the Line"));
        delete.set_valign(gtk::Align::Center);
        delete.add_css_class("flat");
        delete.connect_clicked(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                let deleted = this.tape.borrow_mut().delete(at);
                match deleted {
                    Ok(()) => this.refresh(),
                    Err(error) => this.say(&error.to_string()),
                }
            }
        ));
        row.add_suffix(&delete);
        row
    }

    /// *Name a Number…*: the number and a name for it asked, then `Tape::extract`.
    fn extract(self: &Rc<Self>, at: usize) {
        let number = gtk::Entry::builder()
            .placeholder_text("The number, as the line writes it: 0.19")
            .activates_default(true)
            .build();
        let name = gtk::Entry::builder()
            .placeholder_text("Its name: vat")
            .activates_default(true)
            .build();
        let nth = gtk::Entry::builder()
            .placeholder_text("Which one, if it is there twice: 2")
            .activates_default(true)
            .build();
        let fields = gtk::Box::new(gtk::Orientation::Vertical, 6);
        fields.append(&number);
        fields.append(&name);
        fields.append(&nth);
        let dialog = adw::AlertDialog::new(
            Some("Name a Number"),
            Some("It becomes a named line just before this one, and this line reads the name."),
        );
        dialog.set_extra_child(Some(&fields));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("name", "Name It");
        dialog.set_response_appearance("name", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("name"));
        dialog.set_close_response("cancel");
        dialog.choose(
            &self.dialog,
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |response| {
                    if response != "name" {
                        return;
                    }
                    let which = nth.text().trim().parse().ok();
                    let extracted = this.tape.borrow_mut().extract(
                        at,
                        &number.text(),
                        which,
                        name.text().trim(),
                    );
                    match extracted {
                        Ok(()) => this.refresh(),
                        Err(error) => this.say(&error.to_string()),
                    }
                }
            ),
        );
    }

    fn save(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Save as Spreadsheet")
            .filters(&crate::spreadsheet_save_filters())
            .initial_name("tape.fods")
            .build();
        let parent = self.dialog.root().and_downcast::<gtk::Window>();
        dialog.save(
            parent.as_ref(),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |result| {
                    let Some(path) = result.ok().and_then(|file| file.path()) else {
                        return;
                    };
                    let saved = this.tape.borrow().save(&path);
                    match saved {
                        Ok(()) => this.say(&format!(
                            "Saved {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        )),
                        Err(error) => this.say(&error.to_string()),
                    }
                }
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(list: &gtk::ListBox) -> usize {
        let mut n = 0;
        let mut child = list.first_child();
        while let Some(row) = child {
            n += 1;
            child = row.next_sibling();
        }
        n
    }

    /// Enter adds a line and Undo takes it away, and closing the dialog keeps the tape for the
    /// next time it is shown.
    #[test]
    fn lines_are_typed_undone_and_kept_across_closing() {
        if gtk::init().is_err() || adw::init().is_err() {
            eprintln!("no display — skipping the tape dialog's widget case");
            return;
        }
        let tape = new(None, Rc::new(|_| {}));
        let parent = adw::Window::new();
        parent.present();
        tape.present(&parent);
        assert!(tape.is_open());

        tape.input.set_text("1200 * 1.19");
        tape.input.emit_activate();
        assert_eq!(rows(&tape.list), 1, "Enter adds the line");
        assert_eq!(tape.input.text(), "", "and clears the entry for the next");
        tape.input.set_text("1 +");
        tape.input.emit_activate();
        assert_eq!(
            rows(&tape.list),
            1,
            "a line that does not read is not added"
        );
        assert_eq!(tape.input.text(), "1 +", "and stays to be fixed");

        assert!(tape.undo.is_sensitive());
        tape.undo.emit_clicked();
        assert!(!tape.list.is_visible(), "undo took the line away");
        tape.step(false);
        assert_eq!(rows(&tape.list), 1, "and redo brought it back");

        tape.dialog.close();
        assert!(!tape.is_open());
        tape.present(&parent);
        assert_eq!(rows(&tape.list), 1, "closing kept the tape");
        parent.close();
    }
}
