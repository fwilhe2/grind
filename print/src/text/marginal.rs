// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Headers and footers: their paragraphs laid out once for every page and drawn on each.

use super::*;

/// A header or footer laid out once for every page: each paragraph in the face and alignment its
/// own paragraph style gives it, the height it takes, and the space between it and the body.
/// Its tabs go to the stops its paragraph style sets, which in Writer's own `Header` and `Footer`
/// styles are the centre and the right edge.
pub(super) struct Marginal<'a> {
    pub(super) paragraphs: Vec<MarginalParagraph<'a>>,
    pub(super) width: f64,
    /// The header's own height, its content's or its `fo:min-height` less its spacing, in points.
    pub(super) height: f64,
    pub(super) spacing: f64,
}

impl<'a> Marginal<'a> {
    pub(super) fn lay_out(
        app: &App,
        setter: &'a Typesetter,
        marginal: &grind_text::marginal::Marginal,
        width: f64,
    ) -> Self {
        let paragraphs: Vec<_> = marginal
            .paragraphs
            .iter()
            .map(|paragraph| {
                let resolved = app.resolve_style(paragraph.style.as_deref());
                let props = resolved.props;
                // Its box, drawn as a body paragraph's and never joined (§5c fact 38).
                let frame = props.look().map(|look| Frame {
                    look,
                    open_above: false,
                    open_below: false,
                });
                let align = Align::parse(props.text_align.as_deref());
                let ink = props
                    .color
                    .as_deref()
                    .and_then(Rgb::parse)
                    .unwrap_or(Rgb::BLACK);
                let tabs = props.tabs();
                let stated = TextStyle {
                    font_family: props.font_family,
                    font_size: props.font_size,
                    font_weight: props.font_weight,
                    font_style: props.font_style,
                    letter_spacing: props.letter_spacing,
                    letter_kerning: props.letter_kerning,
                    text_position: None,
                    font_variant: props.font_variant,
                    text_transform: props.text_transform,
                    mid_word: false,
                };
                let line = crate::faces::LineRule::parse(props.line_height.as_deref());
                let face =
                    RoleFace::stating(setter, grind_text::look::Role::Body, stated).with_line(line);
                let points =
                    |value: &Option<String>| value.as_deref().and_then(length_mm).map_or(0.0, pt);
                let inset = |side| frame.as_ref().map_or(0.0, |frame| frame.inset(side));
                use grind_text::table_look::{BOTTOM, LEFT, RIGHT, TOP};
                let room = [
                    points(&props.margin_top) + inset(TOP),
                    points(&props.margin_right) + inset(RIGHT),
                    points(&props.margin_bottom) + inset(BOTTOM),
                    points(&props.margin_left) + inset(LEFT),
                ];
                let looks = paragraph
                    .pieces(1, 1)
                    .map(|(_, look)| look.map_or_default(|look| app.shown(look)))
                    .collect();
                let cell = paragraph
                    .cell
                    .as_ref()
                    .map(|cell| InCell::of(app, cell, width));
                MarginalParagraph {
                    cell,
                    paragraph: paragraph.clone(),
                    looks,
                    face,
                    align,
                    ink,
                    tabs,
                    first: props
                        .text_indent
                        .as_deref()
                        .and_then(length_mm)
                        .map_or(0.0, pt) as f32,
                    room,
                    frame,
                }
            })
            .collect();
        let mut out = Marginal {
            paragraphs,
            width,
            height: 0.0,
            spacing: pt(marginal.spacing),
        };
        // Measured with the fields at one digit; a page number does not wrap a header line.
        let content: f64 = out
            .bands()
            .into_iter()
            .map(|band| out.band_height(band, (1, 1), &[]))
            .sum();
        // `fo:min-height` counts the spacing (`doc/odt-format.md` §5c fact 47).
        out.height = content.max(pt(marginal.min_height) - out.spacing);
        out
    }

