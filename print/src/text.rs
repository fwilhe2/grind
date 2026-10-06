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

use crate::align::{self, Align};
use crate::faces::{Column, RoleFace, SPACING, role_faces};
use crate::metrics::Typesetter;
use crate::ops::{Document, Element, Heading, Mark, Op, Page, Rgb};

/// The gap between a picture and its caption, in points.
const CAPTION_GAP: f64 = 4.0;
/// The ink of a link that states no colour: Writer's own for an unstyled hyperlink.
const LINK: Rgb = Rgb(0, 0, 0x80);
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
    setter.hint(app.font_generics());
    let geometry = options.paper.or(app.page()).unwrap_or_default();
    let (left, margin_top) = (pt(geometry.left), pt(geometry.top));
    let width = pt(geometry.text_width());
    // The header from the top margin down and the footer from the bottom margin up, each with
    // the space between it and the body: what is left is the body's (`doc/odt-format.md` §5c,
    // fact 9).
    let (header, footer) = app.marginals();
    let header = header.map(|m| Marginal::lay_out(app, setter, &m, width));
    let footer = footer.map(|m| Marginal::lay_out(app, setter, &m, width));
    let room = |m: &Option<Marginal<'_>>| m.as_ref().map_or(0.0, |m| m.height + m.spacing);
    let top = margin_top + room(&header);
    let height = (pt(geometry.text_height()) - room(&header) - room(&footer)).max(1.0);
    let faces = role_faces(setter);
    let viewport = app.get_viewport(0..app.block_count());
    let looks: std::collections::HashMap<String, grind_text::table_look::TableLook> = viewport
        .iter()
        .filter_map(|view| view.cell.as_ref().map(|cell| cell.table.clone()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|table| Some((table.clone(), app.table_look(&table)?)))
        .collect();
    let across = flow::across_with(app, width, &SPACING, &crate::faces::TableLooks(&looks));
    let blocks = block_faces(app, setter, &viewport);
    let spacing = block_spacing(app, &viewport);
    let breaks = block_breaks(app, &viewport);
    let indents: std::collections::HashMap<usize, f32> = viewport
        .iter()
        .filter(|view| view.cell.is_none())
        .filter_map(|view| {
            let props = app.paragraph(view.index)?.props;
            let first = props.text_indent.as_deref().and_then(length_mm).map(pt)?;
            (first != 0.0).then_some((view.index, first as f32))
        })
        .collect();
    // Every paragraph's tab stops — its own and the default interval — in points.
    let tabs: std::collections::HashMap<usize, grind_core::layout::Tabs> = viewport
        .iter()
        .filter(|view| view.text.contains('\t'))
        .filter_map(|view| {
            let tabs = app.paragraph(view.index)?.props.tabs();
            (!tabs.is_empty()).then_some((view.index, tabs))
        })
        .collect();
    let column = Column {
        indents: &indents,
        tabs: &tabs,
        looks: &looks,
        faces: &faces,
        blocks: &blocks,
        spacing: &spacing,
        breaks: &breaks,
        width,
        across: &across,
    };
    let body = &faces[0];
    let picture = |view: &BlockView, measure: f64| figure(view, measure, body).map(|f| f.height);
    // The document's own widows and orphans, and none where it states none — what Writer does
    // (`doc/odt-format.md` §5c, fact 3).
    let stated = app.paragraph_defaults();
    let breaking = Rules {
        orphans: stated.orphans.unwrap_or(0) as usize,
        widows: stated.widows.unwrap_or(0) as usize,
    };
    let pages = paginate(app, &column, width, height, &SPACING, breaking, &picture);

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
                let look = viewport.get(cell.first).and_then(|view| {
                    let at = view.cell.as_ref()?;
                    let table = looks.get(&at.table)?;
                    Some((table, table.cell(at.row, at.column)))
                });
                match look {
                    // A table whose document styles its cells is drawn as they say — an
                    // unstyled cell of it with no rules at all, as Writer draws one.
                    Some((table, own)) if !table.cells.is_empty() => {
                        if let Some(own) = own {
                            styled_cell(&mut ops, cell, own, left, top);
                        }
                    }
                    _ => rules(&mut ops, cell, left, top),
                }
            }
            for piece in &page.pieces {
                let Some(view) = viewport.get(piece.index) else {
                    continue;
                };
                let origin = (left + piece.left, top + piece.top);
                if let Some(figure) = figure(view, piece.width, body) {
                    figure.draw(&mut ops, origin, setter, body, piece.index);
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
                let face = column.face(view.index, &view.kind, view.style.as_deref());
                let first = grind_text::Faces::first_indent(&column, view.index);
                let tabs = grind_text::Faces::tabs(&column, view.index);
                let drawn = lines(&mut ops, app, view, piece, face, (first, &tabs), origin);
                if let (Some((from, to)), false) = (drawn, piece.repeat) {
                    cover(caret(piece.index, from), caret(piece.index, to));
                }
            }
            let count = pages.len();
            if let Some(header) = &header {
                header.draw(&mut ops, (left, margin_top), number + 1, count);
            }
            if let Some(footer) = &footer {
                let bottom = pt(geometry.height) - pt(geometry.bottom);
                footer.draw(&mut ops, (left, bottom - footer.height), number + 1, count);
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
    let structure = viewport
        .iter()
        .map(|view| {
            let block = view.index;
            if let Some(cell) = &view.cell {
                return Element::Cell {
                    block,
                    table: cell.table.clone(),
                    row: cell.row,
                    column: cell.column,
                };
            }
            if let Some((_, caption)) = picture_of(view) {
                return Element::Figure {
                    block,
                    alt: caption.map(str::to_owned).filter(|c| !c.trim().is_empty()),
                };
            }
            match view.kind {
                BlockKind::Heading { level } => Element::Heading {
                    block,
                    level,
                    title: view.text.clone(),
                },
                BlockKind::ListItem { depth } => Element::ListItem { block, depth },
                BlockKind::Paragraph => Element::Paragraph { block },
            }
        })
        .collect();
    Document {
        pages,
        outline,
        structure,
    }
}

/// A face for every block whose paragraph style says something about its text
/// (`doc/pdf-export.md` P5): all four properties when the document declares the block's style,
/// and only the default style's family when it does not — such a block keeps the screen's own
/// size and weight for its role, which is the better guess for a heading whose style is missing.
fn block_faces<'a>(
    app: &App,
    setter: &'a Typesetter,
    viewport: &grind_text::Viewport,
) -> std::collections::HashMap<usize, RoleFace<'a>> {
    viewport
        .iter()
        .filter_map(|view| {
            let resolved = app.paragraph(view.index)?;
            let props = resolved.props;
            let stated = match resolved.declared {
                true => TextStyle {
                    font_family: props.font_family,
                    font_size: props.font_size,
                    font_weight: props.font_weight,
                    font_style: props.font_style,
                },
                false => TextStyle {
                    font_family: props.font_family,
                    ..TextStyle::default()
                },
            };
            // A declared style is the whole answer, so its block is set as body text under it:
            // a heading style that states no size or weight is not given the screen's.
            let role = match resolved.declared {
                true => grind_text::look::Role::Body,
                false => grind_text::look::Role::of(&view.kind, view.style.as_deref()),
            };
            let line = crate::faces::LineRule::parse(props.line_height.as_deref());
            (resolved.declared || stated != TextStyle::default() || line.is_some()).then(|| {
                let face = RoleFace::stating(setter, role, stated).with_line(line);
                (view.index, face)
            })
        })
        .collect()
}

