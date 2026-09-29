// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where each block sits down the page, and where the page sits in the window — pure arithmetic.
//!
//! `sheet/geom.rs`'s counterpart for the text pane, and it exists for the same reason: the layout
//! decisions are the part most worth testing and the part hardest to test through a window, so
//! they live in a module that has never heard of one. **No Windows types at all.**
//!
//! **This is neither line layout nor the stacking above it.** Breaking a paragraph into lines is
//! `grind_core::layout`'s job and reaches this shell through [`grind_text::App`]
//! (`doc/text-layout.md`, Path C); how tall each block's box is, where it starts, where a table's
//! blocks go, which ones are on screen and how far to scroll to keep the caret visible is
//! [`grind_text::flow`]'s, shared with every shell that draws a page. This module was one of its
//! two copies and `ui_text_gtk` the other, until the macOS shell would have made a third
//! (`doc/macos-shell.md`, M1). What is left here is this pane's own: its numbers ([`spacing`]),
//! where the page sits in the window ([`Page`]), and the strip.

use std::collections::HashMap;

pub use grind_text::flow::{Across, CellBox, Flow, Slot, Spacing};

use crate::sheet::geom::{Rect, scale};

/// Space either side of the text column, in pixels at 100%.
///
/// Wide enough since W10 to hold the **page** the column is set on: the card is [`PAGE_PAD`]
/// outside the text on each side, and this has to leave a strip of the window's own backdrop
/// showing either side of that or the page is not a page, it is the window.
pub const MARGIN: f64 = 48.0;

/// The widest the text column is allowed to get.
///
/// A maximised window is far wider than a readable measure, and a word processor that sets prose
/// across 1800 pixels is unreadable in a way a spreadsheet never is. Roughly 80 characters at a
/// normal body size; the column is centred in whatever is left.
pub const MEASURE: f64 = 720.0;

/// How far one nesting level of a list indents its text.
pub const INDENT: f64 = 28.0;

/// The page's own top margin — space above the first block, which **scrolls with the text**
/// rather than framing it, so that a document scrolled to the bottom has no dead band at the top.
///
/// It is also the page card's top padding, which is why it grew in W10: a sheet of paper with
/// twenty-four pixels above the first line looks like a mistake, and forty like a margin.
pub const TOP: f64 = 40.0;

/// How far the page card extends past the text column on each side, and below the last block.
///
/// The card is the document's own surface — `Theme::background` — standing on the window's
/// backdrop, which is the whole of W10's answer for this pane: a word processor whose text sits
/// directly on the window chrome has no document in it, only some text.
pub const PAGE_PAD: f64 = 40.0;

/// Space under a block, and the extra a heading gets above it — the whole of this pane's
/// typography beyond the font itself.
pub const GAP: f64 = 10.0;
pub const HEADING_GAP: f64 = 18.0;

/// The gap between a picture and its caption — small, since the two read as one figure.
/// `ui_text_gtk`'s own `CAPTION_GAP`, at the same value.
pub const CAPTION_GAP: f64 = 4.0;

/// The status bar, the same height the grid's is so that the two panes' windows agree.
pub const STATUS_H: f64 = crate::sheet::draw::STATUS_H;

/// The notice bar, when there is a notice. Zero when there is not — see `win.rs`'s `banner_h`.
/// The grid's own height, and drawn the same way since W10: an inset card, not a stripe.
pub const BANNER_H: f64 = crate::sheet::draw::BANNER_H;

/// The format strip — decision 4's admission test applied to this pane: it reads and writes
/// `CharStyle`, so it goes under the menu rather than in it. Never zero, unlike the banner: this
/// bar has no "nothing to show" state, since the five toggles always mean something.
///
/// One Fluent control tall plus its surround, which is the grid's own strip height, so that the
/// two panes' chrome measures the same in the same window.
pub const STRIP_H: f64 = crate::sheet::draw::STRIP_H;

