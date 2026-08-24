// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet half of the browser shell.
//!
//! The DOM is a **renderer**, not the document, the same rule the other shells follow. No
//! `contenteditable` anywhere: every visible cell is rebuilt from [`App::get_viewport`] on
//! each repaint and thrown away, so the page cannot become a second source of truth. The one
//! editable element is the formula bar's `<input>`, and what it holds is not a cell until
//! [`App::enter`] says so.
//!
//! Hit-testing is the DOM's job, not this shell's: every cell carries its address in
//! `data-row`/`data-col` and a click reads it back off the event target. The platform already
//! knows which box the pointer is in, and `layout.rs` is left with only the arithmetic the
//! platform cannot do.

pub mod keymap;
mod layout;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use grind_sheet::formula::{display, lex};
use grind_sheet::style::{CellStyle, EDGES};
use grind_sheet::{App, CellValue, Form, Pos, RecalcMode, a1};
use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Element, Event, HtmlElement, HtmlInputElement, KeyboardEvent, MouseEvent, WheelEvent,
};

use crate::{element, js, listen, request_frame};
use keymap::{Action, Chord, Dir, Motion};

/// The ODF sheet limits, and what a plain move clamps to — the same two numbers
/// `grind-tui` states, for the same reason.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

/// Hand the page the cell size the viewport arithmetic assumes.
///
/// The stylesheet declares the same two numbers so the empty page has a shape before the
/// module arrives, and this overwrites them: two declarations that must agree are two
/// declarations that will not, and when they disagreed the grid grew a column every repaint.
fn declare_cell_size(document: &Document) -> Result<(), JsValue> {
    let Some(root) = document
        .document_element()
        .and_then(|e| e.dyn_into::<HtmlElement>().ok())
    else {
        return Ok(());
    };
    let style = root.style();
    style.set_property("--cell-w", &format!("{}px", layout::CELL.cell_w))?;
    style.set_property("--cell-h", &format!("{}px", layout::CELL.cell_h))
}

/// The selected rectangle: where the selection started and where it is now.
///
/// The same two-`Pos` shape `ui_sheet_gtk/src/keymap.rs` uses, and it is two rather than a
/// rectangle so that extending with Shift knows which corner is pinned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Selection {
    anchor: Pos,
    active: Pos,
}

impl Default for Selection {
    fn default() -> Self {
        Selection::at(Pos::new(0, 0))
    }
}

impl Selection {
    fn at(pos: Pos) -> Self {
        Selection {
            anchor: pos,
            active: pos,
        }
    }

    /// Top-left and bottom-right, whichever way round it was dragged.
    fn rect(&self) -> (Pos, Pos) {
        (
            Pos::new(
                self.anchor.row.min(self.active.row),
                self.anchor.col.min(self.active.col),
            ),
            Pos::new(
                self.anchor.row.max(self.active.row),
                self.anchor.col.max(self.active.col),
            ),
        )
    }

    fn contains(&self, pos: Pos) -> bool {
        let (start, end) = self.rect();
        (start.row..=end.row).contains(&pos.row) && (start.col..=end.col).contains(&pos.col)
    }
}

/// The elements this shell writes to. No document state — that is all in the core.
/// The elements this pane writes to. No document state — that is all in the core.
///
/// The shared chrome — the toolbar, the file input, the document's name — belongs to
/// [`crate::Shell`] and is not here: it is the same chrome whichever document is open, and
/// two panes reaching for the same button is how the two disagree about which is enabled.
struct Dom {
    document: Document,
    /// The scrolling box, and the thing that holds the keyboard.
    surface: HtmlElement,
    head: Element,
    body: Element,
    address: HtmlElement,
    formula: HtmlInputElement,
    tabs: HtmlElement,
    message: HtmlElement,
    summary: HtmlElement,
}

