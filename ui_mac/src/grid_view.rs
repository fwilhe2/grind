// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The grid, inside an `NSScrollView` — the spreadsheet's document view, and the one place this
//! shell draws a sheet on screen (decision 3: AppKit draws the chrome, this draws the document).
//!
//! Four flipped views over one [`Pane`]: the grid itself, as the scroll view's document view and as
//! large as the part of the sheet worth scrolling to; the column and row header bands, as the
//! scroll view's **floating subviews**, so each stays put along its own axis and scrolls with the
//! cells along the other; and the corner where they meet, over both. The scroll view does the
//! scrolling — elastic, with the system's own scrollers — and every view draws through
//! `sheet::paint` and [`crate::render::draw`], the path `--render-to` takes too.
//!
//! **The selection is the pane's** (M3), and every input only asks `sheet::select` what it
//! becomes: a key through `interpretKeyEvents:` and `doCommandBySelector:` — the user's own key
//! bindings, read by `keys.rs` — a click and a drag on the grid, and a click on a header band. A
//! change redraws the four views, scrolls the active cell into sight beside the bands, and tells
//! whoever listens (the name box and the status bar).
//!
//! The grid view's own coordinates put the sheet's top-left cell at `(HEADER_W, HEADER_H)`: the
//! bands float over that margin, so at rest they cover nothing but it.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak as RcWeak};
use std::sync::Arc;

use grind_core::color::{self, Rgb};
use grind_sheet::nav::Selection;
use grind_sheet::{ChartKind, ChartLegend};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibilityAnnouncementKey, NSAccessibilityAnnouncementRequestedNotification,
    NSAccessibilityPostNotificationWithUserInfo, NSAutoresizingMaskOptions, NSBeep, NSColor,
    NSColorPanel, NSColorSpace, NSControlStateValueOff, NSControlStateValueOn, NSCursor,
    NSCursorFrameResizeDirections, NSCursorFrameResizePosition, NSEvent, NSEventGestureAxis,
    NSEventModifierFlags, NSFontManager, NSGraphicsContext, NSMenu, NSMenuItem, NSScrollView,
    NSTextField, NSView,
};
use objc2_core_graphics::CGContext;
use objc2_foundation::{
    NSArray, NSAttributedString, NSDictionary, NSObject, NSPoint, NSRect, NSSize, NSString,
};

use crate::banner::Banner;
use crate::editor::Edit;
use crate::find_bar::FindBar;
use crate::keys;
use crate::metrics::{BASE_PT, CoreText};
use crate::render;
use crate::sheet::chart::Change;
use crate::sheet::geom::{Grid, HEADER_H, HEADER_W, Rect};
use crate::sheet::paint::{self, Look, Op, Palette};
use crate::sheet::resize::{self, Axis, Drag};
use crate::sheet::select;
use crate::sheet::state::{self, Mode, Seed};
use crate::watch::Turn;

/// Something told the new selection whenever it changes.
type Listener = Box<dyn Fn(Selection)>;
/// Something told the text being edited, or the active cell's, whenever it changes.
type TextListener = Box<dyn Fn(&str)>;

/// What every view of one spreadsheet window draws from and answers to: the document, which sheet
/// is showing, where its cells are, the fonts it is measured in, and the selection.
pub struct Pane {
    /// This pane, weakly — what an object the pane creates (the editor's delegate) holds.
    me: RcWeak<Pane>,
    pub app: Arc<grind_sheet::App>,
    pub sheet: Cell<usize>,
    pub grid: RefCell<Grid>,
    pub text: CoreText,
    pub selection: Cell<Selection>,
    /// The four views, held weakly — each holds the pane, and the scroll view holds them.
    views: RefCell<Vec<Weak<NSView>>>,
    /// The grid view itself, which is what scrolls a cell into sight.
    grid_view: RefCell<Option<Weak<GridView>>>,
    /// Told the new selection whenever it changes: the name box and the status bar.
    listeners: RefCell<Vec<Listener>>,
    /// Objects nothing else holds strongly — a control's target, which the control holds weakly.
    kept: RefCell<Vec<Retained<NSObject>>>,
    /// The name box, once the window has one — what Go To puts the keyboard in.
    name_box: RefCell<Option<Weak<NSTextField>>>,
    /// The two bands, which change size with the sheet.
    bands: RefCell<Option<(Weak<ColumnHeader>, Weak<RowHeader>)>>,
    /// The cell being edited, if one is (`editor.rs`).
    pub(crate) edit: RefCell<Option<Edit>>,
    /// The notice banner, once the window has one (`banner.rs`).
    pub(crate) banner: RefCell<Option<Banner>>,
    /// Told the text being edited as it changes: the formula read-out.
    text_listeners: RefCell<Vec<TextListener>>,
    /// Told that the document changed — which is what marks it edited, and what autosave reads.
    on_change: crate::watch::OnChange,
    /// Which way the change being made goes — set around an undo or a redo.
    turn: Cell<Turn>,
    /// Told when the document or the sheet showing changed — the sidebar, which lists the
    /// sheets and what `grind lint` finds, and has nothing to say about a move of the cursor.
    document_listeners: RefCell<Vec<Box<dyn Fn()>>>,
    /// The find bar, once the window has one (`find_bar.rs`).
    pub(crate) find_bar: RefCell<Option<Retained<FindBar>>>,
    /// Whether formulas are read in plain English — View ▸ Friendly Formulas (M8).
    pub friendly: Cell<bool>,
    /// What the last move announced (decision 10) — kept so a drive can read back what
    /// VoiceOver was told.
    pub announced: RefCell<String>,
    /// Which of `doc/view-modes.md`'s overlays are drawn — View ▸ Cell Roles and Names (M8).
    /// Asked for on every paint and never written: a save with every overlay on is the bytes
    /// a save with none would be.
    pub overlays: Cell<grind_sheet::view::Overlays>,
    /// A track picked up by its edge in a header band, until the button comes up (`resize.rs`).
    resizing: Cell<Option<Drag>>,
    /// View ▸ Formulas: every formula shown rather than its value. Drawn, never written.
    pub formulas: Cell<bool>,
    /// View ▸ Calculations…: what the sidebar's Calculations section is narrowed to, while it
    /// is shown.
    pub calculations: RefCell<Option<String>>,
    /// The selected chart, as the sheet it is on and its index there (`doc/chart-handling.md`):
    /// outlined and handled, and what ⌫, the arrows and Return mean. Read through
    /// [`Pane::chart_now`], which lets go of one on another sheet or one deleted away.
    chart_selected: Cell<Option<(usize, usize)>>,
    /// A chart held by the pointer — drawn where the drag has it until the button comes up, which
    /// is one `App::reshape_chart`.
    chart_grab: Cell<Option<crate::sheet::chart::Grab>>,
}

impl Pane {
    pub fn new(app: Arc<grind_sheet::App>) -> Rc<Pane> {
        let text = CoreText::new(BASE_PT);
        let grid = Grid::measured(&app, 0, &text);
        Rc::new_cyclic(|me| Pane {
            me: me.clone(),
            app,
            sheet: Cell::new(0),
            grid: RefCell::new(grid),
            text,
            selection: Cell::new(Selection::default()),
            views: RefCell::new(Vec::new()),
            grid_view: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
            kept: RefCell::new(Vec::new()),
            name_box: RefCell::new(None),
            bands: RefCell::new(None),
            edit: RefCell::new(None),
            banner: RefCell::new(None),
            text_listeners: RefCell::new(Vec::new()),
            on_change: RefCell::new(None),
            turn: Cell::new(Turn::Done),
            document_listeners: RefCell::new(Vec::new()),
            find_bar: RefCell::new(None),
            friendly: Cell::new(true),
            overlays: Cell::new(grind_sheet::view::Overlays::NONE),
            announced: RefCell::new(String::new()),
            resizing: Cell::new(None),
            formulas: Cell::new(false),
            calculations: RefCell::new(None),
            chart_selected: Cell::new(None),
            chart_grab: Cell::new(None),
        })
    }

