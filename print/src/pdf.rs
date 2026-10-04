// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The display list, written as a PDF by krilla (`doc/pdf-export.md` §2).
//!
//! **The only file that knows the PDF library**, as `ui_win32/src/gdi.rs` is the only one that
//! creates a GDI object: an upgrade of krilla — pre-1.0, pinned exactly — or a different backend
//! one day touches this file and nothing else.
//!
//! What krilla does that this file therefore does not: subsetting each face to the glyphs used,
//! the CID font and its `ToUnicode` map (so text copies and searches as text), and refusing to
//! write a file that breaks a standard it was asked to meet.

use std::collections::HashMap;

use krilla::Document as Pdf;
use krilla::color::rgb;
use krilla::destination::XyzDestination;
use krilla::geom::{PathBuilder, Point, Size, Transform};
use krilla::image::Image;
use krilla::outline::{Outline, OutlineNode};
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};

use crate::fonts::{self, FaceId, Fonts};
use crate::ops::{Document, Heading, Op, Rgb};

/// What goes in the PDF's own metadata.
#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub title: Option<String>,
}

/// Write `doc` as a PDF, with its glyphs taken from `fonts` — the same database the document
/// was typeset in, since a [`crate::fonts::FaceId`] means nothing in any other.
pub fn write(doc: &Document, fonts: &Fonts, metadata: &Metadata) -> Result<Vec<u8>, String> {
    let mut pdf = Pdf::new();
    let mut faces: HashMap<FaceId, Font> = HashMap::new();
    for page in &doc.pages {
        let settings = PageSettings::from_wh(page.width, page.height)
            .ok_or_else(|| format!("a page cannot be {} × {} pt", page.width, page.height))?;
        let mut sheet = pdf.start_page_with(settings);
        let mut surface = sheet.surface();
        for op in &page.ops {
            draw(&mut surface, op, fonts, &mut faces);
        }
        surface.finish();
        sheet.finish();
    }
    pdf.set_outline(outline(&doc.outline));
    let mut meta = krilla::metadata::Metadata::new().producer("grind".to_owned());
    if let Some(title) = &metadata.title {
        meta = meta.title(title.clone());
    }
    pdf.set_metadata(meta);
    pdf.finish().map_err(|error| format!("{error:?}"))
}

fn paint(color: Rgb) -> krilla::paint::Paint {
    rgb::Color::new(color.0, color.1, color.2).into()
}

fn draw(surface: &mut Surface<'_>, op: &Op, fonts: &Fonts, faces: &mut HashMap<FaceId, Font>) {
    match op {
        Op::Text {
            x,
            y,
            face,
            size,
            glyphs,
            text,
            color,
        } => {
            let Some(font) = font(fonts, *face, faces) else {
                return;
            };
            // krilla takes advances in ems and scales them by the size itself.
            let em = size.max(f32::MIN_POSITIVE);
            let glyphs: Vec<KrillaGlyph> = glyphs
                .iter()
                .map(|glyph| {
                    KrillaGlyph::new(
                        GlyphId::new(glyph.id),
                        glyph.x_advance / em,
                        glyph.x_offset / em,
                        -glyph.y_offset / em,
                        0.0,
                        glyph.text.clone(),
                        None,
                    )
                })
                .collect();
            surface.set_fill(Some(Fill {
                paint: paint(*color),
                ..Fill::default()
            }));
            surface.draw_glyphs(Point::from_xy(*x, *y), &glyphs, font, text, *size, false);
        }
        Op::Rect {
            x,
            y,
            width,
            height,
            color,
        } => {
            let mut path = PathBuilder::new();
            if let Some(rect) = krilla::geom::Rect::from_xywh(*x, *y, *width, *height) {
                path.push_rect(rect);
            }
            if let Some(path) = path.finish() {
                surface.set_stroke(None);
                surface.set_fill(Some(Fill {
                    paint: paint(*color),
                    ..Fill::default()
                }));
                surface.draw_path(&path);
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
                surface.set_fill(None);
                surface.set_stroke(Some(Stroke {
                    paint: paint(*color),
                    width: *width,
                    ..Stroke::default()
                }));
                surface.draw_path(&path);
                surface.set_stroke(None);
            }
        }
        Op::Image {
            x,
            y,
            width,
            height,
            mime,
            data,
        } => {
            let data: krilla::Data = data.clone().into();
            let image = match mime.as_str() {
                "image/png" => Image::from_png(data, true),
                "image/jpeg" => Image::from_jpeg(data, true),
                _ => return,
            };
            let (Ok(image), Some(size)) = (image, Size::from_wh(*width, *height)) else {
                return;
            };
            surface.push_transform(&Transform::from_translate(*x, *y));
            surface.draw_image(image, size);
            surface.pop();
        }
    }
}

