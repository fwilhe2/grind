// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A laid-out document, painted onto a device context.
//!
//! Two halves, the same split `sheet/draw.rs` makes: **what a line is made of is decided in
//! portable code** — [`pieces`] cuts it at every run boundary, [`selected_range`] says which part
//! of it is selected, [`bullet`] says what marks a list item — and only putting pixels down needs
//! Windows.
//!
//! The Windows half draws **run by run through `crate::metrics::Face`**, never with `DrawTextW`.
//! That is decision 3 rather than a preference: the core placed every caret with GDI's own
//! advance array, so the ink has to be placed with the same one or the caret and the glyph
//! disagree. `Face::draw_run` is `ExtTextOutW` with exactly those advances.
//!
//! `paint` takes an `HDC` and a `Frame` and nothing about the window — no `HWND` — which is
//! what makes `--render-to` a second caller rather than a second drawing path.

use grind_text::RunView;
use grind_text::style::CharStyle;

/// The caret's width in pixels at 100%, and how far a selection's wash moves the ground towards
/// the theme's selection colour. The wash matches the grid's, so the two panes look like one
/// application.
pub const CARET_W: f64 = 2.0;
pub const WASH: f64 = 0.30;

/// One run of uniform formatting, clipped to a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece<'a> {
    /// Where it starts, in characters from the beginning of the **block** — the unit every
    /// `grind_core::layout` offset is in, so it can be handed straight to `Layout::x_at`.
    pub start: usize,
    pub text: &'a str,
    pub props: &'a CharStyle,
}

/// Cut `from..to` characters of a block into the runs it crosses.
///
/// The pieces are in order, none is empty, and together they are exactly the block's characters
/// in that range — which is what lets a painter walk a line once, placing each piece at the x the
/// core measured for its first character.
pub fn pieces<'a>(runs: &'a [RunView], from: usize, to: usize) -> Vec<Piece<'a>> {
    let mut out = Vec::new();
    for run in runs {
        let start = run.start.max(from);
        let end = run.end().min(to);
        if start >= end {
            continue;
        }
        // A `RunView`'s offsets are in characters and a `&str`'s are in bytes; this is the one
        // place in the pane those two meet.
        let mut indices = run.text.char_indices().map(|(byte, _)| byte);
        let head = indices
            .by_ref()
            .nth(start - run.start)
            .unwrap_or(run.text.len());
        let tail = match end - start {
            0 => head,
            n => indices.nth(n - 1).unwrap_or(run.text.len()),
        };
        out.push(Piece {
            start,
            text: &run.text[head..tail],
            props: &run.props,
        });
    }
    out
}

/// Which part of `line_start..line_end` a selection running from `from` to `to` covers, or `None`
/// when the line is outside it entirely.
///
/// The two carets are (block, offset) pairs already reduced to offsets by the caller, which is why
/// this takes plain numbers: whether a *block* is inside the selection is a question about the
/// document, and whether a *line* is is a question about one block.
pub fn selected_range(
    line_start: usize,
    line_end: usize,
    from: usize,
    to: usize,
) -> Option<(usize, usize)> {
    let start = line_start.max(from);
    let end = line_end.min(to);
    (start < end).then_some((start, end))
}

/// One piece's text split at the two characters that are **in the model and never drawn**, each
/// segment with the character offset it starts at.
///
/// A `text:tab` and a `text:line-break` are each one character with an advance of its own
/// (`metrics.rs`), and GDI would draw the font's glyph for U+0009 and U+000A — a box, in Segoe
/// UI. So the drawing is cut around them and each side is placed at the offset the **core**
/// measured rather than at wherever the pen ended up. Empty segments are dropped, which is what
/// makes two of them in a row cost nothing to draw.
pub fn drawable(start: usize, text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut at = start;
    for segment in text.split(['\t', '\n']) {
        if !segment.is_empty() {
            out.push((at, segment));
        }
        // The segment's own characters, plus the one that ended it.
        at += segment.chars().count() + 1;
    }
    out
}