    /// The selected chart's index, if it is on the sheet showing and still exists.
    fn chart_now(&self) -> Option<usize> {
        let (sheet, index) = self.chart_selected.get()?;
        let count = self
            .app
            .charts(self.sheet.get())
            .map_or(0, |charts| charts.len());
        (sheet == self.sheet.get() && index < count).then_some(index)
    }

    fn select_chart(&self, index: Option<usize>) {
        self.chart_selected
            .set(index.map(|index| (self.sheet.get(), index)));
    }

    /// The window's find bar.
    pub fn set_find_bar(&self, bar: Retained<FindBar>) {
        *self.find_bar.borrow_mut() = Some(bar);
    }

    /// `performFindPanelAction:` — which of Find's items, by its tag (`find_bar.rs`).
    pub fn find_panel_action(&self, tag: isize) {
        let bar = self.find_bar.borrow().clone();
        if let Some(bar) = bar {
            bar.action(tag);
        }
    }

    /// Call `listener` whenever the document or the sheet showing changes.
    pub fn listen_document(&self, listener: impl Fn() + 'static) {
        self.document_listeners
            .borrow_mut()
            .push(Box::new(listener));
    }

    fn tell_document(&self) {
        for listener in self.document_listeners.borrow().iter() {
            listener();
        }
    }

    /// This pane, weakly.
    pub fn me(&self) -> RcWeak<Pane> {
        self.me.clone()
    }

    /// The grid view, while it exists.
    pub fn grid_view(&self) -> Option<Retained<GridView>> {
        self.grid_view.borrow().as_ref().and_then(Weak::load)
    }

    /// The window's banner.
    pub fn set_banner(&self, banner: Banner) {
        *self.banner.borrow_mut() = Some(banner);
    }

    /// Call `listener` with the text being edited — or the active cell's — from now on.
    pub fn listen_text(&self, listener: impl Fn(&str) + 'static) {
        listener(&self.edit_text());
        self.text_listeners.borrow_mut().push(Box::new(listener));
    }

    /// Tell the text listeners what the edit, or the active cell, now says.
    pub fn edit_changed(&self) {
        // With View ▸ Names on, a formula is read through the names it uses — `=rate*subtotal`
        // where the file stores `=[.B2]*[.B7]` (`App::named_formula`, `doc/view-modes.md` §3.3).
        let named = (!self.is_editing() && self.overlays.get().names)
            .then(|| {
                self.app
                    .named_formula(self.sheet.get(), self.selection.get().active)
                    .ok()
                    .flatten()
            })
            .flatten();
        let text = crate::sheet::assist::read_out(
            &named.unwrap_or_else(|| self.edit_text()),
            self.is_editing(),
            self.friendly.get(),
        );
        for listener in self.text_listeners.borrow().iter() {
            listener(&text);
        }
    }

