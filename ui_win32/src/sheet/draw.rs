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
pub use windows_impl::{Frame, draw_text, paint};

#[cfg(windows)]
mod windows_impl {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DrawTextW, HDC,
        SetBkMode, SetTextColor, TRANSPARENT,
    };

    use super::{Align, Appearance, GridGeom, Selection, Theme};
    use crate::gdi::{self, Font, Selected};
    use crate::sheet::assist::{Ink, Piece};
    use crate::theme::Rgb;

    /// The padding between a cell's edge and its text, in pixels at 100%.
    ///
    /// Deliberately *not* raised with the bands below it, and a rendered frame is why: at six
    /// pixels `3,710.00 €` no longer fit the column the document sizes for it and came back as
    /// `3,710.0…`. A band's padding costs nothing but a band; a cell's is paid for in the
    /// document's own numbers.
    const PAD: f64 = 4.0;

    /// The padding between a band's edge and the sentence in it — the notice bar, the assist
    /// band and the status bar, all of which are text on a ground rather than text in a box.
    /// Fluent's group spacing, so a band's text lines up with the strip's fields above it.
    const BAND_PAD: f64 = crate::theme::space::GROUP;

    /// How far the corners of the strip's two fields are cut, in pixels at 100%.
    const FIELD_RADIUS: f64 = crate::theme::space::RADIUS;

    /// The width of the `fx` badge at the head of the formula bar, in pixels at 100%.
    const BADGE_W: f64 = 30.0;

    /// The accent bar marking a header button whose track is selected, in pixels at 100%. Drawn
    /// on the band's *inner* edge — under a column's letter, beside a row's number — so the two
    /// bands point at the selection between them.
    const HEADER_BAR: f64 = 3.0;

    /// The stripe down the notice bar's leading edge, in pixels at 100%.
    const STRIPE_W: f64 = 3.0;

    /// The margin the role overlay reserves at a cell's leading edge for its own marker, in
    /// pixels at 100% — wide enough for the widest glyph [`grind_sheet::view::CellRole::marker`]
    /// returns, which is one character.
    const MARKER_W: f64 = 14.0;

    /// Everything the painter needs that is not geometry or colour.
    pub struct Frame<'a> {
        pub geom: &'a GridGeom,
        pub theme: Theme,
        pub viewport: &'a grind_sheet::Viewport,
        /// The status bar's left half: which document is open — `status::status_halves`.
        pub status: &'a str,
        /// Its right half: where the selection is and what it adds up to. Drawn from the right
        /// edge inwards, so it is the *sheet's name* that is elided on a narrow window rather
        /// than the arithmetic somebody is watching.
        pub status_right: &'a str,
        /// What the name box shows — an address, or the name of the range if it has one.
        ///
        /// Drawn here rather than read out of the child `EDIT` control, and that is deliberate
        /// on two counts: the box is a read-out until somebody types in it (the control is
        /// created hidden and shown over this rectangle on demand), and `--render-to` has no
        /// window at all, so anything only a control could draw would be missing from every
        /// rendered frame.
        pub name: &'a str,
        /// What the formula bar shows: the active cell's input text — a formula in display
        /// syntax, or the value as it would be typed back in.
        ///
        /// Drawn rather than held in a control, for the same two reasons the name box is: it is
        /// a read-out until somebody types in it, and `--render-to` has no window at all.
        pub formula: &'a str,
        /// Whether [`Frame::formula`] is the friendly *reading* of a formula rather than the text
        /// that would be typed back in — which the bar says with its own badge rather than by
        /// changing the ink, since a reading nobody can tell from an input is worse than no
        /// reading at all.
        pub friendly: bool,
        /// The notice bar, or `None` when there is nothing to say. Its *height* is the
        /// geometry's, so a frame whose banner text is `Some` and whose `banner_h` is zero draws
        /// nothing — the two are set together by the window.
        pub banner: Option<&'a str>,
        /// The assist band's runs, empty when there is nothing to assist with — and paired with
        /// `geom.hint_h` exactly as the notice bar is with `banner_h`.
        pub hint: &'a [Piece],
        /// The selection, which is presentation state and never leaves the shell.
        pub selection: Selection,
        /// How far the document's own content reaches — `App::used_extent`, the same answer the
        /// status bar reports. Past it the hairlines are drawn quieter (`Theme::grid_line_soft`).
        pub used: (u32, u32),
        /// The size a **cell's** text is drawn at, already scaled for this monitor's DPI.
        pub font_px: i32,
        /// The two chrome sizes, scaled the same way — `theme::text::CAPTION` for the header
        /// bands and the status bar, `theme::text::BODY` for the strip and the two message bands.
        ///
        /// A ramp rather than one number (W10): a status bar and a formula bar are not the same
        /// kind of text, and a window that sets them identically reads as undesigned.
        pub caption_px: i32,
        pub body_px: i32,
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

        // The two grounds (W10). The window is the *backdrop* and the grid is a **layer on it**:
        // the paper the document is written on is a different surface from the chrome round it,
        // which is Fluent's layering and the single change that most stops this window reading as
        // a toolbar stack with a grid underneath.
        //
        // The header bands and the status bar are painted *after* the cells rather than before,
        // so that a cell scrolled under a header cannot show through it — which is cheaper than
        // clipping the cell loop and is why they are not painted here as well.
        gdi::fill(
            dc,
            0,
            0,
            g.width.round() as i32,
            g.height.round() as i32,
            theme.backdrop,
        );
        {
            let (left, top, right, bottom) = body.edges();
            gdi::fill(dc, left, top, right, bottom, theme.background);
        }

        let regular = Font::new(frame.face, frame.font_px, false);
        let bold = Font::new(frame.face, frame.font_px, true);
        // The chrome's own two sizes — see `Frame::caption_px`. Built once per frame, like every
        // other font here, because creating one per label would be visible on every keystroke.
        let caption = Font::new(frame.face, frame.caption_px, false);
        let caption_bold = Font::new(frame.face, frame.caption_px, true);
        let body_font = Font::new(frame.face, frame.body_px, false);
        // `doc/view-modes.md`'s role overlay, smaller than the cell's own text so the marker
        // reads as a margin note rather than a second value — `ui_sheet_gtk`'s own glyph is
        // drawn at 0.7 of the cell's face for the same reason.
        let marker_font = Font::new(
            frame.face,
            (f64::from(frame.font_px) * 0.7).round() as i32,
            false,
        );
        let marker_w = crate::sheet::geom::scale(MARKER_W, g.dpi).round() as i32;

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
                    // that adjacent cells share one line rather than drawing two. Past the used
                    // extent they are drawn quieter — the empty rest of a sixteen-thousand-column
                    // sheet is paper, and the part somebody wrote on is the subject.
                    let (used_rows, used_cols) = frame.used;
                    let line = match row < used_rows && col < used_cols {
                        true => theme.grid_line,
                        false => theme.grid_line_soft,
                    };
                    gdi::fill(dc, right - 1, top, right, bottom, line);
                    gdi::fill(dc, left, bottom - 1, right, bottom, line);

                    // The role overlay reserves a margin at the cell's leading edge for its own
                    // marker rather than drawing over whatever the cell already shows — a label
                    // cell is left-aligned text and the two would otherwise collide. `role` is
                    // `None` both for a plain read and for an overlay that found nothing to say
                    // about an empty cell, and neither wants the margin.
                    let role = frame
                        .viewport
                        .role(row, col)
                        .filter(|r| !r.marker().is_empty());
                    let text_left = match role {
                        Some(_) => left + marker_w,
                        None => left,
                    };

                    if let Some(role) = role
                        && let Some(colour) = crate::theme::role_color(role, theme)
                    {
                        let _marker_font = Selected::font(dc, &marker_font);
                        draw_text(
                            dc,
                            role.marker(),
                            left,
                            top,
                            text_left,
                            bottom,
                            Align::Left,
                            colour,
                            crate::sheet::geom::scale(2.0, g.dpi),
                        );
                    }

                    let Some(text) = frame.viewport.text(row, col) else {
                        continue;
                    };
                    if text.is_empty() {
                        continue;
                    }
                    let _bold = look.bold.then(|| Selected::font(dc, &bold));
                    // A cell with no colour of its own gets ODF's *automatic* — the theme's ink
                    // where that reads on whatever ground this cell ended up with, and black or
                    // white where it does not. See `theme::automatic_ink`: a document that fills
                    // its heading row and leaves the text alone is the ordinary case, and in a
                    // dark palette the theme's near-white on that fill is unreadable.
                    let ink = look.text.unwrap_or_else(|| {
                        crate::theme::automatic_ink(
                            super::ground(look.background, theme, selected, active)
                                .unwrap_or(theme.background),
                            theme,
                        )
                    });
                    draw_text(
                        dc,
                        text,
                        text_left,
                        top,
                        right,
                        bottom,
                        look.align,
                        ink,
                        crate::sheet::geom::scale(PAD, g.dpi),
                    );
                }
            }
        }

        // `doc/view-modes.md`'s name overlay: where a defined name anchors, outlined if it
        // covers more than one cell. Drawn after every cell so the outline sits on the grid
        // lines the way the selection's own does, and before the headers for the same reason.
        draw_names(dc, frame);

        // The outline round the selected rectangle, drawn after the cells so it sits on top of
        // their hairlines, and before the headers so those still cover it where it runs under
        // the band.
        outline(dc, frame);

        // The headers, over the cells — a cell scrolled under the header band must not show
        // through it, and drawing them second is cheaper than clipping the loop above.
        {
            let _font = Selected::font(dc, &caption);
            gdi::fill(
                dc,
                0,
                g.header_top().round() as i32,
                g.width.round() as i32,
                (g.header_top() + g.header_h).round() as i32,
                theme.backdrop,
            );
            gdi::fill(
                dc,
                0,
                g.header_top().round() as i32,
                g.header_w.round() as i32,
                (body.y + body.h).round() as i32,
                theme.backdrop,
            );
            let (start, end) = frame.selection.rect();
            // A selected track's own button is marked twice: a tinted ground, and a bar of the
            // accent along the edge it shares with the grid. The bar is what makes a wide
            // selection legible at a glance — a tint alone is a colour, and a colour on a band
            // that is already a colour reads as nothing much.
            let bar = crate::sheet::geom::scale(HEADER_BAR, g.dpi)
                .round()
                .max(1.0) as i32;
            for col in g.visible_cols() {
                let rect = g.col_header_rect(col);
                if rect.w <= 0.0 {
                    continue;
                }
                let (left, top, right, bottom) = rect.edges();
                let active = (start.col..=end.col).contains(&col);
                if active {
                    gdi::fill(dc, left, top, right, bottom, theme.accent_soft);
                    gdi::fill(dc, left, bottom - bar, right, bottom, theme.accent);
                }
                gdi::fill(dc, right - 1, top, right, bottom, theme.divider);
                let _weight = active.then(|| Selected::font(dc, &caption_bold));
                draw_text(
                    dc,
                    &grind_sheet::formula::lex::column_name(col),
                    left,
                    top,
                    right,
                    bottom,
                    Align::Center,
                    header_ink(theme, active),
                    0.0,
                );
            }
            for row in g.visible_rows() {
                let rect = g.row_header_rect(row);
                if rect.h <= 0.0 {
                    continue;
                }
                let (left, top, right, bottom) = rect.edges();
                let active = (start.row..=end.row).contains(&row);
                if active {
                    gdi::fill(dc, left, top, right, bottom, theme.accent_soft);
                    gdi::fill(dc, right - bar, top, right, bottom, theme.accent);
                }
                gdi::fill(dc, left, bottom - 1, right, bottom, theme.divider);
                let _weight = active.then(|| Selected::font(dc, &caption_bold));
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
                    header_ink(theme, active),
                    0.0,
                );
            }
            // The line closing the header band, and the corner button.
            gdi::fill(
                dc,
                0,
                (g.header_top() + g.header_h - 1.0).round() as i32,
                g.width.round() as i32,
                (g.header_top() + g.header_h).round() as i32,
                theme.divider,
            );
            gdi::fill(
                dc,
                (g.header_w - 1.0).round() as i32,
                g.header_top().round() as i32,
                g.header_w.round() as i32,
                (body.y + body.h).round() as i32,
                theme.divider,
            );
            corner(dc, frame);
        }

        // The strip along the top, with the two read-outs in it: where the selection is, and
        // what is in the cell.
        //
        // No line under it, and that is W10 rather than an omission: the strip, the header band
        // and the status bar are all one surface now — the *backdrop* — and the only boundary
        // worth drawing is the one where the document's own paper starts, which the header band
        // closes above. A hairline between two bands of the same colour is a seam, not an edge.
        {
            let _font = Selected::font(dc, &body_font);
            let strip = g.strip_rect();
            let (left, top, right, bottom) = strip.edges();
            gdi::fill(dc, left, top, right, bottom, theme.backdrop);

            let pad = crate::sheet::geom::scale(PAD, g.dpi);
            let radius = crate::sheet::geom::scale(FIELD_RADIUS, g.dpi).round() as i32;
            let name = g.name_box_rect();
            if name.w > 0.0 {
                let (left, top, right, bottom) = name.edges();
                gdi::round_rect(
                    dc,
                    RECT {
                        left,
                        top,
                        right,
                        bottom,
                    },
                    radius,
                    theme.card,
                    theme.stroke,
                );
                draw_text(
                    dc,
                    frame.name,
                    left,
                    top,
                    right,
                    bottom,
                    Align::Left,
                    theme.text,
                    pad + f64::from(radius),
                );
            }

            let bar = g.formula_rect();
            if bar.w > 0.0 {
                let (left, top, right, bottom) = bar.edges();
                gdi::round_rect(
                    dc,
                    RECT {
                        left,
                        top,
                        right,
                        bottom,
                    },
                    radius,
                    theme.card,
                    theme.stroke,
                );
                // The `fx` badge, which is the one piece of ornament on this strip and earns it
                // twice: it says which of the two fields is the formula bar, and — drawn in the
                // accent — that what follows is the *friendly reading* of a formula rather than
                // the text that would be typed back in.
                let badge = crate::sheet::geom::scale(BADGE_W, g.dpi).round() as i32;
                let ink = match frame.friendly {
                    true => theme.accent,
                    false => theme.text_tertiary,
                };
                {
                    let italic = Font::styled(frame.face, frame.body_px, false, true, false, false);
                    let _badge_font = Selected::font(dc, &italic);
                    draw_text(
                        dc,
                        "fx",
                        left,
                        top,
                        left + badge,
                        bottom,
                        Align::Center,
                        ink,
                        0.0,
                    );
                }
                gdi::fill(
                    dc,
                    left + badge,
                    top + 1 + radius / 2,
                    left + badge + 1,
                    bottom - 1 - radius / 2,
                    theme.stroke,
                );
                draw_text(
                    dc,
                    frame.formula,
                    left + badge,
                    top,
                    right,
                    bottom,
                    Align::Left,
                    theme.text,
                    pad,
                );
            }
        }

        // The notice bar, if there is one. Under the strip and over the grid, which is where
        // the eye goes next after the thing that caused it.
        //
        // **An inset card since W10, not a stripe across the window** — Fluent's `InfoBar` is a
        // rounded rectangle with a margin round it, and the difference is most of why the band
        // used to read as one more toolbar. The stripe down its leading edge survives the change
        // and is what the two panes still have in common.
        if let Some(notice) = frame.banner.filter(|_| g.banner_h > 0.0) {
            let _font = Selected::font(dc, &body_font);
            let card = g.card_in(g.banner_rect());
            let (left, top, right, bottom) = card.edges();
            let stripe = crate::sheet::geom::scale(STRIPE_W, g.dpi).round().max(1.0) as i32;
            let radius = crate::sheet::geom::scale(FIELD_RADIUS, g.dpi).round() as i32;
            gdi::round_rect(
                dc,
                RECT {
                    left,
                    top,
                    right,
                    bottom,
                },
                radius,
                theme.banner,
                theme.banner_edge.blend(theme.banner, 0.55),
            );
            // Inside the rounded corner rather than on it: a bar drawn at the card's own edge
            // would be cut by the corner and leave two nicks.
            gdi::fill(
                dc,
                left + 1,
                top + radius,
                left + 1 + stripe,
                bottom - radius,
                theme.banner_edge,
            );
            draw_text(
                dc,
                notice,
                left + stripe,
                top,
                right,
                bottom,
                Align::Left,
                theme.banner_text,
                crate::sheet::geom::scale(BAND_PAD, g.dpi),
            );
        }

        // The assist band, if a formula is being typed. Under the notice bar rather than
        // instead of it: a formula that would not parse leaves a notice up *while the edit is
        // still open*, which is exactly when a signature hint is worth most. An inset card too,
        // and a quieter one — help while typing is not a state the document is in.
        if g.hint_h > 0.0 && !frame.hint.is_empty() {
            let card = g.card_in(g.hint_rect());
            let (left, top, right, bottom) = card.edges();
            let radius = crate::sheet::geom::scale(FIELD_RADIUS, g.dpi).round() as i32;
            gdi::round_rect(
                dc,
                RECT {
                    left,
                    top,
                    right,
                    bottom,
                },
                radius,
                theme.hint,
                theme.accent.blend(theme.hint, 0.78),
            );
            runs(dc, frame, left, top, right, bottom, &body_font, &bold);
        }

        // The status bar: which sheet on the left, where the selection is and what it adds up to
        // on the right. Two ends rather than one long line, because the left half changes when
        // the document does and the right half on every keystroke — and an eye that knows which
        // side a number is on does not have to read the whole bar to find it.
        {
            let _font = Selected::font(dc, &caption);
            let rect = g.status_rect();
            let (left, top, right, bottom) = rect.edges();
            let pad = crate::sheet::geom::scale(BAND_PAD, g.dpi);
            // No hairline over it, for the same reason the strip has none under it: the grid's
            // paper stops exactly here and the change of surface *is* the edge.
            gdi::fill(dc, left, top, right, bottom, theme.backdrop);
            // The right half is measured and placed first, and the left half is given what is
            // left over — so the two can never overlap, and it is the *sheet's name* that is
            // elided on a narrow window rather than the arithmetic.
            //
            // Two pads and two pixels of slack, which is not arithmetic for its own sake:
            // `draw_text` insets its rectangle by the padding at both ends and by one further
            // pixel, so a split placed at exactly the measured width leaves the string one pixel
            // short of fitting and `DT_END_ELLIPSIS` replaces the whole of it with `…`. The
            // rendered frame said so before this comment did.
            let pad_px = pad.round() as i32;
            let width = gdi::text_width(dc, frame.status_right);
            let split = (right - width - pad_px * 2 - 2).max(left);
            draw_text(
                dc,
                frame.status_right,
                split,
                top,
                right,
                bottom,
                Align::Right,
                theme.text_secondary,
                pad,
            );
            draw_text(
                dc,
                frame.status,
                left,
                top,
                split,
                bottom,
                Align::Left,
                theme.text_tertiary,
                pad,
            );
        }
    }

    /// A header button's lettering: the theme's own, or the accent when its track is selected.
    fn header_ink(theme: Theme, active: bool) -> Rgb {
        match active {
            true => theme.accent,
            false => theme.text_secondary,
        }
    }

    /// The select-all button in the corner of the two header bands.
    ///
    /// A triangle in the corner it points into, which is the convention every spreadsheet uses
    /// and none of them owns. Drawn as rows of a filled right triangle rather than with
    /// `Polygon`, because GDI would antialias neither and this needs no pen, no brush and no
    /// second code path for the DIB target.
    fn corner(dc: HDC, frame: &Frame) {
        let g = frame.geom;
        let size = crate::sheet::geom::scale(7.0, g.dpi).round().max(3.0) as i32;
        let inset = crate::sheet::geom::scale(4.0, g.dpi).round().max(1.0) as i32;
        let right = g.header_w.round() as i32 - inset;
        let bottom = (g.header_top() + g.header_h).round() as i32 - inset;
        let ink = frame.theme.text_tertiary;
        // Widest at the bottom, so the right angle is the corner it sits in and the hypotenuse
        // faces the grid. Drawn the other way up first, which looked like a mistake because it
        // was one.
        for step in 0..size {
            gdi::fill(
                dc,
                right - (size - step),
                bottom - 1 - step,
                right,
                bottom - step,
                ink,
            );
        }
    }

    /// The assist band's runs, laid left to right and stopping at the band's own edge.
    ///
    /// Measured and drawn with the same two fonts (`gdi::text_width` then `gdi::text_at`), which
    /// is the rule `metrics.rs` follows for the text pane: a run placed by one engine and drawn
    /// by another lands somewhere else.
    #[allow(clippy::too_many_arguments)]
    fn runs(
        dc: HDC,
        frame: &Frame,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        regular: &Font,
        bold: &Font,
    ) {
        let theme = frame.theme;
        let pad = crate::sheet::geom::scale(BAND_PAD, frame.geom.dpi).round() as i32;
        let edge = right - pad;
        let mut x = left + pad;
        for Piece { text, ink } in frame.hint {
            if x >= edge {
                return;
            }
            let colour = match ink {
                Ink::Plain => theme.hint_text,
                Ink::Muted => theme.hint_text.blend(theme.hint, 0.45),
                Ink::Strong => theme.accent,
            };
            let _font = Selected::font(
                dc,
                match ink {
                    Ink::Strong => bold,
                    _ => regular,
                },
            );
            // Measured in the font it is about to be drawn in, and drawn through the same
            // rectangle-and-ellipsis path every other label here uses — so the run that reaches
            // the band's edge is cut with an `…` rather than running off the window, and every
            // run is centred by one rule rather than two. The two extra pixels are slack for
            // `draw_text`'s own one-pixel inset, which would otherwise elide a run that fits.
            let width = gdi::text_width(dc, text);
            draw_text(
                dc,
                text,
                x,
                top,
                (x + width + 2).min(edge),
                bottom,
                Align::Left,
                colour,
                0.0,
            );
            x += width;
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
        let edge = frame.theme.accent;
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

    /// `doc/view-modes.md`'s name overlay: an outline round every defined name's range, in the
    /// theme's own ink moved towards the ground — muted rather than an accent, since this is a
    /// label on the document's own structure and not a thing to act on the way the selection is.
    /// Drawn the same whether the name covers one cell or many: a name binds to a *range*, and a
    /// single cell is simply the range that happens to be one wide.
    ///
    /// `Viewport::names` is empty whenever the read did not ask for it, so this has nothing to
    /// do and returns at once when the mode is off — the same "asked for fresh, never stored"
    /// shape [`grind_sheet::Viewport::role`] follows.
    fn draw_names(dc: HDC, frame: &Frame) {
        let g = frame.geom;
        let body = g.body();
        let clamp = |v: f64, low: f64, high: f64| v.clamp(low, high).round() as i32;
        let muted = frame.theme.text.blend(frame.theme.background, 0.55);
        let weight = crate::sheet::geom::scale(1.0, g.dpi).round().max(1.0) as i32;
        for anchor in frame.viewport.names() {
            let first = g.cell_rect(anchor.rows.start, anchor.cols.start);
            let last = g.cell_rect(anchor.rows.end - 1, anchor.cols.end - 1);
            let left = clamp(first.x, body.x, body.x + body.w);
            let top = clamp(first.y, body.y, body.y + body.h);
            let right = clamp(last.x + last.w, body.x, body.x + body.w);
            let bottom = clamp(last.y + last.h, body.y, body.y + body.h);
            if right <= left || bottom <= top {
                continue;
            }
            gdi::fill(dc, left, top, right, top + weight, muted);
            gdi::fill(dc, left, bottom - weight, right, bottom, muted);
            gdi::fill(dc, left, top, left + weight, bottom, muted);
            gdi::fill(dc, right - weight, top, right, bottom, muted);
        }
    }

    /// One string, clipped to a rectangle, vertically centred and elided if it does not fit.
    ///
    /// `DT_END_ELLIPSIS` rather than letting the text run into the next cell: overflow into an
    /// empty neighbour is a real spreadsheet behaviour and a named gap here rather than a
    /// half-done one — it needs to know whether the neighbour is empty *and* to draw outside its
    /// own cell's rectangle, which is W3's problem, not W1's.
    /// Public to the crate because the text pane's chrome — its status bar and its notice bar —
    /// is the same one line of text in a rectangle, and two spellings of "clipped, with an
    /// ellipsis, in the theme's ink" is one too many.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
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
        // Nothing to draw is a *return*, not a zero-length `DrawTextW`, and this one cost an
        // afternoon: an empty `Vec<u16>` has no allocation, so `as_mut_ptr` hands back the
        // dangling well-aligned address `2`, and Wine's `DrawTextW` dereferences the buffer
        // before it looks at the count. The crash was an access violation reading address 2,
        // inside a system DLL, with nothing of ours on the faulting frame. W1 and W2 never hit
        // it because every string they drew had something in it; W3's formula bar is empty
        // whenever the active cell is.
        if wide.is_empty() {
            return;
        }
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
///
/// Every band here grew by two to four pixels in W9 and again in W10, and the reason is the same
/// both times: the chrome was drawn at the smallest size its text fits in, which is a different
/// thing from the size it reads at. W10's numbers are Fluent's own — a 32-pixel control with a
/// 6-pixel surround is a 44-pixel strip — rather than a judgement about how it looks. Nothing
/// else had to move, because every rectangle in `geom.rs` is expressed in these.
pub const HEADER_H: f64 = 26.0;
pub const HEADER_W: f64 = 48.0;
/// The status bar's height at 100%.
pub const STATUS_H: f64 = 28.0;
/// The strip along the top that holds the name box and the formula bar: one Fluent control
/// (`theme::space::CONTROL_H`) with a `GAP`-and-a-half either side of it.
pub const STRIP_H: f64 = 44.0;
/// The notice bar's height at 100%, when there is a notice. One line of the shell font with
/// room to breathe — a banner that needs two lines is a banner saying too much — plus the margin
/// that makes it an inset card rather than a stripe across the window.
pub const BANNER_H: f64 = 36.0;
/// The assist band's height at 100%, when a formula is being typed. One line, like the notice
/// bar, and for the same reason; a shade shorter, because a hint is quieter than a state.
pub const HINT_H: f64 = 34.0;
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
