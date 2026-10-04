// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A text document, typeset onto pages — the word processor's half of this crate.
//!
//! [`typeset`] is the whole pipeline before a backend: the page geometry (the document's own,
//! or the one asked for, or A4), [`grind_text::page::paginate`] with the paper faces, and then
//! each placed line cut into pieces of one formatting ([`grind_text::paint::pieces`], the cut
//! every shell's painter uses) and shaped by the same [`Typesetter`] that measured it.

use std::sync::Arc;

use grind_core::layout::{Fragment, wrap};
use grind_core::page::{PageGeometry, pt};
use grind_core::style::{TextStyle, length_mm};
use grind_text::flow::{self, CellBox};
use grind_text::page::{Piece, Rules, paginate};
use grind_text::{App, BlockKind, BlockView, Caret, paint, picture_of};

use crate::faces::{Column, RoleFace, SPACING, role_faces};
use crate::metrics::Typesetter;
use crate::ops::{Document, Heading, Op, Page, Rgb};

/// The gap between a picture and its caption, in points.
const CAPTION_GAP: f64 = 4.0;
/// A table's rules, in points: a hairline that still prints.
const RULE: f32 = 0.5;

/// What the caller may decide about the pages.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The paper to print on, overriding the document's own. `None` prints on the page the
    /// document states, and on A4 when it states none.
    pub paper: Option<PageGeometry>,
}

/// Typeset `app`'s document with `setter`'s fonts.
pub fn typeset(app: &App, setter: &Typesetter, options: &Options) -> Document {
    let geometry = options.paper.or(app.page()).unwrap_or_default();
    let (left, top) = (pt(geometry.left), pt(geometry.top));
    let (width, height) = (pt(geometry.text_width()), pt(geometry.text_height()));
    let faces = role_faces(setter);
    let across = flow::across(app, width, &SPACING);
    let column = Column {
        faces: &faces,
        width,
        across: &across,
    };
    let body = &faces[0];
    let picture = |view: &BlockView, measure: f64| figure(view, measure, body).map(|f| f.height);
    let pages = paginate(
        app,
        &column,
        width,
        height,
        &SPACING,
        Rules::default(),
        &picture,
    );
    let viewport = app.get_viewport(0..app.block_count());

    let mut outline = Vec::new();
    let pages = pages
        .iter()
        .enumerate()
        .map(|(number, page)| {
            let mut ops = Vec::new();
            let mut span: Option<(Caret, Caret)> = None;
            let mut cover = |from: Caret, to: Caret| {
                span = Some(match span {
                    Some((start, end)) => (start.min(from), end.max(to)),
                    None => (from, to),
                });
            };
            for cell in &page.cells {
                rules(&mut ops, cell, left, top);
            }
            for piece in &page.pieces {
                let Some(view) = viewport.get(piece.index) else {
                    continue;
                };
                let origin = (left + piece.left, top + piece.top);
                if let Some(figure) = figure(view, piece.width, body) {
                    figure.draw(&mut ops, origin, setter, body);
                    let length = view.runs.last().map_or(0, |run| run.end());
                    cover(caret(piece.index, 0), caret(piece.index, length));
                    continue;
                }
                if let BlockKind::Heading { level } = view.kind
                    && piece.lines.start == 0
                {
                    outline.push(Heading {
                        level,
                        title: view.text.clone(),
                        page: number,
                        y: origin.1 as f32,
                    });
                }
                let face = column.face(&view.kind, view.style.as_deref());
                if let Some((from, to)) = lines(&mut ops, app, view, piece, face, origin) {
                    cover(caret(piece.index, from), caret(piece.index, to));
                }
            }
            Page {
                width: pt(geometry.width) as f32,
                height: pt(geometry.height) as f32,
                ops,
                start: span.map(|(start, _)| start),
                end: span.map(|(_, end)| end),
            }
        })
        .collect();
    Document { pages, outline }
}

fn caret(block: usize, offset: usize) -> Caret {
    Caret { block, offset }
}