    /// A header or footer without what LibreOffice does not give below its last paragraph: its
    /// last line's proportional extra and its margin below (`doc/odt-format.md` §5c fact 68).
    /// Its height may come to less than `fo:min-height` leaves; that one still holds.
    pub(super) fn trimmed(mut self, min_height: f64) -> Self {
        let extra = self.paragraphs.last().map_or(0.0, |paragraph| {
            f64::from(paragraph.face.extra_below()) + paragraph.margin_below()
        });
        self.height = (self.height - extra).max(min_height).max(0.0);
        self
    }

    /// The margin below its last paragraph.
    pub(super) fn below(&self) -> f64 {
        self.paragraphs
            .last()
            .map_or(0.0, MarginalParagraph::margin_below)
    }

    /// `paragraph` as printed on page `page` of `pages`: its pieces' text, each with the look it
    /// is set in, and its lines.
    pub(super) fn wrap(
        &self,
        paragraph: &MarginalParagraph<'_>,
        (page, pages): (usize, usize),
        chapters: &[String],
    ) -> (Vec<(String, usize)>, grind_core::layout::Layout) {
        let pieces: Vec<(String, usize)> = paragraph
            .paragraph
            .pieces_in(page, pages, chapters)
            .enumerate()
            .map(|(at, (text, _))| (text, at))
            .collect();
        let styles: Vec<TextStyle> = paragraph
            .looks
            .iter()
            .map(grind_text::CharStyle::metrics)
            .collect();
        let plain = TextStyle::default();
        // Each piece in its look, carrying on the word the one before it ended in.
        let mut last: Option<char> = None;
        let looks: Vec<TextStyle> = pieces
            .iter()
            .map(|(text, at)| {
                let style = TextStyle {
                    mid_word: last.is_some_and(|c| !c.is_whitespace()),
                    ..styles.get(*at).unwrap_or(&plain).clone()
                };
                last = text.chars().next_back().or(last);
                style
            })
            .collect();
        let mut fragments: Vec<Fragment<'_>> = pieces
            .iter()
            .zip(&looks)
            .map(|((text, _), style)| Fragment { text, style })
            .collect();
        // An empty paragraph still has a line, as tall as its face's.
        if fragments.is_empty() {
            fragments.push(Fragment {
                text: "",
                style: &plain,
            });
        }
        let [_, right, _, left] = paragraph.room;
        let layout = grind_core::layout::wrap_set(
            &fragments,
            (paragraph.across(self.width).1 - left - right).max(1.0) as f32,
            &paragraph.face,
            &grind_core::layout::Setting {
                tabs: paragraph.tabs.clone(),
                first: paragraph.first,
                ..Default::default()
            },
        );
        (pieces, layout)
    }

    /// Draw it with its top at `(left, top)`, on page `page` of `pages`. Decoration to a tagged
    /// PDF: a header repeats on every page and is no part of the reading order.
    pub(super) fn draw(
        &self,
        ops: &mut Vec<Op>,
        (left, top): (f64, f64),
        (page, pages, chapters): (usize, usize, &[String]),
        boxes: &Boxes,
    ) {
        let mut y = top;
        for band in self.bands() {
            // A row's cells each from the row's top, one under the other within a cell.
            let mut cells: Vec<(f64, f64)> = Vec::new();
            for paragraph in &self.paragraphs[band.clone()] {
                let (x, _) = paragraph.across(self.width);
                let pad = paragraph.cell.as_ref().map_or(0.0, |cell| cell.pad[0]);
                let at = stack(&mut cells, x, y + pad);
                *at += self.draw_paragraph(
                    ops,
                    paragraph,
                    (left, *at),
                    (page, pages, chapters),
                    boxes,
                );
            }
            y += self.band_height(band, (page, pages), chapters);
        }
    }

