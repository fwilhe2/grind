// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The page, inside an `NSScrollView` — the word processor's document view (M6), and the one
//! place this shell draws a text document on screen (decision 3: AppKit draws the chrome, this
//! draws the document).
//!
//! One flipped view over one [`TextPane`], as tall as the document and as wide as what shows of
//! it; the scroll view does the scrolling. It draws through `text::paint` and
//! [`crate::render::draw`], the path `--render-to` takes too.
//!
//! **Keys arrive as an input method's**: the view is an `NSTextInputClient`, so a key goes to
//! `interpretKeyEvents:` and comes back as `insertText:replacementRange:`, as a selector through
//! `doCommandBySelector:` — read by `keys.rs`'s page table, so a `DefaultKeyBinding.dict` works
//! here as in TextEdit — or as **marked text**, which `text/state.rs` holds and `text/paint.rs`
//! draws in the line until the input method commits it. Every range an input method names is
//! converted in `text/input.rs`; nothing here counts a UTF-16 unit.
//!
//! **The core pushes** (rule 3): an edit's notification lays the document out again, and an edit
//! the view makes itself runs with the pane marked busy, so the notification that arrives in the
//! middle of it waits for the edit to finish rather than borrowing the state the edit holds.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use grind_core::color;
use grind_core::style::TextStyle;
use grind_text::look::Role;
use grind_text::{App, Caret, Faces};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBeep, NSColor, NSColorPanel, NSEvent, NSFontManager,
    NSGraphicsContext, NSMenuItem, NSScrollView, NSTextInputClient, NSView,
};
use objc2_core_graphics::CGContext;
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSPoint, NSRange, NSRect, NSSize, NSString,
    NSUInteger,
};

use crate::clipboard;
use crate::grid_view::{located, ns_rect, rect, rgb, shifted};
use crate::keys::{self, PageAction, Scroll};
use crate::metrics::{BASE_PT, CoreText, Face};
use crate::render;
use crate::sheet::geom::Rect;
use crate::text::face::{self, Column, Laid};
use crate::text::geom;
use crate::text::input::{self, NOT_FOUND};
use crate::text::paint::{self, Frame, Palette};
use crate::text::state::{Page, Refused};

/// What the page view draws from and answers to: the document, its layout, the fonts it is
/// measured in, and the caret.
pub struct TextPane {
    pub app: Arc<App>,
    pub text: CoreText,
    pub state: RefCell<Page>,
    /// The document laid out: its flow, and the cell map the page's `Faces` reads, which may not
    /// ask the document (`grind_text::flow::across`).
    laid: RefCell<Laid>,
    /// The page's width, which it was laid out for.
    width: Cell<f64>,
    view: RefCell<Option<Weak<PageView>>>,
    /// Told that the document changed — what marks it edited.
    on_change: RefCell<Option<Box<dyn Fn()>>>,
    /// An edit of the view's own is running, and the core's notification must wait for it.
    busy: Cell<bool>,
    /// A notification arrived while busy.
    stale: Cell<bool>,
}

impl TextPane {
    pub fn new(app: Arc<App>) -> Rc<TextPane> {
        Rc::new(TextPane {
            app,
            text: CoreText::new(BASE_PT),
            state: RefCell::new(Page::default()),
            laid: RefCell::new(Laid::empty()),
            width: Cell::new(0.0),
            view: RefCell::new(None),
            on_change: RefCell::new(None),
            busy: Cell::new(false),
            stale: Cell::new(false),
        })
    }

