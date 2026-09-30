// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The toolbar (M7): `tools.rs`'s table as AppKit controls — segmented controls for the toggles,
//! pop-ups for the number format and the paragraph kind, colour wells for the two colours.
//!
//! One object is the toolbar's delegate **and** every control's target, and it holds the pane
//! as a [`Formats`], so nothing here knows which kind of document it is over. Every control
//! **reads before it shows**: whenever the pane says something changed, each segment is
//! pressed where the selection already has what it sends, each pop-up shows the row the
//! selection already is, and each well its colour — there is one fact about whether a run is
//! bold, and it is in the document.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use grind_core::DocumentKind;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSColor, NSColorWell, NSColorWellStyle, NSControl, NSImage, NSPopUpButton,
    NSSegmentSwitchTracking, NSSegmentedControl, NSToolbar, NSToolbarDelegate,
    NSToolbarDisplayMode, NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem, NSWindow,
    NSWindowToolbarStyle,
};
use objc2_foundation::{
    NSArray, NSCopying, NSInteger, NSObject, NSPoint, NSRect, NSSize, NSString,
};

use crate::formatting::Formats;
use crate::tools::{Tool, tools};

/// What the toolbar's delegate holds.
pub struct Bar {
    kind: DocumentKind,
    formats: Rc<dyn Formats>,
    /// Each control made, by its tool's place in `tools(kind)` — which is also its tag.
    controls: RefCell<HashMap<usize, Retained<NSControl>>>,
}

/// A colour a document stores, as the colour a well shows — `None` as `fallback`.
fn ns_color(value: Option<&str>, fallback: Retained<NSColor>) -> Retained<NSColor> {
    match value.and_then(grind_core::color::parse) {
        Some((r, g, b)) => NSColor::colorWithSRGBRed_green_blue_alpha(
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
            1.0,
        ),
        None => fallback,
    }
}

define_class!(
    /// The toolbar's delegate, and the target of every control on it.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindToolbar"]
    #[ivars = Bar]
    pub struct Toolbar;

    unsafe impl NSObjectProtocol for Toolbar {}

    unsafe impl NSToolbarDelegate for Toolbar {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for_identifier(
            &self,
            _toolbar: &NSToolbar,
            identifier: &NSString,
            _inserted: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            self.make(identifier)
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            self.identifiers(false)
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            self.identifiers(true)
        }
    }

    impl Toolbar {
        #[unsafe(method(segmentChosen:))]
        fn segment_chosen(&self, sender: &AnyObject) {
            self.chose_segment(sender);
        }

        #[unsafe(method(rowChosen:))]
        fn row_chosen(&self, sender: &AnyObject) {
            self.chose_row(sender);
        }

        #[unsafe(method(colorChosen:))]
        fn color_chosen(&self, sender: &AnyObject) {
            self.chose_color(sender);
        }
    }
);

