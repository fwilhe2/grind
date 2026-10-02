// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A frame of the grid as a list of things to draw — every decision about what goes where and
//! in which colour, with **no CoreGraphics in it**.
//!
//! `ui_win32`'s split, taken one step further. There the portable half decides what a cell looks
//! like and the GDI half still walks the viewport; here the walk is portable too, and what comes
//! out is a list of [`Op`]s — a filled rectangle or a line of text at a place — that `render.rs`
//! executes into a `CGContext`, whether that context is a view's `drawRect:` or `--render-to`'s
//! bitmap (decision 9: one drawing path, two callers). So the arithmetic a user would notice is
//! wrong — where a number sits, when it turns into `###`, which rows a hidden one closes up — is
//! tested on Linux against [`grind_core::layout::Fixed`], and the Mac half has nothing left to
//! decide.
//!
//! Text is measured through [`grind_core::layout::Metrics`], the trait the page's line breaking
//! uses, so the width that decides `###` and the width CoreText draws are one engine's answer
//! (`metrics.rs`, decision 4).
//!
//! **What it does not draw**, each a named gap in `doc/macos-shell.md`: a label spilling into the
//! empty cells beside it (it is clipped to its own cell), borders, and the view-mode overlays
//! (M8).

use grind_core::color::{self, Rgb};
use grind_core::layout::Metrics;
use grind_core::style::TextStyle;
use grind_sheet::nav::Selection;
use grind_sheet::style::{CellStyle, EDGES, border_parts};
use grind_sheet::view::{CellRole, Hue, NameAnchor, Overlays};
use grind_sheet::{App, Pos, look, numfmt};

use super::geom::{self, Grid, HEADER_H, HEADER_W, Rect};
pub use crate::ops::Op;

/// How far a cell's text stands off its edges, across and down.
pub const PAD_X: f64 = 4.0;
pub const PAD_Y: f64 = 2.0;

/// How thick the selection's outline is.
pub const OUTLINE: f64 = 2.0;

/// The colours a frame is drawn in, already resolved.
///
/// The live view fills this from `NSColor`'s semantic colours in the view's own appearance at
/// draw time (decision 8), so dark mode, high contrast and the user's accent follow the system;
/// a render fills it the same way under a named appearance. [`Palette::LIGHT`] and
/// [`Palette::DARK`] are fixed stand-ins for tests and for a host with no AppKit to ask.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The sheet's own ground — `textBackgroundColor`.
    pub page: Rgb,
    /// Text the document gave no colour — `textColor`.
    pub ink: Rgb,
    /// The lines between cells — `gridColor`.
    pub grid: Rgb,
    /// The header bands' ground and their labels — `controlBackgroundColor`,
    /// `secondaryLabelColor`.
    pub header: Rgb,
    pub header_ink: Rgb,
    /// The selection's outline and wash — `controlAccentColor`, the user's own accent.
    pub accent: Rgb,
    /// Whether the page is dark, which is what lifts a document's own colours along their hue
    /// (`grind_core::color::document_ink`).
    pub dark: bool,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        page: (0xff, 0xff, 0xff),
        ink: (0x00, 0x00, 0x00),
        grid: (0xe6, 0xe6, 0xe6),
        header: (0xf5, 0xf5, 0xf5),
        header_ink: (0x6e, 0x6e, 0x73),
        accent: (0x00, 0x7a, 0xff),
        dark: false,
    };
    pub const DARK: Palette = Palette {
        page: (0x1e, 0x1e, 0x1e),
        ink: (0xff, 0xff, 0xff),
        grid: (0x38, 0x38, 0x38),
        header: (0x2a, 0x2a, 0x2a),
        header_ink: (0x98, 0x98, 0x9d),
        accent: (0x0a, 0x84, 0xff),
        dark: true,
    };
}

/// What a frame is drawn with: the colours, the fonts' measurements, and how thin a line is — one
/// device pixel in points, half a point on a Retina screen.
pub struct Look<'a> {
    pub palette: &'a Palette,
    pub metrics: &'a dyn Metrics,
    pub hairline: f64,
    /// Which of `doc/view-modes.md`'s overlays are on — View ▸ Cell Roles and Names (M8).
    pub overlays: Overlays,
    /// View ▸ Formulas: a cell holding a formula shows it, in display syntax, rather than what it
    /// came to — `grind sheet view --formulas`.
    pub formulas: bool,
}

/// A `Rect` kept only when there is something of it left inside `view`.
fn within(rect: Rect, view: &Rect) -> Option<Rect> {
    let rect = rect.intersection(view);
    (!rect.is_empty()).then_some(rect)
}

/// The rectangle a selection covers on the grid, clamped to the shown part of the sheet — a whole
/// column is a million rows, and the grid stops long before that.
fn selection_rect(grid: &Grid, selection: Selection) -> Rect {
    let (start, end) = selection.rect();
    let end = Pos::new(
        end.row.min(grid.shown_rows.saturating_sub(1)),
        end.col.min(grid.shown_cols.saturating_sub(1)),
    );
    let first = grid.cell(start.row, start.col);
    let last = grid.cell(end.row, end.col);
    Rect::new(
        first.x,
        first.y,
        last.right() - first.x,
        last.bottom() - first.y,
    )
}

