// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Ask any model for a formula** — `grind sheet prompt` and `grind sheet answer` with a dialog
//! in front of them (`doc/formula-assistant.md` §11).
//!
//! One dialog, top to bottom in the order a person works: say what the formula should do, copy a
//! prompt (shown as well, so it can be read before it is pasted anywhere), paste the model's
//! answer, check it — every target cell's value, worked out on a copy — and apply it. Apply is
//! only offered for the answer that was checked; editing the answer takes the offer back.
//!
//! Everything the dialog says is `grind_assist`'s: the prompt, the reading of either syntax, the
//! values and the format. This file holds widgets and one pure function ([`rows`]).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use grind_assist::Spelling;
use grind_assist::copy::{self, Tried};
use grind_sheet::{App, Pos, a1};
use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::glib;

/// What a checked answer says, a line each: how it was read, then every target cell and its
/// value — the same lines `grind sheet answer` prints, tab for a gap.
pub fn rows(tried: &Tried) -> Vec<String> {
    let read = match tried.spelling {
        Spelling::Display => "Read as written",
        Spelling::Excel => "Read as Excel's syntax",
    };
    let mut lines = vec![format!("{read}: {}", tried.formula)];
    lines.extend(tried.values.iter().map(|(pos, value)| {
        let shown = match value.is_empty() {
            true => "(empty)",
            false => value.as_str(),
        };
        format!("{}\t{shown}", a1::format(None, *pos))
    }));
    lines
}

