// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The scratch calculator's window: `tape.rs`'s frame drawn in the system's colours, one field
//! over the caret's row, and the buttons under the rows. Every selector the field editor hands its
//! delegate goes to `grind_sheet::TapeEditor` first; after each answer [`TapeView::sync`] moves
//! the field to the caret's row and fills it. The window is kept, hidden, between openings.

use std::cell::{Cell, OnceCell, RefCell};

use grind_core::utf16;
use grind_sheet::{Tape, TapeEditor};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSColor, NSControl, NSControlTextEditingDelegate, NSEvent,
    NSFont, NSForegroundColorAttributeName, NSGraphicsContext, NSModalResponseOK, NSOpenPanel,
    NSTextField, NSTextFieldDelegate, NSTextView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_graphics::CGContext;
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString,
};

use crate::grid_view::{located, rgb};
use crate::metrics::{BASE_PT, CoreText};
use crate::render;
use crate::sheet::geom::Rect;
use crate::tape::{self, Palette};

/// What the window's view holds.
pub struct Held {
    editor: RefCell<TapeEditor>,
    top: Cell<usize>,
    field: OnceCell<Retained<NSTextField>>,
    delegate: OnceCell<Retained<TapeFieldDelegate>>,
    undo: OnceCell<Retained<NSButton>>,
    redo: OnceCell<Retained<NSButton>>,
    text: CoreText,
    /// Set while [`TapeView::sync`] writes into the field, so the change is not typing.
    syncing: Cell<bool>,
    open: Box<dyn Fn(grind_sheet::Document)>,
}

fn palette() -> Palette {
    Palette {
        ground: rgb(&NSColor::windowBackgroundColor()),
        ink: rgb(&NSColor::labelColor()),
        muted: rgb(&NSColor::secondaryLabelColor()),
        divider: rgb(&NSColor::separatorColor()),
        accent: rgb(&NSColor::controlAccentColor()),
        red: rgb(&NSColor::systemRedColor()),
        purple: rgb(&NSColor::systemPurpleColor()),
        green: rgb(&NSColor::systemGreenColor()),
        orange: rgb(&NSColor::systemOrangeColor()),
    }
}

fn ns_color((r, g, b): grind_core::color::Rgb) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
        1.0,
    )
}

fn frame_of(rect: Rect) -> NSRect {
    NSRect::new(NSPoint::new(rect.x, rect.y), NSSize::new(rect.w, rect.h))
}

define_class!(
    /// The window's one view: the rows drawn, and a click on one moves the caret there.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindTapeView"]
    #[ivars = Held]
    pub struct TapeView;

    impl TapeView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let held = self.ivars();
            let ops = tape::frame(&held.editor.borrow(), held.top.get(), &palette(), self.advance());
            if let Some(context) = NSGraphicsContext::currentContext() {
                let cg: Retained<CGContext> = context.CGContext();
                render::draw(&cg, &ops, &held.text);
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let at = located(self, event);
            let Some(shown) = tape::hit(at.x, at.y) else {
                return;
            };
            let held = self.ivars();
            self.typed();
            let row = held.editor.borrow().rows().get(held.top.get() + shown).copied();
            if let Some(row) = row {
                held.editor.borrow_mut().click(row);
                self.sync();
            }
        }

        #[unsafe(method(tapeUndo:))]
        fn tape_undo(&self, _sender: Option<&AnyObject>) {
            self.ivars().editor.borrow_mut().undo();
            self.sync();
        }

        #[unsafe(method(tapeRedo:))]
        fn tape_redo(&self, _sender: Option<&AnyObject>) {
            self.ivars().editor.borrow_mut().redo();
            self.sync();
        }

        /// A saved tape over this one; the note says it opened, or why the file is not one.
        #[unsafe(method(tapeLoad:))]
        fn tape_load(&self, _sender: Option<&AnyObject>) {
            let panel = NSOpenPanel::openPanel(self.mtm());
            panel.setCanChooseDirectories(false);
            panel.setAllowsMultipleSelection(false);
            panel.setPrompt(Some(&NSString::from_str("Open")));
            if panel.runModal() != NSModalResponseOK {
                return;
            }
            let Some(path) = panel.URL().and_then(|url| url.to_file_path()) else {
                return;
            };
            self.ivars().editor.borrow_mut().open_file(&path);
            self.sync();
        }

        #[unsafe(method(tapeOpen:))]
        fn tape_open(&self, _sender: Option<&AnyObject>) {
            let promoted = self.ivars().editor.borrow().tape().promote();
            match promoted {
                Ok(doc) => {
                    if let Some(window) = self.window() {
                        window.orderOut(None);
                    }
                    (self.ivars().open)(doc);
                }
                Err(error) => {
                    crate::prompt::tell(self.mtm(), "The tape is not a spreadsheet yet.", &error.to_string());
                }
            }
        }

        #[unsafe(method(tapeClose:))]
        fn tape_close(&self, _sender: Option<&AnyObject>) {
            if let Some(window) = self.window() {
                window.orderOut(None);
            }
        }
    }
);

