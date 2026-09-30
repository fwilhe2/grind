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
use std::collections::HashMap;
use std::rc::{Rc, Weak as RcWeak};
use std::sync::Arc;

use grind_core::color::{self, Rgb};
use grind_sheet::nav::Selection;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBeep, NSColor, NSColorSpace, NSEvent, NSEventGestureAxis,
    NSEventModifierFlags, NSGraphicsContext, NSMenuItem, NSScrollView, NSTableView, NSTextField,
    NSView,
};
use objc2_core_graphics::CGContext;
use objc2_foundation::{
    NSArray, NSAttributedString, NSIndexSet, NSObject, NSPoint, NSRect, NSSize, NSString,
};

use crate::banner::Banner;
use crate::editor::Edit;
use crate::keys;
use crate::metrics::{BASE_PT, CoreText};
use crate::render;
use crate::sheet::geom::{Grid, HEADER_H, HEADER_W, Rect};
use crate::sheet::paint::{self, Look, Op, Palette};
use crate::sheet::select;
use crate::sheet::state::{self, Mode, Seed};

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
    on_change: RefCell<Option<Box<dyn Fn()>>>,
    /// The sidebar's list of sheets, which a sheet added, renamed or deleted changes.
    sheet_list: RefCell<Option<Weak<NSTableView>>>,
}

impl Pane {
    pub fn new(app: Arc<grind_sheet::App>) -> Rc<Pane> {
        let grid = Grid::of(&app, 0);
        Rc::new_cyclic(|me| Pane {
            me: me.clone(),
            app,
            sheet: Cell::new(0),
            grid: RefCell::new(grid),
            text: CoreText::new(BASE_PT),
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
            sheet_list: RefCell::new(None),
        })
    }

    /// The sidebar's list of sheets.
    pub fn set_sheet_list(&self, table: &Retained<NSTableView>) {
        *self.sheet_list.borrow_mut() = Some(Weak::from_retained(table));
    }

    /// The sidebar's list, with the showing sheet's row selected. Selecting a row programmatically
    /// posts the same notification a click does, and `show_sheet` of the sheet already showing is
    /// nothing.
    fn mark_sheet(&self, reload: bool) {
        let Some(table) = self.sheet_list.borrow().as_ref().and_then(Weak::load) else {
            return;
        };
        if reload {
            table.reloadData();
        }
        table.selectRowIndexes_byExtendingSelection(
            &NSIndexSet::indexSetWithIndex(self.sheet.get()),
            false,
        );
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
        let text = self.edit_text();
        for listener in self.text_listeners.borrow().iter() {
            listener(&text);
        }
    }

    /// What to do when the document changes — the document marks itself edited.
    pub fn on_change(&self, callback: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(callback));
    }

    /// The core said the document changed (architecture rule 3: it pushes, shells never poll):
    /// read the sheet's axes again — an edit may have widened the used extent — resize the views,
    /// redraw them, tell the listeners, and mark the document edited.
    pub fn document_changed(&self) {
        // A sheet deleted — or one taken back by an undo — may be the one showing.
        let count = self.app.sheet_count().max(1);
        if self.sheet.get() >= count {
            self.sheet.set(count - 1);
            self.selection.set(Selection::default());
        }
        self.mark_sheet(true);
        let grid = Grid::of(&self.app, self.sheet.get());
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
        for view in self.views.borrow().iter().filter_map(Weak::load) {
            view.setNeedsDisplay(true);
        }
        let selection = self.selection.get();
        for listener in self.listeners.borrow().iter() {
            listener(selection);
        }
        self.edit_changed();
        if let Some(callback) = self.on_change.borrow().as_ref() {
            callback();
        }
    }

    /// Show another sheet: its own axes, the views resized to them, the cursor home and the view
    /// at the top left — what choosing a sheet in the sidebar does.
    pub fn show_sheet(&self, sheet: usize) {
        if sheet == self.sheet.get() || sheet >= self.app.sheet_count() {
            return;
        }
        self.sheet.set(sheet);
        let grid = Grid::of(&self.app, sheet);
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
        self.mark_sheet(false);
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
    }
}

thread_local! {
    /// The panes the core may tell about a change, by the address of each — see [`Changed`].
    static PANES: RefCell<HashMap<usize, RcWeak<Pane>>> = RefCell::new(HashMap::new());
}

/// The core's observer, which has to be `Send + Sync` because the core does not say which thread
/// a change arrives on — and a pane, holding views, is the main thread's alone. So the observer
/// holds only the pane's address, and finds the pane in a registry the main thread owns. Every
/// edit in this shell is made on the main thread, so every notification arrives there; one that
/// did not would find no registry and change nothing, rather than touch AppKit from elsewhere.
struct Changed(usize);

