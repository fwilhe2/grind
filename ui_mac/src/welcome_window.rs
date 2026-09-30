// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The welcome window's AppKit half (decision 6, M9): a small window whose one view draws
//! `welcome.rs`'s frame — the same `Op`s `--render-to` puts in a PNG — in the system's colours,
//! and answers the keys by selector and the pointer by `Welcome::hit`. What a choice *does* is
//! the application delegate's, handed in as a callback, so this file runs no command itself.

use std::cell::RefCell;
use std::path::PathBuf;

use grind_core::color::luminance;
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSGraphicsContext, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_graphics::CGContext;
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

use crate::grid_view::{located, rgb};
use crate::metrics::{BASE_PT, CoreText};
use crate::render;
use crate::welcome::{Choice, Palette, Target, Welcome, frame};

/// What was chosen.
pub enum Picked {
    Choice(Choice),
    Recent(PathBuf),
}

/// What the view holds.
pub struct Held {
    welcome: RefCell<Welcome>,
    /// The recent documents' paths, in the order their names are shown.
    paths: Vec<PathBuf>,
    text: CoreText,
    pick: Box<dyn Fn(Picked)>,
}

/// The system's colours, resolved in the view's appearance at draw time.
fn palette() -> Palette {
    let ground = rgb(&NSColor::windowBackgroundColor());
    let card = rgb(&NSColor::controlBackgroundColor());
    // A card must stand off the ground in both appearances; the control background is the
    // ground itself in some, and a step towards the ink is the card then.
    let card = match card == ground {
        true => {
            let dark = luminance(ground) < 0.5;
            let step = |c: u8| match dark {
                true => c.saturating_add(14),
                false => c.saturating_sub(8),
            };
            (step(ground.0), step(ground.1), step(ground.2))
        }
        false => card,
    };
    Palette {
        ground,
        card,
        ink: rgb(&NSColor::labelColor()),
        muted: rgb(&NSColor::secondaryLabelColor()),
        accent: rgb(&NSColor::controlAccentColor()),
    }
}

define_class!(
    /// The welcome window's one view.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindWelcomeView"]
    #[ivars = Held]
    pub struct WelcomeView;

    impl WelcomeView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let held = self.ivars();
            let ops = frame(&held.welcome.borrow(), &palette());
            if let Some(context) = NSGraphicsContext::currentContext() {
                let cg: Retained<CGContext> = context.CGContext();
                render::draw(&cg, &ops, &held.text);
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.interpretKeyEvents(&NSArray::from_slice(&[event]));
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, selector: Sel) {
            self.command(selector.name().to_str().unwrap_or_default());
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let at = located(self, event);
            let hit = self.ivars().welcome.borrow().hit(at.x, at.y);
            if let Some(target) = hit {
                self.ivars().welcome.borrow_mut().focus_on(target);
                self.setNeedsDisplay(true);
                self.pick(target);
            }
        }
    }
);

impl WelcomeView {
    fn command(&self, selector: &str) {
        let step = match selector {
            "moveUp:" | "moveLeft:" | "insertBacktab:" => -1,
            "moveDown:" | "moveRight:" | "insertTab:" => 1,
            "insertNewline:" | "insertParagraphSeparator:" => {
                let target = self.ivars().welcome.borrow().focused();
                self.pick(target);
                return;
            }
            "cancelOperation:" => {
                if let Some(window) = self.window() {
                    window.performClose(None);
                }
                return;
            }
            _ => return,
        };
        self.ivars().welcome.borrow_mut().step(step);
        self.setNeedsDisplay(true);
    }

    /// A choice, handed to the delegate, and the window closed behind it — what is chosen opens
    /// a window of its own.
    fn pick(&self, target: Target) {
        let held = self.ivars();
        let picked = match target {
            Target::Choice(at) => Choice::ALL.get(at).copied().map(Picked::Choice),
            Target::Recent(at) => held.paths.get(at).cloned().map(Picked::Recent),
        };
        let Some(picked) = picked else { return };
        if let Some(window) = self.window() {
            window.orderOut(None);
        }
        (held.pick)(picked);
    }
}

/// The welcome window, listing `recent` — paths, newest first — and handing every choice to
/// `pick`. Shown and made key; the caller keeps it.
pub fn show(
    recent: Vec<PathBuf>,
    pick: impl Fn(Picked) + 'static,
    mtm: MainThreadMarker,
) -> Retained<NSWindow> {
    let size = NSSize::new(crate::welcome::WIDTH, crate::welcome::HEIGHT);
    let frame_rect = NSRect::new(NSPoint::new(0.0, 0.0), size);
    // SAFETY: a titled, closable, buffered window is the ordinary kind.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame_rect,
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: the caller owns the window, so AppKit must not also release it on close.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("Welcome to Grind"));
    let names = recent
        .iter()
        .map(|path| {
            path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let view: Retained<WelcomeView> = {
        let this = WelcomeView::alloc(mtm).set_ivars(Held {
            welcome: RefCell::new(Welcome {
                recent: names,
                focus: 0,
            }),
            paths: recent,
            text: CoreText::new(BASE_PT),
            pick: Box::new(pick),
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initialiser.
        unsafe { msg_send![super(this), initWithFrame: frame_rect] }
    };
    window.setContentView(Some(&view));
    window.center();
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&view));
    window
}