    /// What to do when the document changes — the document marks itself edited.
    pub fn on_change(&self, callback: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(callback));
    }

    pub fn view(&self) -> Option<Retained<PageView>> {
        self.view.borrow().as_ref().and_then(Weak::load)
    }

    /// One CoreText face per role, in `Role::ALL`'s order.
    fn faces(&self) -> Vec<Face<'_>> {
        Role::ALL
            .iter()
            .map(|role| Face {
                text: &self.text,
                role: *role,
            })
            .collect()
    }

    /// Run `f` with the layout and the page's `Faces` over it: each block measured in its role's
    /// face at the width it is set.
    fn measured<R>(&self, f: impl FnOnce(&Laid, &Column<'_, Face<'_>>) -> R) -> R {
        let faces = self.faces();
        let laid = self.laid.borrow();
        f(&laid, &laid.faces(&faces))
    }

    /// Lay the whole document out again at the page's width.
    fn reflow(&self) {
        let laid = face::lay_out(&self.app, &self.faces(), self.width.get());
        *self.laid.borrow_mut() = laid;
    }

    /// Size the page to its scroll view: as wide as what shows — laid out again when that
    /// changed — and as tall as the document, or the view when the document is shorter.
    pub fn fit(&self) {
        let Some(view) = self.view() else {
            return;
        };
        // SAFETY: `superview` is an ordinary read of the view hierarchy.
        let visible =
            unsafe { view.superview() }.map_or(view.frame().size, |clip| clip.bounds().size);
        if visible.width != self.width.get() {
            self.width.set(visible.width);
            self.reflow();
        }
        let height = geom::page_height(self.laid.borrow().flow.height(), visible.height);
        let size = NSSize::new(visible.width, height);
        if view.frame().size != size {
            view.setFrameSize(size);
        }
    }

    /// The document changed and nothing is holding the state: put the caret somewhere real, lay
    /// out, resize, redraw, and mark the document edited.
    fn settle(&self) {
        self.state.borrow_mut().clamp(&self.app);
        self.reflow();
        self.fit();
        self.redraw();
        if let Some(callback) = self.on_change.borrow().as_ref() {
            callback();
        }
    }

    fn redraw(&self) {
        if let Some(view) = self.view() {
            view.setNeedsDisplay(true);
        }
    }

    /// Where the caret is drawn, in the page's own coordinates.
    fn caret_rect(&self, state: &Page) -> Option<Rect> {
        self.measured(|laid, faces| {
            paint::caret_rect(&self.app, &laid.flow, faces, laid.column.0, state)
        })
    }

    /// The caret scrolled into sight, and the input method told its characters may have moved.
    fn reveal(&self) {
        let Some(view) = self.view() else {
            return;
        };
        if let Some(caret) = self.caret_rect(&self.state.borrow()) {
            // A little room either side, so the caret is never flush with the edge.
            let room = Rect::new(caret.x - 8.0, caret.y - 8.0, caret.w + 16.0, caret.h + 16.0);
            view.scrollRectToVisible(ns_rect(room));
        }
        if let Some(context) = view.inputContext() {
            context.invalidateCharacterCoordinates();
        }
    }

    /// How many lines a page is: what shows, in body lines, less one kept for context.
    fn page_lines(&self) -> isize {
        let line = f64::from(
            self.text
                .height_of(&face::font(Role::Body, &TextStyle::default())),
        )
        .max(1.0);
        let shown = self
            .view()
            .map_or(0.0, |view| view.visibleRect().size.height);
        ((shown / line).floor() as isize - 1).max(1)
    }

    /// Run `f` over the page's state and the document — a motion or an edit — then show where
    /// the caret went. The core's notification of an edit waits until `f` is done.
    pub(crate) fn act_on(
        &self,
        f: impl FnOnce(&mut Page, &App, &dyn Faces) -> Result<(), Refused>,
    ) {
        self.busy.set(true);
        let done = {
            let mut state = self.state.borrow_mut();
            self.measured(|_, faces| f(&mut state, &self.app, faces))
        };
        self.busy.set(false);
        if self.stale.replace(false) {
            self.settle();
        }
        if done.is_err() {
            // An edit the core refused — a caret in a block that has gone — is the platform's
            // beep, as a key with no meaning is.
            NSBeep();
        }
        self.redraw();
        self.reveal();
    }

    /// What a selector does.
    fn act(&self, action: PageAction) {
        let lines = self.page_lines();
        match action {
            PageAction::Move { motion, extend } => self.act_on(|page, app, faces| {
                page.navigate(app, faces, motion, extend, lines);
                Ok(())
            }),
            PageAction::Collapse => self.act_on(|page, _, _| {
                page.place(page.caret, false);
                Ok(())
            }),
            PageAction::Erase { unit, forward } => {
                self.act_on(|page, app, faces| page.erase(app, faces, unit, forward))
            }
            PageAction::Split => self.act_on(|page, app, _| page.split(app)),
            PageAction::LineBreak => self.act_on(|page, app, _| page.insert_plain(app, "\n")),
            PageAction::Tab { back } => self.act_on(|page, app, _| page.tab(app, back)),
            PageAction::LiteralTab => self.act_on(|page, app, _| page.insert_plain(app, "\t")),
            PageAction::Scroll(to) => self.scroll(to),
            PageAction::Center => self.center(),
        }
    }

    /// Scroll without moving the caret — fn↑, fn↓, Home and End, as on every Mac.
    fn scroll(&self, to: Scroll) {
        let Some(view) = self.view() else {
            return;
        };
        let shown = view.visibleRect();
        let (y, h) = (shown.origin.y, shown.size.height);
        let last = (view.frame().size.height - h).max(0.0);
        let y = match to {
            Scroll::PageUp => y - h,
            Scroll::PageDown => y + h,
            Scroll::Top => 0.0,
            Scroll::Bottom => last,
        };
        view.scrollPoint(NSPoint::new(0.0, y.clamp(0.0, last)));
    }

    /// ⌃L: the caret's line in the middle of what shows.
    fn center(&self) {
        let (Some(view), Some(caret)) = (self.view(), self.caret_rect(&self.state.borrow())) else {
            return;
        };
        let h = view.visibleRect().size.height;
        let last = (view.frame().size.height - h).max(0.0);
        let y = caret.y + caret.h / 2.0 - h / 2.0;
        view.scrollPoint(NSPoint::new(0.0, y.clamp(0.0, last)));
    }

    /// The caret a point on the page lands on — nearest, never nothing.
    fn hit(&self, at: NSPoint) -> Option<Caret> {
        self.measured(|laid, column| {
            grind_text::caret::hit(&laid.flow, at.x - laid.column.0, at.y, |block| {
                let viewport = self.app.get_viewport(block..block + 1);
                let view = viewport.get(block)?;
                let (width, metrics) = column.of(block, &view.kind, view.style.as_deref());
                self.app.layout_block(block, width, metrics).ok()
            })
        })
    }

    /// A click: the caret there, a word on a double-click, the block on a triple — and marked
    /// text committed first, as it is when a Mac text view is clicked.
    fn click(&self, at: NSPoint, count: isize, extend: bool) {
        let Some(caret) = self.hit(at) else {
            return;
        };
        self.act_on(|page, app, _| {
            page.unmark(app)?;
            match count {
                ..=1 => page.place(caret, extend),
                2 => page.select_word(app, caret),
                _ => page.select_block(app, caret),
            }
            Ok(())
        });
    }

    /// Edit ▸ Copy: the selection as plain text, a block break a newline.
    fn copy(&self) {
        if let Some(text) = self.state.borrow().selected_text(&self.app) {
            clipboard::write(&text, false);
        }
    }

    /// The palette the page is drawn in, resolved in the view's own appearance (decision 8).
    fn palette() -> Palette {
        let page = rgb(&NSColor::textBackgroundColor());
        Palette {
            page,
            ink: rgb(&NSColor::textColor()),
            accent: rgb(&NSColor::controlAccentColor()),
            dark: color::luminance(page) < 0.5,
        }
    }

    /// Where the caret is, for a drive's transcript: its address, or the selection's two.
    pub fn caret_text(&self) -> String {
        let state = self.state.borrow();
        let at = |caret: Caret| grind_text::loc::format_offset(caret.block, caret.offset);
        match state.selection() {
            Some((from, to)) => format!("{} to {}", at(from), at(to)),
            None => at(state.shown_caret()),
        }
    }
}