/// Where a caret offset sits on **this** line, in pixels from its left edge.
///
/// Not [`grind_core::layout::Layout::x_at`], and the difference is a bug found by running it: at
/// a break the offset belongs to two lines — the end of one and the start of the next — and
/// `x_at` resolves it to the *later* one, because that is where a caret walking off the end of a
/// wrapped line should appear. Asking it for the far end of the **earlier** line therefore
/// answers a few pixels from the left margin of the next one, which drew a selection's wash as a
/// rectangle of negative width: the first line of a two-line selection was simply not washed.
///
/// The end of a line is [`grind_core::layout::Line::width`], which is exactly that question
/// asked of the line rather than of the layout.
pub fn line_x(
    layout: &grind_core::layout::Layout,
    line: &grind_core::layout::Line,
    offset: usize,
) -> f32 {
    match offset >= line.end {
        true => line.width,
        false => layout.x_at(offset),
    }
}

/// What marks a list item at this depth.
///
/// Three marks that cycle, which is what every word processor does and what a document with a
/// six-deep list needs. The mark is **drawn and never stored**: a list's numbering lives in a
/// list style this build does not read (`doc/text-core.md`), so this is the pane saying "there is
/// a list item here" rather than the document being given a bullet it does not have.
pub fn bullet(depth: u32) -> &'static str {
    const MARKS: [&str; 3] = ["\u{2022}", "\u{25e6}", "\u{25aa}"];
    MARKS[(depth.max(1) as usize - 1) % MARKS.len()]
}

#[cfg(windows)]
pub use windows_impl::{Frame, Painted, paint};

#[cfg(windows)]
mod windows_impl {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{HDC, SetBkMode, TRANSPARENT};

    use grind_core::layout::Layout;
    use grind_text::style::CharStyle;
    use grind_text::{BlockView, Caret};

    use crate::gdi::{self, Font, Selected};
    use crate::metrics::Faces;
    use crate::sheet::draw::{Align, draw_text};
    use crate::sheet::geom::{Rect, scale};
    use crate::theme::{Rgb, Theme};

    use super::super::geom::{Page, Slot, StripHit};
    use super::{CARET_W, WASH, bullet, drawable, line_x, pieces, selected_range};

