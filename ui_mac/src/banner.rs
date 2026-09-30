// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The notice banner: a state the document is in, under the name box (decision 3; M4).
//!
//! A second titlebar accessory, hidden when there is nothing to say — so it takes on the
//! window's own material like the first, and appearing is AppKit's animation rather than this
//! shell's. Every sentence is `notice.rs`'s; this is where they are shown, and where the one
//! notice that offers an action — a recalculation this build declined — carries its button.

use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSColor, NSLayoutAttribute, NSTextField,
    NSTitlebarAccessoryViewController, NSView, NSWindow,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};

use crate::grid_view::Pane;
use crate::notice;

const BAR_H: f64 = 32.0;
const BUTTON_W: f64 = 170.0;

/// What the banner's button does, when it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    RecalculateAnyway,
}

/// The banner of one window: its controller, its sentence and its button.
pub struct Banner {
    controller: Retained<NSTitlebarAccessoryViewController>,
    text: Retained<NSTextField>,
    button: Retained<NSButton>,
}

impl Banner {
    /// Say `sentence`, with a button for `action` when there is one — or nothing, and hide.
    pub fn say(&self, said: Option<(&str, Option<Action>)>) {
        match said {
            None => self.controller.setHidden(true),
            Some((sentence, action)) => {
                self.text.setStringValue(&NSString::from_str(sentence));
                match action {
                    Some(Action::RecalculateAnyway) => {
                        self.button
                            .setTitle(&NSString::from_str(notice::RECALCULATE_ANYWAY));
                        self.button.setHidden(false);
                    }
                    None => self.button.setHidden(true),
                }
                self.controller.setHidden(false);
            }
        }
    }
}

define_class!(
    /// The banner button's target. A control holds its target weakly, so the pane keeps this.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindBannerButton"]
    #[ivars = Weak<Pane>]
    pub struct BannerButton;

    impl BannerButton {
        #[unsafe(method(bannerButton:))]
        fn pressed(&self, _sender: Option<&AnyObject>) {
            if let Some(pane) = self.ivars().upgrade() {
                pane.recalculate_anyway();
            }
        }
    }
);

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

/// A hidden banner under `window`'s title, answering to `pane`.
pub fn attach(window: &NSWindow, pane: &Rc<Pane>, mtm: MainThreadMarker) -> Banner {
    let width = window.frame().size.width;
    let bar = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, width, BAR_H));
    bar.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);

    let text = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    text.setFrame(rect(12.0, 8.0, width - BUTTON_W - 36.0, 16.0));
    text.setTextColor(Some(&NSColor::labelColor()));
    text.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    bar.addSubview(&text);

    let target: Retained<BannerButton> = {
        let this = BannerButton::alloc(mtm).set_ivars(Rc::downgrade(pane));
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };
    // SAFETY: the target answers the action, and the pane keeps it for as long as the views are.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(notice::RECALCULATE_ANYWAY),
            Some(&target),
            Some(sel!(bannerButton:)),
            mtm,
        )
    };
    button.setFrame(rect(width - BUTTON_W - 12.0, 3.0, BUTTON_W, 26.0));
    button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
    button.setHidden(true);
    bar.addSubview(&button);
    pane.keep(target.into_super());

    let controller = NSTitlebarAccessoryViewController::new(mtm);
    controller.setView(&bar);
    controller.setLayoutAttribute(NSLayoutAttribute::Bottom);
    controller.setHidden(true);
    window.addTitlebarAccessoryViewController(&controller);
    Banner {
        controller,
        text,
        button,
    }
}