impl crate::watch::Watched for TextPane {
    fn document_changed(&self) {
        match self.busy.get() {
            true => self.stale.set(true),
            false => self.settle(),
        }
    }
}

/// Have the core tell `pane` about every change to its document from now on.
pub fn watch(pane: &Rc<TextPane>) {
    pane.app.set_observer(crate::watch::observer(pane));
}

/// A string an input method handed over, which is an `NSString` or an `NSAttributedString`.
fn string_of(object: &AnyObject) -> String {
    object
        .downcast_ref::<NSString>()
        .map(|text| text.to_string())
        .or_else(|| {
            object
                .downcast_ref::<NSAttributedString>()
                .map(|text| text.string().to_string())
        })
        .unwrap_or_default()
}

fn range(range: NSRange) -> std::ops::Range<usize> {
    match range.location {
        NOT_FOUND => NOT_FOUND..NOT_FOUND,
        start => start..start.saturating_add(range.length),
    }
}

fn ns_range(range: std::ops::Range<usize>) -> NSRange {
    NSRange::new(range.start, range.end.saturating_sub(range.start))
}

define_class!(
    /// The page, as the scroll view's document view.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindPageView"]
    #[ivars = Rc<TextPane>]
    pub struct PageView;

    impl PageView {
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

        /// The caret shows only while the page has the keyboard, so gaining or losing it
        /// redraws.
        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            self.setNeedsDisplay(true);
            true
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            self.setNeedsDisplay(true);
            true
        }

        /// The scroll view changed size: the page follows it across, and is laid out again when
        /// its width changed.
        #[unsafe(method(resizeWithOldSuperviewSize:))]
        fn resize_with_old_superview_size(&self, _old: NSSize) {
            self.ivars().fit();
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            let pane = self.ivars();
            let palette = TextPane::palette();
            let window = self.window();
            let focused = window.as_ref().is_some_and(|window| {
                window.isKeyWindow()
                    && window.firstResponder().is_some_and(|responder| {
                        std::ptr::eq(
                            Retained::as_ptr(&responder).cast::<()>(),
                            (self as *const Self).cast::<()>(),
                        )
                    })
            });
            let state = pane.state.borrow();
            let ops = pane.measured(|laid, faces| {
                paint::frame(&Frame {
                    app: &pane.app,
                    flow: &laid.flow,
                    faces,
                    column_x: laid.column.0,
                    view: rect(dirty),
                    state: &state,
                    caret: focused,
                    palette: &palette,
                })
            });
            let Some(context) = NSGraphicsContext::currentContext() else {
                return;
            };
            let cg: Retained<CGContext> = context.CGContext();
            render::draw(&cg, &ops, &pane.text);
        }

        /// A key goes to the text system, which answers through the input client below.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.interpretKeyEvents(&NSArray::from_slice(&[event]));
        }

        /// `NSResponder`'s older `insertText:`, which some senders still use — a Services menu
        /// item, a script — rather than the input client's form below.
        #[unsafe(method(insertText:))]
        fn insert_text(&self, string: &AnyObject) {
            let text = string_of(string);
            self.ivars().act_on(|page, app, _| {
                input::insert(app, page, &text, NOT_FOUND..NOT_FOUND)
            });
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
            }
            let at = located(self, event);
            self.ivars().click(at, event.clickCount(), shifted(event));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let at = located(self, event);
            if let Some(caret) = self.ivars().hit(at) {
                self.ivars().act_on(|page, _, _| {
                    page.place(caret, true);
                    Ok(())
                });
            }
        }

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
            let pane = self.ivars();
            pane.copy();
            pane.act_on(|page, app, _| page.drop_selection(app).map(|_| ()));
        }

        /// Edit ▸ Paste: the pasteboard's plain text, a newline a new block.
        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            if let Some(text) = clipboard::read() {
                self.ivars().act_on(|page, app, _| page.paste(app, &text));
            }
        }

        #[unsafe(method(delete:))]
        fn delete(&self, _sender: Option<&AnyObject>) {
            self.ivars()
                .act_on(|page, app, _| page.drop_selection(app).map(|_| ()));
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            self.ivars().act_on(|page, app, _| {
                page.select_all(app);
                Ok(())
            });
        }

        /// Edit ▸ Undo — the core's history, never AppKit's (architecture rule 2).
        #[unsafe(method(undo:))]
        fn undo(&self, _sender: Option<&AnyObject>) {
            self.ivars().act_on(|page, app, _| {
                page.history(app, true);
                Ok(())
            });
        }

        #[unsafe(method(redo:))]
        fn redo(&self, _sender: Option<&AnyObject>) {
            self.ivars().act_on(|page, app, _| {
                page.history(app, false);
                Ok(())
            });
        }

        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let pane = self.ivars();
            let selected = pane.state.borrow().selection().is_some();
            let action = item
                .action()
                .map(|action| action.name().to_str().unwrap_or_default().to_owned());
            match action.as_deref() {
                Some("undo:") => pane.app.can_undo(),
                Some("redo:") => pane.app.can_redo(),
                Some("paste:") => clipboard::can_paste(),
                Some("copy:" | "cut:" | "delete:") => selected,
                _ => true,
            }
        }
    }

    unsafe impl NSObjectProtocol for PageView {}

    /// The input client: what the text system says a key means, and what an input method is
    /// composing (`text/input.rs` for every range).
    unsafe impl NSTextInputClient for PageView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text_replacement_range(&self, string: &AnyObject, replacement: NSRange) {
            let text = string_of(string);
            self.ivars()
                .act_on(|page, app, _| input::insert(app, page, &text, range(replacement)));
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, selector: Sel) {
            let name = selector.name().to_str().unwrap_or_default();
            match keys::page_action(name) {
                Some(action) => self.ivars().act(action),
                None => NSBeep(),
            }
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, selected: NSRange, replacement: NSRange) {
            let text = string_of(string);
            self.ivars().act_on(|page, app, _| {
                input::mark(app, page, &text, range(selected), range(replacement))
            });
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.ivars().act_on(|page, app, _| page.unmark(app));
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            let pane = self.ivars();
            ns_range(input::selected_range(&pane.app, &pane.state.borrow()))
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let pane = self.ivars();
            input::marked_range(&pane.app, &pane.state.borrow())
                .map_or(NSRange::new(NOT_FOUND, 0), ns_range)
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            self.ivars().state.borrow().composing.is_some()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring(
            &self,
            proposed: NSRange,
            actual: *mut NSRange,
        ) -> Option<Retained<NSAttributedString>> {
            // A `method_id` body may not return early, so the work is a function's.
            substring(self.ivars(), proposed, actual)
        }

        /// No attributes are taken from marked text: it is drawn in the run's own face and
        /// underlined, the Mac convention, whatever the input method suggests.
        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        /// Where a range is on the screen — the candidate window goes under it.
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect_for_character_range(&self, wanted: NSRange, actual: *mut NSRange) -> NSRect {
            let pane = self.ivars();
            let state = pane.state.borrow();
            let at = range(wanted);
            let units = if at.start == NOT_FOUND { 0 } else { at.start };
            let caret = input::caret_at(&pane.app, &state, units);
            // The same state with its caret at that unit — inside the marked text when there is
            // some, so the rectangle comes from the laid-out composition.
            let mut probe = state.clone();
            match &mut probe.composing {
                Some(composing) => {
                    let inside = caret.offset.saturating_sub(state.caret.offset);
                    composing.selected = inside..inside;
                }
                None => probe.caret = caret,
            }
            drop(state);
            if !actual.is_null() {
                // SAFETY: a non-null `actualRange` is an out-pointer the caller owns.
                unsafe { *actual = NSRange::new(units, 0) };
            }
            let Some(place) = pane.caret_rect(&probe) else {
                return NSRect::ZERO;
            };
            let in_window = self.convertRect_toView(ns_rect(place), None);
            self.window()
                .map_or(in_window, |window| window.convertRectToScreen(in_window))
        }

        /// Which unit of the caret's block a point on the screen is over, or nothing when it is
        /// over another block.
        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, point: NSPoint) -> NSUInteger {
            let Some(window) = self.window() else {
                return NOT_FOUND;
            };
            let at = self.convertPoint_fromView(window.convertPointFromScreen(point), None);
            let pane = self.ivars();
            let state = pane.state.borrow();
            match pane.hit(at) {
                Some(caret) if caret.block == state.caret.block => {
                    input::units(&input::seen(&pane.app, &state), caret.offset)
                }
                _ => NOT_FOUND,
            }
        }
    }
);