impl Dom {
    fn find(document: &Document) -> Result<Self, JsValue> {
        Ok(Dom {
            document: document.clone(),
            surface: element(document, "surface")?,
            head: element(document, "head-row")?,
            body: element(document, "body")?,
            address: element(document, "address")?,
            formula: element(document, "formula")?,
            tabs: element(document, "tabs")?,
            message: element(document, "message")?,
            summary: element(document, "summary")?,
        })
    }
}

pub struct Ui {
    pub app: Arc<App>,
    dom: Dom,
    /// Set by the observer, cleared by the repaint it asked for.
    pending: Arc<AtomicBool>,
    // Everything below is *presentation*: which part of the document is on screen
    // and what the user has picked out. None of it is the document, which is why
    // the core neither knows nor keeps it.
    sheet: Cell<usize>,
    selection: Cell<Selection>,
    scroll: Cell<Pos>,
    editing: Cell<bool>,
    message: RefCell<String>,
}

impl Ui {
    /// Build the pane and wire the events that belong to it. The shared chrome is
    /// the shell's — see `crate`.
    pub fn new(
        document: &Document,
        app: Arc<App>,
        pending: Arc<AtomicBool>,
    ) -> Result<Rc<Self>, JsValue> {
        declare_cell_size(document)?;
        let ui = Rc::new(Ui {
            app,
            dom: Dom::find(document)?,
            pending,
            sheet: Cell::new(0),
            selection: Cell::new(Selection::default()),
            scroll: Cell::new(Pos::new(0, 0)),
            editing: Cell::new(false),
            message: RefCell::new(String::new()),
        });
        wire_grid(&ui)?;
        wire_editor(&ui)?;
        Ok(ui)
    }

    /// Take the keyboard. The pane that is showing holds it; the other one holds nothing.
    pub fn focus(&self) -> Result<(), JsValue> {
        self.dom.surface.focus()
    }

    /// A document arrived: into the core, and the presentation state a new one resets.
    pub fn open(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.app
            .open_bytes(name, bytes)
            .map_err(|e| e.to_string())?;
        self.sheet.set(0);
        self.scroll.set(Pos::new(0, 0));
        self.selection.set(Selection::default());
        self.editing.set(false);
        Ok(())
    }

    /// What a download is made of. An operation, not a getter — see [`App::save_bytes`].
    pub fn save_bytes(&self, form: Form) -> Result<Vec<u8>, String> {
        self.app.save_bytes(form).map_err(|e| e.to_string())
    }

    pub fn refresh(&self) {
        self.pending.store(false, Ordering::SeqCst);
        if let Err(error) = self.render() {
            web_sys::console::error_1(&error);
        }
    }

