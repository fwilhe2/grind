// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Everything around the grid: the formula bar, the sheet tabs and the status bar.
//!
//! None of it owns anything either. Each piece reads what it needs from [`App`] when it is
//! asked to refresh, and writes through the grid or through `App` — the same rule the grid
//! follows, applied to the parts that are made of ordinary widgets.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::glib;

use sheet_core::{App, CellValue, Pos, a1};

use crate::grid::Grid;
use crate::keymap::{Dir, Selection};

/// The formula bar: a name box, the shared editor, and the two buttons that end an edit.
///
/// The entry is built over the grid's own `EntryBuffer`, which is the whole trick — content
/// stays in step with the in-cell editor for free, and each keeps its own caret.
pub fn formula_bar(grid: &Grid, app: &Arc<App>) -> gtk::Box {
    let name_box = gtk::Entry::builder()
        .width_chars(10)
        .max_width_chars(14)
        .tooltip_text("Go to a cell, a range or a defined name")
        .build();
    name_box.connect_activate(glib::clone!(
        #[weak]
        grid,
        #[strong]
        app,
        move |entry| {
            if let Some(selection) = locate(&app, grid.sheet(), &entry.text()) {
                grid.set_selection(selection);
            }
            entry.set_text("");
            grid.grab_focus();
        }
    ));

    let entry = gtk::Entry::builder()
        .buffer(&grid.buffer())
        .hexpand(true)
        .placeholder_text("Value or formula")
        .build();
    // Typing in the formula bar edits the cell, so the session has to exist before the
    // first character lands — but the focus stays here rather than jumping to the cell.
    entry.connect_has_focus_notify(glib::clone!(
        #[weak]
        grid,
        move |entry| {
            if entry.has_focus() && !grid.is_editing() {
                grid.begin_edit(false);
            }
        }
    ));
    entry.connect_activate(glib::clone!(
        #[weak]
        grid,
        move |_| grid.commit(Some(Dir::Down))
    ));
    // The references, coloured — the same function the in-cell editor uses, over the same
    // buffer, so the two copies of the formula cannot be coloured differently.
    let colour = |entry: &gtk::Entry| {
        let dark = crate::theme::is_dark(&crate::theme::Palette::of(entry));
        let text = entry.text().to_string();
        entry.set_attributes(&crate::theme::reference_attributes(&text, dark));
    };
    colour(&entry);
    grid.buffer().connect_text_notify(glib::clone!(
        #[weak]
        entry,
        move |_| colour(&entry)
    ));

    let accept = icon_button("object-select-symbolic", "Accept");
    accept.connect_clicked(glib::clone!(
        #[weak]
        grid,
        move |_| grid.commit(None)
    ));
    let reject = icon_button("edit-undo-symbolic", "Cancel");
    reject.connect_clicked(glib::clone!(
        #[weak]
        grid,
        move |_| grid.cancel_edit()
    ));

    // The two labels at the end: what the function being typed takes, and what the formula
    // would say if it were committed now.
    let hint = gtk::Label::builder()
        .use_markup(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(44)
        .xalign(1.0)
        .build();
    hint.add_css_class("dim-label");
    let chip = gtk::Label::new(None);
    chip.add_css_class("dim-label");
    chip.add_css_class("numeric");

    let bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_start(6)
        .margin_end(6)
        .margin_top(3)
        .margin_bottom(3)
        .build();
    for widget in [
        name_box.upcast_ref::<gtk::Widget>(),
        reject.upcast_ref(),
        accept.upcast_ref(),
        entry.upcast_ref(),
        hint.upcast_ref(),
        chip.upcast_ref(),
    ] {
        bar.append(widget);
    }

    // Both are recomputed on every change to the shared buffer — the preview after a beat,
    // because it evaluates and a keystroke is not the moment to do that.
    let pending: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let update = glib::clone!(
        #[weak]
        grid,
        #[weak]
        entry,
        #[weak]
        hint,
        #[weak]
        chip,
        #[strong]
        app,
        #[strong]
        pending,
        move || {
            let text = grid.buffer().text().to_string();
            // The caret is the focused editable's; when the formula bar does not have it,
            // the end of the text is where the typing is.
            let caret = match entry.has_focus() {
                true => byte_offset(&text, entry.position()),
                false => grid.caret(),
            };
            match crate::state::call_at(&text, caret)
                .and_then(|(name, argument)| crate::formula_ux::signature_markup(&name, argument))
            {
                Some(markup) => {
                    hint.set_markup(&markup);
                    hint.set_visible(true);
                }
                None => hint.set_visible(false),
            }

            if let Some(source) = pending.take() {
                source.remove();
            }
            if !grid.is_editing() || !text.starts_with('=') {
                chip.set_visible(false);
                return;
            }
            let (app, chip, grid, pending2) = (app.clone(), chip.clone(), grid.clone(), pending.clone());
            pending.set(Some(glib::timeout_add_local_once(
                std::time::Duration::from_millis(150),
                move || {
                    pending2.set(None);
                    let text = grid.buffer().text().to_string();
                    // Errors included, which is half the value: a formula that will not
                    // parse says so before it is committed rather than after.
                    let preview = match sheet_core::formula::display::from_display(&text) {
                        Ok(canonical) => app
                            .preview(grid.sheet(), grid.selection().active, &canonical)
                            .map(|value| show_value(&value))
                            .unwrap_or_else(|error| error.to_string()),
                        Err(error) => error.message,
                    };
                    chip.set_text(&format!("= {preview}"));
                    chip.set_visible(true);
                },
            )));
        }
    );
    update();
    grid.buffer().connect_text_notify(glib::clone!(
        #[strong]
        update,
        move |_| update()
    ));
    grid.connect_editing_changed(glib::clone!(
        #[strong]
        update,
        move |_| update()
    ));
    grid.connect_caret_moved(glib::clone!(
        #[strong]
        update,
        move || update()
    ));
    // And the formula bar's own caret, for when it is the one being typed in.
    entry.connect_cursor_position_notify(glib::clone!(
        #[strong]
        update,
        move |_| update()
    ));

    // The two buttons are only meaningful while an edit is open; the rest of the time they
    // would be two things to wonder about.
    let buttons = move |editing: bool| {
        accept.set_visible(editing);
        reject.set_visible(editing);
    };
    buttons(false);
    grid.connect_editing_changed(buttons);
    bar
}

/// A `GtkEditable` position, which counts characters, as a byte offset — which is what
/// every scanner here counts in.
fn byte_offset(text: &str, position: i32) -> usize {
    text.char_indices()
        .nth(position.max(0) as usize)
        .map_or(text.len(), |(byte, _)| byte)
}

/// A previewed value, spelled the way the cell would spell it with no format.
fn show_value(value: &CellValue) -> String {
    match value {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => show(*n),
        CellValue::Bool(true) => "TRUE".to_owned(),
        CellValue::Bool(false) => "FALSE".to_owned(),
        CellValue::Text(text) => text.clone(),
    }
}

/// An address or a defined name, as the name box takes it.
///
/// Resolved through `core::a1`, so what the name box means by `Data.B2:C9` is what a
/// formula means by it — there is no second address parser anywhere in the workspace.
fn locate(app: &App, sheet: usize, text: &str) -> Option<Selection> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // A defined name first: it is document-level, and a name that is also an address is
    // refused when it is defined, so this cannot shadow a cell.
    let expression = app
        .names()
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
        .map(|(_, expression)| expression);
    let reference = match &expression {
        Some(expression) => a1::parse(expression.trim_start_matches('[').trim_end_matches(']')).ok()?,
        None => a1::parse(text).ok()?,
    };
    let (found, start, end) = a1::resolve(app, &reference).ok()?;
    // Navigating to another sheet is the sheet tabs' job, not the name box's, so a name
    // that lives elsewhere is refused rather than silently landing on the wrong sheet.
    (found == sheet).then_some(Selection {
        anchor: start,
        active: end,
    })
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button
}

