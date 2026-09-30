// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sidebar: *where you can go* (decision 3) — for a spreadsheet, its sheets (M3).
//!
//! An `NSSplitViewController` whose first item is a sidebar — the platform's own, collapsed and
//! restored by View ▸ Show Sidebar (⌃⌘S, `toggleSidebar:`) with nothing here answering it — holding
//! a source-list table of the sheets, and whose second is the grid. Choosing a sheet shows it:
//! the pane reads that sheet's axes, resizes its views and puts the cursor home. Defined names
//! and M8's Problems join the list later; they are more rows, not another surface.

use std::rc::{Rc, Weak};

use objc2::rc::Retained;
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

/// The sidebar's width when it first opens.
const WIDTH: f64 = 180.0;

define_class!(
    /// The sheet list's data source and delegate: its rows are the document's sheets, and
    /// choosing one shows it.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindSheetList"]
    #[ivars = Weak<Pane>]
    pub struct SheetList;

    unsafe impl NSObjectProtocol for SheetList {}

    unsafe impl NSTableViewDataSource for SheetList {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            self.ivars()
                .upgrade()
                .map_or(0, |pane| pane.app.sheet_count() as NSInteger)
        }

        #[unsafe(method_id(tableView:objectValueForTableColumn:row:))]
        fn value(
            &self,
            _table: &NSTableView,
            _column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<AnyObject>> {
            self.sheet_name(row)
        }
    }

    unsafe impl NSControlTextEditingDelegate for SheetList {}

    unsafe impl NSTableViewDelegate for SheetList {
        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            self.chosen(notification);
        }
    }
);

impl SheetList {
    fn sheet_name(&self, row: NSInteger) -> Option<Retained<AnyObject>> {
        let pane = self.ivars().upgrade()?;
        let name = pane.app.sheet_name(usize::try_from(row).ok()?).ok()?;
        Some(NSString::from_str(&name).into_super().into())
    }

    fn chosen(&self, notification: &NSNotification) {
        let Some(pane) = self.ivars().upgrade() else {
            return;
        };
        let row = notification
            .object()
            .and_then(|object| object.downcast::<NSTableView>().ok())
            .map_or(-1, |table| table.selectedRow());
        if let Ok(sheet) = usize::try_from(row) {
            pane.show_sheet(sheet);
        }
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

/// `content` with the sheet list beside it, as a split view controller for the window's content.
pub fn split(
    pane: &Rc<Pane>,
    content: &NSView,
    height: f64,
    mtm: MainThreadMarker,
) -> Retained<NSSplitViewController> {
    let list: Retained<SheetList> = {
        let this = SheetList::alloc(mtm).set_ivars(Rc::downgrade(pane));
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };
    let table = NSTableView::initWithFrame(NSTableView::alloc(mtm), rect(WIDTH, height));
    let column =
        NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str("sheet"));
    column.setTitle(&NSString::from_str("Sheets"));
    table.addTableColumn(&column);
    table.setHeaderView(None);
    table.setStyle(NSTableViewStyle::SourceList);
    // SAFETY: the list answers both protocols' methods it declares, and outlives the table —
    // the pane keeps it for as long as the views exist.
    unsafe {
        table.setDataSource(Some(ProtocolObject::from_ref(&*list)));
        table.setDelegate(Some(ProtocolObject::from_ref(&*list)));
    }
    table.selectRowIndexes_byExtendingSelection(
        &NSIndexSet::indexSetWithIndex(pane.sheet.get()),
        false,
    );
    pane.keep(list.into_super());
    pane.set_sheet_list(&table);

    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(WIDTH, height));
    scroll.setDocumentView(Some(&table));
    scroll.setHasVerticalScroller(true);

    let sidebar = NSViewController::new(mtm);
    sidebar.setView(&scroll);
    let main = NSViewController::new(mtm);
    main.setView(content);

    let split = NSSplitViewController::new(mtm);
    split.addSplitViewItem(&NSSplitViewItem::sidebarWithViewController(&sidebar));
    split.addSplitViewItem(&NSSplitViewItem::splitViewItemWithViewController(&main));
    split
}
