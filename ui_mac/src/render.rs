// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A frame's [`Op`]s, put down on a `CGContext` — the whole of this shell's drawing.
//!
//! **One path, two callers** (decision 9): a view's `drawRect:` hands [`draw`] the context AppKit
//! gave it, and `--render-to` hands it a bitmap from [`bitmap`] with no application and no window
//! server. Nothing is decided here that `sheet/paint.rs` or `text/paint.rs` did not already
//! decide; this file turns a rectangle into `CGContextFillRect` and a line of text into the
//! `CTLine` [`crate::metrics::CoreText`] measured it with — and an underline or a strike into a
//! rectangle placed by that font's own metrics.
//!
//! Both callers draw in a **flipped** space — origin top left, y down — which is what a flipped
//! `NSView` gets for free and what [`bitmap`] sets up. CoreText sets glyphs upright in an
//! unflipped space, so the text matrix is flipped back once per frame.

use grind_core::color::Rgb;
use objc2_core_foundation::{CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextGetBytesPerRow, CGBitmapContextGetData, CGColorSpace,
    CGContext, CGImageAlphaInfo, kCGColorSpaceSRGB,
};

use std::ptr::null_mut;

use grind_text::ImageView;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_app_kit::NSImage;
use objc2_foundation::NSData;

use crate::metrics::CoreText;
use crate::ops::{Op, WASH};
use crate::sheet::geom::Rect;
use crate::text::picture::Decoder;

/// A picture's bytes as an `NSImage` — every format the system reads — or `None` when nothing
/// here can read them.
///
/// ponytail: decoded on every paint rather than cached, as `ui_text_gtk` does; the trigger is a
/// document whose pictures make scrolling visibly slow, and the cache is keyed by the bytes.
fn decode(data: &[u8]) -> Option<Retained<NSImage>> {
    NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(data))
}

/// What decodes a page's pictures for the layout — their natural size, in points, which is
/// `NSImage`'s own reading of a picture's pixels and resolution.
pub struct ImageDecoder;

impl Decoder for ImageDecoder {
    fn size(&self, image: &ImageView) -> Option<(f64, f64)> {
        let size = decode(&image.data)?.size();
        (size.width > 0.0 && size.height > 0.0).then_some((size.width, size.height))
    }
}

fn cg_rect(rect: &Rect) -> CGRect {
    CGRect::new(CGPoint::new(rect.x, rect.y), CGSize::new(rect.w, rect.h))
}

fn fill_color(context: &CGContext, (r, g, b): Rgb, alpha: CGFloat) {
    let channel = |value: u8| CGFloat::from(value) / 255.0;
    CGContext::set_rgb_fill_color(Some(context), channel(r), channel(g), channel(b), alpha);
}