/// The selection's wash: the whole range **but the active cell**, which is left out so it reads as
/// the cell the cursor is in rather than one more selected cell — the GNOME window's rule and the
/// Windows pane's. A single cell has no wash at all.
fn selection_wash(grid: &Grid, view: &Rect, selection: Selection, color: Rgb) -> Vec<Op> {
    if selection.is_single() {
        return Vec::new();
    }
    let range = selection_rect(grid, selection);
    let active = grid.cell(selection.active.row, selection.active.col);
    [
        Rect::new(range.x, range.y, range.w, active.y - range.y),
        Rect::new(
            range.x,
            active.bottom(),
            range.w,
            range.bottom() - active.bottom(),
        ),
        Rect::new(range.x, active.y, active.x - range.x, active.h),
        Rect::new(
            active.right(),
            active.y,
            range.right() - active.right(),
            active.h,
        ),
    ]
    .into_iter()
    .filter_map(|rect| within(rect, view))
    .map(|rect| Op::Wash { rect, color })
    .collect()
}

/// The selection's outline, [`OUTLINE`] thick and straddling the range's edge.
pub fn selection_outline(grid: &Grid, view: &Rect, selection: Selection, color: Rgb) -> Vec<Op> {
    let range = selection_rect(grid, selection);
    let half = OUTLINE / 2.0;
    [
        Rect::new(range.x - half, range.y - half, range.w + OUTLINE, OUTLINE),
        Rect::new(
            range.x - half,
            range.bottom() - half,
            range.w + OUTLINE,
            OUTLINE,
        ),
        Rect::new(range.x - half, range.y - half, OUTLINE, range.h + OUTLINE),
        Rect::new(
            range.right() - half,
            range.y - half,
            OUTLINE,
            range.h + OUTLINE,
        ),
    ]
    .into_iter()
    .filter_map(|rect| within(rect, view))
    .map(|rect| Op::Fill { rect, color })
    .collect()
}

/// How wide `text` is set in `style` — the last cumulative advance.
pub fn width(metrics: &dyn Metrics, text: &str, style: &TextStyle) -> f64 {
    let mut advances = Vec::new();
    metrics.advances(text, style, &mut advances);
    advances.last().copied().map_or(0.0, f64::from)
}

/// The cells of `sheet` that meet `view` — a rectangle in the sheet's own coordinates, as a
/// view's dirty rectangle is — in those same coordinates.
///
/// Grounds first, then the grid lines, then the selection's wash, then the text, then the
/// selection's outline — so a document's fill never covers a line, a line never crosses a word,
/// and the outline is over everything it frames.
pub fn cells(
    app: &App,
    sheet: usize,
    grid: &Grid,
    view: Rect,
    selection: Selection,
    look: &Look,
) -> Vec<Op> {
    let (palette, metrics, hairline) = (look.palette, look.metrics, look.hairline);
    let mut ops = vec![Op::Fill {
        rect: view,
        color: palette.page,
    }];
    let rows = grid.rows_in(view.y, view.h);
    let cols = grid.cols_in(view.x, view.w);
    let Ok(viewport) = app.get_viewport_with(sheet, rows.clone(), cols.clone(), look.overlays)
    else {
        return ops;
    };

    // Grounds the document chose. `transparent` is a real value and means no fill.
    for row in rows.clone() {
        for col in cols.clone() {
            let fill = viewport
                .style(row, col)
                .and_then(|style| style.background.as_deref())
                .and_then(color::parse);
            if let Some(fill) = fill {
                let rect = grid.cell(row, col).intersection(&view);
                if !rect.is_empty() {
                    ops.push(Op::Fill { rect, color: fill });
                }
            }
        }
    }

    // The grid lines, along each shown track's far edge. A hidden track has no edge of its own:
    // its neighbour's line is the one line where it was.
    for col in cols.clone() {
        let cell = grid.cell(0, col);
        if cell.w > 0.0 {
            ops.push(Op::Fill {
                rect: Rect::new(cell.right() - hairline, view.y, hairline, view.h),
                color: palette.grid,
            });
        }
    }
    for row in rows.clone() {
        let cell = grid.cell(row, 0);
        if cell.h > 0.0 {
            ops.push(Op::Fill {
                rect: Rect::new(view.x, cell.bottom() - hairline, view.w, hairline),
                color: palette.grid,
            });
        }
    }

    // The borders the document drew, over the grid lines they replace.
    for row in rows.clone() {
        for col in cols.clone() {
            if let Some(style) = viewport.style(row, col) {
                ops.extend(borders(grid.cell(row, col), style, palette, hairline));
            }
        }
    }

    ops.extend(selection_wash(grid, &view, selection, palette.accent));

    // The text.
    for row in rows {
        for col in cols.clone() {
            // With formulas shown, a formula's own text — set as text, so it reads from the left.
            let formula = look
                .formulas
                .then(|| app.formula(sheet, Pos::new(row, col)).ok().flatten())
                .flatten()
                .and_then(|_| app.input_text(sheet, Pos::new(row, col)).ok());
            let Some(text) = formula
                .as_deref()
                .or_else(|| viewport.text(row, col))
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            let mut cell = grid.cell(row, col);
            if cell.is_empty() {
                continue;
            }
            // The role overlay reserves a margin at the cell's leading edge for its marker,
            // rather than drawing over what the cell shows — a label is left-aligned text and
            // the two would collide. `role` is `None` whenever the read did not ask for roles.
            let role = viewport
                .role(row, col)
                .filter(|role| !role.marker().is_empty());
            if let (Some(role), Some(ink)) = (role, role.and_then(|role| role_color(role, palette)))
            {
                ops.push(marker(role, cell, ink, metrics));
                cell = Rect::new(
                    cell.x + MARKER_W,
                    cell.y,
                    (cell.w - MARKER_W).max(0.0),
                    cell.h,
                );
            }
            let value = match &formula {
                Some(formula) => grind_sheet::CellValue::Text(formula.clone()),
                None => viewport.get(row, col).cloned().unwrap_or_default(),
            };
            let style = viewport.style(row, col);
            let fill = style
                .and_then(|style| style.background.as_deref())
                .and_then(color::parse);
            let ink = color::document_ink(
                style
                    .and_then(|style| style.color.as_deref())
                    .and_then(color::parse),
                fill,
                palette.page,
                palette.ink,
                palette.dark,
            );
            match look::wraps(style) && !numfmt::is_number(&value) {
                true => ops.extend(wrapped_text(text, &value, style, cell, ink, metrics)),
                false => ops.push(cell_text(one_line(text), &value, style, cell, ink, metrics)),
            }
        }
    }
    ops.extend(name_outlines(grid, &view, viewport.names(), palette));
    ops.extend(selection_outline(grid, &view, selection, palette.accent));
    ops
}