    /// What to do when the document changes — the document marks itself edited.
    pub fn on_change(&self, callback: impl Fn(Turn) + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(callback));
    }

    /// The core said the document changed (architecture rule 3: it pushes, shells never poll):
    /// read the sheet's axes again — an edit may have widened the used extent — resize the views,
    /// redraw them, tell the listeners, and mark the document edited.
    fn changed(&self) {
        // A sheet deleted — or one taken back by an undo — may be the one showing.
        let count = self.app.sheet_count().max(1);
        if self.sheet.get() >= count {
            self.sheet.set(count - 1);
            self.selection.set(Selection::default());
        }
        self.tell_document();
        let grid = Grid::measured(&self.app, self.sheet.get(), &self.text);
        let (w, h) = grid.size();
        *self.grid.borrow_mut() = grid;
        if let Some(view) = self.grid_view() {
            view.setFrameSize(NSSize::new(HEADER_W + w, HEADER_H + h));
        }
        if let Some((columns, rows)) = self.bands.borrow().as_ref() {
            if let Some(columns) = columns.load() {
                columns.setFrameSize(NSSize::new(w, HEADER_H));
            }
            if let Some(rows) = rows.load() {
                rows.setFrameSize(NSSize::new(HEADER_W, h));
            }
        }
        self.reset_band_cursors();
        for view in self.views.borrow().iter().filter_map(Weak::load) {
            view.setNeedsDisplay(true);
        }
        let selection = self.selection.get();
        for listener in self.listeners.borrow().iter() {
            listener(selection);
        }
        self.edit_changed();
        if let Some(callback) = self.on_change.borrow().as_ref() {
            callback(self.turn.get());
        }
    }

    /// The views sized to the grid as it stands, its bands' cursors placed again, and all of it
    /// drawn — after a drag changed a track's size on screen without the document knowing yet.
    fn relayout(&self) {
        let (w, h) = self.grid.borrow().size();
        if let Some(view) = self.grid_view() {
            view.setFrameSize(NSSize::new(HEADER_W + w, HEADER_H + h));
        }
        self.reset_band_cursors();
        for view in self.views.borrow().iter().filter_map(Weak::load) {
            view.setNeedsDisplay(true);
        }
    }

    /// The bands' resize cursors, placed again over edges that may have moved.
    fn reset_band_cursors(&self) {
        if let Some((columns, rows)) = self.bands.borrow().as_ref() {
            for band in [
                columns.load().map(Retained::into_super),
                rows.load().map(Retained::into_super),
            ]
            .into_iter()
            .flatten()
            {
                if let Some(window) = band.window() {
                    window.invalidateCursorRectsForView(&band);
                }
            }
        }
    }

    /// The pointer at `at` along a band: a track picked up when it is on an edge — and fitted
    /// instead, on a double-click — answering whether it was.
    fn grab_edge(&self, axis: Axis, at: f64, clicks: isize) -> bool {
        let track = {
            let grid = self.grid.borrow();
            match axis {
                Axis::Columns => resize::edge(&grid.cols, grid.shown_cols, at),
                Axis::Rows => resize::edge(&grid.rows, grid.shown_rows, at),
            }
        };
        let Some(track) = track else {
            return false;
        };
        if clicks >= 2 {
            self.resizing.set(None);
            self.fit_track(axis, track);
            return true;
        }
        let size = {
            let grid = self.grid.borrow();
            match axis {
                Axis::Columns => grid.cols.size_of(track),
                Axis::Rows => grid.rows.size_of(track),
            }
        };
        self.resizing.set(Some(Drag {
            axis,
            track,
            from: at,
            size,
        }));
        true
    }

    /// A drag under way: the track drawn at its new size, and nothing written. Answers whether a
    /// track was being dragged at all.
    fn drag_edge(&self, at: f64) -> bool {
        let Some(drag) = self.resizing.get() else {
            return false;
        };
        let size = drag.size_at(at);
        {
            let mut grid = self.grid.borrow_mut();
            match drag.axis {
                Axis::Columns => grid.cols = grid.cols.with(drag.track, size),
                Axis::Rows => grid.rows = grid.rows.with(drag.track, size),
            }
        }
        if let Some((columns, rows)) = self.bands.borrow().as_ref() {
            let (w, h) = self.grid.borrow().size();
            if let Some(columns) = columns.load() {
                columns.setFrameSize(NSSize::new(w, HEADER_H));
            }
            if let Some(rows) = rows.load() {
                rows.setFrameSize(NSSize::new(HEADER_W, h));
            }
        }
        self.relayout();
        true
    }

    /// The button up: the size the drag ended at, written over every track it sizes as one
    /// undo step — or, when the pointer never moved, nothing.
    fn drop_edge(&self, at: f64) -> bool {
        let Some(drag) = self.resizing.take() else {
            return false;
        };
        if at == drag.from {
            return true;
        }
        let tracks = resize::tracks(self.selection.get(), drag.axis, drag.track);
        let length = Some(resize::length(drag.size_at(at)));
        let sheet = self.sheet.get();
        let done = match drag.axis {
            Axis::Columns => self.app.set_col_width(sheet, tracks, length),
            Axis::Rows => self.app.set_row_height(sheet, tracks, length),
        };
        if let Err(error) = done {
            self.say(Some((&error.to_string(), None)));
            // What the drag drew is not what the document holds; draw what it does.
            *self.grid.borrow_mut() = Grid::measured(&self.app, sheet, &self.text);
            self.relayout();
        }
        true
    }

    /// A double-click on an edge: a column as wide as its widest text, a row given back to its
    /// content — its own height taken away, as the GNOME window does.
    fn fit_track(&self, axis: Axis, track: u32) {
        let sheet = self.sheet.get();
        let tracks = resize::tracks(self.selection.get(), axis, track);
        let done = match axis {
            Axis::Columns => self.fit(tracks, 0..0),
            Axis::Rows => self.app.set_row_height(sheet, tracks, None).map(|_| ()),
        };
        if let Err(error) = done {
            self.say(Some((&error.to_string(), None)));
        }
    }

    /// Each of `cols` as wide as its widest text and every row of `rows` given back to its
    /// content, as one undo step — a double-click on an edge, Format ▸ Column ▸ Fit Width to
    /// Content and Format ▸ Fit Content to Cells (`App::fit`).
    pub fn fit(
        &self,
        cols: std::ops::Range<u32>,
        rows: std::ops::Range<u32>,
    ) -> Result<(), grind_sheet::Error> {
        let sheet = self.sheet.get();
        let widths = cols
            .map(|col| (col, resize::fit_width(&self.app, sheet, col, &self.text)))
            .collect();
        self.app.fit(sheet, widths, rows).map(|_| ())
    }

    /// Where the bands' resize cursors go: a strip [`resize::GRAB`] either side of every shown
    /// edge inside `visible`, in the band's own coordinates.
    fn edge_strips(&self, axis: Axis, visible: Rect) -> Vec<Rect> {
        let grid = self.grid.borrow();
        let (sizes, range) = match axis {
            Axis::Columns => (&grid.cols, grid.cols_in(visible.x, visible.w)),
            Axis::Rows => (&grid.rows, grid.rows_in(visible.y, visible.h)),
        };
        range
            .filter(|track| sizes.size_of(*track) > 0.0)
            .map(|track| {
                let end = sizes.offset_of(track) + sizes.size_of(track) - resize::GRAB;
                match axis {
                    Axis::Columns => Rect::new(end, visible.y, 2.0 * resize::GRAB, visible.h),
                    Axis::Rows => Rect::new(visible.x, end, visible.w, 2.0 * resize::GRAB),
                }
            })
            .collect()
    }

    /// Show another sheet: its own axes, the views resized to them, the cursor home and the view
    /// at the top left — what choosing a sheet in the sidebar does.
    pub fn show_sheet(&self, sheet: usize) {
        if sheet == self.sheet.get() || sheet >= self.app.sheet_count() {
            return;
        }
        self.sheet.set(sheet);
        let grid = Grid::measured(&self.app, sheet, &self.text);
        let (w, h) = grid.size();
        *self.grid.borrow_mut() = grid;
        if let Some(view) = self.grid_view.borrow().as_ref().and_then(Weak::load) {
            view.setFrameSize(NSSize::new(HEADER_W + w, HEADER_H + h));
            view.scrollPoint(NSPoint::new(0.0, 0.0));
        }
        if let Some((columns, rows)) = self.bands.borrow().as_ref() {
            if let Some(columns) = columns.load() {
                columns.setFrameSize(NSSize::new(w, HEADER_H));
            }
            if let Some(rows) = rows.load() {
                rows.setFrameSize(NSSize::new(HEADER_W, h));
            }
        }
        self.tell_document();
        // Another sheet is another selection, even when both are at A1: the listeners' words
        // depend on the sheet, so they are told whether or not the cell moved.
        self.selection.set(Selection::default());
        for view in self.views.borrow().iter().filter_map(Weak::load) {
            view.setNeedsDisplay(true);
        }
        for listener in self.listeners.borrow().iter() {
            listener(Selection::default());
        }
    }

    /// View ▸ Calculations…: the sidebar's Calculations section narrowed to `needle` — every
    /// formula when it is empty — or, with `None`, gone.
    pub fn show_calculations(&self, needle: Option<String>) {
        *self.calculations.borrow_mut() = needle;
        self.tell_document();
    }

    /// View ▸ Formulas, on or off — a redraw and nothing else.
    pub fn toggle_formulas(&self) {
        self.formulas.set(!self.formulas.get());
        for view in self.views.borrow().iter().filter_map(Weak::load) {
            view.setNeedsDisplay(true);
        }
    }

    /// View ▸ Cell Roles or Names, on or off — a redraw and nothing else: an overlay is read
    /// fresh on every paint and never stored in the document.
    pub fn toggle_overlay(&self, roles: bool) {
        let mut overlays = self.overlays.get();
        match roles {
            true => overlays.roles = !overlays.roles,
            false => overlays.names = !overlays.names,
        }
        self.overlays.set(overlays);
        for view in self.views.borrow().iter().filter_map(Weak::load) {
            view.setNeedsDisplay(true);
        }
        self.edit_changed();
    }

    /// Remember the window's name box, for Go To.
    pub fn set_name_box(&self, field: &Retained<NSTextField>) {
        *self.name_box.borrow_mut() = Some(Weak::from_retained(field));
    }

    /// Go To: the keyboard into the name box, whose text a field selects as it takes focus, so
    /// what is typed replaces it.
    pub fn focus_name_box(&self) {
        let Some(field) = self.name_box.borrow().as_ref().and_then(Weak::load) else {
            return;
        };
        if let Some(window) = field.window() {
            window.makeFirstResponder(Some(&field));
        }
    }

    /// Keep `object` alive for as long as the pane is.
    pub fn keep(&self, object: Retained<NSObject>) {
        self.kept.borrow_mut().push(object);
    }

    /// Give the keyboard back to the grid — after the name box has sent the selection somewhere.
    pub fn focus_grid(&self) {
        let Some(grid) = self.grid_view.borrow().as_ref().and_then(Weak::load) else {
            return;
        };
        if let Some(window) = grid.window() {
            window.makeFirstResponder(Some(&grid));
        }
    }

    /// Call `listener` with every new selection from now on, and once with the current one.
    pub fn listen(&self, listener: impl Fn(Selection) + 'static) {
        listener(self.selection.get());
        self.listeners.borrow_mut().push(Box::new(listener));
    }

    /// Make `selection` the selection: redraw, scroll the active cell into sight, and say so.
    pub fn select(&self, selection: Selection) {
        // A single cell inside a merge is the merge — its top-left cell — however it was
        // reached: a click, a go-to, a sidebar row.
        let merges = grind_sheet::nav::merges(&self.app, self.sheet.get());
        let selection = grind_sheet::nav::onto_merge(selection, &merges);
        if selection == self.selection.get() {
            return;
        }
        self.selection.set(selection);
        for view in self.views.borrow().iter().filter_map(Weak::load) {
            view.setNeedsDisplay(true);
        }
        if let Some(grid) = self.grid_view.borrow().as_ref().and_then(Weak::load) {
            let reveal = select::reveal(&self.grid.borrow(), selection);
            grid.scrollRectToVisible(ns_rect(reveal));
        }
        for listener in self.listeners.borrow().iter() {
            listener(selection);
        }
        self.edit_changed();
        self.announce(selection);
    }

    /// Say where the cursor went and what the cell shows — the accessibility floor's grid half,
    /// `ui_sheet_gtk`'s `announce` in the Mac's terms.
    fn announce(&self, selection: Selection) {
        let said = crate::a11y::grid_announcement(&self.app, self.sheet.get(), selection);
        if let Some(grid) = self.grid_view() {
            let text = NSString::from_str(&said);
            let value: &AnyObject = &text;
            // SAFETY: the key is a constant AppKit exports, and its value a string.
            let key: &NSString = unsafe { NSAccessibilityAnnouncementKey };
            let info = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[value]);
            // SAFETY: the grid view is an accessibility element; the name is AppKit's own, and
            // the user info the dictionary its documentation asks for.
            unsafe {
                NSAccessibilityPostNotificationWithUserInfo(
                    &grid,
                    NSAccessibilityAnnouncementRequestedNotification,
                    Some(&info),
                )
            };
        }
        *self.announced.borrow_mut() = said;
    }
}

