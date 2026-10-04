// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The preview: a page of the display list rasterised (`doc/pdf-export.md` §4).
//!
//! **Never re-laid-out with a shell's own fonts.** The preview exists to show what will print,
//! so it draws the very glyphs the PDF embeds, from the same faces, at the same positions —
//! outlines from skrifa, filled by tiny-skia. A shell shows the RGBA this returns and owns
//! nothing else about it. A test holds it against `hayro`'s raster of our own PDF, so a
//! backend that drifted from the other would be caught rather than trusted.

use std::collections::HashMap;

use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

use crate::fonts::{FaceId, Fonts};
use crate::ops::{Op, Page, Rgb};

/// A page as pixels: RGBA, eight bits a channel, **premultiplied**, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Raster {
    /// The pixel at `(x, y)` as `[r, g, b, a]`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[at],
            self.rgba[at + 1],
            self.rgba[at + 2],
            self.rgba[at + 3],
        ]
    }

    /// The raster as a PNG file.
    pub fn png(&self) -> Vec<u8> {
        Pixmap::from_vec(
            self.rgba.clone(),
            tiny_skia::IntSize::from_wh(self.width.max(1), self.height.max(1)).expect("non-zero"),
        )
        .and_then(|pixmap| pixmap.encode_png().ok())
        .unwrap_or_default()
    }
}

/// Rasterise `page` at `scale` pixels per point (1.0 is 72 dpi; a screen at 100% wants 96/72).
pub fn render(page: &Page, fonts: &Fonts, scale: f32) -> Raster {
    // Whole pixels, rounding down, as a PDF renderer sizes the same page.
    let width = ((page.width * scale) as u32).max(1);
    let height = ((page.height * scale) as u32).max(1);
    let Some(mut pixmap) = Pixmap::new(width, height) else {
        return Raster {
            width,
            height,
            rgba: vec![255; (width * height * 4) as usize],
        };
    };
    pixmap.fill(tiny_skia::Color::WHITE);
    let scaled = Transform::from_scale(scale, scale);
    let mut glyphs = Glyphs::default();
    for op in &page.ops {
        draw(&mut pixmap, op, fonts, scaled, &mut glyphs);
    }
    Raster {
        width,
        height,
        rgba: pixmap.take(),
    }
}

fn paint(color: Rgb) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.0, color.1, color.2, 255);
    paint.anti_alias = true;
    paint
}

fn draw(pixmap: &mut Pixmap, op: &Op, fonts: &Fonts, scaled: Transform, glyphs: &mut Glyphs) {
    match op {
        Op::Text {
            x,
            y,
            face,
            size,
            glyphs: run,
            color,
            ..
        } => {
            let paint = paint(*color);
            let mut pen = *x;
            // Outlines are in font units, y up; the page is in points, y down.
            let em = *size / glyphs.units_per_em(fonts, *face);
            for glyph in run {
                if let Some(path) = glyphs.outline(fonts, *face, glyph.id) {
                    let at = Transform::from_row(
                        em,
                        0.0,
                        0.0,
                        -em,
                        pen + glyph.x_offset,
                        *y - glyph.y_offset,
                    );
                    pixmap.fill_path(
                        path,
                        &paint,
                        FillRule::Winding,
                        at.post_concat(scaled),
                        None,
                    );
                }
                pen += glyph.x_advance;
            }
        }
        Op::Rect {
            x,
            y,
            width,
            height,
            color,
        } => {
            if let Some(rect) = tiny_skia::Rect::from_xywh(*x, *y, *width, *height) {
                pixmap.fill_rect(rect, &paint(*color), scaled, None);
            }
        }
        Op::Line {
            from,
            to,
            width,
            color,
        } => {
            let mut path = PathBuilder::new();
            path.move_to(from.0, from.1);
            path.line_to(to.0, to.1);
            if let Some(path) = path.finish() {
                let stroke = Stroke {
                    width: *width,
                    ..Stroke::default()
                };
                pixmap.stroke_path(&path, &paint(*color), &stroke, scaled, None);
            }
        }
        Op::Image {
            x,
            y,
            width,
            height,
            mime,
            data,
            ..
        } => {
            if mime == "image/svg+xml"
                && let Some(tree) = crate::svg::tree(data, fonts)
            {
                let size = tree.size();
                let at = Transform::from_row(
                    *width / size.width(),
                    0.0,
                    0.0,
                    *height / size.height(),
                    *x,
                    *y,
                )
                .post_concat(scaled);
                resvg::render(&tree, at, &mut pixmap.as_mut());
                return;
            }
            let decoded = match mime.as_str() {
                "image/png" => Pixmap::decode_png(data).ok(),
                _ => None,
            };
            match decoded {
                Some(image) => {
                    let sx = *width / image.width() as f32;
                    let sy = *height / image.height() as f32;
                    let at = Transform::from_row(sx, 0.0, 0.0, sy, *x, *y).post_concat(scaled);
                    pixmap.draw_pixmap(0, 0, image.as_ref(), &PixmapPaint::default(), at, None);
                }
                // ponytail: a JPEG previews as its box, in the grey a viewer uses for a picture
                // still loading; the PDF itself carries it. A decoder is `zune-jpeg`, already in
                // the tree under krilla, when somebody previews one.
                None => {
                    if let Some(rect) = tiny_skia::Rect::from_xywh(*x, *y, *width, *height) {
                        pixmap.fill_rect(rect, &paint(Rgb(0xdd, 0xdd, 0xdd)), scaled, None);
                    }
                }
            }
        }
    }
}

