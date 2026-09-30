// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sidebar: *where you can go* (decision 3) — for a spreadsheet its sheets, its defined
//! names and its Problems, for a page its outline, its bookmarks and its Problems (M3, M8).
//!
//! An `NSSplitViewController` whose first item is a sidebar — the platform's own, collapsed and
//! restored by View ▸ Show Sidebar (⌃⌘S, `toggleSidebar:`) with nothing here answering it —
//! holding one source-list table, and whose second is the document. What the rows *are* is
//! `places.rs`'s, portable; this file is the one list over them, for either pane, through
//! [`Places`]. A section's heading is a group row nobody can select, and choosing any other row
//! goes where it says — a sheet, a name, a heading, or where `grind lint` found a problem.
//!
//! The rows are read again when the document changes, not when the cursor moves: a Problems
//! section re-lints the document, which an arrow key has no business costing.
//!
//! ponytail: every change re-lints the whole document, which is `grind lint` on each keystroke
//! that commits. Fine for a document a person edits by hand; the trigger is a document large
//! enough for typing to lag, and the upgrade is linting on idle rather than on change.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use grind_core::lint::Options;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSControlTextEditingDelegate, NSScrollView, NSSplitViewController, NSSplitViewItem,
    NSTableColumn, NSTableView, NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle,
    NSView, NSViewController,
};
use objc2_foundation::{
    NSIndexSet, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString,
};

use crate::grid_view::Pane;
use crate::notice;
use crate::page_view::TextPane;
use crate::places::{self, Go, Row};

/// The sidebar's width when it first opens.
const WIDTH: f64 = 200.0;

/// What the sidebar lists and where its rows go — both panes answer it.
pub trait Places {
    fn rows(&self) -> Vec<Row>;
    fn go(&self, go: &Go);
    /// The row that says where the document is — the sheet showing — kept selected.
    fn here(&self, rows: &[Row]) -> Option<usize>;
    /// Call `listener` whenever the rows may have changed.
    fn watch(&self, listener: Box<dyn Fn()>);
}

/// What the list holds.
pub struct List {
    places: Rc<dyn Places>,
    rows: RefCell<Vec<Row>>,
    table: RefCell<Option<Weak<NSTableView>>>,
    /// A selection the list is making itself, which is not a choice to act on.
    quiet: Cell<bool>,
}

define_class!(
    /// The sidebar's data source and delegate.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindSidebar"]
    #[ivars = List]
    pub struct Sidebar;

    unsafe impl NSObjectProtocol for Sidebar {}

    unsafe impl NSTableViewDataSource for Sidebar {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            self.ivars().rows.borrow().len() as NSInteger
        }

        #[unsafe(method_id(tableView:objectValueForTableColumn:row:))]
        fn value(
            &self,
            _table: &NSTableView,
            _column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<AnyObject>> {
            self.title(row)
        }
    }

    unsafe impl NSControlTextEditingDelegate for Sidebar {}

    unsafe impl NSTableViewDelegate for Sidebar {
        #[unsafe(method(tableView:isGroupRow:))]
        fn is_group_row(&self, _table: &NSTableView, row: NSInteger) -> bool {
            self.row(row).is_some_and(|row| row.is_heading())
        }

        #[unsafe(method(tableView:shouldSelectRow:))]
        fn should_select_row(&self, _table: &NSTableView, row: NSInteger) -> bool {
            self.row(row).is_some_and(|row| !row.is_heading())
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            self.chosen(notification);
        }
    }
);

impl Sidebar {
    fn row(&self, row: NSInteger) -> Option<Row> {
        let at = usize::try_from(row).ok()?;
        self.ivars().rows.borrow().get(at).cloned()
    }

    fn title(&self, row: NSInteger) -> Option<Retained<AnyObject>> {
        let row = self.row(row)?;
        Some(NSString::from_str(&row.title).into_super().into())
    }

    fn chosen(&self, notification: &NSNotification) {
        if self.ivars().quiet.get() {
            return;
        }
        let row = notification
            .object()
            .and_then(|object| object.downcast::<NSTableView>().ok())
            .map_or(-1, |table| table.selectedRow());
        if let Some(go) = self.row(row).and_then(|row| row.go) {
            self.ivars().places.go(&go);
        }
    }

    /// Read the rows again, and keep the row that says where the document is selected.
    pub fn refresh(&self) {
        let list = self.ivars();
        let rows = list.places.rows();
        let here = list.places.here(&rows);
        *list.rows.borrow_mut() = rows;
        let Some(table) = list.table.borrow().as_ref().and_then(Weak::load) else {
            return;
        };
        list.quiet.set(true);
        table.reloadData();
        match here {
            Some(at) => table
                .selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(at), false),
            // SAFETY: a nil sender is what a programmatic deselection passes.
            None => unsafe { table.deselectAll(None) },
        }
        list.quiet.set(false);
    }
}

impl Places for Pane {
    fn rows(&self) -> Vec<Row> {
        places::sheet(&self.app, &self.app.lint(&Options::default()))
    }