/// How many distinct values a filter button's menu lists — the GNOME window's and the browser's
/// ceiling, past which a list is not a list.
const FILTER_VALUES: usize = 500;

/// What a filter button's menu is about: the pane, the field, and the values its rows name, by
/// tag.
pub struct Choice {
    pane: RcWeak<Pane>,
    field: u32,
    values: Vec<String>,
}

define_class!(
    /// A filter button's menu's target: a row hides or shows its value, Show All drops the
    /// field's condition — each one `App::set_filter`, one undo step.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindFilterChoice"]
    #[ivars = Choice]
    pub struct FilterChoice;

    impl FilterChoice {
        #[unsafe(method(filterChose:))]
        fn chose(&self, sender: &NSMenuItem) {
            self.choose(sender.tag());
        }
    }
);

impl FilterChoice {
    fn choose(&self, tag: isize) {
        let choice = self.ivars();
        let Some(pane) = choice.pane.upgrade() else {
            return;
        };
        let sheet = pane.sheet.get();
        let Some(filter) = pane.app.filter(sheet).ok().flatten() else {
            return;
        };
        let next = match usize::try_from(tag)
            .ok()
            .and_then(|at| choice.values.get(at))
        {
            Some(value) => {
                crate::sheet::filter::toggled(&filter, choice.field, value, &choice.values)
            }
            None => crate::sheet::filter::shown_all(&filter, choice.field),
        };
        if let Err(error) = pane.app.set_filter(sheet, Some(next)) {
            pane.say(Some((&error.to_string(), None)));
        }
    }
}

/// A chart menu's tags: the title, the three kinds from [`CHART_KIND`], the five legends from
/// [`CHART_LEGEND`], and removal.
const CHART_TITLE: isize = 0;
const CHART_KIND: isize = 10;
const CHART_LEGEND: isize = 20;
const CHART_DELETE: isize = 30;

const CHART_KINDS: [(&str, ChartKind); 3] = [
    ("Bar Chart", ChartKind::Bar),
    ("Line Chart", ChartKind::Line),
    ("Pie Chart", ChartKind::Pie),
];

const CHART_LEGENDS: [(&str, Option<ChartLegend>); 5] = [
    ("No Legend", None),
    ("Legend on the Right", Some(ChartLegend::End)),
    ("Legend at the Bottom", Some(ChartLegend::Bottom)),
    ("Legend at the Top", Some(ChartLegend::Top)),
    ("Legend on the Left", Some(ChartLegend::Start)),
];

define_class!(
    /// A chart's context menu's target: the pane and which chart.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindChartChoice"]
    #[ivars = (RcWeak<Pane>, usize)]
    pub struct ChartChoice;

    impl ChartChoice {
        #[unsafe(method(chartChose:))]
        fn chose(&self, sender: &NSMenuItem) {
            self.choose(sender.tag());
        }
    }
);

impl ChartChoice {
    fn choose(&self, tag: isize) {
        let (pane, index) = self.ivars();
        let Some(pane) = pane.upgrade() else {
            return;
        };
        let sheet = pane.sheet.get();
        let Some(chart) = pane
            .app
            .charts(sheet)
            .ok()
            .and_then(|charts| charts.get(*index).cloned())
        else {
            return;
        };
        let change = match tag {
            CHART_DELETE => {
                if let Err(error) = pane.app.remove_chart(sheet, *index) {
                    pane.say(Some((&error.to_string(), None)));
                }
                return;
            }
            CHART_TITLE => {
                let Some(title) = crate::prompt::ask(
                    self.mtm(),
                    "Chart Title",
                    "What the chart is called, over it — or nothing for no title.",
                    "Set",
                    chart.title.as_deref().unwrap_or_default(),
                ) else {
                    return;
                };
                Change::Title(Some(title))
            }
            tag if (CHART_KIND..CHART_KIND + 3).contains(&tag) => {
                Change::Kind(CHART_KINDS[(tag - CHART_KIND) as usize].1)
            }
            tag if (CHART_LEGEND..CHART_LEGEND + 5).contains(&tag) => {
                Change::Legend(CHART_LEGENDS[(tag - CHART_LEGEND) as usize].1)
            }
            _ => return,
        };
        let spec = crate::sheet::chart::changed(&chart, change);
        if let Err(error) = pane.app.edit_chart(sheet, *index, &spec) {
            pane.say(Some((&error.to_string(), None)));
        }
    }
}

/// The cursor a selected chart's handle shows: AppKit's own for resizing a rectangular frame from
/// that edge or corner, both ways — the view is flipped, so north is the top.
fn frame_cursor(grip: grind_sheet::chart_frame::Grip) -> Retained<NSCursor> {
    use grind_sheet::chart_frame::Grip;
    let position = match grip {
        Grip::North | Grip::Body => NSCursorFrameResizePosition::Top,
        Grip::NorthEast => NSCursorFrameResizePosition::TopRight,
        Grip::East => NSCursorFrameResizePosition::Right,
        Grip::SouthEast => NSCursorFrameResizePosition::BottomRight,
        Grip::South => NSCursorFrameResizePosition::Bottom,
        Grip::SouthWest => NSCursorFrameResizePosition::BottomLeft,
        Grip::West => NSCursorFrameResizePosition::Left,
        Grip::NorthWest => NSCursorFrameResizePosition::TopLeft,
    };
    NSCursor::frameResizeCursorFromPosition_inDirections(
        position,
        NSCursorFrameResizeDirections::All,
    )
}

/// Have the core tell `pane` about every change to its document from now on.
pub fn watch(pane: &Rc<Pane>) {
    pane.app.set_observer(crate::watch::observer(pane));
}

/// A colour AppKit resolves for the current drawing appearance, as three bytes. A colour with no
/// sRGB form — a pattern — is a mid grey, which reads on either appearance.
pub(crate) fn rgb(color: &NSColor) -> Rgb {
    let Some(srgb) = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) else {
        return (0x80, 0x80, 0x80);
    };
    let byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    (
        byte(srgb.redComponent()),
        byte(srgb.greenComponent()),
        byte(srgb.blueComponent()),
    )
}

/// The system's semantic colours, resolved in the appearance AppKit is drawing in — a view's
/// `drawRect:` runs with the view's own `effectiveAppearance` current, so dark mode, high contrast,
/// the user's accent and a window forced into one appearance all follow with nothing here knowing
/// (decision 8).
pub(crate) fn palette() -> Palette {
    let page = rgb(&NSColor::textBackgroundColor());
    Palette {
        page,
        ink: rgb(&NSColor::textColor()),
        grid: rgb(&NSColor::gridColor()),
        header: rgb(&NSColor::controlBackgroundColor()),
        header_ink: rgb(&NSColor::secondaryLabelColor()),
        accent: rgb(&NSColor::controlAccentColor()),
        dark: color::luminance(page) < 0.5,
    }
}