/// The four sides of a table cell.
fn rules(ops: &mut Vec<Op>, cell: &CellBox, left: f64, top: f64) {
    let (x0, y0) = ((left + cell.left) as f32, (top + cell.top) as f32);
    let (x1, y1) = (x0 + cell.width as f32, y0 + cell.height as f32);
    for (from, to) in [
        ((x0, y0), (x1, y0)),
        ((x1, y0), (x1, y1)),
        ((x0, y1), (x1, y1)),
        ((x0, y0), (x0, y1)),
    ] {
        ops.push(Op::Line {
            from,
            to,
            width: RULE,
            color: Rgb::BLACK,
        });
    }
}

/// One piece of a block: its lines, each cut where its formatting changes, and the bullet in
/// front of a list item's first line. Answers the character offsets the piece runs between.
fn lines(
    ops: &mut Vec<Op>,
    app: &App,
    view: &BlockView,
    piece: &Piece,
    face: &RoleFace<'_>,
    (left, top): (f64, f64),
) -> Option<(usize, usize)> {
    let layout = app
        .layout_block(view.index, piece.width as f32, face)
        .ok()?;
    let first = layout.lines().get(piece.lines.start)?;
    let last = layout.lines().get(piece.lines.end.checked_sub(1)?)?;
    let span = (first.start, last.end);
    let first_top = first.top;
    let setter = face.setter();
    for at in piece.lines.clone() {
        let Some(line) = layout.lines().get(at) else {
            continue;
        };
        let line_top = top as f32 + (line.top - first_top);
        let baseline = line_top + layout.baseline();
        if at == 0
            && let BlockKind::ListItem { depth } = view.kind
        {
            let style = face.style(&TextStyle::default());
            let mark = paint::bullet(depth);
            ops.push(Op::Text {
                x: (left - SPACING.indent * 2.0 / 3.0) as f32,
                y: baseline,
                face: setter.face_of(&style).0,
                size: setter.face_of(&style).1,
                glyphs: setter.shape(mark, &style).glyphs,
                text: mark.to_owned(),
                color: Rgb::BLACK,
            });
        }
        for cut in paint::pieces(&view.runs, line.start, line.end) {
            for (start, text) in paint::drawable(cut.start, cut.text) {
                if text.is_empty() {
                    continue;
                }
                let style = face.style(&cut.props.metrics());
                let shaped = setter.shape(text, &style);
                let x = left as f32 + layout.x_at(start);
                let run: f32 = shaped.glyphs.iter().map(|g| g.x_advance).sum();
                let color = cut
                    .props
                    .color
                    .as_deref()
                    .and_then(Rgb::parse)
                    .unwrap_or(Rgb::BLACK);
                if let Some(fill) = cut.props.background.as_deref().and_then(Rgb::parse) {
                    ops.push(Op::Rect {
                        x,
                        y: line_top,
                        width: run,
                        height: line.height,
                        color: fill,
                    });
                }
                let size = shaped.size;
                ops.push(Op::Text {
                    x,
                    y: baseline,
                    face: shaped.face,
                    size,
                    glyphs: shaped.glyphs,
                    text: text.to_owned(),
                    color,
                });
                // Where an underline and a strike sit, as fractions of the size: close to what
                // the bundled faces' own `post` and `OS/2` tables say, and the same for any face.
                for (on, drop) in [
                    (&cut.props.underline, 0.12),
                    (&cut.props.line_through, -0.3),
                ] {
                    if on.as_deref().is_some_and(|value| value != "none") {
                        let y = baseline + size * drop;
                        ops.push(Op::Line {
                            from: (x, y),
                            to: (x + run, y),
                            width: size * 0.06,
                            color,
                        });
                    }
                }
            }
        }
    }
    Some(span)
}

/// A picture block on paper: the picture at the size the document gives it, fitted to the
/// measure, and its caption's lines under it.
struct Figure {
    width: f64,
    picture: f64,
    mime: String,
    data: Arc<Vec<u8>>,
    caption: Option<String>,
    height: f64,
}

