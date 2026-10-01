// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Find and replace (M5, and the page's since M11): Edit ▸ Find's four items, the platform's own —
//! `performFindPanelAction:` with `NSFindPanelAction`'s tags, so ⌘F, ⌘G, ⇧⌘G and ⌘E mean here what
//! they mean in every text view — and the bar ⌘F opens.
//!
//! The bar is a titlebar accessory, hidden until asked for: a search field whose Return finds the
//! next place holding the word, a field for what to replace it with, Replace All, a line saying
//! which of how many the selection is on, and Done. **It is one bar for either pane**, through
//! [`Findable`]: what a match is and where Next goes is the grid's `sheet/search.rs` over
//! `grind_sheet::find`, or the page's `text/search.rs` over `App::find_ignoring_case`, and what
//! the bar says is `notice.rs`'s.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use grind_sheet::RecalcMode;
use grind_sheet::find::{Search, Towards};
use grind_sheet::nav::Selection;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSColor, NSLayoutAttribute, NSSearchField, NSTextField,
    NSTitlebarAccessoryViewController, NSView, NSWindow,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};

use crate::grid_view::Pane;
use crate::menu::find;
use crate::notice;
use crate::page_view::TextPane;
use crate::sheet::search;

const BAR_H: f64 = 34.0;
const FIELD_W: f64 = 200.0;

/// What a find bar searches — both panes answer it.
pub trait Findable {
    /// Select the next or previous place holding `needle`, and say what happened.
    fn find_next(&self, needle: &str, towards: Towards) -> String;
    /// Replace every occurrence, as one ⌘Z, and say what happened.
    fn replace_every(&self, needle: &str, with: &str) -> String;
    /// What ⌘E puts in the search field.
    fn selected_text(&self) -> Option<String>;
    /// Give the keyboard back to the document.
    fn focus_document(&self);
}

/// The bar's controls, made after its target is.
struct Parts {
    controller: Retained<NSTitlebarAccessoryViewController>,
    needle: Retained<NSSearchField>,
    with: Retained<NSTextField>,
    status: Retained<NSTextField>,
}

/// What the bar holds: what it searches, and its controls.
pub struct Held {
    findable: Weak<dyn Findable>,
    parts: RefCell<Option<Parts>>,
}

define_class!(
    /// One window's find bar, and its controls' target.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindFindBar"]
    #[ivars = Held]
    pub struct FindBar;

    impl FindBar {
        /// Return in the search field: the next place holding the word.
        #[unsafe(method(findEntered:))]
        fn entered(&self, _sender: Option<&AnyObject>) {
            self.find(Towards::Next);
        }

        #[unsafe(method(replaceAll:))]
        fn replace_all_clicked(&self, _sender: Option<&AnyObject>) {
            self.replace_all();
        }

        #[unsafe(method(findDone:))]
        fn done(&self, _sender: Option<&AnyObject>) {
            self.close();
        }
    }
);

impl FindBar {
    fn needle(&self) -> String {
        self.ivars()
            .parts
            .borrow()
            .as_ref()
            .map(|parts| parts.needle.stringValue().to_string())
            .unwrap_or_default()
    }

    fn say(&self, said: &str) {
        if let Some(parts) = self.ivars().parts.borrow().as_ref() {
            parts.status.setStringValue(&NSString::from_str(said));
        }
    }

    /// `performFindPanelAction:` — which of Find's items, by its tag.
    pub fn action(&self, tag: isize) {
        match tag {
            find::SHOW => self.open(),
            find::NEXT => self.find(Towards::Next),
            find::PREVIOUS => self.find(Towards::Previous),
            find::USE_SELECTION => {
                let text = self
                    .ivars()
                    .findable
                    .upgrade()
                    .and_then(|findable| findable.selected_text());
                if let (Some(text), Some(parts)) = (text, self.ivars().parts.borrow().as_ref()) {
                    parts.needle.setStringValue(&NSString::from_str(&text));
                }
            }
            _ => {}
        }
    }

    /// Show the bar, with the keyboard in its search field.
    pub fn open(&self) {
        let Some(field) = self.ivars().parts.borrow().as_ref().map(|parts| {
            parts.controller.setHidden(false);
            parts.needle.clone()
        }) else {
            return;
        };
        if let Some(window) = field.window() {
            window.makeFirstResponder(Some(&field));
        }
    }

    /// Done: hide the bar and give the keyboard back to the document.
    fn close(&self) {
        if let Some(parts) = self.ivars().parts.borrow().as_ref() {
            parts.controller.setHidden(true);
        }
        if let Some(findable) = self.ivars().findable.upgrade() {
            findable.focus_document();
        }
    }

    /// The next or previous place holding the word; with no word yet, the bar, to ask for one.
    pub fn find(&self, towards: Towards) {
        let needle = self.needle();
        if needle.is_empty() {
            self.open();
            return;
        }
        if let Some(findable) = self.ivars().findable.upgrade() {
            let said = findable.find_next(&needle, towards);
            self.say(&said);
        }
    }

