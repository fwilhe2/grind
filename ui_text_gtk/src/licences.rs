// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Every third-party licence, built only when somebody asks for one** (`doc/third-party.md`).
//!
//! The About dialog's Legal page used to carry one section per component, and libadwaita builds
//! every one of them as a `GtkLabel` the moment the page opens: three hundred-odd labels and
//! some 700 KB of licence text laid out at once, which took the window down. So the page now
//! holds one section — the index, a line per component — and a link to a dialog of its own,
//! where a component's text is put into a `GtkTextView` when its row is activated and not
//! before.
//!
//! `ui_sheet_gtk/src/licences.rs` is this file, word for word, for `lint.rs`'s reason: there is
//! no crate a widget both GTK shells could share.

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_core::third_party::{self, Component};
use gtk::glib;

/// The link on the Legal page that opens [`show`], answered by `activate-link`.
const URI: &str = "grind:third-party";

/// Put the index on `about`'s Legal page, with a link to every text.
pub fn add_to(about: &adw::AboutDialog) {
    let mut index = String::from(
        "Grind is built from the third-party components below, each under its own licence.\n\n",
    );
    for component in third_party::components() {
        index.push_str(&format!("{}  ({})\n", component.title(), component.licence));
    }
    let markup = format!(
        "{}\n<a href=\"{URI}\">Show every licence text</a>",
        glib::markup_escape_text(&index)
    );
    about.add_legal_section(
        "Third-Party Components",
        None,
        gtk::License::Custom,
        Some(&markup),
    );
    about.connect_activate_link(|about, uri| {
        if uri != URI {
            return false;
        }
        show(about);
        true
    });
}

/// A dialog over `about`: the components as a list, and one component's text a page deeper.
fn show(about: &adw::AboutDialog) {
    let navigation = adw::NavigationView::new();

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    for component in third_party::components() {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&component.title()))
            .subtitle(glib::markup_escape_text(component.licence))
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        list.append(&row);
    }
    let nav = navigation.clone();
    list.connect_row_activated(move |_, row| {
        if let Some(component) = usize::try_from(row.index())
            .ok()
            .and_then(|at| third_party::components().get(at))
        {
            nav.push(&text_page(component));
        }
    });
    let clamp = adw::Clamp::builder()
        .child(&list)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();
    navigation.add(&page("Third-Party Licences", &scrolled));

    let dialog = adw::Dialog::builder()
        .title("Third-Party Licences")
        .content_width(640)
        .content_height(600)
        .child(&navigation)
        .build();
    dialog.present(Some(about));
}

/// One component's notices, read out of the core only now.
fn text_page(component: &Component) -> adw::NavigationPage {
    let mut text = String::new();
    if !component.copyright.is_empty() {
        text.push_str(component.copyright);
        text.push_str("\n\n");
    }
    text.push_str(&component.text());
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .build();
    view.buffer().set_text(&text);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&view)
        .build();
    page(&component.title(), &scrolled)
}

fn page(title: &str, content: &impl IsA<gtk::Widget>) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(content));
    adw::NavigationPage::new(&toolbar, title)
}