/// The sheet tab strip: one toggle button per sheet, and a `+`.
///
/// Plain buttons rather than `adw::TabBar`, which is for documents in a window and brings
/// close buttons and drag-to-detach with it — the wrong tool with the right name.
pub struct Tabs {
    pub widget: gtk::Box,
    strip: gtk::Box,
    app: Arc<App>,
    grid: Grid,
    /// Set while the strip is being rebuilt, so that toggling a button programmatically
    /// does not read as the user picking a sheet.
    rebuilding: Rc<Cell<bool>>,
}

impl Tabs {
    pub fn new(grid: &Grid, app: &Arc<App>, add: &gtk::Button) -> Rc<Self> {
        let strip = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(0)
            .build();
        strip.add_css_class("linked");

        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_start(6)
            .margin_end(6)
            .margin_top(3)
            .margin_bottom(3)
            .build();
        widget.append(&strip);
        widget.append(add);

        let tabs = Rc::new(Self {
            widget,
            strip,
            app: app.clone(),
            grid: grid.clone(),
            rebuilding: Rc::new(Cell::new(false)),
        });
        tabs.refresh();
        tabs
    }

    /// Rebuild the strip from the document. Called on every change rather than on the ones
    /// that touch sheets, because "which changes affect the tabs" is a list that goes stale.
    pub fn refresh(self: &Rc<Self>) {
        self.rebuilding.set(true);
        while let Some(child) = self.strip.first_child() {
            self.strip.remove(&child);
        }
        let current = self.grid.sheet();
        let mut first: Option<gtk::ToggleButton> = None;
        for index in 0..self.app.sheet_count() {
            let name = self.app.sheet_name(index).unwrap_or_default();
            let button = gtk::ToggleButton::builder()
                .label(&name)
                .active(index == current)
                .build();
            button.add_css_class("flat");
            // One group, so picking a sheet unpicks the last one without any bookkeeping.
            match &first {
                Some(first) => button.set_group(Some(first)),
                None => first = Some(button.clone()),
            }
            button.connect_toggled(glib::clone!(
                #[strong(rename_to = tabs)]
                self,
                move |button| {
                    if button.is_active() && !tabs.rebuilding.get() {
                        tabs.grid.set_sheet(index);
                    }
                }
            ));
            self.strip.append(&button);
        }
        self.rebuilding.set(false);
    }
}