    fn replace_all(&self) {
        let needle = self.needle();
        if needle.is_empty() {
            return;
        }
        let with = self
            .ivars()
            .parts
            .borrow()
            .as_ref()
            .map(|parts| parts.with.stringValue().to_string())
            .unwrap_or_default();
        if let Some(findable) = self.ivars().findable.upgrade() {
            let said = findable.replace_every(&needle, &with);
            self.say(&said);
        }
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// A hidden find bar under `window`'s title, searching `pane`. The caller keeps it.
pub fn attach<T: Findable + 'static>(
    window: &NSWindow,
    pane: &Rc<T>,
    mtm: MainThreadMarker,
) -> Retained<FindBar> {
    let findable: Rc<dyn Findable> = pane.clone();
    let width = window.frame().size.width;
    let bar = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, width, BAR_H));
    bar.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    let target: Retained<FindBar> = {
        let this = FindBar::alloc(mtm).set_ivars(Held {
            findable: Rc::downgrade(&findable),
            parts: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };

    let needle =
        NSSearchField::initWithFrame(NSSearchField::alloc(mtm), rect(8.0, 5.0, FIELD_W, 24.0));
    needle.setPlaceholderString(Some(&NSString::from_str("Find")));
    let with = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
    with.setFrame(rect(FIELD_W + 16.0, 5.0, FIELD_W, 24.0));
    with.setPlaceholderString(Some(&NSString::from_str("Replace with")));
    // SAFETY: the target answers every action named here, and the window's owner keeps it for as
    // long as the bar exists.
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
    let status = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    let status_x = 2.0 * FIELD_W + 142.0;
    status.setFrame(rect(
        status_x,
        9.0,
        (width - status_x - 96.0).max(0.0),
        16.0,
    ));
    status.setTextColor(Some(&NSColor::secondaryLabelColor()));
    status.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    done.setFrame(rect(width - 88.0, 3.0, 80.0, 28.0));
    done.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
    bar.addSubview(&needle);
    bar.addSubview(&with);
    bar.addSubview(&replace);
    bar.addSubview(&status);
    bar.addSubview(&done);

    let controller = NSTitlebarAccessoryViewController::new(mtm);
    controller.setView(&bar);
    controller.setLayoutAttribute(NSLayoutAttribute::Bottom);
    controller.setHidden(true);
    window.addTitlebarAccessoryViewController(&controller);
    *target.ivars().parts.borrow_mut() = Some(Parts {
        controller,
        needle,
        with,
        status,
    });
    target
}

impl Findable for Pane {
    fn find_next(&self, needle: &str, towards: Towards) -> String {
        let at = (self.sheet.get(), self.selection.get().active);
        match search::next(&self.app, needle, at, towards) {
            Some(found) => {
                self.show_sheet(found.sheet);
                self.select(Selection::at(found.pos));
                notice::found(found.index, found.count, needle)
            }
            None => notice::not_found(needle),
        }
    }

    /// Every cell holding the word, on every sheet, rewritten through the typing rule — one ⌘Z
    /// for all of them.
    fn replace_every(&self, needle: &str, with: &str) -> String {
        match self
            .app
            .replace(&Search::new(needle), with, RecalcMode::Document)
        {
            Ok(replaced) => {
                let first = replaced
                    .refused
                    .first()
                    .map(|(hit, _)| grind_sheet::a1::format(None, hit.pos));
                let refused = first
                    .as_deref()
                    .map(|first| (first, replaced.refused.len()));
                notice::replaced(replaced.cells, refused)
            }
            Err(error) => error.to_string(),
        }
    }

    fn selected_text(&self) -> Option<String> {
        self.app
            .input_text(self.sheet.get(), self.selection.get().active)
            .ok()
    }

    fn focus_document(&self) {
        self.focus_grid();
    }
}

impl Findable for TextPane {
    /// From the selection's start, so Next moves past a hit already selected and Previous does
    /// not land on it again.
    fn find_next(&self, needle: &str, towards: Towards) -> String {
        let at = {
            let state = self.state.borrow();
            state.selection().map_or(state.caret, |(from, _)| from)
        };
        match crate::text::search::next(&self.app, needle, at, towards) {
            Some(found) => {
                self.act_on(|page, _, _| {
                    page.place(found.from, false);
                    page.place(found.to, true);
                    Ok(())
                });
                notice::found_text(found.index, found.count, needle)
            }
            None => notice::not_found_text(needle),
        }
    }

    fn replace_every(&self, needle: &str, with: &str) -> String {
        match self.app.replace(needle, with) {
            Ok(count) => notice::replaced_text(count),
            Err(error) => error.to_string(),
        }
    }

    fn selected_text(&self) -> Option<String> {
        self.state.borrow().selected_text(&self.app)
    }

    fn focus_document(&self) {
        if let Some(view) = self.view()
            && let Some(window) = view.window()
        {
            window.makeFirstResponder(Some(&view));
        }
    }
}
