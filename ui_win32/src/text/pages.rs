// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The document as its printed pages in this window (`doc/page-model.md` PM7): which pages are in
//! view and where, in client pixels, and every point and rectangle between those pixels and the
//! pages' points. **No Windows types**; `grind_print::place` answers where text is on a page.

use std::ops::Range;
use std::rc::Rc;

use grind_print::ops::Document;
use grind_print::place;
use grind_print::strip::Strip;
use grind_text::Caret;

use crate::sheet::geom::Rect;

/// Pixels per point at `zoom` on a monitor of `dpi`: 100 % is the page at its printed size.
pub fn pixels_per_point(zoom: f64, dpi: u32) -> f64 {
    zoom * f64::from(dpi.max(1)) / 72.0
}

/// The pages as they are now and where each sits, scrolled into a body of the window.
pub struct Shown {
    pub set: Rc<Document>,
    pub strip: Strip,
    /// The window's body, where the strip is seen through.
    pub body: Rect,
    /// How far the strip is scrolled across and down, in pixels.
    pub scroll: (f64, f64),
}

impl Shown {
    /// Pages of `set` at `scale` pixels per point, in `body` scrolled by `scroll`.
    pub fn new(set: Rc<Document>, scale: f64, body: Rect, scroll: (f64, f64)) -> Self {
        let sizes = set.pages.iter().map(|page| (page.width, page.height));
        let strip = Strip::new(sizes, scale, body.w);
        Shown {
            set,
            strip,
            body,
            scroll,
        }
    }

    /// A point of the strip in client pixels.
    fn client(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.body.x + x - self.scroll.0,
            self.body.y + y - self.scroll.1,
        )
    }

    /// The pages any part of which is in the body.
    pub fn visible(&self) -> Range<usize> {
        self.strip
            .visible(self.scroll.1, self.scroll.1 + self.body.h)
    }

    /// Where page `page` is drawn, in client pixels.
    pub fn sheet(&self, page: usize) -> Option<Rect> {
        let sheet = self.strip.sheet(page)?;
        let (x, y) = self.client(sheet.x, sheet.y);
        Some(Rect {
            x,
            y,
            w: sheet.width,
            h: sheet.height,
        })
    }

    /// The caret a point in the window means: on the page under it, or the nearer of two.
    pub fn caret_at(&self, x: f64, y: f64) -> Option<Caret> {
        let (x, y) = (
            x - self.body.x + self.scroll.0,
            y - self.body.y + self.scroll.1,
        );
        let page = self.strip.page_at(y)?;
        let (x, y) = self.strip.on_page(page, x, y)?;
        place::hit(&self.set, page, x, y)
    }

    /// The caret's stroke in the strip's pixels: x, top and height.
    pub fn spot_in_strip(&self, at: Caret) -> Option<(f64, f64, f64)> {
        let spot = place::locate(&self.set, at)?;
        let (x, y) = self.strip.in_strip(spot.page, spot.x, spot.top)?;
        Some((x, y, f64::from(spot.height) * self.strip.scale()))
    }

    /// The caret's stroke in client pixels, `width` wide.
    pub fn spot(&self, at: Caret, width: f64) -> Option<Rect> {
        let (x, y, h) = self.spot_in_strip(at)?;
        let (x, y) = self.client(x, y);
        Some(Rect { x, y, w: width, h })
    }

    /// The text between two carets as client rectangles, one per line, on the pages in view.
    pub fn bands(&self, from: Caret, to: Caret) -> Vec<Rect> {
        let scale = self.strip.scale();
        place::selection_on(&self.set, self.visible(), from, to)
            .into_iter()
            .filter_map(|(page, band)| {
                let (x, y) = self.strip.in_strip(page, band.x, band.y)?;
                let (x, y) = self.client(x, y);
                Some(Rect {
                    x,
                    y,
                    w: f64::from(band.width) * scale,
                    h: f64::from(band.height) * scale,
                })
            })
            .collect()
    }

    /// The blocks with a line on the pages in view.
    pub fn blocks(&self) -> Option<Range<usize>> {
        place::blocks_on(&self.set, self.visible())
    }

    /// How far down and across the strip can scroll in this body.
    pub fn limits(&self) -> (f64, f64) {
        let (width, height) = self.strip.size();
        (
            (width - self.body.w).max(0.0),
            (height - self.body.h).max(0.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::{App, BlockKind};

    fn shown(scroll: (f64, f64)) -> Shown {
        let app = App::new();
        let paragraph = "All work and no play makes a dull document. ".repeat(40);
        for at in 0..8 {
            app.insert(at, BlockKind::Paragraph, &paragraph).unwrap();
        }
        let session = grind_print::session::Session::new(true);
        let body = Rect {
            x: 0.0,
            y: 50.0,
            w: 900.0,
            h: 600.0,
        };
        Shown::new(session.document(&app), 96.0 / 72.0, body, scroll)
    }

    #[test]
    fn a_click_lands_on_the_caret_drawn_there() {
        let shown = shown((0.0, 0.0));
        assert!(shown.set.pages.len() >= 2);
        let at = Caret {
            block: 0,
            offset: 40,
        };
        let spot = shown.spot(at, 2.0).unwrap();
        assert!(spot.y >= shown.body.y, "below the bands: {spot:?}");
        assert_eq!(shown.caret_at(spot.x, spot.y + spot.h / 2.0), Some(at));
    }

    #[test]
    fn scrolling_moves_everything_by_as_much() {
        let still = shown((0.0, 0.0));
        let moved = shown((0.0, 300.0));
        let at = Caret {
            block: 0,
            offset: 10,
        };
        let (a, b) = (still.spot(at, 2.0).unwrap(), moved.spot(at, 2.0).unwrap());
        assert!((a.y - b.y - 300.0).abs() < 1e-9);
        assert_eq!(a.x, b.x);
        assert_eq!(moved.caret_at(b.x, b.y + b.h / 2.0), Some(at));
        assert_eq!(still.sheet(0).unwrap().y - moved.sheet(0).unwrap().y, 300.0);
    }

    #[test]
    fn the_pages_in_view_and_their_blocks_follow_the_scroll() {
        let top = shown((0.0, 0.0));
        assert_eq!(top.visible().start, 0);
        let (_, down) = top.limits();
        let bottom = shown((0.0, down));
        assert_eq!(bottom.visible().end, bottom.set.pages.len());
        assert!(bottom.visible().start > 0);
        assert!(bottom.blocks().unwrap().end >= 8);
    }

    #[test]
    fn a_selection_is_a_band_per_line_in_view() {
        let shown = shown((0.0, 0.0));
        let at = |offset| Caret { block: 0, offset };
        let bands = shown.bands(at(0), at(300));
        assert!(bands.len() >= 2, "{bands:?}");
        assert!(bands.iter().all(|band| band.w > 0.0 && band.h > 0.0));
        assert!(bands[1].y > bands[0].y);
    }
}