/// The status bar: where the selection is, and what it adds up to.
///
/// The aggregates are `App::preview` over generated formulas rather than a second summing
/// loop here — `SUM`, `COUNTA` (a status bar's Count is non-empty, not numeric) and
/// `AVERAGE` — so what the bar says and what a cell would say cannot differ.
///
/// Debounced, because a drag changes the selection on every motion event and each change
/// costs a walk of the range. ponytail: the walk runs on the main thread, so a selection
/// spanning a very large used extent stutters the drag; the plan's answer is a worker and a
/// generation counter, which is the threading milestone's machinery rather than this one's.
pub fn status_bar(grid: &Grid, app: &Arc<App>) -> gtk::Box {
    let label = gtk::Label::builder()
        .xalign(0.0)
        .margin_start(10)
        .margin_end(10)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    // libadwaita paints a toolbar; without the class the bar is transparent and the grid
    // scrolls underneath the text.
    bar.add_css_class("toolbar");
    bar.append(&label);

    let pending: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    let app = app.clone();
    let grid = grid.clone();
    grid.connect_selection_changed(glib::clone!(
        #[weak(rename_to = grid)]
        grid,
        move |selection| {
            if let Some(source) = pending.take() {
                source.remove();
            }
            let (app, label, pending) = (app.clone(), label.clone(), pending.clone());
            pending.clone().set(Some(glib::timeout_add_local_once(
                std::time::Duration::from_millis(100),
                move || {
                    pending.set(None);
                    label.set_text(&status_text(&app, grid.sheet(), selection));
                },
            )));
        }
    ));
    bar
}

/// `B2:C4  ·  Sum 21215.51  ·  Count 6  ·  Average 3535.9`, or just the address when the
/// selection holds nothing.
fn status_text(app: &App, sheet: usize, selection: Selection) -> String {
    let (start, end) = selection.rect();
    if selection.is_single() {
        // One cell has nothing to add up, and every other spreadsheet stays quiet about it.
        return a1::format(None, start);
    }
    let address = format!("{}:{}", a1::format(None, start), a1::format(None, end));
    // Clamped to the used extent first: a whole-column selection must not ask the evaluator
    // to walk a million empty rows.
    let Ok((rows, cols)) = app.used_extent(sheet) else {
        return address;
    };
    let end = Pos::new(
        end.row.min(rows.saturating_sub(1)),
        end.col.min(cols.saturating_sub(1)),
    );
    if rows == 0 || cols == 0 || end.row < start.row || end.col < start.col {
        return address;
    }

    let range = format!("[.{}:.{}]", a1::format(None, start), a1::format(None, end));
    // Evaluated at a cell one past the used extent: a formula is evaluated *as if* it sat
    // somewhere, and somewhere inside the range would be a circular reference.
    let at = Pos::new(rows, 0);
    let of = |formula: String| match app.preview(sheet, at, &formula) {
        Ok(CellValue::Number(n)) => Some(n),
        _ => None,
    };
    let count = of(format!("=COUNTA({range})")).unwrap_or(0.0);
    if count == 0.0 {
        return address;
    }
    let mut parts = vec![address, format!("Count {}", show(count))];
    // Sum and Average of no numbers are not zero, they are nothing — AVERAGE says so with
    // #DIV/0!, which is why both are read back as an optional number.
    if let Some(sum) = of(format!("=SUM({range})"))
        && let Some(average) = of(format!("=AVERAGE({range})"))
    {
        parts.insert(1, format!("Sum {}", show(sum)));
        parts.push(format!("Average {}", show(average)));
    }
    parts.join("  ·  ")
}

fn show(n: f64) -> String {
    sheet_core::formula::value::format_number(n)
}