/// A cell's own borders, each centred on the grid line it stands on — so a cell's right border
/// and its neighbour's left one are the same line, and either covers the hairline under it.
///
/// A border is ODF's three parts (`0.5pt solid #000000`, `grind_sheet::style::border_parts`):
/// never thinner than one device pixel, in the document's colour lifted to read on a dark page,
/// and `double` as two thin lines with a gap between. `none` and `hidden` draw nothing, and
/// neither does a border this build cannot read — R5's tolerance.
///
/// ponytail: `dashed` and `dotted` are drawn solid. The trigger is a document whose dashes are
/// what distinguishes two of its tables; a dash is a run of fills along the edge.
pub fn borders(cell: Rect, style: &CellStyle, palette: &Palette, hairline: f64) -> Vec<Op> {
    let mut ops = Vec::new();
    if cell.is_empty() {
        return ops;
    }
    for (edge, border) in style.borders.iter().enumerate() {
        let Some((points, line, ink)) = border.as_deref().and_then(border_parts) else {
            continue;
        };
        if matches!(line, "none" | "hidden") || points <= 0.0 {
            continue;
        }
        let Some(own) = color::parse(ink) else {
            continue;
        };
        let color = color::document_ink(Some(own), None, palette.page, own, palette.dark);
        let width = points.max(hairline);
        // The line the edge stands on, where the grid's own hairline is drawn.
        let at = match EDGES[edge] {
            "left" => cell.x,
            "right" => cell.right(),
            "top" => cell.y,
            _ => cell.bottom(),
        } - hairline / 2.0;
        let vertical = matches!(EDGES[edge], "left" | "right");
        // One line, or for `double` two of a third of the width each, either side of a gap.
        let strokes: Vec<(f64, f64)> = match line {
            "double" => {
                let thin = (width / 3.0).max(hairline);
                let gap = thin.max(hairline);
                vec![(at - gap / 2.0 - thin, thin), (at + gap / 2.0, thin)]
            }
            _ => vec![(at - width / 2.0, width)],
        };
        for (from, thickness) in strokes {
            let rect = match vertical {
                true => Rect::new(from, cell.y, thickness, cell.h),
                false => Rect::new(cell.x, from, cell.w, thickness),
            };
            ops.push(Op::Fill { rect, color });
        }
    }
    ops
}

/// How wide the margin is that the role overlay reserves at a cell's leading edge.
pub const MARKER_W: f64 = 12.0;

/// `a` moved `by` of the way towards `b`.
fn mix(a: Rgb, b: Rgb, by: f64) -> Rgb {
    let one = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * by).round() as u8;
    (one(a.0, b.0), one(a.1, b.1), one(a.2, b.2))
}

/// The colour a role's marker is drawn in: `CellRole::hue`, with a palette colour lifted to be
/// legible on a dark page the way a document's own colour is (`color::document_ink`), the ink
/// as the page's own, and a quieter ink for a label.
pub fn role_color(role: CellRole, palette: &Palette) -> Option<Rgb> {
    match role.hue() {
        Hue::None => None,
        Hue::Palette(name) => grind_core::style::palette(name)
            .and_then(color::parse)
            .map(|hue| {
                color::document_ink(Some(hue), None, palette.page, palette.ink, palette.dark)
            }),
        Hue::Ink => Some(palette.ink),
        Hue::Quiet => Some(mix(palette.ink, palette.page, 0.4)),
    }
}

/// A role's marker in the margin it reserves — `CellRole::marker`, the core's glyph, so no shell
/// invents a table of its own; a glyph ships with every colour, since a mode whose whole output
/// is colour excludes anyone who cannot tell colours apart.
fn marker(role: CellRole, cell: Rect, ink: Rgb, metrics: &dyn Metrics) -> Op {
    let style = header_style();
    let line_h = f64::from(metrics.line_height(&style));
    Op::Text {
        x: cell.x + 2.0,
        top: cell.y + (cell.h - line_h) / 2.0,
        text: role.marker().to_owned(),
        style,
        color: ink,
        clip: Rect::new(cell.x, cell.y, MARKER_W.min(cell.w), cell.h),
    }
}