/// One device pixel, in points — how thin a grid line is on the screen `view` is on.
fn hairline(view: &NSView) -> f64 {
    view.window()
        .map_or(1.0, |window| window.backingScaleFactor())
        .max(1.0)
        .recip()
}

pub(crate) fn rect(frame: NSRect) -> Rect {
    Rect::new(
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    )
}

pub(crate) fn ns_rect(rect: Rect) -> NSRect {
    NSRect::new(NSPoint::new(rect.x, rect.y), NSSize::new(rect.w, rect.h))
}

/// The context AppKit is drawing into right now, and `ops` put down on it.
fn draw(ops: &[Op], pane: &Pane) {
    let Some(context) = NSGraphicsContext::currentContext() else {
        return;
    };
    let cg: Retained<CGContext> = context.CGContext();
    render::draw(&cg, ops, &pane.text);
}

/// Where a mouse event happened, in `view`'s own coordinates.
pub(crate) fn located(view: &NSView, event: &NSEvent) -> NSPoint {
    view.convertPoint_fromView(event.locationInWindow(), None)
}

pub(crate) fn shifted(event: &NSEvent) -> bool {
    event.modifierFlags().contains(NSEventModifierFlags::Shift)
}

define_class!(
    /// The cells, as the scroll view's document view.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindGridView"]
    #[ivars = Rc<Pane>]
    pub struct GridView;

    impl GridView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            let pane = self.ivars();
            let palette = palette();
            let look = Look {
                palette: &palette,
                metrics: &pane.text,
                hairline: hairline(self),
                overlays: pane.overlays.get(),
                formulas: pane.formulas.get(),
            };
            // The dirty rectangle in the sheet's own coordinates: the view keeps a margin the
            // header bands float over.
            let view = rect(dirty).offset(-HEADER_W, -HEADER_H);
            // The filter's buttons sit in their heading cells; the charts float over everything.
            let buttons = pane
                .app
                .filter(pane.sheet.get())
                .ok()
                .flatten()
                .map(|filter| {
                    crate::sheet::filter::ops(&filter, &pane.grid.borrow(), &view, &palette)
                })
                .unwrap_or_default();
            let charts = crate::sheet::chart::charts(
                &pane.app,
                pane.sheet.get(),
                &view,
                &palette,
                &pane.text,
                crate::sheet::chart::Held {
                    selected: pane.chart_now(),
                    grab: pane.chart_grab.get(),
                },
            );
            let ops: Vec<Op> = paint::cells(
                &pane.app,
                pane.sheet.get(),
                &pane.grid.borrow(),
                view,
                pane.selection.get(),
                &look,
            )
            .into_iter()
            .chain(buttons)
            .chain(charts)
            // The cells a formula being typed is pointing at, over everything.
            .chain(pane.pointed().into_iter().flat_map(|pointed| {
                paint::selection_outline(
                    &pane.grid.borrow(),
                    &view,
                    pointed,
                    crate::editor::POINTED,
                )
            }))
            .map(|op| op.shifted(HEADER_W, HEADER_H))
            .collect();
            draw(&ops, pane);
        }

        /// A key goes to the text system, which answers with what it means for this user — a
        /// selector, handled below, or text, which starts an edit. ⌃U, Excel for Mac's key for
        /// editing the active cell, is bound to nothing in the standard bindings, so it is read
        /// here first.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            crate::drive::hear("keyDown:");
            let control = event.modifierFlags().contains(NSEventModifierFlags::Control);
            let u = event
                .charactersIgnoringModifiers()
                .is_some_and(|keys| keys.to_string() == "u");
            if control && u {
                self.ivars().begin_edit(Seed::Cell);
                return;
            }
            self.interpretKeyEvents(&NSArray::from_slice(&[event]));
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, selector: Sel) {
            let name = selector.name().to_str().unwrap_or_default();
            crate::drive::hear(name);
            // A selected chart has the keys it uses before the grid does.
            if self.chart_key(name) {
                return;
            }
            match keys::grid_action(name) {
                Some(action) => self.act(action),
                // A key with no meaning here is the platform's beep, as in every other view.
                None => NSBeep(),
            }
        }

        /// Text typed on the grid starts an edit, seeded with it — whatever an input method
        /// committed, which may be a word.
        #[unsafe(method(insertText:))]
        fn insert_text(&self, text: &AnyObject) {
            let typed = text
                .downcast_ref::<NSString>()
                .map(|text| text.to_string())
                .or_else(|| {
                    text.downcast_ref::<NSAttributedString>()
                        .map(|text| text.string().to_string())
                })
                .unwrap_or_default();
            crate::drive::hear(format!("insertText:{typed:?}"));
            // Typing lets go of a selected chart and means what it always means.
            if self.ivars().chart_now().is_some() {
                self.ivars().select_chart(None);
                self.chart_changed();
            }
            // Space on a checkbox's cell ticks it, rather than starting an edit with a space.
            let pane = self.ivars();
            let selection = pane.selection.get();
            if typed == " "
                && selection.is_single()
                && pane
                    .app
                    .checkboxes(pane.sheet.get())
                    .is_ok_and(|b| b.iter().any(|(p, _)| *p == selection.active))
            {
                let _ = pane.app.toggle_checkbox(pane.sheet.get(), selection.active);
                return;
            }
            if let Some(seed) = state::typed(Mode::Ready, &typed) {
                self.ivars().begin_edit(seed);
            }
        }

        /// A click selects; a double-click opens the cell to amend it; a press on a chart picks
        /// it up to be dragged somewhere else.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if self.filter_button(event) || self.grab_chart(event) {
                return;
            }
            self.click(event, shifted(event));
            if event.clickCount() == 2 {
                self.ivars().begin_edit(Seed::Cell);
            }
        }

        /// Edit ▸ Copy, Cut, Paste and Delete — `clipboard.rs`.
        /// Format ▸ Font ▸ Show Fonts: the panel told what the selection is set in first, so
        /// it opens on it (M7).
        #[unsafe(method(orderFrontFontPanel:))]
        fn order_front_font_panel(&self, sender: Option<&AnyObject>) {
            self.ivars().show_font(self.mtm());
            // SAFETY: the sender is whatever sent this action, which is what the font
            // manager's own action takes.
            unsafe { NSFontManager::sharedFontManager(self.mtm()).orderFrontFontPanel(sender) };
        }

        /// The font panel's answer (`formatting.rs`).
        #[unsafe(method(changeFont:))]
        fn change_font(&self, _sender: Option<&AnyObject>) {
            self.ivars().change_font(self.mtm());
        }

        /// The colour panel's colour, as the text's.
        #[unsafe(method(changeColor:))]
        fn change_color(&self, sender: Option<&AnyObject>) {
            let color = sender
                .and_then(|sender| sender.downcast_ref::<NSColorPanel>())
                .map(NSColorPanel::color)
                .unwrap_or_else(|| NSColorPanel::sharedColorPanel(self.mtm()).color());
            self.ivars().change_color(&color);
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            self.ivars().copy();
        }

        #[unsafe(method(cut:))]
        fn cut(&self, _sender: Option<&AnyObject>) {
            self.ivars().cut();
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            self.ivars().paste();
        }

        #[unsafe(method(delete:))]
        fn delete(&self, _sender: Option<&AnyObject>) {
            self.ivars().clear();
        }

        /// Edit ▸ Select All: everything the sheet uses, the view going home.
        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            let pane = self.ivars();
            pane.select(grind_sheet::nav::all(&pane.app, pane.sheet.get()));
        }

        /// Edit ▸ Find's four items, told apart by the sender's tag (`find_bar.rs`).
        #[unsafe(method(performFindPanelAction:))]
        fn perform_find_panel_action(&self, sender: Option<&AnyObject>) {
            let tag = sender
                .and_then(|sender| sender.downcast_ref::<NSMenuItem>())
                .map_or(0, |item| item.tag());
            self.ivars().find_panel_action(tag);
        }

        /// Edit ▸ Undo — the core's history, never AppKit's (architecture rule 2).
        #[unsafe(method(undo:))]
        fn undo(&self, _sender: Option<&AnyObject>) {
            let pane = self.ivars();
            pane.turn.set(Turn::Undone);
            pane.app.undo();
            pane.turn.set(Turn::Done);
        }

        #[unsafe(method(redo:))]
        fn redo(&self, _sender: Option<&AnyObject>) {
            let pane = self.ivars();
            pane.turn.set(Turn::Redone);
            pane.app.redo();
            pane.turn.set(Turn::Done);
        }

        /// Undo and Redo are grey when the core has nothing to take back or bring back — the
        /// named gap `ui_win32` has, closed by the platform's own mechanism.
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let app = &self.ivars().app;
            match item.action().map(|action| action.name().to_str().unwrap_or_default().to_owned()) {
                Some(name) if name == "undo:" => app.can_undo(),
                Some(name) if name == "redo:" => app.can_redo(),
                Some(name) if name == "paste:" => Pane::can_paste(),
                _ => true,
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if !self.drag_chart(event) {
                self.click(event, true);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            self.drop_chart();
        }

        /// Over a chart the pointer says what a drag there does: the open hand on its body, a
        /// resize cursor on each of the selected one's handles.
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            let pane = self.ivars();
            let sheet = pane.sheet.get();
            let frames = crate::sheet::chart::frames(&pane.app, sheet);
            let at = |r: grind_sheet::chart_frame::Frame| {
                ns_rect(Rect::new(r.x + HEADER_W, r.y + HEADER_H, r.w, r.h))
            };
            let hand = NSCursor::openHandCursor();
            for frame in frames.iter().flatten() {
                self.addCursorRect_cursor(at(*frame), &hand);
            }
            if let Some(Some(frame)) = pane.chart_now().map(|index| frames.get(index).copied().flatten())
            {
                let reach = grind_sheet::chart_frame::HANDLE + 2.0 * grind_sheet::chart_frame::HANDLE_SLOP;
                for (grip, square) in grind_sheet::chart_frame::handles(&frame, reach) {
                    self.addCursorRect_cursor(at(square), &frame_cursor(grip));
                }
            }
        }

        /// A right-click, or a Control-click: the cells' context menu, from `menu.rs`'s table.
        /// A click outside the selection moves it there first, as every Mac grid does, so the
        /// menu acts on what was clicked.
        #[unsafe(method_id(menuForEvent:))]
        fn menu_for_event(&self, event: &NSEvent) -> Option<Retained<NSMenu>> {
            self.menu_for(event)
        }
    }
);

