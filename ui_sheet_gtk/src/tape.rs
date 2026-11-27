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

use grind_sheet::formula::assist::{self, Assist, Ink};
use grind_sheet::locale::Locale;
use grind_sheet::{Document, Tape, TapeInk};
use gtk::{gio, glib, pango};

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
    /// The offers for the word the entry's caret is in, or the call it is inside.
    assist: RefCell<Assist>,
    band: gtk::Label,
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

    let band = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    band.add_css_class("caption");
    band.add_css_class("monospace");
    let bottom = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    bottom.append(&input);
    bottom.append(&band);

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
    header.set_title_widget(Some(&adw::WindowTitle::new("Scratch Calculator", "")));
    header.pack_start(&undo);
    header.pack_start(&redo);
    header.pack_end(&open);
    header.pack_end(&save);
    let view = adw::ToolbarView::builder().content(&toasts).build();
    view.add_top_bar(&header);
    view.add_bottom_bar(&bottom);
    let dialog = adw::Dialog::builder()
        .title("Scratch Calculator")
        // As wide as a window allows up to this, so a long formula and its name stay in view;
        // adw::Dialog shrinks it to fit a narrower one.
        .content_width(880)
        .content_height(640)
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
        assist: RefCell::new(Assist::default()),
        band,
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
    this.input.connect_changed(glib::clone!(
        #[weak]
        this,
        move |_| this.typed()
    ));
    this.input.connect_notify_local(
        Some("cursor-position"),
        glib::clone!(
            #[weak]
            this,
            move |_, _| this.typed()
        ),
    );
    // Tab takes the offer and the arrows choose one, while there are offers; Escape puts them
    // away rather than closing the dialog.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        this,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, modifiers| {
            if !this.assist.borrow().is_offering() || !modifiers.is_empty() {
                return glib::Propagation::Proceed;
            }
            match key {
                gtk::gdk::Key::Tab => this.accept(),
                gtk::gdk::Key::Down => this.assist.borrow_mut().step(1),
                gtk::gdk::Key::Up => this.assist.borrow_mut().step(-1),
                gtk::gdk::Key::Escape => this.assist.borrow_mut().dismiss(),
                _ => return glib::Propagation::Proceed,
            }
            this.draw_band();
            glib::Propagation::Stop
        }
    ));
    this.input.add_controller(keys);
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

    /// The entry's words coloured, and the offers for the one its caret is in.
    fn typed(&self) {
        let text = self.input.text().to_string();
        let caret = byte_at(&text, self.input.position());
        {
            let tape = self.tape.borrow();
            tape.assist(&mut self.assist.borrow_mut(), &text, caret);
            self.input.set_attributes(&inked(&tape, &text, Some(caret)));
        }
        self.draw_band();
    }

    /// Tab: the chosen offer into the entry, the caret after it.
    fn accept(&self) {
        let Some((span, insert)) = self.assist.borrow_mut().accept() else {
            return;
        };
        let mut text = self.input.text().to_string();
        if text.get(span.clone()).is_none() {
            return;
        }
        text.replace_range(span.clone(), &insert);
        let caret = text[..span.start + insert.len()].chars().count();
        self.input.set_text(&text);
        self.input.set_position(i32::try_from(caret).unwrap_or(-1));
        self.typed();
    }

    fn draw_band(&self) {
        let pieces = assist::band(&self.assist.borrow(), true);
        if pieces.is_empty() {
            self.band.set_markup(&format!(
                "<span alpha=\"60%\">{}</span>",
                glib::markup_escape_text(
                    "Enter: add the line  \u{b7}  Tab: complete a name or a function"
                )
            ));
            return;
        }
        let accent = rgb_hex(ink_rgb(TapeInk::Name, is_dark()).unwrap_or((0, 0, 0)));
        let markup: String = pieces
            .iter()
            .map(|piece| {
                let text = glib::markup_escape_text(&piece.text);
                match piece.ink {
                    Ink::Strong => format!("<b><span foreground=\"{accent}\">{text}</span></b>"),
                    Ink::Muted => format!("<span alpha=\"60%\">{text}</span>"),
                    Ink::Plain => text.to_string(),
                }
            })
            .collect();
        self.band.set_markup(&markup);
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
        drop(tape);
        // A name the entry reads may have come or gone with the line.
        self.typed();
    }

    /// One line: its handle, its text editable in place, its value, and what can be done to it.
    fn row(self: &Rc<Self>, at: usize, line: &grind_sheet::TapeLine) -> adw::EntryRow {
        let row = adw::EntryRow::builder()
            .title(format!("#{}", line.handle))
            .text(&line.text)
            .show_apply_button(true)
            .build();
        row.add_css_class("monospace");
        row.set_attributes(Some(&inked(&self.tape.borrow(), &line.text, None)));
        row.connect_changed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |row| {
                let text = row.text().to_string();
                row.set_attributes(Some(&inked(&this.tape.borrow(), &text, None)));
            }
        ));
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
            .xalign(1.0)
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