/// The space above and below every block whose paragraph style the document declares, in
/// points — added, as Writer adds them (`doc/odt-format.md` §5c, fact 5). A margin in a unit with
/// no length (a percentage of a parent this build does not keep) counts as none.
fn block_spacing(
    app: &App,
    viewport: &grind_text::Viewport,
) -> std::collections::HashMap<usize, crate::faces::Room> {
    let points = |value: &Option<String>| value.as_deref().and_then(length_mm).map_or(0.0, pt);
    viewport
        .iter()
        .filter_map(|view| {
            let resolved = app.paragraph(view.index)?;
            // A block in a table is spaced by its cell, which the flow places itself.
            (resolved.declared && view.cell.is_none()).then(|| {
                let props = &resolved.props;
                let room = crate::faces::Room {
                    space: grind_text::flow::Space {
                        above: points(&props.margin_top),
                        below: points(&props.margin_bottom),
                        left: points(&props.margin_left),
                    },
                    right: points(&props.margin_right),
                };
                (view.index, room)
            })
        })
        .collect()
}

/// Every block whose paragraph style says something about page breaks around it: a page break
/// before or after (`page`, and `even-page`/`odd-page` as a page, since this build prints no
/// blank pages), and `fo:keep-with-next` (`always`, or `auto` to let even a heading go).
fn block_breaks(
    app: &App,
    viewport: &grind_text::Viewport,
) -> std::collections::HashMap<usize, grind_text::page::Breaks> {
    let page = |value: &Option<String>| {
        matches!(value.as_deref(), Some("page" | "even-page" | "odd-page"))
    };
    viewport
        .iter()
        .filter(|view| view.cell.is_none())
        .filter_map(|view| {
            let props = app.paragraph(view.index)?.props;
            let breaks = grind_text::page::Breaks {
                page_before: page(&props.break_before),
                page_after: page(&props.break_after),
                keep_with_next: match props.keep_with_next.as_deref() {
                    Some("always") => Some(true),
                    Some("auto") => Some(false),
                    _ => None,
                },
            };
            (breaks != grind_text::page::Breaks::default()).then_some((view.index, breaks))
        })
        .collect()
}

fn caret(block: usize, offset: usize) -> Caret {
    Caret { block, offset }
}