/// Glyph outlines, each built once per page rather than once per occurrence.
#[derive(Default)]
struct Glyphs {
    paths: HashMap<(FaceId, u32), Option<tiny_skia::Path>>,
    units: HashMap<FaceId, f32>,
}

impl Glyphs {
    fn units_per_em(&mut self, fonts: &Fonts, face: FaceId) -> f32 {
        *self.units.entry(face).or_insert_with(|| {
            let face = fonts.face(face);
            skrifa::FontRef::from_index(face.bytes(), face.index)
                .ok()
                .map_or(1000.0, |font| {
                    f32::from(font.head().map_or(1000, |h| h.units_per_em()))
                })
        })
    }

    fn outline(&mut self, fonts: &Fonts, face: FaceId, glyph: u32) -> Option<&tiny_skia::Path> {
        self.paths
            .entry((face, glyph))
            .or_insert_with(|| {
                let face = fonts.face(face);
                let font = skrifa::FontRef::from_index(face.bytes(), face.index).ok()?;
                let outlines = font.outline_glyphs();
                let outline = outlines.get(skrifa::GlyphId::new(glyph))?;
                let mut pen = Pen(PathBuilder::new());
                outline
                    .draw(
                        DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                        &mut pen,
                    )
                    .ok()?;
                pen.0.finish()
            })
            .as_ref()
    }
}