/// The name overlay: an outline round every defined name's range, in the ink moved most of the
/// way to the page — a label on the document's own structure, quieter than the selection, which
/// is a thing to act on. `names` is empty whenever the read did not ask for them.
fn name_outlines(grid: &Grid, view: &Rect, names: &[NameAnchor], palette: &Palette) -> Vec<Op> {
    let muted = mix(palette.ink, palette.page, 0.55);
    let mut ops = Vec::new();
    for anchor in names {
        let (Some(last_row), Some(last_col)) = (
            anchor.rows.end.checked_sub(1),
            anchor.cols.end.checked_sub(1),
        ) else {
            continue;
        };
        let first = grid.cell(anchor.rows.start, anchor.cols.start);
        let last = grid.cell(last_row, last_col);
        let (l, t, r, b) = (first.x, first.y, last.right(), last.bottom());
        for rect in [
            Rect::new(l, t, r - l, 1.0),
            Rect::new(l, b - 1.0, r - l, 1.0),
            Rect::new(l, t, 1.0, b - t),
            Rect::new(r - 1.0, t, 1.0, b - t),
        ] {
            if let Some(rect) = within(rect, view) {
                ops.push(Op::Fill { rect, color: muted });
            }
        }
    }
    ops
}

/// One cell's text, placed: aligned by `grind_sheet::look`, and a number too wide for its column
/// drawn as `numfmt::overflow`'s hashes rather than as part of itself.
fn cell_text(
    text: String,
    value: &grind_sheet::model::CellValue,
    style: Option<&grind_sheet::style::CellStyle>,
    cell: Rect,
    ink: Rgb,
    metrics: &dyn Metrics,
) -> Op {
    let text_style = look::text_style(style);
    let room = (cell.w - 2.0 * PAD_X).max(0.0);
    let mut text = text;
    let mut text_w = width(metrics, &text, &text_style);
    if numfmt::is_number(value) && text_w > room {
        text = numfmt::overflow(room, width(metrics, "#", &text_style));
        text_w = width(metrics, &text, &text_style);
    }
    let line_h = f64::from(metrics.line_height(&text_style));
    let x = match look::align(value, style) {
        look::Align::Left => cell.x + PAD_X,
        look::Align::Center => cell.x + (cell.w - text_w) / 2.0,
        look::Align::Right => cell.right() - PAD_X - text_w,
    };
    let top = match look::valign(style) {
        look::VAlign::Top => cell.y + PAD_Y,
        look::VAlign::Middle => cell.y + (cell.h - line_h) / 2.0,
        look::VAlign::Bottom => cell.bottom() - PAD_Y - line_h,
    };
    Op::Text {
        x,
        top,
        text,
        style: text_style,
        color: ink,
        clip: cell,
    }
}

/// A wrapping cell's text, broken at its column's width by `grind_core::layout::wrap` — the
/// breaker the page and the GNOME window's row heights use — one line an op, aligned across as a
/// line is and the block of lines placed down the cell as one line would be. What does not fit
/// the row's height is cut by the cell, since the row is as tall as the document says.
///
/// ponytail: a row with no height of its own is not grown to fit (the GNOME window's L3 does);
/// the trigger is a wrapped cell somebody cannot read, and the upgrade is `Grid` asking this
/// same breaker for a row's height.
fn wrapped_text(
    text: &str,
    value: &grind_sheet::model::CellValue,
    style: Option<&grind_sheet::style::CellStyle>,
    cell: Rect,
    ink: Rgb,
    metrics: &dyn Metrics,
) -> Vec<Op> {
    let text_style = look::text_style(style);
    let room = (cell.w - 2.0 * PAD_X).max(1.0);
    let layout = grind_core::layout::wrap(
        &[grind_core::layout::Fragment {
            text,
            style: &text_style,
        }],
        room as f32,
        metrics,
    );
    let block_h = f64::from(layout.height());
    let top = match look::valign(style) {
        look::VAlign::Top => cell.y + PAD_Y,
        look::VAlign::Middle => cell.y + (cell.h - block_h) / 2.0,
        look::VAlign::Bottom => cell.bottom() - PAD_Y - block_h,
    };
    layout
        .lines()
        .iter()
        .map(|line| {
            let piece: String = text
                .chars()
                .skip(line.start)
                .take(line.end.saturating_sub(line.start))
                .collect::<String>()
                .trim_end()
                .to_owned();
            let piece = one_line(&piece);
            let piece_w = width(metrics, &piece, &text_style);
            let x = match look::align(value, style) {
                look::Align::Left => cell.x + PAD_X,
                look::Align::Center => cell.x + (cell.w - piece_w) / 2.0,
                look::Align::Right => cell.right() - PAD_X - piece_w,
            };
            Op::Text {
                x,
                top: top + f64::from(line.top),
                text: piece,
                style: text_style.clone(),
                color: ink,
                clip: cell,
            }
        })
        .collect()
}

/// A cell's text as the one line a grid draws: a line break the document kept (`text:line-break`
/// is a `\n` in the model) or any other control character becomes a space, so the two sides stay
/// two words. Where a second line would *go* is row auto-height's question, and a named gap here.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The size a header label is set in — smaller than a cell's, the way every spreadsheet's are.
fn header_style() -> TextStyle {
    TextStyle {
        font_size: Some("9pt".to_owned()),
        ..TextStyle::default()
    }
}