/// A styled cell: its fill under everything, and each of its four borders as its style spells
/// it (`"0.5pt solid #8da5a5"`; `none` or a width of zero draws nothing).
fn styled_cell(
    ops: &mut Vec<Op>,
    cell: &CellBox,
    look: &grind_text::table_look::CellLook,
    left: f64,
    top: f64,
) {
    let (x0, y0) = ((left + cell.left) as f32, (top + cell.top) as f32);
    let (x1, y1) = (x0 + cell.width as f32, y0 + cell.height as f32);
    if let Some(fill) = look.background.as_deref().and_then(Rgb::parse) {
        ops.push(Op::Rect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
            color: fill,
        });
    }
    let sides = [
        ((x0, y0), (x1, y0)),
        ((x1, y0), (x1, y1)),
        ((x0, y1), (x1, y1)),
        ((x0, y0), (x0, y1)),
    ];
    for (border, (from, to)) in look.border.iter().zip(sides) {
        let Some((width, _, colour)) = border.as_deref().and_then(grind_core::style::border_parts)
        else {
            continue;
        };
        if width <= 0.0 {
            continue;
        }
        ops.push(Op::Line {
            from,
            to,
            width: width as f32,
            color: Rgb::parse(colour).unwrap_or(Rgb::BLACK),
        });
    }
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
    (first, tabs): (f32, &grind_core::layout::Tabs),
    (left, top): (f64, f64),
) -> Option<(usize, usize)> {
    let layout = app
        .layout_block_tabbed(view.index, piece.width as f32, face, first, tabs)
        .ok()?;
    let first = layout.lines().get(piece.lines.start)?;
    let last = layout.lines().get(piece.lines.end.checked_sub(1)?)?;
    let span = (first.start, last.end);
    let first_top = first.top;
    let setter = face.setter();
    let align = Align::parse(
        app.paragraph(view.index)
            .and_then(|resolved| resolved.props.text_align)
            .as_deref(),
    );
    let chars: Vec<char> = view.text.chars().collect();
    let count = layout.lines().len();
    // The paragraph style's ink, under every run that states none.
    let ink = app
        .paragraph(view.index)
        .and_then(|resolved| resolved.props.color)
        .as_deref()
        .and_then(Rgb::parse)
        .unwrap_or(Rgb::BLACK);
    for at in piece.lines.clone() {
        let Some(line) = layout.lines().get(at) else {
            continue;
        };
        // The line's visible end — trailing spaces are not part of what is aligned — and the
        // spaces between its first and last visible characters, which justification widens.
        let visible_end = (line.start..line.end)
            .rev()
            .find(|&i| chars.get(i).is_some_and(|c| !c.is_whitespace()))
            .map_or(line.start, |i| i + 1);
        let content = match visible_end == line.end {
            true => line.width,
            false => layout.x_at(visible_end),
        };
        let interior = |upto: usize| {
            (line.start..upto.min(visible_end))
                .filter(|&i| chars.get(i) == Some(&' '))
                .count()
        };
        let fit = align::fit(
            align,
            piece.width as f32,
            content,
            interior(visible_end),
            at + 1 == count,
        );
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
                mark: match piece.repeat {
                    true => Mark::Decoration,
                    false => Mark::Label(view.index),
                },
            });
        }
        for cut in paint::pieces(&view.runs, line.start, line.end) {
            for (start, text) in paint::drawable(cut.start, cut.text) {
                if text.is_empty() {
                    continue;
                }
                let style = face.style(&cut.props.metrics());
                let mut shaped = setter.shape(text, &style);
                let x = left as f32
                    + fit.offset
                    + layout.x_at(start)
                    + fit.extra * interior(start) as f32;
                if fit.extra > 0.0 {
                    // Every interior space this piece holds is drawn `extra` wider.
                    for glyph in &mut shaped.glyphs {
                        let here = start + text[..glyph.text.start].chars().count();
                        if &text[glyph.text.clone()] == " " && here < visible_end {
                            glyph.x_advance += fit.extra;
                        }
                    }
                }
                let run: f32 = shaped.glyphs.iter().map(|g| g.x_advance).sum();
                // A link with no colour of its own is drawn as Writer draws an unstyled one,
                // navy and underlined (`doc/odt-format.md` §5c, fact 12).
                let link = view
                    .runs
                    .iter()
                    .any(|run| run.href.is_some() && run.start <= start && start < run.end());
                let color = match (cut.props.color.as_deref().and_then(Rgb::parse), link) {
                    (Some(color), _) => color,
                    (None, true) => LINK,
                    (None, false) => ink,
                };
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
                    // A heading row repeated on a later page is drawn, not read again.
                    mark: match piece.repeat {
                        true => Mark::Decoration,
                        false => Mark::Content(view.index),
                    },
                });
                // Where an underline and a strike sit, as fractions of the size: close to what
                // the bundled faces' own `post` and `OS/2` tables say, and the same for any face.
                let underline = match (&cut.props.underline, link) {
                    (None, true) => Some("solid".to_owned()),
                    (stated, _) => stated.clone(),
                };
                for (on, drop) in [(&underline, 0.12), (&cut.props.line_through, -0.3)] {
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

/// A header or footer laid out once for every page: each paragraph in the face and alignment its
/// own paragraph style gives it, the height it takes, and the space between it and the body.
/// Its tabs go to the stops its paragraph style sets, which in Writer's own `Header` and `Footer`
/// styles are the centre and the right edge.
struct Marginal<'a> {
    paragraphs: Vec<MarginalParagraph<'a>>,
    width: f64,
    /// The header's own height, its content's or its `fo:min-height`, in points.
    height: f64,
    spacing: f64,
}

