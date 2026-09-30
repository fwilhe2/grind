// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Find and replace over cells (M5): Edit ▸ Find's four items, the platform's own —
//! `performFindPanelAction:` with `NSFindPanelAction`'s tags, so ⌘F, ⌘G, ⇧⌘G and ⌘E mean here what
//! they mean in every text view — and the bar ⌘F opens.
//!
//! The bar is a third titlebar accessory, hidden until asked for, as the banner is: a search field
//! whose Return finds the next cell holding the word, a field for what to replace it with, Replace
//! All, and Done. What a match is and where Next goes are `grind_sheet::find`'s, through
//! `sheet/search.rs`; Replace All is `App::replace`, one ⌘Z for every cell it changed. Every
//! sentence it prompts is `notice.rs`'s.

use std::rc::{Rc, Weak};

use grind_sheet::RecalcMode;
use grind_sheet::find::{Search, Towards};
use grind_sheet::nav::Selection;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSLayoutAttribute, NSSearchField, NSTextField,
    NSTitlebarAccessoryViewController, NSView, NSWindow,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};

use crate::grid_view::Pane;
use crate::menu::find;
use crate::notice;
use crate::sheet::search;

const BAR_H: f64 = 34.0;
const FIELD_W: f64 = 220.0;

/// The find bar of one window.
pub struct FindBar {
    controller: Retained<NSTitlebarAccessoryViewController>,
    needle: Retained<NSSearchField>,
    with: Retained<NSTextField>,
}

define_class!(
    /// The bar's controls' target. A control holds its target weakly, so the pane keeps this.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindFindBar"]
    #[ivars = Weak<Pane>]
    pub struct FindTarget;

    impl FindTarget {
        /// Return in the search field: the next cell holding the word.
        #[unsafe(method(findEntered:))]
        fn entered(&self, _sender: Option<&AnyObject>) {
            if let Some(pane) = self.ivars().upgrade() {
                pane.find(Towards::Next);
            }
        }

        #[unsafe(method(replaceAll:))]
        fn replace_all(&self, _sender: Option<&AnyObject>) {
            if let Some(pane) = self.ivars().upgrade() {
                pane.replace_all();
            }
        }

        #[unsafe(method(findDone:))]
        fn done(&self, _sender: Option<&AnyObject>) {
            if let Some(pane) = self.ivars().upgrade() {
                pane.close_find();
            }
        }
    }
);

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// A hidden find bar under `window`'s title, answering to `pane`.
pub fn attach(window: &NSWindow, pane: &Rc<Pane>, mtm: MainThreadMarker) -> FindBar {
    let width = window.frame().size.width;
    let bar = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, width, BAR_H));
    bar.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    let target: Retained<FindTarget> = {
        let this = FindTarget::alloc(mtm).set_ivars(Rc::downgrade(pane));
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };

    let needle =
        NSSearchField::initWithFrame(NSSearchField::alloc(mtm), rect(8.0, 5.0, FIELD_W, 24.0));
    needle.setPlaceholderString(Some(&NSString::from_str("Find")));
    let with = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
    with.setFrame(rect(FIELD_W + 16.0, 5.0, FIELD_W, 24.0));
    with.setPlaceholderString(Some(&NSString::from_str("Replace with")));
    // SAFETY: the target answers every action named here, and the pane keeps it for as long as
    // the bar exists.
    let (replace, done) = unsafe {
        needle.setTarget(Some(&target));
        needle.setAction(Some(sel!(findEntered:)));
        (
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Replace All"),
                Some(&target),
                Some(sel!(replaceAll:)),
                mtm,
            ),
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Done"),
                Some(&target),
                Some(sel!(findDone:)),
                mtm,
            ),
        )
    };
    replace.setFrame(rect(2.0 * FIELD_W + 24.0, 3.0, 110.0, 28.0));
    done.setFrame(rect(width - 88.0, 3.0, 80.0, 28.0));
    done.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
    bar.addSubview(&needle);
    bar.addSubview(&with);
    bar.addSubview(&replace);
    bar.addSubview(&done);
    pane.keep(target.into_super());

    let controller = NSTitlebarAccessoryViewController::new(mtm);
    controller.setView(&bar);
    controller.setLayoutAttribute(NSLayoutAttribute::Bottom);
    controller.setHidden(true);
    window.addTitlebarAccessoryViewController(&controller);
    FindBar {
        controller,
        needle,
        with,
    }
}

impl Pane {
    /// The word in the find bar.
    fn needle(&self) -> String {
        self.find_bar
            .borrow()
            .as_ref()
            .map(|bar| bar.needle.stringValue().to_string())
            .unwrap_or_default()
    }

    /// `performFindPanelAction:` — which of Find's items, by its tag.
    pub fn find_panel_action(&self, tag: isize) {
        match tag {
            find::SHOW => self.open_find(),
            find::NEXT => self.find(Towards::Next),
            find::PREVIOUS => self.find(Towards::Previous),
            find::USE_SELECTION => {
                // The active cell's text becomes the word, and the search starts where it is.
                let text = self
                    .app
                    .input_text(self.sheet.get(), self.selection.get().active)
                    .unwrap_or_default();
                if let Some(bar) = self.find_bar.borrow().as_ref() {
                    bar.needle.setStringValue(&NSString::from_str(&text));
                }
            }
            _ => {}
        }
    }

    /// Show the bar, with the keyboard in its search field.
    fn open_find(&self) {
        let Some(field) = self.find_bar.borrow().as_ref().map(|bar| {
            bar.controller.setHidden(false);
            bar.needle.clone()
        }) else {
            return;
        };
        if let Some(window) = field.window() {
            window.makeFirstResponder(Some(&field));
        }
    }

    /// Done: hide the bar and give the keyboard back to the grid.
    fn close_find(&self) {
        if let Some(bar) = self.find_bar.borrow().as_ref() {
            bar.controller.setHidden(true);
        }
        self.focus_grid();
    }

    /// The next or previous cell holding the word, on whichever sheet it is; with no word yet,
    /// the bar, to ask for one.
    pub fn find(&self, towards: Towards) {
        let needle = self.needle();
        if needle.is_empty() {
            self.open_find();
            return;
        }
        let at = (self.sheet.get(), self.selection.get().active);
        match search::next(&self.app, &needle, at, towards) {
            Some(found) => {
                self.show_sheet(found.sheet);
                self.select(Selection::at(found.pos));
                self.say(Some((
                    &notice::found(found.index, found.count, &needle),
                    None,
                )));
            }
            None => self.say(Some((&notice::not_found(&needle), None))),
        }
    }

    /// Replace All: every cell holding the word, on every sheet, rewritten through the typing
    /// rule — one ⌘Z for all of them.
    fn replace_all(&self) {
        let needle = self.needle();
        if needle.is_empty() {
            return;
        }
        let with = self
            .find_bar
            .borrow()
            .as_ref()
            .map(|bar| bar.with.stringValue().to_string())
            .unwrap_or_default();
        match self
            .app
            .replace(&Search::new(needle), &with, RecalcMode::Document)
        {
            Ok(replaced) => {
                let first = replaced
                    .refused
                    .first()
                    .map(|(hit, _)| grind_sheet::a1::format(None, hit.pos));
                let refused = first
                    .as_deref()
                    .map(|first| (first, replaced.refused.len()));
                self.say(Some((&notice::replaced(replaced.cells, refused), None)));
            }
            Err(error) => self.say(Some((&error.to_string(), None))),
        }
    }
}
