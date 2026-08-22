// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `sheet-web` — the browser shell over `sheet-core`.
//!
//! A third kind of shell. `sheet-cli` and `sheet-tui` are Rust calling the core
//! directly and `sheet-gtk` is Rust through GTK's bindings; this one is Rust
//! compiled to `wasm32-unknown-unknown`, talking to the page through
//! `wasm-bindgen`. The core is an ordinary Cargo dependency — there is no FFI
//! layer, because the shell is Rust too.
//!
//! **This is the honest test of rule 5** (doc/plan.md: *do not assume a
//! filesystem*). The browser has no paths. A document arrives from the File API as
//! bytes and leaves as a download, which is exactly what [`App::open_bytes`] and
//! [`App::save_bytes`] exist for — the shell never learns a path, because there
//! isn't one. The core needed no change to run here, which is the whole point of
//! having paired every `*_file` with a `*_bytes` from the start.
//!
//! The DOM is a **renderer**, not the document, the same rule the other three
//! shells follow. No `contenteditable` anywhere: every visible cell is rebuilt from
//! [`App::get_viewport`] on each repaint and thrown away, so the page cannot become
//! a second source of truth. The one editable element is the formula bar's
//! `<input>`, and what it holds is not a cell until [`App::enter`] says so.
//!
//! Hit-testing is the DOM's job, not this shell's: every cell carries its address
//! in `data-row`/`data-col` and a click reads it back off the event target. That is
//! rung four of the ladder — the platform already knows which box the pointer is
//! in, and `layout.rs` is left with only the arithmetic the platform cannot do.

mod keymap;
mod layout;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sheet_core::formula::{display, lex};
use sheet_core::style::{CellStyle, EDGES};
use sheet_core::{App, CellValue, Form, Observer, Pos, RecalcMode, a1};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Document, Element, Event, File, HtmlAnchorElement, HtmlButtonElement, HtmlElement,
    HtmlInputElement, KeyboardEvent, MouseEvent, WheelEvent,
};

use keymap::{Action, Chord, Dir, Motion};

/// The ODF sheet limits, and what a plain move clamps to — the same two numbers
/// `sheet-tui` states, for the same reason.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

/// What a document nobody has opened is downloaded as. `.ods` rather than `.fods`
/// because it is the form every other spreadsheet opens without being told.
const UNTITLED: &str = "untitled.ods";

thread_local! {
    /// The live shell, so an animation-frame callback can find its way back. The
    /// page owns it until the tab closes; nothing ever takes it out again.
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

/// Called by the generated glue as soon as the module is instantiated.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    let app = Arc::new(App::new());
    let ui = Rc::new(Ui {
        app: app.clone(),
        dom: Dom::find(&document)?,
        pending: Arc::new(AtomicBool::new(false)),
        sheet: Cell::new(0),
        selection: Cell::new(Selection::default()),
        scroll: Cell::new(Pos::new(0, 0)),
        editing: Cell::new(false),
        name: RefCell::new(String::new()),
        message: RefCell::new("Open an .ods or .fods file, or start typing.".to_string()),
    });

    // The core pushes, the shell never polls (doc/plan.md rule 3) — the same
    // contract `sheet-tui` implements with a flag and `sheet-gtk` with observers.
    app.set_observer(Arc::new(Notifier(ui.pending.clone())));

    declare_cell_size(&document)?;
    wire_grid(&ui)?;
    wire_editor(&ui)?;
    wire_toolbar(&ui)?;
    wire_file_input(&ui)?;
    wire_window(&ui, &window)?;

    UI.with(|slot| *slot.borrow_mut() = Some(ui.clone()));

    // `?doc=<url>` opens that document at startup — the page's own address is the
    // only way to point this shell at a file without a picker.
    if let Ok(search) = window.location().search()
        && let Some(url) = doc_param(&search)
    {
        spawn_local(ui.clone().fetch(url));
    }

    ui.dom.surface.focus()?;
    ui.refresh();
    Ok(())
}

/// The `doc` parameter of a query string, still percent-encoded — which is the form
/// `fetch` wants, so nothing here decodes it.
fn doc_param(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("doc="))
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

