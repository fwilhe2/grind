// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A viewport, painted onto a device context.
//!
//! Two halves, and the split is the same one the crate makes everywhere: **what a cell looks
//! like is decided in portable code** ([`Appearance::of`], tested on any host) and only putting
//! pixels down needs Windows. That matters more here than it looks — "a number is right-aligned
//! unless the document says otherwise" is a rule about documents, not about GDI, and it is the
//! sort of thing that quietly differs between shells when each one re-decides it in its own
//! painting code.
//!
//! `paint` takes an `HDC` and a `Frame` and nothing else about the window — no `HWND` — which is
//! what makes W2's `--render-to` a second caller rather than a second drawing path
//! (`doc/windows-shell.md`, decision 5). It is not linked here because it does not exist off
//! Windows, and this crate's documentation is built on Linux.

use grind_sheet::model::CellValue;
use grind_sheet::style::CellStyle;

use crate::theme::{Rgb, Theme};

use super::geom::GridGeom;
use super::keymap::Selection;

/// How far a selected cell's ground moves towards [`Theme::selection`].
///
/// A wash rather than a fill, and the number is what makes that true: at 0.22 a document's own
/// red is still red and still visibly selected. GDI has no alpha, so this is applied by
/// [`Rgb::blend`] before anything is painted — see [`ground`].
pub const WASH: f64 = 0.22;

/// What colour a cell's ground is actually painted, once the selection is taken into account.
///
/// `None` means "paint nothing here" — the window's own background is already down, and filling
/// it again per cell is work for no pixels.
///
/// The **active cell is never washed**, even inside a large selection. That is what makes it
/// read as the cell the cursor is in rather than as one more selected cell, and it is the same
/// choice `ui_sheet_gtk` makes ("the active cell left out of the wash").
pub fn ground(background: Option<Rgb>, theme: Theme, selected: bool, active: bool) -> Option<Rgb> {
    match selected && !active {
        false => background,
        true => Some(
            background
                .unwrap_or(theme.background)
                .blend(theme.selection, WASH),
        ),
    }
}

/// Which end of the cell the text sits at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// How one cell is drawn, resolved from what the document says and what it leaves open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    pub align: Align,
    pub bold: bool,
    pub italic: bool,
    /// `None` means "the theme's ink" — a cell without a colour of its own has to follow the
    /// user's light/dark choice, and baking the theme's value in here would make a document
    /// that was opened in light mode unreadable when the theme changed under it.
    pub text: Option<Rgb>,
    pub background: Option<Rgb>,
}

impl Appearance {
    /// What the document asks for, with the spreadsheet's own defaults underneath.
    ///
    /// The default alignment is by **type**, which is ODF's behaviour and everyone else's: a
    /// number, a boolean and a date go to the right and a label to the left, so that a column
    /// of figures lines up on its digits. `fo:text-align` overrides it when the document sets
    /// one; `start`/`end` are the writing-direction spellings and this shell is LTR by decision
    /// (`doc/text-layout.md` excludes RTL), so they resolve to left and right.
    pub fn of(value: &CellValue, style: Option<&CellStyle>) -> Self {
        let default = match value {
            CellValue::Number(_) | CellValue::Bool(_) => Align::Right,
            CellValue::Text(_) | CellValue::Empty => Align::Left,
        };
        let Some(style) = style else {
            return Self {
                align: default,
                bold: false,
                italic: false,
                text: None,
                background: None,
            };
        };
        let align = match style.align.as_deref() {
            Some("left") | Some("start") => Align::Left,
            Some("right") | Some("end") => Align::Right,
            Some("center") => Align::Center,
            // `justify` on a cell means nothing this shell can honour with one line of text,
            // and an unknown value is a document being tolerated rather than obeyed (R5).
            _ => default,
        };
        Self {
            align,
            // A weight is `bold`, `normal`, or a hundreds number — and 600 and up is bold
            // everywhere else, so it is bold here.
            bold: match style.font_weight.as_deref() {
                Some("bold") => true,
                Some(other) => other.parse::<u32>().is_ok_and(|weight| weight >= 600),
                None => false,
            },
            italic: matches!(
                style.font_style.as_deref(),
                Some("italic") | Some("oblique")
            ),
            text: style.color.as_deref().and_then(Rgb::parse),
            // `transparent` is a real value and it means *no fill*, not black.
            background: match style.background.as_deref() {
                Some("transparent") | None => None,
                Some(hex) => Rgb::parse(hex),
            },
        }
    }
}

