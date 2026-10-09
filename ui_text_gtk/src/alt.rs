// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Alt Text… — a picture's alternative text, in a libadwaita dialog.
//!
//! The picture is shown at the top, because alt text is written *looking at* what it describes;
//! under it an entry row for the short text a screen reader says (`svg:title`, and Markdown's
//! `![alt]`), the one sentence of advice `grind_text::picture::advice` gives every client as it is
//! typed, and a box for the long description (`svg:desc`). An `adw::Dialog` rather than an
//! `adw::AlertDialog`, since it carries a picture and a multi-line field, and it becomes a bottom
//! sheet by itself on a narrow window. Which picture is grind_text's answer
//! (`picture::at`), and the write is `picture::set_alt` — the same pair `grind text alt` and
//! the five other clients reach.

use std::rc::Rc;
use std::sync::Arc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_text::{App, Caret, ImageView, picture};
use gtk::glib;

/// Open the dialog on the picture at `at`; `done` hears the error, if the write had one.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    app: &Arc<App>,
    at: Caret,
    image: &ImageView,
    done: impl Fn(Option<String>) + 'static,
) {
    let title = adw::EntryRow::builder()
        .title("Alt text")
        .text(image.title.as_deref().unwrap_or(""))
        .show_apply_button(false)
        .build();
    let advice = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["caption", "dim-label"])
        .margin_top(6)
        .build();
    let count = gtk::Label::builder()
        .xalign(1.0)
        .css_classes(["caption", "dim-label", "numeric"])
        .margin_top(6)
        .build();
    let note = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    advice.set_hexpand(true);
    note.append(&advice);
    note.append(&count);
    let short = adw::PreferencesGroup::builder()
        .description("What a screen reader says in the picture's place.")
        .build();
    short.add(&title);

    let long = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(10)
        .bottom_margin(10)
        .left_margin(12)
        .right_margin(12)
        .height_request(96)
        .build();
    long.buffer()
        .set_text(image.description.as_deref().unwrap_or(""));
    long.update_property(&[gtk::accessible::Property::Label("Long description")]);
    let frame = gtk::Frame::builder()
        .child(&long)
        .css_classes(["card"])
        .build();
    let detail = adw::PreferencesGroup::builder()
        .title("Long Description")
        .description("Optional — for a chart or a diagram whose detail a sentence cannot hold.")
        .build();
    detail.add(&frame);

    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    if let Some(texture) = crate::view::texture(image) {
        let shown = gtk::Picture::builder()
            .paintable(&texture)
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .height_request(160)
            .css_classes(["card"])
            .build();
        // Decorative here: the dialog is *about* this picture, and its text is being written.
        shown.set_accessible_role(gtk::AccessibleRole::Presentation);
        column.append(&shown);
    }
    let fields = gtk::Box::new(gtk::Orientation::Vertical, 0);
    fields.append(&short);
    fields.append(&note);
    column.append(&fields);
    column.append(&detail);
    let clamp = adw::Clamp::builder()
        .maximum_size(520)
        .child(&column)
        .build();
    let page = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&clamp)
        .build();

    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save);
    let view = adw::ToolbarView::builder().content(&page).build();
    view.add_top_bar(&header);

    let dialog = adw::Dialog::builder()
        .title("Alt Text")
        .content_width(480)
        .child(&view)
        .default_widget(&save)
        .focus_widget(&title)
        .build();

    let advise = {
        let (advice, count) = (advice.clone(), count.clone());
        move |text: &str| {
            advice.set_label(picture::advice(text).unwrap_or(""));
            let n = text.trim().chars().count();
            count.set_label(&format!("{n} / {}", picture::SHORT));
            match n > picture::SHORT {
                true => count.add_css_class("warning"),
                false => count.remove_css_class("warning"),
            }
        }
    };
    advise(&title.text());
    title.connect_changed(move |row| advise(&row.text()));

    let done = Rc::new(done);
    let commit = glib::clone!(
        #[weak]
        dialog,
        #[weak]
        title,
        #[weak]
        long,
        #[strong]
        app,
        #[strong]
        done,
        move || {
            let buffer = long.buffer();
            let description = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let error = picture::set_alt(&app, at, &title.text(), &description)
                .err()
                .map(|e| e.to_string());
            dialog.close();
            done(error);
        }
    );
    save.connect_clicked({
        let commit = commit.clone();
        move |_| commit()
    });
    // Enter in the short field saves — the one field most people fill in.
    title.connect_entry_activated(move |_| commit());
    cancel.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));
    dialog.present(Some(parent));
}