/// The krilla font for a face, made once per face per document.
fn font(fonts: &Fonts, id: FaceId, faces: &mut HashMap<FaceId, Font>) -> Option<Font> {
    if let Some(font) = faces.get(&id) {
        return Some(font.clone());
    }
    let face = fonts.face(id);
    let data: krilla::Data = match &face.data {
        fonts::Data::Static(bytes) => (*bytes).into(),
        fonts::Data::Shared(bytes) => bytes.clone().into(),
    };
    let font = Font::new(data, face.index)?;
    faces.insert(id, font.clone());
    Some(font)
}

/// The headings as a tree: each one a child of the nearest shallower heading before it.
fn outline(headings: &[Heading]) -> Outline {
    fn node(heading: &Heading) -> OutlineNode {
        OutlineNode::new(
            heading.title.clone(),
            XyzDestination::new(heading.page, Point::from_xy(0.0, heading.y)),
        )
    }
    // Build bottom-up: a stack of open nodes with their levels, closed into their parent when a
    // heading at their level or shallower arrives.
    let mut outline = Outline::new();
    let mut open: Vec<(u32, OutlineNode)> = Vec::new();
    let close = |open: &mut Vec<(u32, OutlineNode)>, outline: &mut Outline| {
        let (_, done) = open.pop().expect("only called with something open");
        match open.last_mut() {
            Some((_, parent)) => parent.push_child(done),
            None => outline.push_child(done),
        }
    };
    for heading in headings {
        while open
            .last()
            .is_some_and(|(level, _)| *level >= heading.level)
        {
            close(&mut open, &mut outline);
        }
        open.push((heading.level, node(heading)));
    }
    while !open.is_empty() {
        close(&mut open, &mut outline);
    }
    outline
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Typesetter;
    use crate::text::{Options, typeset};
    use grind_text::{App, BlockKind};

    fn sample() -> App {
        let app = App::new();
        app.insert(0, BlockKind::Heading { level: 1 }, "Grind on paper")
            .unwrap();
        app.insert(
            1,
            BlockKind::Paragraph,
            &"Office affine fjord — Ünïcödé. ".repeat(40),
        )
        .unwrap();
        app.insert(2, BlockKind::ListItem { depth: 1 }, "an item")
            .unwrap();
        app
    }

    fn pdf(app: &App) -> Vec<u8> {
        let setter = Typesetter::new(Fonts::bundled());
        let doc = typeset(app, &setter, &Options::default());
        write(
            &doc,
            setter.fonts(),
            &Metadata {
                title: Some("Sample".into()),
            },
        )
        .unwrap()
    }

    #[test]
    fn it_is_a_pdf_and_the_same_document_writes_the_same_bytes() {
        let app = sample();
        let first = pdf(&app);
        assert!(
            first.starts_with(b"%PDF-"),
            "{:?}",
            &first[..first.len().min(8)]
        );
        assert_eq!(
            first,
            pdf(&app),
            "deterministic, so a test can compare two exports"
        );
    }

    #[test]
    fn it_has_the_pages_typeset_at_their_size() {
        let long: Vec<String> = (0..40).map(|_| "word ".repeat(80)).collect();
        let app = App::new();
        for (at, text) in long.iter().enumerate() {
            app.insert(at, BlockKind::Paragraph, text).unwrap();
        }
        let setter = Typesetter::new(Fonts::bundled());
        let doc = typeset(&app, &setter, &Options::default());
        let bytes = write(&doc, setter.fonts(), &Metadata::default()).unwrap();
        let read = hayro_syntax::Pdf::new(bytes).expect("it reads back");
        assert_eq!(read.pages().len(), doc.pages.len());
        for page in read.pages().iter() {
            let (w, h) = page.render_dimensions();
            assert!(
                (w - 595.2756).abs() < 0.01 && (h - 841.8898).abs() < 0.01,
                "{w} × {h}"
            );
        }
    }

    #[test]
    fn its_fonts_are_subsets_far_smaller_than_the_faces() {
        // Four faces' worth of glyphs from a 400 KB family would be well over a megabyte.
        assert!(pdf(&sample()).len() < 60_000, "{}", pdf(&sample()).len());
    }

    /// `pdftotext` is poppler's, and on most machines and in the VM image; without it the
    /// structural tests above still run, and this one says why it did not.
    #[test]
    fn its_text_copies_as_the_text_it_was_set_from() {
        let dir = std::env::temp_dir().join(format!("grind-print-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample.pdf");
        std::fs::write(&file, pdf(&sample())).unwrap();
        let Ok(out) = std::process::Command::new("pdftotext")
            .arg(&file)
            .arg("-")
            .output()
        else {
            eprintln!("pdftotext is not installed; skipping the text check");
            return;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let squashed: String = text.split_whitespace().collect();
        assert!(squashed.contains("Grindonpaper"), "{text}");
        assert!(squashed.contains("Officeaffinefjord—Ünïcödé."), "{text}");
        assert!(squashed.contains("anitem"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