/// The dialog, for a formula over the selection `start..=end` on `sheet`.
pub fn present(
    window: &adw::ApplicationWindow,
    app: &Arc<App>,
    sheet: usize,
    start: Pos,
    end: Pos,
) {
    let target = match start == end {
        true => a1::format(None, start),
        false => format!("{}:{}", a1::format(None, start), a1::format(None, end)),
    };
    let sheet_name = app.sheet_name(sheet).unwrap_or_default();
    let dialog = adw::Dialog::builder()
        .title("Ask Any Model for a Formula")
        .content_width(620)
        .content_height(680)
        .build();

    let heading = |text: &str| {
        let label = gtk::Label::builder().label(text).xalign(0.0).build();
        label.add_css_class("heading");
        label
    };
    let note = |text: &str| {
        let label = gtk::Label::builder()
            .label(text)
            .xalign(0.0)
            .wrap(true)
            .build();
        label.add_css_class("dim-label");
        label
    };
    let framed = |child: &gtk::Widget, height: i32| {
        let scroller = gtk::ScrolledWindow::builder()
            .min_content_height(height)
            .child(child)
            .build();
        scroller.add_css_class("card");
        scroller
    };

    // 1. What it should do.
    let request = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    let private = gtk::CheckButton::builder()
        .label("Leave the document's values out — headers and types only")
        .build();
    let copy_button = gtk::Button::builder().label("Copy Prompt").build();
    copy_button.add_css_class("suggested-action");
    let copied = note("");
    let prompt_view = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    let prompt_box = framed(prompt_view.upcast_ref(), 160);
    prompt_box.set_visible(false);

    // 2. What the model answered.
    let answer = gtk::Entry::builder()
        .placeholder_text("Paste the model's formula here, e.g. =SUMIF(B2:B40,\"Rent\",D2:D40)")
        .build();
    let check_button = gtk::Button::builder().label("Check").build();
    let result_view = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    let result_box = framed(result_view.upcast_ref(), 140);
    result_box.set_visible(false);
    let apply_button = gtk::Button::builder()
        .label(format!("Apply to {target}"))
        .sensitive(false)
        .build();
    apply_button.add_css_class("suggested-action");

    // The answer that was checked, which is the only one Apply may write.
    let checked: Rc<RefCell<Option<Tried>>> = Rc::new(RefCell::new(None));

    copy_button.connect_clicked(glib::clone!(
        #[strong]
        app,
        #[weak]
        request,
        #[weak]
        private,
        #[weak]
        copied,
        #[weak]
        prompt_view,
        #[weak]
        prompt_box,
        move |button| {
            let buffer = request.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            if text.trim().is_empty() {
                copied.set_label("Say first what the formula should do.");
                return;
            }
            match copy::copy_prompt(&app, sheet, start, end, &text, !private.is_active()) {
                Ok(prompt) => {
                    button.clipboard().set_text(&prompt);
                    prompt_view.buffer().set_text(&prompt);
                    prompt_box.set_visible(true);
                    copied.set_label(
                        "Copied. Paste it into any language model you choose — this is \
                         everything it will see of the document — and paste its answer below.",
                    );
                }
                Err(e) => copied.set_label(&e),
            }
        }
    ));

    check_button.connect_clicked(glib::clone!(
        #[strong]
        app,
        #[strong]
        checked,
        #[weak]
        answer,
        #[weak]
        result_view,
        #[weak]
        result_box,
        #[weak]
        apply_button,
        move |_| {
            let text = answer.text();
            let lines = match copy::try_answer(&app, sheet, start, end, &text) {
                Ok(tried) => {
                    let lines = rows(&tried);
                    *checked.borrow_mut() = Some(tried);
                    apply_button.set_sensitive(true);
                    lines
                }
                Err(e) => {
                    *checked.borrow_mut() = None;
                    apply_button.set_sensitive(false);
                    vec![e]
                }
            };
            result_view.buffer().set_text(&lines.join("\n"));
            result_box.set_visible(true);
        }
    ));
    answer.connect_activate(glib::clone!(
        #[weak]
        check_button,
        move |_| check_button.emit_clicked()
    ));
    // A changed answer is an unchecked one.
    answer.connect_changed(glib::clone!(
        #[strong]
        checked,
        #[weak]
        apply_button,
        move |_| {
            *checked.borrow_mut() = None;
            apply_button.set_sensitive(false);
        }
    ));

    apply_button.connect_clicked(glib::clone!(
        #[strong]
        app,
        #[strong]
        checked,
        #[weak]
        dialog,
        #[weak]
        result_view,
        move |_| {
            let Some(tried) = checked.borrow().clone() else {
                return;
            };
            match copy::apply_answer(&app, sheet, start, end, &tried) {
                Ok(()) => {
                    dialog.close();
                }
                Err(e) => result_view.buffer().set_text(&e),
            }
        }
    ));

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&note(&format!(
        "A formula for {target} on {sheet_name}{}. Grind sends nothing anywhere: you copy the \
         prompt into the model you choose, and grind checks what comes back before anything is \
         written.",
        match start == end {
            true => "",
            false => ", filled down the selection",
        }
    )));
    content.append(&heading("What should it do?"));
    content.append(&framed(request.upcast_ref(), 70));
    content.append(&private);
    let copy_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    copy_row.append(&copy_button);
    copied.set_hexpand(true);
    copy_row.append(&copied);
    content.append(&copy_row);
    content.append(&prompt_box);
    content.append(&heading("The model's answer"));
    let answer_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    answer.set_hexpand(true);
    answer_row.append(&answer);
    answer_row.append(&check_button);
    content.append(&answer_row);
    content.append(&result_box);
    let apply_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    apply_row.set_halign(gtk::Align::End);
    apply_row.append(&apply_button);
    content.append(&apply_row);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&content)
        .build();
    let view = adw::ToolbarView::builder().content(&scroller).build();
    view.add_top_bar(&adw::HeaderBar::new());
    dialog.set_child(Some(&view));
    dialog.present(Some(window));
    request.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::RecalcMode;

    /// No display needed: the lines are the part that says something.
    #[test]
    fn a_checked_answer_reads_as_how_it_was_read_then_every_cell() {
        let app = App::new();
        let rows_in: Vec<Vec<String>> = vec![vec!["2".into()], vec!["".into()]];
        app.enter_range(0, Pos { row: 0, col: 0 }, &rows_in, RecalcMode::No)
            .unwrap();
        let (b1, b2) = (Pos { row: 0, col: 1 }, Pos { row: 1, col: 1 });
        let tried = copy::try_answer(&app, 0, b1, b2, "=IF(A1>0,A1*2,\"\")").unwrap();
        assert_eq!(
            rows(&tried),
            [
                "Read as Excel's syntax: =IF(A1>0;A1*2;\"\")",
                "B1\t4",
                "B2\t(empty)"
            ]
        );
    }
}
