// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The pages as a window shows them (`doc/page-model.md`): a typeset document placed in a
//! [`Strip`], and every question a shell asks about where its text is — a caret's stroke, the
//! caret a point means, a selection's rectangles — answered in the strip's pixels. A shell adds
//! only its own offset: a scroll, a band above the pages.

use std::ops::Range;
use std::rc::Rc;

use grind_text::Caret;

use crate::ops::Document;
use crate::place;
use crate::strip::Strip;

/// A rectangle in the strip: x, y, width, height.
pub type Rect = [f64; 4];

/// A caret's stroke in the strip, and where it is on its page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub page: usize,
    pub x: f64,
    pub y: f64,
    pub height: f64,
    /// Its x on the page, in points — what a run of Down presses aims for.
    pub page_x: f32,
}

/// A typeset document's pages, placed.
pub struct Shown {
    pub set: Rc<Document>,
    pub strip: Strip,
}

impl Shown {
    /// `set`'s pages at `scale` pixels per point in a view `width` pixels wide.
    #[must_use]
    pub fn new(set: Rc<Document>, scale: f64, width: f64) -> Self {
        let sizes = set.pages.iter().map(|page| (page.width, page.height));
        let strip = Strip::new(sizes, scale, width);
        Shown { set, strip }
    }

    /// Where `at` is drawn.
    #[must_use]
    pub fn stroke(&self, at: Caret) -> Option<Stroke> {
        let spot = place::locate(&self.set, at)?;
        let (x, y) = self.strip.in_strip(spot.page, spot.x, spot.top)?;
        Some(Stroke {
            page: spot.page,
            x,
            y,
            height: f64::from(spot.height) * self.strip.scale(),
            page_x: spot.x,
        })
    }

    /// The caret a point of the strip means: on the page under it, or the nearer of two, and on
    /// it the nearest line — a click in a margin is the text beside it.
    #[must_use]
    pub fn hit(&self, x: f64, y: f64) -> Option<Caret> {
        let page = self.strip.page_at(y)?;
        let (x, y) = self.strip.on_page(page, x, y)?;
        place::hit(&self.set, page, x, y)
    }

    /// The text between two carets on `pages`, one rectangle per line.
    #[must_use]
    pub fn bands(&self, from: Caret, to: Caret, pages: Range<usize>) -> Vec<Rect> {
        let scale = self.strip.scale();
        place::selection_on(&self.set, pages, from, to)
            .into_iter()
            .filter_map(|(page, band)| {
                let (x, y) = self.strip.in_strip(page, band.x, band.y)?;
                Some([
                    x,
                    y,
                    f64::from(band.width) * scale,
                    f64::from(band.height) * scale,
                ])
            })
            .collect()
    }

    /// The pages any part of which is between `top` and `bottom`.
    #[must_use]
    pub fn visible(&self, top: f64, bottom: f64) -> Range<usize> {
        self.strip.visible(top, bottom)
    }

    /// The blocks with a line on `pages`.
    #[must_use]
    pub fn blocks(&self, pages: Range<usize>) -> Option<Range<usize>> {
        place::blocks_on(&self.set, pages)
    }

    /// Page `page`'s rectangle in the strip.
    #[must_use]
    pub fn sheet(&self, page: usize) -> Option<Rect> {
        let sheet = self.strip.sheet(page)?;
        Some([sheet.x, sheet.y, sheet.width, sheet.height])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::{App, BlockKind};

    fn shown() -> Shown {
        let app = App::new();
        let paragraph = "All work and no play makes a dull document. ".repeat(40);
        for at in 0..8 {
            app.insert(at, BlockKind::Paragraph, &paragraph).unwrap();
        }
        let session = crate::session::Session::new(true);
        Shown::new(session.document(&app), 96.0 / 72.0, 900.0)
    }

    #[test]
    fn a_click_where_a_caret_is_drawn_is_that_caret() {
        let shown = shown();
        let at = Caret {
            block: 3,
            offset: 40,
        };
        let stroke = shown.stroke(at).unwrap();
        assert!(stroke.page > 0, "on a later page");
        assert_eq!(
            shown.hit(stroke.x, stroke.y + stroke.height / 2.0),
            Some(at)
        );
    }

    #[test]
    fn a_selection_is_a_rectangle_a_line_on_the_pages_asked_about() {
        let shown = shown();
        let at = |offset| Caret { block: 0, offset };
        let all = shown.bands(at(0), at(300), 0..shown.set.pages.len());
        assert!(all.len() >= 2);
        assert!(all[1][1] > all[0][1]);
        assert_eq!(shown.bands(at(0), at(300), 1..2), Vec::<Rect>::new());
        assert_eq!(shown.blocks(0..1).map(|blocks| blocks.start), Some(0));
    }
}