    /// One block, ready to draw: where it goes, what is in it, and how its lines broke.
    ///
    /// The layout is a value the window computed and will throw away, which is the same contract
    /// `App::get_viewport` offers for content — a shell that kept one would have a second copy of
    /// the document's shape and no way to know when it went stale.
    pub struct Painted<'a> {
        pub slot: Slot,
        pub view: &'a BlockView,
        pub layout: Layout,
    }

    /// Everything one frame of the text pane needs.
    pub struct Frame<'a> {
        pub page: &'a Page,
        pub theme: Theme,
        pub faces: &'a Faces<'a>,
        /// The blocks on screen, in document order, and nothing else — architecture rule 1.
        pub blocks: &'a [Painted<'a>],
        /// How tall the whole document measured — `Flow::height`, which is what the page card is
        /// drawn to and what the scrollbar already reports. The blocks above are only the ones on
        /// screen, so this cannot be derived from them.
        pub height: f64,
        /// The selection's two ends, in document order. Presentation state: the core is never
        /// told about it, and it reaches `App` as two carets when something is done to it.
        pub selection: (Caret, Caret),
        pub caret: Caret,
        /// Whether the caret is in its *on* phase — the blink, which is the user's own
        /// `GetCaretBlinkTime` rather than a constant. Always on where there is no window at all,
        /// so that two `--render-to` frames of one document are identical.
        pub caret_on: bool,
        pub status: &'a str,
        pub banner: Option<&'a str>,
        /// The chrome's two sizes — never the document's. `font_px` is `theme::text::CAPTION`,
        /// for the status bar and the name overlay's marks; `body_px` is `theme::text::BODY`, for
        /// the strip's own labels and the notice bar's sentence.
        pub font_px: i32,
        pub body_px: i32,
        pub face: &'a str,
        /// Which strip control the pointer is over, if any — the whole of this pane's hover
        /// feedback, and presentation state like the selection: the core is never told.
        ///
        /// A control with no fill of its own is invisible until you point at it, which is what
        /// lets nine of them sit in a row without the strip looking like a wall of boxes. Without
        /// this the row is dead under the pointer, which is the single thing that most makes a
        /// custom-drawn window feel unlike the rest of the system.
        pub hover: Option<StripHit>,
        /// Which one is held down, if any — drawn a step quieter than a hover, and the reason
        /// this strip acts on *release*: a press you can take back has to look pressed first.
        pub pressed: Option<StripHit>,
        /// Which of the strip's three buttons — Bold, Italic, Underline — apply to the selection,
        /// or to the style the next character typed would carry when there is none. Presentation
        /// state, computed by the caller for the same reason the selection is: the core is never
        /// told which of its own answers a shell drew a button in response to. Bold, Italic,
        /// Underline, Strike, Code — [`super::super::geom::Page::strip_buttons`]'s own order.
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
        /// `doc/view-modes.md`'s name overlay, this pane's answer to it — `:names`' equivalent
        /// for a document with no cells. A bookmark contributes no characters of its own
        /// (`doc/text-core.md` §3.6), so nothing a reader sees says it is there; `BlockView::marks`
        /// is where the reader lives, the same list `grind text view --names` prints, and this is
        /// that list drawn beside the text it anchors rather than spliced into it.
        pub names: bool,
    }

    /// Draw one frame of the document onto `dc`.
    ///
    /// Every pixel of the client area is written, which is what lets `WM_ERASEBKGND` be answered
    /// with "already done" — see `gdi::BackBuffer`.
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

        let body = page.body();
        let (column_x, column_w) = page.text_column();

        // **The page** (W10): the document's own surface, standing on the window's backdrop.
        // Drawn before anything on it and clipped to the body by hand, since GDI's clipping
        // region is state on the DC and this file sets none — a card whose top is a thousand
        // pixels above the window is what a document scrolled that far should have, and a
        // `RoundRect` given those coordinates would put its corner off-screen, which is exactly
        // right, but its *other* corner has to stay on.
        {
            let card = page.page_card(frame.height);
            let radius = scale(crate::theme::space::RADIUS_SURFACE, page.dpi).round() as i32;
            let (left, mut top, right, mut bottom) = card.edges();
            let (body_top, body_bottom) = (body.y.round() as i32, (body.y + body.h).round() as i32);
            // Overshoot the band by a corner's worth rather than clamping to it: that way an edge
            // that is really off-screen is drawn off-screen, and only the visible corners round.
            top = top.max(body_top - radius * 2);
            bottom = bottom.min(body_bottom + radius * 2);
            if bottom > top {
                gdi::round_rect(
                    dc,
                    RECT {
                        left,
                        top,
                        right,
                        bottom,
                    },
                    radius,
                    theme.background,
                    theme.stroke,
                );
            }
        }
        let (from, to) = frame.selection;
        // The chrome's own font, small and never the document's — used here for the name
        // overlay's marks and again below for the banner and the status bar. One `Font`, so a
        // resize does not build it twice.
        let chrome = Font::new(frame.face, frame.font_px, false);
        let muted = theme.text.blend(theme.background, 0.45);
        for painted in frame.blocks {
            let x = column_x + painted.slot.indent;
            let top = body.y + painted.slot.top - page.scroll;
            let face = frame
                .faces
                .face(&painted.view.kind, painted.view.style.as_deref());

            // A block that is a picture — optionally with its caption's text — is drawn as one
            // rather than as the placeholder character `RunView::text` returns everywhere else
            // (`grind_text::picture_of`'s own doc comment). Decoding happens here rather than in
            // `text::geom::flow_of`'s picture hook a second time — `image.rs`'s own `ponytail`
            // names the cost and the trigger for a cache.
            if let Some((image, caption)) = grind_text::picture_of(painted.view) {
                let width = (column_w - painted.slot.indent).max(1.0);
                if let Some(decoded) = crate::image::decode(&image.data) {
                    let (w, h) = (f64::from(decoded.width), f64::from(decoded.height));
                    let draw_w = width.min(w);
                    let draw_h = h * (draw_w / w);
                    crate::image::draw(
                        dc,
                        Rect {
                            x,
                            y: top,
                            w: draw_w,
                            h: draw_h,
                        },
                        &decoded,
                    );
                    if let Some(caption) = caption.filter(|text| !text.is_empty()) {
                        let gap = scale(super::super::geom::CAPTION_GAP, page.dpi);
                        face.draw_wrapped(dc, caption, x, top + draw_h + gap, width, muted);
                    }
                }
                continue;
            }

            // The bullet, drawn outside the text column and outside the model — a list's
            // numbering lives in a list style this build does not read.
            if let grind_text::BlockKind::ListItem { depth } = painted.view.kind {
                face.draw_run(
                    dc,
                    x - scale(super::super::geom::INDENT, page.dpi) * 0.7,
                    top,
                    bullet(depth),
                    &CharStyle::default(),
                    theme.text,
                );
            }

            let caret_line = (painted.slot.index == frame.caret.block && frame.caret_on)
                .then(|| painted.layout.line_at(frame.caret.offset));
            for (number, line) in painted.layout.lines().iter().enumerate() {
                let line_top = top + f64::from(line.top);
                // Wholly above or below the body: nothing to draw, and no measuring either.
                if line_top + f64::from(line.height) < body.y || line_top > body.y + body.h {
                    continue;
                }
                // The selection's wash, under the text. A block's own offsets only mean
                // something once the block is known to be inside the selection at all, which is
                // what the two clamps below decide.
                let (sel_from, sel_to) = block_selection(painted.slot.index, from, to);
                if let Some((s, e)) =
                    selected_range(line.start, line.end, sel_from, sel_to).filter(|_| from != to)
                {
                    let left = x + f64::from(line_x(&painted.layout, line, s));
                    let right = x + f64::from(line_x(&painted.layout, line, e));
                    gdi::fill(
                        dc,
                        left.round() as i32,
                        line_top.round() as i32,
                        right.round() as i32,
                        (line_top + f64::from(line.height)).round() as i32,
                        theme.background.blend(theme.selection, WASH),
                    );
                }

                for piece in pieces(&painted.view.runs, line.start, line.end) {
                    // A run the document highlighted but gave no colour to is ODF's *automatic*
                    // over that highlight, not over the page — the grid's own rule
                    // (`theme::automatic_ink`), and the case that matters here is a yellow
                    // highlight in a dark palette, where the theme's near-white ink vanishes.
                    let ink = match piece
                        .props
                        .background
                        .as_deref()
                        .and_then(crate::theme::Rgb::parse)
                    {
                        Some(ground) => crate::theme::automatic_ink(ground, theme),
                        None => theme.text,
                    };
                    // Every segment is placed at the x the **core** measured for its first
                    // character, never at where the last one happened to end — which is what
                    // makes a tab and a line break work: both are measured and neither is drawn.
                    for (start, segment) in drawable(piece.start, piece.text) {
                        face.draw_run(
                            dc,
                            x + f64::from(painted.layout.x_at(start)),
                            line_top,
                            segment,
                            piece.props,
                            ink,
                        );
                    }
                }

                // The caret, over the text it sits in. Which line it is on is `Layout`'s answer
                // rather than a range test, because at a soft break the offset is on two lines
                // and only the core knows which one it resolved to.
                if caret_line == Some(number) {
                    let caret_x = x + f64::from(painted.layout.x_at(frame.caret.offset));
                    gdi::fill(
                        dc,
                        caret_x.round() as i32,
                        line_top.round() as i32,
                        (caret_x + scale(CARET_W, page.dpi)).round() as i32,
                        (line_top + f64::from(line.height)).round() as i32,
                        theme.text,
                    );
                }

                // `doc/view-modes.md`'s name overlay: every bookmark anchored to a character on
                // this line, drawn beside it rather than spliced into the text the way
                // `grind text view --names` prints it — splicing here would move every offset
                // after it, which is exactly the caret-and-ink disagreement decision 3 exists to
                // rule out. Muted, the same "quieter, not louder" rule the grid's own name
                // overlay follows.
                if frame.names {
                    for (offset, name) in &painted.view.marks {
                        if painted.layout.line_at(*offset) != number {
                            continue;
                        }
                        let mark_x = x + f64::from(painted.layout.x_at(*offset));
                        let label = format!("\u{2039}{name}\u{203a}");
                        let _font = Selected::font(dc, &chrome);
                        draw_text(
                            dc,
                            &label,
                            mark_x.round() as i32,
                            line_top.round() as i32,
                            (mark_x + scale(120.0, page.dpi)).round() as i32,
                            (line_top + f64::from(line.height)).round() as i32,
                            Align::Left,
                            muted,
                            scale(2.0, page.dpi),
                        );
                    }
                }
            }
        }

        // The bands, over the text: a line scrolled under the status bar must not show through
        // it, and drawing them second is cheaper than clipping the loop above. The chrome is set
        // in the shell font at the shell's size — it is the *window* talking, not the document.
        if let Some(notice) = frame.banner.filter(|_| page.banner_h > 0.0) {
            // An inset card with a stripe down its leading edge — the grid's own notice bar, to
            // the pixel (W9 gave them the stripe, W10 the card): two panes in one window that say
            // things in two different-looking bands are two programs in one binary, which is the
            // thing `Pane` exists not to be.
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
        draw_text(
            dc,
            frame.status,
            left,
            top,
            right,
            bottom,
            Align::Left,
            theme.text_secondary,
            scale(crate::theme::space::GROUP, page.dpi),
        );

        draw_strip(dc, page, theme, frame);
    }

    /// The format strip: five toggles, pressed in when [`Frame::format`] says the property
    /// applies, then the Family and Size pickers, the two colour swatches and *Clear*.
    /// `doc/windows-shell.md`'s admission test for this surface — "a property of the
    /// selection" — is exactly `App::char_style`, so this is the drawn half of what
    /// `super::super::super::text_emphasise` and `super::super::super::text_format` already read
    /// and write; a control only decides whether to wash its own ground or draw a swatch before
    /// drawing the same label every one of its callers agrees on.
    fn draw_strip(dc: HDC, page: &Page, theme: Theme, frame: &Frame) {
        use crate::theme::{Interaction, control_fill};

        const LABELS: [&str; 5] = ["B", "I", "U", "S", "M"];
        let strip = page.strip();
        let (left, top, right, bottom) = strip.edges();
        gdi::fill(dc, left, top, right, bottom, theme.backdrop);

        let radius = scale(crate::theme::space::RADIUS, page.dpi).round() as i32;
        let state = |hit: StripHit| match (frame.pressed == Some(hit), frame.hover == Some(hit)) {
            (true, _) => Interaction::Pressed,
            (false, true) => Interaction::Hover,
            (false, false) => Interaction::Rest,
        };
        // One control, drawn: its ground if it has one in this state, then its label. `filled`
        // is the difference between a *toggle* — transparent until you point at it, so five in a
        // row read as one group — and a *picker*, which is a field and looks like one.
        let button = |rect: Rect, hit: StripHit, checked: bool, filled: bool| {
            let (l, t, r, b) = rect.edges();
            if let Some(fill) = control_fill(theme, state(hit), checked, filled) {
                let border = match checked {
                    true => theme.accent.blend(fill, 0.55),
                    false => match filled {
                        true => theme.stroke,
                        false => fill,
                    },
                };
                gdi::round_rect(
                    dc,
                    RECT {
                        left: l,
                        top: t,
                        right: r,
                        bottom: b,
                    },
                    radius,
                    fill,
                    border,
                );
            }
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
            let ink = match on {
                true => theme.accent,
                false => theme.text,
            };
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

        let pad = scale(crate::theme::space::GAP + 2.0, page.dpi);
        let picker = |rect: Rect, hit: StripHit, label: &str, set: bool| {
            button(rect, hit, false, true);
            let (l, t, r, b) = rect.edges();
            let chevron = scale(14.0, page.dpi).round() as i32;
            draw_text(
                dc,
                label,
                l,
                t,
                r - chevron,
                b,
                Align::Left,
                match set {
                    true => theme.text,
                    // Nothing set is a *placeholder*, and Fluent's placeholders are the tertiary
                    // ink — the difference between "this text is set in Consolas" and "this
                    // picker sets the family" was invisible before W10, since both were drawn in
                    // the same grey.
                    false => theme.text_tertiary,
                },
                pad,
            );
            // Drawn, not typed: see `gdi::triangle_down`, which is here because a rendered
            // frame came back with a missing-glyph box where `▾` should have been.
            gdi::triangle_down(
                dc,
                r - chevron / 2,
                (t + b) / 2 - scale(1.0, page.dpi).round() as i32,
                scale(7.0, page.dpi).round() as i32,
                theme.text_tertiary,
            );
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
        draw_swatch(
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
        draw_swatch(
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
            let rule = page.strip_separator(next);
            let (l, t, r, b) = rule.edges();
            gdi::fill(dc, l, t, r.max(l + 1), b, theme.divider);
        }
    }

    /// One colour swatch: an **A** with the colour it carries shown the way that colour is used —
    /// as a bar under the letter for the text colour, and as the ground behind it for the
    /// highlight. So the two swatches are told apart by what they *do* rather than by a label,
    /// and neither needs a glyph beyond the one letter.
    ///
    /// Every ornament here is drawn or is an ASCII letter, which is the rule the chevron above
    /// arrived at the hard way: a `▓` for the highlight came back from a rendered frame as a
    /// missing-glyph box under the face Wine substitutes.
    ///
    /// *Automatic* — no colour set — draws the bar in the theme's own ink, since that is what
    /// automatic resolves to, with a hairline round it so an empty swatch is still a swatch; a
    /// highlight with nothing set draws no ground at all, which is what no highlight looks like.
    fn draw_swatch(
        dc: HDC,
        rect: Rect,
        theme: Theme,
        hex: Option<&str>,
        ground: bool,
        font_px: i32,
        face: &str,
    ) {
        let (left, top, right, bottom) = rect.edges();
        let inset = ((right - left) / 5).max(2);
        let bar = (rect.h / 6.0).round().max(3.0) as i32;
        let fill = hex.and_then(crate::theme::Rgb::parse);
        if ground {
            // Hollow when nothing is set, which is what no highlight looks like — and still a
            // control rather than a bare letter floating on the strip.
            gdi::round_rect(
                dc,
                RECT {
                    left: left + inset,
                    top: top + inset,
                    right: right - inset,
                    bottom: bottom - bar - inset / 2,
                },
                2,
                fill.unwrap_or(theme.backdrop),
                fill.unwrap_or(theme.stroke),
            );
        }
        {
            let font = Font::new(face, font_px, false);
            let _font = Selected::font(dc, &font);
            // On a highlight the letter has to read against the *document's* colour, which can be
            // any of `PALETTE`'s — so it is chosen for contrast rather than fixed, the same
            // question `Theme::on_accent` answers for the accent.
            let ink = match (ground, fill) {
                (true, Some(colour)) => match colour.contrast(theme.text) > 3.0 {
                    true => theme.text,
                    false => Rgb(0, 0, 0),
                },
                _ => theme.text,
            };
            draw_text(
                dc,
                "A",
                left,
                top,
                right,
                bottom - bar,
                Align::Center,
                ink,
                0.0,
            );
        }
        if !ground {
            gdi::round_rect(
                dc,
                RECT {
                    left: left + inset,
                    top: bottom - bar - inset,
                    right: right - inset,
                    bottom: bottom - inset,
                },
                1,
                fill.unwrap_or(theme.text),
                match fill {
                    Some(colour) => colour,
                    None => theme.stroke,
                },
            );
        }
    }

    /// The part of `block` a selection from `from` to `to` covers, as two offsets into that block.
    ///
    /// A block wholly inside the selection is covered from nothing to everything, which is
    /// spelled `0..usize::MAX` and then clipped against the line — cheaper and less error-prone
    /// than carrying "the whole block" as a third case.
    fn block_selection(block: usize, from: Caret, to: Caret) -> (usize, usize) {
        if block < from.block || block > to.block {
            return (0, 0);
        }
        let start = match block == from.block {
            true => from.offset,
            false => 0,
        };
        let end = match block == to.block {
            true => to.offset,
            false => usize::MAX,
        };
        (start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(start: usize, text: &str, bold: bool) -> RunView {
        RunView {
            start,
            text: text.to_owned(),
            props: CharStyle {
                font_weight: bold.then(|| "bold".to_owned()),
                ..CharStyle::default()
            },
            style: None,
            href: None,
            image: None,
        }
    }

    #[test]
    fn a_line_is_cut_at_every_run_boundary() {
        let runs = [run(0, "hello ", false), run(6, "world", true)];
        let cut = pieces(&runs, 0, 11);
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0].text, "hello ");
        assert_eq!((cut[1].start, cut[1].text), (6, "world"));
        assert!(cut[1].props.is_bold());
    }

    /// A line in the middle of a long run gets the slice of it that is on the line, with the
    /// offset it really has in the block — which is what `Layout::x_at` is indexed by.
    #[test]
    fn a_run_is_clipped_to_the_line_and_keeps_its_own_offsets() {
        let runs = [run(0, "abcdefghij", false)];
        let cut = pieces(&runs, 3, 7);
        assert_eq!(cut.len(), 1);
        assert_eq!((cut[0].start, cut[0].text), (3, "defg"));
    }

    #[test]
    fn a_run_outside_the_line_is_not_drawn_at_all() {
        let runs = [run(0, "abc", false), run(3, "def", false)];
        assert!(pieces(&runs, 0, 0).is_empty());
        assert_eq!(pieces(&runs, 3, 6).len(), 1);
    }

    /// The offsets are characters and the slicing is bytes, which is the one place in the pane
    /// those two meet — and the one place a document in any language but English would break.
    #[test]
    fn a_run_is_cut_by_characters_and_not_by_bytes() {
        let runs = [run(0, "héllo wörld", false)];
        let cut = pieces(&runs, 0, 5);
        assert_eq!(cut[0].text, "héllo");
        let tail = pieces(&runs, 6, 11);
        assert_eq!(tail[0].text, "wörld");
    }

    #[test]
    fn only_the_selected_part_of_a_line_is_washed() {
        assert_eq!(selected_range(0, 10, 3, 7), Some((3, 7)));
        assert_eq!(selected_range(0, 10, 0, 40), Some((0, 10)), "all of it");
        assert_eq!(selected_range(20, 30, 0, 10), None, "a line before it");
        assert_eq!(selected_range(0, 10, 5, 5), None, "an empty selection");
    }

    /// A tab and a line break are measured and never drawn, so the drawing is cut around them —
    /// and each side keeps the offset the core measured it at, which is what places it. Found by
    /// *running* it: both came out as the font's missing-glyph box.
    #[test]
    fn a_tab_and_a_break_cut_the_drawing_and_keep_the_offsets() {
        assert_eq!(
            drawable(0, "name\tvalue"),
            vec![(0, "name"), (5, "value")],
            "the tab itself is one character and is not drawn"
        );
        assert_eq!(
            drawable(0, "value\nsecond"),
            vec![(0, "value"), (6, "second")],
            "and so is a text:line-break"
        );
        assert_eq!(
            drawable(10, "a\t\tb"),
            vec![(10, "a"), (13, "b")],
            "two in a row"
        );
        assert_eq!(drawable(0, "\tx"), vec![(1, "x")], "a leading tab");
        assert!(
            drawable(0, "\t").is_empty(),
            "nothing but a tab draws nothing"
        );
        assert_eq!(
            drawable(3, "plain"),
            vec![(3, "plain")],
            "and neither at all"
        );
    }

    /// The wash's far end is the *line's* width, not the layout's x for an offset that belongs
    /// to the next line — which is what a two-line selection showed by leaving its first line
    /// unwashed. Found by running it under Wine.
    #[test]
    fn the_end_of_a_line_is_the_lines_own_width() {
        use grind_core::layout::{Fixed, Fragment, wrap};
        use grind_core::style::TextStyle;

        let style = TextStyle::default();
        let text = "aaa bbb ccc ddd";
        let layout = wrap(
            &[Fragment {
                text,
                style: &style,
            }],
            8.0,
            &Fixed,
        );
        assert!(layout.lines().len() > 1, "the fixture has to wrap");
        let first = layout.lines()[0];
        // `x_at` resolves the offset at the break to the *second* line and answers from its left
        // edge; the first line's own end is its width.
        assert!(layout.x_at(first.end) < first.width);
        assert_eq!(line_x(&layout, &first, first.end), first.width);
        // Past the end — a selection running through the whole block — stops there too.
        assert_eq!(line_x(&layout, &first, usize::MAX), first.width);
        // And inside the line it is `x_at` unchanged.
        assert_eq!(line_x(&layout, &first, 1), layout.x_at(1));
    }

    #[test]
    fn a_list_marks_each_depth_differently_and_cycles() {
        assert_ne!(bullet(1), bullet(2));
        assert_eq!(bullet(1), bullet(4), "three marks, then round again");
        // Depth is 1-based in the model, and a document claiming zero must not index past the
        // start of the table.
        assert_eq!(bullet(0), bullet(1));
    }
}
