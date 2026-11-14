// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where each page sits in the window: a column of sheets on a desk, centred, scrolled as one
//! (`doc/page-model.md` PM5). Pure arithmetic between the window's pixels and a page's points;
//! what is *on* a page is `grind_print`'s.

use std::ops::Range;

/// The desk showing round the pages, and between one and the next, in pixels.
pub const PAD: f64 = 24.0;
pub const GAP: f64 = 16.0;

/// One page's rectangle in the strip, in pixels from the strip's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sheet {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Every page's place at one zoom in a window one width.
#[derive(Clone, Debug)]
pub struct Strip {
    /// Pixels per point.
    scale: f64,
    sheets: Vec<Sheet>,
    width: f64,
    height: f64,
}

impl Strip {
    /// Stack pages of these sizes, in points, at `scale` pixels per point in a window `window`
    /// pixels wide. A sheet is whole pixels, rounded down as a raster of it is.
    pub fn new(sizes: impl IntoIterator<Item = (f32, f32)>, scale: f64, window: f64) -> Self {
        let sizes: Vec<(f64, f64)> = sizes
            .into_iter()
            .map(|(w, h)| {
                (
                    (f64::from(w) * scale).floor(),
                    (f64::from(h) * scale).floor(),
                )
            })
            .collect();
        let widest = sizes.iter().map(|size| size.0).fold(0.0, f64::max);
        let width = (widest + 2.0 * PAD).max(window);
        let mut y = PAD;
        let sheets = sizes
            .into_iter()
            .map(|(w, h)| {
                let sheet = Sheet {
                    x: ((width - w) / 2.0).floor(),
                    y,
                    width: w,
                    height: h,
                };
                y += h + GAP;
                sheet
            })
            .collect::<Vec<_>>();
        let height = match sheets.last() {
            Some(last) => last.y + last.height + PAD,
            None => 2.0 * PAD,
        };
        Strip {
            scale,
            sheets,
            width,
            height,
        }
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// How wide and how tall the whole strip is: what the scrollbars cover.
    pub fn size(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    pub fn sheet(&self, page: usize) -> Option<Sheet> {
        self.sheets.get(page).copied()
    }

    /// The pages any part of which is between `top` and `bottom`.
    pub fn visible(&self, top: f64, bottom: f64) -> Range<usize> {
        let from = self
            .sheets
            .partition_point(|sheet| sheet.y + sheet.height < top);
        let to = self.sheets.partition_point(|sheet| sheet.y <= bottom);
        from..to.max(from)
    }

    /// The page at a height in the strip — between two, the nearer; `None` with no pages.
    pub fn page_at(&self, y: f64) -> Option<usize> {
        let below = self
            .sheets
            .partition_point(|sheet| sheet.y + sheet.height < y);
        let Some(next) = self.sheets.get(below) else {
            return self.sheets.len().checked_sub(1);
        };
        match below.checked_sub(1).and_then(|at| self.sheets.get(at)) {
            Some(prev) if y < next.y && y - (prev.y + prev.height) < next.y - y => Some(below - 1),
            _ => Some(below),
        }
    }

    /// A point of the strip, in points on `page` — outside the sheet when the point is.
    pub fn on_page(&self, page: usize, x: f64, y: f64) -> Option<(f32, f32)> {
        let sheet = self.sheet(page)?;
        Some((
            ((x - sheet.x) / self.scale) as f32,
            ((y - sheet.y) / self.scale) as f32,
        ))
    }

    /// A point on `page`, in the strip's pixels.
    pub fn in_strip(&self, page: usize, x: f32, y: f32) -> Option<(f64, f64)> {
        let sheet = self.sheet(page)?;
        Some((
            sheet.x + f64::from(x) * self.scale,
            sheet.y + f64::from(y) * self.scale,
        ))
    }
}

/// Where a scroll must be for a stretch `from..from + length` to be in a view `view` long over
/// content `content` long, moving as little as possible.
pub fn follow(scroll: f64, view: f64, content: f64, from: f64, length: f64) -> f64 {
    let scroll = match scroll {
        _ if from < scroll => from,
        s if from + length > s + view => from + length - view,
        s => s,
    };
    scroll.clamp(0.0, (content - view).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4: (f32, f32) = (595.0, 842.0);

    #[test]
    fn pages_are_stacked_with_the_desk_between_them_and_centred() {
        let strip = Strip::new([A4, A4], 1.0, 1000.0);
        let (first, second) = (strip.sheet(0).unwrap(), strip.sheet(1).unwrap());
        assert_eq!((first.x, first.y), (202.0, PAD));
        assert_eq!(second.y, PAD + 842.0 + GAP);
        assert_eq!(strip.size(), (1000.0, PAD + 842.0 + GAP + 842.0 + PAD));
        assert_eq!(strip.sheet(2), None);
    }

    #[test]
    fn a_page_wider_than_the_window_keeps_its_desk_and_the_strip_widens() {
        let strip = Strip::new([A4], 2.0, 800.0);
        assert_eq!(strip.size().0, 1190.0 + 2.0 * PAD);
        assert_eq!(strip.sheet(0).unwrap().x, PAD);
        // A narrower page beside a wider one is centred on the same axis.
        let mixed = Strip::new([A4, (842.0, 595.0)], 1.0, 400.0);
        let (portrait, landscape) = (mixed.sheet(0).unwrap(), mixed.sheet(1).unwrap());
        assert_eq!(landscape.x, PAD);
        assert_eq!(
            portrait.x + portrait.width / 2.0,
            landscape.x + landscape.width / 2.0 - 0.5
        );
    }

    #[test]
    fn the_pages_in_view_are_the_ones_any_part_of_which_shows() {
        let strip = Strip::new([A4, A4, A4], 1.0, 1000.0);
        assert_eq!(strip.visible(0.0, 400.0), 0..1);
        assert_eq!(strip.visible(800.0, 1200.0), 0..2);
        let second = strip.sheet(1).unwrap();
        assert_eq!(strip.visible(second.y + 10.0, second.y + 20.0), 1..2);
        assert_eq!(strip.visible(9000.0, 9100.0), 3..3);
        // Only the desk between two pages is in view.
        let gap = strip.sheet(0).unwrap().y + 842.0 + 2.0;
        assert_eq!(strip.visible(gap, gap + 4.0), 1..1);
    }

    #[test]
    fn a_point_between_two_pages_is_on_the_nearer() {
        let strip = Strip::new([A4, A4], 1.0, 1000.0);
        let end = PAD + 842.0;
        assert_eq!(strip.page_at(0.0), Some(0));
        assert_eq!(strip.page_at(end - 1.0), Some(0));
        assert_eq!(strip.page_at(end + 3.0), Some(0));
        assert_eq!(strip.page_at(end + GAP - 3.0), Some(1));
        assert_eq!(strip.page_at(99_999.0), Some(1));
        assert_eq!(Strip::new([], 1.0, 1000.0).page_at(10.0), None);
    }

    #[test]
    fn points_and_pixels_convert_both_ways_at_a_zoom() {
        let strip = Strip::new([A4, A4], 1.5, 1200.0);
        let sheet = strip.sheet(1).unwrap();
        let (x, y) = strip.in_strip(1, 100.0, 200.0).unwrap();
        assert_eq!((x, y), (sheet.x + 150.0, sheet.y + 300.0));
        assert_eq!(strip.on_page(1, x, y), Some((100.0, 200.0)));
        // Left of the sheet is a negative x on it: the margin, to whatever asks.
        assert!(strip.on_page(1, sheet.x - 15.0, sheet.y).unwrap().0 < 0.0);
        assert_eq!(strip.on_page(5, 0.0, 0.0), None);
    }

    #[test]
    fn a_scroll_follows_a_stretch_by_as_little_as_it_can() {
        assert_eq!(follow(100.0, 400.0, 2000.0, 150.0, 20.0), 100.0, "in view");
        assert_eq!(follow(100.0, 400.0, 2000.0, 40.0, 20.0), 40.0, "above");
        assert_eq!(follow(100.0, 400.0, 2000.0, 600.0, 20.0), 220.0, "below");
        assert_eq!(
            follow(100.0, 400.0, 2000.0, 1990.0, 20.0),
            1600.0,
            "the end"
        );
        assert_eq!(
            follow(0.0, 400.0, 300.0, 250.0, 20.0),
            0.0,
            "nothing to scroll"
        );
    }
}
