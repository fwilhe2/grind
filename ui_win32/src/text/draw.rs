// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The text pane painted onto a device context: the pages in view, blitted from
//! `grind_print::raster`, the marks over them that are not on paper (selection, underlines,
//! bookmarks, the caret), and the window's bands. Where any of it goes is `text::pages`'; `paint`
//! takes an `HDC` and a `Frame`, so `--render-to` is a second caller.

/// The caret's width in pixels at 100%, and how far a selection's wash moves the ground towards
/// the theme's selection colour. The wash matches the grid's, so the two panes look like one
/// application.
pub const CARET_W: f64 = 2.0;
pub const WASH: f64 = 0.30;

#[cfg(windows)]
pub use windows_impl::{Frame, Sheet, paint};

#[cfg(windows)]
mod windows_impl {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{HDC, SetBkMode, TRANSPARENT};

    use crate::gdi::{self, Font, Selected};
    use crate::sheet::draw::{Align, draw_text};
    use crate::sheet::geom::{Rect, scale};
    use crate::theme::Theme;

    use super::super::geom::{Page, StripHit};

    use super::WASH;

    /// One page in view: where it is drawn, in client pixels, and its raster — premultiplied
    /// BGRA, `size` pixels, the same scale as `rect`.
    pub struct Sheet<'a> {
        pub rect: Rect,
        pub size: (u32, u32),
        pub pixels: &'a [u8],
    }

    /// Everything one frame of the text pane needs.
    pub struct Frame<'a> {
        pub page: &'a Page,
        pub theme: Theme,
        /// The pages in view, in order.
        pub sheets: &'a [Sheet<'a>],
        /// What is selected, one rectangle per line, in client pixels.
        pub selected: &'a [Rect],
        /// The caret's stroke, when it is drawn by the paint — only `render`'s windowless frame,
        /// which has no system caret.
        pub caret: Option<Rect>,
        pub status: &'a str,
        /// The spelling language in letters at the bar's end, in `Page::status_picker` — GDI has
        /// no colour glyphs and Windows no flags (`grind_spell::Setting::chip`).
        pub status_picker: &'a str,
        pub banner: Option<&'a str>,
        /// The chrome's two sizes — never the document's. `font_px` is `theme::text::CAPTION`,
        /// for the status bar and the name overlay's marks; `body_px` is `theme::text::BODY`, for
        /// the strip's own labels and the notice bar's sentence.
        pub font_px: i32,
        pub body_px: i32,
        pub face: &'a str,
        /// Which strip control the pointer is over — this pane's hover feedback, presentation
        /// state. A control with no fill of its own is invisible until pointed at, which lets nine
        /// sit in a row without a wall of boxes; without hover the row is dead under the pointer.
        pub hover: Option<StripHit>,
        /// Which one is held down, if any — drawn a step quieter than a hover, and the reason
        /// this strip acts on *release*: a press you can take back has to look pressed first.
        pub pressed: Option<StripHit>,
        /// Which of the strip's toggles apply to the selection, or to the style the next character
        /// typed would carry when there is none — Bold, Italic, Underline, Strike, Code,
        /// [`super::super::geom::Page::strip_buttons`]' order. Presentation state.
        pub format: [bool; 5],
        /// The family and size the strip's two pickers show, already resolved to what
        /// `text_style_here` says the selection or the next keystroke agrees on — `None` draws
        /// *Font*/*Size*, the same "nothing set" the two drop-downs mean in `grind-text-gtk`.
        pub family: Option<&'a str>,
        pub size: Option<&'a str>,
        /// The two swatches' own fill, `#rrggbb` verbatim — `None` draws the swatch hollow, which
        /// is what *Automatic* looks like.
        pub color: Option<&'a str>,
        pub highlight: Option<&'a str>,
        /// Each bookmark in view, with `View ▸ Names` on: a tick where it anchors, and its
        /// name drawn from `x` past the end of its line (`doc/view-modes.md` §3.6).
        pub names: &'a [(Rect, f64, String)],
        /// Each misspelt word's lines, less the word still being typed — a squiggle under each.
        pub misspelt: &'a [Rect],
        /// Each grammar finding's lines, and whether it is a quiet one (typography and style),
        /// dotted rather than wavy.
        pub flagged: &'a [(Rect, bool)],
    }

    /// Draw one frame of the document onto `dc`, writing every pixel of the client area, so
    /// `WM_ERASEBKGND` can answer "already done" (`gdi::BackBuffer`).
    pub fn paint(dc: HDC, frame: &Frame) {
        let page = frame.page;
        let theme = frame.theme;
        gdi::fill(
            dc,
            0,
            0,
            page.width.round() as i32,
            page.height.round() as i32,
            theme.backdrop,
        );
        // SAFETY: the DC is the caller's and live for this function. Every run is drawn with
        // `ExtTextOutW`, which paints its own ground only when the run asks for a highlight.
        unsafe {
            SetBkMode(dc, TRANSPARENT);
        }

        let chrome = Font::new(frame.face, frame.font_px, false);
        // White paper in either theme (`doc/page-model.md` decision 5), on the window's backdrop
        // with a hairline round it: the raster is the page, and the page is what prints.
        for sheet in frame.sheets {
            let (left, top, right, bottom) = sheet.rect.edges();
            gdi::fill(dc, left - 1, top - 1, right + 1, bottom + 1, theme.stroke);
            gdi::fill(
                dc,
                left,
                top,
                right,
                bottom,
                crate::theme::Rgb(255, 255, 255),
            );
            gdi::blit_image(dc, sheet.rect, sheet.size, sheet.pixels);
        }
        for band in frame.selected {
            gdi::wash(dc, *band, theme.selection, WASH);
        }
        let below = |band: &Rect| band.y + band.h - scale(3.0, page.dpi);
        for band in frame.misspelt {
            gdi::squiggle(
                dc,
                band.x,
                band.x + band.w,
                below(band),
                scale(2.0, page.dpi),
                theme.misspelt,
            );
        }
        for (band, quiet) in frame.flagged {
            let (left, right) = (band.x, band.x + band.w);
            if *quiet {
                gdi::dotted(
                    dc,
                    left,
                    right,
                    below(band),
                    scale(1.5, page.dpi),
                    theme.accent,
                );
            } else {
                gdi::squiggle(
                    dc,
                    left,
                    right,
                    below(band),
                    scale(2.0, page.dpi),
                    theme.accent,
                );
            }
        }
        if !frame.names.is_empty() {
            let ink = crate::theme::Rgb(0x60, 0x60, 0x60);
            let _font = Selected::font(dc, &chrome);
            for (tick, end, name) in frame.names {
                let (left, top, _, bottom) = tick.edges();
                gdi::fill(dc, left - 1, top, left + 1, bottom, ink);
                draw_text(
                    dc,
                    &format!("  \u{2039}{name}\u{203a}"),
                    end.round() as i32,
                    top,
                    (end + scale(160.0, page.dpi)).round() as i32,
                    bottom,
                    Align::Left,
                    ink,
                    0.0,
                );
            }
        }
        if let Some(caret) = frame.caret {
            let (left, top, right, bottom) = caret.edges();
            gdi::fill(
                dc,
                left,
                top,
                right.max(left + 1),
                bottom,
                crate::theme::Rgb(0, 0, 0),
            );
        }

        // The bands, over the pages: a page scrolled under the status bar must not show through
        // it, and drawing them second is cheaper than clipping. The chrome is set
        // in the shell font at the shell's size — it is the *window* talking, not the document.
        if let Some(notice) = frame.banner.filter(|_| page.banner_h > 0.0) {
            // An inset card with a stripe down its leading edge, as the grid's notice bar is: two
            // panes saying things in different-looking bands would be two programs in one binary.
            let body_font = Font::new(frame.face, frame.body_px, false);
            let _font = Selected::font(dc, &body_font);
            let card = crate::sheet::geom::card_in(page.banner(), page.dpi);
            let (left, top, right, bottom) = card.edges();
            let stripe = scale(3.0, page.dpi).round().max(1.0) as i32;
            let radius = scale(crate::theme::space::RADIUS, page.dpi).round() as i32;
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
                scale(crate::theme::space::GROUP, page.dpi),
            );
        }
        let _chrome = Selected::font(dc, &chrome);
        let rect = page.status();
        let (left, top, right, bottom) = rect.edges();
        // No hairline over it: the page's own surface already ends above, and the change from
        // one ground to the other is the edge. See `sheet::draw::paint`'s status bar.
        gdi::fill(dc, left, top, right, bottom, theme.backdrop);
        let (picker, _, _, _) = page.status_picker().edges();
        draw_text(
            dc,
            frame.status,
            left,
            top,
            picker,
            bottom,
            Align::Left,
            theme.text_secondary,
            scale(crate::theme::space::GROUP, page.dpi),
        );
        // In a control's ink rather than the read-out's: it is the one thing on the bar to press.
        draw_text(
            dc,
            frame.status_picker,
            picker,
            top,
            right,
            bottom,
            Align::Right,
            theme.text,
            scale(crate::theme::space::GROUP, page.dpi),
        );

        draw_strip(dc, page, theme, frame);
    }

    /// The format strip: five toggles, pressed when [`Frame::format`] says so, the Family and Size
    /// pickers, the two colour swatches, and *Clear*. Its admission test — a property of the
    /// selection — is exactly `App::char_style`, so this is the drawn half of what
    /// `text_emphasise` and `text_format` read and write.
    fn draw_strip(dc: HDC, page: &Page, theme: Theme, frame: &Frame) {
        use crate::strip;
        use crate::theme::Interaction;

        const LABELS: [&str; 5] = ["B", "I", "U", "S", "M"];
        let strip = page.strip();
        let (left, top, right, bottom) = strip.edges();
        gdi::fill(dc, left, top, right, bottom, theme.backdrop);

        let state = |hit: StripHit| match (frame.pressed == Some(hit), frame.hover == Some(hit)) {
            (true, _) => Interaction::Pressed,
            (false, true) => Interaction::Hover,
            (false, false) => Interaction::Rest,
        };
        let look = |hit: StripHit| strip::Look {
            theme,
            dpi: page.dpi,
            state: state(hit),
        };
        // One control's ground — `strip::button_ground`, shared with the grid's strip so the two
        // panes' controls are one kind of thing.
        let button = |rect: Rect, hit: StripHit, checked: bool, filled: bool| {
            strip::button_ground(dc, rect, look(hit), checked, filled);
        };

        // The five emphasis toggles. A pressed-in toggle is drawn in the accent's own ink as well
        // as on its own ground — Fluent's `ToggleButton` does both, and one without the other is
        // either a button that looks selected and says nothing, or ink with nothing under it.
        let toggles = page.strip_buttons();
        for (index, rect) in toggles.iter().enumerate() {
            let hit = StripHit::Toggle(index);
            let on = frame.format[index];
            button(*rect, hit, on, false);
            let (l, t, r, b) = rect.edges();
            let ink = if on { theme.accent } else { theme.text };
            // Bold's own label is bold, Italic's italic, and so on: the control shows what it
            // does rather than naming it, which is the one place a letter can carry its own icon.
            let font = Font::styled(
                frame.face,
                frame.body_px,
                index == 0,
                index == 1,
                index == 2,
                index == 3,
            );
            let _font = Selected::font(dc, &font);
            draw_text(dc, LABELS[index], l, t, r, b, Align::Center, ink, 0.0);
        }

        let picker = |rect: Rect, hit: StripHit, label: &str, set: bool| {
            strip::picker(dc, rect, look(hit), label, set);
        };
        picker(
            page.strip_family(),
            StripHit::Family,
            frame.family.unwrap_or("Font"),
            frame.family.is_some(),
        );
        picker(
            page.strip_size(),
            StripHit::Size,
            frame.size.unwrap_or("Size"),
            frame.size.is_some(),
        );

        let color = page.strip_color();
        button(color, StripHit::Color, false, false);
        strip::swatch(
            dc,
            color,
            theme,
            frame.color,
            false,
            frame.body_px,
            frame.face,
        );
        let highlight = page.strip_highlight();
        button(highlight, StripHit::Highlight, false, false);
        strip::swatch(
            dc,
            highlight,
            theme,
            frame.highlight,
            true,
            frame.body_px,
            frame.face,
        );

        let clear = page.strip_clear();
        button(clear, StripHit::Clear, false, true);
        let (cl, ct, cr, cb) = clear.edges();
        draw_text(dc, "Clear", cl, ct, cr, cb, Align::Center, theme.text, 0.0);

        // The separators between the three groups. Fluent's `AppBarSeparator`: half a control
        // tall, one pixel wide, in the middle of the gap — which says "a different kind of thing
        // follows" without putting a border round anything.
        for next in [page.strip_family(), color, clear] {
            strip::separator(dc, page.strip_separator(next), theme);
        }
    }
}