/// Hand the page the cell size the viewport arithmetic assumes.
///
/// The stylesheet declares the same two numbers so the empty page has a shape before
/// the module arrives, and this overwrites them: two declarations that must agree are
/// two declarations that will not, and when they disagreed the grid grew a column
/// every repaint.
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

/// Raises a repaint when the core changes, and schedules the frame that draws it.
///
/// [`Observer`] is `Send + Sync`, so this may not hold anything from the page — a
/// wasm module is single-threaded but the trait does not know that. It holds a flag
/// instead, which doubles as "a frame is already scheduled": an edit that notifies
/// twice still repaints once.
struct Notifier(Arc<AtomicBool>);

impl Observer for Notifier {
    fn changed(&self) {
        if !self.0.swap(true, Ordering::SeqCst) {
            request_frame();
        }
    }
}

fn request_frame() {
    let Some(window) = web_sys::window() else {
        return;
    };
    // `once_into_js` hands the closure to JS and frees it after the call, which is
    // what makes a per-frame allocation acceptable.
    let callback = Closure::once_into_js(move || {
        let ui = UI.with(|slot| slot.borrow().clone());
        if let Some(ui) = ui {
            ui.refresh();
        }
    });
    let _ = window.request_animation_frame(callback.unchecked_ref());
}

/// The selected rectangle: where the selection started and where it is now.
///
/// The same two-`Pos` shape `ui_gtk/src/keymap.rs` uses, and it is two rather than a
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
struct Dom {
    document: Document,
    /// The scrolling box, and the thing that holds the keyboard.
    surface: HtmlElement,
    head: Element,
    body: Element,
    address: HtmlElement,
    formula: HtmlInputElement,
    tabs: HtmlElement,
    name: HtmlElement,
    message: HtmlElement,
    summary: HtmlElement,
    open: HtmlButtonElement,
    save: HtmlButtonElement,
    undo: HtmlButtonElement,
    redo: HtmlButtonElement,
    recalc: HtmlButtonElement,
    file_input: HtmlInputElement,
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
            name: element(document, "name")?,
            message: element(document, "message")?,
            summary: element(document, "summary")?,
            open: element(document, "open")?,
            save: element(document, "save")?,
            undo: element(document, "undo")?,
            redo: element(document, "redo")?,
            recalc: element(document, "recalc")?,
            file_input: element(document, "file-input")?,
        })
    }
}

fn element<T: JsCast>(document: &Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("index.html is missing #{id}")))?
        .dyn_into::<T>()
        .map_err(|_| JsValue::from_str(&format!("#{id} is not the element this shell expects")))
}

struct Ui {
    app: Arc<App>,
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
    /// The name the document arrived under, so a download has something to be
    /// called. Not a path — there are none here.
    name: RefCell<String>,
    message: RefCell<String>,
}