/// A block's figure, when it is a picture whose size the document states. Paper is physical,
/// so `svg:width` and `svg:height` are exactly the right size here, where a screen ignores
/// them. A picture with no stated size is measured as text (ponytail: decoding its pixels for a
/// natural size is the shells' `Decoder`, which this crate does not have yet).
fn figure(view: &BlockView, measure: f64, body: &RoleFace<'_>) -> Option<Figure> {
    let (image, caption) = picture_of(view)?;
    let length = |value: &Option<String>| value.as_deref().and_then(length_mm).map(pt);
    let (w, h) = (
        length(&image.width)?.max(1.0),
        length(&image.height)?.max(1.0),
    );
    let width = w.min(measure.max(1.0));
    let picture = h * width / w;
    let caption = caption.map(str::to_owned).filter(|c| !c.trim().is_empty());
    let below = caption.as_deref().map_or(0.0, |text| {
        CAPTION_GAP + f64::from(caption_layout(text, measure, body).height())
    });
    Some(Figure {
        width,
        picture,
        mime: image.mime.clone(),
        data: Arc::new(image.data.clone()),
        caption,
        height: picture + below,
    })
}

fn caption_layout(text: &str, measure: f64, body: &RoleFace<'_>) -> grind_core::layout::Layout {
    let style = TextStyle::default();
    wrap(
        &[Fragment {
            text,
            style: &style,
        }],
        measure as f32,
        body,
    )
}

