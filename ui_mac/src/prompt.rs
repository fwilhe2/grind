// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One line asked for, and a sentence said: an `NSAlert`, which is the platform's own shape for
//! both — a sheet's new name, an address on a page, a column's width, a name for a range, a
//! table's size. One function, so every question in the shell looks and behaves alike.

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSPopUpButton, NSTextField};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// Ask for one line: `title` and `detail` above a field holding `initial`, answered with
/// `action` or Cancel. The answer trimmed, or `None` when cancelled or left empty.
pub fn ask(
    mtm: MainThreadMarker,
    title: &str,
    detail: &str,
    action: &str,
    initial: &str,
) -> Option<String> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(detail));
    alert.addButtonWithTitle(&NSString::from_str(action));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let field = NSTextField::textFieldWithString(&NSString::from_str(initial), mtm);
    field.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(240.0, 24.0),
    ));
    alert.setAccessoryView(Some(&field));
    alert.window().setInitialFirstResponder(Some(&field));
    (alert.runModal() == NSAlertFirstButtonReturn)
        .then(|| field.stringValue().to_string().trim().to_owned())
        .filter(|answer| !answer.is_empty())
}

/// Ask for one of `items`: a pop-up under an alert, `initial` chosen, answered with `action` or
/// Cancel. The chosen row's index, or `None` when cancelled.
pub fn pick(
    mtm: MainThreadMarker,
    title: &str,
    detail: &str,
    action: &str,
    items: &[String],
    initial: usize,
) -> Option<usize> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(detail));
    alert.addButtonWithTitle(&NSString::from_str(action));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 26.0)),
        false,
    );
    for item in items {
        popup.addItemWithTitle(&NSString::from_str(item));
    }
    popup.selectItemAtIndex(isize::try_from(initial).unwrap_or(0));
    alert.setAccessoryView(Some(&popup));
    (alert.runModal() == NSAlertFirstButtonReturn)
        .then(|| usize::try_from(popup.indexOfSelectedItem()).ok())
        .flatten()
}

/// Say what went wrong, in an alert with nothing to answer but OK.
pub fn tell(mtm: MainThreadMarker, message: &str, detail: &str) {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(message));
    alert.setInformativeText(&NSString::from_str(detail));
    alert.runModal();
}
