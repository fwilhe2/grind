// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The source pane (D9, M8): the document as its projection, read-only, in a trailing split item
//! — decision 3's surface for it — toggled by View ▸ Show Source (⌥⌘U, Safari's key for the same
//! idea).
//!
//! It is an `NSTextView`, so selecting, copying and Find in it are the system's. Its colours are
//! the projection's own token map (`code.rs`), resolved against the system's semantic colours so
//! both appearances follow. The line the selection or the caret projects to is kept selected in
//! it, and **clicking a line goes where that line projects** — a cell, a sheet, a block — through
//! the same jump the sidebar makes. Nothing here writes: the projection is a view of the model,
//! and §6.4 has no error-tolerant parser that would make it editable.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use grind_core::projection::Projection;
use objc2::rc::{Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSColor, NSFont, NSForegroundColorAttributeName, NSScrollView, NSTextDelegate, NSTextView,
    NSTextViewDelegate,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSRange, NSString};

use crate::code::{self, Tint};
use crate::grid_view::Pane;
use crate::page_view::TextPane;
use crate::places::Go;
use crate::sidebar::Places;

/// What the pane shows and where it goes — both panes answer it.
pub trait Sourced: Places {
    fn project(&self) -> Projection;
    /// The address the selection or the caret is at, as the projection spells it.
    fn place(&self) -> Option<String>;
    /// Call `listener` when the selection or the caret moves.
    fn watch_place(&self, listener: Box<dyn Fn()>);
}

impl Sourced for Pane {
    fn project(&self) -> Projection {
        self.app.project()
    }

    fn place(&self) -> Option<String> {
        let name = self.app.sheet_name(self.sheet.get()).ok();
        Some(grind_sheet::a1::format(
            name.as_deref(),
            self.selection.get().active,
        ))
    }

    fn watch_place(&self, listener: Box<dyn Fn()>) {
        self.listen(move |_| listener());
    }
}

impl Sourced for TextPane {
    fn project(&self) -> Projection {
        self.app.project()
    }

    fn place(&self) -> Option<String> {
        Some(grind_text::loc::format(self.state.borrow().caret.block))
    }

    fn watch_place(&self, listener: Box<dyn Fn()>) {
        self.listen(listener);
    }
}

/// What the pane's delegate holds.
pub struct Code {
    source: Rc<dyn Sourced>,
    view: RefCell<Option<Weak<NSTextView>>>,
    projection: RefCell<Option<Projection>>,
    /// A selection the pane is making itself, which is not a click to follow.
    quiet: Cell<bool>,
    /// Whether the pane is open. A closed one re-projects nothing: the projection is the whole
    /// document, and nobody is reading it.
    shown: Cell<bool>,
}

fn color(tint: Tint) -> Retained<NSColor> {
    match tint {
        Tint::Node => NSColor::systemPurpleColor(),
        Tint::Property => NSColor::systemTealColor(),
        Tint::Text => NSColor::systemOrangeColor(),
        Tint::Number => NSColor::systemBlueColor(),
        Tint::Keyword => NSColor::systemPinkColor(),
        Tint::Comment => NSColor::tertiaryLabelColor(),
        Tint::Plain => NSColor::textColor(),
    }
}

fn ns_range(range: std::ops::Range<usize>) -> NSRange {
    NSRange::new(range.start, range.end - range.start)
}

define_class!(
    /// The source pane's text view delegate.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindSourcePane"]
    #[ivars = Code]
    pub struct SourcePane;

    unsafe impl NSObjectProtocol for SourcePane {}

    unsafe impl NSTextDelegate for SourcePane {}

    unsafe impl NSTextViewDelegate for SourcePane {
        #[unsafe(method(textViewDidChangeSelection:))]
        fn did_change_selection(&self, _notification: &NSNotification) {
            self.clicked();
        }
    }
);

impl SourcePane {
    fn view(&self) -> Option<Retained<NSTextView>> {
        self.ivars().view.borrow().as_ref().and_then(Weak::load)
    }

