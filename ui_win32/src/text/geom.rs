// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The text pane's bands — notice, format strip, body, status bar — and the strip's controls, in
//! window pixels: pure arithmetic, **no Windows types**. Where the pages sit in the body is
//! `text::pages`'.

use crate::sheet::geom::{Rect, scale};

/// The status bar, the same height the grid's is so that the two panes' windows agree.
pub const STATUS_H: f64 = crate::sheet::draw::STATUS_H;

/// The notice bar, when there is a notice, and zero when not (`win.rs`'s `banner_h`) — the grid's
/// height, an inset card.
pub const BANNER_H: f64 = crate::sheet::draw::BANNER_H;

/// The format strip — decision 4's test applied to this pane: it reads and writes `CharStyle`, so
/// it goes under the menu rather than in it. Never zero: the toggles always mean something. The
/// grid's strip height, so both panes' chrome measures the same.
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

/// The window's furniture round the page: what is left for the document, and where. The banner's
/// height is the window's to set and zero with no notice, so every rectangle below is one
/// expression either way.
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

    /// The status bar's far end, where the spelling language is drawn in letters (`ALL`, `DE`,
    /// `Off`) and a click chooses another — wide enough for three capitals at any DPI.
    pub fn status_picker(&self) -> Rect {
        let status = self.status();
        let w = scale(56.0, self.dpi).min(status.w);
        Rect {
            x: status.x + status.w - w,
            w,
            ..status
        }
    }

    /// One control on the strip: `w` wide, Fluent's control height, centred in the band — a
    /// control standing on a band, so adding one never turns the strip back into full-height
    /// slabs.
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

    /// The gap between two controls of a group, and between groups. Three groups: the five
    /// emphasis toggles as one segmented control, the two pickers (the text's shape), the two
    /// swatches (its colour), and Clear alone, undoing all three.
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

    /// *Font* — opens `dialog::choose` over the families known here.
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for [prev, next] in buttons.array_windows() {
            assert!(prev.x + prev.w < next.x, "they overlap");
            assert!(
                next.x - (prev.x + prev.w) <= crate::theme::space::GAP,
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
        for [prev, next] in rects.array_windows() {
            assert!(prev.x + prev.w < next.x, "no overlap");
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
}