    fn render(&self) -> Result<(), JsValue> {
        let sheet = self.sheet.get();
        let visible = self.visible();
        let scroll = self.scroll.get();
        let rows = scroll.row..scroll.row.saturating_add(visible.0);
        let cols = scroll.col..scroll.col.saturating_add(visible.1);
        let viewport = self
            .app
            .get_viewport(sheet, rows.clone(), cols.clone())
            .map_err(js)?;

        let selection = self.selection.get();
        let editing = self.editing.get();

        // Column headers. Rebuilt with the body, because a horizontal scroll
        // changes which letters are over which cells.
        self.dom.head.set_text_content(None);
        let corner = self.corner()?;
        self.dom.head.append_child(&corner)?;
        for col in cols.clone() {
            let cell = self.dom.document.create_element("th")?;
            cell.set_class_name(
                match selection.contains(Pos::new(selection.active.row, col)) {
                    true => "head col current",
                    false => "head col",
                },
            );
            cell.set_text_content(Some(&lex::column_name(col)));
            self.dom.head.append_child(&cell)?;
        }

        // One element per visible cell, thrown away and rebuilt each frame. The
        // cost is bounded by the window rather than the document, which is what the
        // viewport is for.
        self.dom.body.set_text_content(None);
        for row in rows.clone() {
            let line = self.dom.document.create_element("tr")?;
            let header = self.dom.document.create_element("th")?;
            header.set_class_name(match row == selection.active.row {
                true => "head row current",
                false => "head row",
            });
            header.set_text_content(Some(&(row + 1).to_string()));
            line.append_child(&header)?;

            for col in cols.clone() {
                let pos = Pos::new(row, col);
                let cell = self.dom.document.create_element("td")?;
                let active = pos == selection.active;
                cell.set_class_name(match (active, selection.contains(pos)) {
                    (true, _) => "cell active",
                    (_, true) => "cell selected",
                    _ => "cell",
                });
                cell.set_attribute("data-row", &row.to_string())?;
                cell.set_attribute("data-col", &col.to_string())?;
                // While editing, the active cell shows what is being typed. The
                // text still comes from the one `<input>` that holds it — this is a
                // second *view*, never a second copy.
                let text = match active && editing {
                    true => self.dom.formula.value(),
                    false => viewport.text(row, col).unwrap_or_default().to_string(),
                };
                cell.set_text_content(Some(&text));
                let numeric = matches!(viewport.get(row, col), Some(CellValue::Number(_)));
                let css = css_of(viewport.style(row, col), numeric);
                if !css.is_empty() {
                    cell.set_attribute("style", &css)?;
                }
                line.append_child(&cell)?;
            }
            self.dom.body.append_child(&line)?;
        }

        self.render_tabs()?;
        self.render_chrome(&selection)?;
        Ok(())
    }

    fn corner(&self) -> Result<Element, JsValue> {
        let corner = self.dom.document.create_element("th")?;
        corner.set_class_name("head corner");
        Ok(corner)
    }

    fn render_tabs(&self) -> Result<(), JsValue> {
        self.dom.tabs.set_text_content(None);
        for index in 0..self.app.sheet_count() {
            let Ok(name) = self.app.sheet_name(index) else {
                continue;
            };
            let tab = self.dom.document.create_element("button")?;
            tab.set_class_name(match index == self.sheet.get() {
                true => "tab current",
                false => "tab",
            });
            tab.set_attribute("type", "button")?;
            tab.set_attribute("data-sheet", &index.to_string())?;
            tab.set_text_content(Some(&name));
            self.dom.tabs.append_child(&tab)?;
        }
        Ok(())
    }

    fn render_chrome(&self, selection: &Selection) -> Result<(), JsValue> {
        let sheet = self.sheet.get();
        let active = selection.active;
        self.dom
            .address
            .set_text_content(Some(&a1::format(None, active)));

        // The formula bar shows the cell only when it is not the thing being
        // edited; overwriting it mid-edit would throw away what was typed.
        if !self.editing.get() {
            let text = self.app.input_text(sheet, active).unwrap_or_default();
            self.dom.formula.set_value(&text);
        }

        self.dom
            .message
            .set_text_content(Some(&self.message.borrow()));

        let (rows, cols) = self.app.used_extent(sheet).unwrap_or((0, 0));
        let (start, end) = selection.rect();
        let span = match start == end {
            true => String::new(),
            false => format!(
                " · {}×{} selected",
                end.row - start.row + 1,
                end.col - start.col + 1
            ),
        };
        self.dom.summary.set_text_content(Some(&format!(
            "{} · {rows}×{cols} used{span}",
            self.app.sheet_name(sheet).unwrap_or_default(),
        )));

        Ok(())
    }

    /// How much of the sheet is on screen. A function of the surface alone — never
    /// of the grid inside it, which is [`layout::CELL`]'s whole point.
    fn visible(&self) -> (u32, u32) {
        layout::CELL.visible(
            self.dom.surface.client_width() as f64,
            self.dom.surface.client_height() as f64,
        )
    }

    // --- input ---

