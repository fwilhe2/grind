// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A viewport painted onto a device context. **What a cell looks like is decided in portable
//! code** ([`Appearance::of`], tested on any host); only putting pixels down needs Windows.
//! `paint` takes an `HDC` and a `Frame` and nothing about the window, so `--render-to` is a second
//! caller rather than a second drawing path (decision 5).

use grind_sheet::look;
use grind_sheet::model::CellValue;
use grind_sheet::style::CellStyle;

use crate::theme::{Rgb, Theme};

use super::geom::GridGeom;
use super::keymap::Selection;

/// How far a selected cell's ground moves towards [`Theme::selection`]: at 0.22 a document's red
/// is still red and visibly selected. GDI has no alpha, so [`Rgb::blend`] applies it before
/// painting — see [`ground`].
pub const WASH: f64 = 0.22;

/// The colour a cell's ground is painted once the selection is taken into account; `None` is
/// "paint nothing", the window's background being down already. The **active cell is never
/// washed**, even inside a selection, so it reads as the cursor's cell.
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

/// Which end of the cell the text sits at — `grind_sheet::look`'s, which also decides it.
pub use grind_sheet::look::Align;

/// How one cell is drawn, resolved from what the document says and what it leaves open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    pub align: Align,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// `fo:margin-left`, in millimetres — zero for none.
    pub indent_mm: f64,
    /// `None` means "the theme's ink" — a cell without a colour of its own has to follow the
    /// user's light/dark choice, and baking the theme's value in here would make a document
    /// that was opened in light mode unreadable when the theme changed under it.
    pub text: Option<Rgb>,
    pub background: Option<Rgb>,
}

impl Appearance {
    /// Which of the sixteen faces a cell's text is drawn in — bold, italic, underlined and
    /// struck through being four switches of one `LOGFONTW` — as an index into a frame's
    /// `faces` array in `paint`. Zero is the regular face.
    pub fn face(&self) -> usize {
        usize::from(self.bold)
            | usize::from(self.italic) << 1
            | usize::from(self.underline) << 2
            | usize::from(self.strike) << 3
    }

    /// What the document asks for, with the spreadsheet's defaults underneath. Where text sits and
    /// whether it is bold or italic are `grind_sheet::look`'s — numbers right, text left, a
    /// boolean or an error centred, unless the style says otherwise; the colour, parsed into GDI's
    /// [`Rgb`], is this shell's.
    pub fn of(value: &CellValue, style: Option<&CellStyle>) -> Self {
        Self {
            align: look::align(value, style),
            bold: look::is_bold(style),
            italic: look::is_italic(style),
            underline: style.is_some_and(CellStyle::is_underlined),
            strike: style.is_some_and(CellStyle::is_struck),
            indent_mm: style.and_then(CellStyle::indent_mm).unwrap_or(0.0),
            text: style.and_then(|s| s.color.as_deref()).and_then(Rgb::parse),
            // `transparent` is a real value and it means *no fill*, not black.
            background: match style.and_then(|s| s.background.as_deref()) {
                Some("transparent") | None => None,
                Some(hex) => Rgb::parse(hex),
            },
        }
    }
}

/// One cell's text as a single line. A cell's text can hold a line break (`text:line-break`, kept
/// as `\n`), which `DT_SINGLELINE` draws as a box glyph, so every control character becomes a
/// space, keeping the two sides two words. A wrapping cell is drawn line by line elsewhere.
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

/// Where text `w` by `h` turned `degrees` anticlockwise starts — its unturned top-left corner —
/// so that its middle is at `(cx, cy)`, in a y-down space.
pub fn turned_origin(w: f64, h: f64, (cx, cy): (f64, f64), degrees: f64) -> (f64, f64) {
    let (sin, cos) = degrees.to_radians().sin_cos();
    (
        cx - w / 2.0 * cos - h / 2.0 * sin,
        cy + w / 2.0 * sin - h / 2.0 * cos,
    )
}

#[cfg(windows)]
pub use windows_impl::{FormatStrip, Frame, draw_text, paint};

