// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A picture on the page: how big it is drawn, and how tall its block is — portable.
//!
//! `grind_text::flow::lay_out` asks a shell for a picture block's height through a hook, because
//! decoding needs a platform decoder; on a Mac that is `NSImage` (`render.rs`'s `ImageDecoder`), and
//! everything else is this file. The rule is `ui_text_gtk`'s, so a figure takes the same room in
//! both windows: **fit the column, keep the aspect ratio, never draw larger than the picture's
//! own size** — ODF's `svg:width`/`svg:height` are not used, for the same reason that window
//! gives — and a caption, when the block has one, under it after a small gap.

use grind_core::layout::{Fragment, Metrics, wrap};
use grind_core::style::TextStyle;
use grind_text::{BlockView, ImageView, picture_of};

/// The gap between a picture and its caption — small, since the two read as one figure.
pub const CAPTION_GAP: f64 = 4.0;

/// What a picture's bytes decode to: its natural size, in points.
pub trait Decoder {
    fn size(&self, image: &ImageView) -> Option<(f64, f64)>;
}

/// A decoder that decodes nothing — every picture stays an outline — for the tests of what
/// happens to bytes nothing can read.
#[cfg(test)]
pub struct Undecoded;

#[cfg(test)]
impl Decoder for Undecoded {
    fn size(&self, _: &ImageView) -> Option<(f64, f64)> {
        None
    }
}

/// How big a picture of natural size `size` is drawn in a column `width` wide.
pub fn fitted(size: (f64, f64), width: f64) -> (f64, f64) {
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    let drawn = w.min(width.max(1.0));
    (drawn, h * (drawn / w))
}

/// How tall a picture block is at `width` — the picture fitted, and its caption's height under
/// it, measured by `caption(text, width)` — or `None` when the block is not a picture or its
/// bytes will not decode, in which case the flow measures it as text (R5's tolerance).
pub fn height(
    block: &BlockView,
    width: f64,
    decoder: &dyn Decoder,
    caption: impl Fn(&str, f64) -> f64,
) -> Option<f64> {
    let (image, text) = picture_of(block)?;
    let picture = fitted(decoder.size(image)?, width).1;
    Some(match text {
        Some(text) => picture + CAPTION_GAP + caption(text, width),
        None => picture,
    })
}

/// A caption's lines at `width` in `metrics` — each line's text and its top from the caption's
/// own — through `grind_core::layout::wrap`, the breaker every block is laid out with, so the
/// room the flow reserves and the lines the paint draws are one answer.
pub fn caption_lines(text: &str, width: f64, metrics: &dyn Metrics) -> (Vec<(String, f64)>, f64) {
    let style = TextStyle::default();
    let layout = wrap(
        &[Fragment {
            text,
            style: &style,
        }],
        width as f32,
        metrics,
    );
    let lines = layout
        .lines()
        .iter()
        .map(|line| {
            let piece: String = text
                .chars()
                .skip(line.start)
                .take(line.end.saturating_sub(line.start))
                .collect();
            (piece.trim_end().to_owned(), f64::from(line.top))
        })
        .collect();
    (lines, f64::from(layout.height()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every picture is 400 by 200 points.
    pub struct Landscape;

    impl Decoder for Landscape {
        fn size(&self, _: &ImageView) -> Option<(f64, f64)> {
            Some((400.0, 200.0))
        }
    }

    #[test]
    fn a_picture_fits_the_column_and_never_grows() {
        assert_eq!(fitted((400.0, 200.0), 100.0), (100.0, 50.0));
        assert_eq!(
            fitted((400.0, 200.0), 1000.0),
            (400.0, 200.0),
            "its own size at most"
        );
    }

    #[test]
    fn a_figure_is_its_picture_and_its_caption() {
        let app = grind_text::App::new();
        app.insert_image(
            grind_text::Caret {
                block: 0,
                offset: 0,
            },
            "image/png".into(),
            vec![0x89, b'P', b'N', b'G'],
            None,
            None,
        )
        .unwrap();
        let view = app.get_viewport(0..app.block_count());
        let picture = (0..app.block_count())
            .filter_map(|i| view.get(i))
            .find(|block| picture_of(block).is_some())
            .expect("a picture block");
        let tall = height(picture, 100.0, &Landscape, |_, _| 12.0).unwrap();
        let captioned = picture_of(picture).unwrap().1.is_some();
        assert_eq!(
            tall,
            50.0 + if captioned { CAPTION_GAP + 12.0 } else { 0.0 }
        );
        assert_eq!(height(picture, 100.0, &Undecoded, |_, _| 12.0), None);
    }
}
