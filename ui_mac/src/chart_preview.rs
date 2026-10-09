// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Insert ▸ Chart Preview… — the chart a selection would make, shown before it is made.
//!
//! An `NSAlert` with the picture as its accessory view — under a segmented control of the three
//! kinds, the GNOME dialog's buttons, which swaps the picture — and **Insert** / **Cancel** as its buttons,
//! the platform's own shape for a question with a picture in it (`prompt.rs` is the one-line
//! sibling). The picture is not drawn by a view of this shell's own: `sheet/chart.rs`'s `Op`s —
//! the very list the grid draws a chart from — are rendered headless to a bitmap by `render`, as
//! `--render-to` does, and handed to an `NSImageView`. So what is previewed is what the grid then
//! draws, by construction, and no second chart painter exists.

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSImage, NSImageView, NSSegmentSwitchTracking,
    NSSegmentedControl, NSView,
};
use objc2_foundation::{NSArray, NSData, NSPoint, NSRect, NSSize, NSString};

use crate::sheet::geom::Rect;
use crate::{metrics, png, render};

/// The picture's size in points, and how many device pixels a point is drawn at.
const WIDTH: f64 = 420.0;
const HEIGHT: f64 = 260.0;
const SCALE: f64 = 2.0;
/// The room over the picture the kind control stands in.
const KINDS_ROW: f64 = 32.0;

define_class!(
    /// The kind control's target: the chart drawn as each kind, and the view showing one of them.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GrindChartPreviewKinds"]
    #[ivars = (Vec<Retained<NSImage>>, Retained<NSImageView>)]
    struct Kinds;

    impl Kinds {
        #[unsafe(method(kindChosen:))]
        fn chosen(&self, sender: &NSSegmentedControl) {
            let (images, view) = self.ivars();
            if let Some(image) = usize::try_from(sender.selectedSegment())
                .ok()
                .and_then(|at| images.get(at))
            {
                view.setImage(Some(image));
            }
        }
    }
);

/// Show the chart a selection would make as each of `pictures`' kinds — a segmented control of
/// the GNOME dialog's three kinds over the picture, `shown` chosen first — and ask whether to
/// insert it: the kind chosen for Insert, `None` for Cancel. A picture that cannot be made is a
/// reason to say so, not to insert blind, so that is `None` too.
pub fn choose(
    mtm: MainThreadMarker,
    pictures: &[(
        grind_sheet::ChartKind,
        grind_sheet::Chart,
        grind_sheet::ChartData,
    )],
    shown: usize,
) -> Option<grind_sheet::ChartKind> {
    let images: Option<Vec<Retained<NSImage>>> = pictures
        .iter()
        .map(|(_, chart, data)| picture(chart, data))
        .collect();
    let Some(images) = images.filter(|images| !images.is_empty()) else {
        crate::prompt::tell(
            mtm,
            "The preview could not be drawn",
            "Nothing was inserted.",
        );
        return None;
    };
    let shown = shown.min(images.len() - 1);
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Insert this chart?"));
    alert.setInformativeText(&NSString::from_str(
        "It goes beside the table, and ⌘Z takes it back.",
    ));
    alert.addButtonWithTitle(&NSString::from_str("Insert"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let view = NSImageView::imageViewWithImage(&images[shown], mtm);
    view.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(WIDTH, HEIGHT),
    ));
    let target: Retained<Kinds> = {
        let this = Kinds::alloc(mtm).set_ivars((images, view.clone()));
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    };
    let labels: Vec<Retained<NSString>> = pictures
        .iter()
        .map(|(kind, _, _)| NSString::from_str(kind.name()))
        .collect();
    // SAFETY: the target answers the action, which takes one sender, and outlives the alert.
    let kinds = unsafe {
        NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(
            &NSArray::from_retained_slice(&labels),
            NSSegmentSwitchTracking::SelectOne,
            Some(&target),
            Some(sel!(kindChosen:)),
            mtm,
        )
    };
    kinds.setSelectedSegment(shown as isize);
    kinds.sizeToFit();
    let size = kinds.frame().size;
    // The accessory is not flipped: the picture at the bottom, the kinds centred over it.
    kinds.setFrameOrigin(NSPoint::new(
        ((WIDTH - size.width) / 2.0).max(0.0),
        HEIGHT + (KINDS_ROW - size.height) / 2.0,
    ));
    let accessory = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(WIDTH, HEIGHT + KINDS_ROW),
        ),
    );
    accessory.addSubview(&view);
    accessory.addSubview(&kinds);
    alert.setAccessoryView(Some(&accessory));
    let insert = alert.runModal() == NSAlertFirstButtonReturn;
    let chosen = usize::try_from(kinds.selectedSegment()).unwrap_or(shown);
    drop(target);
    insert
        .then(|| pictures.get(chosen).map(|(kind, _, _)| *kind))
        .flatten()
}

/// The chart as an image, in the appearance the shell is drawing in.
fn picture(chart: &grind_sheet::Chart, data: &grind_sheet::ChartData) -> Option<Retained<NSImage>> {
    let text = metrics::CoreText::new(metrics::BASE_PT);
    let palette = crate::grid_view::palette();
    let ops = crate::sheet::chart::draw(
        chart,
        data,
        Rect::new(0.0, 0.0, WIDTH, HEIGHT),
        &palette,
        &text,
    );
    let (w, h, rgba) = render::bitmap(WIDTH, HEIGHT, SCALE, |context| {
        render::draw(context, &ops, &text)
    })
    .ok()?;
    let image = NSImage::initWithData(
        NSImage::alloc(),
        &NSData::with_bytes(&png::encode(w, h, &rgba)),
    )?;
    // The bitmap is two device pixels a point; saying so keeps it sharp on a Retina screen.
    image.setSize(NSSize::new(WIDTH, HEIGHT));
    Some(image)
}
