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
//! **What M2 does not draw**, each a named gap in `doc/macos-shell.md`: the selection (M3), a
//! label spilling into the empty cells beside it (it is clipped to its own cell), borders, and
//! the view-mode overlays (M8).

use grind_core::color::{self, Rgb};
use grind_core::layout::Metrics;
use grind_core::style::TextStyle;
use grind_sheet::{App, look, numfmt};

use super::geom::{self, Grid, HEADER_H, HEADER_W, Rect};

/// How far a cell's text stands off its edges, across and down.
pub const PAD_X: f64 = 4.0;
pub const PAD_Y: f64 = 2.0;

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
        dark: false,
    };
    pub const DARK: Palette = Palette {
        page: (0x1e, 0x1e, 0x1e),
        ink: (0xff, 0xff, 0xff),
        grid: (0x38, 0x38, 0x38),
        header: (0x2a, 0x2a, 0x2a),
        header_ink: (0x98, 0x98, 0x9d),
        dark: true,
    };
}

/// One thing to draw.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A filled rectangle — a ground, a fill the document chose, a grid line, a header band.
    Fill { rect: Rect, color: Rgb },
    /// One line of text whose box starts at `(x, top)`, set in `style`, drawn only inside
    /// `clip`. The renderer finds the baseline from the font it resolves `style` to — the same
    /// resolution [`Metrics`] measured with.
    Text {
        x: f64,
        top: f64,
        text: String,
        style: TextStyle,
        color: Rgb,
        clip: Rect,
    },
}

impl Op {
    /// The same thing drawn `(dx, dy)` further on, kept inside `clip` — how one part of a frame
    /// (a header band, the cells) is placed in the whole.
    fn placed(self, dx: f64, dy: f64, clip: &Rect) -> Option<Op> {
        match self {
            Op::Fill { rect, color } => {
                let rect = rect.offset(dx, dy).intersection(clip);
                (!rect.is_empty()).then_some(Op::Fill { rect, color })
            }
            Op::Text {
                x,
                top,
                text,
                style,
                color,
                clip: own,
            } => {
                let clip = own.offset(dx, dy).intersection(clip);
                (!clip.is_empty()).then_some(Op::Text {
                    x: x + dx,
                    top: top + dy,
                    text,
                    style,
                    color,
                    clip,
                })
            }
        }
    }
}

/// How wide `text` is set in `style` — the last cumulative advance.
fn width(metrics: &dyn Metrics, text: &str, style: &TextStyle) -> f64 {
    let mut advances = Vec::new();
    metrics.advances(text, style, &mut advances);
    advances.last().copied().map_or(0.0, f64::from)
}

/// The cells of `sheet` that meet `view` — a rectangle in the sheet's own coordinates, as a
/// view's dirty rectangle is — in those same coordinates.
///
/// Grounds first, then the grid lines, then the text, so a document's fill never covers a line
/// and a line never crosses a word. `hairline` is one device pixel in points — half a point on a
/// Retina screen — which is how thin a grid line is.
pub fn cells(
    app: &App,
    sheet: usize,
    grid: &Grid,
    view: Rect,
    palette: &Palette,
    metrics: &dyn Metrics,
    hairline: f64,
) -> Vec<Op> {
    let mut ops = vec![Op::Fill {
        rect: view,
        color: palette.page,
    }];
    let rows = grid.rows_in(view.y, view.h);
    let cols = grid.cols_in(view.x, view.w);
    let Ok(viewport) = app.get_viewport(sheet, rows.clone(), cols.clone()) else {
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

    // The text.
    for row in rows {
        for col in cols.clone() {
            let Some(text) = viewport.text(row, col).filter(|text| !text.is_empty()) else {
                continue;
            };
            let cell = grid.cell(row, col);
            if cell.is_empty() {
                continue;
            }
            let value = viewport.get(row, col).cloned().unwrap_or_default();
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
            ops.push(cell_text(one_line(text), &value, style, cell, ink, metrics));
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
pub fn column_header(
    grid: &Grid,
    x: f64,
    w: f64,
    palette: &Palette,
    metrics: &dyn Metrics,
    hairline: f64,
) -> Vec<Op> {
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
pub fn row_header(
    grid: &Grid,
    y: f64,
    h: f64,
    palette: &Palette,
    metrics: &dyn Metrics,
    hairline: f64,
) -> Vec<Op> {
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
#[allow(clippy::too_many_arguments)]
pub fn frame(
    app: &App,
    sheet: usize,
    grid: &Grid,
    (width, height): (f64, f64),
    (scroll_x, scroll_y): (f64, f64),
    palette: &Palette,
    metrics: &dyn Metrics,
    hairline: f64,
) -> Vec<Op> {
    let body_w = (width - HEADER_W).max(0.0);
    let body_h = (height - HEADER_H).max(0.0);
    let body = Rect::new(HEADER_W, HEADER_H, body_w, body_h);
    let top = Rect::new(HEADER_W, 0.0, body_w, HEADER_H);
    let left = Rect::new(0.0, HEADER_H, HEADER_W, body_h);

    let mut ops = Vec::new();
    let view = Rect::new(scroll_x, scroll_y, body_w, body_h);
    let (dx, dy) = (HEADER_W - scroll_x, HEADER_H - scroll_y);
    for op in cells(app, sheet, grid, view, palette, metrics, hairline) {
        ops.extend(op.placed(dx, dy, &body));
    }
    for op in column_header(grid, scroll_x, body_w, palette, metrics, hairline) {
        ops.extend(op.placed(dx, 0.0, &top));
    }
    for op in row_header(grid, scroll_y, body_h, palette, metrics, hairline) {
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
                Op::Fill { .. } => None,
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

    fn drawn(app: &App) -> Vec<Op> {
        let grid = Grid::of(app, 0);
        let view = Rect::new(0.0, 0.0, 500.0, 200.0);
        cells(app, 0, &grid, view, &Palette::LIGHT, &Fixed, HAIR)
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
        let ops = column_header(&grid, 0.0, 2.0 * geom::COL_W, &Palette::LIGHT, &Fixed, HAIR);
        let labels: Vec<&str> = texts(&ops).iter().map(|(t, ..)| *t).collect();
        assert_eq!(labels, ["A", "B"]);
        let ops = row_header(&grid, 0.0, 2.0 * geom::ROW_H, &Palette::LIGHT, &Fixed, HAIR);
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
            &Palette::LIGHT,
            &Fixed,
            HAIR,
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
            &Palette::LIGHT,
            &Fixed,
            HAIR,
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

    #[test]
    fn a_line_break_in_a_cell_is_a_space_on_one_line() {
        assert_eq!(one_line("rent\nincrease"), "rent increase");
    }
}
