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

use grind_sheet::formula::friendly;
use grind_sheet::{App, CellValue, Pos, a1};

use crate::grid::{Grid, Notice};
use crate::keymap::{Dir, Selection};

/// The formula bar, and the one thing outside it may do: turn friendly mode on or off.
///
/// Same shape as [`crate::formatting::Strip`], and for the same reason — the window owns the
/// action, the bar owns the widgets.
pub struct FormulaBar {
    pub widget: gtk::Box,
    /// The ⓘ button, kept so the window's `win.explain-formula` action opens the same
    /// popover a click on it does.
    pub explain: gtk::MenuButton,
    friendly: Rc<Cell<bool>>,
    refresh: Box<dyn Fn()>,
}

impl FormulaBar {
    pub fn friendly(&self) -> bool {
        self.friendly.get()
    }

    pub fn set_friendly(&self, friendly: bool) {
        self.friendly.set(friendly);
        (self.refresh)();
    }
}

/// The formula bar: a name box, the shared editor, and the two buttons that end an edit.
///
/// The entry is built over the grid's own `EntryBuffer`, which is the whole trick — content
/// stays in step with the in-cell editor for free, and each keeps its own caret.
///
/// **Friendly mode is a second *view* of that buffer, never a second copy of it.** A cell
/// holding a formula is shown, while nothing is being edited, as
/// [`friendly::explain_inline`] spells it — a `gtk::Button` in a `gtk::Stack`, in front of
/// the entry rather than instead of it. Clicking it (or any of the ordinary ways an edit
/// starts) swaps the entry back in, holding the *stored* ODF formula, which is the only
/// spelling anything ever writes. Nothing here parses friendly text back: `doc/plan.md` R1
/// says the document's formula is ODF's, and an editable friendly syntax would need a
/// parameter-label spelling that cannot be confused with `=` as a comparison operator.
pub fn formula_bar(grid: &Grid, app: &Arc<App>, friendly: bool) -> Rc<FormulaBar> {
    let name_box = gtk::Entry::builder()
        .width_chars(10)
        .max_width_chars(14)
        .tooltip_text("Go to a cell, a range or a defined name — or name the selection")
        .build();
    name_box.connect_activate(glib::clone!(
        #[weak]
        grid,
        #[strong]
        app,
        move |entry| {
            let text = entry.text();
            match locate(&app, grid.sheet(), &text) {
                Some(selection) => grid.set_selection(selection),
                // Not somewhere to go, so it is something to name — what every other
                // spreadsheet's name box does with a word it does not know.
                None => {
                    if let Err(error) = name_selection(&app, &grid, text.trim()) {
                        grid.report(Notice::Refused(error));
                        // The text stays put so it can be corrected rather than retyped.
                        return;
                    }
                }
            }
            entry.set_text(&name_box_text(&app, grid.sheet(), grid.selection()));
            grid.grab_focus();
        }
    ));
    // The name box shows where the selection is — or, when the selection is exactly a
    // defined range, what it is called. That is the only way a name is visible without
    // opening a dialog, and it is how a user finds out one exists at all.
    name_box.set_text(&name_box_text(app, grid.sheet(), grid.selection()));
    grid.connect_selection_changed(glib::clone!(
        #[weak]
        name_box,
        #[weak]
        grid,
        #[strong]
        app,
        move |selection| name_box.set_text(&name_box_text(&app, grid.sheet(), selection))
    ));

    let entry = gtk::Entry::builder()
        .buffer(&grid.buffer())
        .hexpand(true)
        .placeholder_text("Value or formula")
        .build();
    // Typing in the formula bar edits the cell, so the session has to exist before the
    // first character lands — but the focus stays here rather than jumping to the cell.
    entry.connect_state_flags_changed(glib::clone!(
        #[weak]
        grid,
        move |entry, _| {
            if !typing_here(entry) {
                return;
            }
            if !grid.is_editing() {
                grid.begin_edit(false);
            }
            grid.set_caret(entry.position());
        }
    ));
    entry.connect_activate(glib::clone!(
        #[weak]
        grid,
        move |_| grid.commit(Some(Dir::Down))
    ));
    // The references, coloured — the same function the in-cell editor uses, over the same
    // buffer, so the two copies of the formula cannot be coloured differently.
    colour_references(&entry);
    grid.buffer().connect_text_notify(glib::clone!(
        #[weak]
        entry,
        move |_| colour_references(&entry)
    ));
    // A theme flip swaps which half of the reference palette reads, and fires no buffer
    // signal — the same recolouring the grid does for itself in `restyle`.
    libadwaita::StyleManager::default().connect_dark_notify(glib::clone!(
        #[weak]
        entry,
        move |_| colour_references(&entry)
    ));

    // The friendly view, and the stack that puts it in front of the entry. A button rather
    // than a label so it is focusable and activatable — the friendly view is where the
    // keyboard lands when nothing is being edited, and Space on it starts the edit.
    let friendly = Rc::new(Cell::new(friendly));
    let friendly_label = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let friendly_view = gtk::Button::builder()
        .child(&friendly_label)
        .has_frame(false)
        .hexpand(true)
        .tooltip_text("Click to edit the formula this build stores")
        .build();
    friendly_view.connect_clicked(glib::clone!(
        #[weak]
        grid,
        move |_| grid.begin_edit(false)
    ));
    // `doc/view-modes.md` §3.3's half of inline names, and the same stand-in shape for the
    // same reason: what it shows is a **reading**, so it must not be what a commit takes
    // back in. `=tax_rate*subtotal` typed into the entry would store the names; shown here
    // it is a label, and clicking it swaps the entry — holding the formula the file has —
    // back in front.
    let named_label = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let named_view = gtk::Button::builder()
        .child(&named_label)
        .has_frame(false)
        .hexpand(true)
        .tooltip_text("Click to edit the formula this build stores")
        .build();
    named_view.connect_clicked(glib::clone!(
        #[weak]
        grid,
        move |_| grid.begin_edit(false)
    ));
    let stack = gtk::Stack::builder().hexpand(true).build();
    stack.add_named(&entry, Some("raw"));
    stack.add_named(&friendly_view, Some("friendly"));
    stack.add_named(&named_view, Some("named"));

    // The multi-line rendering of the same formula, for when one line is not enough. Built
    // on every popup, so there is nothing to keep in step while it is closed.
    let explain_label = gtk::Label::builder().selectable(true).xalign(0.0).build();
    explain_label.add_css_class("monospace");
    let explain = gtk::MenuButton::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text("Explain Formula")
        .build();
    explain.add_css_class("flat");
    explain.set_popover(Some(
        &gtk::Popover::builder()
            .child(
                &gtk::ScrolledWindow::builder()
                    .child(&explain_label)
                    .propagate_natural_height(true)
                    .propagate_natural_width(true)
                    .max_content_height(400)
                    .max_content_width(560)
                    .margin_top(6)
                    .margin_bottom(6)
                    .margin_start(6)
                    .margin_end(6)
                    .build(),
            )
            .build(),
    ));
    explain.set_create_popup_func(glib::clone!(
        #[weak]
        grid,
        #[strong]
        app,
        move |_| explain_label.set_label(&explanation(&app, &grid))
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
        stack.upcast_ref(),
        explain.upcast_ref(),
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
        #[weak]
        stack,
        #[weak]
        friendly_label,
        #[weak]
        named_label,
        #[weak]
        explain,
        #[strong]
        app,
        #[strong]
        friendly,
        #[strong]
        pending,
        move || {
            let text = grid.buffer().text().to_string();

            // The friendly view stands in for the entry only when there is a formula to
            // show it for and nobody is editing — a value is already spelled the way a
            // person would spell it, and mid-edit the stored formula is the thing being
            // worked on.
            //
            let explained = (!grid.is_editing()).then(|| friendly_line(&text)).flatten();
            explain.set_visible(explained.is_some());
            // The name reading, when the overlay is on and the formula actually mentions a
            // named place — a reading identical to what the entry already shows is not a
            // reading worth swapping the entry out for.
            let named = (!grid.is_editing() && grid.overlays().names)
                .then(|| {
                    app.named_formula(grid.sheet(), grid.selection().active)
                        .ok()
                        .flatten()
                })
                .flatten()
                .filter(|reading| *reading != text);
            // Friendly wins where both are on: it is the more thorough reading of the two,
            // and two readings at once is one too many.
            match (explained.filter(|_| friendly.get()), named) {
                (Some(explained), _) => {
                    friendly_label.set_label(&explained);
                    stack.set_visible_child_name("friendly");
                }
                (None, Some(named)) => {
                    named_label.set_label(&named);
                    stack.set_visible_child_name("named");
                }
                _ => stack.set_visible_child_name("raw"),
            }

            // The caret is the focused editable's; when the formula bar does not have it,
            // the end of the text is where the typing is.
            let caret = match typing_here(&entry) {
                true => byte_offset(&text, entry.position()),
                false => grid.caret(),
            };
            match crate::state::call_at(&text, caret).and_then(|(name, argument)| {
                crate::formula_ux::signature_markup(&name, argument, friendly.get())
            }) {
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
            let (app, chip, grid, pending2) =
                (app.clone(), chip.clone(), grid.clone(), pending.clone());
            pending.set(Some(glib::timeout_add_local_once(
                std::time::Duration::from_millis(150),
                move || {
                    pending2.set(None);
                    let text = grid.buffer().text().to_string();
                    // Errors included, which is half the value: a formula that will not
                    // parse says so before it is committed rather than after.
                    let preview = match grind_sheet::formula::display::from_display(&text) {
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
    // And the formula bar's own caret, for when it is the one being typed in — mirrored into
    // the in-cell editor too, since point mode judges a click against *its* caret.
    entry.connect_cursor_position_notify(glib::clone!(
        #[weak]
        grid,
        #[strong]
        update,
        move |entry| {
            if typing_here(entry) {
                grid.set_caret(entry.position());
            }
            update();
        }
    ));

    // A cell can change under an unmoved selection (undo, redo, a load), and two cells can
    // hold the same text — neither fires the buffer's own signal, and both change what the
    // friendly view should say.
    grid.connect_selection_changed(glib::clone!(
        #[strong]
        update,
        move |_| update()
    ));
    // Swapping the entry in is only half of starting an edit from the friendly view: it has
    // to take the focus too, or the first keystroke goes to the button that is no longer
    // there.
    stack.connect_visible_child_name_notify(glib::clone!(
        #[weak]
        entry,
        move |stack| {
            if stack.visible_child_name().as_deref() == Some("raw") && typing_here(stack) {
                entry.grab_focus();
            }
        }
    ));
    friendly_view.connect_clicked(glib::clone!(
        #[weak]
        entry,
        move |_| {
            entry.grab_focus();
        }
    ));

    // The two buttons are only meaningful while an edit is open; the rest of the time they
    // would be two things to wonder about.
    let buttons = move |editing: bool| {
        accept.set_visible(editing);
        reject.set_visible(editing);
    };
    buttons(false);
    grid.connect_editing_changed(buttons);

    Rc::new(FormulaBar {
        widget: bar,
        explain,
        friendly,
        refresh: Box::new(update),
    })
}

/// Whether the keyboard is in this entry — the question the formula bar asks three times:
/// to open an edit when it is clicked into, to mirror its caret into the in-cell editor, and
/// to decide whose caret the signature hint should read.
///
/// **Not `has_focus`.** A `gtk::Entry` is a wrapper around an internal `gtk::Text`, and that
/// child is what actually takes the focus — so `has-focus` on the entry is never true, its
/// `notify` never fires, and all three questions answered "no" for as long as they were
/// asked that way. `FOCUS_WITHIN` is the flag GTK sets on the ancestors of the focus widget,
/// which is what "the keyboard is in there" means for a composite widget.
fn typing_here(widget: &impl IsA<gtk::Widget>) -> bool {
    widget
        .as_ref()
        .state_flags()
        .contains(gtk::StateFlags::FOCUS_WITHIN)
}

/// The one-line friendly rendering of what the shared buffer holds, or `None` when there is
/// nothing to render one for.
///
/// **The buffer holds *display* form and [`friendly`] reads the stored form**, so this
/// converts before it explains — the same step the preview chip takes. Skipping it does not
/// merely fail: an unqualified `A1` parses the same in both spellings, but `$Sheet1.$B$2`
/// lexes as a quoted *name* rather than a reference, so a sheet-qualified formula came out as
/// `$$'Sheet1.B2':B4` instead of erroring and falling back to the entry.
pub(crate) fn friendly_line(text: &str) -> Option<String> {
    if !text.starts_with('=') {
        return None;
    }
    let canonical = grind_sheet::formula::display::from_display(text).ok()?;
    friendly::explain_inline(&canonical).ok()
}

/// What the ⓘ popover says: the active cell's formula explained over as many lines as it
/// takes, or why there is nothing to explain. A sentence rather than an empty popover,
/// because a popover that opens empty reads as a bug.
fn explanation(app: &App, grid: &Grid) -> String {
    match app.formula(grid.sheet(), grid.selection().active) {
        Ok(Some(formula)) => match friendly::explain(&formula) {
            Ok(explained) => explained,
            Err(error) => error.to_string(),
        },
        Ok(None) => "This cell has no formula.".to_owned(),
        Err(error) => error.to_string(),
    }
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
        Some(expression) => a1::parse(strip_brackets(expression)).ok()?,
        // A bare word is a *whole column* to the grammar — `foo` is `[.FOO]`, column 4460 —
        // and taking that literally means no three-letter word can ever be a name, since
        // every one of them is a column up to `XFD`. A name box wants the other reading, so
        // an address without a `:` has to name both axes: `A1` and `Data.B2` are places,
        // `A:A` and `3:3` are the whole column and the whole row, and `foo` is a name.
        None => match a1::parse(text).ok()? {
            reference if text.contains(':') || is_a_cell(&reference) => reference,
            _ => return None,
        },
    };
    let (found, start, end) = a1::resolve(app, &reference).ok()?;
    // Navigating to another sheet is the sheet tabs' job, not the name box's, so a name
    // that lives elsewhere is refused rather than silently landing on the wrong sheet.
    //
    // The active cell is the range's *start*: going to a range means looking at the top of
    // it, and the active cell is what the grid scrolls to.
    (found == sheet).then_some(Selection {
        anchor: end,
        active: start,
    })
}

/// What the name box shows for a selection: what it is called, or where it is.
pub fn name_box_text(app: &App, sheet: usize, selection: Selection) -> String {
    name_of(app, sheet, selection).unwrap_or_else(|| a1::format(None, selection.active))
}

/// The defined name covering exactly this selection, if there is one.
///
/// Exactly, not overlapping: a name is a handle on one range, and offering it for a
/// selection that merely sits inside would put a word in the box that typing back would
/// move the selection.
fn name_of(app: &App, sheet: usize, selection: Selection) -> Option<String> {
    let want = selection.rect();
    app.names().into_iter().find_map(|(name, expression)| {
        let reference = a1::parse(strip_brackets(&expression)).ok()?;
        let (found, start, end) = a1::resolve(app, &reference).ok()?;
        (found == sheet && (start, end) == want).then_some(name)
    })
}

/// A stored definition without ODF's brackets — `[$Sheet1.$A$1]` is how a reference is
/// written in a file, and `a1::parse` takes the address a person types.
fn strip_brackets(expression: &str) -> &str {
    expression
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(expression)
}

/// Whether a reference names a cell rather than a whole column or row — both axes present
/// on both ends.
fn is_a_cell(reference: &grind_sheet::formula::lex::Reference) -> bool {
    std::iter::once(&reference.start)
        .chain(reference.end.as_ref())
        .all(|end| end.row.is_some() && end.col.is_some())
}

/// Define `name` over whatever is selected — the other half of the name box.
///
/// Sheet-qualified and absolute, through `a1::as_definition`, because that is what a name
/// has to be to mean the same range read from anywhere (§5.11). The core does the refusing:
/// `App::set_name` already knows which words are names and which are addresses wearing a
/// name's clothes, and a second opinion here would be a second thing to keep in step.
fn name_selection(app: &App, grid: &Grid, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Type a cell, a range, or a name for the selection".to_owned());
    }
    let sheet = grid.sheet();
    let (start, end) = grid.selection().rect();
    let sheet_name = app.sheet_name(sheet).map_err(|e| e.to_string())?;
    let reference = a1::reference(Some(&sheet_name), start, end);
    let expression = a1::as_definition(app, &reference).map_err(|e| e.to_string())?;
    app.set_name(name, &expression).map_err(|e| e.to_string())
}

/// Colour the references in the formula bar's entry — the same spans and palette as the
/// in-cell editor, over the same buffer, so the two cannot be coloured differently.
fn colour_references(entry: &gtk::Entry) {
    let dark = crate::theme::is_dark(&crate::theme::Palette::of(entry));
    let text = entry.text().to_string();
    entry.set_attributes(&crate::theme::reference_attributes(&text, dark));
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button
}

/// The tool strip: a small mode switch — Format · Calculate · View — in front of a stack
/// of tool rows, one visible at a time.
///
/// The GNOME shape of "more tools than one row holds": plain linked toggle buttons over a
/// `gtk::Stack` (`AdwToggleGroup` is libadwaita 1.7; this shell pins 1.5), and the mode
/// never changes itself — chrome that moves under the pointer is the one ribbon behaviour
/// deliberately not copied here. Every button activates a `win.` action the window already
/// owns, so the strip adds reachability, never capability, and the parity ratchet is
/// untouched.
pub fn tools(grid: &Grid, format: &impl IsA<gtk::Widget>) -> gtk::Box {
    let stack = gtk::Stack::new();
    stack.add_named(format, Some("Format"));
    stack.add_named(&calculate_tools(), Some("Calculate"));
    stack.add_named(&view_tools(grid), Some("View"));

    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    modes.add_css_class("linked");
    let mut first: Option<gtk::ToggleButton> = None;
    for name in ["Format", "Calculate", "View"] {
        let button = gtk::ToggleButton::builder()
            .label(name)
            .active(first.is_none())
            .build();
        match &first {
            Some(first) => button.set_group(Some(first)),
            None => first = Some(button.clone()),
        }
        button.connect_toggled(glib::clone!(
            #[weak]
            stack,
            move |button| {
                if button.is_active() {
                    stack.set_visible_child_name(name);
                }
            }
        ));
        modes.append(&button);
    }

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.add_css_class("toolbar");
    bar.append(&modes);
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&stack);
    bar
}

/// The Calculate row: what was buried in the primary menu, as labelled buttons.
fn calculate_tools() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for (icon, label, action, tooltip) in [
        (
            "view-refresh-symbolic",
            "Recalculate",
            "win.recalc",
            "Recalculate Now (F9)",
        ),
        (
            "dialog-information-symbolic",
            "Explain",
            "win.explain-formula",
            "Explain Formula (Ctrl+Shift+E)",
        ),
        (
            "edit-find-symbolic",
            "Calculations",
            "win.calculations",
            "Find everything this document calculates (Ctrl+Shift+F)",
        ),
        (
            "dialog-warning-symbolic",
            "Check",
            "win.lint",
            "Check the document against `grind lint`'s rules — a stale value, a formula \
             naming a sheet that is gone, anything the projection cannot carry (F8)",
        ),
        (
            "user-bookmarks-symbolic",
            "Names",
            "win.names",
            "Name a range, or manage the names",
        ),
        // The obvious `funnel-symbolic` is not in Adwaita — a missing icon draws as a blank
        // box, so this uses the same chevron the grid draws in a filtered heading cell.
        (
            "pan-down-symbolic",
            "Filter",
            "win.filter",
            "Filter the selected rows by a column's values, or clear the filter \
             (Ctrl+Shift+L)",
        ),
        // No Fill Down / Fill Right buttons: the grid's fill handle is where a fill is
        // reached from, and Ctrl+D / Ctrl+R still work. The actions stay on the window.
        (
            "edit-copy-symbolic",
            "Copy Value",
            "win.copy-value",
            "Copy what the cells show, not the formulas behind them (Ctrl+Shift+C)",
        ),
    ] {
        row.append(&tool_button(icon, label, action, tooltip));
    }
    row
}

/// The View row: the zoom, with its level readable and clickable in the middle, autofit,
/// and the friendly-formulas toggle — whose checked state is the stateful action's own.
fn view_tools(grid: &Grid) -> gtk::Box {
    let out = gtk::Button::builder()
        .icon_name("zoom-out-symbolic")
        .action_name("win.zoom-out")
        .tooltip_text("Zoom Out")
        .build();
    let level = gtk::Button::builder()
        .label("100 %")
        .action_name("win.zoom-reset")
        .tooltip_text("Back to normal size")
        .build();
    level.add_css_class("numeric");
    let level_in = gtk::Button::builder()
        .icon_name("zoom-in-symbolic")
        .action_name("win.zoom-in")
        .tooltip_text("Zoom In")
        .build();
    grid.connect_zoom_changed(glib::clone!(
        #[weak]
        level,
        move |factor| level.set_label(&format!("{:.0} %", factor * 100.0))
    ));
    let zoom = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    zoom.add_css_class("linked");
    for button in [&out, &level, &level_in] {
        button.add_css_class("flat");
        zoom.append(button);
    }

    let friendly = gtk::ToggleButton::builder()
        .label("Friendly Formulas")
        .action_name("win.friendly-formulas")
        .tooltip_text("Read a formula as a sentence while it is not being edited")
        .build();
    friendly.add_css_class("flat");

    // `doc/view-modes.md`'s overlays, linked because they are two readings of the same
    // document and a reader turns one on to answer a question and off again. Neither
    // changes the file, which is the whole reason they can live on a toolbar rather than
    // behind a confirmation.
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    modes.add_css_class("linked");
    for (label, action, tooltip) in [
        (
            "Names",
            "win.show-names",
            "Show where each named expression lives, inside the cell it is bound to",
        ),
        (
            "Roles",
            "win.show-roles",
            "Colour every cell by what it is: an input, a computed value, a label, \
             an unnamed constant",
        ),
        // `doc/dsl.md` §6 — the third reading, and the only one that shows the *whole*
        // document rather than marking up the cells on screen.
        (
            "Source",
            "win.show-source",
            "Show the document as its projection — the same document, spelled as plain text",
        ),
    ] {
        let toggle = gtk::ToggleButton::builder()
            .label(label)
            .action_name(action)
            .tooltip_text(tooltip)
            .build();
        toggle.add_css_class("flat");
        modes.append(&toggle);
    }

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&zoom);
    row.append(&tool_button(
        "zoom-fit-best-symbolic",
        "Fit Content",
        "win.autofit-all",
        "Resize every column and row to fit what is in it",
    ));
    row.append(&friendly);
    row.append(&modes);
    row
}

/// An icon-and-label button that activates a window action — the tool rows' one shape.
fn tool_button(icon: &str, label: &str, action: &str, tooltip: &str) -> gtk::Button {
    let content = libadwaita::ButtonContent::builder()
        .icon_name(icon)
        .label(label)
        .build();
    let button = gtk::Button::builder()
        .child(&content)
        .action_name(action)
        .tooltip_text(tooltip)
        .build();
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
        .hexpand(true)
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

    // The zoom readout: invisible at 100%, because that is the resting state and a bar
    // full of resting-state numbers is noise — and a button, because the one thing to do
    // with a zoom you can see is put it back.
    let zoom = gtk::Button::builder()
        .visible(false)
        .tooltip_text("Zoom — click for normal size")
        .build();
    zoom.add_css_class("flat");
    zoom.add_css_class("numeric");
    zoom.connect_clicked(glib::clone!(
        #[weak]
        grid,
        move |_| grid.set_zoom(1.0)
    ));
    grid.connect_zoom_changed(glib::clone!(
        #[weak]
        zoom,
        move |factor| {
            zoom.set_label(&format!("{:.0} %", factor * 100.0));
            zoom.set_visible((factor - 1.0).abs() > 0.005);
        }
    ));
    bar.append(&zoom);

    // `doc/view-modes.md` §9: role mode suppresses the document's own colours, which is the
    // right call and still a surprise. It needs an indication that is *always* visible —
    // the toolbar toggles live on the View row, which is one tab of three — and this is it:
    // a button that says the mode is on and, being a button, turns it off.
    let modes = gtk::Button::builder()
        .visible(false)
        .action_name("win.show-roles")
        .tooltip_text(
            "Cell colours say what each cell is, not what the document chose — \
                       click to go back",
        )
        .label("Roles")
        .build();
    modes.add_css_class("flat");
    grid.connect_overlays_changed(glib::clone!(
        #[weak]
        modes,
        move |overlays| modes.set_visible(overlays.roles)
    ));
    bar.append(&modes);

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
    grind_sheet::formula::value::format_number(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression that made friendly mode look broken on any sheet but the first: the
    /// buffer's display form has to be converted before it is explained, or a
    /// sheet-qualified reference silently renders as a quoted name.
    #[test]
    fn the_friendly_line_reads_the_display_form_the_buffer_actually_holds() {
        assert_eq!(
            friendly_line("=COUNT($Sheet1.$B$2:$B$4)").as_deref(),
            Some("Count Numbers(Number: $Sheet1.$B$2:$B$4)")
        );
        assert_eq!(
            friendly_line("=ROUND(A1;2)").as_deref(),
            Some("Round(Value: A1, Digits: 2)")
        );
        // Not a formula, and a formula that will not parse: the entry stays.
        assert_eq!(friendly_line("12"), None);
        assert_eq!(friendly_line("=SUM("), None);
    }

    /// The name box's whole ambiguity, pinned: a word is a name, an address is a place, and
    /// a whole column has to be asked for as a range. Without the last rule every word up to
    /// `XFD` would silently be a column instead of a name.
    #[test]
    fn a_word_is_a_name_and_an_address_is_a_place() {
        let app = App::new();
        let go = |text: &str| locate(&app, 0, text);

        // Places, with both axes named.
        assert_eq!(go("A1").expect("A1 is a cell").active, Pos::new(0, 0));
        assert_eq!(go("b3:c9").expect("a range").active, Pos::new(2, 1));
        // A whole column, asked for the way a name box asks.
        assert!(go("A:A").is_some());

        // Words. Every one of these parses as a column and must not be taken as one.
        for word in ["foo", "abc", "sales", "Total"] {
            assert!(go(word).is_none(), "{word} should be a name, not a column");
        }

        // Until it is defined, and then it is where it points.
        app.set_name("foo", "[$Sheet1.$B$2:.$B$4]").expect("a name");
        assert_eq!(
            go("foo").expect("foo is defined now").active,
            Pos::new(1, 1)
        );
    }

    /// The name box shows a name when the selection is exactly that name's range, and the
    /// bare address otherwise — including when the selection only overlaps a name rather
    /// than matching it.
    #[test]
    fn the_name_box_shows_a_name_when_the_selection_is_exactly_one() {
        let app = App::new();
        app.set_name("total", "[$Sheet1.$B$2:.$B$4]")
            .expect("a name");

        let exact = Selection {
            anchor: Pos::new(1, 1),
            active: Pos::new(3, 1),
        };
        assert_eq!(name_box_text(&app, 0, exact), "total");

        let overlapping = Selection {
            anchor: Pos::new(1, 1),
            active: Pos::new(2, 1),
        };
        assert_eq!(name_box_text(&app, 0, overlapping), "B3");

        let plain = Selection::at(Pos::new(0, 0));
        assert_eq!(name_box_text(&app, 0, plain), "A1");
    }
}