/// One cell's text as a single line, which is what a grid draws.
///
/// A cell's text can contain a line break — `text:line-break` inside a `text:p`, which the
/// reader keeps as a `\n` — and GDI's `DT_SINGLELINE` draws a control character as a *box
/// glyph* rather than ignoring it. That was visible on screen before this existed: the note in
/// `examples/sample-sheet.sh`'s H2 came out as `rent increase□starting…`.
///
/// Every control character becomes a space rather than being dropped, so the two sides of a
/// break stay two words. Where the second line *goes* is a wider question — a wrapped cell
/// needs row auto-height, which is L3 and named as this shell's gap — so W1 shows the whole
/// text on one line and elides it like any other.
pub fn one_line(text: &str) -> std::borrow::Cow<'_, str> {
    match text.contains(|c: char| c.is_control()) {
        false => std::borrow::Cow::Borrowed(text),
        true => std::borrow::Cow::Owned(
            text.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect(),
        ),
    }
}

#[cfg(windows)]
pub use windows_impl::{Frame, paint};

#[cfg(windows)]
mod windows_impl {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DrawTextW, HDC,
        SetBkMode, SetTextColor, TRANSPARENT,
    };

    use super::{Align, Appearance, GridGeom, Selection, Theme};
    use crate::gdi::{self, Font, Selected};
    use crate::theme::Rgb;

    /// The padding between a cell's edge and its text, in pixels at 100%.
    const PAD: f64 = 4.0;

    /// Everything the painter needs that is not geometry or colour.
    pub struct Frame<'a> {
        pub geom: &'a GridGeom,
        pub theme: Theme,
        pub viewport: &'a grind_sheet::Viewport,
        /// The name of the sheet being drawn and what the status bar says about it.
        pub status: &'a str,
        /// What the name box shows — an address, or the name of the range if it has one.
        ///
        /// Drawn here rather than read out of the child `EDIT` control, and that is deliberate
        /// on two counts: the box is a read-out until somebody types in it (the control is
        /// created hidden and shown over this rectangle on demand), and `--render-to` has no
        /// window at all, so anything only a control could draw would be missing from every
        /// rendered frame.
        pub name: &'a str,
        /// The selection, which is presentation state and never leaves the shell.
        pub selection: Selection,
        /// The point size the shell font is drawn at, already scaled for this monitor's DPI.
        pub font_px: i32,
        pub face: &'a str,
    }

    /// Draw one frame of the grid onto `dc`.
    ///
    /// Every pixel of the client area is written, which is what lets `WM_ERASEBKGND` be
    /// answered with "already done" — see `gdi::BackBuffer`.
    pub fn paint(dc: HDC, frame: &Frame) {
        let g = frame.geom;
        let theme = frame.theme;
        let body = g.body();

        // The ground. The header bands and the status bar are painted *after* the cells rather
        // than before, so that a cell scrolled under a header cannot show through it — which is
        // cheaper than clipping the cell loop and is why they are not painted here as well.
        gdi::fill(
            dc,
            0,
            0,
            g.width.round() as i32,
            g.height.round() as i32,
            theme.background,
        );

        let regular = Font::new(frame.face, frame.font_px, false);
        let bold = Font::new(frame.face, frame.font_px, true);

        // SAFETY: the DC is the caller's and live for this function.
        unsafe {
            SetBkMode(dc, TRANSPARENT);
        }

        // The cells. Hidden tracks have zero width or height and are skipped rather than drawn
        // as a line, because a zero-width column is not a boundary.
        {
            let _font = Selected::font(dc, &regular);
            for row in g.visible_rows() {
                for col in g.visible_cols() {
                    let rect = g.cell_rect(row, col);
                    if rect.w <= 0.0 || rect.h <= 0.0 {
                        continue;
                    }
                    let empty = grind_sheet::model::CellValue::Empty;
                    let value = frame.viewport.get(row, col).unwrap_or(&empty);
                    let look = Appearance::of(value, frame.viewport.style(row, col));
                    let (left, top, right, bottom) = rect.edges();
                    let selected = frame.selection.contains(row, col);
                    let active =
                        frame.selection.active.row == row && frame.selection.active.col == col;
                    if let Some(fill) = super::ground(look.background, theme, selected, active) {
                        gdi::fill(dc, left, top, right, bottom, fill);
                    }
                    // The grid's own hairlines: the right and bottom edges of every cell, so
                    // that adjacent cells share one line rather than drawing two.
                    gdi::fill(dc, right - 1, top, right, bottom, theme.grid_line);
                    gdi::fill(dc, left, bottom - 1, right, bottom, theme.grid_line);

                    let Some(text) = frame.viewport.text(row, col) else {
                        continue;
                    };
                    if text.is_empty() {
                        continue;
                    }
                    let _bold = look.bold.then(|| Selected::font(dc, &bold));
                    draw_text(
                        dc,
                        text,
                        left,
                        top,
                        right,
                        bottom,
                        look.align,
                        look.text.unwrap_or(theme.text),
                        crate::sheet::geom::scale(PAD, g.dpi),
                    );
                }
            }
        }

        // The outline round the selected rectangle, drawn after the cells so it sits on top of
        // their hairlines, and before the headers so those still cover it where it runs under
        // the band.
        outline(dc, frame);

        // The headers, over the cells — a cell scrolled under the header band must not show
        // through it, and drawing them second is cheaper than clipping the loop above.
        {
            let _font = Selected::font(dc, &regular);
            gdi::fill(
                dc,
                0,
                g.strip_h.round() as i32,
                g.width.round() as i32,
                (g.strip_h + g.header_h).round() as i32,
                theme.header,
            );
            gdi::fill(
                dc,
                0,
                g.strip_h.round() as i32,
                g.header_w.round() as i32,
                (body.y + body.h).round() as i32,
                theme.header,
            );
            let (start, end) = frame.selection.rect();
            for col in g.visible_cols() {
                let rect = g.col_header_rect(col);
                if rect.w <= 0.0 {
                    continue;
                }
                let (left, top, right, bottom) = rect.edges();
                if (start.col..=end.col).contains(&col) {
                    gdi::fill(dc, left, top, right, bottom, theme.header_active);
                }
                gdi::fill(dc, right - 1, top, right, bottom, theme.header_line);
                draw_text(
                    dc,
                    &grind_sheet::formula::lex::column_name(col),
                    left,
                    top,
                    right,
                    bottom,
                    Align::Center,
                    theme.header_text,
                    0.0,
                );
            }
            for row in g.visible_rows() {
                let rect = g.row_header_rect(row);
                if rect.h <= 0.0 {
                    continue;
                }
                let (left, top, right, bottom) = rect.edges();
                if (start.row..=end.row).contains(&row) {
                    gdi::fill(dc, left, top, right, bottom, theme.header_active);
                }
                gdi::fill(dc, left, bottom - 1, right, bottom, theme.header_line);
                draw_text(
                    dc,
                    // The only `+ 1` in this shell, and it is a label rather than arithmetic —
                    // `sheet/src/a1.rs` owns the conversion everywhere it is one.
                    &(u64::from(row) + 1).to_string(),
                    left,
                    top,
                    right,
                    bottom,
                    Align::Center,
                    theme.header_text,
                    0.0,
                );
            }
            // The line closing the header band, and the corner button.
            gdi::fill(
                dc,
                0,
                (g.strip_h + g.header_h - 1.0).round() as i32,
                g.width.round() as i32,
                (g.strip_h + g.header_h).round() as i32,
                theme.header_line,
            );
            gdi::fill(
                dc,
                (g.header_w - 1.0).round() as i32,
                g.strip_h.round() as i32,
                g.header_w.round() as i32,
                (body.y + body.h).round() as i32,
                theme.header_line,
            );
        }

        // The strip along the top, and the name box in it.
        {
            let _font = Selected::font(dc, &regular);
            let strip = g.strip_rect();
            let (left, top, right, bottom) = strip.edges();
            gdi::fill(dc, left, top, right, bottom, theme.header);
            gdi::fill(dc, left, bottom - 1, right, bottom, theme.header_line);

            let field = g.name_box_rect();
            let (left, top, right, bottom) = field.edges();
            gdi::fill(dc, left, top, right, bottom, theme.field_line);
            gdi::fill(dc, left + 1, top + 1, right - 1, bottom - 1, theme.field);
            draw_text(
                dc,
                frame.name,
                left,
                top,
                right,
                bottom,
                Align::Left,
                theme.text,
                crate::sheet::geom::scale(PAD, g.dpi),
            );
        }

        // The status bar.
        {
            let _font = Selected::font(dc, &regular);
            let rect = g.status_rect();
            let (left, top, right, bottom) = rect.edges();
            gdi::fill(dc, left, top, right, bottom, theme.status);
            gdi::fill(dc, left, top, right, top + 1, theme.header_line);
            draw_text(
                dc,
                frame.status,
                left,
                top,
                right,
                bottom,
                Align::Left,
                theme.status_text,
                crate::sheet::geom::scale(8.0, g.dpi),
            );
        }
    }

    /// The outline round the selected rectangle.
    ///
    /// Four bars rather than a frame, each clipped to the body, because a selection is
    /// routinely bigger than the window — clicking a column header selects a million rows, and
    /// its bottom edge is twenty million pixels down. Each edge is drawn only where the body
    /// actually reaches it, so the two sides of a tall selection are drawn and its bottom is
    /// not, which is what the eye wants anyway.
    fn outline(dc: HDC, frame: &Frame) {
        let g = frame.geom;
        let (start, end) = frame.selection.rect();
        let first = g.cell_rect(start.row, start.col);
        let last = g.cell_rect(end.row, end.col);
        let body = g.body();
        // Clamped in `f64` before the cast: `cell_rect` for the last row of the sheet is tens
        // of millions of pixels down, and while that fits an `i32` it is worth never letting
        // the arithmetic depend on that.
        let clamp = |v: f64, low: f64, high: f64| v.clamp(low, high).round() as i32;
        let left = clamp(first.x, body.x, body.x + body.w);
        let top = clamp(first.y, body.y, body.y + body.h);
        let right = clamp(last.x + last.w, body.x, body.x + body.w);
        let bottom = clamp(last.y + last.h, body.y, body.y + body.h);
        if right <= left || bottom <= top {
            return;
        }
        let weight = crate::sheet::geom::scale(2.0, g.dpi).round().max(1.0) as i32;
        let edge = frame.theme.selection_edge;
        // A bar is drawn only if the edge it marks is really where the body stops, so a
        // selection running off the bottom of the window has no bottom bar.
        if first.x >= body.x {
            gdi::fill(dc, left, top, left + weight, bottom, edge);
        }
        if last.x + last.w <= body.x + body.w {
            gdi::fill(dc, right - weight, top, right, bottom, edge);
        }
        if first.y >= body.y {
            gdi::fill(dc, left, top, right, top + weight, edge);
        }
        if last.y + last.h <= body.y + body.h {
            gdi::fill(dc, left, bottom - weight, right, bottom, edge);
        }
    }

    /// One string, clipped to a rectangle, vertically centred and elided if it does not fit.
    ///
    /// `DT_END_ELLIPSIS` rather than letting the text run into the next cell: overflow into an
    /// empty neighbour is a real spreadsheet behaviour and a named gap here rather than a
    /// half-done one — it needs to know whether the neighbour is empty *and* to draw outside its
    /// own cell's rectangle, which is W3's problem, not W1's.
    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        dc: HDC,
        text: &str,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        align: Align,
        colour: Rgb,
        pad: f64,
    ) {
        let pad = pad.round() as i32;
        let mut rect = RECT {
            left: left + pad,
            top,
            right: (right - pad - 1).max(left + pad),
            bottom,
        };
        if rect.right <= rect.left {
            return;
        }
        // **Not** `gdi::wide`, and the difference was visible on screen: `DrawTextW` in the
        // `windows` crate takes a *slice* and uses its length as the character count, so a
        // NUL-terminated buffer draws the terminator too — every label in the grid came out
        // with a box glyph after it under Wine. The `…W` entry points that take a `PCWSTR` want
        // the terminator; the ones that take a length do not.
        let mut wide: Vec<u16> = super::one_line(text).encode_utf16().collect();
        let format = DT_SINGLELINE
            | DT_VCENTER
            | DT_NOPREFIX
            | DT_END_ELLIPSIS
            | match align {
                Align::Left => DT_LEFT,
                Align::Right => DT_RIGHT,
                Align::Center => windows::Win32::Graphics::Gdi::DT_CENTER,
            };
        // SAFETY: the DC is live, and both the rectangle and the buffer are locals that outlive
        // the call. `DrawTextW` writes back into `rect` when asked to calculate, which is why it
        // is `mut`, and it is never read afterwards.
        unsafe {
            SetTextColor(dc, COLORREF(colour.colorref()));
            DrawTextW(dc, &mut wide[..], &mut rect, format);
        }
    }
}

