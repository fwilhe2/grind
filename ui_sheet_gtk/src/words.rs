// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Personal Dictionary…** — `grind_spell::personal`'s list with a window in front of it, so a
//! word added by mistake can be taken back and one added before it is typed. `grind words` is the
//! same list from a terminal; `ui_text_gtk/src/words.rs` is a copy, for `code.rs`'s reason.

use std::rc::Rc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

/// Open the dialog over `parent`. `changed` runs after every add and every removal, so the
/// window can attach its speller afresh and redraw what is underlined.
pub fn present(parent: &impl IsA<gtk::Widget>, changed: impl Fn() + 'static) {
    let changed: Rc<dyn Fn()> = Rc::new(changed);
    let dialog = adw::AlertDialog::new(
        Some("Personal Dictionary"),
        Some(
            "Words spelling accepts in every document and every language. Yours, kept on this \
             computer and never written into a document.",
        ),
    );

    let entry = gtk::Entry::builder()
        .placeholder_text("Add a word")
        .hexpand(true)
        .build();
    let add = gtk::Button::with_label("Add");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&entry);
    row.append(&add);

    let error = gtk::Label::builder().xalign(0.0).visible(false).build();
    error.add_css_class("error");

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    list.set_placeholder(Some(&gtk::Label::new(Some("No words yet"))));
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .min_content_height(220)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&row);
    body.append(&error);
    body.append(&scroller);
    dialog.set_extra_child(Some(&body));
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");

    fill(&list, &error, &changed);

    let adding = {
        let (entry, error, list, changed) =
            (entry.clone(), error.clone(), list.clone(), changed.clone());
        move || match grind_spell::personal::word(&entry.text()) {
            Err(why) => say(&error, why),
            Ok(word) => match grind_spell::personal::add(&word) {
                Err(why) => say(&error, &why.to_string()),
                Ok(_) => {
                    error.set_visible(false);
                    entry.set_text("");
                    changed();
                    fill(&list, &error, &changed);
                }
            },
        }
    };
    let on_enter = adding.clone();
    entry.connect_activate(move |_| on_enter());
    add.connect_clicked(move |_| adding());

    dialog.present(Some(parent));
    entry.grab_focus();
}

/// One row per word on the list, each with its own Remove — rebuilt after every change, since
/// the list is the file's and not this dialog's.
fn fill(list: &gtk::ListBox, error: &gtk::Label, changed: &Rc<dyn Fn()>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for word in grind_spell::personal::words() {
        let row = adw::ActionRow::builder().title(&word).build();
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Remove from your dictionary")
            .valign(gtk::Align::Center)
            .build();
        remove.add_css_class("flat");
        let (rows, error, changed) = (list.clone(), error.clone(), changed.clone());
        remove.connect_clicked(move |_| {
            match grind_spell::personal::remove(&word) {
                Ok(_) => error.set_visible(false),
                Err(why) => say(&error, &why.to_string()),
            }
            changed();
            fill(&rows, &error, &changed);
        });
        row.add_suffix(&remove);
        list.append(&row);
    }
}

fn say(label: &gtk::Label, text: &str) {
    label.set_text(text);
    label.set_visible(true);
}