impl<'a> Marginal<'a> {
    fn lay_out(
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
                };
                let face = RoleFace::stating(setter, grind_text::look::Role::Body, stated);
                (paragraph.clone(), face, align, ink, tabs)
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
            .paragraphs
            .iter()
            .map(|(paragraph, face, _, _, tabs)| {
                f64::from(out.wrap(&paragraph.text(1, 1), face, tabs).height())
            })
            .sum();
        out.height = content.max(pt(marginal.min_height));
        out
    }

    fn wrap(
        &self,
        text: &str,
        face: &RoleFace<'_>,
        tabs: &grind_core::layout::Tabs,
    ) -> grind_core::layout::Layout {
        let style = TextStyle::default();
        grind_core::layout::wrap_tabbed(
            &[Fragment {
                text,
                style: &style,
            }],
            self.width as f32,
            face,
            0.0,
            tabs,
        )
    }

    /// Draw it with its top at `(left, top)`, on page `page` of `pages`. Decoration to a tagged
    /// PDF: a header repeats on every page and is no part of the reading order.
    fn draw(&self, ops: &mut Vec<Op>, (left, top): (f64, f64), page: usize, pages: usize) {
        let mut y = top as f32;
        for (paragraph, face, align, ink, tabs) in &self.paragraphs {
            let text = paragraph.text(page, pages);
            let layout = self.wrap(&text, face, tabs);
            let chars: Vec<char> = text.chars().collect();
            let style = face.style(&TextStyle::default());
            let setter = face.setter();
            for line in layout.lines() {
                let visible_end = (line.start..line.end)
                    .rev()
                    .find(|&i| !chars[i].is_whitespace())
                    .map_or(line.start, |i| i + 1);
                let content = match visible_end == line.end {
                    true => line.width,
                    false => layout.x_at(visible_end),
                };
                let fit = align::fit(*align, self.width as f32, content, 0, true);
                // Each stretch between tabs is drawn where the layout put it: a footer's
                // `Seite 1 / 2` at the right-hand stop Writer's `Footer` style sets.
                let mut from = line.start;
                for at in line.start..=visible_end {
                    if at < visible_end && chars[at] != '\t' && chars[at] != '\n' {
                        continue;
                    }
                    let piece: String = chars[from..at].iter().collect();
                    if !piece.trim().is_empty() {
                        let shaped = setter.shape(&piece, &style);
                        ops.push(Op::Text {
                            x: left as f32 + fit.offset + layout.x_at(from),
                            y: y + line.top + layout.baseline(),
                            face: shaped.face,
                            size: shaped.size,
                            glyphs: shaped.glyphs,
                            text: piece,
                            color: *ink,
                            mark: Mark::Decoration,
                        });
                    }
                    from = at + 1;
                }
            }
            y += layout.height();
        }
    }
}