impl Figure {
    fn draw(
        &self,
        ops: &mut Vec<Op>,
        (x, y): (f64, f64),
        setter: &Typesetter,
        body: &RoleFace<'_>,
    ) {
        ops.push(Op::Image {
            x: x as f32,
            y: y as f32,
            width: self.width as f32,
            height: self.picture as f32,
            mime: self.mime.clone(),
            data: self.data.clone(),
        });
        let Some(caption) = &self.caption else {
            return;
        };
        let layout = caption_layout(caption, self.width.max(1.0), body);
        let chars: Vec<char> = caption.chars().collect();
        let style = body.style(&TextStyle::default());
        let (face, size) = setter.face_of(&style);
        for line in layout.lines() {
            let text: String = chars[line.start..line.end].iter().collect();
            let top = (y + self.picture + CAPTION_GAP) as f32 + line.top;
            ops.push(Op::Text {
                x: x as f32,
                y: top + layout.baseline(),
                face,
                size,
                glyphs: setter.shape(&text, &style).glyphs,
                text,
                color: Rgb::BLACK,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::Fonts;
    use crate::ops::{Op, Rgb};
    use grind_text::{BlockKind, Caret, CharStyle};

    fn setter() -> Typesetter {
        Typesetter::new(Fonts::bundled())
    }

    fn doc(blocks: &[(BlockKind, &str)]) -> App {
        let app = App::new();
        for (at, (kind, text)) in blocks.iter().enumerate() {
            app.insert(at, kind.clone(), text).unwrap();
        }
        app.delete(blocks.len()..blocks.len() + 1).unwrap();
        app
    }

    /// Every text op on a page as `(x, y, text)`.
    fn texts(page: &crate::ops::Page) -> Vec<(f32, f32, String)> {
        page.ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { x, y, text, .. } => Some((*x, *y, text.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_short_document_is_one_a4_page_set_inside_its_margins() {
        let app = doc(&[(BlockKind::Paragraph, "Hello, paper.")]);
        let t = setter();
        let typeset = typeset(&app, &t, &Options::default());
        assert_eq!(typeset.pages.len(), 1);
        let page = &typeset.pages[0];
        assert!((page.width - 595.2756).abs() < 1e-3 && (page.height - 841.8898).abs() < 1e-3);
        let texts = texts(page);
        assert_eq!(texts.len(), 1);
        let (x, y, text) = &texts[0];
        assert_eq!(text, "Hello, paper.");
        assert!((x - 56.6929).abs() < 1e-3, "the left margin, 20 mm: {x}");
        let ascent = grind_core::layout::Metrics::ascent(
            &t,
            &grind_core::style::TextStyle {
                font_size: Some("12pt".into()),
                ..Default::default()
            },
        );
        assert!(
            (y - (56.6929 + ascent)).abs() < 1e-3,
            "the first baseline: {y}"
        );
    }

    #[test]
    fn the_paper_asked_for_wins_over_the_documents() {
        let app = doc(&[(BlockKind::Paragraph, "x")]);
        let options = Options {
            paper: PageGeometry::paper("a5-landscape"),
        };
        let page = &typeset(&app, &setter(), &options).pages[0];
        assert!((page.width - pt(210.0) as f32).abs() < 1e-3);
        assert!((page.height - pt(148.0) as f32).abs() < 1e-3);
    }

    #[test]
    fn every_character_is_printed_once_however_many_pages_it_takes() {
        let paragraph = "The quick brown fox jumps over the lazy dog, again and again. ".repeat(12);
        let blocks: Vec<(BlockKind, &str)> = (0..30)
            .map(|_| (BlockKind::Paragraph, paragraph.as_str()))
            .collect();
        let app = doc(&blocks);
        let typeset = typeset(&app, &setter(), &Options::default());
        assert!(typeset.pages.len() > 2, "{} pages", typeset.pages.len());
        let printed: String = typeset
            .pages
            .iter()
            .flat_map(texts)
            .map(|(_, _, t)| t)
            .collect();
        let expected: String = (0..30).map(|_| paragraph.as_str()).collect();
        assert_eq!(printed.replace(' ', ""), expected.replace(' ', ""));
        for page in &typeset.pages {
            for (_, y, _) in texts(page) {
                assert!(
                    y > 56.0 && y < page.height - 56.0,
                    "inside the margins: {y}"
                );
            }
        }
    }

    #[test]
    fn a_bold_run_is_set_in_the_bold_face_and_a_coloured_one_in_its_colour() {
        let app = doc(&[(BlockKind::Paragraph, "plain loud")]);
        let style = CharStyle {
            font_weight: Some("bold".into()),
            color: Some("#c00000".into()),
            background: Some("#ffff00".into()),
            underline: Some("solid".into()),
            ..CharStyle::default()
        };
        app.set_char_style(
            Caret {
                block: 0,
                offset: 6,
            },
            Caret {
                block: 0,
                offset: 10,
            },
            &style,
        )
        .unwrap();
        let t = setter();
        let page = &typeset(&app, &t, &Options::default()).pages[0];
        let loud = page.ops.iter().find_map(|op| match op {
            Op::Text {
                text, face, color, ..
            } if text == "loud" => Some((*face, *color)),
            _ => None,
        });
        let (face, color) = loud.expect("the bold run is its own piece");
        assert!(t.fonts().face(face).bold);
        assert_eq!(color, Rgb(0xc0, 0, 0));
        assert!(page.ops.iter().any(|op| matches!(
            op,
            Op::Rect {
                color: Rgb(255, 255, 0),
                ..
            }
        )));
        assert!(page.ops.iter().any(|op| matches!(
            op,
            Op::Line {
                color: Rgb(0xc0, 0, 0),
                ..
            }
        )));
    }

    #[test]
    fn a_list_item_is_indented_and_has_its_bullet_before_it() {
        let app = doc(&[
            (BlockKind::Paragraph, "para"),
            (BlockKind::ListItem { depth: 1 }, "item"),
        ]);
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let texts = texts(page);
        let para = texts.iter().find(|t| t.2 == "para").unwrap();
        let item = texts.iter().find(|t| t.2 == "item").unwrap();
        let bullet = texts.iter().find(|t| t.2 == "\u{2022}").unwrap();
        assert!((item.0 - para.0 - 18.0).abs() < 1e-3, "one list level in");
        assert!(bullet.0 < item.0 && bullet.0 >= para.0);
        assert_eq!(bullet.1, item.1, "on the item's baseline");
    }

    #[test]
    fn every_heading_is_in_the_outline_with_the_page_it_landed_on() {
        let long = "words ".repeat(400);
        let app = doc(&[
            (BlockKind::Heading { level: 1 }, "One"),
            (BlockKind::Paragraph, &long),
            (BlockKind::Paragraph, &long),
            (BlockKind::Heading { level: 2 }, "Two"),
        ]);
        let typeset = typeset(&app, &setter(), &Options::default());
        let outline: Vec<(u32, &str, usize)> = typeset
            .outline
            .iter()
            .map(|h| (h.level, h.title.as_str(), h.page))
            .collect();
        assert_eq!(outline[0], (1, "One", 0));
        assert_eq!(outline[1].0, 2);
        assert_eq!(outline[1].2, typeset.pages.len() - 1);
    }

    #[test]
    fn a_table_is_drawn_with_its_rules() {
        let app = doc(&[(BlockKind::Paragraph, "before")]);
        app.insert_table(1, 2, 2, None).unwrap();
        app.insert_text(
            Caret {
                block: 1,
                offset: 0,
            },
            "cell",
        )
        .unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let rules = page
            .ops
            .iter()
            .filter(|op| matches!(op, Op::Line { .. }))
            .count();
        assert_eq!(rules, 16, "four sides of four cells");
        assert!(texts(page).iter().any(|t| t.2 == "cell"));
    }

    #[test]
    fn a_picture_prints_at_the_size_the_document_gives_it_fitted_to_the_measure() {
        let app = doc(&[(BlockKind::Paragraph, ""), (BlockKind::Paragraph, "")]);
        let png = b"\x89PNG\r\n\x1a\n".to_vec();
        let at = |block| Caret { block, offset: 0 };
        app.insert_image(
            at(0),
            "image/png".into(),
            png.clone(),
            Some("5cm".into()),
            Some("2.5cm".into()),
        )
        .unwrap();
        app.insert_image(
            at(1),
            "image/png".into(),
            png,
            Some("40cm".into()),
            Some("20cm".into()),
        )
        .unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let boxes: Vec<(f32, f32)> = page
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Image { width, height, .. } => Some((*width, *height)),
                _ => None,
            })
            .collect();
        assert_eq!(boxes.len(), 2);
        assert!(
            (boxes[0].0 - pt(50.0) as f32).abs() < 1e-3
                && (boxes[0].1 - pt(25.0) as f32).abs() < 1e-3
        );
        let measure = pt(170.0) as f32;
        assert!((boxes[1].0 - measure).abs() < 1e-3 && (boxes[1].1 - measure / 2.0).abs() < 1e-3);
    }

    #[test]
    fn each_page_knows_the_first_and_last_character_on_it() {
        let paragraph = "The quick brown fox jumps over the lazy dog. ".repeat(60);
        let blocks: Vec<(BlockKind, &str)> = (0..6)
            .map(|_| (BlockKind::Paragraph, paragraph.as_str()))
            .collect();
        let app = doc(&blocks);
        let typeset = typeset(&app, &setter(), &Options::default());
        assert!(typeset.pages.len() >= 2);
        let first = &typeset.pages[0];
        assert_eq!(
            first.start,
            Some(Caret {
                block: 0,
                offset: 0
            })
        );
        // Each page starts exactly where the one before it ended.
        for pair in typeset.pages.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        let last = typeset.pages.last().unwrap();
        let length = paragraph.chars().count();
        assert_eq!(
            last.end,
            Some(Caret {
                block: 5,
                offset: length
            })
        );
        // Some page starts inside a paragraph rather than at one.
        assert!(
            typeset
                .pages
                .iter()
                .any(|p| p.start.is_some_and(|c| c.offset > 0))
        );
    }

    #[test]
    fn an_empty_document_is_one_blank_page() {
        let app = doc(&[]);
        let typeset = typeset(&app, &setter(), &Options::default());
        assert_eq!(typeset.pages.len(), 1);
        assert!(typeset.pages[0].ops.is_empty());
        assert_eq!((typeset.pages[0].start, typeset.pages[0].end), (None, None));
    }
}