impl Toolbar {
    fn tools(&self) -> &'static [Tool] {
        tools(self.ivars().kind)
    }

    /// Every item's identifier, and a flexible space when the customisation palette asks.
    fn identifiers(&self, allowed: bool) -> Retained<NSArray<NSString>> {
        let mut ids: Vec<Retained<NSString>> = self
            .tools()
            .iter()
            .map(|tool| NSString::from_str(tool.id()))
            .collect();
        if allowed {
            // SAFETY: a constant AppKit exports.
            ids.push(unsafe { NSToolbarFlexibleSpaceItemIdentifier }.copy());
        }
        NSArray::from_retained_slice(&ids)
    }

    /// The item for `identifier`, with its control made and remembered.
    fn make(&self, identifier: &NSString) -> Option<Retained<NSToolbarItem>> {
        let mtm = self.mtm();
        let id = identifier.to_string();
        let (index, tool) = self
            .tools()
            .iter()
            .enumerate()
            .find(|(_, tool)| tool.id() == id)?;
        let target: &AnyObject = self;
        let control: Retained<NSControl> = match tool {
            Tool::Segments {
                segments, toggles, ..
            } => {
                let images: Option<Vec<Retained<NSImage>>> = segments
                    .iter()
                    .map(|segment| {
                        NSImage::imageWithSystemSymbolName_accessibilityDescription(
                            &NSString::from_str(segment.symbol),
                            Some(&NSString::from_str(segment.label)),
                        )
                    })
                    .collect();
                let mode = match toggles {
                    true => NSSegmentSwitchTracking::SelectAny,
                    false => NSSegmentSwitchTracking::Momentary,
                };
                // SAFETY: the target is this object, which answers the action, and the
                // selector takes one sender.
                let control = unsafe {
                    match images {
                        Some(images) => {
                            NSSegmentedControl::segmentedControlWithImages_trackingMode_target_action(
                                &NSArray::from_retained_slice(&images),
                                mode,
                                Some(target),
                                Some(sel!(segmentChosen:)),
                                mtm,
                            )
                        }
                        // A symbol this system does not have: the segments say what they do
                        // in words instead.
                        None => {
                            let labels: Vec<Retained<NSString>> = segments
                                .iter()
                                .map(|segment| NSString::from_str(segment.label))
                                .collect();
                            NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(
                                &NSArray::from_retained_slice(&labels),
                                mode,
                                Some(target),
                                Some(sel!(segmentChosen:)),
                                mtm,
                            )
                        }
                    }
                };
                for (at, segment) in segments.iter().enumerate() {
                    control.setToolTip_forSegment(
                        Some(&NSString::from_str(segment.label)),
                        at as NSInteger,
                    );
                }
                control.into_super()
            }
            Tool::PopUp { rows, .. } => {
                let popup = NSPopUpButton::initWithFrame_pullsDown(
                    NSPopUpButton::alloc(mtm),
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(140.0, 24.0)),
                    false,
                );
                let titles: Vec<Retained<NSString>> = rows
                    .iter()
                    .map(|(title, _)| NSString::from_str(title))
                    .collect();
                popup.addItemsWithTitles(&NSArray::from_retained_slice(&titles));
                // SAFETY: as above.
                unsafe {
                    popup.setTarget(Some(target));
                    popup.setAction(Some(sel!(rowChosen:)));
                }
                popup.into_super().into_super()
            }
            Tool::Well { .. } => {
                let well = NSColorWell::colorWellWithStyle(NSColorWellStyle::Minimal, mtm);
                // SAFETY: as above.
                unsafe {
                    well.setTarget(Some(target));
                    well.setAction(Some(sel!(colorChosen:)));
                }
                well.into_super()
            }
        };
        control.setTag(index as NSInteger);
        self.ivars()
            .controls
            .borrow_mut()
            .insert(index, control.clone());
        let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), identifier);
        let label = NSString::from_str(tool.label());
        item.setLabel(&label);
        item.setPaletteLabel(&label);
        item.setToolTip(Some(&label));
        item.setView(Some(&control));
        self.refresh_one(index);
        Some(item)
    }

    /// The tool a control is, by its tag.
    fn tool_of(&self, sender: &AnyObject) -> Option<(usize, &'static Tool)> {
        let control = sender.downcast_ref::<NSControl>()?;
        let index = usize::try_from(control.tag()).ok()?;
        self.tools().get(index).map(|tool| (index, tool))
    }

    fn chose_segment(&self, sender: &AnyObject) {
        let (Some((_, tool)), Some(control)) = (
            self.tool_of(sender),
            sender.downcast_ref::<NSSegmentedControl>(),
        ) else {
            return;
        };
        let Tool::Segments {
            segments, toggles, ..
        } = tool
        else {
            return;
        };
        let formats = &self.ivars().formats;
        // A toggle's segment is the one whose state no longer says what the selection is — a
        // click flipped it; a momentary control's is simply the one pressed.
        let chosen = match toggles {
            true => segments.iter().enumerate().find(|(at, segment)| {
                control.isSelectedForSegment(*at as NSInteger) != formats.checked(segment.command)
            }),
            false => usize::try_from(control.selectedSegment())
                .ok()
                .and_then(|at| segments.get(at).map(|segment| (at, segment))),
        };
        if let Some((_, segment)) = chosen {
            formats.format(segment.command);
        }
        self.refresh();
    }

    fn chose_row(&self, sender: &AnyObject) {
        let (Some((_, Tool::PopUp { rows, .. })), Some(popup)) =
            (self.tool_of(sender), sender.downcast_ref::<NSPopUpButton>())
        else {
            return;
        };
        if let Some((_, command)) = usize::try_from(popup.indexOfSelectedItem())
            .ok()
            .and_then(|at| rows.get(at))
        {
            self.ivars().formats.format(*command);
        }
        self.refresh();
    }

    fn chose_color(&self, sender: &AnyObject) {
        let (Some((_, Tool::Well { background, .. })), Some(well)) =
            (self.tool_of(sender), sender.downcast_ref::<NSColorWell>())
        else {
            return;
        };
        self.ivars().formats.set_color(&well.color(), *background);
    }

    /// Every control shows what the selection is.
    pub fn refresh(&self) {
        for index in 0..self.tools().len() {
            self.refresh_one(index);
        }
    }

    fn refresh_one(&self, index: usize) {
        let Some(control) = self.ivars().controls.borrow().get(&index).cloned() else {
            return;
        };
        let formats = &self.ivars().formats;
        match self.tools().get(index) {
            Some(Tool::Segments {
                segments,
                toggles: true,
                ..
            }) => {
                if let Some(control) = control.downcast_ref::<NSSegmentedControl>() {
                    for (at, segment) in segments.iter().enumerate() {
                        control.setSelected_forSegment(
                            formats.checked(segment.command),
                            at as NSInteger,
                        );
                    }
                }
            }
            Some(Tool::PopUp { rows, .. }) => {
                if let Some(popup) = control.downcast_ref::<NSPopUpButton>() {
                    let at = rows
                        .iter()
                        .position(|(_, command)| formats.checked(*command))
                        .map_or(-1, |at| at as NSInteger);
                    popup.selectItemAtIndex(at);
                }
            }
            Some(Tool::Well { background, .. }) => {
                if let Some(well) = control.downcast_ref::<NSColorWell>() {
                    let (text, behind) = formats.colors();
                    let color = match background {
                        true => ns_color(behind.as_deref(), NSColor::clearColor()),
                        false => ns_color(text.as_deref(), NSColor::textColor()),
                    };
                    well.setColor(&color);
                }
            }
            _ => {}
        }
    }
}

/// Give `window` its toolbar for a document of `kind` over `formats`. The toolbar holds its
/// delegate weakly, so the caller keeps what this returns for as long as the window lives.
pub fn attach(
    window: &NSWindow,
    kind: DocumentKind,
    formats: Rc<dyn Formats>,
    mtm: MainThreadMarker,
) -> Retained<Toolbar> {
    let this = Toolbar::alloc(mtm).set_ivars(Bar {
        kind,
        formats: formats.clone(),
        controls: RefCell::new(HashMap::new()),
    });
    // SAFETY: `init` is `NSObject`'s designated initialiser.
    let delegate: Retained<Toolbar> = unsafe { msg_send![super(this), init] };
    let name = match kind {
        DocumentKind::Text => "GrindPage",
        _ => "GrindGrid",
    };
    let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str(name));
    toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
    toolbar.setAllowsUserCustomization(true);
    toolbar.setAutosavesConfiguration(true);
    window.setToolbar(Some(&toolbar));
    window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    let weak = Weak::from_retained(&delegate);
    formats.watch(Box::new(move || {
        if let Some(delegate) = weak.load() {
            delegate.refresh();
        }
    }));
    delegate.refresh();
    delegate
}