/// One header or footer paragraph, with the face, alignment, ink and tab stops its style gives it.
type MarginalParagraph<'a> = (
    grind_text::marginal::Paragraph,
    RoleFace<'a>,
    Align,
    Rgb,
    grind_core::layout::Tabs,
);

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
        block: usize,
    ) {
        ops.push(Op::Image {
            x: x as f32,
            y: y as f32,
            width: self.width as f32,
            height: self.picture as f32,
            mime: self.mime.clone(),
            data: self.data.clone(),
            mark: Mark::Content(block),
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
                mark: Mark::Content(block),
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
    fn every_block_is_in_the_structure_and_its_text_is_marked_as_its_own() {
        let app = doc(&[
            (BlockKind::Heading { level: 2 }, "Two"),
            (BlockKind::Paragraph, "para"),
            (BlockKind::ListItem { depth: 1 }, "item"),
        ]);
        app.insert_table(3, 1, 2, Some("T".into())).unwrap();
        let typeset = typeset(&app, &setter(), &Options::default());
        use crate::ops::{Element, Mark};
        assert_eq!(
            typeset.structure,
            vec![
                Element::Heading {
                    block: 0,
                    level: 2,
                    title: "Two".into()
                },
                Element::Paragraph { block: 1 },
                Element::ListItem { block: 2, depth: 1 },
                Element::Cell {
                    block: 3,
                    table: "T".into(),
                    row: 0,
                    column: 0
                },
                Element::Cell {
                    block: 4,
                    table: "T".into(),
                    row: 0,
                    column: 1
                },
            ]
        );
        let marks: Vec<(String, Mark)> = typeset.pages[0]
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { text, mark, .. } => Some((text.clone(), *mark)),
                _ => None,
            })
            .collect();
        assert!(marks.contains(&("para".into(), Mark::Content(1))));
        assert!(marks.contains(&("\u{2022}".into(), Mark::Label(2))));
    }

    /// `doc/odt-format.md` §5c fact 3: a paragraph that does not fit leaves a lone line on the
    /// next page when the document states no widows, and two when it states two.
    #[test]
    fn widows_are_the_documents_own_and_none_when_it_states_none() {
        let lines_on = |widows: Option<&str>| {
            let props = widows.map_or(String::new(), |w| {
                format!(r#"<office:styles><style:default-style style:family="paragraph"><style:paragraph-properties fo:widows="{w}" fo:orphans="{w}"/></style:default-style></office:styles>"#)
            });
            // A page with room for exactly four lines of 12 pt Liberation Serif (13.8 pt each),
            // and a paragraph of five.
            let height = 4.0 * 13.8 * 25.4 / 72.0 + 0.5;
            let bytes = format!(
                r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">{props}<office:automatic-styles><style:page-layout style:name="pm"><style:page-layout-properties fo:page-width="10cm" fo:page-height="{height}mm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm"/></office:master-styles><office:body><office:text><text:p>a<text:line-break/>b<text:line-break/>c<text:line-break/>d<text:line-break/>e</text:p></office:text></office:body></office:document>"#
            );
            let app = App::new();
            app.open_bytes("w.fodt", bytes.as_bytes()).unwrap();
            let doc = typeset(&app, &setter(), &Options::default());
            doc.pages.iter().map(|p| texts(p).len()).collect::<Vec<_>>()
        };
        assert_eq!(lines_on(None), vec![4, 1]);
        assert_eq!(lines_on(Some("2")), vec![3, 2]);
    }

    /// `doc/pdf-export.md` P5: a block whose style the document declares is set as that style
    /// says; one whose style it does not declare keeps the screen's face, in the document's own
    /// default family.
    #[test]
    fn a_declared_style_sets_its_blocks_face() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:default-style style:family="paragraph"><style:text-properties fo:font-family="'Liberation Sans'" fo:font-size="12pt"/></style:default-style>
              <style:style style:name="Heading" style:family="paragraph"><style:text-properties fo:font-size="14pt"/></style:style>
              <style:style style:name="Heading_20_1" style:family="paragraph" style:parent-style-name="Heading"><style:text-properties fo:font-size="130%" fo:font-weight="bold" fo:font-family="'Liberation Mono'"/></style:style>
            </office:styles>
            <office:body><office:text>
              <text:h text:style-name="Heading_20_1" text:outline-level="1">declared</text:h>
              <text:h text:style-name="Undeclared" text:outline-level="1">undeclared</text:h>
              <text:p>plain</text:p>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("s.fodt", bytes.as_bytes()).unwrap();
        let t = setter();
        let page = &typeset(&app, &t, &Options::default()).pages[0];
        let face = |want: &str| {
            page.ops.iter().find_map(|op| match op {
                Op::Text {
                    text, face, size, ..
                } if text == want => {
                    let f = t.fonts().face(*face);
                    Some((f.family.clone(), f.bold, *size))
                }
                _ => None,
            })
        };
        assert_eq!(
            face("declared"),
            Some(("Liberation Mono".into(), true, 18.2))
        );
        assert_eq!(
            face("undeclared"),
            Some(("Liberation Sans".into(), true, 21.6)),
            "the role's scale and weight"
        );
        assert_eq!(face("plain"), Some(("Liberation Sans".into(), false, 12.0)));
    }

    /// `doc/odt-format.md` §5c facts 5 and 6, on paper: a declared style's space below and the
    /// next one's space above add, and the first paragraph's space above is applied.
    #[test]
    fn a_declared_styles_spacing_adds_as_writers_does() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:style style:name="A" style:family="paragraph"><style:paragraph-properties fo:margin-top="1cm" fo:margin-bottom="1cm"/></style:style>
              <style:style style:name="B" style:family="paragraph"><style:paragraph-properties fo:margin-top="0.5cm" fo:margin-bottom="0.5cm"/></style:style>
            </office:styles>
            <office:body><office:text><text:p text:style-name="A">one</text:p><text:p text:style-name="B">two</text:p></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("s.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let y = |want: &str| texts(page).into_iter().find(|t| t.2 == want).unwrap().1;
        let cm = 72.0 / 2.54;
        let first_baseline = 56.6929 + cm + 10.6934;
        assert!(
            (y("one") - first_baseline as f32).abs() < 0.01,
            "{}",
            y("one")
        );
        let between = y("two") - y("one");
        assert!(
            (between - (13.7988 + 1.5 * cm) as f32).abs() < 0.01,
            "{between}"
        );
    }

    /// `doc/odt-format.md` §5c fact 8, on paper: a justified paragraph's lines reach the right
    /// margin except its last, and a centred line sits in the middle.
    #[test]
    fn justified_and_centred_paragraphs_are_drawn_so() {
        let words = "word ".repeat(60);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="J" style:family="paragraph"><style:paragraph-properties fo:text-align="justify"/></style:style>
              <style:style style:name="C" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/></style:style>
            </office:automatic-styles>
            <office:body><office:text><text:p text:style-name="J">{words}</text:p><text:p text:style-name="C">middle</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("a.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        // Every line's right end: the x of its last text op plus that op's advances.
        let mut ends: Vec<(f32, f32)> = Vec::new();
        for op in &page.ops {
            if let Op::Text {
                x, y, glyphs, text, ..
            } = op
            {
                let end = x + glyphs.iter().map(|g| g.x_advance).sum::<f32>();
                let visible = text.trim_end().len() == text.len();
                match ends.last_mut() {
                    Some((line, right)) if *line == *y => {
                        *right = right.max(if visible { end } else { *right })
                    }
                    _ => ends.push((*y, end)),
                }
            }
        }
        let right_margin = (595.2756 - 56.6929) as f32;
        let justified = &ends[..ends.len() - 2];
        assert!(justified.len() >= 2, "{ends:?}");
        for (_, right) in justified {
            // The last word's own trailing space is the one thing past the margin.
            assert!(
                (right - right_margin).abs() < 4.0,
                "{right} against {right_margin}"
            );
        }
        let last = ends[ends.len() - 2].1;
        assert!(
            last < right_margin - 100.0,
            "the paragraph's last line is not stretched"
        );
        let middle = texts(page).into_iter().find(|t| t.2 == "middle").unwrap();
        let centre = (56.6929 + 595.2756 - 56.6929) / 2.0;
        assert!(
            middle.0 > 250.0 && (middle.0 as f64) < centre,
            "{}",
            middle.0
        );
    }

    /// A declared style's side margins narrow the block: it starts its left margin in from the
    /// text area, and wraps that much and its right margin sooner.
    #[test]
    fn a_declared_styles_side_margins_narrow_the_block() {
        let words = "word ".repeat(60);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles><style:style style:name="Quote" style:family="paragraph"><style:paragraph-properties fo:margin-left="2cm" fo:margin-right="3cm"/></style:style></office:styles>
            <office:body><office:text><text:p text:style-name="Quote">{words}</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("q.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let cm = 72.0 / 2.54;
        let mut right = 0.0_f32;
        for op in &page.ops {
            if let Op::Text {
                x, glyphs, text, ..
            } = op
            {
                assert!(
                    (x - (56.6929 + 2.0 * cm) as f32).abs() < 0.01 || *x > 100.0,
                    "{x}"
                );
                if !text.ends_with(' ') {
                    right = right.max(x + glyphs.iter().map(|g| g.x_advance).sum::<f32>());
                }
            }
        }
        let first = texts(page)[0].0;
        assert!(
            (first - (56.6929 + 2.0 * cm) as f32).abs() < 0.01,
            "{first}"
        );
        assert!(
            right <= (595.2756 - 56.6929 - 3.0 * cm) as f32 + 0.01,
            "{right}"
        );
    }

    /// `doc/odt-format.md` §5c fact 13, measured against LibreOffice 26.8: a tab goes to the
    /// paragraph's own stop, else the next whole `style:tab-stop-distance` — from its left
    /// margin, not the page's — and a right stop ends its text there. A header gets the same.
    #[test]
    fn tabs_go_to_the_stops_the_paragraph_style_sets() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:default-style style:family="paragraph"><style:paragraph-properties style:tab-stop-distance="1.25cm"/></style:default-style>
              <style:style style:name="Standard" style:family="paragraph"/>
              <style:style style:name="Stops" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties><style:tab-stops><style:tab-stop style:position="5cm"/><style:tab-stop style:position="17cm" style:type="right"/></style:tab-stops></style:paragraph-properties></style:style>
              <style:style style:name="Indented" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="2cm"/></style:style>
            </office:styles>
            <office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout></office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:footer><text:p text:style-name="Stops">left<text:tab/><text:tab/>Page <text:page-number/></text:p></style:footer></style:master-page></office:master-styles>
            <office:body><office:text>
              <text:p text:style-name="Standard">Longer label<text:tab/>X</text:p>
              <text:p text:style-name="Stops">Name:<text:tab/>Value<text:tab/>Right</text:p>
              <text:p text:style-name="Indented">I<text:tab/>J</text:p>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("t.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let cm = (72.0 / 2.54) as f32;
        let margin = 2.0 * cm;
        let start = |want: &str| {
            texts(page)
                .into_iter()
                .find(|t| t.2 == want)
                .unwrap_or_else(|| panic!("{want} is drawn"))
                .0
        };
        let end = |want: &str| {
            page.ops
                .iter()
                .find_map(|op| match op {
                    Op::Text {
                        x, glyphs, text, ..
                    } if text == want => Some(x + glyphs.iter().map(|g| g.x_advance).sum::<f32>()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{want} is drawn"))
        };
        let near = |got: f32, want: f32| assert!((got - want).abs() < 0.05, "{got} vs {want}");
        near(start("X"), margin + 2.0 * 1.25 * cm);
        near(start("Value"), margin + 5.0 * cm);
        near(end("Right"), margin + 17.0 * cm);
        near(start("J"), 2.0 * margin + 1.25 * cm);
        near(end("Page 1"), margin + 17.0 * cm);
    }

    /// A style's `fo:break-before="page"` starts its paragraph on a page of its own.
    #[test]
    fn a_styles_page_break_starts_a_new_page() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:break-before="page"/></style:style></office:automatic-styles>
            <office:body><office:text><text:p>one</text:p><text:p text:style-name="P1">two</text:p></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("b.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let pages: Vec<Vec<String>> = doc
            .pages
            .iter()
            .map(|page| texts(page).into_iter().map(|t| t.2).collect())
            .collect();
        assert_eq!(pages, vec![vec!["one".to_owned()], vec!["two".to_owned()]]);
    }

    /// A declared style is the whole answer: what it does not state is the default, upright and
    /// regular at the default size — never the screen's guess for the block's role, which is
    /// what made Writer's `Subtitle` print in italic.
    #[test]
    fn a_declared_style_is_complete_without_the_screens_role() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:style style:name="Subtitle" style:family="paragraph"><style:text-properties fo:font-size="18pt"/></style:style>
              <style:style style:name="Heading_20_1" style:family="paragraph"/>
            </office:styles>
            <office:body><office:text>
              <text:p text:style-name="Subtitle">sub</text:p>
              <text:h text:style-name="Heading_20_1" text:outline-level="1">head</text:h>
            </office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("c.fodt", bytes.as_bytes()).unwrap();
        let t = setter();
        let page = &typeset(&app, &t, &Options::default()).pages[0];
        let face = |want: &str| {
            page.ops.iter().find_map(|op| match op {
                Op::Text {
                    text, face, size, ..
                } if text == want => {
                    let f = t.fonts().face(*face);
                    Some((f.bold, f.italic, *size))
                }
                _ => None,
            })
        };
        assert_eq!(
            face("sub"),
            Some((false, false, 18.0)),
            "upright, as the style says nothing"
        );
        assert_eq!(
            face("head"),
            Some((false, false, 12.0)),
            "a heading style stating nothing is body text"
        );
    }

    /// A family nobody has falls back to the kind of face its declaration says it is.
    #[test]
    fn an_uninstalled_sans_family_prints_in_the_bundled_sans() {
        let bytes = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:font-face-decls><style:font-face style:name="Adwaita Sans" svg:font-family="'Adwaita Sans'" style:font-family-generic="swiss"/></office:font-face-decls>
            <office:styles><style:style style:name="Standard" style:family="paragraph"><style:text-properties style:font-name="Adwaita Sans"/></style:style></office:styles>
            <office:body><office:text><text:p text:style-name="Standard">sans</text:p></office:text></office:body></office:document>"#;
        let app = App::new();
        app.open_bytes("g.fodt", bytes.as_bytes()).unwrap();
        let t = setter();
        let page = &typeset(&app, &t, &Options::default()).pages[0];
        let family = page.ops.iter().find_map(|op| match op {
            Op::Text { face, .. } => Some(t.fonts().face(*face).family.clone()),
            _ => None,
        });
        assert_eq!(family.as_deref(), Some("Liberation Sans"));
        assert_eq!(t.substitutions()[0].used, "Liberation Sans");
    }

    /// `doc/odt-format.md` §5c fact 9: the header sits on the top margin and the footer on the
    /// bottom one, the body between them less their spacing, and the page fields are each
    /// page's own — Writer's 48 lines a page, `pg 2 of 3`.
    #[test]
    fn headers_and_footers_take_their_room_and_number_the_pages() {
        let lines: String = (1..=120)
            .map(|i| format!(r#"<text:p text:style-name="N">fill {i}</text:p>"#))
            .collect();
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>
              <style:style style:name="N" style:family="paragraph"><style:text-properties fo:font-family="'Liberation Serif'" fo:font-size="12pt"/></style:style>
              <style:style style:name="H" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/><style:text-properties fo:font-family="'Liberation Serif'" fo:font-size="12pt"/></style:style>
              <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/>
                <style:header-style><style:header-footer-properties fo:min-height="0cm" fo:margin-bottom="0.5cm"/></style:header-style>
                <style:footer-style><style:header-footer-properties fo:min-height="0cm" fo:margin-top="0.5cm"/></style:footer-style></style:page-layout>
            </office:automatic-styles>
            <office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1">
              <style:header><text:p text:style-name="N">headline</text:p></style:header>
              <style:footer><text:p text:style-name="H">pg <text:page-number>1</text:page-number> of <text:page-count>3</text:page-count></text:p></style:footer>
            </style:master-page></office:master-styles>
            <office:body><office:text>{lines}</office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("hf.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        let body: Vec<usize> = doc
            .pages
            .iter()
            .map(|page| {
                texts(page)
                    .iter()
                    .filter(|t| t.2.starts_with("fill"))
                    .count()
            })
            .collect();
        assert_eq!(body, vec![48, 48, 24]);
        let page = &doc.pages[1];
        let texts = texts(page);
        let header = texts
            .iter()
            .find(|t| t.2 == "headline")
            .expect("a header on page 2");
        assert!(
            (header.1 - (56.6929 + 10.6934) as f32).abs() < 0.01,
            "{}",
            header.1
        );
        let footer = texts
            .iter()
            .find(|t| t.2 == "pg 2 of 3")
            .expect("page 2's own footer");
        let bottom_baseline = 841.8898 - 56.6929 - 13.7988 + 10.6934;
        assert!(
            (footer.1 - bottom_baseline as f32).abs() < 0.01,
            "{}",
            footer.1
        );
        assert!(footer.0 > 250.0, "centred: {}", footer.0);
        let first = texts.iter().find(|t| t.2.starts_with("fill")).unwrap();
        let below_header = 56.6929 + 13.7988 + 72.0 / 2.54 * 0.5 + 10.6934;
        assert!((first.1 - below_header as f32).abs() < 0.01, "{}", first.1);
    }

    /// fact 10 on paper: a double-spaced paragraph's lines are 27.6 pt apart and its first line
    /// sits where a single-spaced one would.
    #[test]
    fn a_double_spaced_paragraph_doubles_its_lines() {
        let words = "word ".repeat(50);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles><style:style style:name="D" style:family="paragraph"><style:paragraph-properties fo:line-height="200%"/></style:style></office:automatic-styles>
            <office:body><office:text><text:p text:style-name="D">{words}</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("d.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let mut ys: Vec<f32> = texts(page).iter().map(|t| t.1).collect();
        ys.dedup();
        assert!(
            (ys[0] - (56.6929 + 10.6934) as f32).abs() < 0.01,
            "{}",
            ys[0]
        );
        assert!((ys[1] - ys[0] - 27.5977).abs() < 0.01, "{}", ys[1] - ys[0]);
    }

    /// fact 11 on paper: a 2 cm first-line indent starts the first line 2 cm in and the rest at
    /// the margin.
    #[test]
    fn a_first_line_indent_moves_the_first_line_alone() {
        let words = "word ".repeat(50);
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles><style:style style:name="I" style:family="paragraph"><style:paragraph-properties fo:text-indent="2cm"/></style:style></office:automatic-styles>
            <office:body><office:text><text:p text:style-name="I">{words}</text:p></office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("i.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let texts = texts(page);
        let first_line = texts[0].1;
        let first_x = texts
            .iter()
            .filter(|t| t.1 == first_line)
            .map(|t| t.0)
            .fold(f32::MAX, f32::min);
        let second_x = texts
            .iter()
            .filter(|t| t.1 > first_line)
            .map(|t| t.0)
            .fold(f32::MAX, f32::min);
        assert!(
            (first_x - (56.6929 + 72.0 / 2.54 * 2.0) as f32).abs() < 0.01,
            "{first_x}"
        );
        assert!((second_x - 56.6929).abs() < 0.01, "{second_x}");
    }

    /// A paragraph style's colour colours its text where the run states none, and a link with no
    /// colour of its own is drawn as Writer draws an unstyled one: navy and underlined.
    #[test]
    fn a_styles_colour_and_a_links_look_are_drawn() {
        let bytes = r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles><style:style style:name="Teal" style:family="paragraph"><style:text-properties fo:color="#174a5b"/></style:style></office:styles>
            <office:body><office:text>
              <text:p text:style-name="Teal">teal</text:p>
              <text:p>see <text:a xlink:href="https://example.invalid/">there</text:a></text:p>
            </office:text></office:body></office:document>"##;
        let app = App::new();
        app.open_bytes("c.fodt", bytes.as_bytes()).unwrap();
        let page = &typeset(&app, &setter(), &Options::default()).pages[0];
        let colour = |want: &str| {
            page.ops.iter().find_map(|op| match op {
                Op::Text { text, color, .. } if text == want => Some(*color),
                _ => None,
            })
        };
        assert_eq!(colour("teal"), Some(Rgb(0x17, 0x4a, 0x5b)));
        assert_eq!(colour("there"), Some(Rgb(0, 0, 0x80)));
        assert!(page.ops.iter().any(|op| matches!(
            op,
            Op::Line {
                color: Rgb(0, 0, 0x80),
                ..
            }
        )));
        assert_eq!(colour("see "), Some(Rgb::BLACK));
    }

    /// A table drawn as its document says: its own column widths, each cell's fill, borders and
    /// padding, and its heading row again at the top of every page it continues on.
    #[test]
    fn a_table_prints_its_own_look_and_repeats_its_heading() {
        let rows: String = (1..=80)
            .map(|i| format!(r#"<table:table-row><table:table-cell table:style-name="Body"><text:p>row {i}</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row>"#))
            .collect();
        let bytes = format!(
            r##"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:styles>
              <style:style style:name="Head" style:family="table-cell"><style:table-cell-properties fo:border="0.5pt solid #174a5b" fo:padding="0.07in" fo:background-color="#287271"/></style:style>
              <style:style style:name="Body" style:family="table-cell"><style:table-cell-properties fo:border="0.5pt solid #8da5a5" fo:padding="0.06in"/></style:style>
            </office:styles>
            <office:automatic-styles><style:style style:name="W" style:family="table-column"><style:table-column-properties style:column-width="2in"/></style:style></office:automatic-styles>
            <office:body><office:text><table:table table:name="T"><table:table-column table:style-name="W"/><table:table-column/>
              <table:table-header-rows><table:table-row><table:table-cell table:style-name="Head"><text:p>Case</text:p></table:table-cell><table:table-cell table:style-name="Head"><text:p>Notes</text:p></table:table-cell></table:table-row></table:table-header-rows>
              {rows}
            </table:table></office:text></office:body></office:document>"##
        );
        let app = App::new();
        app.open_bytes("t.fodt", bytes.as_bytes()).unwrap();
        let doc = typeset(&app, &setter(), &Options::default());
        assert!(doc.pages.len() >= 2);
        for page in &doc.pages {
            let first = texts(page)
                .into_iter()
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert_eq!(first.2, "Case", "every page starts with the heading");
        }
        let page = &doc.pages[0];
        assert!(
            page.ops.iter().any(|op| matches!(
                op,
                Op::Rect {
                    color: Rgb(0x28, 0x72, 0x71),
                    ..
                }
            )),
            "the heading's fill"
        );
        assert!(
            page.ops.iter().any(|op| matches!(
                op,
                Op::Line {
                    color: Rgb(0x8d, 0xa5, 0xa5),
                    ..
                }
            )),
            "a body cell's border"
        );
        // The second column starts where the first's two inches end, plus its own padding.
        let notes = texts(page).into_iter().find(|t| t.2 == "Notes").unwrap();
        let expected = 56.6929 + 144.0 + 0.07 * 72.0;
        assert!((notes.0 - expected as f32).abs() < 0.01, "{}", notes.0);
        // The repeated heading is decoration, not text read twice.
        let repeated = doc.pages[1].ops.iter().find_map(|op| match op {
            Op::Text { text, mark, .. } if text == "Case" => Some(*mark),
            _ => None,
        });
        assert_eq!(repeated, Some(crate::ops::Mark::Decoration));
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