impl grind_core::Observer for Changed {
    fn changed(&self) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let pane = PANES.with(|panes| panes.borrow().get(&self.0).and_then(RcWeak::upgrade));
        if let Some(pane) = pane {
            pane.document_changed();
        }
    }
}

/// Have the core tell `pane` about every change to its document from now on.
pub fn watch(pane: &Rc<Pane>) {
    let id = Rc::as_ptr(pane) as usize;
    PANES.with(|panes| {
        let mut panes = panes.borrow_mut();
        // A pane that is gone leaves its address behind; drop those as new ones arrive.
        panes.retain(|_, pane| pane.strong_count() > 0);
        panes.insert(id, Rc::downgrade(pane));
    });
    pane.app.set_observer(Arc::new(Changed(id)));
}

/// A colour AppKit resolves for the current drawing appearance, as three bytes. A colour with no
/// sRGB form — a pattern — is a mid grey, which reads on either appearance.
fn rgb(color: &NSColor) -> Rgb {
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
fn palette() -> Palette {
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

fn rect(frame: NSRect) -> Rect {
    Rect::new(
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    )
}

fn ns_rect(rect: Rect) -> NSRect {
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
fn located(view: &NSView, event: &NSEvent) -> NSPoint {
    view.convertPoint_fromView(event.locationInWindow(), None)
}

fn shifted(event: &NSEvent) -> bool {
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
            };
            // The dirty rectangle in the sheet's own coordinates: the view keeps a margin the
            // header bands float over.
            let view = rect(dirty).offset(-HEADER_W, -HEADER_H);
            let ops: Vec<Op> = paint::cells(
                &pane.app,
                pane.sheet.get(),
                &pane.grid.borrow(),
                view,
                pane.selection.get(),
                &look,
            )
            .into_iter()
            .map(|op| shift(op, HEADER_W, HEADER_H))
            .collect();
            draw(&ops, pane);
        }

        /// A key goes to the text system, which answers with what it means for this user — a
        /// selector, handled below, or text, which starts an edit. ⌃U, Excel for Mac's key for
        /// editing the active cell, is bound to nothing in the standard bindings, so it is read
        /// here first.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
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
            if let Some(seed) = state::typed(Mode::Ready, &typed) {
                self.ivars().begin_edit(seed);
            }
        }

        /// A click selects; a double-click opens the cell to amend it.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.click(event, shifted(event));
            if event.clickCount() == 2 {
                self.ivars().begin_edit(Seed::Cell);
            }
        }

        /// Edit ▸ Undo — the core's history, never AppKit's (architecture rule 2).
        #[unsafe(method(undo:))]
        fn undo(&self, _sender: Option<&AnyObject>) {
            self.ivars().app.undo();
        }

        #[unsafe(method(redo:))]
        fn redo(&self, _sender: Option<&AnyObject>) {
            self.ivars().app.redo();
        }

        /// Undo and Redo are grey when the core has nothing to take back or bring back — the
        /// named gap `ui_win32` has, closed by the platform's own mechanism.
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let app = &self.ivars().app;
            match item.action().map(|action| action.name().to_str().unwrap_or_default().to_owned()) {
                Some(name) if name == "undo:" => app.can_undo(),
                Some(name) if name == "redo:" => app.can_redo(),
                _ => true,
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.click(event, true);
        }
    }
);

impl GridView {
    fn act(&self, action: keys::GridAction) {
        let pane = self.ivars();
        let visible = rect(self.visibleRect());
        let grid = pane.grid.borrow();
        let page = select::page(&grid, visible.y, visible.h - HEADER_H);
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
        let next = select::click(
            &pane.grid.borrow(),
            pane.selection.get(),
            at.x - HEADER_W,
            at.y - HEADER_H,
            extend,
        );
        pane.select(next);
    }
}

define_class!(
    /// The column letters, floating over the grid's top margin. A click selects the column.
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
            self.click(event, shifted(event));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.click(event, true);
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
    /// The row numbers, floating over the grid's left margin. A click selects the row.
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
            self.click(event, shifted(event));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.click(event, true);
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

/// `op` drawn `(dx, dy)` further on, with nothing cut away.
fn shift(op: Op, dx: f64, dy: f64) -> Op {
    match op {
        Op::Fill { rect, color } => Op::Fill {
            rect: rect.offset(dx, dy),
            color,
        },
        Op::Wash { rect, color } => Op::Wash {
            rect: rect.offset(dx, dy),
            color,
        },
        Op::Text {
            x,
            top,
            text,
            style,
            color,
            clip,
        } => Op::Text {
            x: x + dx,
            top: top + dy,
            text,
            style,
            color,
            clip: clip.offset(dx, dy),
        },
    }
}

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