    /// The runs of paragraphs set together: a table row's, else each paragraph alone.
    fn bands(&self) -> Vec<std::ops::Range<usize>> {
        let mut out: Vec<std::ops::Range<usize>> = Vec::new();
        for (i, paragraph) in self.paragraphs.iter().enumerate() {
            let row = paragraph.cell.as_ref().map(|cell| &cell.row);
            match out.last_mut() {
                Some(band)
                    if row.is_some()
                        && self.paragraphs[band.start].cell.as_ref().map(|c| &c.row) == row =>
                {
                    band.end = i + 1;
                }
                _ => out.push(i..i + 1),
            }
        }
        out
    }

    /// How tall a band is: its paragraph's, or its tallest cell's with the cell's padding.
    fn band_height(
        &self,
        band: std::ops::Range<usize>,
        (page, pages): (usize, usize),
        chapters: &[String],
    ) -> f64 {
        let mut cells: Vec<(f64, f64)> = Vec::new();
        for paragraph in &self.paragraphs[band] {
            let [above, _, below, _] = paragraph.room;
            let tall = above
                + f64::from(paragraph.tall(&self.wrap(paragraph, (page, pages), chapters).1))
                + below;
            let (x, _) = paragraph.across(self.width);
            let pad = paragraph
                .cell
                .as_ref()
                .map_or(0.0, |cell| cell.pad[0] + cell.pad[2]);
            *stack(&mut cells, x, pad) += tall;
        }
        cells.into_iter().map(|(_, tall)| tall).fold(0.0, f64::max)
    }