impl GridView {
    /// A click on one of the filter's buttons: the field's values as a menu under it, each ticked
    /// while it is shown, and Show All. Answers whether the click was on a button.
    fn filter_button(&self, event: &NSEvent) -> bool {
        let pane = self.ivars();
        let sheet = pane.sheet.get();
        let Some(filter) = pane.app.filter(sheet).ok().flatten() else {
            return false;
        };
        let at = located(self, event);
        let grid = pane.grid.borrow().clone();
        let Some(field) =
            crate::sheet::filter::button_at(&filter, &grid, at.x - HEADER_W, at.y - HEADER_H)
        else {
            return false;
        };
        let col = filter.column(field);
        let Ok(cells) = pane
            .app
            .get_viewport(sheet, 0..filter.end.row + 1, col..col + 1)
        else {
            return true;
        };
        let values = grind_sheet::filter::offered(&cells, &filter, field, FILTER_VALUES);
        let mtm = self.mtm();
        let target: Retained<FilterChoice> = {
            let this = FilterChoice::alloc(mtm).set_ivars(Choice {
                pane: pane.me.clone(),
                field,
                values: values.clone(),
            });
            // SAFETY: `init` is `NSObject`'s designated initialiser.
            unsafe { msg_send![super(this), init] }
        };
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        let item = |title: &str, tag: isize| {
            // SAFETY: an item made with its title, the target's own action and no key.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(sel!(filterChose:)),
                    &NSString::from_str(""),
                )
            };
            // SAFETY: the target outlives the menu, which is shown and closed within this call.
            unsafe { item.setTarget(Some(&target)) };
            item.setTag(tag);
            item
        };
        let all = item("Show All", -1);
        all.setEnabled(filter.keep.contains_key(&field));
        menu.addItem(&all);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        for (index, value) in values.iter().enumerate() {
            let title = match value.is_empty() {
                true => "(empty)",
                false => value.as_str(),
            };
            let row = item(title, index as isize);
            row.setState(match crate::sheet::filter::shown(&filter, field, value) {
                true => NSControlStateValueOn,
                false => NSControlStateValueOff,
            });
            menu.addItem(&row);
        }
        let buttons = crate::sheet::filter::buttons(&filter, &grid);
        let below = buttons
            .iter()
            .find(|(at, _)| *at == field)
            .map_or(at, |(_, rect)| {
                NSPoint::new(rect.x + HEADER_W, rect.bottom() + HEADER_H)
            });
        menu.popUpMenuPositioningItem_atLocation_inView(None, below, Some(self));
        true
    }

    /// A right-click on a chart: its own menu — its title, its kind, its legend, and taking it
    /// away — each one `App::edit_chart` or `App::remove_chart`, one undo step.
    /// It selects the chart first, so what the menu acts on is the chart that is outlined.
    fn chart_menu(&self, event: &NSEvent) -> Option<Retained<NSMenu>> {
        let pane = self.ivars();
        let sheet = pane.sheet.get();
        let at = located(self, event);
        let (index, _) = crate::sheet::chart::hit(
            &pane.app,
            sheet,
            pane.chart_now(),
            at.x - HEADER_W,
            at.y - HEADER_H,
        )?;
        pane.select_chart(Some(index));
        self.chart_changed();
        self.chart_menu_for(index)
    }

    fn chart_menu_for(&self, index: usize) -> Option<Retained<NSMenu>> {
        let pane = self.ivars();
        let sheet = pane.sheet.get();
        let chart = pane.app.charts(sheet).ok()?.get(index)?.clone();
        let mtm = self.mtm();
        let target: Retained<ChartChoice> = {
            let this = ChartChoice::alloc(mtm).set_ivars((pane.me.clone(), index));
            // SAFETY: `init` is `NSObject`'s designated initialiser.
            unsafe { msg_send![super(this), init] }
        };
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        let item = |title: &str, tag: isize, on: bool| {
            // SAFETY: an item made with its title, the target's own action and no key.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(sel!(chartChose:)),
                    &NSString::from_str(""),
                )
            };
            // SAFETY: an item holds its target weakly, so it holds it as its represented object
            // too — strongly, for exactly as long as the menu lives.
            unsafe {
                item.setTarget(Some(&target));
                item.setRepresentedObject(Some(&target));
            }
            item.setTag(tag);
            if on {
                item.setState(NSControlStateValueOn);
            }
            menu.addItem(&item);
        };
        item("Title…", CHART_TITLE, false);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        for (offset, (title, kind)) in CHART_KINDS.iter().enumerate() {
            item(title, CHART_KIND + offset as isize, chart.kind == *kind);
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        for (offset, (title, legend)) in CHART_LEGENDS.iter().enumerate() {
            item(
                title,
                CHART_LEGEND + offset as isize,
                chart.legend == *legend,
            );
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        item("Delete Chart", CHART_DELETE, false);
        // The key that does the same to a selected chart, shown beside it as every Mac menu
        // shows one: ⌫, with no modifier.
        if let Some(delete) = menu.itemWithTag(CHART_DELETE) {
            delete.setKeyEquivalent(&NSString::from_str("\u{8}"));
            delete.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
        }
        Some(menu)
    }

    /// A press on a chart (`doc/chart-handling.md`): it is selected and picked up — its body to
    /// move it, a handle of the selected one to resize it. A double click on one changes it.
    /// Answers whether there was a chart there; a press anywhere else lets go of one.
    fn grab_chart(&self, event: &NSEvent) -> bool {
        let pane = self.ivars();
        let sheet = pane.sheet.get();
        let at = located(self, event);
        let (x, y) = (at.x - HEADER_W, at.y - HEADER_H);
        let Some((index, grip)) =
            crate::sheet::chart::hit(&pane.app, sheet, pane.chart_now(), x, y)
        else {
            if pane.chart_now().is_some() {
                pane.select_chart(None);
                self.chart_changed();
            }
            return false;
        };
        pane.select_chart(Some(index));
        if event.clickCount() == 2 {
            self.change_chart(index);
            return true;
        }
        pane.chart_grab.set(crate::sheet::chart::Grab::new(
            &pane.app, sheet, index, grip, x, y,
        ));
        self.chart_changed();
        true
    }

    /// The chart being dragged drawn where the pointer has it; nothing written yet. ⇧ on a
    /// corner keeps its proportions.
    fn drag_chart(&self, event: &NSEvent) -> bool {
        let pane = self.ivars();
        let Some(mut grab) = pane.chart_grab.get() else {
            return false;
        };
        let at = located(self, event);
        let keep = event.modifierFlags().contains(NSEventModifierFlags::Shift);
        grab.follow(at.x - HEADER_W, at.y - HEADER_H, keep);
        pane.chart_grab.set(Some(grab));
        self.setNeedsDisplay(true);
        true
    }

    /// The button up over a held chart: a press that barely moved only selected it; any other
    /// writes its new frame once, one undo step.
    fn drop_chart(&self) {
        let pane = self.ivars();
        let Some(grab) = pane.chart_grab.take() else {
            return;
        };
        if !grab.is_click() {
            self.write_chart(grab.index, grab.now);
        }
        self.chart_changed();
    }

    /// A chart's frame, in the sheet's points, written back — one `App::reshape_chart`.
    fn write_chart(&self, index: usize, frame: grind_sheet::chart_frame::Frame) {
        let pane = self.ivars();
        let [x, y, w, h] = crate::sheet::chart::lengths(frame);
        if let Err(error) = pane
            .app
            .reshape_chart(pane.sheet.get(), index, &x, &y, &w, &h)
        {
            pane.say(Some((&error.to_string(), None)));
        }
    }

    /// The selection of a chart changed: drawn again, and the cursor rects placed again, since
    /// a selected chart's handles have cursors of their own.
    fn chart_changed(&self) {
        self.setNeedsDisplay(true);
        if let Some(window) = self.window() {
            window.invalidateCursorRectsForView(self);
        }
    }

    /// A selector while a chart is selected (`sheet::chart::key`): ⌫ deletes it, Escape lets go,
    /// the arrows nudge it (⇧ for a large step), Return opens its menu. Any other lets go of it and
    /// means what it always means — `false`.
    fn chart_key(&self, selector: &str) -> bool {
        use crate::sheet::chart::KeyAction;
        use grind_sheet::chart_frame::Key;
        let pane = self.ivars();
        let Some(index) = pane.chart_now() else {
            return false;
        };
        match crate::sheet::chart::key(selector) {
            None => {
                pane.select_chart(None);
                self.chart_changed();
                false
            }
            Some(KeyAction::Change) => {
                self.change_chart(index);
                true
            }
            Some(KeyAction::Key(Key::Delete)) => {
                pane.select_chart(None);
                match pane.app.remove_chart(pane.sheet.get(), index) {
                    Ok(()) => {
                        let said = grind_sheet::chart_frame::deleted_sentence("⌘Z");
                        pane.say(Some((&said, None)));
                    }
                    Err(error) => pane.say(Some((&error.to_string(), None))),
                }
                self.chart_changed();
                true
            }
            Some(KeyAction::Key(Key::Deselect)) => {
                pane.select_chart(None);
                self.chart_changed();
                true
            }
            Some(KeyAction::Key(Key::Nudge(dx, dy))) => {
                let frames = crate::sheet::chart::frames(&pane.app, pane.sheet.get());
                if let Some(Some(mut frame)) = frames.get(index).copied() {
                    frame.x += dx;
                    frame.y += dy;
                    self.write_chart(index, frame);
                }
                self.chart_changed();
                true
            }
        }
    }

    /// Return on a selected chart, or a double click on one: its own menu, under its top-left
    /// corner — the title, the kinds, the legends and Delete.
    fn change_chart(&self, index: usize) {
        let pane = self.ivars();
        let Some(Some(frame)) = crate::sheet::chart::frames(&pane.app, pane.sheet.get())
            .get(index)
            .copied()
        else {
            return;
        };
        let Some(menu) = self.chart_menu_for(index) else {
            return;
        };
        let at = NSPoint::new(frame.x + HEADER_W, frame.y + HEADER_H);
        menu.popUpMenuPositioningItem_atLocation_inView(None, at, Some(self));
    }

    /// A chart's own menu over a chart, and the cells' otherwise.
    fn menu_for(&self, event: &NSEvent) -> Option<Retained<NSMenu>> {
        if let Some(menu) = self.chart_menu(event) {
            return Some(menu);
        }
        self.context(event);
        Some(crate::app::context_menu(
            &crate::menu::GRID_CONTEXT,
            self.mtm(),
        ))
    }

    fn context(&self, event: &NSEvent) {
        let pane = self.ivars();
        let at = located(self, event);
        let (x, y) = (at.x - HEADER_W, at.y - HEADER_H);
        let clicked = select::click(&pane.grid.borrow(), Selection::default(), x, y, false);
        let (start, end) = pane.selection.get().rect();
        let cell = clicked.active;
        let inside =
            (start.row..=end.row).contains(&cell.row) && (start.col..=end.col).contains(&cell.col);
        if !inside {
            pane.select(clicked);
        }
    }

    fn act(&self, action: keys::GridAction) {
        let pane = self.ivars();
        let visible = rect(self.visibleRect());
        let grid = pane.grid.borrow();
        let page = select::page(&grid, visible.y, visible.h - HEADER_H);
        // The Delete keys empty the selection; every other action moves it.
        if action == keys::GridAction::Clear {
            drop(grid);
            pane.clear();
            return;
        }
        let next = select::apply(
            &pane.app,
            pane.sheet.get(),
            &grid,
            pane.selection.get(),
            action,
            page,
        );
        drop(grid);
        pane.select(next);
    }

    fn click(&self, event: &NSEvent, extend: bool) {
        let pane = self.ivars();
        let at = located(self, event);
        // A plain click on a checkbox's box ticks it — the linked cell's value — and selects it.
        if !extend
            && !pane.is_editing()
            && let Some(cell) = select::checkbox_click(
                &pane.app,
                pane.sheet.get(),
                &pane.grid.borrow(),
                at.x - HEADER_W,
                at.y - HEADER_H,
            )
        {
            pane.select(Selection::at(cell));
            let _ = pane.app.toggle_checkbox(pane.sheet.get(), cell);
            return;
        }
        let next = select::click(
            &pane.grid.borrow(),
            pane.selection.get(),
            at.x - HEADER_W,
            at.y - HEADER_H,
            extend,
        );
        // While a cell is being edited a click points at a cell or commits the edit first.
        if pane.is_editing() && !pane.click_while_editing(next, extend) {
            return;
        }
        pane.select(next);
    }
}