    fn go(&self, go: &Go) {
        match go {
            Go::Sheet(sheet) => self.show_sheet(*sheet),
            Go::Address(address) => {
                if let Some((sheet, selection)) =
                    places::cells(&self.app, self.sheet.get(), address)
                {
                    self.show_sheet(sheet);
                    self.select(selection);
                    self.focus_grid();
                }
            }
        }
    }

    fn here(&self, rows: &[Row]) -> Option<usize> {
        let showing = Go::Sheet(self.sheet.get());
        rows.iter()
            .position(|row| row.go.as_ref() == Some(&showing))
    }

    fn watch(&self, listener: Box<dyn Fn()>) {
        self.listen_document(listener);
    }
}

impl Places for TextPane {
    fn rows(&self) -> Vec<Row> {
        places::text(&self.app, &self.app.lint(&Options::default()))
    }

    fn go(&self, go: &Go) {
        let Go::Address(address) = go else { return };
        if let Some(caret) = places::caret(&self.app, address) {
            self.go_to(caret);
        }
    }

    fn here(&self, _rows: &[Row]) -> Option<usize> {
        None
    }

    fn watch(&self, listener: Box<dyn Fn()>) {
        self.listen_document(listener);
    }
}

impl Pane {
    /// The showing sheet's name.
    pub fn sheet_name(&self) -> String {
        self.app.sheet_name(self.sheet.get()).unwrap_or_default()
    }

    /// Insert ▸ Sheet: a sheet after the last, under the first free `SheetN`, shown.
    pub fn add_sheet(&self) {
        match self.app.add_sheet(&self.app.fresh_sheet_name()) {
            Ok(index) => self.show_sheet(index),
            Err(error) => self.say(Some((&error.to_string(), None))),
        }
    }

    /// Format ▸ Rename Sheet…: every reference that named the sheet follows it, and the banner
    /// says how many did — one ⌘Z takes the whole rename back.
    pub fn rename_sheet(&self, name: &str) {
        match self.app.rename_sheet(self.sheet.get(), name) {
            Ok(0) => self.say(None),
            Ok(count) => self.say(Some((&notice::references_renamed(count), None))),
            // The core's own sentence — an empty name, a duplicate — not a second copy of the
            // rule.
            Err(error) => self.say(Some((&error.to_string(), None))),
        }
    }

    /// Edit ▸ Delete Sheet. The last sheet stays, and the banner says why.
    pub fn delete_sheet(&self) {
        if self.app.sheet_count() <= 1 {
            self.say(Some((&notice::last_sheet(), None)));
            return;
        }
        if let Err(error) = self.app.remove_sheet(self.sheet.get()) {
            self.say(Some((&error.to_string(), None)));
        }
    }
}

fn rect(w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h))
}

/// `content` with the sidebar before it, listing `places`, and `trailing` after it as a
/// collapsed inspector — the source pane — as a split view controller for the window's content.
/// Also the sidebar's list, which the caller keeps, since the table holds its data source
/// weakly, and the trailing item, which View ▸ Show Source opens and closes.
pub fn split(
    places: Rc<dyn Places>,
    content: &NSView,
    trailing: &NSView,
    height: f64,
    mtm: MainThreadMarker,
) -> (
    Retained<NSSplitViewController>,
    Retained<Sidebar>,
    Retained<NSSplitViewItem>,
) {
    let list: Retained<Sidebar> = {
        let this = Sidebar::alloc(mtm).set_ivars(List {
            places: places.clone(),
            rows: RefCell::new(Vec::new()),
            table: RefCell::new(None),
            quiet: Cell::new(false),
        });
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };
    let table = NSTableView::initWithFrame(NSTableView::alloc(mtm), rect(WIDTH, height));
    let column =
        NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str("place"));
    table.addTableColumn(&column);
    table.setHeaderView(None);
    table.setStyle(NSTableViewStyle::SourceList);
    // SAFETY: the list answers both protocols' methods it declares, and outlives the table —
    // the caller keeps it for as long as the window lives.
    unsafe {
        table.setDataSource(Some(ProtocolObject::from_ref(&*list)));
        table.setDelegate(Some(ProtocolObject::from_ref(&*list)));
    }
    *list.ivars().table.borrow_mut() = Some(Weak::from_retained(&table));
    let weak = Weak::from_retained(&list);
    places.watch(Box::new(move || {
        if let Some(list) = weak.load() {
            list.refresh();
        }
    }));
    list.refresh();

    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(WIDTH, height));
    scroll.setDocumentView(Some(&table));
    scroll.setHasVerticalScroller(true);

    let sidebar = NSViewController::new(mtm);
    sidebar.setView(&scroll);
    let main = NSViewController::new(mtm);
    main.setView(content);

    let inspector = NSViewController::new(mtm);
    inspector.setView(trailing);
    let source = NSSplitViewItem::inspectorWithViewController(&inspector);
    source.setCanCollapse(true);
    source.setCollapsed(true);

    let split = NSSplitViewController::new(mtm);
    split.addSplitViewItem(&NSSplitViewItem::sidebarWithViewController(&sidebar));
    split.addSplitViewItem(&NSSplitViewItem::splitViewItemWithViewController(&main));
    split.addSplitViewItem(&source);
    (split, list, source)
}