/// skrifa's outline, in font units, into tiny-skia's path.
struct Pen(PathBuilder);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y)
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y)
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.quad_to(cx, cy, x, y)
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.0.cubic_to(a, b, c, d, x, y)
    }
    fn close(&mut self) {
        self.0.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Typesetter;
    use crate::ops::{Op, Rgb};
    use crate::text::{Options, typeset};
    use grind_text::{App, BlockKind};

    fn blank() -> Page {
        Page {
            width: 100.0,
            height: 50.0,
            ops: Vec::new(),
            start: None,
            end: None,
        }
    }

    fn dark(raster: &Raster, x: u32, y: u32) -> bool {
        raster.pixel(x, y)[0] < 128
    }

    #[test]
    fn a_page_is_white_paper_at_its_size_times_the_scale() {
        let raster = render(&blank(), &Fonts::bundled(), 2.0);
        assert_eq!((raster.width, raster.height), (200, 100));
        assert_eq!(raster.rgba.len(), 200 * 100 * 4);
        assert!(raster.rgba.chunks(4).all(|p| p == [255, 255, 255, 255]));
    }

    #[test]
    fn a_rect_and_a_line_are_drawn_where_they_are() {
        let mut page = blank();
        page.ops.push(Op::Rect {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 10.0,
            color: Rgb(255, 0, 0),
        });
        page.ops.push(Op::Line {
            from: (50.0, 25.0),
            to: (90.0, 25.0),
            width: 2.0,
            color: Rgb::BLACK,
        });
        let raster = render(&page, &Fonts::bundled(), 1.0);
        assert_eq!(raster.pixel(20, 15), [255, 0, 0, 255]);
        assert_eq!(raster.pixel(5, 5), [255, 255, 255, 255]);
        assert!(dark(&raster, 70, 25));
        assert!(!dark(&raster, 70, 35));
    }

    #[test]
    fn text_is_inked_on_its_baseline_and_nowhere_else() {
        let setter = Typesetter::new(Fonts::bundled());
        let style = grind_core::style::TextStyle::default();
        let shaped = setter.shape("MMMM", &style);
        let mut page = blank();
        page.ops.push(Op::Text {
            x: 10.0,
            y: 30.0,
            face: shaped.face,
            size: shaped.size,
            glyphs: shaped.glyphs,
            text: "MMMM".into(),
            color: Rgb::BLACK,
            mark: crate::ops::Mark::Decoration,
        });
        let raster = render(&page, setter.fonts(), 1.0);
        let inked = |x0: u32, y0: u32, x1: u32, y1: u32| {
            (y0..y1)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|&(x, y)| dark(&raster, x, y))
                .count()
        };
        assert!(
            inked(10, 20, 50, 31) > 40,
            "the capitals between cap height and baseline"
        );
        assert_eq!(inked(0, 0, 100, 18), 0, "nothing above them");
        assert_eq!(inked(60, 0, 100, 50), 0, "nothing after them");
    }

    #[test]
    fn the_same_page_rasterises_to_the_same_pixels() {
        let app = App::new();
        app.insert(0, BlockKind::Heading { level: 1 }, "Preview")
            .unwrap();
        let setter = Typesetter::new(Fonts::bundled());
        let doc = typeset(&app, &setter, &Options::default());
        let a = render(&doc.pages[0], setter.fonts(), 1.0);
        assert_eq!(a, render(&doc.pages[0], setter.fonts(), 1.0));
        assert!(a.png().starts_with(b"\x89PNG"));
    }

    /// A red square in an SVG, drawn into a box: the box is red in the preview and in the PDF a
    /// renderer reads back, and outside it the page is white.
    #[test]
    fn an_svg_picture_is_drawn_in_its_box_in_the_preview_and_the_pdf() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#ff0000"/></svg>"##;
        let mut page = blank();
        page.ops.push(Op::Image {
            x: 20.0,
            y: 10.0,
            width: 30.0,
            height: 30.0,
            mime: "image/svg+xml".into(),
            data: std::sync::Arc::new(svg.to_vec()),
            mark: crate::ops::Mark::Decoration,
        });
        let fonts = Fonts::bundled();
        let raster = render(&page, &fonts, 1.0);
        assert_eq!(raster.pixel(35, 25), [255, 0, 0, 255]);
        assert_eq!(raster.pixel(5, 5), [255, 255, 255, 255]);

        let doc = crate::ops::Document {
            pages: vec![page],
            outline: vec![],
            structure: vec![],
        };
        let bytes = crate::pdf::write(&doc, &fonts, &crate::pdf::Metadata::default()).unwrap();
        let pdf = hayro::hayro_syntax::Pdf::new(bytes).unwrap();
        let theirs = hayro::render(
            &pdf.pages()[0],
            &hayro::RenderCache::new(),
            &hayro::hayro_interpret::InterpreterSettings::default(),
            &hayro::RenderSettings::default(),
            &hayro::PixmapSettings {
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                ..hayro::PixmapSettings::default()
            },
        );
        let at = |x: usize, y: usize| {
            let i = (y * theirs.width() as usize + x) * 4;
            theirs.data_as_u8_slice()[i..i + 3].to_vec()
        };
        assert_eq!(at(35, 25), vec![255, 0, 0]);
        assert_eq!(at(5, 5), vec![255, 255, 255]);
    }

    /// The preview is the PDF: our raster of a page against `hayro`'s raster of the PDF we
    /// wrote of it. Antialiasing differs between any two rasterisers, so the claim is that
    /// almost every pixel agrees closely, not that every one does.
    #[test]
    fn the_preview_agrees_with_a_pdf_renderer_reading_our_pdf() {
        let app = App::new();
        app.insert(0, BlockKind::Heading { level: 1 }, "Grind on paper")
            .unwrap();
        app.insert(
            1,
            BlockKind::Paragraph,
            &"Office affine fjord — ÄÖÜ. ".repeat(30),
        )
        .unwrap();
        app.insert(2, BlockKind::ListItem { depth: 1 }, "an item")
            .unwrap();
        let setter = Typesetter::new(Fonts::bundled());
        let doc = typeset(&app, &setter, &Options::default());
        let ours = render(&doc.pages[0], setter.fonts(), 1.0);
        let bytes =
            crate::pdf::write(&doc, setter.fonts(), &crate::pdf::Metadata::default()).unwrap();

        let pdf = hayro::hayro_syntax::Pdf::new(bytes).unwrap();
        let page = &pdf.pages()[0];
        let theirs = hayro::render(
            page,
            &hayro::RenderCache::new(),
            &hayro::hayro_interpret::InterpreterSettings::default(),
            &hayro::RenderSettings::default(),
            &hayro::PixmapSettings {
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                ..hayro::PixmapSettings::default()
            },
        );
        assert_eq!(
            (u32::from(theirs.width()), u32::from(theirs.height())),
            (ours.width, ours.height)
        );
        let theirs = theirs.data_as_u8_slice();
        let pixels = (ours.width * ours.height) as usize;
        let apart = (0..pixels)
            .filter(|&i| (0..3).any(|c| ours.rgba[i * 4 + c].abs_diff(theirs[i * 4 + c]) > 96))
            .count();
        let inked = (0..pixels).filter(|&i| ours.rgba[i * 4] < 128).count();
        assert!(inked > 2000, "there is text to compare: {inked}");
        // Measured at none apart on the day this was written; one in a hundred leaves room for
        // either rasteriser's antialiasing to change, and none for a glyph in the wrong place.
        assert!(apart * 100 < inked, "{apart} pixels apart of {inked} inked");
    }
}
