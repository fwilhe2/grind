// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Format ▸ Image Description… — a picture's alternative text, in the words Pages uses for it.
//!
//! An `NSAlert` whose accessory view holds the picture — `NSImage` decodes the document's own
//! bytes, so it is the picture as it is, not a thumbnail of this shell's making — a field for the
//! short description VoiceOver reads in the picture's place (`svg:title`), the one sentence of
//! advice `grind_text::picture::advice` gives every client, kept up to date as it is typed by the
//! field's delegate, and a scrolling text view for the long description (`svg:desc`). The
//! platform's own shape for a question with a picture in it, `chart_preview.rs`'s sibling.

use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSColor, NSControlTextEditingDelegate, NSFont, NSImage,
    NSImageScaling, NSImageView, NSTextField, NSTextFieldDelegate, NSTextView, NSView,
};
use objc2_foundation::{
    NSData, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

/// The accessory's width, and the heights of its rows, in points, bottom up — an alert's
/// accessory view is not flipped.
const WIDTH: f64 = 360.0;
const LONG_H: f64 = 72.0;
const LABEL_H: f64 = 18.0;
const FIELD_H: f64 = 24.0;
const ADVICE_H: f64 = 32.0;
const PICTURE_H: f64 = 150.0;
const GAP: f64 = 8.0;

define_class!(
    /// The short field's delegate: the advice under it, said again as it changes.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindImageDescriptionAdvice"]
    #[ivars = Retained<NSTextField>]
    struct Advice;

    unsafe impl NSObjectProtocol for Advice {}

    unsafe impl NSControlTextEditingDelegate for Advice {
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self, notification: &NSNotification) {
            let Some(field) = notification
                .object()
                .and_then(|object| object.downcast::<NSTextField>().ok())
            else {
                return;
            };
            advise(self.ivars(), &field.stringValue().to_string());
        }
    }

    unsafe impl NSTextFieldDelegate for Advice {}
);

/// What the advice line says for `title`: the advice if there is any, and the count.
fn advise(line: &NSTextField, title: &str) {
    let count = title.trim().chars().count();
    let advice = grind_text::picture::advice(title).unwrap_or("");
    let said = match advice.is_empty() {
        true => format!(
            "{count} of about {} characters.",
            grind_text::picture::SHORT
        ),
        false => format!("{advice} {count} of about {}.", grind_text::picture::SHORT),
    };
    line.setStringValue(&NSString::from_str(&said));
    line.setTextColor(Some(&*match count > grind_text::picture::SHORT {
        true => NSColor::systemOrangeColor(),
        false => NSColor::secondaryLabelColor(),
    }));
}

/// Ask for a picture's description: `picture` is its bytes, `title` and `description` what it
/// says now. `Some((title, description))` for Save, `None` for Cancel.
pub fn ask(
    mtm: MainThreadMarker,
    picture: &[u8],
    title: &str,
    description: &str,
) -> Option<(String, String)> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Image Description"));
    alert.setInformativeText(&NSString::from_str(
        "What VoiceOver says in the picture's place, and in Markdown its alt text.",
    ));
    alert.addButtonWithTitle(&NSString::from_str("Save"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    let row = |y: f64, h: f64| NSRect::new(NSPoint::new(0.0, y), NSSize::new(WIDTH, h));
    let label = |text: &str, frame: NSRect| {
        let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        label.setFrame(frame);
        label
    };

    // Bottom up: the long description, its label, the advice, the field, its label, the picture.
    let mut y = 0.0;
    let long = NSTextView::scrollableTextView(mtm);
    long.setFrame(row(y, LONG_H));
    long.setBorderType(objc2_app_kit::NSBorderType::BezelBorder);
    let long_view = long
        .documentView()
        .and_then(|view| view.downcast::<NSTextView>().ok());
    if let Some(view) = &long_view {
        view.setString(&NSString::from_str(description));
        view.setFont(Some(&NSFont::systemFontOfSize(NSFont::systemFontSize())));
        view.setRichText(false);
    }
    y += LONG_H + GAP / 2.0;
    let long_label = label("Long description (optional)", row(y, LABEL_H));
    y += LABEL_H + GAP;

    let advice = NSTextField::wrappingLabelWithString(&NSString::from_str(""), mtm);
    advice.setFrame(row(y, ADVICE_H));
    advice.setFont(Some(&NSFont::systemFontOfSize(
        NSFont::smallSystemFontSize(),
    )));
    advise(&advice, title);
    y += ADVICE_H + GAP / 2.0;

    let field = NSTextField::textFieldWithString(&NSString::from_str(title), mtm);
    field.setFrame(row(y, FIELD_H));
    field.setPlaceholderString(Some(&NSString::from_str("Describe the picture")));
    y += FIELD_H + GAP / 2.0;
    let field_label = label("Description", row(y, LABEL_H));
    y += LABEL_H + GAP;

    let shown = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(picture));
    let mut views: Vec<Retained<NSView>> = vec![
        Retained::into_super(long),
        Retained::into_super(Retained::into_super(long_label)),
        Retained::into_super(Retained::into_super(advice.clone())),
        Retained::into_super(Retained::into_super(field.clone())),
        Retained::into_super(Retained::into_super(field_label)),
    ];
    if let Some(image) = shown {
        let view = NSImageView::imageViewWithImage(&image, mtm);
        view.setImageScaling(NSImageScaling::ScaleProportionallyDown);
        view.setFrame(row(y, PICTURE_H));
        y += PICTURE_H + GAP;
        views.push(Retained::into_super(Retained::into_super(view)));
    }

    let accessory = NSView::initWithFrame(NSView::alloc(mtm), row(0.0, y));
    for view in &views {
        accessory.addSubview(view);
    }
    let delegate: Retained<Advice> = {
        let this = Advice::alloc(mtm).set_ivars(advice);
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };
    // SAFETY: the delegate is a weak reference; it lives until after the alert has run and is
    // taken off the field before it is dropped.
    unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    alert.setAccessoryView(Some(&accessory));
    alert.layout();
    alert.window().setInitialFirstResponder(Some(&field));

    let saved = alert.runModal() == NSAlertFirstButtonReturn;
    // SAFETY: clearing the weak reference before its target goes.
    unsafe { field.setDelegate(None) };
    drop(delegate);
    saved.then(|| {
        (
            field.stringValue().to_string(),
            long_view
                .map(|view| view.string().to_string())
                .unwrap_or_default(),
        )
    })
}