/// `attributedSubstringForProposedRange:actualRange:`'s answer.
fn substring(
    pane: &TextPane,
    proposed: NSRange,
    actual: *mut NSRange,
) -> Option<Retained<NSAttributedString>> {
    let (text, covered) = input::substring(&pane.app, &pane.state.borrow(), range(proposed))?;
    if !actual.is_null() {
        // SAFETY: a non-null `actualRange` is an out-pointer the caller owns.
        unsafe { *actual = ns_range(covered) };
    }
    Some(NSAttributedString::from_nsstring(&NSString::from_str(
        &text,
    )))
}

fn frame(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// The word processor's scroll view, with the page in it. The view holds `pane`, and the scroll
/// view holds the view.
pub fn page_view(
    pane: &Rc<TextPane>,
    size: NSSize,
    mtm: MainThreadMarker,
) -> Retained<NSScrollView> {
    let scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        frame(0.0, 0.0, size.width, size.height),
    );
    scroll.setHasVerticalScroller(true);
    scroll.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let page: Retained<PageView> = {
        let this = PageView::alloc(mtm).set_ivars(pane.clone());
        // SAFETY: `initWithFrame:` is `NSView`'s designated initialiser.
        unsafe { msg_send![super(this), initWithFrame: frame(0.0, 0.0, size.width, size.height)] }
    };
    scroll.setDocumentView(Some(&page));
    *pane.view.borrow_mut() = Some(Weak::from_retained(&page));
    pane.fit();
    scroll
}
