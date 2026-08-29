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

mod chart;
pub mod keymap;
mod layout;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use grind_sheet::formula::{display, lex};
use grind_sheet::numfmt::{self, Kind};
use grind_sheet::style::{CellStyle, EDGES};
use grind_sheet::{App, CellValue, Form, Pos, RecalcMode, a1};
use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Element, Event, HtmlElement, HtmlInputElement, KeyboardEvent, MouseEvent, WheelEvent,
};

use crate::command::Entry;
use crate::{element, js, listen, request_frame, set_pressed, set_select, set_swatch};
use keymap::{Action, Chord, Dir, Motion};
use layout::{PX_PER_MM, Tracks};

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
    /// The `<colgroup>`: where a document's own column widths land.
    cols: Element,
    head: Element,
    body: Element,
    /// An `<input>`, not a label — typing an address in it goes there.
    address: HtmlInputElement,
    formula: HtmlInputElement,
    tabs: HtmlElement,
    /// The layer charts float in, over the cells they sit above.
    charts: HtmlElement,
    message: HtmlElement,
    summary: HtmlElement,
}

impl Dom {
    fn find(document: &Document) -> Result<Self, JsValue> {
        Ok(Dom {
            document: document.clone(),
            surface: element(document, "surface")?,
            cols: element(document, "cols")?,
            head: element(document, "head-row")?,
            body: element(document, "body")?,
            address: element(document, "address")?,
            formula: element(document, "formula")?,
            tabs: element(document, "tabs")?,
            charts: element(document, "charts")?,
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
    /// Whether the pointer is down and dragging a rectangle out.
    dragging: Cell<bool>,
    /// Which of `doc/view-modes.md`'s overlays this pane draws. Presentation state, like
    /// everything else here: a view mode is a reading of the document and never a change to
    /// it, so turning one off puts the page back exactly.
    overlays: Cell<grind_sheet::view::Overlays>,
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
            dragging: Cell::new(false),
            overlays: Cell::new(grind_sheet::view::Overlays::NONE),
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
        let widths = self.widths();
        let heights = self.heights();
        let scroll = self.scroll.get();
        let visible = self.visible_with(&widths, &heights);
        let rows = scroll.row..scroll.row.saturating_add(visible.0);
        let cols = scroll.col..scroll.col.saturating_add(visible.1);
        let overlays = self.overlays.get();
        let viewport = self
            .app
            .get_viewport_with(sheet, rows.clone(), cols.clone(), overlays)
            .map_err(js)?;
        // Filtered *and* manually hidden, which is what `hidden_rows` already unions —
        // a row with no height is one the document says is not there.
        let hidden = self.app.hidden_rows(sheet).unwrap_or_default();
        let hidden_cols = self.app.hidden_cols(sheet).unwrap_or_default();

        let selection = self.selection.get();
        let editing = self.editing.get();

        // The column widths the document chose, as `<col>` elements: one declaration per
        // column, which is what `table-layout: fixed` sizes from.
        self.dom.cols.set_text_content(None);
        let corner_col = self.dom.document.create_element("col")?;
        corner_col.set_attribute("style", "width:3.5rem")?;
        self.dom.cols.append_child(&corner_col)?;
        for col in cols.clone() {
            let declaration = self.dom.document.create_element("col")?;
            let width = match hidden_cols.contains(&col) {
                true => 0.0,
                false => widths.size(col),
            };
            declaration.set_attribute("style", &format!("width:{width:.1}px"))?;
            self.dom.cols.append_child(&declaration)?;
        }

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
            cell.set_attribute("data-col", &col.to_string())?;
            cell.set_text_content(Some(&lex::column_name(col)));
            self.dom.head.append_child(&cell)?;
        }

        // One element per visible cell, thrown away and rebuilt each frame. The
        // cost is bounded by the window rather than the document, which is what the
        // viewport is for.
        self.dom.body.set_text_content(None);
        for row in rows.clone() {
            let line = self.dom.document.create_element("tr")?;
            if hidden.contains(&row) {
                // Drawn at no height rather than left out: the rows around it keep their
                // addresses, and the run reads as a fold rather than as a gap.
                line.set_attribute("style", "display:none")?;
            } else if heights.is_sized(row) {
                line.set_attribute("style", &format!("height:{:.1}px", heights.size(row)))?;
            }
            let header = self.dom.document.create_element("th")?;
            header.set_class_name(match row == selection.active.row {
                true => "head row current",
                false => "head row",
            });
            header.set_attribute("data-row", &row.to_string())?;
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
                // `doc/view-modes.md`, both overlays, in two attributes and no extra
                // elements: the stylesheet draws the marker and the hint with
                // `content: attr(…)`, so a mode costs one attribute per cell rather than a
                // second DOM node per cell. In role mode the document's own colours are
                // suppressed (§4.5) — colour means role, exclusively.
                match viewport.role(row, col) {
                    Some(role) => {
                        cell.set_attribute("data-role", role.name())?;
                        if !role.marker().is_empty() {
                            cell.set_attribute("data-mark", role.marker())?;
                        }
                    }
                    None => {
                        let css = css_of(viewport.style(row, col), numeric);
                        if !css.is_empty() {
                            cell.set_attribute("style", &css)?;
                        }
                    }
                }
                if overlays.names
                    && let Some(name) = viewport.name_at(row, col)
                    && hint_here(&viewport, &hidden, row, col)
                {
                    cell.set_attribute("data-name", name)?;
                }
                line.append_child(&cell)?;
            }
            self.dom.body.append_child(&line)?;
        }

        self.render_charts(&widths, &heights)?;
        self.render_tabs()?;
        self.render_chrome(&selection)?;
        Ok(())
    }