define_class!(
    /// The column letters, floating over the grid's top margin. A click selects the column; one
    /// on a column's right edge picks it up to be dragged wider or narrower, and a double-click
    /// there fits it to what it holds.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindColumnHeader"]
    #[ivars = Rc<Pane>]
    pub struct ColumnHeader;

    impl ColumnHeader {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            let pane = self.ivars();
            let dirty = rect(dirty);
            let palette = palette();
            let look = Look {
                palette: &palette,
                metrics: &pane.text,
                hairline: hairline(self),
                overlays: pane.overlays.get(),
                formulas: pane.formulas.get(),
            };
            let ops = paint::column_header(
                &pane.grid.borrow(),
                dirty.x,
                dirty.w,
                pane.selection.get(),
                &look,
            );
            draw(&ops, pane);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let at = located(self, event).x;
            if !self.ivars().grab_edge(Axis::Columns, at, event.clickCount()) {
                self.click(event, shifted(event));
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if !self.ivars().drag_edge(located(self, event).x) {
                self.click(event, true);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.ivars().drop_edge(located(self, event).x);
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            let cursor = NSCursor::columnResizeCursor();
            for strip in self.ivars().edge_strips(Axis::Columns, rect(self.visibleRect())) {
                self.addCursorRect_cursor(ns_rect(strip), &cursor);
            }
        }
    }
);

impl ColumnHeader {
    fn click(&self, event: &NSEvent, extend: bool) {
        let pane = self.ivars();
        let at = located(self, event);
        let next = select::column_click(&pane.grid.borrow(), pane.selection.get(), at.x, extend);
        pane.select(next);
    }
}

define_class!(
    /// The row numbers, floating over the grid's left margin. A click selects the row; one on a
    /// row's bottom edge picks it up to be dragged, and a double-click there gives the row back
    /// to its content.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindRowHeader"]
    #[ivars = Rc<Pane>]
    pub struct RowHeader;

    impl RowHeader {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            let pane = self.ivars();
            let dirty = rect(dirty);
            let palette = palette();
            let look = Look {
                palette: &palette,
                metrics: &pane.text,
                hairline: hairline(self),
                overlays: pane.overlays.get(),
                formulas: pane.formulas.get(),
            };
            let ops = paint::row_header(
                &pane.grid.borrow(),
                dirty.y,
                dirty.h,
                pane.selection.get(),
                &look,
            );
            draw(&ops, pane);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let at = located(self, event).y;
            if !self.ivars().grab_edge(Axis::Rows, at, event.clickCount()) {
                self.click(event, shifted(event));
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if !self.ivars().drag_edge(located(self, event).y) {
                self.click(event, true);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.ivars().drop_edge(located(self, event).y);
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            let cursor = NSCursor::rowResizeCursor();
            for strip in self.ivars().edge_strips(Axis::Rows, rect(self.visibleRect())) {
                self.addCursorRect_cursor(ns_rect(strip), &cursor);
            }
        }
    }
);

impl RowHeader {
    fn click(&self, event: &NSEvent, extend: bool) {
        let pane = self.ivars();
        let at = located(self, event);
        let next = select::row_click(&pane.grid.borrow(), pane.selection.get(), at.y, extend);
        pane.select(next);
    }
}

define_class!(
    /// Where the two bands meet, over both, so neither's labels show through it.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindCorner"]
    #[ivars = Rc<Pane>]
    pub struct Corner;

    impl Corner {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let palette = palette();
            let line = hairline(self);
            let ops = [
                Op::Fill {
                    rect: Rect::new(0.0, 0.0, HEADER_W, HEADER_H),
                    color: palette.header,
                },
                Op::Fill {
                    rect: Rect::new(0.0, HEADER_H - line, HEADER_W, line),
                    color: palette.grid,
                },
                Op::Fill {
                    rect: Rect::new(HEADER_W - line, 0.0, line, HEADER_H),
                    color: palette.grid,
                },
            ];
            draw(&ops, self.ivars());
        }
    }
);

fn frame(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// The spreadsheet's scroll view, with the grid in it and the bands floating over it. The views
/// hold `pane` between them, and the scroll view holds the views.
pub fn sheet_view(pane: &Rc<Pane>, size: NSSize, mtm: MainThreadMarker) -> Retained<NSScrollView> {
    let scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        frame(0.0, 0.0, size.width, size.height),
    );
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(true);
    // A pinch, and View ▸ Zoom's three items (`zoom.rs`).
    scroll.setAllowsMagnification(true);
    scroll.setMinMagnification(crate::zoom::STOPS[0]);
    scroll.setMaxMagnification(crate::zoom::STOPS[crate::zoom::STOPS.len() - 1]);
    scroll.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );

    let (w, h) = pane.grid.borrow().size();
    let grid: Retained<GridView> = {
        let this = GridView::alloc(mtm).set_ivars(pane.clone());
        // SAFETY: `initWithFrame:` is `NSView`'s designated initialiser.
        unsafe {
            msg_send![super(this), initWithFrame: frame(0.0, 0.0, HEADER_W + w, HEADER_H + h)]
        }
    };
    let columns: Retained<ColumnHeader> = {
        let this = ColumnHeader::alloc(mtm).set_ivars(pane.clone());
        // SAFETY: as above.
        unsafe { msg_send![super(this), initWithFrame: frame(HEADER_W, 0.0, w, HEADER_H)] }
    };
    let rows: Retained<RowHeader> = {
        let this = RowHeader::alloc(mtm).set_ivars(pane.clone());
        // SAFETY: as above.
        unsafe { msg_send![super(this), initWithFrame: frame(0.0, HEADER_H, HEADER_W, h)] }
    };
    let corner: Retained<Corner> = {
        let this = Corner::alloc(mtm).set_ivars(pane.clone());
        // SAFETY: as above.
        unsafe { msg_send![super(this), initWithFrame: frame(0.0, 0.0, HEADER_W, HEADER_H)] }
    };

    scroll.setDocumentView(Some(&grid));
    // The column band never moves down with the cells, and the row band never moves across.
    scroll.addFloatingSubview_forAxis(&columns, NSEventGestureAxis::Vertical);
    scroll.addFloatingSubview_forAxis(&rows, NSEventGestureAxis::Horizontal);
    // The corner moves with neither: it is the scroll view's own, over its content's top left,
    // added last so it is over both bands. A scroll view need not be flipped, so where its top is
    // — and which margin stretches as it grows — is asked rather than assumed.
    if !scroll.isFlipped() {
        corner.setFrameOrigin(NSPoint::new(0.0, size.height - HEADER_H));
        corner.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
    }
    scroll.addSubview(&corner);

    *pane.views.borrow_mut() = vec![
        Weak::from_retained(&grid.clone().into_super()),
        Weak::from_retained(&columns.clone().into_super()),
        Weak::from_retained(&rows.clone().into_super()),
    ];
    *pane.grid_view.borrow_mut() = Some(Weak::from_retained(&grid));
    *pane.bands.borrow_mut() = Some((Weak::from_retained(&columns), Weak::from_retained(&rows)));
    scroll
}

impl crate::watch::Watched for Pane {
    fn document_changed(&self) {
        self.changed();
    }
}