    fn on_key(&self, event: &KeyboardEvent) {
        let key = event.key();
        let chord = Chord {
            key: &key,
            // ⌘ on macOS, Ctrl everywhere else, resolved here so the keymap never
            // asks what it is running on.
            primary: event.ctrl_key() || event.meta_key(),
            shift: event.shift_key(),
            alt: event.alt_key(),
        };
        let Some(action) = keymap::action_for(&chord, self.editing.get()) else {
            // Not ours: Ctrl+T and F5 still belong to the browser.
            return;
        };
        // A key this shell claimed must not also do its default — Tab moves focus,
        // Ctrl+S opens the browser's own save dialog.
        event.prevent_default();
        if let Err(error) = self.apply(action) {
            web_sys::console::error_1(&error);
        }
    }

    pub fn apply(&self, action: Action) -> Result<(), JsValue> {
        match action {
            Action::Move { motion, extend } => self.move_to(motion, extend),
            Action::Begin(seed) => self.begin(seed)?,
            Action::Commit(direction) => self.commit(direction)?,
            Action::Cancel => self.cancel()?,
            Action::Clear => self.clear(),
            Action::Undo => self.step_history(true),
            Action::Redo => self.step_history(false),
            // Chrome, not grid: opening and saving are the same gesture whichever
            // document type is showing, so they belong to the one shell that owns both.
            Action::Open => crate::with_shell(|shell| shell.open_picker()),
            Action::Save => crate::with_shell(|shell| shell.save()),
            Action::Recalc => self.recalc(),
        }
        Ok(())
    }

    fn move_to(&self, motion: Motion, extend: bool) {
        let sheet = self.sheet.get();
        let extent = self.app.used_extent(sheet).unwrap_or((0, 0));
        let selection = self.selection.get();
        let page = self.visible().0.saturating_sub(1);
        let active = keymap::moved(selection.active, motion, extent, page);
        self.set_selection(match extend {
            true => Selection {
                anchor: selection.anchor,
                active,
            },
            false => Selection::at(active),
        });
    }

    fn set_selection(&self, selection: Selection) {
        self.selection.set(selection);
        self.scroll.set(layout::follow(
            self.scroll.get(),
            selection.active,
            self.visible(),
        ));
        // Nothing in the core changed, so nothing will tell the page to repaint.
        self.request_repaint();
    }

    /// Start editing. `Some(c)` replaces the cell with that character, `None` keeps
    /// what is there — F2, and a double-click.
    fn begin(&self, seed: Option<char>) -> Result<(), JsValue> {
        let text = match seed {
            Some(c) => c.to_string(),
            None => self
                .app
                .input_text(self.sheet.get(), self.selection.get().active)
                .unwrap_or_default(),
        };
        self.editing.set(true);
        self.dom.formula.set_value(&text);
        self.dom.formula.focus()?;
        // Focusing an `<input>` selects it in some browsers, and the caret belongs
        // after what is there or the next keystroke deletes the seed — the same trap
        // `ui_sheet_gtk`'s `Grid::begin` documents, in a different toolkit.
        let end = text.chars().count() as u32;
        self.dom.formula.set_selection_range(end, end)?;
        self.set_message(String::new());
        Ok(())
    }