/// Draw `ops`, in order, into `context`, which is flipped — origin top left.
pub fn draw(context: &CGContext, ops: &[Op], text: &CoreText) {
    let context_ref = Some(context);
    // Glyphs are drawn upright in an unflipped space; in this flipped one they would come out
    // upside down without this.
    CGContext::set_text_matrix(
        context_ref,
        CGAffineTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            tx: 0.0,
            ty: 0.0,
        },
    );
    for op in ops {
        match op {
            Op::Image { rect, data } => {
                let Some(image) = decode(data).and_then(|image| {
                    // SAFETY: a null proposed rectangle and no context or hints ask for the
                    // image at its own size, all documented as allowed.
                    unsafe { image.CGImageForProposedRect_context_hints(null_mut(), None, None) }
                }) else {
                    continue;
                };
                // A CGImage is drawn the right way up in an unflipped space; this one is
                // flipped, so the picture's own rectangle is flipped back around its middle.
                CGContext::save_g_state(context_ref);
                CGContext::translate_ctm(context_ref, rect.x, rect.y + rect.h);
                CGContext::scale_ctm(context_ref, 1.0, -1.0);
                CGContext::draw_image(
                    context_ref,
                    cg_rect(&Rect::new(0.0, 0.0, rect.w, rect.h)),
                    Some(&image),
                );
                CGContext::restore_g_state(context_ref);
            }
            Op::Fill { rect, color } => {
                fill_color(context, *color, 1.0);
                CGContext::fill_rect(context_ref, cg_rect(rect));
            }
            Op::Wash { rect, color } => {
                fill_color(context, *color, WASH);
                CGContext::fill_rect(context_ref, cg_rect(rect));
            }
            Op::Text {
                x,
                top,
                text: string,
                style,
                color,
                clip,
            } => {
                CGContext::save_g_state(context_ref);
                CGContext::clip_to_rect(context_ref, cg_rect(clip));
                // The line takes its colour from the context
                // (`kCTForegroundColorFromContextAttributeName`), so one cached font serves every
                // colour a document uses.
                fill_color(context, *color, 1.0);
                let line = text.line(string, style);
                CGContext::set_text_position(context_ref, *x, *top + text.ascent(style));
                // SAFETY: the context is live for the whole of this call.
                unsafe { line.draw(context) };
                CGContext::restore_g_state(context_ref);
            }
            Op::Run {
                x,
                top,
                text: string,
                font,
                color,
                underline,
                strike,
                clip,
            } => {
                CGContext::save_g_state(context_ref);
                CGContext::clip_to_rect(context_ref, cg_rect(clip));
                fill_color(context, *color, 1.0);
                let line = text.line_of(string, font);
                let baseline = *top + text.ascent_of(font);
                CGContext::set_text_position(context_ref, *x, baseline);
                // SAFETY: the context is live for the whole of this call.
                unsafe { line.draw(context) };
                if *underline || *strike {
                    // SAFETY: null out-pointers are documented as "not wanted".
                    let width = unsafe {
                        line.typographic_bounds(
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                        )
                    };
                    let (below, thickness) = text.underline_of(font);
                    if *underline {
                        let y = baseline + below - thickness / 2.0;
                        CGContext::fill_rect(
                            context_ref,
                            cg_rect(&Rect::new(*x, y, width, thickness)),
                        );
                    }
                    if *strike {
                        // Through the middle of a lower-case letter.
                        let y = baseline - text.x_height_of(font) / 2.0 - thickness / 2.0;
                        CGContext::fill_rect(
                            context_ref,
                            cg_rect(&Rect::new(*x, y, width, thickness)),
                        );
                    }
                }
                CGContext::restore_g_state(context_ref);
            }
        }
    }
}

/// A frame `width` by `height` points drawn at `scale` device pixels a point, as RGBA bytes — top
/// row first, `width * scale` pixels a row — ready for [`crate::png::encode`].
///
/// With no application object and no window: a `CGBitmapContext` in sRGB, which the probe showed
/// CoreGraphics and CoreText draw into headless (`doc/macos-shell.md`, *Evidence*). The bitmap is
/// flipped and scaled before `paint` runs, so `paint` is the same function a view calls.
pub fn bitmap(
    width: f64,
    height: f64,
    scale: f64,
    paint: impl FnOnce(&CGContext),
) -> Result<(u32, u32, Vec<u8>), String> {
    let (w, h) = (
        (width * scale).round() as usize,
        (height * scale).round() as usize,
    );
    // SAFETY: the name is one of CoreGraphics' own exported constants.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .ok_or("no sRGB colour space")?;
    // SAFETY: a null data pointer asks CoreGraphics to allocate and own the pixels, and a row
    // stride of zero asks it to choose; both are documented.
    let context = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            w,
            h,
            8,
            0,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }
    .ok_or("CGBitmapContextCreate returned nothing")?;
    let bitmap: &CGContext = &context;
    let context_ref = Some(bitmap);

    // Flipped, then scaled: y down from the top left, one unit a point.
    CGContext::translate_ctm(context_ref, 0.0, h as CGFloat);
    CGContext::scale_ctm(context_ref, scale, -scale);
    paint(bitmap);

    let stride = CGBitmapContextGetBytesPerRow(context_ref);
    let data = CGBitmapContextGetData(context_ref) as *const u8;
    if data.is_null() {
        return Err("the bitmap context has no pixels".into());
    }
    // SAFETY: the context owns `stride * h` bytes at `data` and outlives this read. Its rows are
    // stored top first, since a bitmap context's memory is laid out the way an image's is.
    let pixels = unsafe { std::slice::from_raw_parts(data, stride * h) };
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in pixels.chunks_exact(stride) {
        // Premultiplied RGBA: every frame fills its whole rectangle first, so alpha is 255 and
        // premultiplied is straight.
        rgba.extend_from_slice(&row[..w * 4]);
    }
    Ok((w as u32, h as u32, rgba))
}