/// The column header band for the part of the sheet `x..x + w`, `x` in the sheet's coordinates
/// and the band [`HEADER_H`] tall from zero.
///
/// The selection's columns are tinted, so where the cursor is can be read off the band.
pub fn column_header(grid: &Grid, x: f64, w: f64, selection: Selection, look: &Look) -> Vec<Op> {
    let (palette, metrics, hairline) = (look.palette, look.metrics, look.hairline);
    let (start, end) = selection.rect();
    let band = Rect::new(x, 0.0, w, HEADER_H);
    let mut ops = vec![
        Op::Fill {
            rect: band,
            color: palette.header,
        },
        Op::Fill {
            rect: Rect::new(x, HEADER_H - hairline, w, hairline),
            color: palette.grid,
        },
    ];
    let style = header_style();
    let line_h = f64::from(metrics.line_height(&style));
    for col in grid.cols_in(x, w) {
        let cell = grid.cell(0, col);
        if cell.w <= 0.0 {
            continue;
        }
        let label = geom::column_label(col);
        let label_w = width(metrics, &label, &style);
        let clip = Rect::new(cell.x, 0.0, cell.w, HEADER_H);
        if (start.col..=end.col).contains(&col) {
            ops.push(Op::Wash {
                rect: clip,
                color: palette.accent,
            });
        }
        ops.push(Op::Fill {
            rect: Rect::new(cell.right() - hairline, 0.0, hairline, HEADER_H),
            color: palette.grid,
        });
        ops.push(Op::Text {
            x: cell.x + (cell.w - label_w) / 2.0,
            top: (HEADER_H - line_h) / 2.0,
            text: label,
            style: style.clone(),
            color: palette.header_ink,
            clip,
        });
    }
    ops
}

/// The row header band for the part of the sheet `y..y + h`, [`HEADER_W`] wide from zero.
///
/// The selection's rows are tinted, as the columns are.
pub fn row_header(grid: &Grid, y: f64, h: f64, selection: Selection, look: &Look) -> Vec<Op> {
    let (palette, metrics, hairline) = (look.palette, look.metrics, look.hairline);
    let (start, end) = selection.rect();
    let band = Rect::new(0.0, y, HEADER_W, h);
    let mut ops = vec![
        Op::Fill {
            rect: band,
            color: palette.header,
        },
        Op::Fill {
            rect: Rect::new(HEADER_W - hairline, y, hairline, h),
            color: palette.grid,
        },
    ];
    let style = header_style();
    let line_h = f64::from(metrics.line_height(&style));
    for row in grid.rows_in(y, h) {
        let cell = grid.cell(row, 0);
        if cell.h <= 0.0 {
            continue;
        }
        let label = geom::row_label(row);
        let label_w = width(metrics, &label, &style);
        if (start.row..=end.row).contains(&row) {
            ops.push(Op::Wash {
                rect: Rect::new(0.0, cell.y, HEADER_W, cell.h),
                color: palette.accent,
            });
        }
        ops.push(Op::Fill {
            rect: Rect::new(0.0, cell.bottom() - hairline, HEADER_W, hairline),
            color: palette.grid,
        });
        ops.push(Op::Text {
            x: HEADER_W - PAD_X - label_w,
            top: cell.y + (cell.h - line_h) / 2.0,
            text: label,
            style: style.clone(),
            color: palette.header_ink,
            clip: Rect::new(0.0, cell.y, HEADER_W, cell.h),
        });
    }
    ops
}