impl Ui {
    fn refresh(&self) {
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

        let name = self.document_name();
        self.dom.name.set_text_content(Some(&name));
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

        self.dom.undo.set_disabled(!self.app.can_undo());
        self.dom.redo.set_disabled(!self.app.can_redo());
        self.dom.document.set_title(&format!("{name} — sheet"));
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

    fn apply(&self, action: Action) -> Result<(), JsValue> {
        match action {
            Action::Move { motion, extend } => self.move_to(motion, extend),
            Action::Begin(seed) => self.begin(seed)?,
            Action::Commit(direction) => self.commit(direction)?,
            Action::Cancel => self.cancel()?,
            Action::Clear => self.clear(),
            Action::Undo => self.step_history(true),
            Action::Redo => self.step_history(false),
            Action::Open => self.open_picker(),
            Action::Save => self.save(),
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
        // `ui_gtk`'s `Grid::begin` documents, in a different toolkit.
        let end = text.chars().count() as u32;
        self.dom.formula.set_selection_range(end, end)?;
        self.set_message(String::new());
        Ok(())
    }

    /// Store what the formula bar holds, then move on.
    ///
    /// The display-form → canonical → [`App::enter`] path, and the "stay open on a
    /// bad formula" rule, are the same three lines `sheet-tui` and `sheet-gtk` run:
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

    // --- documents ---

    fn open_picker(&self) {
        // Cleared first, or picking the same file twice fires no change event and
        // the second open silently does nothing.
        self.dom.file_input.set_value("");
        self.dom.file_input.click();
    }

    /// Read a picked file into the core.
    ///
    /// **Bytes, not text** — an `.ods` is a zip, and the name is all the browser
    /// gives us. It travels with the document only so a download has something to
    /// be called; there is no path to write back to.
    async fn load(self: Rc<Self>, file: File) {
        let name = file.name();
        let buffer = match JsFuture::from(file.array_buffer()).await {
            Ok(buffer) => buffer,
            Err(_) => return self.set_message(format!("Could not read {name}")),
        };
        self.open(name, &js_sys::Uint8Array::new(&buffer).to_vec());
    }

    /// A document named in the page's own URL — `?doc=sample.fods` — fetched and
    /// opened as if it had been picked.
    ///
    /// The browser hands a page no path and no way to preload the file picker, so a
    /// document that is *served next to the page* has no other way in. That is the
    /// whole point: `scripts/run.sh web` puts the sample document in `dist/` and
    /// prints the URL, which is how this shell gets demo data in front of it without
    /// a picker and without a second open path — the bytes end up in
    /// [`App::open_bytes`] either way.
    async fn fetch(self: Rc<Self>, url: String) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let failed = || format!("Could not load {url}");
        let response = match JsFuture::from(window.fetch_with_str(&url)).await {
            Ok(response) => web_sys::Response::from(response),
            Err(_) => return self.set_message(failed()),
        };
        if !response.ok() {
            return self.set_message(format!("{url}: {} {}", response.status(), failed()));
        }
        let buffer = match response.array_buffer() {
            Ok(promise) => JsFuture::from(promise).await,
            Err(error) => Err(error),
        };
        let Ok(buffer) = buffer else {
            return self.set_message(failed());
        };
        // The last path segment is the document's name — all a download needs, and
        // the only thing a name is used for here.
        let name = url.rsplit('/').next().unwrap_or(&url).to_owned();
        self.open(name, &js_sys::Uint8Array::new(&buffer).to_vec());
    }

    /// Bytes into the core, and the presentation state that a new document resets.
    fn open(&self, name: String, bytes: &[u8]) {
        match self.app.open_bytes(&name, bytes) {
            Ok(()) => {
                *self.name.borrow_mut() = name.clone();
                self.sheet.set(0);
                self.scroll.set(Pos::new(0, 0));
                self.selection.set(Selection::default());
                self.editing.set(false);
                self.set_message(format!("Opened {name}"));
            }
            // Tolerance is the reader's job, not the shell's: if the core will not
            // take it, saying why is all there is to do.
            Err(error) => self.set_message(format!("{name}: {error}")),
        }
    }

    /// Save by handing the document to a download — the only writing a page may do.
    ///
    /// The form follows the name the document arrived under, so an `.fods` opened
    /// here goes back as flat XML and an `.ods` as a package. R6's spliced writer is
    /// underneath: a document that was only edited comes back as the file it was,
    /// with those cells replaced.
    fn save(&self) {
        let name = self.document_name();
        let bytes = match self.app.save_bytes(form_of(&name)) {
            Ok(bytes) => bytes,
            Err(error) => return self.set_message(error.to_string()),
        };
        match self.download(&name, &bytes) {
            Ok(()) => self.set_message(format!("Saved {name} to your downloads")),
            Err(_) => self.set_message(format!("The browser refused to download {name}")),
        }
    }

    fn download(&self, name: &str, bytes: &[u8]) -> Result<(), JsValue> {
        let parts = js_sys::Array::new();
        parts.push(&js_sys::Uint8Array::from(bytes));
        let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;

        let anchor: HtmlAnchorElement = self
            .dom
            .document
            .create_element("a")?
            .dyn_into()
            .map_err(|_| JsValue::from_str("an anchor is not an anchor"))?;
        anchor.set_href(&url);
        anchor.set_download(name);
        anchor.click();

        // The blob would otherwise be held until the tab closes.
        web_sys::Url::revoke_object_url(&url)
    }

    fn document_name(&self) -> String {
        let name = self.name.borrow();
        match name.is_empty() {
            true => UNTITLED.to_string(),
            false => name.clone(),
        }
    }

    /// Ask for a repaint the core will not send: a scroll, a selection, a resize —
    /// anything that changes the picture without changing the document. Coalesces
    /// exactly as the observer does, and for the same reason.
    fn request_repaint(&self) {
        if !self.pending.swap(true, Ordering::SeqCst) {
            request_frame();
        }
    }

    /// Change something only the shell knows about, and show it.
    fn set_message(&self, message: String) {
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

/// Which ODF form a name asks for. Anything that is not flat XML is a package,
/// which is also the right answer for a name with no extension at all.
fn form_of(name: &str) -> Form {
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("fods") | Some("xml") => Form::Flat,
        _ => Form::Package,
    }
}

fn attribute(element: &Element, name: &str) -> Option<u32> {
    element.get_attribute(name)?.parse().ok()
}

fn closest_number(element: &Element, selector: &str, name: &str) -> Option<u32> {
    attribute(&element.closest(selector).ok()??, name)
}

fn js(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
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

fn wire_toolbar(ui: &Rc<Ui>) -> Result<(), JsValue> {
    // Every button hands the keyboard back: a toolbar that keeps focus leaves the
    // user typing into nothing.
    for (button, action) in [
        (&ui.dom.open, Action::Open),
        (&ui.dom.save, Action::Save),
        (&ui.dom.undo, Action::Undo),
        (&ui.dom.redo, Action::Redo),
        (&ui.dom.recalc, Action::Recalc),
    ] {
        let ui = ui.clone();
        listen(button, "click", move |_: Event| {
            if let Err(error) = ui.apply(action) {
                web_sys::console::error_1(&error);
            }
            // Opening a file puts focus in the picker, which is the one place it
            // should stay.
            if !matches!(action, Action::Open) {
                let _ = ui.dom.surface.focus();
            }
        })?;
    }
    Ok(())
}

fn wire_file_input(ui: &Rc<Ui>) -> Result<(), JsValue> {
    let input = ui.dom.file_input.clone();
    let ui = ui.clone();
    listen(&input, "change", move |_: Event| {
        let Some(file) = ui.dom.file_input.files().and_then(|files| files.get(0)) else {
            return;
        };
        // Reading a file is a promise; nothing else in this shell is async.
        spawn_local(ui.clone().load(file));
    })
}

fn wire_window(ui: &Rc<Ui>, window: &web_sys::Window) -> Result<(), JsValue> {
    // A resize changes how many cells fit, which only a repaint can discover. It
    // must not go through `set_message`: passing the current message back in would
    // hold a `RefCell` borrow across the write.
    let resize = ui.clone();
    listen(window, "resize", move |_: Event| {
        resize.request_repaint();
    })?;

    // The browser's answer to "save before closing?". A page may ask for the prompt
    // but not word it, so there is nothing to phrase here.
    let unload = ui.clone();
    listen(window, "beforeunload", move |event: Event| {
        if !unload.app.can_undo() {
            return;
        }
        event.prevent_default();
        // Chrome still wants the legacy property set as well.
        let _ = js_sys::Reflect::set(
            &event,
            &JsValue::from_str("returnValue"),
            &JsValue::from_str(""),
        );
    })
}

/// Attach a listener for the lifetime of the page.
///
/// The closure is deliberately leaked: it lives exactly as long as the element it
/// is attached to, and both die with the tab.
fn listen<E, F>(target: &web_sys::EventTarget, event: &str, handler: F) -> Result<(), JsValue>
where
    // Any DOM event type: `FromWasmAbi` is what lets JS hand it to a Rust closure.
    E: wasm_bindgen::convert::FromWasmAbi + 'static,
    F: FnMut(E) + 'static,
{
    let closure = Closure::wrap(Box::new(handler) as Box<dyn FnMut(E)>);
    target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_can_be_told_which_document_to_open() {
        assert_eq!(doc_param("?doc=sample.fods").as_deref(), Some("sample.fods"));
        assert_eq!(doc_param("?x=1&doc=a%20b.ods").as_deref(), Some("a%20b.ods"));
        assert_eq!(doc_param(""), None);
        assert_eq!(doc_param("?doc="), None);
    }

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
    fn the_download_form_follows_the_name() {
        assert_eq!(form_of("book.fods"), Form::Flat);
        assert_eq!(form_of("BOOK.FODS"), Form::Flat);
        assert_eq!(form_of("book.ods"), Form::Package);
        assert_eq!(form_of("book"), Form::Package);
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
