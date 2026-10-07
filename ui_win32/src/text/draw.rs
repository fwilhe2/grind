// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A laid-out document, painted onto a device context.
//!
//! Two halves, the same split `sheet/draw.rs` makes: **what a line is made of is decided in
//! portable code** — `grind_text::paint`'s, since the macOS page would have copied it: `pieces`
//! cuts a line at every run boundary, `covered` and `band` say which part of it is selected,
//! `bullet` says what marks a list item — and only putting pixels down needs Windows.
//!
//! The Windows half draws **run by run through `crate::metrics::Face`**, never with `DrawTextW`.
//! That is decision 3 rather than a preference: the core placed every caret with GDI's own
//! advance array, so the ink has to be placed with the same one or the caret and the glyph
//! disagree. `Face::draw_run` is `ExtTextOutW` with exactly those advances.
//!
//! `paint` takes an `HDC` and a `Frame` and nothing about the window — no `HWND` — which is
//! what makes `--render-to` a second caller rather than a second drawing path.

/// The caret's width in pixels at 100%, and how far a selection's wash moves the ground towards
/// the theme's selection colour. The wash matches the grid's, so the two panes look like one
/// application.
pub const CARET_W: f64 = 2.0;
pub const WASH: f64 = 0.30;

/// How far a table's rule moves the page towards the text's own ink — `ui_text_gtk`'s
/// `Palette::rule`, the foreground at 28%, so a table is one weight in both desktop windows.
///
/// Deliberately not the grid pane's `grid_line`: that is the hairline of an *infinite* sheet and
/// is meant to recede, where a table in a document is structure a reader follows across a row.
/// Measured in the first frames that drew one, `grid_line` came out 228 on 255 — an empty table
/// all but vanished, which was the bug this replaced in a quieter form. This one is quieter than
/// the text and stronger than the furniture, in either palette.
pub const RULE_INK: f64 = 0.28;

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
    use crate::theme::Theme;

    use super::super::geom::{CellBox, Page, RULE, Slot, StripHit};
    use grind_text::paint::{band, covered, drawable, pieces};

    use super::{CARET_W, WASH};

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
        /// Every table cell's box (`Flow::cells`) — the grid's rules, drawn under the text. All of
        /// them rather than the ones on screen, because a box is four numbers and the painter
        /// skips the ones outside the body by itself.
        pub cells: &'a [CellBox],
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
        /// The misspelt words in the blocks on screen (`App::misspellings`), less the one still
        /// being typed — drawn as a squiggle under each. Empty with spelling off.
        pub misspelt: &'a [grind_text::Misspelling],
    }

    /// A wavy line from `left` to `right` with its crests at `top`: steps of `step` pixels,
    /// alternately up and down, each a filled square — `FillRect` is the one primitive this
    /// painter trusts to land on the same pixels at every DPI, and a pen's diagonal does not.
    fn squiggle(dc: HDC, left: f64, right: f64, top: f64, step: f64, ink: crate::theme::Rgb) {
        let step = step.max(1.0).round() as i32;
        let (left, right, top) = (
            left.round() as i32,
            right.round() as i32,
            top.round() as i32,
        );
        let mut x = left;
        let mut down = false;
        while x < right {
            let y = top + if down { step } else { 0 };
            gdi::fill(dc, x, y, (x + step).min(right), y + step, ink);
            x += step;
            down = !down;
        }
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
        let (column_x, _) = page.text_column();

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
        // **A table's grid**, under its text — `ui_text_gtk`'s rule, at `RULE_INK`. Each edge is
        // a one-pixel band *on* the box's outer line (right and bottom at `left + width` and
        // `top + height`), which is where the neighbour's left and top edges are too: a shared
        // edge is drawn twice onto the same pixels, and so stays one pixel.
        let rule = scale(RULE, page.dpi).round().max(1.0) as i32;
        let rule_ink = theme.background.blend(theme.text, super::RULE_INK);
        for cell in frame.cells {
            let shift = body.y - page.scroll;
            if cell.bottom() + shift < body.y || cell.top + shift > body.y + body.h {
                continue;
            }
            let (l, t) = (
                (column_x + cell.left).round() as i32,
                (cell.top + shift).round() as i32,
            );
            let (r, b) = (
                (column_x + cell.right_edge()).round() as i32,
                (cell.bottom() + shift).round() as i32,
            );
            for (x0, y0, x1, y1) in [
                (l, t, r + rule, t + rule),
                (l, b, r + rule, b + rule),
                (l, t, l + rule, b + rule),
                (r, t, r + rule, b + rule),
            ] {
                gdi::fill(dc, x0, y0, x1, y1, rule_ink);
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
                let width = painted.slot.width;
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

            // The list's mark — its number or its style's bullet (`BlockView::mark`) — drawn
            // outside the text column and outside the model.
            if let Some(mark) = painted.view.mark() {
                face.draw_run(
                    dc,
                    x - scale(super::super::geom::INDENT, page.dpi) * 0.7,
                    top,
                    mark,
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
                if let Some((left, right)) = covered(painted.slot.index, from, to)
                    .and_then(|(start, end)| band(&painted.layout, line, start, end))
                {
                    let left = x + f64::from(left);
                    let right = x + f64::from(right);
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
                    //
                    // And a run the document *did* colour is lifted until it reads on the dark
                    // page (`theme::document_ink`) — the navy word that vanished there.
                    let fill = piece
                        .props
                        .background
                        .as_deref()
                        .filter(|value| *value != "transparent")
                        .and_then(crate::theme::Rgb::parse);
                    let ink = crate::theme::document_ink(
                        piece
                            .props
                            .color
                            .as_deref()
                            .and_then(crate::theme::Rgb::parse),
                        fill.unwrap_or(theme.background),
                        fill.is_some(),
                        theme,
                    );
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

                // A misspelt word's squiggle, under each line it crosses — `band` is the
                // selection's own arithmetic, so a word that wraps is marked on both lines
                // exactly where its characters are.
                for wrong in frame
                    .misspelt
                    .iter()
                    .filter(|m| m.block == painted.slot.index)
                {
                    if let Some((left, right)) = band(
                        &painted.layout,
                        line,
                        wrong.offset,
                        wrong.offset + wrong.len,
                    ) {
                        squiggle(
                            dc,
                            x + f64::from(left),
                            x + f64::from(right),
                            line_top + f64::from(line.height) - scale(3.0, page.dpi),
                            scale(2.0, page.dpi),
                            theme.misspelt,
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