/// The header band at 100%, in pixels — a design measurement, scaled for the monitor by
/// `geom::scale` at the one place the geometry is built.
pub const HEADER_H: f64 = 22.0;
pub const HEADER_W: f64 = 46.0;
/// The status bar's height at 100%.
pub const STATUS_H: f64 = 24.0;
/// The strip along the top that holds the name box — and, from W3, the formula bar.
pub const STRIP_H: f64 = 28.0;
/// The default track sizes at 100%, for the columns and rows a document does not size.
pub const COL_W: f64 = 80.0;
pub const ROW_H: f64 = 20.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn styled(f: impl FnOnce(&mut CellStyle)) -> CellStyle {
        let mut style = CellStyle::default();
        f(&mut style);
        style
    }

    #[test]
    fn a_line_break_in_a_cell_becomes_a_space() {
        // GDI draws a control character as a box glyph, so this is not cosmetic tidying: it is
        // the difference between `rent increase starting` and `rent increase\u{25a1}starting`.
        assert_eq!(one_line("a\nb"), "a b");
        assert_eq!(one_line("a\r\nb\tc"), "a  b c");
        // The common case allocates nothing.
        assert!(matches!(one_line("plain"), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn a_number_goes_right_and_a_label_goes_left() {
        assert_eq!(
            Appearance::of(&CellValue::Number(1.0), None).align,
            Align::Right
        );
        assert_eq!(
            Appearance::of(&CellValue::Bool(true), None).align,
            Align::Right
        );
        assert_eq!(
            Appearance::of(&CellValue::Text("hi".into()), None).align,
            Align::Left
        );
        assert_eq!(Appearance::of(&CellValue::Empty, None).align, Align::Left);
    }

    #[test]
    fn the_document_overrides_the_default_alignment() {
        let left = styled(|s| s.align = Some("left".into()));
        assert_eq!(
            Appearance::of(&CellValue::Number(1.0), Some(&left)).align,
            Align::Left
        );
        // The writing-direction spellings, resolved LTR by decision.
        let end = styled(|s| s.align = Some("end".into()));
        assert_eq!(
            Appearance::of(&CellValue::Text("x".into()), Some(&end)).align,
            Align::Right
        );
    }

    /// R5: an unknown property value is inert rather than an error, so the type default stands.
    #[test]
    fn an_alignment_this_build_does_not_know_falls_back() {
        let odd = styled(|s| s.align = Some("justify".into()));
        assert_eq!(
            Appearance::of(&CellValue::Number(1.0), Some(&odd)).align,
            Align::Right
        );
    }

    #[test]
    fn a_numeric_weight_is_bold_from_600() {
        for (weight, bold) in [
            ("bold", true),
            ("normal", false),
            ("700", true),
            ("400", false),
        ] {
            let style = styled(|s| s.font_weight = Some(weight.into()));
            assert_eq!(
                Appearance::of(&CellValue::Empty, Some(&style)).bold,
                bold,
                "{weight}"
            );
        }
    }

    #[test]
    fn transparent_is_no_fill_rather_than_a_colour() {
        let clear = styled(|s| s.background = Some("transparent".into()));
        assert_eq!(
            Appearance::of(&CellValue::Empty, Some(&clear)).background,
            None
        );
        let red = styled(|s| s.background = Some("#ff4136".into()));
        assert_eq!(
            Appearance::of(&CellValue::Empty, Some(&red)).background,
            Some(Rgb(0xff, 0x41, 0x36))
        );
    }

    /// A cell with no colour of its own follows the *theme*, so the same document is readable
    /// in both. Resolving it to a literal here is the bug this asserts against.
    /// The selection is a *wash*, not a fill: a selected cell the document coloured keeps its
    /// colour, and the active cell keeps its ground entirely so that it reads as the cursor.
    #[test]
    fn the_wash_covers_the_selection_and_spares_the_active_cell() {
        let theme = crate::theme::Theme::of(crate::theme::Mode::Light);
        let red = Rgb(0xff, 0x41, 0x36);
        // Not selected: nothing to paint over the window's own ground.
        assert_eq!(ground(None, theme, false, false), None);
        assert_eq!(ground(Some(red), theme, false, false), Some(red));
        // Selected: washed, and still recognisably the document's red.
        let washed = ground(Some(red), theme, true, false).expect("a wash is a colour");
        assert_ne!(washed, red);
        assert!(washed.0 > washed.2, "still more red than blue: {washed:?}");
        // The active cell is spared, selected or not — that is what makes it the cursor.
        assert_eq!(ground(Some(red), theme, true, true), Some(red));
        assert_eq!(ground(None, theme, true, true), None);
    }

    #[test]
    fn an_uncoloured_cell_defers_to_the_theme() {
        let look = Appearance::of(&CellValue::Text("x".into()), None);
        assert_eq!(look.text, None);
        assert_eq!(look.background, None);
    }
}
