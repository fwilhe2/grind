// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Insert ▸ Chart Preview… — the chart a selection would make, shown before it is made.
//!
//! An `NSAlert` with the picture as its accessory view and **Insert** / **Cancel** as its buttons,
//! the platform's own shape for a question with a picture in it (`prompt.rs` is the one-line
//! sibling). The picture is not drawn by a view of this shell's own: `sheet/chart.rs`'s `Op`s —
//! the very list the grid draws a chart from — are rendered headless to a bitmap by `render`, as
//! `--render-to` does, and handed to an `NSImageView`. So what is previewed is what the grid then
//! draws, by construction, and no second chart painter exists.

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSImage, NSImageView};
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize, NSString};

use crate::sheet::geom::Rect;
use crate::{metrics, png, render};

/// The picture's size in points, and how many device pixels a point is drawn at.
const WIDTH: f64 = 420.0;
const HEIGHT: f64 = 260.0;
const SCALE: f64 = 2.0;

/// Show `chart` and ask whether to insert it: `true` for Insert, `false` for Cancel. A picture
/// that cannot be made is a reason to say so, not to insert blind, so that is `false` too.
pub fn confirm(
    mtm: MainThreadMarker,
    chart: &grind_sheet::Chart,
    data: &grind_sheet::ChartData,
) -> bool {
    let Some(image) = picture(chart, data) else {
        crate::prompt::tell(
            mtm,
            "The preview could not be drawn",
            "Nothing was inserted.",
        );
        return false;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Insert this chart?"));
    alert.setInformativeText(&NSString::from_str(
        "It goes beside the table, and ⌘Z takes it back.",
    ));
    alert.addButtonWithTitle(&NSString::from_str("Insert"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let view = NSImageView::imageViewWithImage(&image, mtm);
    view.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(WIDTH, HEIGHT),
    ));
    alert.setAccessoryView(Some(&view));
    alert.runModal() == NSAlertFirstButtonReturn
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
