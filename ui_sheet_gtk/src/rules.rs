// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! *Conditional Formatting…* — the sheet's rules as a list, each one removable, and a row to add
//! one over the selection (`doc/conditional-format.md` §4).
//!
//! Built on the palette rather than on the format bar, because a rule is not a property of the
//! selection — it is a thing the sheet has, and the bar's admission test (`doc/sheet-shell.md`,
//! "Four surfaces") is "reads and writes a property of the selection". Every decision about what
//! a rule *is* — the looks on offer, how `=B2>0` becomes ODF, how one reads in a list, what is
//! refused — is `grind_sheet::rule`'s, shared with every other shell; this file is where the
//! fields go. Each add and each removal is one undo step in the document, so there is no OK.

use std::rc::Rc;
use std::sync::Arc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use grind_sheet::App;
use grind_sheet::model::Pos;
use grind_sheet::rule::{self, Look};
use gtk::glib;

/// Open the dialog over `window`, for `sheet`, with `selection` in the new rule's range field.
pub fn present(
    window: &impl IsA<gtk::Widget>,
    app: &Arc<App>,
    sheet: usize,
    selection: (Pos, Pos),
) {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    let empty = gtk::Label::builder()
        .label(
            "No rules on this sheet. A rule draws a cell differently while a formula is true \
             there — =B2>100 fills the cells over a hundred.",
        )
        .wrap(true)
        .xalign(0.0)
        .build();
    empty.add_css_class("dim-label");

    let toasts = adw::ToastOverlay::new();
    let say: Rc<dyn Fn(&str)> = {
        let toasts = toasts.clone();
        Rc::new(move |text: &str| toasts.add_toast(adw::Toast::new(text)))
    };

    // The list is rebuilt from the document after every change rather than edited in place: a
    // rule has no identity but its index, and removing one renumbers the rest. A row reaches the
    // rebuild through a weak handle, since the rebuild owns the rows.
    let refresh: Rc<std::cell::RefCell<std::rc::Weak<dyn Fn()>>> = Rc::new(
        std::cell::RefCell::new(std::rc::Weak::<Box<dyn Fn()>>::new()),
    );
    let rebuild: Rc<dyn Fn()> = {
        let (list, empty, app, refresh) =
            (list.clone(), empty.clone(), app.clone(), refresh.clone());
        Rc::new(move || {
            while let Some(row) = list.first_child() {
                list.remove(&row);
            }
            let rules = app.rules(sheet).unwrap_or_default();
            for (index, r) in rules.iter().enumerate() {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&rule::condition_to_display(r)))
                    .subtitle(glib::markup_escape_text(&format!(
                        "{} · {}",
                        rule::ranges_text(r),
                        rule::look_text(&r.style)
                    )))
                    .build();
                let remove = gtk::Button::from_icon_name("user-trash-symbolic");
                remove.set_tooltip_text(Some("Remove this rule"));
                remove.set_valign(gtk::Align::Center);
                remove.add_css_class("flat");
                let (app, refresh) = (app.clone(), refresh.clone());
                remove.connect_clicked(move |_| {
                    let _ = app.remove_rule(sheet, index);
                    let refresh = refresh.borrow().upgrade();
                    if let Some(refresh) = refresh {
                        refresh();
                    }
                });
                row.add_suffix(&remove);
                list.append(&row);
            }
            list.set_visible(!rules.is_empty());
            empty.set_visible(rules.is_empty());
        })
    };
    *refresh.borrow_mut() = Rc::downgrade(&rebuild);
    rebuild();

    let (start, end) = selection;
    let ranges = adw::EntryRow::builder()
        .title("Applies to")
        .text(rule::range_hint(start, end))
        .build();
    let condition = adw::EntryRow::builder()
        .title("Condition, written for the first cell")
        .text(rule::condition_hint(start))
        .build();
    let labels: Vec<&str> = Look::ALL.iter().map(|look| look.label()).collect();
    let look = adw::ComboRow::builder()
        .title("Draw it with")
        .model(&gtk::StringList::new(&labels))
        .build();
    let fields = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    fields.add_css_class("boxed-list");
    fields.append(&ranges);
    fields.append(&condition);
    fields.append(&look);
    let add = gtk::Button::with_label("Add Rule");
    add.add_css_class("suggested-action");
    add.set_halign(gtk::Align::End);
    add.connect_clicked(glib::clone!(
        #[strong]
        app,
        #[strong]
        say,
        #[strong]
        rebuild,
        #[strong]
        ranges,
        #[strong]
        condition,
        #[strong]
        look,
        move |_| {
            let chosen = Look::ALL[(look.selected() as usize).min(Look::ALL.len() - 1)];
            match rule::add_from_input(&app, sheet, &ranges.text(), &condition.text(), chosen) {
                Ok((on, _)) if on != sheet => say("Added on the sheet the range names"),
                Ok(_) => rebuild(),
                Err(error) => say(&error),
            }
        }
    ));
    condition.connect_entry_activated(glib::clone!(
        #[weak]
        add,
        move |_| add.emit_clicked()
    ));

    let order = gtk::Label::builder()
        .label("The first rule that holds at a cell is the one drawn.")
        .wrap(true)
        .xalign(0.0)
        .build();
    order.add_css_class("dim-label");
    order.add_css_class("caption");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    for widget in [
        empty.upcast_ref::<gtk::Widget>(),
        list.upcast_ref(),
        order.upcast_ref(),
        fields.upcast_ref(),
        add.upcast_ref(),
    ] {
        content.append(widget);
    }
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&content)
        .build();
    toasts.set_child(Some(&scroller));

    let view = adw::ToolbarView::builder().content(&toasts).build();
    view.add_top_bar(&adw::HeaderBar::new());
    let dialog = adw::Dialog::builder()
        .title("Conditional Formatting")
        .content_width(520)
        .content_height(520)
        .child(&view)
        .build();
    dialog.present(Some(window));
}
