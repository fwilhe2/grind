// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The titlebar accessory: the name box and the status bar's read-out — decision 3's surface for
//! the two read-outs of *where* and *what* (M3), and where M4's formula field joins them.
//!
//! An `NSTitlebarAccessoryViewController` at the title bar's bottom edge, so it takes on the
//! window's own material and, on macOS 26, its Liquid Glass, with nothing here knowing. Both
//! read-outs are `grind_sheet::place`'s, the words every other shell's name box and status bar
//! say: the name box shows a name over exactly the selection or the active cell's address, and
//! **typing a place into it goes there** — `g20`, `Data.B2:C9`, a defined name — through
//! `place::locate`; the read-out says nothing for one cell and the range's sum, count and average
//! for more.

use std::rc::{Rc, Weak};

use grind_sheet::place;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBeep, NSColor, NSLayoutAttribute, NSTextAlignment, NSTextField,
    NSTitlebarAccessoryViewController, NSView, NSWindow,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};

use crate::grid_view::Pane;

/// The bar's height, and the name box's width — room for `AA1048576` or a short name.
const BAR_H: f64 = 30.0;
const NAME_W: f64 = 120.0;

define_class!(
    /// The name box's target: what Return in the box does. A control holds its target weakly, so
    /// the pane keeps this alive.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindNameBox"]
    #[ivars = Weak<Pane>]
    pub struct NameBox;

    impl NameBox {
        /// Go where the box says, or put back what it said: a place this sheet does not have is
        /// not a place to move to.
        #[unsafe(method(nameBoxEntered:))]
        fn entered(&self, sender: &AnyObject) {
            let (Some(pane), Some(field)) = (
                self.ivars().upgrade(),
                sender.downcast_ref::<NSTextField>(),
            ) else {
                return;
            };
            let text = field.stringValue().to_string();
            match place::locate(&pane.app, pane.sheet.get(), &text) {
                Some(selection) => {
                    pane.select(selection);
                    // Back to the grid, where the next key belongs.
                    pane.focus_grid();
                }
                None => {
                    NSBeep();
                    let shown = place::name_box(&pane.app, pane.sheet.get(), pane.selection.get());
                    field.setStringValue(&NSString::from_str(&shown));
                }
            }
        }
    }
);

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// Put the name box and the read-out under `window`'s title, answering to `pane`.
pub fn attach(window: &NSWindow, pane: &Rc<Pane>, mtm: MainThreadMarker) {
    let width = window.frame().size.width;
    let bar = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, width, BAR_H));
    bar.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);

    let name = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
    name.setFrame(rect(8.0, 4.0, NAME_W, 22.0));
    let target: Retained<NameBox> = {
        let this = NameBox::alloc(mtm).set_ivars(Rc::downgrade(pane));
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };
    // SAFETY: the target answers the action's selector, and outlives the field (the pane keeps
    // it for as long as its views exist).
    unsafe {
        name.setTarget(Some(&target));
        name.setAction(Some(sel!(nameBoxEntered:)));
    }
    bar.addSubview(&name);
    pane.set_name_box(&name);

    let status = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    status.setFrame(rect(NAME_W + 16.0, 7.0, width - NAME_W - 24.0, 16.0));
    status.setAlignment(NSTextAlignment::Right);
    status.setTextColor(Some(&NSColor::secondaryLabelColor()));
    status.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    bar.addSubview(&status);

    let controller = NSTitlebarAccessoryViewController::new(mtm);
    controller.setView(&bar);
    controller.setLayoutAttribute(NSLayoutAttribute::Bottom);
    window.addTitlebarAccessoryViewController(&controller);

    pane.keep(target.into_super());
    let weak = Rc::downgrade(pane);
    pane.listen(move |selection| {
        let Some(pane) = weak.upgrade() else { return };
        let sheet = pane.sheet.get();
        name.setStringValue(&NSString::from_str(&place::name_box(
            &pane.app, sheet, selection,
        )));
        status.setStringValue(&NSString::from_str(&place::status(
            &pane.app, sheet, selection,
        )));
    });
}