define_class!(
    /// The field's delegate: every selector its field editor would act on comes here first.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindTapeField"]
    #[ivars = Weak<TapeView>]
    pub struct TapeFieldDelegate;

    unsafe impl NSObjectProtocol for TapeFieldDelegate {}

    unsafe impl NSControlTextEditingDelegate for TapeFieldDelegate {
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, _text_view: &NSTextView, selector: Sel) -> bool {
            let name = selector.name().to_str().unwrap_or_default();
            self.ivars().load().is_some_and(|view| view.command(name))
        }

        #[unsafe(method(controlTextDidChange:))]
        fn did_change(&self, _notification: &NSNotification) {
            if let Some(view) = self.ivars().load() {
                view.typed();
            }
        }
    }

    unsafe impl NSTextFieldDelegate for TapeFieldDelegate {}
);

impl TapeView {
    /// The fixed-pitch face's width of one character.
    fn advance(&self) -> f64 {
        let line = self.ivars().text.line_of("0000000000", &tape::font());
        // SAFETY: a null secondary offset is documented as allowed.
        let width = unsafe { line.offset_for_string_index(10, std::ptr::null_mut()) };
        width / 10.0
    }

    /// The field's text and caret, told to the editor; its words coloured again.
    fn typed(&self) {
        let held = self.ivars();
        if held.syncing.get() {
            return;
        }
        let Some(field) = held.field.get() else {
            return;
        };
        let text = field.stringValue().to_string();
        let caret = field.currentEditor().map_or(text.len(), |editor| {
            let range = editor.selectedRange();
            utf16::byte_of(&text, range.location + range.length)
        });
        held.editor.borrow_mut().typed(&text, caret);
        self.colour();
        self.setNeedsDisplay(true);
    }

    /// The field editor's words in the colours `Tape::spans` gives them.
    fn colour(&self) {
        let held = self.ivars();
        let Some(field) = held.field.get() else {
            return;
        };
        let Some(view) = field
            .currentEditor()
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
        else {
            return;
        };
        // SAFETY: the field editor's own storage, read on the main thread.
        let Some(storage) = (unsafe { view.textStorage() }) else {
            return;
        };
        let (text, spans) = {
            let editor = held.editor.borrow();
            editor.shown(editor.row())
        };
        let colours = palette();
        // SAFETY: AppKit's own attribute key, with a colour, over ranges inside the text.
        unsafe {
            let key = NSForegroundColorAttributeName;
            let whole = NSRange::new(0, storage.length());
            storage.beginEditing();
            storage.addAttribute_value_range(key, &NSColor::labelColor(), whole);
            for span in spans {
                let start = utf16::units_before(&text, span.range.start);
                let end = utf16::units_before(&text, span.range.end);
                if end > storage.length() || start >= end {
                    continue;
                }
                let color = ns_color(tape::ink(span.ink, &colours));
                storage.addAttribute_value_range(key, &color, NSRange::new(start, end - start));
            }
            storage.endEditing();
        }
    }

    /// What a selector from the field editor does; `true` when the tape took it.
    fn command(&self, selector: &str) -> bool {
        let Some(key) = tape::key(selector) else {
            return false;
        };
        self.typed();
        let took = self.ivars().editor.borrow_mut().key(key);
        if took {
            self.sync();
        } else if key == grind_sheet::Key::Escape {
            if let Some(window) = self.window() {
                window.orderOut(None);
            }
            return true;
        }
        took
    }