    /// Draw one paragraph with its top margin at `y`, in its cell when it is in one; the height
    /// it took, its margins included.
    fn draw_paragraph(
        &self,
        ops: &mut Vec<Op>,
        paragraph: &MarginalParagraph<'_>,
        (left, top): (f64, f64),
        (page, pages, chapters): (usize, usize, &[String]),
        boxes: &Boxes,
    ) -> f64 {
        let (x, width) = paragraph.across(self.width);
        let left = left + x;
        let mut y = top as f32;
        {
            let [above, right, below, indent] = paragraph.room;
            y += above as f32;
            let (pieces, layout) = self.wrap(paragraph, (page, pages), chapters);
            let measure = (width - indent - right).max(1.0) as f32;
            let tall = paragraph.tall(&layout);
            if let Some(frame) = &paragraph.frame {
                let piece = Piece {
                    index: 0,
                    lines: 0..layout.lines().len(),
                    top: 0.0,
                    left: 0.0,
                    width: f64::from(measure),
                    repeat: false,
                };
                frame.draw(
                    ops,
                    &piece,
                    (left + indent, f64::from(y)),
                    (f64::from(tall), true),
                    crate::faces::Room::default(),
                    (f64::from(y), f64::from(y + tall)),
                );
            }
            // Its pictures first, behind any text (`doc/odt-format.md` §5c fact 27).
            if let Some((row, foot)) = paragraph.row() {
                // A row of pictures set as characters, sitting on the line's foot (§5c fact 29).
                let (offset, _) = paragraph.align.fit(measure, row as f32, 0, true);
                let mut x = left + indent + f64::from(offset);
                for picture in paragraph.paragraph.pictures.iter().filter(|p| p.inline) {
                    let (width, height) = (pt(picture.width), pt(picture.height));
                    ops.push(Op::Image {
                        x: x as f32,
                        y: y + (foot - height) as f32,
                        width: width as f32,
                        height: height as f32,
                        mime: picture.mime.clone(),
                        data: picture.data.clone(),
                        crop: cut(picture.crop.as_ref(), &picture.data),
                        mark: Mark::Decoration,
                    });
                    x += width;
                }
            }
            for picture in paragraph.paragraph.pictures.iter().filter(|p| !p.inline) {
                let (width, height) = (pt(picture.width), pt(picture.height));
                let scale = pt(1.0);
                let x = picture.across.start(
                    scale,
                    width,
                    (0.0, boxes.paper.0),
                    (left - x, self.width),
                    (left - x, self.width),
                );
                let y = picture.down.start(
                    scale,
                    height,
                    (0.0, boxes.paper.1),
                    boxes.body,
                    (f64::from(y), f64::from(tall)),
                );
                ops.push(Op::Image {
                    x: x as f32,
                    y: y as f32,
                    width: width as f32,
                    height: height as f32,
                    mime: picture.mime.clone(),
                    data: picture.data.clone(),
                    crop: cut(picture.crop.as_ref(), &picture.data),
                    mark: Mark::Decoration,
                });
            }
            // Its lines, each from the corner of the box its style names (§5c fact 49).
            for line in &paragraph.paragraph.lines {
                let corner = |rel: &str, paper, content, own| match rel {
                    "page" => paper,
                    "page-content" => content,
                    _ => own,
                };
                let x = corner(&line.rel.0, 0.0, left, left);
                let down = corner(&line.rel.1, 0.0, boxes.body.0, f64::from(y));
                let at = |(across, along): (f64, f64)| {
                    ((x + pt(across)) as f32, (down + pt(along)) as f32)
                };
                ops.push(Op::Line {
                    from: at(line.from),
                    to: at(line.to),
                    width: pt(line.width) as f32,
                    color: Rgb::parse(&line.color).unwrap_or(Rgb::BLACK),
                });
            }
            // Each character with the piece it belongs to.
            let chars: Vec<(char, usize)> = pieces
                .iter()
                .flat_map(|(text, at)| text.chars().map(move |c| (c, *at)))
                .collect();
            let text: Vec<char> = chars.iter().map(|(c, _)| *c).collect();
            let setter = paragraph.face.setter();
            let look = |at: usize| paragraph.looks.get(chars[at].1);
            for line in layout.lines() {
                let visible_end = (line.start..line.end)
                    .rev()
                    .find(|&i| !text[i].is_whitespace())
                    .map_or(line.start, |i| i + 1);
                let last = std::ptr::eq(line, layout.lines().last().unwrap_or(line));
                let content = aligned_width(&layout, line, &text, visible_end, last).min(measure);
                let (offset, _) = paragraph.align.fit(measure, content, 0, true);
                // Each stretch between tabs, and between pieces that look different, is drawn
                // where the layout put it: a footer's `Seite 1 / 2` at the right-hand stop
                // Writer's `Footer` style sets.
                let mut from = line.start;
                for at in line.start..=visible_end {
                    let breaks = at == visible_end
                        || matches!(text[at], '\t' | '\n')
                        || (at > from && look(at) != look(from));
                    if !breaks {
                        continue;
                    }
                    let piece: String = text[from..at].iter().collect();
                    if !piece.trim().is_empty() {
                        let look = look(from);
                        let style = paragraph.face.style(&TextStyle {
                            mid_word: from
                                .checked_sub(1)
                                .is_some_and(|before| !text[before].is_whitespace()),
                            ..look.map_or_default(grind_text::CharStyle::metrics)
                        });
                        let ink = look
                            .and_then(|look| look.color.as_deref())
                            .and_then(Rgb::parse)
                            .unwrap_or(paragraph.ink);
                        let shaped = setter.shape(&piece, &style);
                        ops.push(Op::Text {
                            x: (left + indent) as f32 + offset + layout.x_at(from),
                            y: y + line.top + layout.baseline_of(line),
                            face: shaped.face,
                            size: shaped.size,
                            glyphs: shaped.glyphs,
                            text: piece,
                            color: ink,
                            mark: Mark::Decoration,
                        });
                    }
                    from = if at < visible_end && matches!(text[at], '\t' | '\n') {
                        at + 1
                    } else {
                        at
                    };
                }
            }
            y += tall + below as f32;
        }
        f64::from(y) - top
    }
}

/// The running height of the cell at `x` in a row, starting at `start` the first time it is met.
fn stack(cells: &mut Vec<(f64, f64)>, x: f64, start: f64) -> &mut f64 {
    let at = cells
        .iter()
        .position(|(cx, _)| *cx == x)
        .unwrap_or_else(|| {
            cells.push((x, start));
            cells.len() - 1
        });
    &mut cells[at].1
}