/// One toggle button's width on the strip — square, at Fluent's control height, which is what a
/// one-letter toggle wants and what lets the five of them read as one segmented group.
pub const BUTTON_W: f64 = crate::theme::space::CONTROL_H;

/// The Family picker's width — wide enough for a short font name and the chevron that says it
/// opens something; [`crate::sheet::draw::Align::Left`] elides anything longer.
pub const PICKER_W: f64 = 108.0;

/// The Size picker's, which needs room for `999pt` and a chevron and nothing else.
pub const SIZE_W: f64 = 72.0;

/// One colour swatch's width — square, like a toggle: a swatch draws a fill rather than a label
/// and needs no room for one.
pub const SWATCH_W: f64 = crate::theme::space::CONTROL_H;

/// *Clear Formatting*'s width — wider than a toggle, since "Clear" does not fit [`BUTTON_W`].
pub const CLEAR_W: f64 = 64.0;

/// The text column inside a pane `width` pixels wide: where it starts and how wide it is.
///
/// Centred rather than left-aligned once the window is wider than [`MEASURE`], which keeps the
/// measure constant as a window grows instead of letting the lines stretch.
pub fn column(width: f64, dpi: u32) -> (f64, f64) {
    let margin = scale(MARGIN, dpi);
    let available = (width - 2.0 * margin).max(1.0);
    let text = available.min(scale(MEASURE, dpi));
    (margin + (available - text) / 2.0, text)
}

/// Which of the strip's controls a click landed on — [`Page::strip_hit`]'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripHit {
    /// One of the five toggles, by [`Page::strip_buttons`]'s own order.
    Toggle(usize),
    Family,
    Size,
    Color,
    Highlight,
    Clear,
}

/// The window's furniture around the page: what is left for the document, and where.
///
/// The banner's *height* is the window's to set, and it is zero when there is no notice — so
/// every rectangle below it is one arithmetic expression whether or not it is showing, which is
/// the arrangement the grid already has.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Page {
    pub width: f64,
    pub height: f64,
    pub banner_h: f64,
    pub strip_h: f64,
    pub status_h: f64,
    pub dpi: u32,
    /// How far down the document the top of the body is, in pixels.
    pub scroll: f64,
}