    /// The field moved to the caret's row, scrolled into view, holding the editor's text with
    /// its caret; the buttons and the rows to match.
    fn sync(&self) {
        let held = self.ivars();
        let Some(field) = held.field.get() else {
            return;
        };
        let (text, caret, at, can_undo, can_redo) = {
            let editor = held.editor.borrow();
            let at = editor
                .rows()
                .iter()
                .position(|row| *row == editor.row())
                .unwrap_or(0);
            let tape = editor.tape();
            (
                editor.text().to_owned(),
                editor.caret(),
                at,
                tape.can_undo(),
                tape.can_redo(),
            )
        };
        let top = tape::scrolled(held.top.get(), at, tape::visible());
        held.top.set(top);
        held.syncing.set(true);
        field.setFrame(frame_of(tape::well(at - top)));
        if let Some(window) = self.window()
            && field.currentEditor().is_none()
        {
            window.makeFirstResponder(Some(field));
        }
        let units = utf16::units_before(&text, caret);
        match field.currentEditor() {
            Some(editor) => {
                if editor.string().to_string() != text {
                    editor.setString(&NSString::from_str(&text));
                }
                editor.setSelectedRange(NSRange::new(units, 0));
            }
            None => field.setStringValue(&NSString::from_str(&text)),
        }
        held.syncing.set(false);
        self.colour();
        if let Some(button) = held.undo.get() {
            button.setEnabled(can_undo);
        }
        if let Some(button) = held.redo.get() {
            button.setEnabled(can_redo);
        }
        self.setNeedsDisplay(true);
    }
}

/// The scratch calculator's window over `tape`, shown and made key. `open` takes the tape as a
/// spreadsheet when it is opened as one. The caller keeps the window and shows it again.
pub fn show(
    tape: Tape,
    open: impl Fn(grind_sheet::Document) + 'static,
    mtm: MainThreadMarker,
) -> Retained<NSWindow> {
    let size = NSSize::new(tape::WIDTH, tape::HEIGHT);
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);
    // SAFETY: a titled, closable, buffered window is the ordinary kind.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: the caller owns the window, so AppKit must not also release it on close.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("Scratch Calculator"));
    let view: Retained<TapeView> = {
        let this = TapeView::alloc(mtm).set_ivars(Held {
            editor: RefCell::new(TapeEditor::new(tape)),
            top: Cell::new(0),
            field: OnceCell::new(),
            delegate: OnceCell::new(),
            undo: OnceCell::new(),
            redo: OnceCell::new(),
            text: CoreText::new(BASE_PT),
            syncing: Cell::new(false),
            open: Box::new(open),
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initialiser.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    };

    let field = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
    field.setFont(Some(
        &NSFont::userFixedPitchFontOfSize(tape::SIZE)
            .unwrap_or_else(|| NSFont::systemFontOfSize(tape::SIZE)),
    ));
    let delegate: Retained<TapeFieldDelegate> = {
        let this = TapeFieldDelegate::alloc(mtm).set_ivars(Weak::from_retained(&view));
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };
    // SAFETY: the delegate answers the protocol, and the view keeps it as long as the field.
    unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    view.addSubview(&field);

    let footer = tape::footer();
    let button = |title: &str, action: Sel| {
        // SAFETY: the view answers every action it is given here.
        let made = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(&view),
                Some(action),
                mtm,
            )
        };
        view.addSubview(&made);
        made
    };
    let undo = button("Undo", sel!(tapeUndo:));
    let redo = button("Redo", sel!(tapeRedo:));
    let load = button("Open Tape…", sel!(tapeLoad:));
    let close = button("Close", sel!(tapeClose:));
    let opening = button("Open as Spreadsheet", sel!(tapeOpen:));
    let y = footer.y + (footer.h - 32.0) / 2.0;
    undo.setFrame(NSRect::new(
        NSPoint::new(footer.x, y),
        NSSize::new(80.0, 32.0),
    ));
    redo.setFrame(NSRect::new(
        NSPoint::new(footer.x + 86.0, y),
        NSSize::new(80.0, 32.0),
    ));
    load.setFrame(NSRect::new(
        NSPoint::new(footer.x + 172.0, y),
        NSSize::new(110.0, 32.0),
    ));
    let right = footer.x + footer.w;
    opening.setFrame(NSRect::new(
        NSPoint::new(right - 170.0, y),
        NSSize::new(170.0, 32.0),
    ));
    close.setFrame(NSRect::new(
        NSPoint::new(right - 266.0, y),
        NSSize::new(90.0, 32.0),
    ));
    let held = view.ivars();
    let _ = held.field.set(field.clone());
    let _ = held.delegate.set(delegate);
    let _ = held.undo.set(undo);
    let _ = held.redo.set(redo);

    window.setContentView(Some(&view));
    window.center();
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&field));
    view.sync();
    window
}