/// Where a table cell's paragraphs go across a header or footer, in points from its left edge:
/// the row they share, the cell's box inside its padding, and the padding.
pub(super) struct InCell {
    row: (String, u32),
    x: f64,
    width: f64,
    pad: [f64; 4],
}

impl InCell {
    /// Its place in a header `width` wide, by its table's look as a body cell's is.
    fn of(app: &App, cell: &grind_text::Cell, width: f64) -> Self {
        let look = app.table_look(&cell.table).unwrap_or_default();
        let (offset, span) = look
            .placement
            .as_ref()
            .and_then(|placement| placement.span(width))
            .unwrap_or((0.0, width));
        let columns: Vec<Option<f64>> = look.columns.iter().map(|w| w.map(pt)).collect();
        let count = (columns.len() as u32).max(cell.column + cell.columns_spanned);
        let (x, cell_width) = grind_text::flow::cell_x_with(cell, count, span, &columns);
        let pad = look
            .cells
            .get(&(cell.row, cell.column))
            .map_or([0.0; 4], |cell| cell.padding.map(pt));
        InCell {
            row: (cell.table.clone(), cell.row),
            x: offset + x + pad[3],
            width: (cell_width - pad[1] - pad[3]).max(1.0),
            pad,
        }
    }
}

/// The boxes a header's or footer's picture can be placed in besides the text area: the paper's
/// width and height, and the body's top and height, in points.
pub(super) struct Boxes {
    pub(super) paper: (f64, f64),
    pub(super) body: (f64, f64),
}

/// One header or footer paragraph, with each piece's look, and the face, alignment, ink and tab
/// stops its style gives it.
pub(super) struct MarginalParagraph<'a> {
    pub(super) paragraph: grind_text::marginal::Paragraph,
    /// Its table cell's place, when it is in one.
    pub(super) cell: Option<InCell>,
    pub(super) looks: Vec<grind_text::CharStyle>,
    pub(super) face: RoleFace<'a>,
    pub(super) align: Align,
    pub(super) ink: Rgb,
    pub(super) tabs: grind_core::layout::Tabs,
    /// Its first line's indent from the others (`fo:text-indent`); negative hangs.
    pub(super) first: f32,
    /// Its margins — top, right, bottom, left — with its box's border and padding, in points.
    pub(super) room: [f64; 4],
    pub(super) frame: Option<Frame>,
}

impl MarginalParagraph<'_> {
    /// Its box's left edge and width across a header `width` wide: its cell's, else the whole.
    fn across(&self, width: f64) -> (f64, f64) {
        self.cell
            .as_ref()
            .map_or((0.0, width), |cell| (cell.x, cell.width))
    }

    /// Its margin below, without the border and padding its box adds there.
    pub(super) fn margin_below(&self) -> f64 {
        let inset = self
            .frame
            .as_ref()
            .map_or(0.0, |frame| frame.inset(grind_text::table_look::BOTTOM));
        (self.room[2] - inset).max(0.0)
    }

    /// The width and height, in points, of the row its pictures set as characters make when it
    /// has no text besides; `None` otherwise, when they are not drawn.
    pub(super) fn row(&self) -> Option<(f64, f64)> {
        let inline = self.paragraph.pictures.iter().filter(|p| p.inline);
        let blank = self.paragraph.text(1, 1).trim().is_empty();
        inline.clone().next().filter(|_| blank)?;
        Some(inline.fold((0.0, 0.0), |(width, height), p| {
            (width + pt(p.width), f64::max(height, pt(p.height)))
        }))
    }

    /// How tall it is: its lines', or its row of pictures' — with the line's descent under them
    /// when it holds a space.
    pub(super) fn tall(&self, layout: &grind_core::layout::Layout) -> f32 {
        let Some((_, height)) = self.row() else {
            return layout.height();
        };
        let mut tall = height as f32;
        if !self.paragraph.text(1, 1).is_empty() {
            let style = self.face.style(&TextStyle::default());
            tall += layout.height() - grind_core::layout::Metrics::ascent(&self.face, &style);
        }
        tall
    }
}