    /// Every chart on this sheet, over the cells it floats above.
    ///
    /// Read fresh and thrown away like everything else here (doc/plan.md rule 1). A chart's
    /// own position is an ODF length from the table's corner, so it is placed in pixels from
    /// that corner *minus* whatever has been scrolled past — the same arithmetic
    /// `ui_sheet_gtk/src/geom.rs` does, in a different unit.
    fn render_charts(&self, widths: &Tracks, heights: &Tracks) -> Result<(), JsValue> {
        self.dom.charts.set_text_content(None);
        let sheet = self.sheet.get();
        let Ok(charts) = self.app.charts(sheet) else {
            return Ok(());
        };
        if charts.is_empty() {
            return Ok(());
        }
        let scroll = self.scroll.get();
        // The distance the corner has been scrolled past, and the headers' own band.
        let past_x = widths.span(0, scroll.col);
        let past_y = heights.span(0, scroll.row);
        let header_w = 3.5 * 16.0;
        let header_h = layout::CELL.cell_h;

        for (index, chart) in charts.iter().enumerate() {
            let px = |length: &str| grind_sheet::style::length_mm(length).map(|mm| mm * PX_PER_MM);
            let (Some(x), Some(y), Some(w), Some(h)) = (
                px(&chart.x),
                px(&chart.y),
                px(&chart.width),
                px(&chart.height),
            ) else {
                // A length this build cannot parse is a chart it does not draw, which is §9's
                // own tolerance applied to a picture.
                continue;
            };
            let Ok(data) = self.app.chart_data(sheet, index) else {
                continue;
            };
            let frame = self.dom.document.create_element("div")?;
            frame.set_class_name("chart");
            frame.set_attribute("data-chart", &index.to_string())?;
            frame.set_attribute(
                "style",
                &format!(
                    "left:{:.1}px;top:{:.1}px;width:{w:.1}px;height:{h:.1}px",
                    header_w + x - past_x,
                    header_h + y - past_y
                ),
            )?;
            frame.set_inner_html(&chart::svg(chart, &data, w, h));
            self.dom.charts.append_child(&frame)?;
        }
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
        // Not while it is being typed into: the address box is an input, and rewriting it
        // under the caret would make it impossible to type a second character.
        if self.dom.document.active_element().as_ref() != Some(self.dom.address.as_ref()) {
            let (start, end) = selection.rect();
            self.dom.address.set_value(&match start == end {
                true => a1::format(None, active),
                false => format!("{}:{}", a1::format(None, start), a1::format(None, end)),
            });
        }

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

    /// The column widths this sheet declares, over the default. Read per frame like
    /// everything else — a sheet sizes a handful of columns, and re-reading them is cheaper
    /// than knowing when they changed.
    fn widths(&self) -> Tracks {
        Tracks::new(
            layout::CELL.cell_w,
            self.app.col_widths(self.sheet.get()).unwrap_or_default(),
        )
    }

    fn heights(&self) -> Tracks {
        Tracks::new(
            layout::CELL.cell_h,
            self.app.row_heights(self.sheet.get()).unwrap_or_default(),
        )
    }

    /// How much of the sheet is on screen. A function of the surface and the document's own
    /// track sizes — never of the grid inside it, which is [`layout::CELL`]'s whole point.
    fn visible(&self) -> (u32, u32) {
        self.visible_with(&self.widths(), &self.heights())
    }

    fn visible_with(&self, widths: &Tracks, heights: &Tracks) -> (u32, u32) {
        let scroll = self.scroll.get();
        (
            heights.fit(scroll.row, self.dom.surface.client_height() as f64),
            widths.fit(scroll.col, self.dom.surface.client_width() as f64),
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
            // Everything else is a command, and takes the same path a palette row and a
            // toolbar button do — the chrome answers its own and hands the rest back here.
            Action::Run(id) => crate::run_command(id),
        }
        Ok(())
    }

    // --- commands ---

    /// Every verb this pane answers, by the id the palette, the toolbar and the keyboard all
    /// name it with ([`crate::command::SHEET`]).
    pub fn run(&self, id: &str) {
        let style = |set: fn(&mut CellStyle)| self.merge_style(set);
        match id {
            "sheet.recalc" => self.recalc(),
            "view.roles" => self.overlay(true),
            "view.names" => self.overlay(false),
            "edit.clear" => self.clear(),
            "edit.fill-down" => self.fill(true),
            "edit.fill-right" => self.fill(false),
            "edit.select-all" => self.select_all(),

            "style.bold" => style(|s| toggle(&mut s.font_weight, "bold")),
            "style.italic" => style(|s| toggle(&mut s.font_style, "italic")),
            "style.align-left" => style(|s| set(&mut s.align, "start")),
            "style.align-center" => style(|s| set(&mut s.align, "center")),
            "style.align-right" => style(|s| set(&mut s.align, "end")),
            "style.align-clear" => style(|s| s.align = None),
            "style.wrap" => style(|s| toggle(&mut s.wrap, "wrap")),
            "style.border" => style(|s| s.set_border(Some(BORDER.to_owned()))),
            "style.border-clear" => style(|s| s.set_border(None)),
            "style.clear" => self.set_style_of_selection(None),

            "format.general" => self.set_format_of_selection(None),
            "format.integer" => self.preset(Kind::Number, 0),
            "format.number" => self.preset(Kind::Number, 2),
            "format.percent" => self.preset(Kind::Percentage, 0),
            "format.currency" => self.preset(Kind::Currency, 2),
            "format.date" => self.preset(Kind::Date, 0),
            "format.time" => self.preset(Kind::Time, 0),
            "format.datetime" => self.set_format_of_selection(Some(numfmt::datetime_preset())),
            "format.more" => self.step_decimals(1),
            "format.fewer" => self.step_decimals(-1),

            "sheet.add" => self.add_sheet(),
            "sheet.rename" => self.rename_sheet(),
            "sheet.delete" => self.delete_sheet(),

            id => match id.strip_prefix("goto:") {
                Some(where_) => self.go_to(where_),
                None => self.set_message(format!("No such command: {id}")),
            },
        }
    }

    /// What the palette offers for a query that is not a verb: somewhere to go.
    ///
    /// An address or a range, a defined name, or a sheet — the three things a spreadsheet
    /// navigates by, in one box. This is why there is no separate go-to dialog.
    pub fn targets(&self, query: &str) -> Vec<Entry> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        // An address only counts if it names a *cell* — `bol` lexes as the column BOL, and
        // offering "Go to BOL" above "Bold" would put a destination where a verb was meant.
        // Requiring a row is what tells a word from an address.
        let names_a_cell = a1::parse(query)
            .is_ok_and(|reference| reference.start.row.is_some() && reference.start.col.is_some());
        if names_a_cell {
            out.push(Entry::target(
                format!("goto:{query}"),
                format!("Go to {}", query.to_uppercase()),
                "Go",
            ));
        }
        let lower = query.to_lowercase();
        for (name, expression) in self.app.names() {
            if name.to_lowercase().contains(&lower) {
                out.push(Entry::target(
                    format!("goto:{name}"),
                    format!("{name} — {expression}"),
                    "Name",
                ));
            }
        }
        for index in 0..self.app.sheet_count() {
            let Ok(name) = self.app.sheet_name(index) else {
                continue;
            };
            if index != self.sheet.get() && name.to_lowercase().contains(&lower) {
                out.push(Entry::target(
                    format!("goto:{name}."),
                    format!("Sheet {name}"),
                    "Go",
                ));
            }
        }
        out.truncate(6);
        out
    }

