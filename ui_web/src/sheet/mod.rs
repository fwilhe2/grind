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

mod assist;
mod chart;
mod filter_ui;
pub mod keymap;
mod layout;

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use grind_core::utf16;
use grind_sheet::find::{self, Search, Towards};
use grind_sheet::format::{self, Toggle};
use grind_sheet::formula::{display, lex};
use grind_sheet::numfmt::{self, Kind};
use grind_sheet::style::{CellStyle, EDGES};
use grind_sheet::{App, CellValue, Filter, Form, Pos, RecalcMode, a1, csv};
use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Element, Event, HtmlButtonElement, HtmlElement, HtmlInputElement, KeyboardEvent,
    MouseEvent, WheelEvent,
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
    /// The assist band under the formula bar — autocomplete offers or a signature hint
    /// (`assist.rs`).
    assist: HtmlElement,
    tabs: HtmlElement,
    /// The layer charts float in, over the cells they sit above.
    charts: HtmlElement,
    message: HtmlElement,
    summary: HtmlElement,
    /// The autofilter's dropdown (§9.4, `filter_ui.rs`): one popover, reused for every field
    /// and moved under whichever button opened it — the same technique `swatch.rs` uses.
    filter_menu: HtmlElement,
    filter_list: Element,
    filter_all: HtmlInputElement,
    filter_clear: HtmlButtonElement,
    filter_apply: HtmlButtonElement,
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
            assist: element(document, "assist")?,
            tabs: element(document, "tabs")?,
            charts: element(document, "charts")?,
            message: element(document, "message")?,
            summary: element(document, "summary")?,
            filter_menu: element(document, "filter-menu")?,
            filter_list: element(document, "filter-list")?,
            filter_all: element(document, "filter-select-all")?,
            filter_clear: element(document, "filter-clear")?,
            filter_apply: element(document, "filter-apply")?,
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
    /// The reference being pointed at while a formula is typed (point mode): where its text is in
    /// the formula bar, as bytes, and the cell it names.
    pointing: RefCell<Option<(std::ops::Range<usize>, Pos)>>,
    scroll: Cell<Pos>,
    editing: Cell<bool>,
    /// Whether the pointer is down and dragging a rectangle out.
    dragging: Cell<bool>,
    /// The formula assist band's own state — autocomplete offers or a signature hint,
    /// recomputed from the formula bar's text and caret on every change (`assist.rs`).
    assist: RefCell<assist::Assist>,
    /// Which of `doc/view-modes.md`'s overlays this pane draws. Presentation state, like
    /// everything else here: a view mode is a reading of the document and never a change to
    /// it, so turning one off puts the page back exactly.
    overlays: Cell<grind_sheet::view::Overlays>,
    message: RefCell<String>,
    /// The word the last cell picked from the palette was found by — what F3 and Shift+F3
    /// step through, and what *Replace in every cell…* offers to replace.
    needle: RefCell<String>,
    /// Which field the autofilter's popover is open for, when it is open (`filter_ui.rs`).
    /// `None` *is* "closed" — one fact rather than two that can disagree.
    filter_field: Cell<Option<u32>>,
    /// The checkbox behind each value the popover currently lists, so `Ui::filter_ticked` can
    /// read them back without a DOM query — the same cache `ui_sheet_gtk`'s `FilterMenu` keeps.
    filter_checks: RefCell<Vec<(String, HtmlInputElement)>>,
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
            pointing: RefCell::new(None),
            scroll: Cell::new(Pos::new(0, 0)),
            editing: Cell::new(false),
            dragging: Cell::new(false),
            assist: RefCell::new(assist::Assist::default()),
            overlays: Cell::new(grind_sheet::view::Overlays::NONE),
            message: RefCell::new(String::new()),
            needle: RefCell::new(String::new()),
            filter_field: Cell::new(None),
            filter_checks: RefCell::new(Vec::new()),
        });
        wire_grid(&ui)?;
        wire_editor(&ui)?;
        wire_filter_menu(&ui)?;
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
        self.assist.borrow_mut().clear();
        self.close_filter_menu();
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
        // The autofilter, read once and reused for the heading row's dropdown buttons below
        // (§9.4, `filter_ui.rs`).
        let filter = self.filter();

        let selection = self.selection.get();
        let editing = self.editing.get();
        let dark = crate::ink::page_is_dark();

        // The column widths the document chose, as `<col>` elements: one declaration per
        // column, which is what `table-layout: fixed` sizes from.
        self.dom.cols.set_text_content(None);
        let corner_col = self.dom.document.create_element("col")?;
        corner_col.set_attribute("style", "width:3.5rem")?;
        self.dom.cols.append_child(&corner_col)?;
        //
        // A hidden column is **left out** of all three — the `<col>`, the header and the cells —
        // rather than given a width of nothing. A zero-width `<col>` was what this did, and a
        // `max-content` table sizes a column from what is in it, so the column the document
        // hid was drawn at full width with its heading and its values in it. Leaving it out is
        // the column's own answer to a filtered row's `display: none`.
        let shown: Vec<u32> = cols
            .clone()
            .filter(|col| !hidden_cols.contains(col))
            .collect();
        for &col in &shown {
            let declaration = self.dom.document.create_element("col")?;
            declaration.set_attribute("style", &format!("width:{:.1}px", widths.size(col)))?;
            self.dom.cols.append_child(&declaration)?;
        }
        // **The table is as wide as its columns add up to**, stated. `table-layout: fixed` sizes
        // columns from the `<col>`s only when the table has a width of its own; with the
        // stylesheet's `max-content` the browser sized them from what was *in* them, so a column
        // the document made 0.6in wide grew to fit a long label, and a number too wide for its
        // column was never too wide — every other shell drew the document's width and this one
        // quietly did not. With it, a cell's content is clipped at the document's width, and a
        // number that no longer fits is `###` (`hash_overflowing`).
        let total: f64 = shown.iter().map(|&col| widths.size(col)).sum();
        if let Some(table) = self.dom.cols.parent_element() {
            table.set_attribute("style", &format!("width:calc(3.5rem + {total:.1}px)"))?;
        }

        // Column headers. Rebuilt with the body, because a horizontal scroll
        // changes which letters are over which cells.
        self.dom.head.set_text_content(None);
        let corner = self.corner()?;
        self.dom.head.append_child(&corner)?;
        for &col in &shown {
            let cell = self.dom.document.create_element("th")?;
            cell.set_class_name(
                match selection.contains(Pos::new(selection.active.row, col)) {
                    true => "head col current",
                    false => "head col",
                },
            );
            // The one trace a hidden run leaves: an accent edge on the heading after it, with
            // a tooltip saying what is there and how to have it back.
            if col > 0 && hidden_cols.contains(&(col - 1)) {
                cell.class_list().add_1("after-hidden")?;
                cell.set_attribute(
                    "title",
                    &format!(
                        "Column {} is hidden — Ctrl+K, Unhide columns",
                        lex::column_name(col - 1)
                    ),
                )?;
            }
            cell.set_attribute("data-col", &col.to_string())?;
            cell.set_text_content(Some(&lex::column_name(col)));
            self.dom.head.append_child(&cell)?;
        }

        // One element per visible cell, thrown away and rebuilt each frame. The
        // cost is bounded by the window rather than the document, which is what the
        // viewport is for.
        self.dom.body.set_text_content(None);
        // Every number drawn, to be checked once the rows are in the page — only then has the
        // browser laid them out and can say whether one overflows (`hash_overflowing`).
        let mut numbers: Vec<web_sys::Element> = Vec::new();
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

            for &col in &shown {
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
                if numeric && !(active && editing) {
                    numbers.push(cell.clone());
                }
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
                        let css = css_of(viewport.style(row, col), numeric, dark);
                        if !css.is_empty() {
                            cell.set_attribute("style", &css)?;
                        }
                    }
                }
                if overlays.names {
                    if let Some(name) = viewport.name_at(row, col)
                        && hint_here(&viewport, &hidden, row, col)
                    {
                        cell.set_attribute("data-name", name)?;
                    }
                    let edges = anchor_edges(&viewport, row, col);
                    if !edges.is_empty() {
                        cell.set_attribute("data-anchor", &edges)?;
                    }
                }
                // The autofilter's dropdown button (§9.4): one per field, on the range's
                // own heading row — appended last, since `set_text_content` above would
                // have thrown it away. `table:display-filter-buttons="false"` is honoured:
                // the document asked for no buttons, and `run("sheet.filter")` still reaches
                // the filter itself.
                if let Some(filter) = &filter
                    && filter.buttons
                    && row == filter.start.row
                    && (filter.start.col..=filter.end.col).contains(&col)
                {
                    let field = col - filter.start.col;
                    // Room for the button at the cell's end, so the heading is clipped before
                    // it rather than running underneath (`Differenc▾e`).
                    cell.class_list().add_1("has-filter")?;
                    let button = self.dom.document.create_element("button")?;
                    button.set_attribute("type", "button")?;
                    button.set_class_name(match filter.keep.contains_key(&field) {
                        true => "filter-btn on",
                        false => "filter-btn",
                    });
                    button.set_attribute("data-field", &field.to_string())?;
                    button.set_attribute("title", "Filter this column")?;
                    let label = self.dom.document.create_element("span")?;
                    label.set_class_name("sr");
                    label.set_text_content(Some("Filter this column"));
                    button.append_child(&label)?;
                    cell.append_child(&button)?;
                }
                line.append_child(&cell)?;
            }
            self.dom.body.append_child(&line)?;
        }
        hash_overflowing(&numbers)?;

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

        let (start, end) = selection.rect();
        self.dom
            .summary
            .set_text_content(Some(&summary(&self.app, sheet, start, end)));

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
        // The assist band gets first refusal on Tab, the arrows and Escape while it is
        // offering — the same three keys `ui_win32`'s band claims, and for the same reason:
        // everything else stays with the `<input>`.
        let offering = self.editing.get() && self.assist.borrow().is_offering();
        if let Some(reply) = assist::on_key(
            offering,
            &key,
            event.ctrl_key() || event.meta_key(),
            event.alt_key(),
            event.shift_key(),
        ) {
            event.prevent_default();
            let result = match reply {
                assist::Reply::Accept => self.accept_assist(),
                assist::Reply::Step(delta) => {
                    self.assist.borrow_mut().step(delta);
                    self.render_assist()
                }
                assist::Reply::Dismiss => {
                    self.assist.borrow_mut().dismiss();
                    self.render_assist()
                }
            };
            if let Err(error) = result {
                web_sys::console::error_1(&error);
            }
            return;
        }
        // Point mode: where a reference could go, an arrow points at a cell and writes its address
        // (`assist::ref_eligible`, as the GNOME and Mac windows do); anything else ends it.
        let plain = !(event.ctrl_key() || event.meta_key() || event.alt_key() || event.shift_key());
        if self.editing.get()
            && plain
            && matches!(
                key.as_str(),
                "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight"
            )
            && self.point_at(&key).unwrap_or(false)
        {
            event.prevent_default();
            return;
        }
        if !matches!(key.as_str(), "Shift" | "Control" | "Alt" | "Meta") {
            self.pointing.replace(None);
        }
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

    /// An arrow in the formula bar as point mode: start a reference one cell off the cell being
    /// edited, or move the one being pointed at. `Ok(false)` when a reference could not go here,
    /// and the arrow is the caret's.
    fn point_at(&self, key: &str) -> Result<bool, JsValue> {
        let text = self.dom.formula.value();
        let units = self.dom.formula.selection_start()?.unwrap_or(0) as usize;
        let caret = utf16::byte_of(&text, units);
        let pending = self.pointing.borrow().clone();
        if pending.is_none() && !grind_sheet::formula::assist::ref_eligible(&text, caret) {
            return Ok(false);
        }
        let from = pending
            .as_ref()
            .map_or(self.selection.get().active, |(_, cell)| *cell);
        let cell = match key {
            "ArrowUp" => Pos::new(from.row.saturating_sub(1), from.col),
            "ArrowDown" => Pos::new((from.row + 1).min(grind_sheet::MAX_ROWS - 1), from.col),
            "ArrowLeft" => Pos::new(from.row, from.col.saturating_sub(1)),
            _ => Pos::new(from.row, (from.col + 1).min(grind_sheet::MAX_COLS - 1)),
        };
        let reference = grind_sheet::a1::format(None, cell);
        let span = pending.map_or(caret..caret, |(span, _)| span);
        let end = span.start + reference.len();
        let mut next = text.clone();
        next.replace_range(span.clone(), &reference);
        self.dom.formula.set_value(&next);
        let at = utf16::units_before(&next, end) as u32;
        self.dom.formula.set_selection_range(at, at)?;
        self.pointing.replace(Some((span.start..end, cell)));
        self.request_repaint();
        self.refresh_assist()?;
        Ok(true)
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
        let style = |write: fn(&CellStyle) -> Option<CellStyle>| self.merge_style(write);
        match id {
            "sheet.recalc" => self.recalc(),
            "view.roles" => self.overlay(true),
            "view.names" => self.overlay(false),
            "edit.clear" => self.clear(),
            "edit.fill-down" => self.fill(true),
            "edit.fill-right" => self.fill(false),
            "edit.select-all" => self.select_all(),
            "sheet.hide-rows" => self.hide_rows(true),
            "sheet.unhide-rows" => self.hide_rows(false),
            "sheet.hide-cols" => self.hide_cols(true),
            "sheet.unhide-cols" => self.hide_cols(false),
            "sheet.row-height" => self.track_size(true),
            "sheet.col-width" => self.track_size(false),
            "name.define" => self.define_name(),
            "name.rename" => self.rename_name(),
            "name.inline" => self.inline_name(),
            "name.delete" => self.delete_name(),
            "edit.evaluate" => self.evaluate(),
            "edit.explain" => self.explain(),
            "chart.insert" => self.insert_chart(),
            "chart.delete" => self.delete_chart(),
            "edit.fill-across" => self.fill_across(),
            "edit.formula-to-value" => self.formula_to_value(),
            "sheet.filter" => self.toggle_filter(),
            "sheet.format-table" => self.format_table(None),
            "sheet.format-table-totals" => self.format_table_with_totals(),

            "style.bold" => style(|s| Toggle::Bold.flipped(s)),
            "style.italic" => style(|s| Toggle::Italic.flipped(s)),
            // The alignments *set* rather than flip here, with a command of their own to clear
            // them, because a palette row has no pressed state to flip from.
            "style.align-left" => style(|s| Toggle::AlignStart.set(s, true)),
            "style.align-center" => style(|s| Toggle::AlignCenter.set(s, true)),
            "style.align-right" => style(|s| Toggle::AlignEnd.set(s, true)),
            "style.align-clear" => style(|s| format::restyled(s, |s| s.align = None)),
            "style.wrap" => style(|s| Toggle::Wrap.flipped(s)),
            "style.border" => style(|s| {
                format::restyled(s, |s| {
                    s.set_border(Some(grind_sheet::format::BORDER.to_owned()))
                })
            }),
            "style.border-clear" => style(|s| format::restyled(s, |s| s.set_border(None))),
            "style.clear" => self.set_style_of_selection(None),

            "format.general" => self.set_format_of_selection(None),
            "format.integer" => self.preset(Kind::Number, 0),
            "format.number" => self.preset(Kind::Number, 2),
            "format.percent" => self.preset(Kind::Percentage, 0),
            "format.currency" => self.currency(numfmt::DEFAULT_CURRENCY),
            "format.currency-usd" => self.currency("$"),
            "format.currency-gbp" => self.currency("£"),
            "format.date" => self.preset(Kind::Date, 0),
            "format.time" => self.preset(Kind::Time, 0),
            "format.datetime" => self.set_format_of_selection(Some(numfmt::datetime_preset())),
            "format.more" => self.step_decimals(1),
            "format.fewer" => self.step_decimals(-1),

            "edit.find-next" => self.find_step(Towards::Next),
            "edit.find-previous" => self.find_step(Towards::Previous),
            "edit.replace" => self.replace(),

            "sheet.add" => self.add_sheet(),
            "sheet.rename" => self.rename_sheet(),
            "sheet.delete" => self.delete_sheet(),

            id => match (
                id.strip_prefix("goto:"),
                id.strip_prefix("find:"),
                id.strip_prefix("fn:"),
            ) {
                (Some(where_), _, _) => self.go_to(where_),
                (_, Some(found), _) => self.pick_found(found),
                (_, _, Some(name)) => {
                    if let Err(error) = self.begin_with(&format!("={name}(")) {
                        web_sys::console::error_1(&error);
                    }
                }
                _ => self.set_message(format!("No such command: {id}")),
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

    // --- the function list ---

    /// What the palette offers for a query that names a function: its name or its plain-English
    /// name contains the query (`grind sheet functions --long`'s four columns, from the same
    /// catalog), five rows at most. Picking one starts an edit seeded `=NAME(` with the caret
    /// where the first argument goes — the Windows window's *Function List* in the box that was
    /// already here.
    pub fn functions(&self, query: &str) -> Vec<Entry> {
        let query = query.trim().to_uppercase();
        if query.chars().count() < 2 {
            return Vec::new();
        }
        grind_sheet::formula::funcs::catalog()
            .iter()
            .filter_map(|info| {
                let friendly = grind_sheet::formula::friendly::signature(info.name)
                    .map(|(head, _)| head)
                    .unwrap_or_else(|| info.name.to_owned());
                (info.name.to_uppercase().contains(&query)
                    || friendly.to_uppercase().contains(&query))
                .then(|| {
                    Entry::target(
                        format!("fn:{}", info.name),
                        format!("{friendly} — {}", info.brief),
                        grind_sheet::formula::funcs::category(info),
                    )
                })
            })
            .take(5)
            .collect()
    }

    // --- find and replace (`grind_sheet::find`) ---

    /// What the palette offers *after* the verbs for a query: the cells holding it.
    ///
    /// After, not before like [`Ui::targets`] — a word somebody types is far more often a verb
    /// than a cell's contents, and `bold` must still put *Bold* first however many cells say
    /// "bold". Two characters at least, since one letter is in half the sheet. Each row is a
    /// cell; when there are more than fit, the last row is all of them, stepped through with F3.
    ///
    /// This is the browser's whole find UI, and it is the palette's go-to rule applied once
    /// more: **one box** rather than a find bar, which this page has no surface for. It is also
    /// why Ctrl+F opens that box — the browser's own find can only see the cells on screen.
    pub fn found(&self, query: &str) -> Vec<Entry> {
        const SHOWN: usize = 5;
        let query = query.trim();
        if query.chars().count() < 2 {
            return Vec::new();
        }
        let Ok(hits) = self.app.find(&Search::new(query)) else {
            return Vec::new();
        };
        let mut out: Vec<Entry> = hits
            .iter()
            .take(SHOWN)
            .map(|hit| {
                let text: String = hit.text.chars().take(60).collect();
                Entry::target(
                    format!("find:{}\n{query}", hit.address()),
                    format!("{} — {text}", hit.address()),
                    "Cell",
                )
            })
            .collect();
        if hits.len() > SHOWN {
            out.push(Entry::target(
                format!("find:\n{query}"),
                format!(
                    "All {} cells holding “{query}” — F3 steps through them",
                    hits.len()
                ),
                "Find",
            ));
        }
        out
    }

    /// A cell picked from [`Ui::found`]: go there and remember the word, so F3 carries on.
    /// No address means "the first hit from here", which is the *All N cells* row.
    fn pick_found(&self, found: &str) {
        let Some((address, needle)) = found.split_once('\n') else {
            return;
        };
        *self.needle.borrow_mut() = needle.to_owned();
        if address.is_empty() {
            self.find_step(Towards::Here);
        } else {
            self.go_to(address);
            self.say_where();
        }
    }

    /// F3 and Shift+F3: the next or previous cell holding the remembered word, across every
    /// sheet, wrapping at either end. The hits are asked for again each time, since the
    /// document may have changed since the last press.
    fn find_step(&self, towards: Towards) {
        let needle = self.needle.borrow().clone();
        if needle.is_empty() {
            return self.set_message("Nothing to find yet — Ctrl+F, then type".to_owned());
        }
        let hits = self.hits(&needle);
        let here = (self.sheet.get(), self.selection.get().active);
        let Some(index) = find::step(&hits, here, towards) else {
            return self.set_message(format!("No cell holds “{needle}”"));
        };
        let (sheet, pos) = hits[index];
        if sheet != self.sheet.get() {
            self.sheet.set(sheet);
            self.scroll.set(Pos::new(0, 0));
        }
        self.set_selection(Selection::at(pos));
        let _ = self.dom.surface.focus();
        self.set_message(format!(
            "{} of {} · F3 next, Shift+F3 previous",
            index + 1,
            hits.len()
        ));
    }

    /// "3 of 7" for the cell the selection is on, after a jump that was not a step.
    fn say_where(&self) {
        let needle = self.needle.borrow().clone();
        let hits = self.hits(&needle);
        let here = (self.sheet.get(), self.selection.get().active);
        if let Some(index) = hits.iter().position(|hit| *hit == here) {
            self.set_message(format!(
                "{} of {} · F3 next, Shift+F3 previous",
                index + 1,
                hits.len()
            ));
        }
    }

    fn hits(&self, needle: &str) -> Vec<(usize, Pos)> {
        self.app
            .find(&Search::new(needle))
            .map(|hits| hits.into_iter().map(|hit| (hit.sheet, hit.pos)).collect())
            .unwrap_or_default()
    }

    /// *Replace in every cell…* — two questions, then `App::replace` over every sheet in one
    /// undo step. Prompts rather than a form, the way *Rename this sheet…* already asks: this
    /// page has no dialog surface, and a replace is two words.
    fn replace(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let offered = self.needle.borrow().clone();
        let Ok(Some(what)) = window.prompt_with_message_and_default("Replace what?", &offered)
        else {
            return;
        };
        if what.is_empty() {
            return;
        }
        let Ok(Some(with)) =
            window.prompt_with_message_and_default(&format!("Replace “{what}” with"), "")
        else {
            return;
        };
        *self.needle.borrow_mut() = what.clone();
        match self
            .app
            .replace(&Search::new(what.as_str()), &with, RecalcMode::Document)
        {
            Ok(done) => self.set_message(replaced(&what, &done)),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    // --- the code view (doc/dsl.md §6, D9) ---

    /// The document as its projection, for the code view.
    pub fn project(&self) -> grind_sheet::projection::Projection {
        self.app.project()
    }

    /// Where the selection is, spelled the way the span map spells it — sheet-qualified,
    /// because two sheets have an `A1` and the map has to tell them apart.
    pub fn projection_address(&self) -> Option<String> {
        let name = self.app.sheet_name(self.sheet.get()).ok()?;
        Some(a1::format(Some(&name), self.selection.get().active))
    }

    /// Select whatever a code-view line projects.
    ///
    /// A **sheet's own name** is checked first, because a `sheet` node anchors one and a bare
    /// name is also a perfectly good cell address — `Sheet1` parses as column `SHEET`, row 1,
    /// and answering a click on `sheet Sales {` with a jump to a cell nobody has used would be
    /// worse than doing nothing. The palette's own spelling for *that sheet* is a trailing dot,
    /// so this is the one place the two vocabularies meet.
    /// What the document says about itself (`doc/dsl.md` §4.3, D6) — the core's rules, and
    /// nothing decided here.
    pub fn lint(&self) -> grind_core::lint::Report {
        self.app.lint(&grind_core::lint::Options::default())
    }

    pub fn select_projected(&self, address: &str) {
        match a1::sheet(&self.app, address) {
            Ok(_) => self.go_to(&format!("{address}.")),
            Err(_) => self.go_to(address),
        }
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
    /// the merge is here, where "make this bold as well" is a sentence about what is under the
    /// cursor rather than about every cell in the range. `write` is one of
    /// `grind_sheet::format`'s, which answer `None` for a style that sets nothing: un-bolding the
    /// only bold cell used to leave an empty `style:style` behind.
    fn merge_style(&self, write: impl Fn(&CellStyle) -> Option<CellStyle>) {
        let style = self
            .app
            .style_at(self.sheet.get(), self.selection.get().active)
            .ok()
            .flatten()
            .unwrap_or_default();
        self.set_style_of_selection(write(&style));
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

    /// A currency with two decimals and grouping, in the symbol a command named — one command
    /// per [`numfmt::CURRENCIES`] entry, so none of them has to be typed.
    fn currency(&self, symbol: &str) {
        self.set_format_of_selection(Some(numfmt::preset(Kind::Currency, 2, true, symbol)));
    }

    /// More or fewer decimal places — `grind_sheet::format::stepped`, which keeps whatever
    /// kind the cell already had and starts a plain cell from the decimals it *shows*. A date,
    /// text, or a format this build did not write has no decimals to step, and nothing is
    /// written: this used to turn a date into a number format with decimals.
    fn step_decimals(&self, by: i8) {
        let (sheet, at) = (self.sheet.get(), self.selection.get().active);
        let current = self.app.format_at(sheet, at).ok().flatten();
        let shown = self.app.value_text(sheet, at).unwrap_or_default();
        let shown = format::decimals_shown(&shown, self.app.locale().as_ref());
        if let Some(stepped) = format::stepped(current.as_ref(), by, shown, None) {
            self.set_format_of_selection(Some(stepped));
        }
    }

    /// A colour picked from the swatch grid — `"color"` for the text, `"fill"` for behind it.
    pub fn set_color(&self, target: &str, hex: Option<String>) {
        match target {
            "color" => self.merge_style(|s| format::coloured(s, false, hex.clone())),
            "fill" => self.merge_style(|s| format::coloured(s, true, hex.clone())),
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
        for (id, toggle) in [
            ("s-bold", Toggle::Bold),
            ("s-italic", Toggle::Italic),
            ("s-wrap", Toggle::Wrap),
            ("s-align-left", Toggle::AlignStart),
            ("s-align-center", Toggle::AlignCenter),
            ("s-align-right", Toggle::AlignEnd),
        ] {
            set_pressed(document, id, toggle.is_on(&style));
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
    /// reads, so a range copied here pastes into one of them and back. The codec is
    /// `grind_sheet::clip`'s, shared with every other shell's clipboard.
    ///
    /// The cells' *input* text, not their displayed text: a formula copies as a formula, which
    /// is what a user who copies `=SUM(A1:A9)` means. It is also what `paste_text` feeds back
    /// to `App::enter_range`, so a round trip through the clipboard is lossless.
    pub fn clipboard_text(&self) -> Option<String> {
        let (start, end) = self.rect();
        let text = grind_sheet::clip::rect_text(
            &self.app,
            self.sheet.get(),
            start,
            end,
            App::input_text,
            "\n",
        );
        Some(text)
    }

    /// The selection as it is *shown* — a formula's formatted result rather than its source
    /// (`App::value_text`), for pasting into something that is not a spreadsheet.
    pub fn value_text(&self) -> String {
        let (start, end) = self.rect();
        grind_sheet::clip::rect_text(
            &self.app,
            self.sheet.get(),
            start,
            end,
            App::value_text,
            "\n",
        )
    }

    /// The cell the selection grew from into the whole selection, references shifted — one
    /// `App::fill`.
    fn fill_across(&self) {
        let (start, end) = self.rect();
        match self.app.fill(
            self.sheet.get(),
            // The cell the selection grew from, which is the one somebody means.
            self.selection.get().anchor,
            start,
            end,
            RecalcMode::Document,
        ) {
            Ok(outcome) => self.set_message(format!("Filled {} cell(s)", outcome.cells)),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// Every formula in the selection dropped, each cell keeping the value it last computed.
    fn formula_to_value(&self) {
        let (start, end) = self.rect();
        match grind_sheet::verbs::formulas_to_values(&self.app, self.sheet.get(), start, end) {
            Ok(0) => self.set_message("There was no formula in the selection".to_owned()),
            Ok(n) => self.set_message(format!("{n} formula(s) are now plain values")),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// Tab-separated text, entered as a rectangle from the active cell — one undo step,
    /// because `App::enter_range` is one action.
    pub fn paste_text(&self, text: &str) {
        // Display syntax back to ODF's (`grind_sheet::clip`) — without it a formula copied here
        // pasted back as `#NAME?`.
        let rows = grind_sheet::clip::parse_rows(text);
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
        let name = self.app.fresh_sheet_name();
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
        // A rename carries every reference with it (`doc/dsl.md` §6.5, D10) — formulas, named
        // expressions, chart ranges — in one undo step. Saying how many is what makes a
        // document-wide edit visible.
        match self.app.rename_sheet(sheet, name.trim()) {
            Ok(0) => {}
            Ok(rewritten) => self.set_message(format!("{rewritten} reference(s) rewritten")),
            Err(error) => self.set_message(error.to_string()),
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

    /// Replicate each column's top cell down the selection — or each row's left cell across it —
    /// which is what `App::fill` already is, over `grind_sheet::nav::fills` (the same lines the
    /// Mac and the Windows grid fill). It used to copy one cell over the whole rectangle, which
    /// made a two-column fill write column A's formulas into column B.
    fn fill(&self, down: bool) {
        let (start, end) = self.rect();
        let lines = grind_sheet::nav::fill_lines(start, end, down);
        if lines.is_empty() {
            return self.set_message(match down {
                true => "Select the cells to fill into as well — more than one row".to_owned(),
                false => "Select the cells to fill into as well — more than one column".to_owned(),
            });
        }
        let mut cells = 0;
        for (source, start, end) in lines {
            match self
                .app
                .fill(self.sheet.get(), source, start, end, RecalcMode::Document)
            {
                Ok(outcome) => cells += outcome.cells,
                Err(error) => return self.set_message(error.to_string()),
            }
        }
        self.set_message(format!("Filled {cells} cell(s)"));
    }

    /// A selection in the core's own type, for `grind_sheet::verbs`.
    fn nav_selection(&self) -> grind_sheet::nav::Selection {
        let selection = self.selection.get();
        grind_sheet::nav::Selection {
            anchor: selection.anchor,
            active: selection.active,
        }
    }

    /// A window prompt, or `None` when there is no window or the question was cancelled.
    fn ask(&self, message: &str, default: &str) -> Option<String> {
        web_sys::window()?
            .prompt_with_message_and_default(message, default)
            .ok()
            .flatten()
    }

    /// *Row height…* / *Column width…* — a length (`2.5cm`, `1in`, `64pt`), set on every selected
    /// row or column in one undo step; empty puts the default back. `App` checks the length.
    fn track_size(&self, rows: bool) {
        let Some(answer) = self.ask(
            match rows {
                true => "Row height — 2.5cm, 1in or 64pt; empty for the default",
                false => "Column width — 2.5cm, 1in or 64pt; empty for the default",
            },
            "",
        ) else {
            return;
        };
        let size = Some(answer.trim().to_owned()).filter(|size| !size.is_empty());
        let selection = self.nav_selection();
        let result = match rows {
            true => {
                self.app
                    .set_row_height(self.sheet.get(), grind_sheet::verbs::rows(selection), size)
            }
            false => {
                self.app
                    .set_col_width(self.sheet.get(), grind_sheet::verbs::cols(selection), size)
            }
        };
        self.set_message(match result {
            Ok(0) => "That was the size already".to_owned(),
            Ok(n) => format!(
                "Resized {n} {} — Ctrl+Z takes it back",
                if rows { "row(s)" } else { "column(s)" }
            ),
            Err(error) => error.to_string(),
        });
    }

    /// *Define a name for the selection…* — sheet-qualified so it means the same place from every
    /// sheet (`grind_sheet::verbs::name_target`), read the way `grind sheet name` reads one.
    fn define_name(&self) {
        let sheet = self.app.sheet_name(self.sheet.get()).unwrap_or_default();
        let target = grind_sheet::verbs::name_target(&sheet, self.nav_selection());
        let Some(name) = self.ask(&format!("A name for {target}"), "") else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let result = grind_sheet::a1::definition(&self.app, &target)
            .and_then(|expression| self.app.set_name(name, &expression));
        self.set_message(match result {
            Ok(()) => format!("“{name}” now means {target}"),
            Err(error) => error.to_string(),
        });
    }

    /// *Rename a name…* — every formula and every other name that uses it comes with it, in one
    /// undo step (`App::rename_name`, §6.5).
    fn rename_name(&self) {
        let Some(from) = self.ask("Rename which name?", "") else {
            return;
        };
        let Some(to) = self.ask(&format!("Rename “{}” to", from.trim()), from.trim()) else {
            return;
        };
        self.set_message(match self.app.rename_name(from.trim(), to.trim()) {
            Ok(n) => format!("Renamed — {n} use(s) rewritten"),
            Err(error) => error.to_string(),
        });
    }

    /// *Inline a name into its uses…* — the name's definition written where it is used, and the
    /// name deleted, in one undo step (`App::inline_name`, §6.5).
    fn inline_name(&self) {
        let Some(name) = self.ask("Inline which name?", "") else {
            return;
        };
        self.set_message(match self.app.inline_name(name.trim()) {
            Ok(n) => format!("Inlined — {n} use(s) rewritten, the name is gone"),
            Err(error) => error.to_string(),
        });
    }

    /// *Delete a name…* — the definition only; formulas that used it will say `#NAME?`.
    fn delete_name(&self) {
        let Some(name) = self.ask("Delete which name?", "") else {
            return;
        };
        self.set_message(match self.app.clear_name(name.trim()) {
            true => format!("“{}” deleted", name.trim()),
            false => format!("There is no name “{}”", name.trim()),
        });
    }

    /// *Insert a chart from the selection* — `grind_sheet::verbs::insert_chart`, which reads the
    /// table the way the CLI and the GNOME window do; this shell says only where a column and a
    /// row sit.
    fn insert_chart(&self) {
        let (start, end) = self.rect();
        let (widths, heights) = (self.widths(), self.heights());
        match grind_sheet::verbs::insert_chart(
            &self.app,
            self.sheet.get(),
            start,
            end,
            |col, row| {
                (
                    widths.span(0, col) / PX_PER_MM,
                    heights.span(0, row) / PX_PER_MM,
                )
            },
        ) {
            Ok(_) => self.set_message("A chart beside the table — Ctrl+Z takes it back".to_owned()),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// *Delete the last chart* — a chart is picked by clicking it in the GNOME window; here there is no pick.
    fn delete_chart(&self) {
        let sheet = self.sheet.get();
        let Ok(count) = self.app.charts(sheet).map(|charts| charts.len()) else {
            return;
        };
        if count == 0 {
            return self.set_message("This sheet has no chart".to_owned());
        }
        match self.app.remove_chart(sheet, count - 1) {
            Ok(()) => self.set_message("Deleted the last chart — Ctrl+Z brings it back".to_owned()),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// *Explain this formula in words* — the active cell's formula with its functions by their
    /// plain names and its arguments labelled (`friendly::explain_inline`). Presentation only: it
    /// never parses back and nothing is written.
    fn explain(&self) {
        let at = self.selection.get().active;
        let text = self
            .app
            .input_text(self.sheet.get(), at)
            .unwrap_or_default();
        let address = grind_sheet::a1::format(None, at);
        self.set_message(
            match grind_sheet::formula::friendly::explain_inline(&text) {
                Ok(words) if text.starts_with('=') => format!("{address}: {words}"),
                _ => format!("{address} holds no formula to explain"),
            },
        );
    }

    /// *Evaluate a formula…* — worked out at the active cell and said, never stored
    /// (`grind_sheet::verbs::evaluated`).
    fn evaluate(&self) {
        let Some(typed) = self.ask("A formula, worked out at the active cell", "=") else {
            return;
        };
        let at = self.selection.get().active;
        self.set_message(
            match grind_sheet::verbs::evaluated(&self.app, self.sheet.get(), at, &typed) {
                Ok(value) => format!("{} = {value} (nothing was stored)", typed.trim()),
                Err(why) => format!("That cannot be worked out: {why}"),
            },
        );
    }

    /// Hide — or, with `hidden: false`, unhide — the rows the selection spans.
    ///
    /// `doc/sheet-shell.md`'s row/column hiding, over `App::set_row_hidden`, the same call
    /// the CLI's `sheet hide`/`--unhide` makes. There is no fill handle or drag here to hide
    /// a column edge, so a selected rectangle's own row span is the whole of "which rows" —
    /// unhiding reaches a hidden run the same way, since Shift+arrow through one still moves
    /// the selection's index across it even though it draws at no height.
    fn hide_rows(&self, hidden: bool) {
        let (start, end) = self.selection.get().rect();
        match self
            .app
            .set_row_hidden(self.sheet.get(), start.row..end.row + 1, hidden)
        {
            Ok(n) => self.set_message(match hidden {
                true => format!("Hid {n} row(s)"),
                false => format!("Unhid {n} row(s)"),
            }),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// The column twin of [`Ui::hide_rows`].
    fn hide_cols(&self, hidden: bool) {
        let (start, end) = self.selection.get().rect();
        match self
            .app
            .set_col_hidden(self.sheet.get(), start.col..end.col + 1, hidden)
        {
            Ok(n) => self.set_message(match hidden {
                true => format!("Hid {n} column(s)"),
                false => format!("Unhid {n} column(s)"),
            }),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    // --- the autofilter (§9.4) ---

    /// The sheet's filter, if it has one.
    fn filter(&self) -> Option<Filter> {
        self.app.filter(self.sheet.get()).ok().flatten()
    }

    pub fn is_filter_open(&self) -> bool {
        self.filter_field.get().is_some()
    }

    pub fn close_filter_menu(&self) {
        if self.filter_field.take().is_some() {
            self.dom.filter_menu.set_hidden(true);
        }
    }

    /// Filter the selection, or clear the filter the sheet already has — the toolbar's
    /// `sheet.filter`, `ui_sheet_gtk`'s `Grid::toggle_filter` mirrored.
    ///
    /// Over a sheet that already has one this clears it, so the command is the on/off switch
    /// its name implies; otherwise the selection becomes the range, with its first row the
    /// heading, which is what a person selecting a table with its titles means.
    fn toggle_filter(&self) {
        let sheet = self.sheet.get();
        if self.filter().is_some() {
            self.close_filter_menu();
            if let Err(error) = self.app.set_filter(sheet, None) {
                self.set_message(error.to_string());
            }
            return;
        }
        let (start, end) = self.selection.get().rect();
        let used = self.app.used_extent(sheet).unwrap_or((0, 0));
        match Filter::over_selection(start, end, used) {
            Ok(filter) => {
                if let Err(error) = self.app.set_filter(sheet, Some(filter)) {
                    self.set_message(error.to_string());
                }
            }
            Err(why) => self.set_message(why.to_owned()),
        }
    }

    /// Format the selection as a table — the toolbar's `sheet.format-table`.
    ///
    /// No dialog: like [`Self::toggle_filter`], the selection is the range, its first row is
    /// the heading, and the name auto-generates (`Table1`, `Table2`, …). There is no
    /// "un-format" to match — ODF has nothing resembling a persisted table object, so this is
    /// a one-shot composite the same way LibreOffice's own AutoFormat is
    /// (`sheet/src/table_format.rs`).
    fn format_table(&self, totals: Option<grind_sheet::TotalsFunction>) {
        let sheet = self.sheet.get();
        let (start, mut end) = self.selection.get().rect();
        // A single cell is a click, not a range — the same rule `toggle_filter` uses.
        if start == end
            && let Ok((rows, cols)) = self.app.used_extent(sheet)
        {
            end = Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
        }
        if end.row <= start.row {
            return self
                .set_message("Select the rows to format, including their headings".to_owned());
        }
        let options = grind_sheet::TableOptions {
            header: true,
            totals,
            name: None,
        };
        match self.app.format_table(sheet, start, end, options) {
            Ok(name) => self.set_message(format!("Formatted as table \u{201c}{name}\u{201d}")),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// *Format as table with totals…* — the same composite, having asked which aggregate the
    /// totals row carries.
    ///
    /// `window.prompt` for the same reason [`Self::rename_sheet`] uses one: it is one word of
    /// input and the browser already has a box for that. It is a *second* command rather than
    /// a prompt bolted onto the first so that the plain one stays zero-prompt — the palette's
    /// own way of offering two shapes of a verb, since there are no submenus here.
    fn format_table_with_totals(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let ask = format!("Totals row: {}", grind_sheet::TotalsFunction::ids());
        let Ok(Some(answer)) = window.prompt_with_message_and_default(&ask, "sum") else {
            return;
        };
        match answer.parse::<grind_sheet::TotalsFunction>() {
            Ok(function) => self.format_table(Some(function)),
            Err(say) => self.set_message(say),
        }
    }

    // --- CSV, the one non-ODF format (`doc/not-doing.md` §2) ---

    /// A delimited file, read in **at the cursor**, in one undo step.
    ///
    /// The shell hands over the text because only it can reach a file (rule 5 — a browser has
    /// no filesystem, and `App::import_csv` takes a `&str` rather than a path for exactly that
    /// reason); what is this pane's is where the fields land and what they are read as, and
    /// both come from what it already holds. `csv::Import::sniffed` is the same answer every
    /// other window gives, so one file imports the same way in all four.
    pub fn import_csv(&self, text: &str) {
        let sheet = self.sheet.get();
        let at = self.selection.get().active;
        let options = csv::Import::sniffed(text);
        match self
            .app
            .import_csv(sheet, at, text, &options, RecalcMode::Document)
        {
            Ok(outcome) => self.set_message(match outcome.cells {
                1 => format!("1 cell imported at {}", a1::format(None, at)),
                cells => format!("{cells} cells imported at {}", a1::format(None, at)),
            }),
            Err(error) => self.set_message(error.to_string()),
        }
    }

    /// The selection as delimited text — or everything the sheet uses, when the selection is
    /// one cell and therefore a cursor rather than a range (`format_table`'s own reading).
    ///
    /// `None` is "there is nothing to write", already said on the message line: a download of
    /// an empty file is a worse answer than a sentence.
    pub fn export_csv(&self, dialect: csv::Dialect) -> Option<String> {
        let sheet = self.sheet.get();
        let (mut start, mut end) = self.selection.get().rect();
        if start == end {
            let (rows, cols) = self.app.used_extent(sheet).ok()?;
            if rows == 0 || cols == 0 {
                self.set_message("There is nothing in this sheet to export".to_owned());
                return None;
            }
            (start, end) = (Pos::new(0, 0), Pos::new(rows - 1, cols - 1));
        }
        let options = csv::Export {
            dialect,
            ..csv::Export::default()
        };
        match self.app.export_csv(sheet, start, end, &options) {
            Ok(text) => Some(text),
            Err(error) => {
                self.set_message(error.to_string());
                None
            }
        }
    }

    /// Open the popover for one field, under the button that asked for it.
    fn open_filter_menu(&self, field: u32, anchor: &Element) -> Result<(), JsValue> {
        let Some(filter) = self.filter() else {
            return Ok(());
        };
        // The whole filtered column, not the visible part: a value scrolled off screen is
        // still one of the column's values.
        let col = filter.column(field);
        let cells = self
            .app
            .get_viewport(
                self.sheet.get(),
                filter.start.row..filter.end.row.saturating_add(1),
                col..col.saturating_add(1),
            )
            .map_err(js)?;
        let values = filter_ui::field_values(&cells, &filter, field);
        self.filter_field.set(Some(field));
        self.build_filter_list(&values, filter.keep.get(&field))?;

        let at = anchor.get_bounding_client_rect();
        let style = self.dom.filter_menu.style();
        style.set_property("left", &format!("{}px", at.left()))?;
        style.set_property("top", &format!("{}px", at.bottom() + 4.0))?;
        self.dom.filter_menu.set_hidden(false);
        Ok(())
    }

    /// Rebuild the popover's checkbox list. `kept` is the field's current condition — `None`
    /// when it has none, which ticks everything, because a field nobody has filtered keeps
    /// every value it has.
    fn build_filter_list(
        &self,
        values: &[String],
        kept: Option<&BTreeSet<String>>,
    ) -> Result<(), JsValue> {
        self.dom.filter_list.set_text_content(None);
        let mut checks = Vec::with_capacity(values.len());
        for value in values {
            let row = self.dom.document.create_element("label")?;
            row.set_class_name("filter-row");
            let check: HtmlInputElement = self
                .dom
                .document
                .create_element("input")?
                .dyn_into()
                .map_err(|_| JsValue::from_str("an input is not an input"))?;
            check.set_type("checkbox");
            check.set_checked(kept.is_none_or(|k| k.contains(value)));
            row.append_child(&check)?;
            let label = self.dom.document.create_element("span")?;
            match value.is_empty() {
                // Italic, so "(empty)" cannot be confused with a cell that literally says it.
                true => {
                    label.set_class_name("filter-empty");
                    label.set_text_content(Some(filter_ui::EMPTY_LABEL));
                }
                false => label.set_text_content(Some(value)),
            }
            row.append_child(&label)?;
            self.dom.filter_list.append_child(&row)?;
            checks.push((value.clone(), check));
        }
        *self.filter_checks.borrow_mut() = checks;
        self.sync_filter_all();
        Ok(())
    }

    /// The "Select all" box reflects the rows rather than driving them: all, none, or the
    /// inconsistent state in between.
    fn sync_filter_all(&self) {
        let checks = self.filter_checks.borrow();
        let ticked = checks.iter().filter(|(_, c)| c.checked()).count();
        self.dom
            .filter_all
            .set_indeterminate(ticked > 0 && ticked < checks.len());
        self.dom
            .filter_all
            .set_checked(ticked == checks.len() && ticked > 0);
    }

    fn filter_ticked(&self) -> BTreeSet<String> {
        self.filter_checks
            .borrow()
            .iter()
            .filter(|(_, c)| c.checked())
            .map(|(value, _)| value.clone())
            .collect()
    }

    /// What the popover decided, as an undoable change. The whole filter is replaced because
    /// that is the vocabulary `App::set_filter` has — one filter is one value
    /// (`grind_sheet::model`), so a field's condition is edited by reading, changing and
    /// writing it back.
    fn apply_filter(&self, chosen: filter_ui::Chosen) {
        let Some(field) = self.filter_field.get() else {
            return;
        };
        let Some(mut filter) = self.filter() else {
            return;
        };
        match chosen {
            filter_ui::Chosen::Clear => {
                filter.keep.remove(&field);
            }
            // Applying nothing would hide every row — a document that has apparently
            // emptied itself. Refused rather than accepted, with the popover left open.
            filter_ui::Chosen::Keep(values) if values.is_empty() => {
                return self
                    .set_message("Keep at least one value, or Clear to show every row".to_owned());
            }
            filter_ui::Chosen::Keep(values) => {
                filter.keep.insert(field, values);
            }
        }
        self.close_filter_menu();
        if let Err(error) = self.app.set_filter(self.sheet.get(), Some(filter)) {
            self.set_message(error.to_string());
        }
        let _ = self.dom.surface.focus();
    }

    fn move_to(&self, motion: Motion, extend: bool) {
        let sheet = self.sheet.get();
        let extent = self.app.used_extent(sheet).unwrap_or((0, 0));
        let selection = self.selection.get();
        let page = self.visible().0.saturating_sub(1);
        let active = match motion {
            // The next edge of the data is a question for the document: `grind_sheet::nav`'s
            // rule, over the occupied cells, the same one every other window asks.
            Motion::Edge(dir) => {
                use grind_sheet::nav;
                let dir = match dir {
                    keymap::Dir::Left => nav::Dir::Left,
                    keymap::Dir::Right => nav::Dir::Right,
                    keymap::Dir::Up => nav::Dir::Up,
                    keymap::Dir::Down => nav::Dir::Down,
                };
                nav::moved(
                    nav::Selection {
                        anchor: selection.anchor,
                        active: selection.active,
                    },
                    nav::Motion::Edge(dir),
                    false,
                    nav::Extent {
                        rows: extent.0,
                        cols: extent.1,
                        page: page.max(1),
                    },
                    &nav::occupied(&self.app, sheet),
                )
                .active
            }
            _ => keymap::moved(selection.active, motion, extent, page),
        };
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
        self.begin_with(&text)
    }

    /// Start editing with `text` already in the bar and the caret after it.
    fn begin_with(&self, text: &str) -> Result<(), JsValue> {
        self.editing.set(true);
        self.dom.formula.set_value(text);
        self.dom.formula.focus()?;
        // Focusing an `<input>` selects it in some browsers, and the caret belongs
        // after what is there or the next keystroke deletes the seed — the same trap
        // `ui_sheet_gtk`'s `Grid::begin` documents, in a different toolkit.
        let end = utf16::units_before(text, text.len()) as u32;
        self.dom.formula.set_selection_range(end, end)?;
        self.set_message(String::new());
        self.refresh_assist()
    }

    /// Recompute the assist band from the formula bar's own text and caret, and draw it
    /// (`assist.rs`). Called on every keystroke and every caret move while editing, and
    /// clears the band the moment editing ends.
    fn refresh_assist(&self) -> Result<(), JsValue> {
        if !self.editing.get() {
            self.assist.borrow_mut().clear();
            return self.render_assist();
        }
        let text = self.dom.formula.value();
        // `selection_start` counts UTF-16 code units, as every DOM offset does, and the assist
        // counts bytes — `grind_core::utf16` is the conversion, the one `ui_win32`'s
        // `EM_GETSEL` goes through too. Counting `char`s instead put the caret one place off
        // for every emoji before it.
        let units = self.dom.formula.selection_start()?.unwrap_or(0) as usize;
        let caret = utf16::byte_of(&text, units);
        let names: Vec<String> = self.app.names().into_iter().map(|(name, _)| name).collect();
        self.assist.borrow_mut().refresh(&text, caret, &names);
        self.render_assist()
    }

    /// Draw the band `assist::band` describes, one `<span>` per run — or hide it, when there
    /// is nothing to say.
    fn render_assist(&self) -> Result<(), JsValue> {
        let pieces = assist::band(&self.assist.borrow());
        self.dom.assist.set_text_content(None);
        self.dom.assist.set_hidden(pieces.is_empty());
        for piece in pieces {
            let span = self.dom.document.create_element("span")?;
            span.set_class_name(match piece.ink {
                assist::Ink::Plain => "plain",
                assist::Ink::Muted => "muted",
                assist::Ink::Strong => "strong",
            });
            span.set_text_content(Some(&piece.text));
            self.dom.assist.append_child(&span)?;
        }
        Ok(())
    }

    /// Put the highlighted offer into the formula bar and keep editing — Tab, while the band
    /// is offering.
    fn accept_assist(&self) -> Result<(), JsValue> {
        let Some((span, replacement)) = self.assist.borrow_mut().accept() else {
            return Ok(());
        };
        let text = self.dom.formula.value();
        let mut next = String::with_capacity(text.len() + replacement.len());
        next.push_str(&text[..span.start]);
        next.push_str(&replacement);
        next.push_str(&text[span.end..]);
        let caret_byte = span.start + replacement.len();
        let caret = utf16::units_before(&next, caret_byte) as u32;
        self.dom.formula.set_value(&next);
        self.dom.formula.focus()?;
        self.dom.formula.set_selection_range(caret, caret)?;
        self.request_repaint();
        self.refresh_assist()
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
            let input = match display::to_input(&text) {
                Ok(input) => input,
                Err(error) => {
                    // The edit stays open, with the caret on the problem.
                    self.set_message(format!("{} (at {})", error.message, error.at));
                    let at = utf16::units_before(&text, error.at) as u32;
                    self.dom.formula.focus()?;
                    self.dom.formula.set_selection_range(at, at)?;
                    return Ok(());
                }
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
        self.refresh_assist()
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
        // A filter button beats the cell under it, for the same reason the fill handle would:
        // it sits over the corner of a cell that is also a click target for selecting it.
        if let Some(button) = target.closest("button.filter-btn")? {
            let field = attribute(&button, "data-field").unwrap_or(0);
            self.dragging.set(false);
            return self.open_filter_menu(field, &button);
        }
        // Clicking the grid while the popover is open is how every spreadsheet dismisses one
        // — there is no backdrop here to catch it, so the ordinary click path does instead.
        if self.is_filter_open() {
            self.close_filter_menu();
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

/// What a replace says in the message line — pure, so it is tested on the host.
fn replaced(what: &str, done: &find::Replaced) -> String {
    let mut said = match done.cells {
        0 if done.refused.is_empty() => format!("No cell holds “{what}”"),
        0 => "Nothing replaced".to_owned(),
        1 => "Replaced in 1 cell — Ctrl+Z takes it back".to_owned(),
        n => format!("Replaced in {n} cells — Ctrl+Z takes them back"),
    };
    if let Some((hit, _)) = done.refused.first() {
        said.push_str(&match done.refused.len() {
            1 => format!(
                "; {} was left alone, since its formula would not parse",
                hit.address()
            ),
            n => format!(
                "; {n} formulas were left alone, since they would not parse (first {})",
                hit.address()
            ),
        });
    }
    if done.recalc.is_some_and(|recalc| recalc.spoiled > 0) {
        said.push_str(" — not recalculated, since that would spoil a saved value");
    }
    said
}

/// What `format.currency` spells: the suite's default currency, the euro, so a cell formatted
/// here looks the way the same command formats it everywhere else.
const CURRENCY: &str = grind_sheet::numfmt::DEFAULT_CURRENCY;

/// Turn a property on, or — when it is already that value — off. What a *toggle* means, as
/// opposed to a value a picker sets.
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
    let (kind, decimals, _, symbol) = format.preset_params();
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
        Kind::Currency if symbol == "$" => "format.currency-usd",
        Kind::Currency if symbol == "£" => "format.currency-gbp",
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

/// Which edges of a name anchor's range this cell sits on, space-separated for
/// `data-anchor` — `doc/view-modes.md`'s "a range anchor is not outlined" gap, closed the
/// way `ui_sheet_gtk/src/grid.rs`'s `draw_hints` does it: a range says how far it reaches
/// by being outlined, and a single cell needs no outline since the hint already sits
/// inside the only cell it means. `content: attr(data-anchor)` is not used here — the
/// stylesheet reads each edge word with `~=` and turns it into a `border-*`, so the
/// outline is one attribute per boundary cell rather than a second element.
fn anchor_edges(viewport: &grind_sheet::Viewport, row: u32, col: u32) -> String {
    let mut edges = Vec::new();
    for anchor in viewport.names() {
        if !anchor.is_range() || !anchor.rows.contains(&row) || !anchor.cols.contains(&col) {
            continue;
        }
        if row == anchor.rows.start {
            edges.push("top");
        }
        if row == anchor.rows.end - 1 {
            edges.push("bottom");
        }
        if col == anchor.cols.start {
            edges.push("left");
        }
        if col == anchor.cols.end - 1 {
            edges.push("right");
        }
    }
    edges.join(" ")
}

/// The status line's reading of a selection: `B8:C10 · Sum 9,391.13 · Count 4 · Average …`,
/// the range alone when it holds nothing, and **nothing for one cell** — the address box above
/// the grid is already saying where that is, and the tab below it which sheet.
///
/// It used to read `Budget · 20×10 used · 3×2 selected`: the sheet's name the tab beside it was
/// showing, the used extent — a number about the file that nobody selecting cells is asking —
/// and a size where every other spreadsheet puts the sum. The aggregates are `App::preview` over
/// generated formulas, the way `ui_sheet_gtk`'s and `ui_win32`'s status bars do it: `COUNTA`,
/// since a status bar's Count is of what is there rather than of numbers, and Sum and Average
/// only when there is a number to add. The range is clamped to the used extent first, so a
/// whole-column selection does not walk a million rows.
fn summary(app: &App, sheet: usize, start: Pos, end: Pos) -> String {
    if start == end {
        return String::new();
    }
    let address = format!("{}:{}", a1::format(None, start), a1::format(None, end));
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
    // Evaluated one row past the used extent, since somewhere inside the range would be a
    // circular reference.
    let at = Pos::new(rows, 0);
    let of = |formula: String| match app.preview(sheet, at, &formula) {
        Ok(CellValue::Number(n)) => Some(n),
        _ => None,
    };
    let count = of(format!("=COUNTA({range})")).unwrap_or(0.0);
    if count == 0.0 {
        return address;
    }
    let mut parts = vec![address];
    if let Some(sum) = of(format!("=SUM({range})"))
        && let Some(average) = of(format!("=AVERAGE({range})"))
    {
        parts.push(format!("Sum {}", app.display_number(sum)));
        parts.push(format!("Count {}", app.display_number(count)));
        parts.push(format!("Average {}", app.display_number(average)));
    } else {
        parts.push(format!("Count {}", app.display_number(count)));
    }
    parts.join(" · ")
}

fn css_of(style: Option<&CellStyle>, numeric: bool, dark: bool) -> String {
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
    // The document's colour, made to read on the page it lands on (`crate::ink`).
    let ink = crate::ink::color(style.color.as_deref(), style.background.as_deref(), dark);
    set(&mut css, "color", &ink);
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
        if let Err(error) = input.refresh_assist() {
            web_sys::console::error_1(&error);
        }
    })?;

    // Neither a click nor an arrow key changes the text, so `input` never fires for them —
    // and the band has to catch up to a caret the mouse or the browser's own text handling
    // just moved.
    let moved = ui.clone();
    listen(&ui.dom.formula, "keyup", move |_: Event| {
        if let Err(error) = moved.refresh_assist() {
            web_sys::console::error_1(&error);
        }
    })?;
    let clicked = ui.clone();
    listen(&ui.dom.formula, "click", move |_: Event| {
        if let Err(error) = clicked.refresh_assist() {
            web_sys::console::error_1(&error);
        }
    })
}

/// The autofilter's popover (§9.4, `filter_ui.rs`): "Select all", a value ticked or
/// unticked, and the two buttons that decide what sticks.
fn wire_filter_menu(ui: &Rc<Ui>) -> Result<(), JsValue> {
    let all = ui.clone();
    listen(&ui.dom.filter_all, "change", move |_: Event| {
        let checked = all.dom.filter_all.checked();
        for (_, check) in all.filter_checks.borrow().iter() {
            check.set_checked(checked);
        }
    })?;

    // One listener for the whole list rather than one per checkbox, since the list is
    // rebuilt every time the popover opens.
    let list = ui.clone();
    listen(&ui.dom.filter_list, "change", move |_: Event| {
        list.sync_filter_all();
    })?;

    let clear = ui.clone();
    listen(&ui.dom.filter_clear, "click", move |_: Event| {
        clear.apply_filter(filter_ui::Chosen::Clear);
    })?;

    let apply = ui.clone();
    listen(&ui.dom.filter_apply, "click", move |_: Event| {
        let ticked = apply.filter_ticked();
        apply.apply_filter(filter_ui::Chosen::Keep(ticked));
    })
}

/// A number that does not fit its cell is **`###`**, never part of itself — `numfmt::overflow`,
/// the rule every shell in the suite draws. The stylesheet clips a cell's content, so without this
/// `3,710.00 €` in a narrow column read `3,710.0` with nothing to say a digit and the currency
/// had gone.
///
/// Measured after the rows are in the page, since only then has the browser laid them out: a
/// cell whose content is wider than its box (`scrollWidth > clientWidth`) is measured once more
/// with ten hashes in its own font, and given as many as its room holds. The box's horizontal
/// padding is taken off both, since both widths include it. Nothing happens where nothing is laid
/// out — jsdom reports every width as zero, so the smoke test sees the numbers unchanged.
fn hash_overflowing(cells: &[web_sys::Element]) -> Result<(), JsValue> {
    /// `.grid td`'s `padding: 0 4px`, both sides.
    const PADDING: f64 = 8.0;
    for cell in cells {
        let (scroll, client) = (cell.scroll_width(), cell.client_width());
        if client <= 0 || scroll <= client {
            continue;
        }
        cell.set_text_content(Some("##########"));
        let hash = (f64::from(cell.scroll_width()) - PADDING) / 10.0;
        let room = f64::from(client) - PADDING;
        cell.set_text_content(Some(&grind_sheet::numfmt::overflow(room, hash)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The message line after a replace, over a real `App` — the one piece of the browser's
    /// find that is this shell's own words rather than the core's answer.
    #[test]
    fn a_replace_says_how_many_cells_and_which_it_left_alone() {
        let app = App::new();
        for (row, input) in ["Apples", "apples pie", "=SUM([.A9])"].iter().enumerate() {
            app.enter(0, Pos::new(row as u32, 0), input, RecalcMode::No)
                .expect("enters");
        }
        let done = app
            .replace(&Search::new("apples"), "Pears", RecalcMode::No)
            .expect("replaces");
        assert_eq!(
            replaced("apples", &done),
            "Replaced in 2 cells — Ctrl+Z takes them back"
        );
        let done = app
            .replace(&Search::new("SUM("), "SUM", RecalcMode::No)
            .expect("replaces");
        assert_eq!(
            replaced("SUM(", &done),
            "Nothing replaced; Sheet1.A3 was left alone, since its formula would not parse"
        );
        let done = app
            .replace(&Search::new("nowhere"), "x", RecalcMode::No)
            .expect("replaces");
        assert_eq!(replaced("nowhere", &done), "No cell holds “nowhere”");
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
    fn the_status_line_adds_up_a_range_and_is_quiet_for_one_cell() {
        let app = App::new();
        for (row, value) in [(0, "10"), (1, "20.5"), (2, "Label")] {
            app.enter(0, Pos::new(row, 1), value, RecalcMode::Document)
                .expect("enters");
        }
        assert_eq!(summary(&app, 0, Pos::new(0, 1), Pos::new(0, 1)), "");
        assert_eq!(
            summary(&app, 0, Pos::new(0, 1), Pos::new(2, 1)),
            "B1:B3 · Sum 30.5 · Count 3 · Average 15.25"
        );
        assert_eq!(
            summary(&app, 0, Pos::new(2, 1), Pos::new(2, 2)),
            "B3:C3 · Count 1",
            "text counts and does not add"
        );
        assert_eq!(summary(&app, 0, Pos::new(5, 5), Pos::new(6, 6)), "F6:G7");
    }

    #[test]
    fn a_style_becomes_the_css_the_document_asked_for() {
        let style = CellStyle {
            font_weight: Some("bold".into()),
            background: Some("#ffdc00".into()),
            borders: [Some("0.06pt solid #000000".into()), None, None, None],
            ..CellStyle::default()
        };
        let css = css_of(Some(&style), false, false);
        assert!(css.contains("font-weight:bold;"), "{css}");
        assert!(css.contains("background-color:#ffdc00;"), "{css}");
        assert!(css.contains("border-left:0.06pt solid #000000;"), "{css}");
        assert!(!css.contains("border-right"), "{css}");
    }

    #[test]
    fn a_number_right_aligns_until_the_document_says_otherwise() {
        assert_eq!(css_of(None, true, false), "text-align:right;");
        assert_eq!(css_of(None, false, false), "");
        let centred = CellStyle {
            align: Some("center".into()),
            ..CellStyle::default()
        };
        let css = css_of(Some(&centred), true, false);
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
        assert_eq!(css_of(Some(&style), false, false), "");
        let middle = CellStyle {
            vertical_align: Some("middle".into()),
            ..CellStyle::default()
        };
        assert_eq!(
            css_of(Some(&middle), false, false),
            "vertical-align:middle;"
        );
    }

    /// A range anchor is outlined on its boundary cells only — `doc/view-modes.md`'s "a range
    /// anchor is not outlined" gap, closed the way `ui_sheet_gtk`'s `draw_hints` outlines the
    /// same rectangle. A single-cell name gets no edges at all: the hint already sits inside
    /// the only cell it means (`NameAnchor::is_range`).
    #[test]
    fn a_range_anchor_is_outlined_on_its_boundary_cells_only() {
        let app = App::new();
        app.set_cell(0, Pos::new(0, 0), 1.0).unwrap();
        let sheet_name = app.sheet_name(0).unwrap();
        app.set_name("sales", &format!("[${sheet_name}.$A$1:.$B$3]"))
            .unwrap();
        let overlays = grind_sheet::view::Overlays {
            names: true,
            ..Default::default()
        };
        let viewport = app.get_viewport_with(0, 0..5, 0..5, overlays).unwrap();

        assert_eq!(anchor_edges(&viewport, 0, 0), "top left");
        assert_eq!(anchor_edges(&viewport, 0, 1), "top right");
        assert_eq!(anchor_edges(&viewport, 1, 0), "left");
        assert_eq!(anchor_edges(&viewport, 2, 1), "bottom right");
        assert_eq!(anchor_edges(&viewport, 3, 0), "");

        app.set_name("total", &format!("[${sheet_name}.$D$4]"))
            .unwrap();
        let viewport = app.get_viewport_with(0, 0..5, 0..5, overlays).unwrap();
        assert_eq!(anchor_edges(&viewport, 3, 3), "");
    }
}