    /// Store what the formula bar holds, then move on.
    ///
    /// The display-form → canonical → [`App::enter`] path, and the "stay open on a
    /// bad formula" rule, are the same three lines `grind-tui` and `grind-sheet-gtk` run:
    /// a formula is typed in A1 form and stored in ODF's, and the one place that
    /// conversion lives is the core.
    fn commit(&self, direction: Option<Dir>) -> Result<(), JsValue> {
        if !self.editing.get() {
            // Enter with no edit open is "edit this cell", which is what a
            // spreadsheet does with it.
            return self.begin(None);
        }
        let sheet = self.sheet.get();
        let active = self.selection.get().active;
        let text = self.dom.formula.value();
        let before = self.app.input_text(sheet, active).unwrap_or_default();

        if before != text {
            let input = match text.starts_with('=') {
                true => match display::from_display(&text) {
                    Ok(canonical) => canonical,
                    Err(error) => {
                        // The edit stays open, with the caret on the problem.
                        self.set_message(format!("{} (at {})", error.message, error.at));
                        let at = text[..error.at.min(text.len())].chars().count() as u32;
                        self.dom.formula.focus()?;
                        self.dom.formula.set_selection_range(at, at)?;
                        return Ok(());
                    }
                },
                false => text,
            };
            match self.app.enter(sheet, active, &input, RecalcMode::Document) {
                Ok(outcome) => self.set_message(match outcome.recalc.filter(|r| r.spoiled > 0) {
                    Some(recalc) => format!(
                        "{} cell(s) skipped recalculating — press Recalc",
                        recalc.spoiled
                    ),
                    None => String::new(),
                }),
                Err(error) => {
                    self.set_message(error.to_string());
                    return Ok(());
                }
            }
        }
        self.end_edit()?;
        self.set_selection(Selection::at(keymap::after_commit(active, direction)));
        Ok(())
    }

    fn cancel(&self) -> Result<(), JsValue> {
        if !self.editing.get() {
            return Ok(());
        }
        self.end_edit()?;
        self.set_message(String::new());
        Ok(())
    }

    /// Close the editor and hand the keyboard back, or the next keystroke goes
    /// nowhere.
    fn end_edit(&self) -> Result<(), JsValue> {
        self.editing.set(false);
        self.dom.surface.focus()?;
        self.request_repaint();
        Ok(())
    }