    /// View ▸ Show Source opened or closed the pane: read the document when it opens.
    pub fn shown(&self, shown: bool) {
        self.ivars().shown.set(shown);
        if shown {
            self.reload();
        }
    }

    /// The projection again, coloured, and the selection's line marked.
    pub fn reload(&self) {
        if !self.ivars().shown.get() {
            return;
        }
        let Some(view) = self.view() else { return };
        let projection = self.ivars().source.project();
        self.ivars().quiet.set(true);
        view.setString(&NSString::from_str(projection.text()));
        // SAFETY: the storage is the view's own, alive while the view is; each value is a colour
        // for the foreground key.
        if let Some(storage) = unsafe { view.textStorage() } {
            storage.beginEditing();
            for (range, tint) in code::runs(&projection) {
                // SAFETY: as above.
                unsafe {
                    storage.addAttribute_value_range(
                        NSForegroundColorAttributeName,
                        &color(tint),
                        ns_range(range),
                    )
                };
            }
            storage.endEditing();
        }
        self.ivars().quiet.set(false);
        *self.ivars().projection.borrow_mut() = Some(projection);
        self.mark();
    }

    /// The line the selection or the caret projects to, selected and in sight.
    pub fn mark(&self) {
        if !self.ivars().shown.get() {
            return;
        }
        let Some(view) = self.view() else { return };
        let range = {
            let projection = self.ivars().projection.borrow();
            let Some(projection) = projection.as_ref() else {
                return;
            };
            self.ivars()
                .source
                .place()
                .and_then(|place| code::line_of(projection, &place))
                .and_then(|line| code::line_range(projection, line))
        };
        let Some(range) = range else { return };
        self.ivars().quiet.set(true);
        view.setSelectedRange(ns_range(range.clone()));
        view.scrollRangeToVisible(ns_range(range));
        self.ivars().quiet.set(false);
    }

    /// A click in the pane: go where the line under it projects.
    fn clicked(&self) {
        if self.ivars().quiet.get() {
            return;
        }
        let Some(view) = self.view() else { return };
        let target = {
            let projection = self.ivars().projection.borrow();
            let Some(projection) = projection.as_ref() else {
                return;
            };
            let line = code::line_at(projection, view.selectedRange().location);
            projection.address_on_line(line).map(str::to_owned)
        };
        if let Some(target) = target {
            self.ivars().source.go(&Go::Address(target));
        }
    }
}

/// The pane over `source`: its scroll view, for a split item, and its delegate, which the caller
/// keeps since the text view holds it weakly.
pub fn make(
    source: Rc<dyn Sourced>,
    mtm: MainThreadMarker,
) -> (Retained<NSScrollView>, Retained<SourcePane>) {
    let scroll = NSTextView::scrollableTextView(mtm);
    let view = scroll
        .documentView()
        .and_then(|view| view.downcast::<NSTextView>().ok())
        .expect("a scrollable text view holds a text view");
    view.setEditable(false);
    view.setSelectable(true);
    if let Some(font) = NSFont::userFixedPitchFontOfSize(11.0) {
        view.setFont(Some(&font));
    }
    let pane: Retained<SourcePane> = {
        let this = SourcePane::alloc(mtm).set_ivars(Code {
            source: source.clone(),
            view: RefCell::new(Some(Weak::from_retained(&view))),
            projection: RefCell::new(None),
            quiet: Cell::new(false),
            shown: Cell::new(false),
        });
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };
    view.setDelegate(Some(ProtocolObject::from_ref(&*pane)));
    let weak = Weak::from_retained(&pane);
    source.watch(Box::new(move || {
        if let Some(pane) = weak.load() {
            pane.reload();
        }
    }));
    let weak = Weak::from_retained(&pane);
    source.watch_place(Box::new(move || {
        if let Some(pane) = weak.load() {
            pane.mark();
        }
    }));
    (scroll, pane)
}