#[cfg(windows)]
mod windows_impl {
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DrawTextW, HDC,
        SetBkMode, SetTextColor, TRANSPARENT,
    };

    use grind_core::style::TextStyle;
    use grind_sheet::look;
    use grind_sheet::model::CellValue;
    use grind_sheet::style::CellStyle;

    use super::{Align, Appearance, GridGeom, Selection, Theme};
    use crate::gdi::{self, Font, Selected};
    use crate::sheet::assist::{Ink, Piece};
    use crate::sheet::format::{self, Control, Shape};
    use crate::strip;
    use crate::theme::Interaction;
    use crate::theme::Rgb;

    /// The padding between a cell's edge and its text, in pixels at 100%. Not raised with the
    /// bands: at six pixels `3,710.00 €` stopped fitting the column the document sized for it. A
    /// band's padding costs a band; a cell's is paid in the document's numbers.
    const PAD: f64 = 4.0;

    /// A wrapping cell's text: `grind_core::layout::wrap` breaks it at the column's width, each
    /// line is aligned across the cell as one line would be, and the block of lines is placed down
    /// it by the cell's vertical alignment. Drawn inside a clip of the cell.
    fn draw_wrapped(
        dc: HDC,
        frame: &Frame,
        (text, misspelt): (&str, &[std::ops::Range<usize>]),
        value: &CellValue,
        style: Option<&CellStyle>,
        cell: (i32, i32, i32, i32),
        ink: Rgb,
    ) {
        use windows::Win32::Graphics::Gdi::{IntersectClipRect, RestoreDC, SaveDC};
        let (left, top, right, bottom) = cell;
        let dpi = frame.geom.dpi;
        let metrics = crate::sheet::measure::CellMetrics {
            dc,
            face: frame.face,
            dpi,
            default_pt: f64::from(frame.font_px) * 72.0 / (f64::from(dpi) * frame.geom.zoom),
            zoom: frame.geom.zoom,
        };
        let pad = crate::sheet::geom::scale(PAD, dpi).round();
        let text_style = look::text_style(style);
        let room = (f64::from(right - left) - 2.0 * pad).max(1.0);
        let laid = grind_core::layout::wrap(
            &[grind_core::layout::Fragment {
                text,
                style: &text_style,
            }],
            room as f32,
            &metrics,
        );
        let block = f64::from(laid.height());
        let y0 = match look::valign(style) {
            look::VAlign::Top => f64::from(top) + 2.0,
            look::VAlign::Middle => f64::from(top) + (f64::from(bottom - top) - block) / 2.0,
            look::VAlign::Bottom => f64::from(bottom) - 2.0 - block,
        };
        let font = metrics.font(&text_style);
        let _selected = Selected::font(dc, &font);
        // SAFETY: the DC is live; the clip is undone by the matching `RestoreDC`.
        let saved = unsafe {
            let saved = SaveDC(dc);
            IntersectClipRect(dc, left, top, right, bottom);
            saved
        };
        for line in laid.lines() {
            let piece: String = text
                .chars()
                .skip(line.start)
                .take(line.end.saturating_sub(line.start))
                .collect::<String>()
                .trim_end()
                .to_owned();
            let line_top = (y0 + f64::from(line.top)).round() as i32;
            let line_bottom = line_top + metrics_line_height(&metrics, &text_style);
            draw_text(
                dc,
                &piece,
                left,
                line_top,
                right,
                line_bottom,
                look::align(value, style),
                ink,
                pad,
            );
            // Each misspelt word's part on this line, counted from the line's own start.
            let here: Vec<std::ops::Range<usize>> = misspelt
                .iter()
                .filter_map(|m| {
                    let (start, end) = (m.start.max(line.start), m.end.min(line.end));
                    (start < end).then(|| start - line.start..end - line.start)
                })
                .collect();
            draw_squiggles(
                dc,
                &piece,
                &here,
                (left, line_top, right, line_bottom),
                look::align(value, style),
                pad,
                frame,
            );
        }
        // SAFETY: restoring the state saved above.
        unsafe {
            let _ = RestoreDC(dc, saved);
        }
    }

    fn metrics_line_height(metrics: &crate::sheet::measure::CellMetrics, style: &TextStyle) -> i32 {
        use grind_core::layout::Metrics;
        metrics.line_height(style).round() as i32
    }

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

    /// What the format strip shows: the active cell's style and number format, read fresh for this
    /// frame, and where the pointer is on the strip — presentation state.
    pub struct FormatStrip<'a> {
        pub style: &'a grind_sheet::style::CellStyle,
        /// The number picker's face — `sheet::format::face`, already worked out by the window.
        pub number: &'a str,
        /// Whether that face is a format rather than *General*: a picker with nothing set shows
        /// its placeholder in the tertiary ink, the text strip's own "Font" rule.
        pub number_set: bool,
        pub hover: Option<Control>,
        pub pressed: Option<Control>,
    }

    /// Everything the painter needs that is not geometry or colour.
    pub struct Frame<'a> {
        pub geom: &'a GridGeom,
        pub theme: Theme,
        /// The format strip across the top — `sheet/format.rs`'s controls over the active cell.
        pub format: FormatStrip<'a>,
        pub viewport: &'a grind_sheet::Viewport,
        /// The sheets' names, in order, and which one is on screen — the tabs the status bar's
        /// left half is (`super::tabs`).
        pub sheets: &'a [String],
        pub sheet: usize,
        /// Its right half: where the selection is and what it adds up to. Drawn from the right
        /// edge inwards, so it is the *sheet's name* that is elided on a narrow window rather
        /// than the arithmetic somebody is watching.
        pub status_right: &'a str,
        /// At the very end, past it: the spelling language in letters — `ALL`, `DE`, `Off` —
        /// since GDI draws no colour glyph and Windows has no flag emoji to draw anyway
        /// (`grind_spell::Setting::chip`). A click there chooses one.
        pub status_picker: &'a str,
        /// What the name box shows — an address, or the range's name. Drawn here rather than by
        /// the child `EDIT`, which is shown over this rectangle only while somebody types in it,
        /// and because `--render-to` has no window to put a control in.
        pub name: &'a str,
        /// What the formula bar shows: the active cell's input text, drawn rather than held in a
        /// control for the name box's two reasons.
        pub formula: &'a str,
        /// Whether [`Frame::formula`] is the friendly *reading* of a formula rather than the text
        /// that would be typed back in — said by the bar's badge, since a reading nobody can tell
        /// from an input is worse than none.
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
        /// The sheet's autofilter, if any — `App::filter`, drawn as a dropdown button per heading
        /// cell (`GridGeom::filter_button`). The rows it hides are already folded out of
        /// `geom.rows`.
        pub filter: Option<grind_sheet::Filter>,
        /// The sheet's charts and, for each, the data it draws — read fresh for this frame like
        /// everything else here (`sheet/chart.rs`).
        pub charts: &'a [grind_sheet::Chart],
        /// With View ▸ Show Formulas, the formula text of each formula cell in view, which is drawn
        /// in place of the result.
        pub formula_text: &'a std::collections::HashMap<(u32, u32), String>,
        pub chart_data: &'a [Option<grind_sheet::ChartData>],
        /// The selected chart, which wears the accent and its eight handles, and the chart the
        /// pointer is holding, drawn where the drag has put it (`sheet/chart.rs`).
        pub chart_selected: Option<usize>,
        pub chart_grab: Option<crate::sheet::chart::Grab>,
        /// How far the document's own content reaches — `App::used_extent`, the same answer the
        /// status bar reports. Past it the hairlines are drawn quieter (`Theme::grid_line_soft`).
        pub used: (u32, u32),
        /// The size a **cell's** text is drawn at, already scaled for this monitor's DPI.
        pub font_px: i32,
        /// The two chrome sizes, scaled the same way: `theme::text::CAPTION` for the header bands
        /// and status bar, `theme::text::BODY` for the strip and the two message bands — a status
        /// bar and a formula bar are not the same kind of text.
        pub caption_px: i32,
        pub body_px: i32,
        pub face: &'a str,
    }

    /// Draw one frame of the grid onto `dc`, writing every pixel of the client area, so
    /// `WM_ERASEBKGND` can be answered "already done" (`gdi::BackBuffer`). Returns where the sheet
    /// tabs were drawn, which a click is tested against (`crate::sheet::tabs::hit`).
    pub fn paint(dc: HDC, frame: &Frame) -> Vec<crate::sheet::tabs::Tab> {
        let g = frame.geom;
        let theme = frame.theme;
        let body = g.body();

        // Two grounds: the window is the *backdrop* and the grid a **layer on it**, the document's
        // paper a different surface from the chrome round it. The header bands and status bar are
        // painted after the cells, so a cell scrolled under a header cannot show through.
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
        // Every other combination a cell can ask for, made the first time one does: a frame of
        // plain cells creates none of them.
        let faces: [std::cell::OnceCell<Font>; 16] = Default::default();
        let face = |look: &Appearance| -> Option<&Font> {
            match look.face() {
                0 => None,
                1 => Some(&bold),
                i => Some(faces[i].get_or_init(|| {
                    Font::styled(
                        frame.face,
                        frame.font_px,
                        look.bold,
                        look.italic,
                        look.underline,
                        look.strike,
                    )
                })),
            }
        };
        // The chrome's own two sizes — see `Frame::caption_px`. Built once per frame, like every
        // other font here, because creating one per label would be visible on every keystroke.
        let caption = Font::new(frame.face, frame.caption_px, false);
        let caption_bold = Font::new(frame.face, frame.caption_px, true);
        let body_font = Font::new(frame.face, frame.body_px, false);
        // The role overlay's marker, smaller than the cell's text, so it reads as a margin note
        // rather than a second value.
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
                    // A merge is drawn whole, below: one cell, with no hairline through it.
                    if frame.viewport.merge_at(row, col).is_some() {
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
                    // The grid's hairlines: every cell's right and bottom edges, so neighbours
                    // share one line. Past the used extent they are quieter — the empty rest of
                    // the sheet is paper.
                    let (used_rows, used_cols) = frame.used;
                    let line = match row < used_rows && col < used_cols {
                        true => theme.grid_line,
                        false => theme.grid_line_soft,
                    };
                    gdi::fill(dc, right - 1, top, right, bottom, line);
                    gdi::fill(dc, left, bottom - 1, right, bottom, line);

                    // A filtered heading's text ends where its button begins, rather than running
                    // under it (`Budgete▼`) — the same fix every other window in the suite made.
                    let mut text_right = right;
                    if let Some(filter) = &frame.filter
                        && filter.buttons
                        && row == filter.start.row
                        && (filter.start.col..=filter.end.col).contains(&col)
                        && let Some(button) = g.filter_button(row, col)
                    {
                        draw_filter_button(dc, button, theme);
                        text_right = (button.x.round() as i32).max(left + 1);
                    }

                    // The role overlay reserves a margin at the cell's leading edge for its marker
                    // rather than drawing over a left-aligned label. `role` is `None` for a plain
                    // read and for an empty cell the overlay has nothing to say about; neither
                    // wants the margin.
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

                    let Some(text) = frame
                        .formula_text
                        .get(&(row, col))
                        .map(String::as_str)
                        .or_else(|| frame.viewport.text(row, col))
                    else {
                        continue;
                    };
                    if text.is_empty() {
                        continue;
                    }
                    let _face = face(&look).map(|font| Selected::font(dc, font));
                    // A cell with no colour of its own gets ODF's *automatic* — the theme's ink
                    // where it reads on this cell's ground, black or white where not
                    // (`theme::automatic_ink`), or a filled heading row's text vanishes in the
                    // dark palette. A colour it chose is lifted until it reads on the dark theme's
                    // ground (`theme::document_ink`), so navy stays visible.
                    let ink = crate::theme::document_ink(
                        look.text,
                        super::ground(look.background, theme, selected, active)
                            .unwrap_or(theme.background),
                        look.background.is_some(),
                        theme,
                    );
                    // A wrapping text cell is broken at its column's width by the core's breaker,
                    // measured in its own font, and drawn a line at a time; what the row cannot
                    // hold is cut by the cell.
                    let style = frame.viewport.style(row, col);
                    // Turned text is turned about the cell's middle and cut by the cell.
                    if let Some(degrees) = style.and_then(CellStyle::rotation_degrees) {
                        gdi::turned(dc, text, (left, top, text_right, bottom), degrees, ink);
                        continue;
                    }
                    if grind_sheet::look::wraps(style) && !grind_sheet::numfmt::is_number(value) {
                        let misspelt = match frame.formula_text.contains_key(&(row, col)) {
                            true => &[][..],
                            false => frame.viewport.misspelt(row, col),
                        };
                        draw_wrapped(
                            dc,
                            frame,
                            (text, misspelt),
                            value,
                            style,
                            (left, top, right, bottom),
                            ink,
                        );
                        continue;
                    }
                    // A number that does not fit is **never** elided — `DrawTextW`'s ellipsis made
                    // `3,710.00 €` into `3,710.0…`, a magnitude short a digit. Measured in the
                    // cell's own font against exactly the room `draw_text` leaves, and drawn as
                    // `numfmt::overflow`'s hashes.
                    let pad = crate::sheet::geom::scale(PAD, g.dpi);
                    let room = f64::from(text_right - text_left) - 2.0 * pad.round() - 1.0;
                    let hashes;
                    let text = match grind_sheet::numfmt::is_number(value)
                        && f64::from(gdi::text_width(dc, text)) > room
                    {
                        true => {
                            // A plain number narrows first — fewer decimals, then scientific —
                            // as LibreOffice's General format does (`Viewport::narrower`).
                            let narrower = match frame.formula_text.contains_key(&(row, col)) {
                                true => Vec::new(),
                                false => frame.viewport.narrower(row, col),
                            };
                            hashes = narrower
                                .into_iter()
                                .find(|shorter| f64::from(gdi::text_width(dc, shorter)) <= room)
                                .unwrap_or_else(|| {
                                    let hash = f64::from(gdi::text_width(dc, "#"));
                                    grind_sheet::numfmt::overflow(room, hash)
                                });
                            hashes.as_str()
                        }
                        false => text,
                    };
                    // An indent (`fo:margin-left`) moves text in from the leading edge — not text
                    // set against the right one, which a left margin does not reach.
                    let text_left = match look.align {
                        Align::Right => text_left,
                        _ => {
                            let px = crate::sheet::geom::scale(look.indent_mm * 96.0 / 25.4, g.dpi);
                            (text_left + px.round() as i32).min(text_right)
                        }
                    };
                    draw_text(
                        dc, text, text_left, top, text_right, bottom, look.align, ink, pad,
                    );
                    // A misspelt word's squiggle — only under the text the core checked, never
                    // a formula shown in its place (`Viewport::misspelt`).
                    if !frame.formula_text.contains_key(&(row, col)) {
                        draw_squiggles(
                            dc,
                            text,
                            frame.viewport.misspelt(row, col),
                            (text_left, top, text_right, bottom),
                            look.align,
                            pad,
                            frame,
                        );
                    }
                }
            }
        }

        // Each merge, as one cell over its whole area: its top-left cell's ground, text and
        // look, whether or not that cell is in view, and its hairlines only round the outside.
        {
            let _font = Selected::font(dc, &regular);
            for m in frame.viewport.merges() {
                let area = merged_rect(frame, m);
                if area.w <= 0.0 || area.h <= 0.0 {
                    continue;
                }
                let (left, top, right, bottom) = area.edges();
                let look = Appearance::of(&m.value, m.style.as_ref());
                let (row, col) = (m.anchor.row, m.anchor.col);
                let selected = frame.selection.contains(row, col);
                let active = frame.selection.active == m.anchor;
                let ground = super::ground(look.background, theme, selected, active);
                if let Some(fill) = ground {
                    gdi::fill(dc, left, top, right, bottom, fill);
                }
                gdi::fill(dc, right - 1, top, right, bottom, theme.grid_line);
                gdi::fill(dc, left, bottom - 1, right, bottom, theme.grid_line);
                let text = frame
                    .formula_text
                    .get(&(row, col))
                    .map(String::as_str)
                    .unwrap_or(&m.text);
                if text.is_empty() {
                    continue;
                }
                let _face = face(&look).map(|font| Selected::font(dc, font));
                let ink = crate::theme::document_ink(
                    look.text,
                    ground.unwrap_or(theme.background),
                    look.background.is_some(),
                    theme,
                );
                let style = m.style.as_ref();
                let misspelt = match frame.formula_text.contains_key(&(row, col)) {
                    true => &[][..],
                    false => frame.viewport.misspelt(row, col),
                };
                if grind_sheet::look::wraps(style) && !grind_sheet::numfmt::is_number(&m.value) {
                    draw_wrapped(
                        dc,
                        frame,
                        (text, misspelt),
                        &m.value,
                        style,
                        (left, top, right, bottom),
                        ink,
                    );
                    continue;
                }
                let pad = crate::sheet::geom::scale(PAD, g.dpi);
                let room = f64::from(right - left) - 2.0 * pad.round() - 1.0;
                let hashes;
                let text = match grind_sheet::numfmt::is_number(&m.value)
                    && f64::from(gdi::text_width(dc, text)) > room
                {
                    true => {
                        let hash = f64::from(gdi::text_width(dc, "#"));
                        hashes = grind_sheet::numfmt::overflow(room, hash);
                        hashes.as_str()
                    }
                    false => text,
                };
                draw_text(dc, text, left, top, right, bottom, look.align, ink, pad);
                draw_squiggles(
                    dc,
                    text,
                    misspelt,
                    (left, top, right, bottom),
                    look.align,
                    pad,
                    frame,
                );
            }
        }

        // Each checkbox: a box in the ink on the paper, filled with the accent and ticked when
        // its linked cell says so.
        {
            let _font = Selected::font(dc, &bold);
            for &(pos, ticked) in frame.viewport.checkboxes() {
                let b = g.checkbox(pos.row, pos.col);
                if b.w < 4.0 {
                    continue;
                }
                let (left, top, right, bottom) = b.edges();
                gdi::fill(dc, left, top, right, bottom, theme.text);
                let inner = match ticked {
                    true => theme.accent,
                    false => theme.background,
                };
                gdi::fill(dc, left + 1, top + 1, right - 1, bottom - 1, inner);
                if ticked {
                    draw_text(
                        dc,
                        "\u{2713}",
                        left,
                        top,
                        right,
                        bottom,
                        Align::Center,
                        theme.background,
                        0.0,
                    );
                }
            }
        }

        // The cells' own borders, after every cell so a neighbour's fill cannot cover half a line
        // centred on their shared edge. `look::border_strokes` is the geometry, in points, so the
        // cell goes in as points and strokes come back to pixels; a hairline is one device pixel.
        {
            let to_pt = 72.0 / (f64::from(g.dpi) * g.zoom);
            let to_px = f64::from(g.dpi) * g.zoom / 72.0;
            let dark = theme.mode == crate::theme::Mode::Dark;
            // A merge's borders are its top-left cell's, round the whole of it.
            let plain = g.visible_rows().flat_map(|row| {
                g.visible_cols().filter_map(move |col| {
                    if frame.viewport.merge_at(row, col).is_some() {
                        return None;
                    }
                    Some((frame.viewport.style(row, col)?, g.cell_rect(row, col)))
                })
            });
            let merged = frame
                .viewport
                .merges()
                .iter()
                .filter_map(|m| Some((m.style.as_ref()?, merged_rect(frame, m))));
            for (style, rect) in plain.chain(merged) {
                {
                    if style.borders.iter().all(Option::is_none) {
                        continue;
                    }
                    for stroke in grind_sheet::look::border_strokes(
                        (
                            rect.x * to_pt,
                            rect.y * to_pt,
                            rect.w * to_pt,
                            rect.h * to_pt,
                        ),
                        style,
                        theme.background.into(),
                        dark,
                        to_pt,
                    ) {
                        let (x, y, w, h) = stroke.rect;
                        let left = (x * to_px).round() as i32;
                        let top = (y * to_px).round() as i32;
                        let right = ((x + w) * to_px).round().max(f64::from(left) + 1.0) as i32;
                        let bottom = ((y + h) * to_px).round().max(f64::from(top) + 1.0) as i32;
                        gdi::fill(dc, left, top, right, bottom, stroke.color.into());
                    }
                }
            }
        }

        // The charts float over the cells and their borders, under the overlays and the selection.
        crate::sheet::chart::paint(
            dc,
            g,
            theme,
            frame.face,
            frame.charts,
            frame.chart_data,
            frame.chart_grab.as_ref(),
        );

        // `doc/view-modes.md`'s name overlay: where a defined name anchors, outlined if it
        // covers more than one cell. Drawn after every cell so the outline sits on the grid
        // lines the way the selection's own does, and before the headers for the same reason.
        draw_names(dc, frame);

        // The outline round the selected rectangle, drawn after the cells so it sits on top of
        // their hairlines, and before the headers so those still cover it where it runs under
        // the band.
        outline(dc, frame);

        // The selected chart's outline and handles over everything in the cells, the cell
        // selection's outline included — a chart that is selected is the thing that is selected.
        let held = match frame.chart_grab {
            Some(grab) => Some(grab.now),
            None => frame.chart_selected.and_then(|index| {
                crate::sheet::chart::frames(frame.charts, g)
                    .get(index)
                    .copied()
                    .flatten()
            }),
        };
        if let Some(at) = held {
            crate::sheet::chart::paint_held(dc, g, theme, at);
        }

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
            // A selected track's button is marked twice: a tinted ground, and an accent bar along
            // the edge it shares with the grid — a tint on a band that is already a colour reads
            // as nothing much.
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

        draw_format(dc, frame);

        // The strip with its two read-outs: where the selection is, and what is in the cell. No
        // line under it: the strip, header band and status bar are one surface, the backdrop, and
        // the only edge worth drawing is where the document's paper starts.
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
                // The `fx` badge, the strip's one ornament: it says which field is the formula
                // bar, and — in the accent — that what follows is a formula's friendly reading
                // rather than its text.
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

        // The notice bar, under the strip and over the grid, where the eye goes after the thing
        // that caused it — an inset card (Fluent's `InfoBar`) with a stripe down its leading edge,
        // as in the text pane.
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

        // The assist band while a formula is typed, under the notice bar rather than instead of
        // it: a formula that will not parse leaves a notice up while the edit is open, when a
        // signature hint is worth most. A quieter card — help while typing is not a state of the
        // document.
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

        let tabs;
        // The status bar: sheet tabs on the left, where the selection is and what it adds up to on
        // the right — the left changes with the document, the right every keystroke.
        {
            let _font = Selected::font(dc, &caption);
            let rect = g.status_rect();
            let (left, top, right, bottom) = rect.edges();
            let pad = crate::sheet::geom::scale(BAND_PAD, g.dpi);
            // No hairline over it, for the same reason the strip has none under it: the grid's
            // paper stops exactly here and the change of surface *is* the edge.
            gdi::fill(dc, left, top, right, bottom, theme.backdrop);
            // The right half is measured and placed first and the tabs get what is left, so they
            // never overlap and a narrow window drops a tab rather than the arithmetic. Two pads
            // and two pixels of slack: `draw_text` insets by the padding and one more pixel, and a
            // split at exactly the measured width makes `DT_END_ELLIPSIS` replace the whole string
            // with `…`.
            let pad_px = pad.round() as i32;
            // The spelling picker first, at the very end, in the ink a control has rather than
            // the read-out's quieter one: it is the one thing on this side to press.
            let picker_w = gdi::text_width(dc, frame.status_picker) + pad_px * 2 + 2;
            let picker = (right - picker_w).max(left);
            draw_text(
                dc,
                frame.status_picker,
                picker,
                top,
                right,
                bottom,
                Align::Right,
                theme.text,
                pad,
            );
            let width = gdi::text_width(dc, frame.status_right);
            let split = (picker - width - pad_px * 2 - 2).max(left);
            draw_text(
                dc,
                frame.status_right,
                split,
                top,
                picker,
                bottom,
                Align::Right,
                theme.text_secondary,
                pad,
            );
            let mut laid = draw_tabs(dc, frame, rect, f64::from(split));
            laid.push(crate::sheet::tabs::Tab {
                target: crate::sheet::tabs::Target::Spelling,
                rect: crate::sheet::geom::Rect {
                    x: f64::from(picker),
                    y: f64::from(top),
                    w: f64::from(right - picker),
                    h: f64::from(bottom - top),
                },
            });
            tabs = laid;
        }
        tabs
    }

    /// The sheet tabs, from the status bar's left edge up to `limit` (`super::tabs`). The sheet on
    /// screen is a card with an accent bar under its name — Fluent's selected pivot, the mark the
    /// header buttons wear — and the others are names in the secondary ink. `+` is Add Sheet.
    fn draw_tabs(
        dc: HDC,
        frame: &Frame,
        bar: crate::sheet::geom::Rect,
        limit: f64,
    ) -> Vec<crate::sheet::tabs::Tab> {
        use crate::sheet::tabs::{self, Target};
        let theme = frame.theme;
        let dpi = frame.geom.dpi;
        let widths: Vec<f64> = frame
            .sheets
            .iter()
            .map(|name| f64::from(gdi::text_width(dc, name)))
            .collect();
        let laid = tabs::layout(&widths, bar, limit, f64::from(dpi) / 96.0);
        let radius = crate::sheet::geom::scale(4.0, dpi).round() as i32;
        let pad = crate::sheet::geom::scale(tabs::PAD, dpi);
        for tab in &laid {
            let (left, top, right, bottom) = tab.rect.edges();
            match tab.target {
                Target::Sheet(index) => {
                    let current = index == frame.sheet;
                    if current {
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
                        let bar_h = crate::sheet::geom::scale(3.0, dpi).round() as i32;
                        let inset = crate::sheet::geom::scale(12.0, dpi).round() as i32;
                        gdi::fill(
                            dc,
                            left + inset,
                            bottom - bar_h - 1,
                            right - inset,
                            bottom - 1,
                            theme.accent,
                        );
                    }
                    draw_text(
                        dc,
                        frame.sheets.get(index).map_or("", String::as_str),
                        left,
                        top,
                        right,
                        bottom,
                        Align::Center,
                        match current {
                            true => theme.text,
                            false => theme.text_secondary,
                        },
                        pad,
                    );
                }
                // Laid out by the status bar's own painter, never by `tabs::layout`.
                Target::Spelling => {}
                Target::Add => draw_text(
                    dc,
                    "+",
                    left,
                    top,
                    right,
                    bottom,
                    Align::Center,
                    theme.text_secondary,
                    0.0,
                ),
            }
        }
        laid
    }

    /// A header button's lettering: the theme's own, or the accent when its track is selected.
    fn header_ink(theme: Theme, active: bool) -> Rgb {
        match active {
            true => theme.accent,
            false => theme.text_secondary,
        }
    }

    /// One autofilter dropdown button — `GridGeom::filter_button`'s square with a downward
    /// triangle, stacked fills like [`corner`]'s, so this shell draws a triangle one way.
    fn draw_filter_button(dc: HDC, rect: crate::sheet::geom::Rect, theme: Theme) {
        let (left, top, right, bottom) = rect.edges();
        gdi::fill(dc, left, top, right, bottom, theme.card);
        let size = ((right - left) / 2).max(2);
        let cx = (left + right) / 2;
        let top2 = top + ((bottom - top) - size) / 2;
        for step in 0..size {
            let half = (size - step) / 2;
            gdi::fill(
                dc,
                cx - half,
                top2 + step,
                cx + half + 1,
                top2 + step + 1,
                theme.text_tertiary,
            );
        }
    }

    /// The select-all button in the corner of the two header bands: a triangle pointing into
    /// the corner, every spreadsheet's convention, drawn as rows of fills rather than `Polygon`,
    /// which would need a pen, a brush and a second path for the DIB target.
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

    /// The assist band's runs, left to right, stopping at the band's edge — measured and drawn
    /// with the same fonts (`gdi::text_width`, `gdi::text_at`), or a run lands elsewhere than it
    /// was placed.
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
            // Measured in the font about to draw it, through the rectangle-and-ellipsis path every
            // label takes, so a run reaching the edge ends in `…` and all are centred one way. Two
            // pixels of slack for `draw_text`'s one-pixel inset.
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

    /// A merge's whole area, from its top-left cell's corner to its bottom-right cell's.
    fn merged_rect(frame: &Frame, m: &grind_sheet::Merged) -> crate::sheet::geom::Rect {
        let first = frame.geom.cell_rect(m.anchor.row, m.anchor.col);
        let last = frame.geom.cell_rect(m.end.row, m.end.col);
        crate::sheet::geom::Rect {
            w: last.x + last.w - first.x,
            h: last.y + last.h - first.y,
            ..first
        }
    }

    /// The outline round the selected rectangle: four bars clipped to the body rather than a frame,
    /// since a whole-column selection's bottom edge is twenty million pixels down, and each edge
    /// is drawn only where the body reaches it.
    fn outline(dc: HDC, frame: &Frame) {
        let g = frame.geom;
        let (start, mut end) = frame.selection.rect();
        // A single cell that is a merge is outlined round the whole of it.
        if start == end
            && let Some(m) = frame.viewport.merge_at(start.row, start.col)
        {
            end = m.end;
        }
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

    /// The name overlay: an outline round every defined name's range, in the theme's ink moved
    /// towards the ground — muted, a label on the document's structure rather than something to
    /// act on — the same for one cell or many. `Viewport::names` is empty unless asked for, so
    /// with the mode off this returns at once.
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

    /// The format strip (`sheet/format.rs`): every control in [`format::CONTROLS`]' order at
    /// `GridGeom::format_controls`' rectangles, painted by the `crate::strip` functions the text
    /// pane's strip uses. Each toggle's label shows what it does — bold, italic, underlined,
    /// struck — and the three alignments are **drawn** as four ragged lines, since `L`, `C` and
    /// `R` name a direction without showing one.
    fn draw_format(dc: HDC, frame: &Frame) {
        let g = frame.geom;
        let theme = frame.theme;
        let band = g.format_rect();
        if band.h <= 0.0 {
            return;
        }
        {
            let (left, top, right, bottom) = band.edges();
            gdi::fill(dc, left, top, right, bottom, theme.backdrop);
        }
        let strip_view = &frame.format;
        let look = |control: Control| strip::Look {
            theme,
            dpi: g.dpi,
            state: match (
                strip_view.pressed == Some(control),
                strip_view.hover == Some(control),
            ) {
                (true, _) => Interaction::Pressed,
                (false, true) => Interaction::Hover,
                (false, false) => Interaction::Rest,
            },
        };
        let placed = g.format_controls();
        for (place, &(control, shape, _)) in placed.iter().zip(format::CONTROLS.iter()) {
            if let Some(rule) = place.separator {
                strip::separator(dc, rule, theme);
            }
            let rect = place.rect;
            match shape {
                Shape::Toggle => {
                    let on = format::pressed(strip_view.style, control);
                    strip::button_ground(dc, rect, look(control), on, false);
                    let ink = match on {
                        true => theme.accent,
                        false => theme.text,
                    };
                    match control {
                        Control::Bold | Control::Italic | Control::Underline | Control::Strike => {
                            let font = Font::styled(
                                frame.face,
                                frame.body_px,
                                control == Control::Bold,
                                control == Control::Italic,
                                control == Control::Underline,
                                control == Control::Strike,
                            );
                            let text = match control {
                                Control::Bold => "B",
                                Control::Italic => "I",
                                Control::Underline => "U",
                                _ => "S",
                            };
                            strip::label(dc, rect, text, &font, ink);
                        }
                        _ => align_icon(dc, rect, control, ink, g.dpi),
                    }
                }
                Shape::Swatch => {
                    strip::button_ground(dc, rect, look(control), false, false);
                    let background = control == Control::Background;
                    let value = match background {
                        true => strip_view.style.background.as_deref(),
                        false => strip_view.style.color.as_deref(),
                    };
                    strip::swatch(
                        dc,
                        rect,
                        theme,
                        value.filter(|v| *v != "transparent"),
                        background,
                        frame.body_px,
                        frame.face,
                    );
                }
                Shape::Picker => {
                    strip::picker(
                        dc,
                        rect,
                        look(control),
                        strip_view.number,
                        strip_view.number_set,
                    );
                }
                Shape::Icon => {
                    strip::button_ground(dc, rect, look(control), false, false);
                    indent_icon(dc, rect, control == Control::Indent, theme.text, g.dpi);
                }
                Shape::Button => {
                    strip::button_ground(dc, rect, look(control), false, true);
                    let font = Font::new(frame.face, frame.body_px, false);
                    strip::label(dc, rect, control.label(), &font, theme.text);
                }
            }
        }
    }

    /// The alignment icon: four lines, alternately long and short, flush to the side the text is
    /// aligned to — or centred.
    fn align_icon(dc: HDC, rect: crate::sheet::geom::Rect, control: Control, ink: Rgb, dpi: u32) {
        let unit = |v: f64| crate::sheet::geom::scale(v, dpi).round() as i32;
        let (long, short, thick, step) = (unit(14.0), unit(9.0), unit(1.5).max(1), unit(4.0));
        let height = step * 3 + thick;
        let (left, top, right, bottom) = rect.edges();
        let (cx, y0) = ((left + right) / 2, (top + bottom - height) / 2);
        for line in 0..4 {
            let w = match line % 2 {
                0 => long,
                _ => short,
            };
            let x = match control {
                Control::AlignLeft => cx - long / 2,
                Control::AlignRight => cx + long / 2 - w,
                _ => cx - w / 2,
            };
            let y = y0 + line * step;
            gdi::fill(dc, x, y, x + w, y + thick, ink);
        }
    }

    /// The indent icon: three short lines pushed right, and beside them a small triangle pointing
    /// the way the text will move — right for *Increase*, left for *Decrease*.
    fn indent_icon(dc: HDC, rect: crate::sheet::geom::Rect, more: bool, ink: Rgb, dpi: u32) {
        let unit = |v: f64| crate::sheet::geom::scale(v, dpi).round() as i32;
        let (long, short, thick, step) = (unit(14.0), unit(8.0), unit(1.5).max(1), unit(4.0));
        let height = step * 3 + thick;
        let (left, top, right, bottom) = rect.edges();
        let (cx, y0) = ((left + right) / 2, (top + bottom - height) / 2);
        let x0 = cx - long / 2;
        for line in 0..4 {
            let (x, w) = match line {
                0 | 3 => (x0, long),
                _ => (x0 + long - short, short),
            };
            let y = y0 + line * step;
            gdi::fill(dc, x, y, x + w, y + thick, ink);
        }
        // The arrow, one column of fills a pixel narrower each step from its base.
        let (mid, half) = (y0 + step + (step + thick) / 2, step);
        let base = x0;
        let depth = long - short - unit(2.0);
        for i in 0..depth.max(1) {
            let reach = half - half * i / depth.max(1);
            let x = match more {
                true => base + i,
                false => base + depth - 1 - i,
            };
            gdi::fill(dc, x, mid - reach, x + 1, mid + reach, ink);
        }
    }

    /// The squiggle under each misspelt word of a line `draw_text` has just drawn with the same
    /// rectangle, alignment and padding — placed by measuring the text before the word, and the
    /// word, in the DC's current font, so mark and ink agree. Text cut with an ellipsis cuts its
    /// marks too. `misspelt` counts characters of `text`.
    fn draw_squiggles(
        dc: HDC,
        text: &str,
        misspelt: &[std::ops::Range<usize>],
        (left, top, right, bottom): (i32, i32, i32, i32),
        align: Align,
        pad: f64,
        frame: &Frame,
    ) {
        if misspelt.is_empty() {
            return;
        }
        let text = super::one_line(text);
        let pad = pad.round() as i32;
        let (from, to) = (left + pad, (right - pad - 1).max(left + pad));
        let room = to - from;
        let full = gdi::text_width(dc, &text);
        let (origin, limit) = match full > room {
            true => (from, to - gdi::text_width(dc, "\u{2026}")),
            false => (
                match align {
                    Align::Left => from,
                    Align::Right => to - full,
                    Align::Center => from + (room - full) / 2,
                },
                to,
            ),
        };
        let height = gdi::line_height(dc);
        let line_top = top + (bottom - top - height) / 2;
        let dpi = frame.geom.dpi;
        let width = |chars: usize| -> i32 {
            gdi::text_width(dc, &text.chars().take(chars).collect::<String>())
        };
        for range in misspelt {
            let start = f64::from(origin + width(range.start));
            let end = f64::from(origin + width(range.end)).min(f64::from(limit));
            if end <= start {
                continue;
            }
            gdi::squiggle(
                dc,
                start,
                end,
                f64::from(line_top + height) - crate::sheet::geom::scale(3.0, dpi),
                crate::sheet::geom::scale(2.0, dpi),
                frame.theme.misspelt,
            );
        }
    }

    /// One string, clipped to a rectangle, vertically centred and elided if it does not fit —
    /// `DT_END_ELLIPSIS` rather than running into the next cell, since overflow into an empty
    /// neighbour is a named gap. Public to the crate: the text pane's status and notice bars are
    /// the same one line in a rectangle.
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
        // **Not** `gdi::wide`: `DrawTextW` in the `windows` crate takes a *slice* and its length
        // as the count, so a NUL-terminated buffer draws the terminator as a box glyph. The `…W`
        // entry points taking a `PCWSTR` want the terminator; those taking a length do not.
        let mut wide: Vec<u16> = super::one_line(text).encode_utf16().collect();
        // Nothing to draw is a *return*, not a zero-length `DrawTextW`: an empty `Vec<u16>` has no
        // allocation, so `as_mut_ptr` is the dangling address `2`, and Wine's `DrawTextW`
        // dereferences the buffer before reading the count — an access violation inside a system
        // DLL.
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

/// The header band at 100%, in pixels — a design measurement, scaled by `geom::scale` where the
/// geometry is built. The chrome's sizes are Fluent's own (a 32-pixel control with a 6-pixel
/// surround is a 44-pixel strip), and every rectangle in `geom.rs` is expressed in these.
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

    /// Level text starts half its size up and left of the middle; turned up, it starts below the
    /// middle, reading upwards.
    #[test]
    fn turned_text_is_placed_about_its_middle() {
        let near = |(x, y): (f64, f64), (ex, ey): (f64, f64)| {
            assert!((x - ex).abs() < 1e-9 && (y - ey).abs() < 1e-9, "{x},{y}")
        };
        near(turned_origin(40.0, 10.0, (50.0, 50.0), 0.0), (30.0, 45.0));
        near(turned_origin(40.0, 10.0, (50.0, 50.0), 90.0), (45.0, 70.0));
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

    /// `grind_sheet::look`'s rule: a number right, a label left, a boolean centred.
    #[test]
    fn a_number_goes_right_a_label_left_and_a_boolean_in_the_middle() {
        assert_eq!(
            Appearance::of(&CellValue::Number(1.0), None).align,
            Align::Right
        );
        assert_eq!(
            Appearance::of(&CellValue::Bool(true), None).align,
            Align::Center
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

    /// The selection is a *wash*, not a fill: a selected cell the document coloured keeps its
    /// colour, and the active cell keeps its ground, so it reads as the cursor.
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