impl Page {
    pub fn banner(&self) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: self.width,
            h: self.banner_h,
        }
    }

    /// The format strip, under the banner and above the body — decision 4's growable menu bar
    /// holds a verb, this holds a property of the selection, and the two never trade places.
    pub fn strip(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.banner_h,
            w: self.width,
            h: self.strip_h,
        }
    }

    /// The part of the window the document is drawn in.
    pub fn body(&self) -> Rect {
        Rect {
            x: 0.0,
            y: self.banner_h + self.strip_h,
            w: self.width,
            h: (self.height - self.banner_h - self.strip_h - self.status_h).max(0.0),
        }
    }

    pub fn status(&self) -> Rect {
        Rect {
            x: 0.0,
            y: (self.height - self.status_h).max(0.0),
            w: self.width,
            h: self.status_h,
        }
    }

    /// One control on the strip: `w` wide, at Fluent's control height, centred in the band.
    ///
    /// Every rectangle below is one of these, which is what stops the strip drifting back into
    /// full-height slabs the moment a control is added: a button is a *control standing on a
    /// band*, and the band's own height is not its business.
    fn strip_control(&self, x: f64, w: f64) -> Rect {
        let strip = self.strip();
        let h = scale(crate::theme::space::CONTROL_H, self.dpi).min(strip.h);
        Rect {
            x,
            y: strip.y + ((strip.h - h) / 2.0).max(0.0),
            w,
            h,
        }
    }

    /// Where the strip's controls begin — the same margin the grid's name box keeps, so the two
    /// panes' chrome lines up down the left of the window.
    fn strip_start(&self) -> f64 {
        scale(crate::theme::space::GROUP, self.dpi)
    }

    /// The gap between two controls that belong together, and between two groups of them.
    ///
    /// Three groups, and the grouping is the point (W10): the five emphasis toggles are one
    /// segmented control, the two pickers are the *shape* of the text, the two swatches are its
    /// *colour*, and Clear stands alone because it undoes all three. A row of nine evenly spaced
    /// buttons says none of that.
    fn strip_gaps(&self) -> (f64, f64) {
        (
            scale(crate::theme::space::GAP / 2.0, self.dpi),
            scale(crate::theme::space::GROUP, self.dpi),
        )
    }

    /// The strip's five toggle buttons, left to right: Bold, Italic, Underline, Strike, Code.
    pub fn strip_buttons(&self) -> [Rect; 5] {
        let (tight, _) = self.strip_gaps();
        let w = scale(BUTTON_W, self.dpi);
        let start = self.strip_start();
        std::array::from_fn(|i| self.strip_control(start + (w + tight) * i as f64, w))
    }

    /// *Font* — opens `dialog::choose` over the families this build knows.
    pub fn strip_family(&self) -> Rect {
        let (_, group) = self.strip_gaps();
        let last = self.strip_buttons()[4];
        self.strip_control(last.x + last.w + group, scale(PICKER_W, self.dpi))
    }

    /// *Size* — the ladder `grind_text::format::sizes` offers.
    pub fn strip_size(&self) -> Rect {
        let (tight, _) = self.strip_gaps();
        let family = self.strip_family();
        self.strip_control(family.x + family.w + tight, scale(SIZE_W, self.dpi))
    }

    /// The text colour swatch.
    pub fn strip_color(&self) -> Rect {
        let (_, group) = self.strip_gaps();
        let size = self.strip_size();
        self.strip_control(size.x + size.w + group, scale(SWATCH_W, self.dpi))
    }

    /// The highlight swatch.
    pub fn strip_highlight(&self) -> Rect {
        let (tight, _) = self.strip_gaps();
        let color = self.strip_color();
        self.strip_control(color.x + color.w + tight, scale(SWATCH_W, self.dpi))
    }

    /// *Clear Formatting* — the one-shot the toggles can only approximate.
    pub fn strip_clear(&self) -> Rect {
        let (_, group) = self.strip_gaps();
        let highlight = self.strip_highlight();
        self.strip_control(highlight.x + highlight.w + group, scale(CLEAR_W, self.dpi))
    }

    /// Where a separator goes between two groups: the middle of the gap before `next`, drawn at
    /// half a control's height. Fluent's `AppBarSeparator`, which is what says "these two things
    /// are not the same kind of thing" without a border round either of them.
    pub fn strip_separator(&self, next: Rect) -> Rect {
        let (_, group) = self.strip_gaps();
        let control = self.strip_control(0.0, 0.0);
        let h = control.h / 2.0;
        Rect {
            x: (next.x - group / 2.0).round(),
            y: control.y + (control.h - h) / 2.0,
            w: 1.0,
            h,
        }
    }

    /// Which control, if any, a click at `x, y` on the strip landed on.
    pub fn strip_hit(&self, x: f64, y: f64) -> Option<StripHit> {
        if let Some(index) = self
            .strip_buttons()
            .iter()
            .position(|rect| rect.contains(x, y))
        {
            return Some(StripHit::Toggle(index));
        }
        for (rect, hit) in [
            (self.strip_family(), StripHit::Family),
            (self.strip_size(), StripHit::Size),
            (self.strip_color(), StripHit::Color),
            (self.strip_highlight(), StripHit::Highlight),
            (self.strip_clear(), StripHit::Clear),
        ] {
            if rect.contains(x, y) {
                return Some(hit);
            }
        }
        None
    }

    /// The text column, in window coordinates.
    pub fn text_column(&self) -> (f64, f64) {
        column(self.width, self.dpi)
    }

    /// **The page** — the document's own surface, `height` pixels of content tall.
    ///
    /// The card the text is set on, in window coordinates and already scrolled: its top is the
    /// top of the document, so it moves off the screen as the document does, and its bottom is
    /// the last block plus one [`PAGE_PAD`]. Nothing is clipped here — the painter clips to the
    /// body, and a card whose top is a thousand pixels above the window is exactly what a
    /// document scrolled a thousand pixels down should have.
    ///
    /// This is decorative and the caret knows nothing about it: [`Page::text_column`] is
    /// unchanged, so where a line breaks and where a click lands are the same answers they were
    /// before there was a page to draw them on.
    pub fn page_card(&self, height: f64) -> Rect {
        let body = self.body();
        let (x, w) = self.text_column();
        let pad = scale(PAGE_PAD, self.dpi);
        Rect {
            x: x - pad,
            y: body.y - self.scroll,
            w: w + pad * 2.0,
            h: height + pad,
        }
    }
}