    /// Go where an address says. A trailing dot is a *sheet* with no cell — `Data.` means
    /// "that sheet, wherever I was" — which is the one spelling `a1` already refuses and the
    /// palette needs.
    fn go_to(&self, where_: &str) {
        if let Some(name) = where_.strip_suffix('.') {
            match a1::sheet(&self.app, name) {
                Ok(index) => {
                    let _ = self.switch_to(index);
                }
                Err(error) => self.set_message(error.to_string()),
            }
            return;
        }
        // A defined name resolves through the same parser a formula's reference does, so
        // `Totals` goes wherever the document says it is.
        let expression = self
            .app
            .names()
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(where_))
            .map(|(_, expression)| expression);
        let address = expression.as_deref().unwrap_or(where_);
        let parsed = match address.starts_with('[') {
            true => a1::parse_bracketed(address),
            false => a1::parse(address),
        };
        let Ok(reference) = parsed else {
            return self.set_message(format!("{where_} is not an address"));
        };
        match a1::resolve(&self.app, &reference) {
            Ok((sheet, start, end)) => {
                if sheet != self.sheet.get() {
                    self.sheet.set(sheet);
                    self.scroll.set(Pos::new(0, 0));
                }
                self.set_selection(Selection {
                    anchor: end,
                    active: start,
                });
                let _ = self.dom.surface.focus();
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    // --- formatting ---

    /// The selected rectangle, as the two corners every `App` call over a range takes.
    fn rect(&self) -> (Pos, Pos) {
        self.selection.get().rect()
    }

    /// Read the active cell's style, change one field, write the whole rectangle.
    ///
    /// `App::set_style` *replaces* rather than merges, deliberately (`sheet/src/lib.rs`) — so
    /// the merge policy is here, where "make this bold as well" is a sentence about what is
    /// under the cursor rather than about every cell in the range.
    fn merge_style(&self, change: impl Fn(&mut CellStyle)) {
        let mut style = self
            .app
            .style_at(self.sheet.get(), self.selection.get().active)
            .ok()
            .flatten()
            .unwrap_or_default();
        change(&mut style);
        self.set_style_of_selection(Some(style));
    }

    fn set_style_of_selection(&self, style: Option<CellStyle>) {
        let (start, end) = self.rect();
        if let Err(error) = self.app.set_style(self.sheet.get(), start, end, style) {
            self.set_message(error.to_string());
        }
    }

    fn set_format_of_selection(&self, format: Option<numfmt::Format>) {
        let (start, end) = self.rect();
        if let Err(error) = self.app.set_format(self.sheet.get(), start, end, format) {
            self.set_message(error.to_string());
        }
    }

    /// One of the number-format presets, in the core's own vocabulary
    /// (`grind_sheet::numfmt::preset`) rather than a format-code string — this build has no
    /// such thing, which is `doc/ods-format.md` §5.2's decision and not this shell's.
    fn preset(&self, kind: Kind, decimals: u8) {
        let grouping = matches!(kind, Kind::Number | Kind::Currency) && decimals > 0;
        self.set_format_of_selection(Some(numfmt::preset(kind, decimals, grouping, CURRENCY)));
    }

    /// More or fewer decimal places, keeping whatever kind the cell already had — General
    /// becomes a plain number, which is what pressing it on an unformatted cell means.
    fn step_decimals(&self, by: i16) {
        let current = self
            .app
            .format_at(self.sheet.get(), self.selection.get().active)
            .ok()
            .flatten();
        let (kind, decimals, grouping, symbol) = match &current {
            Some(format) => format.preset_params(),
            None => (Kind::Number, 2, false, String::new()),
        };
        let decimals = (i16::from(decimals) + by).clamp(0, 10) as u8;
        let symbol = match symbol.is_empty() {
            true => CURRENCY.to_owned(),
            false => symbol,
        };
        self.set_format_of_selection(Some(numfmt::preset(kind, decimals, grouping, &symbol)));
    }

    /// A colour picked from the swatch grid — `"color"` for the text, `"fill"` for behind it.
    pub fn set_color(&self, target: &str, hex: Option<String>) {
        match target {
            "color" => self.merge_style(|s| s.color = hex.clone()),
            "fill" => self.merge_style(|s| s.background = hex.clone()),
            _ => {}
        }
    }

    /// Show what the active cell already is, on the tool row.
    pub fn refresh_tools(&self) -> Result<(), JsValue> {
        let document = &self.dom.document;
        let at = self.selection.get().active;
        let style = self
            .app
            .style_at(self.sheet.get(), at)
            .ok()
            .flatten()
            .unwrap_or_default();
        set_pressed(
            document,
            "s-bold",
            style.font_weight.as_deref() == Some("bold"),
        );
        set_pressed(
            document,
            "s-italic",
            style.font_style.as_deref() == Some("italic"),
        );
        set_pressed(document, "s-wrap", style.wrap.as_deref() == Some("wrap"));
        for (id, value) in [
            ("s-align-left", "start"),
            ("s-align-center", "center"),
            ("s-align-right", "end"),
        ] {
            set_pressed(document, id, style.align.as_deref() == Some(value));
        }
        set_swatch(document, "s-color-bar", style.color.as_deref());
        set_swatch(document, "s-fill-bar", style.background.as_deref());

        // The format `<select>` shows the preset the cell's format *is*, and General for
        // anything this vocabulary cannot spell — which is honest: choosing a preset would
        // overwrite a format the document brought and this build only knows how to display.
        let format = self.app.format_at(self.sheet.get(), at).ok().flatten();
        set_select(document, "s-format", &named_format(format.as_ref()));
        Ok(())
    }

    // --- the clipboard ---

    /// The selection as tab-separated text — the shape every spreadsheet on every platform
    /// reads, so a range copied here pastes into one of them and back.
    ///
    /// The cells' *input* text, not their displayed text: a formula copies as a formula, which
    /// is what a user who copies `=SUM(A1:A9)` means. It is also what `paste_text` feeds back
    /// to `App::enter_range`, so a round trip through the clipboard is lossless.
    pub fn clipboard_text(&self) -> Option<String> {
        let (start, end) = self.rect();
        let sheet = self.sheet.get();
        let mut out = String::new();
        for row in start.row..=end.row {
            if row > start.row {
                out.push('\n');
            }
            for col in start.col..=end.col {
                if col > start.col {
                    out.push('\t');
                }
                let text = self.app.input_text(sheet, Pos::new(row, col)).ok()?;
                // A tab or a newline *inside* a cell would be read back as a cell boundary.
                // Spaces are the lossy-but-legible answer; a quoting scheme would be a second
                // dialect of TSV that nothing else reads.
                out.push_str(&text.replace(['\t', '\n'], " "));
            }
        }
        Some(out)
    }

    /// Tab-separated text, entered as a rectangle from the active cell — one undo step,
    /// because `App::enter_range` is one action.
    pub fn paste_text(&self, text: &str) {
        let rows: Vec<Vec<String>> = text
            .replace("\r\n", "\n")
            .trim_end_matches('\n')
            .split('\n')
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect();
        if rows.is_empty() {
            return;
        }
        let anchor = self.selection.get().active;
        match self
            .app
            .enter_range(self.sheet.get(), anchor, &rows, RecalcMode::Document)
        {
            Ok(outcome) => {
                let last = Pos::new(
                    anchor.row + rows.len().saturating_sub(1) as u32,
                    anchor.col
                        + rows
                            .iter()
                            .map(Vec::len)
                            .max()
                            .unwrap_or(1)
                            .saturating_sub(1) as u32,
                );
                // The pasted rectangle is selected, with the *active* cell back where the
                // paste started: what was just pasted is highlighted, and the next thing
                // typed replaces its first cell rather than its last.
                self.set_selection(Selection {
                    anchor: last,
                    active: anchor,
                });
                self.set_message(format!("Pasted {} cell(s)", outcome.cells));
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    pub fn is_editing(&self) -> bool {
        self.editing.get()
    }

    // --- the workbook ---

    fn add_sheet(&self) {
        let taken: Vec<String> = (0..self.app.sheet_count())
            .filter_map(|i| self.app.sheet_name(i).ok())
            .collect();
        let name = (1..)
            .map(|n| format!("Sheet{n}"))
            .find(|name| !taken.iter().any(|t| t.eq_ignore_ascii_case(name)))
            .expect("there is always a free number");
        match self.app.add_sheet(&name) {
            Ok(index) => {
                let _ = self.switch_to(index);
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// `window.prompt` rather than a dialog of this shell's own: it is one line of input, the
    /// browser already has one, and a modal built here would be a second thing to keep
    /// accessible for no gain.
    fn rename_sheet(&self) {
        let sheet = self.sheet.get();
        let Ok(current) = self.app.sheet_name(sheet) else {
            return;
        };
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(Some(name)) = window.prompt_with_message_and_default("Sheet name", &current) else {
            return;
        };
        // A rename does not rewrite the formulas that name the old sheet — they go stale,
        // which `App::stale` counts. Saying so is what keeps it from being a surprise.
        if let Err(error) = self.app.rename_sheet(sheet, name.trim()) {
            self.set_message(error.to_string());
        }
    }

    fn delete_sheet(&self) {
        let sheet = self.sheet.get();
        let name = self.app.sheet_name(sheet).unwrap_or_default();
        match self.app.remove_sheet(sheet) {
            Ok(()) => {
                let _ = self.switch_to(sheet.saturating_sub(1));
                self.set_message(format!("Deleted “{name}” — Ctrl+Z brings it back"));
            }
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// Everything in the used region — Ctrl+A, and what "the whole sheet" means when the
    /// address space is a million rows of mostly nothing.
    fn select_all(&self) {
        let (rows, cols) = self.app.used_extent(self.sheet.get()).unwrap_or((1, 1));
        self.set_selection(Selection {
            anchor: Pos::new(0, 0),
            active: Pos::new(rows.saturating_sub(1), cols.saturating_sub(1)),
        });
    }

    /// Replicate the top row — or the left column — across the selection, which is what
    /// `App::fill` already is.
    fn fill(&self, down: bool) {
        let (start, end) = self.rect();
        if (down && end.row == start.row) || (!down && end.col == start.col) {
            return self.set_message("Select the cells to fill into as well".to_owned());
        }
        let from = match down {
            true => Pos::new(start.row + 1, start.col),
            false => Pos::new(start.row, start.col + 1),
        };
        match self
            .app
            .fill(self.sheet.get(), start, from, end, RecalcMode::Document)
        {
            Ok(outcome) => self.set_message(format!("Filled {} cell(s)", outcome.cells)),
            Err(error) => self.set_message(error.to_string()),
        }
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

    /// Turn one of `doc/view-modes.md`'s overlays on or off, and say which.
    ///
    /// The message is not a nicety: role mode suppresses the document's own colours (§4.5),
    /// which is the right call and still a surprise, and §9 asks for the mode to say it is
    /// on somewhere a reader will see it.
    fn overlay(&self, roles: bool) {
        let mut overlays = self.overlays.get();
        let on = match roles {
            true => {
                overlays.roles = !overlays.roles;
                overlays.roles
            }
            false => {
                overlays.names = !overlays.names;
                overlays.names
            }
        };
        self.overlays.set(overlays);
        let what = match roles {
            true => "Cell colours say what each cell is",
            false => "Names are shown where they live",
        };
        self.set_message(match on {
            true => format!("{what} — nothing was written; run it again to stop"),
            false => "Back to the document's own colours".to_owned(),
        });
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

/// The border this shell draws when asked for one.
///
/// LibreOffice's own hairline, in the three-part form ODF stores (`doc/ods-format.md` §5.4) —
/// so a box drawn here is the box a document already full of them has, rather than a second
/// weight nothing else uses.
const BORDER: &str = "0.06pt solid #000000";

/// What `format.currency` spells. A gap, and a named one: the core carries the symbol a
/// document chose and this shell has no locale to pick one from, so it offers the one that is
/// unambiguous rather than guessing at the reader's.
const CURRENCY: &str = "¤";

/// Turn a property on, or — when it is already that value — off. What a *toggle* means, as
/// opposed to a value a picker sets.
fn toggle(field: &mut Option<String>, value: &str) {
    *field = match field.as_deref() == Some(value) {
        true => None,
        false => Some(value.to_owned()),
    };
}

fn set(field: &mut Option<String>, value: &str) {
    *field = Some(value.to_owned());
}

/// Which of the format `<select>`'s options a cell's format *is* — the command id, so the
/// toolbar reports in the same vocabulary it commands in.
///
/// A format the preset vocabulary cannot spell (`DD.MM.YYYY`, a document's own) reports as
/// General, because none of the options is it: this build displays such a format faithfully
/// and has no name for it (`Format::is_preset`).
fn named_format(format: Option<&numfmt::Format>) -> String {
    let Some(format) = format else {
        return "format.general".to_owned();
    };
    if !format.is_preset() {
        return "format.general".to_owned();
    }
    let (kind, decimals, _, _) = format.preset_params();
    // "Date and time" is not a `Kind` of its own — it is a `Date` format with the time's own
    // parts appended (`numfmt::datetime_preset`), so the two are told apart by what is in it.
    let has_time = format
        .parts
        .iter()
        .any(|part| matches!(part, numfmt::Part::Hours { .. }));
    match kind {
        Kind::Number if decimals == 0 => "format.integer",
        Kind::Number => "format.number",
        Kind::Percentage => "format.percent",
        Kind::Currency => "format.currency",
        Kind::Date if has_time => "format.datetime",
        Kind::Date => "format.date",
        Kind::Time => "format.time",
        _ => "format.general",
    }
    .to_owned()
}

/// A `CellStyle` as inline CSS.
///
/// ODF's style vocabulary is CSS's here, near enough to pass values through
/// verbatim: `fo:font-weight`, `fo:color` and `fo:border` are spelled the way CSS
/// spells them, which is not a coincidence — both took them from XSL. The values
/// stay exactly as the document wrote them (doc/ods-format.md's rule), so anything
/// this shell does not understand is passed to the browser rather than dropped.
/// Whether this cell is the one that carries its anchor's hint — `doc/view-modes.md` §3.2's
/// "drawn once".
///
/// A name over `A2:A50` is one anchor, not forty-nine, and scrolling must move the hint
/// rather than lose it: it goes on the first cell of the anchor that is actually on screen.
/// Rows the document says are not there — filtered or hidden — are skipped, because a hint
/// drawn into one of those is a hint nobody sees.
fn hint_here(viewport: &grind_sheet::Viewport, hidden: &[u32], row: u32, col: u32) -> bool {
    viewport.names().iter().any(|anchor| {
        let first_col = anchor.cols.start.max(viewport.cols.start);
        let first_row = (anchor.rows.start.max(viewport.rows.start)..anchor.rows.end)
            .find(|r| !hidden.contains(r));
        first_row == Some(row) && first_col == col
    })
}

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
        click.dragging.set(true);
        if let Err(error) = click.on_click(&event) {
            web_sys::console::error_1(&error);
        }
    })?;

    // Dragging a rectangle out: every move with the button down extends the
    // selection, which is the gesture every grid has and this one did not.
    let drag = ui.clone();
    listen(&ui.dom.surface, "mousemove", move |event: MouseEvent| {
        if !drag.dragging.get() || drag.editing.get() {
            return;
        }
        let Some(pos) = cell_at(&event) else { return };
        let anchor = drag.selection.get().anchor;
        if drag.selection.get().active != pos {
            drag.set_selection(Selection {
                anchor,
                active: pos,
            });
        }
    })?;

    // On the *window*, not the surface: a drag that ends outside it still ends.
    if let Some(window) = web_sys::window() {
        let release = ui.clone();
        listen(&window, "mouseup", move |_: MouseEvent| {
            release.dragging.set(false);
        })?;
    }

    let tabs = ui.clone();
    listen(&ui.dom.tabs, "click", move |event: MouseEvent| {
        if let Err(error) = tabs.on_click(&event) {
            web_sys::console::error_1(&error);
        }
    })?;

    // Double-clicking a tab renames the sheet it names — the gesture every
    // spreadsheet's tab bar has, and the reason there is no rename button.
    let rename = ui.clone();
    listen(&ui.dom.tabs, "dblclick", move |event: MouseEvent| {
        rename.run("sheet.rename");
        event.prevent_default();
    })?;

    // The address box: type a cell, a range or a name and go there.
    let address = ui.clone();
    listen(&ui.dom.address, "keydown", move |event: KeyboardEvent| {
        match event.key().as_str() {
            "Enter" => {
                event.prevent_default();
                let where_ = address.dom.address.value();
                address.go_to(where_.trim());
            }
            "Escape" => {
                let _ = address.dom.surface.focus();
            }
            _ => {}
        }
        // Every other key belongs to the input, including the arrows — this is a
        // text field, and the grid's own keymap must not reach into it.
        event.stop_propagation();
    })?;

    let wheel = ui.clone();
    listen(&ui.dom.surface, "wheel", move |event: WheelEvent| {
        wheel.on_wheel(&event);
    })
}

/// The cell a pointer event landed on, from the address the cell carries.
fn cell_at(event: &MouseEvent) -> Option<Pos> {
    let target = event.target()?.dyn_into::<Element>().ok()?;
    let cell = target.closest("td.cell").ok()??;
    Some(Pos::new(
        attribute(&cell, "data-row")?,
        attribute(&cell, "data-col")?,
    ))
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