fn is_dark() -> bool {
    adw::StyleManager::default().is_dark()
}

/// Adwaita's palette for each kind of word; `None` for the label's own ink.
fn ink_rgb(ink: TapeInk, dark: bool) -> Option<(u8, u8, u8)> {
    let pick = |light, night| Some(if dark { night } else { light });
    match ink {
        TapeInk::Definition => None,
        TapeInk::Name | TapeInk::Handle => pick((0x1c, 0x71, 0xd8), (0x78, 0xae, 0xed)),
        TapeInk::Function => pick((0x81, 0x3d, 0x9c), (0xdc, 0x8a, 0xdd)),
        TapeInk::Number => pick((0x26, 0xa2, 0x69), (0x8f, 0xf0, 0xa4)),
        TapeInk::Text => pick((0xc6, 0x46, 0x00), (0xff, 0xbe, 0x6f)),
        TapeInk::Error | TapeInk::Unknown => pick((0xc0, 0x1c, 0x28), (0xff, 0x7b, 0x63)),
    }
}

fn rgb_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Pango's attributes for `text`'s words, in the byte ranges `Tape::spans` gives, which are
/// Pango's own unit. The name a line defines is bold.
fn inked(tape: &Tape, text: &str, caret: Option<usize>) -> pango::AttrList {
    let list = pango::AttrList::new();
    let dark = is_dark();
    for span in tape.spans(text, caret) {
        let mut attr: pango::Attribute = match ink_rgb(span.ink, dark) {
            Some((r, g, b)) => pango::AttrColor::new_foreground(
                u16::from(r) * 257,
                u16::from(g) * 257,
                u16::from(b) * 257,
            )
            .into(),
            None => pango::AttrInt::new_weight(pango::Weight::Bold).into(),
        };
        attr.set_start_index(u32::try_from(span.range.start).unwrap_or(u32::MAX));
        attr.set_end_index(u32::try_from(span.range.end).unwrap_or(u32::MAX));
        list.insert(attr);
    }
    list
}

/// The byte offset of the `chars`th character, as a GTK caret position counts them.
fn byte_at(text: &str, chars: i32) -> usize {
    usize::try_from(chars)
        .ok()
        .and_then(|n| text.char_indices().nth(n).map(|(at, _)| at))
        .unwrap_or(text.len())
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
        if !crate::gtk_for_test() {
            eprintln!(
                "no display, or GTK is another test's — skipping the tape dialog's widget case"
            );
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

        // A word offers the tape's names first, Tab takes it, and the entry colours it.
        tape.input.set_text("net = 1200");
        tape.input.emit_activate();
        tape.input.set_text("2 * ne");
        tape.input.set_position(-1);
        assert!(tape.band.text().starts_with("net"), "{}", tape.band.text());
        tape.accept();
        assert_eq!(tape.input.text(), "2 * net");
        tape.input.set_text("");

        tape.dialog.close();
        assert!(!tape.is_open());
        tape.present(&parent);
        assert_eq!(rows(&tape.list), 2, "closing kept the tape");
        parent.close();
    }
}