/// The space between a cell's rule and its text, and the rule's own width, in pixels at 100% —
/// `ui_text_gtk::geom::CELL_PAD` and `RULE`, so a table is one shape in every window.
pub const CELL_PAD: f64 = 6.0;
pub const RULE: f64 = 1.0;

/// This pane's page at a monitor's scaling — the numbers [`grind_text::flow`] stacks with.
///
/// Rebuilt from the constants at each DPI rather than scaled from the last answer, the same rule
/// the grid follows.
pub fn spacing(dpi: u32) -> Spacing {
    Spacing {
        top: scale(TOP, dpi),
        gap: scale(GAP, dpi),
        heading: scale(HEADING_GAP, dpi),
        indent: scale(INDENT, dpi),
        cell_pad: scale(CELL_PAD, dpi),
    }
}

/// Every block that is in a table cell, with where it sits across a column `column` wide —
/// [`grind_text::flow::across`] at this monitor's scaling, built once per reflow and read by
/// `metrics::Faces`, so that a caret motion inside a cell measures where the ink is.
pub fn across(app: &grind_text::App, column: f64, dpi: u32) -> HashMap<usize, Across> {
    grind_text::flow::across(app, column, &spacing(dpi))
}

/// Measure every block of a document and stack them, laying a table out as a grid —
/// [`grind_text::flow::lay_out`] with this pane's numbers.
///
/// **Portable on purpose.** The only thing it asks of Windows is the [`grind_text::Faces`] it is
/// handed, so on a machine with no Windows at all it can be handed [`grind_text::Uniform`] over
/// [`grind_text::Fixed`] and checked against what `grind text view --width` prints — W5's exit
/// criterion, which is now `grind_text::flow`'s own test and holds for every shell.
///
/// `picture` is `lay_out`'s hook: `image.rs`'s WIC decoder sizing a picture block, which this
/// module must not depend on. A caller with no decoder passes `&|_, _| None`.
pub fn flow_of(
    app: &grind_text::App,
    faces: &dyn grind_text::Faces,
    column: f64,
    dpi: u32,
    picture: &dyn Fn(&grind_text::BlockView, f64) -> Option<f64>,
) -> Flow {
    grind_text::flow::lay_out(app, faces, column, &spacing(dpi), picture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::BlockKind;

    /// An ordinary window on an ordinary document: 800 by 600 at 100%, no notice up.
    fn page() -> Page {
        Page {
            width: 800.0,
            height: 600.0,
            banner_h: 0.0,
            strip_h: STRIP_H,
            status_h: STATUS_H,
            dpi: 96,
            scroll: 0.0,
        }
    }

    #[test]
    fn the_text_column_is_centred_once_the_window_is_wider_than_the_measure() {
        let (x, w) = column(400.0, 96);
        assert_eq!((x, w), (MARGIN, 400.0 - 2.0 * MARGIN), "narrow: all of it");
        let (x, w) = column(2.0 * MEASURE, 96);
        assert_eq!(w, MEASURE, "wide: the measure holds");
        assert!(x > MARGIN, "and what is left is split either side");
        assert_eq!(x + w + x, 2.0 * MEASURE, "symmetrically");
        assert!(
            column(1.0, 96).1 >= 1.0,
            "a window narrower than its margins"
        );
    }

    /// Everything measured is rebuilt from the constants at this monitor's scaling rather than
    /// scaled from the last answer, which is the same rule the grid follows.
    #[test]
    fn the_column_scales_with_the_monitor() {
        let (_, wide) = column(4000.0, 192);
        assert_eq!(wide, 2.0 * MEASURE, "the measure is a physical size");
    }

    /// The page is the document's own surface: wider than the text on it, starting where the
    /// document starts, and moving with the scroll rather than framing the window.
    #[test]
    fn the_page_is_a_surface_the_text_stands_on() {
        let page = page();
        let (x, w) = page.text_column();
        let card = page.page_card(1000.0);
        assert!(card.x < x, "the page is wider than the text on it");
        assert_eq!(card.x, x - PAGE_PAD);
        assert_eq!(card.x + card.w, x + w + PAGE_PAD);
        assert_eq!(card.y, page.body().y, "unscrolled, it starts at the body");
        assert!(card.h > 1000.0, "and there is room under the last line");
        // Scrolled, it moves with the document — a page is a thing in the document, not a frame
        // round the window.
        let scrolled = Page {
            scroll: 300.0,
            ..page
        };
        assert_eq!(scrolled.page_card(1000.0).y, page.body().y - 300.0);
        // And there is always window left either side of it, or it would not read as a page.
        for width in [400.0, 800.0, 2400.0] {
            let card = Page { width, ..page }.page_card(100.0);
            assert!(card.x > 0.0, "{width}: the page touches the window's edge");
            assert!(card.x + card.w < width, "{width}");
        }
    }

    /// The four bands are contiguous and add up to the window, banner or no banner.
    #[test]
    fn the_page_bands_tile_the_window() {
        let page = page();
        assert_eq!(page.body().y, STRIP_H);
        assert_eq!(page.strip().h + page.body().h + page.status().h, 600.0);
        let with_notice = Page {
            banner_h: BANNER_H,
            ..page
        };
        assert_eq!(with_notice.body().y, BANNER_H + STRIP_H);
        assert_eq!(
            with_notice.banner().h
                + with_notice.strip().h
                + with_notice.body().h
                + with_notice.status().h,
            600.0
        );
    }

    /// The five toggles sit side by side on the strip, in order, none overlapping — a segmented
    /// group with the tight gap between its members and the window's own margin before the first.
    #[test]
    fn the_strip_buttons_tile_left_to_right_and_dont_overlap() {
        let page = page();
        let buttons = page.strip_buttons();
        assert_eq!(buttons[0].x, crate::theme::space::GROUP);
        for pair in buttons.windows(2) {
            assert!(pair[0].x + pair[0].w < pair[1].x, "they overlap");
            assert!(
                pair[1].x - (pair[0].x + pair[0].w) <= crate::theme::space::GAP,
                "a segmented group, not a scattering"
            );
        }
        // A control standing on the band rather than filling it, which is what makes the strip
        // read as a surface with things on it.
        assert_eq!(buttons[0].h, crate::theme::space::CONTROL_H);
        assert!(buttons[0].y > page.strip().y);
        assert_eq!(
            page.strip_hit(buttons[0].x + 1.0, buttons[0].y + 1.0),
            Some(StripHit::Toggle(0))
        );
        assert_eq!(
            page.strip_hit(buttons[2].x + 1.0, buttons[2].y + 1.0),
            Some(StripHit::Toggle(2))
        );
        assert_eq!(
            page.strip_hit(0.0, page.body().y + 5.0),
            None,
            "below the strip"
        );
        // The window's own margin is not a button: a click there is a click on the band.
        assert_eq!(page.strip_hit(1.0, buttons[0].y + 1.0), None);
    }

    /// The buttons scale with the monitor, the same rule every other measurement here follows.
    #[test]
    fn the_strip_buttons_scale_with_the_monitor() {
        let page = Page { dpi: 192, ..page() };
        assert_eq!(page.strip_buttons()[0].w, 2.0 * BUTTON_W);
        assert_eq!(page.strip_buttons()[0].x, 2.0 * crate::theme::space::GROUP);
    }

    /// The four pickers follow the five toggles in order, none overlapping, each its own
    /// [`StripHit`] — and the gap between two *groups* is wider than the gap inside one, which is
    /// the whole of how the strip says which controls belong together.
    #[test]
    fn the_pickers_continue_where_the_toggles_end_and_dont_overlap() {
        let page = page();
        let toggles = page.strip_buttons();
        let rects = [
            page.strip_family(),
            page.strip_size(),
            page.strip_color(),
            page.strip_highlight(),
            page.strip_clear(),
        ];
        let after = |a: Rect, b: Rect| b.x - (a.x + a.w);
        let tight = after(toggles[0], toggles[1]);
        assert!(after(toggles[4], rects[0]) > tight, "toggles | pickers");
        assert_eq!(after(rects[0], rects[1]), tight, "family and size pair up");
        assert!(after(rects[1], rects[2]) > tight, "pickers | colour");
        assert_eq!(after(rects[2], rects[3]), tight, "the two swatches pair up");
        assert!(after(rects[3], rects[4]) > tight, "colour | clear");
        for pair in rects.windows(2) {
            assert!(pair[0].x + pair[0].w < pair[1].x, "no overlap");
        }
        // The separator sits in a group gap and touches neither side of it.
        let rule = page.strip_separator(rects[0]);
        assert!(rule.x > toggles[4].x + toggles[4].w && rule.x < rects[0].x);
        assert!(rule.h < toggles[0].h, "half a control tall");
        assert_eq!(
            page.strip_hit(rects[0].x + 1.0, rects[0].y + 1.0),
            Some(StripHit::Family)
        );
        assert_eq!(
            page.strip_hit(rects[1].x + 1.0, rects[1].y + 1.0),
            Some(StripHit::Size)
        );
        assert_eq!(
            page.strip_hit(rects[2].x + 1.0, rects[2].y + 1.0),
            Some(StripHit::Color)
        );
        assert_eq!(
            page.strip_hit(rects[3].x + 1.0, rects[3].y + 1.0),
            Some(StripHit::Highlight)
        );
        assert_eq!(
            page.strip_hit(rects[4].x + 1.0, rects[4].y + 1.0),
            Some(StripHit::Clear)
        );
    }

    /// `flow_of` is `grind_text::flow::lay_out` with this pane's numbers, and those numbers are
    /// rebuilt at each monitor's scaling: the page's top margin, a list's indent and a cell's
    /// padding all double at 200%. The stacking rules themselves are tested where they live.
    #[test]
    fn the_flow_is_set_in_this_panes_numbers_at_this_monitors_scale() {
        let app = grind_text::App::new();
        app.insert(0, BlockKind::ListItem { depth: 1 }, "One item")
            .unwrap();
        app.insert_table(1, 1, 2, Some("T".into())).unwrap();
        let column = 400.0;
        for dpi in [96, 192] {
            let factor = f64::from(dpi) / 96.0;
            let across = across(&app, column, dpi);
            let faces = grind_text::Uniform::new(column as f32, &grind_text::Fixed);
            let flow = flow_of(&app, &faces, column, dpi, &|_, _| None);
            let item = flow.slot(0).unwrap();
            assert_eq!(item.top, TOP * factor, "{dpi}: the page's margin");
            assert_eq!(item.indent, INDENT * factor, "{dpi}: one level");
            let cell = across[&1];
            assert_eq!(cell.left, CELL_PAD * factor, "{dpi}: inside the padding");
            assert_eq!(flow.slot(1).unwrap().indent, cell.left);
        }
    }
}