    /// Empty the selection. `App::enter` with nothing in it is what clearing *is*,
    /// so a whole rectangle goes through `clear_range` and lands as one undo step.
    fn clear(&self) {
        let (start, end) = self.selection.get().rect();
        match self.app.clear_range(self.sheet.get(), start, end) {
            Ok(0) => self.set_message(String::new()),
            Ok(n) => self.set_message(format!("Cleared {n} cell(s)")),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    fn step_history(&self, undo: bool) {
        let moved = match undo {
            true => self.app.undo(),
            false => self.app.redo(),
        };
        if !moved {
            self.set_message(match undo {
                true => "Nothing to undo".into(),
                false => "Nothing to redo".into(),
            });
        }
    }

    fn recalc(&self) {
        match self.app.recalc() {
            Ok(recalc) => self.set_message(match recalc.spoiled {
                0 => format!("Recalculated {} cell(s)", recalc.changed),
                spoiled => format!(
                    "Recalculated {} cell(s); {spoiled} left alone — this build cannot \
                     reproduce their functions",
                    recalc.changed
                ),
            }),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    fn on_click(&self, event: &MouseEvent) -> Result<(), JsValue> {
        let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return Ok(());
        };
        if let Some(sheet) = closest_number(&target, "button.tab", "data-sheet") {
            return self.switch_to(sheet as usize);
        }
        // The DOM knows which box the pointer is in; the cell carries its address.
        let Some(cell) = target.closest("td.cell")? else {
            return Ok(());
        };
        let (Some(row), Some(col)) = (attribute(&cell, "data-row"), attribute(&cell, "data-col"))
        else {
            return Ok(());
        };
        let pos = Pos::new(row, col);
        // Clicking away from an open edit stores it, which is what every
        // spreadsheet does and what a user who clicks the next cell means.
        if self.editing.get() {
            self.commit(None)?;
        }
        match event.detail() >= 2 {
            // A double-click edits, keeping what the cell holds.
            true => {
                self.set_selection(Selection::at(pos));
                self.begin(None)?;
            }
            false => {
                let anchor = match event.shift_key() {
                    true => self.selection.get().anchor,
                    false => pos,
                };
                self.set_selection(Selection {
                    anchor,
                    active: pos,
                });
                self.dom.surface.focus()?;
            }
        }
        Ok(())
    }

    fn switch_to(&self, sheet: usize) -> Result<(), JsValue> {
        if sheet >= self.app.sheet_count() {
            return Ok(());
        }
        if self.editing.get() {
            self.commit(None)?;
        }
        self.sheet.set(sheet);
        self.scroll.set(Pos::new(0, 0));
        self.set_selection(Selection::default());
        Ok(())
    }

    fn on_wheel(&self, event: &WheelEvent) {
        // The surface scrolls by whole cells rather than pixels: the viewport is
        // addressed in rows and columns, so anything else would ask the core for a
        // fraction of a cell it has no way to give.
        let (rows, cols) = match event.shift_key() {
            // Shift+wheel is sideways, the convention every browser already has.
            true => (0.0, event.delta_y() + event.delta_x()),
            false => (event.delta_y(), event.delta_x()),
        };
        let step = |delta: f64| match delta {
            d if d > 0.0 => 3,
            d if d < 0.0 => -3,
            _ => 0,
        };
        let (rows, cols) = (step(rows), step(cols));
        if (rows, cols) == (0, 0) {
            return;
        }
        event.prevent_default();
        self.scroll
            .set(layout::scrolled_by(self.scroll.get(), rows, cols));
        self.request_repaint();
    }

    /// Ask for a repaint the core will not send: a scroll, a selection, a resize —
    /// anything that changes the picture without changing the document. Coalesces
    /// exactly as the observer does, and for the same reason.
    pub fn request_repaint(&self) {
        if !self.pending.swap(true, Ordering::SeqCst) {
            request_frame();
        }
    }

    /// Change something only the shell knows about, and show it.
    pub fn set_message(&self, message: String) {
        *self.message.borrow_mut() = message;
        self.request_repaint();
    }
}

/// A `CellStyle` as inline CSS.
///
/// ODF's style vocabulary is CSS's here, near enough to pass values through
/// verbatim: `fo:font-weight`, `fo:color` and `fo:border` are spelled the way CSS
/// spells them, which is not a coincidence — both took them from XSL. The values
/// stay exactly as the document wrote them (doc/ods-format.md's rule), so anything
/// this shell does not understand is passed to the browser rather than dropped.
fn css_of(style: Option<&CellStyle>, numeric: bool) -> String {
    let mut css = String::new();
    // A number right-aligns unless the document says otherwise — the convention
    // every spreadsheet has, and the reason it is here rather than in the core is
    // that it is a rendering default, not a property of the cell.
    if numeric && style.is_none_or(|s| s.align.is_none()) {
        css.push_str("text-align:right;");
    }
    let Some(style) = style else {
        return css;
    };
    fn set(css: &mut String, property: &str, value: &Option<String>) {
        if let Some(value) = value {
            css.push_str(&format!("{property}:{value};"));
        }
    }
    set(&mut css, "font-weight", &style.font_weight);
    set(&mut css, "font-style", &style.font_style);
    set(&mut css, "font-size", &style.font_size);
    set(&mut css, "color", &style.color);
    set(&mut css, "background-color", &style.background);
    set(&mut css, "text-align", &style.align);
    // `automatic` is ODF's "you decide", which in CSS is saying nothing at all.
    if style
        .vertical_align
        .as_deref()
        .is_some_and(|v| v != "automatic")
    {
        set(&mut css, "vertical-align", &style.vertical_align);
    }
    if let Some(wrap) = &style.wrap {
        css.push_str(match wrap.as_str() {
            "wrap" => "white-space:normal;",
            _ => "white-space:nowrap;",
        });
    }
    for (edge, value) in EDGES.iter().zip(&style.borders) {
        set(&mut css, &format!("border-{edge}"), value);
    }
    css
}

fn attribute(element: &Element, name: &str) -> Option<u32> {
    element.get_attribute(name)?.parse().ok()
}

fn closest_number(element: &Element, selector: &str, name: &str) -> Option<u32> {
    attribute(&element.closest(selector).ok()??, name)
}

// --- wiring ---

fn wire_grid(ui: &Rc<Ui>) -> Result<(), JsValue> {
    let keys = ui.clone();
    listen(&ui.dom.surface, "keydown", move |event: KeyboardEvent| {
        keys.on_key(&event);
    })?;

    // One listener for the whole grid rather than one per cell: the cells are
    // rebuilt every frame, and a listener each would be a listener each frame.
    let click = ui.clone();
    listen(&ui.dom.surface, "mousedown", move |event: MouseEvent| {
        if let Err(error) = click.on_click(&event) {
            web_sys::console::error_1(&error);
        }
    })?;

    let tabs = ui.clone();
    listen(&ui.dom.tabs, "click", move |event: MouseEvent| {
        if let Err(error) = tabs.on_click(&event) {
            web_sys::console::error_1(&error);
        }
    })?;

    let wheel = ui.clone();
    listen(&ui.dom.surface, "wheel", move |event: WheelEvent| {
        wheel.on_wheel(&event);
    })
}

fn wire_editor(ui: &Rc<Ui>) -> Result<(), JsValue> {
    // The formula bar has its own key listener: the surface's does not see these,
    // because focus is in the `<input>`.
    let keys = ui.clone();
    listen(&ui.dom.formula, "keydown", move |event: KeyboardEvent| {
        keys.on_key(&event);
    })?;

    // Typing in the bar with no edit open starts one, so the first character is not
    // lost — and every keystroke repaints, which is what mirrors the text into the
    // cell being edited.
    let input = ui.clone();
    listen(&ui.dom.formula, "input", move |_: Event| {
        input.editing.set(true);
        input.request_repaint();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_is_a_rectangle_whichever_way_it_was_dragged() {
        let up = Selection {
            anchor: Pos::new(4, 4),
            active: Pos::new(1, 2),
        };
        assert_eq!(up.rect(), (Pos::new(1, 2), Pos::new(4, 4)));
        assert!(up.contains(Pos::new(2, 3)));
        assert!(!up.contains(Pos::new(5, 3)));
        assert_eq!(
            Selection::at(Pos::new(2, 2)).rect(),
            (Pos::new(2, 2), Pos::new(2, 2))
        );
    }

    #[test]
    fn a_style_becomes_the_css_the_document_asked_for() {
        let style = CellStyle {
            font_weight: Some("bold".into()),
            background: Some("#ffdc00".into()),
            borders: [Some("0.06pt solid #000000".into()), None, None, None],
            ..CellStyle::default()
        };
        let css = css_of(Some(&style), false);
        assert!(css.contains("font-weight:bold;"), "{css}");
        assert!(css.contains("background-color:#ffdc00;"), "{css}");
        assert!(css.contains("border-left:0.06pt solid #000000;"), "{css}");
        assert!(!css.contains("border-right"), "{css}");
    }

    #[test]
    fn a_number_right_aligns_until_the_document_says_otherwise() {
        assert_eq!(css_of(None, true), "text-align:right;");
        assert_eq!(css_of(None, false), "");
        let centred = CellStyle {
            align: Some("center".into()),
            ..CellStyle::default()
        };
        let css = css_of(Some(&centred), true);
        assert!(css.contains("text-align:center;"), "{css}");
        assert!(!css.contains("text-align:right;"), "{css}");
    }

    /// ODF says `automatic` where CSS has nothing to say, and `vertical-align` is
    /// the one property where passing the value through would be invalid.
    #[test]
    fn an_automatic_vertical_alignment_is_left_unsaid() {
        let style = CellStyle {
            vertical_align: Some("automatic".into()),
            ..CellStyle::default()
        };
        assert_eq!(css_of(Some(&style), false), "");
        let middle = CellStyle {
            vertical_align: Some("middle".into()),
            ..CellStyle::default()
        };
        assert_eq!(css_of(Some(&middle), false), "vertical-align:middle;");
    }
}