/// A whole frame `width` by `height`, scrolled to `(scroll_x, scroll_y)` — the corner, both
/// header bands and the cells, placed where the window has them. What `--render-to` draws.
pub fn frame(
    app: &App,
    sheet: usize,
    grid: &Grid,
    (width, height): (f64, f64),
    (scroll_x, scroll_y): (f64, f64),
    selection: Selection,
    look: &Look,
) -> Vec<Op> {
    let (palette, hairline) = (look.palette, look.hairline);
    let body_w = (width - HEADER_W).max(0.0);
    let body_h = (height - HEADER_H).max(0.0);
    let body = Rect::new(HEADER_W, HEADER_H, body_w, body_h);
    let top = Rect::new(HEADER_W, 0.0, body_w, HEADER_H);
    let left = Rect::new(0.0, HEADER_H, HEADER_W, body_h);

    let mut ops = Vec::new();
    let view = Rect::new(scroll_x, scroll_y, body_w, body_h);
    let (dx, dy) = (HEADER_W - scroll_x, HEADER_H - scroll_y);
    let charts = super::chart::charts(app, sheet, &view, palette, look.metrics, None);
    for op in cells(app, sheet, grid, view, selection, look)
        .into_iter()
        .chain(charts)
    {
        ops.extend(op.placed(dx, dy, &body));
    }
    for op in column_header(grid, scroll_x, body_w, selection, look) {
        ops.extend(op.placed(dx, 0.0, &top));
    }
    for op in row_header(grid, scroll_y, body_h, selection, look) {
        ops.extend(op.placed(0.0, dy, &left));
    }
    // The corner, over the two bands' ends, with the lines that finish both.
    ops.push(Op::Fill {
        rect: Rect::new(0.0, 0.0, HEADER_W, HEADER_H),
        color: palette.header,
    });
    ops.push(Op::Fill {
        rect: Rect::new(0.0, HEADER_H - hairline, HEADER_W, hairline),
        color: palette.grid,
    });
    ops.push(Op::Fill {
        rect: Rect::new(HEADER_W - hairline, 0.0, hairline, HEADER_H),
        color: palette.grid,
    });
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::layout::Fixed;
    use grind_sheet::style::CellStyle;
    use grind_sheet::{Pos, RecalcMode};

    /// `Fixed` is one unit per character and one unit tall — so a sheet measured with it has
    /// every width assertable, if tiny.
    const HAIR: f64 = 0.5;

    fn texts(ops: &[Op]) -> Vec<(&str, f64, f64)> {
        ops.iter()
            .filter_map(|op| match op {
                Op::Text { text, x, top, .. } => Some((text.as_str(), *x, *top)),
                Op::Fill { .. }
                | Op::Wash { .. }
                | Op::Run { .. }
                | Op::Image { .. }
                | Op::Path { .. } => None,
            })
            .collect()
    }

    fn sheet() -> App {
        let app = App::new();
        for (row, col, input) in [
            (0, 0, "Rent"),
            (0, 1, "1250"),
            (1, 1, "=[.B1]*2"),
            (2, 0, "TRUE"),
        ] {
            app.enter(0, Pos::new(row, col), input, RecalcMode::Document)
                .unwrap();
        }
        app
    }

    const LOOK: Look = Look {
        palette: &Palette::LIGHT,
        metrics: &Fixed,
        hairline: HAIR,
        overlays: Overlays::NONE,
        formulas: false,
    };

    fn drawn(app: &App) -> Vec<Op> {
        drawn_with(app, Selection::default())
    }

    fn drawn_with(app: &App, selection: Selection) -> Vec<Op> {
        let grid = Grid::of(app, 0);
        let view = Rect::new(0.0, 0.0, 500.0, 200.0);
        cells(app, 0, &grid, view, selection, &LOOK)
    }

    fn washes(ops: &[Op]) -> Vec<Rect> {
        ops.iter()
            .filter_map(|op| match op {
                Op::Wash { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_label_sits_left_a_number_right_and_a_boolean_in_the_middle() {
        let ops = drawn(&sheet());
        let texts = texts(&ops);
        let find = |label: &str| *texts.iter().find(|(t, ..)| *t == label).unwrap();
        assert_eq!(find("Rent").1, PAD_X, "left, inside the padding");
        let (_, x, _) = find("1250");
        assert_eq!(
            x,
            2.0 * geom::COL_W - PAD_X - 4.0,
            "right: four characters wide"
        );
        let (_, x, _) = find("TRUE");
        assert_eq!(x, (geom::COL_W - 4.0) / 2.0, "centred");
        assert!(
            texts.iter().any(|(t, ..)| *t == "2500"),
            "a formula draws its value"
        );
    }

    #[test]
    fn text_sits_in_the_middle_of_its_row_unless_the_cell_says() {
        let app = sheet();
        let ops = drawn(&app);
        let (_, _, top) = *texts(&ops).iter().find(|(t, ..)| *t == "Rent").unwrap();
        assert_eq!(top, (geom::ROW_H - 1.0) / 2.0, "Fixed is one unit tall");
        app.set_style(
            0,
            Pos::new(0, 0),
            Pos::new(0, 0),
            Some(CellStyle {
                vertical_align: Some("top".into()),
                ..CellStyle::default()
            }),
        )
        .unwrap();
        let (_, _, top) = *texts(&drawn(&app))
            .iter()
            .find(|(t, ..)| *t == "Rent")
            .unwrap();
        assert_eq!(top, PAD_Y);
    }

    /// A number that does not fit is `###`, never part of itself; text is only clipped.
    #[test]
    fn a_number_too_wide_for_its_column_is_hashes() {
        let app = sheet();
        app.set_col_width(0, 1..2, Some("0.1in".into())).unwrap();
        app.set_col_width(0, 0..1, Some("0.1in".into())).unwrap();
        let ops = drawn(&app);
        let texts = texts(&ops);
        let room = (7.2 - 2.0 * PAD_X).max(0.0);
        assert!(
            texts
                .iter()
                .any(|(t, ..)| *t == numfmt::overflow(room, 1.0)),
            "{texts:?}"
        );
        assert!(
            !texts.iter().any(|(t, ..)| *t == "1250"),
            "never a piece of it"
        );
        assert!(
            texts.iter().any(|(t, ..)| *t == "Rent"),
            "text keeps its words"
        );
    }

    #[test]
    fn a_documents_fill_is_drawn_and_its_ink_reads_on_it() {
        let app = sheet();
        app.set_style(
            0,
            Pos::new(0, 0),
            Pos::new(0, 0),
            Some(CellStyle {
                background: Some("#001f3f".into()),
                ..CellStyle::default()
            }),
        )
        .unwrap();
        let ops = drawn(&app);
        assert!(ops.iter().any(|op| matches!(
            op,
            Op::Fill { color: (0x00, 0x1f, 0x3f), rect } if rect.x == 0.0 && rect.y == 0.0
        )));
        let ink = ops.iter().find_map(|op| match op {
            Op::Text { text, color, .. } if text == "Rent" => Some(*color),
            _ => None,
        });
        assert_eq!(ink, Some((0xff, 0xff, 0xff)), "automatic: white on navy");
    }

    /// A hidden row draws nothing and has no line of its own.
    #[test]
    fn a_hidden_row_closes_up() {
        let app = sheet();
        app.set_row_hidden(0, 1..2, true).unwrap();
        let ops = drawn(&app);
        assert!(
            !texts(&ops).iter().any(|(t, ..)| *t == "2500"),
            "row 2 is hidden"
        );
        let (_, _, top) = *texts(&ops).iter().find(|(t, ..)| *t == "TRUE").unwrap();
        assert_eq!(
            top,
            geom::ROW_H + (geom::ROW_H - 1.0) / 2.0,
            "row 3 moved up"
        );
    }

    #[test]
    fn the_headers_are_labelled_and_centred_on_their_tracks() {
        let grid = Grid::of(&sheet(), 0);
        let ops = column_header(&grid, 0.0, 2.0 * geom::COL_W, Selection::default(), &LOOK);
        let labels: Vec<&str> = texts(&ops).iter().map(|(t, ..)| *t).collect();
        assert_eq!(labels, ["A", "B"]);
        let ops = row_header(&grid, 0.0, 2.0 * geom::ROW_H, Selection::default(), &LOOK);
        let labels: Vec<&str> = texts(&ops).iter().map(|(t, ..)| *t).collect();
        assert_eq!(labels, ["1", "2"]);
    }

    /// The frame puts the cells under and beside the two bands, and nothing of the cells spills
    /// into them.
    #[test]
    fn a_frame_places_the_cells_inside_the_headers() {
        let app = sheet();
        let grid = Grid::of(&app, 0);
        let ops = frame(
            &app,
            0,
            &grid,
            (400.0, 300.0),
            (0.0, 0.0),
            Selection::default(),
            &LOOK,
        );
        let (_, x, top) = *texts(&ops).iter().find(|(t, ..)| *t == "Rent").unwrap();
        assert_eq!(x, HEADER_W + PAD_X);
        assert!(top > HEADER_H);
        for op in &ops {
            if let Op::Text { text, clip, .. } = op
                && text == "Rent"
            {
                assert!(clip.x >= HEADER_W && clip.y >= HEADER_H, "{clip:?}");
            }
        }
        // Scrolled one column right, B is where A was.
        let ops = frame(
            &app,
            0,
            &grid,
            (400.0, 300.0),
            (geom::COL_W, 0.0),
            Selection::default(),
            &LOOK,
        );
        assert!(
            !texts(&ops).iter().any(|(t, ..)| *t == "Rent"),
            "off screen"
        );
        let labels: Vec<&str> = texts(&ops).iter().map(|(t, ..)| *t).collect();
        assert!(
            labels.contains(&"B") && !labels.contains(&"A"),
            "{labels:?}"
        );
    }

    /// One cell has an outline and no wash; a range is washed everywhere but its active cell.
    #[test]
    fn a_selection_is_washed_but_for_its_active_cell_and_outlined() {
        let app = sheet();
        assert!(washes(&drawn(&app)).is_empty(), "a single cell has no wash");
        let outline = |ops: &[Op]| {
            ops.iter()
                .filter(
                    |op| matches!(op, Op::Fill { color, .. } if *color == Palette::LIGHT.accent),
                )
                .count()
        };
        assert_eq!(outline(&drawn(&app)), 4, "four sides");

        // B2:C3, with the cursor in B2.
        let range = Selection {
            anchor: Pos::new(2, 2),
            active: Pos::new(1, 1),
        };
        let ops = drawn_with(&app, range);
        let washed = washes(&ops);
        let area: f64 = washed.iter().map(|rect| rect.w * rect.h).sum();
        let cell = geom::COL_W * geom::ROW_H;
        assert!(
            (area - 3.0 * cell).abs() < 1e-9,
            "three of the four cells: {washed:?}"
        );
        let active = Grid::of(&app, 0).cell(1, 1);
        for rect in &washed {
            assert!(
                rect.intersection(&active).is_empty(),
                "the active cell is left out"
            );
        }
        // The wash is under the text, and the outline over it.
        let first_text = ops
            .iter()
            .position(|op| matches!(op, Op::Text { .. }))
            .unwrap();
        let last_wash = ops
            .iter()
            .rposition(|op| matches!(op, Op::Wash { .. }))
            .unwrap();
        assert!(last_wash < first_text);
        assert!(
            matches!(ops.last(), Some(Op::Fill { color, .. }) if *color == Palette::LIGHT.accent)
        );
    }

    /// A whole column is a million rows, and the outline stops where the grid does.
    #[test]
    fn a_whole_column_is_outlined_as_far_as_the_grid_goes() {
        let app = sheet();
        let grid = Grid::of(&app, 0);
        let rect = selection_rect(&grid, Selection::whole_col(1));
        assert_eq!(rect.y, 0.0);
        assert_eq!(rect.bottom(), grid.size().1);
    }

    /// The selection's tracks are tinted in the header bands.
    #[test]
    fn the_headers_tint_the_selections_tracks() {
        let grid = Grid::of(&sheet(), 0);
        let range = Selection {
            anchor: Pos::new(0, 1),
            active: Pos::new(1, 2),
        };
        let columns = column_header(&grid, 0.0, 4.0 * geom::COL_W, range, &LOOK);
        let tinted: Vec<f64> = washes(&columns).iter().map(|rect| rect.x).collect();
        assert_eq!(tinted, [geom::COL_W, 2.0 * geom::COL_W], "B and C");
        let rows = row_header(&grid, 0.0, 4.0 * geom::ROW_H, range, &LOOK);
        assert_eq!(washes(&rows).len(), 2, "rows 1 and 2");
    }

    #[test]
    fn a_line_break_in_a_cell_is_a_space_on_one_line() {
        assert_eq!(one_line("rent\nincrease"), "rent increase");
    }

    /// View ▸ Cell Roles: a marker in the role's colour at the cell's leading edge, and the
    /// cell's own text moved clear of it; with the overlay off, neither.
    #[test]
    fn a_role_marker_takes_its_margin_and_the_text_moves_over() {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "Rent", RecalcMode::Document)
            .unwrap();
        let grid = Grid::of(&app, 0);
        let view = Rect::new(0.0, 0.0, 500.0, 200.0);
        let plain_ops = cells(&app, 0, &grid, view, Selection::default(), &LOOK);
        let plain = texts(&plain_ops);
        let roles = Look {
            overlays: Overlays::ROLES,
            ..LOOK
        };
        let marked_ops = cells(&app, 0, &grid, view, Selection::default(), &roles);
        let marked = texts(&marked_ops);
        let label = CellRole::Label.marker();
        assert!(plain.iter().all(|(text, ..)| *text != label));
        let at = |texts: &[(&str, f64, f64)], what: &str| {
            texts
                .iter()
                .find(|(text, ..)| *text == what)
                .map(|(_, x, _)| *x)
        };
        assert!(at(&marked, label).is_some(), "{marked:?}");
        assert_eq!(
            at(&marked, "Rent").unwrap() - at(&plain, "Rent").unwrap(),
            MARKER_W
        );
    }

    #[test]
    fn a_defined_name_is_outlined_only_when_asked() {
        let app = App::new();
        app.enter(0, Pos::new(1, 1), "5", RecalcMode::Document)
            .unwrap();
        app.set_name("rate", "[$Sheet1.$B$2]").unwrap();
        let grid = Grid::of(&app, 0);
        let view = Rect::new(0.0, 0.0, 500.0, 200.0);
        let muted = mix(Palette::LIGHT.ink, Palette::LIGHT.page, 0.55);
        let count = |look: &Look| {
            cells(&app, 0, &grid, view, Selection::default(), look)
                .iter()
                .filter(|op| matches!(op, Op::Fill { color, .. } if *color == muted))
                .count()
        };
        assert_eq!(count(&LOOK), 0);
        let names = Look {
            overlays: Overlays::NAMES,
            ..LOOK
        };
        assert_eq!(count(&names), 4, "four edges round B2");
    }

    #[test]
    fn every_drawn_role_has_a_colour_in_both_appearances() {
        for palette in [Palette::LIGHT, Palette::DARK] {
            for role in CellRole::ALL {
                assert_eq!(
                    role_color(role, &palette).is_none(),
                    role == CellRole::Empty,
                    "{role:?}"
                );
            }
        }
    }

    /// Wrap Text draws a long label as lines inside its cell, each where the breaker put it.
    #[test]
    fn a_wrapping_cell_is_drawn_as_lines() {
        let app = App::new();
        app.enter(
            0,
            Pos::new(0, 0),
            "the quick brown fox jumps",
            RecalcMode::Document,
        )
        .unwrap();
        let wrap = grind_sheet::style::CellStyle {
            wrap: Some("wrap".into()),
            ..Default::default()
        };
        app.set_style(0, Pos::new(0, 0), Pos::new(0, 0), Some(wrap))
            .unwrap();
        app.set_row_height(0, 0..1, Some("2in".into())).unwrap();
        // About ten `Fixed` characters of room.
        app.set_col_width(0, 0..1, Some("0.25in".into())).unwrap();
        let grid = Grid::of(&app, 0);
        let view = Rect::new(0.0, 0.0, 500.0, 300.0);
        let ops = cells(&app, 0, &grid, view, Selection::default(), &LOOK);
        let lines: Vec<(&str, f64, f64)> = texts(&ops);
        assert!(lines.len() > 1, "{lines:?}");
        assert!(
            lines.windows(2).all(|pair| pair[0].2 < pair[1].2),
            "down the cell"
        );
        let joined: Vec<&str> = lines.iter().map(|(text, ..)| *text).collect();
        assert_eq!(joined.join(" "), "the quick brown fox jumps");
    }

    #[test]
    fn a_border_is_centred_on_its_grid_line_and_double_is_two() {
        let cell = Rect::new(10.0, 20.0, 50.0, 18.0);
        let mut style = CellStyle::default();
        style.borders[1] = Some("2pt solid #ff0000".into());
        style.borders[3] = Some("0.06pt solid #000000".into());
        style.borders[0] = Some("none".into());
        style.borders[2] = Some("garbage".into());
        let ops = borders(cell, &style, &Palette::LIGHT, 0.5);
        assert_eq!(
            ops,
            [
                Op::Fill {
                    rect: Rect::new(60.0 - 0.25 - 1.0, 20.0, 2.0, 18.0),
                    color: (0xff, 0, 0),
                },
                Op::Fill {
                    rect: Rect::new(10.0, 38.0 - 0.25 - 0.25, 50.0, 0.5),
                    color: (0, 0, 0),
                },
            ],
            "the right edge 2pt wide, the bottom no thinner than a device pixel"
        );
        style.borders = [None, None, Some("3pt double #000000".into()), None];
        assert_eq!(borders(cell, &style, &Palette::LIGHT, 0.5).len(), 2);
    }

    #[test]
    fn with_formulas_shown_a_formula_cell_shows_its_formula() {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "2", RecalcMode::Document)
            .unwrap();
        app.enter(0, Pos::new(0, 1), "=[.A1]*3", RecalcMode::Document)
            .unwrap();
        let grid = Grid::of(&app, 0);
        let view = Rect::new(0.0, 0.0, 500.0, 200.0);
        let shown = |look: &Look| {
            texts(&cells(&app, 0, &grid, view, Selection::default(), look))
                .into_iter()
                .map(|(text, ..)| text.to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(shown(&LOOK), ["2", "6"]);
        let formulas = Look {
            formulas: true,
            ..LOOK
        };
        assert_eq!(
            shown(&formulas),
            ["2", "=A1*3"],
            "a plain value is still itself"
        );
    }
}
