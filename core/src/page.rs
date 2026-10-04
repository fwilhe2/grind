// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The size of a page and where its text goes. **\[GENERIC\]**
//!
//! `doc/pdf-export.md` is normative here. A page is a `style:page-layout` (rng:12213), whose
//! `style:page-layout-properties` (rng:12248) carry `fo:page-width` and `fo:page-height`
//! (rng:12255, rng:12260) and four margins. Both document types have one, which is why this is
//! the core's: nothing here mentions a paragraph or a cell (R8).
//!
//! **What this is not.** There is no pagination here. Breaking a document into pages needs its
//! own vocabulary (a heading kept with what follows it, a table row never split), so it lives
//! with the document type. This module answers only "how big, and how far in".
//!
//! **Paper is ISO 216.** When a document states no page, the answer is [`PageGeometry::a4`],
//! never a size chosen by locale (`doc/pdf-export.md` §8, decision 7). A document that *says*
//! US Letter is still read as US Letter: its own geometry always wins, and we only refuse to
//! *choose* one.

use crate::style::length_mm;

/// Millimetres per PostScript point, the unit a PDF's user space is in.
const MM_PER_PT: f64 = 25.4 / 72.0;

/// One page's size and margins, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGeometry {
    pub width: f64,
    pub height: f64,
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
}

impl Default for PageGeometry {
    fn default() -> Self {
        PageGeometry::a4()
    }
}

impl PageGeometry {
    /// A4 portrait with 2 cm margins all round: the page a document gets when it states none.
    pub fn a4() -> Self {
        PageGeometry::iso_a(4).expect("A4 is in the series")
    }

    /// ISO 216's A series, `A0` through `A10`, portrait, with 2 cm margins.
    ///
    /// Derived rather than tabled: A0 is 841 × 1189 mm (one square metre), and each next size
    /// halves the longer side, rounding down to a whole millimetre. That rule is the standard's
    /// own, and it is what makes A5 148 mm wide rather than 148.5.
    pub fn iso_a(n: u8) -> Option<Self> {
        if n > 10 {
            return None;
        }
        let (mut short, mut long) = (841u32, 1189u32);
        for _ in 0..n {
            (short, long) = (long / 2, short);
        }
        Some(PageGeometry {
            width: f64::from(short),
            height: f64::from(long),
            top: 20.0,
            bottom: 20.0,
            left: 20.0,
            right: 20.0,
        })
    }

    /// A paper name as a person types it — `a4`, `A3`, `a4-landscape` — and nothing else.
    ///
    /// Only the A series: the point of the decision is that this program never offers a US
    /// size, so it does not parse one either.
    pub fn paper(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let (size, landscape) = match name.strip_suffix("-landscape") {
            Some(size) => (size, true),
            None => (name.as_str(), false),
        };
        let page = PageGeometry::iso_a(size.strip_prefix('a')?.parse().ok()?)?;
        Some(if landscape { page.landscape() } else { page })
    }

    /// The same page turned on its side: the longer edge across.
    pub fn landscape(self) -> Self {
        if self.width >= self.height {
            return self;
        }
        PageGeometry {
            width: self.height,
            height: self.width,
            ..self
        }
    }

    /// Read from `style:page-layout-properties`. `get` answers an `fo:` attribute by its local
    /// name, which keeps this module free of the reader's types.
    ///
    /// A size that is missing or unreadable falls back to A4's, side by side, so a layout that
    /// states only margins still has a page. A **margin** that is missing is zero: that is
    /// XSL-FO's initial value, which ODF takes over, and a layout that bothered to exist and
    /// said nothing about a margin meant none. `fo:margin` is the shorthand for all four, and a
    /// side stated on its own wins over it.
    pub fn from_properties<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Self {
        let a4 = PageGeometry::a4();
        let length = |name: &str| get(name).and_then(length_mm).filter(|mm| *mm >= 0.0);
        let all = length("margin").unwrap_or(0.0);
        let side = |name: &str| length(name).unwrap_or(all);
        PageGeometry {
            width: length("page-width")
                .filter(|w| *w > 0.0)
                .unwrap_or(a4.width),
            height: length("page-height")
                .filter(|h| *h > 0.0)
                .unwrap_or(a4.height),
            top: side("margin-top"),
            bottom: side("margin-bottom"),
            left: side("margin-left"),
            right: side("margin-right"),
        }
    }

    /// The width text is set in: the page less its side margins, never negative.
    pub fn text_width(&self) -> f64 {
        (self.width - self.left - self.right).max(0.0)
    }

    /// The height text is set in: the page less its top and bottom margins, never negative.
    pub fn text_height(&self) -> f64 {
        (self.height - self.top - self.bottom).max(0.0)
    }

    /// The ISO 216 name of this size if it is one, either way up: `"A4"`, `"A5"`.
    pub fn iso_name(&self) -> Option<String> {
        let (short, long) = (self.width.min(self.height), self.width.max(self.height));
        (0..=10u8).find_map(|n| {
            let a = PageGeometry::iso_a(n)?;
            ((short - a.width).abs() < 0.5 && (long - a.height).abs() < 0.5)
                .then(|| format!("A{n}"))
        })
    }
}

/// Millimetres to PostScript points.
pub fn pt(mm: f64) -> f64 {
    mm / MM_PER_PT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<&'a str> {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }

    #[test]
    fn the_a_series_follows_the_standard_halving_rule() {
        let sizes: Vec<(f64, f64)> = (0..=10)
            .map(|n| {
                let p = PageGeometry::iso_a(n).unwrap();
                (p.width, p.height)
            })
            .collect();
        assert_eq!(sizes[0], (841.0, 1189.0));
        assert_eq!(sizes[3], (297.0, 420.0));
        assert_eq!(sizes[4], (210.0, 297.0));
        assert_eq!(sizes[5], (148.0, 210.0));
        assert_eq!(sizes[10], (26.0, 37.0));
        assert_eq!(PageGeometry::iso_a(11), None);
    }

    #[test]
    fn a_document_with_no_page_gets_a4_with_two_centimetre_margins() {
        let page = PageGeometry::default();
        assert_eq!((page.width, page.height), (210.0, 297.0));
        assert_eq!(
            (page.top, page.bottom, page.left, page.right),
            (20.0, 20.0, 20.0, 20.0)
        );
        assert_eq!(page.text_width(), 170.0);
        assert_eq!(page.text_height(), 257.0);
    }

    #[test]
    fn paper_names_are_the_a_series_and_nothing_else() {
        assert_eq!(PageGeometry::paper("a4"), Some(PageGeometry::a4()));
        assert_eq!(PageGeometry::paper(" A3 ").map(|p| p.width), Some(297.0));
        let landscape = PageGeometry::paper("a4-landscape").unwrap();
        assert_eq!((landscape.width, landscape.height), (297.0, 210.0));
        assert_eq!(PageGeometry::paper("letter"), None);
        assert_eq!(PageGeometry::paper("legal"), None);
        assert_eq!(PageGeometry::paper("a11"), None);
        assert_eq!(PageGeometry::paper("b5"), None);
    }

    #[test]
    fn a_page_layout_is_read_in_whatever_units_it_was_written() {
        let page = PageGeometry::from_properties(props(&[
            ("page-width", "21.001cm"),
            ("page-height", "29.7cm"),
            ("margin-top", "2cm"),
            ("margin-bottom", "20mm"),
            ("margin-left", "1in"),
            ("margin-right", "72pt"),
        ]));
        assert!((page.width - 210.01).abs() < 1e-9);
        assert!((page.height - 297.0).abs() < 1e-9);
        assert!((page.top - 20.0).abs() < 1e-9);
        assert!((page.bottom - 20.0).abs() < 1e-9);
        assert!((page.left - 25.4).abs() < 1e-9);
        assert!((page.right - 25.4).abs() < 1e-9);
        assert_eq!(page.iso_name().as_deref(), Some("A4"));
    }

    #[test]
    fn a_missing_margin_is_zero_and_a_missing_size_is_a4s() {
        let page = PageGeometry::from_properties(props(&[("margin-left", "1cm")]));
        assert_eq!((page.width, page.height), (210.0, 297.0));
        assert_eq!(
            (page.top, page.bottom, page.left, page.right),
            (0.0, 0.0, 10.0, 0.0)
        );
    }

    #[test]
    fn the_margin_shorthand_covers_every_side_not_stated_on_its_own() {
        let page =
            PageGeometry::from_properties(props(&[("margin", "1cm"), ("margin-top", "3cm")]));
        assert_eq!(
            (page.top, page.bottom, page.left, page.right),
            (30.0, 10.0, 10.0, 10.0)
        );
    }

    #[test]
    fn nonsense_is_ignored_rather_than_believed() {
        let page = PageGeometry::from_properties(props(&[
            ("page-width", "wide"),
            ("page-height", "-3cm"),
            ("margin-top", "-1cm"),
        ]));
        assert_eq!((page.width, page.height, page.top), (210.0, 297.0, 0.0));
    }

    #[test]
    fn us_letter_is_kept_when_the_document_says_so_but_has_no_iso_name() {
        let page = PageGeometry::from_properties(props(&[
            ("page-width", "8.5in"),
            ("page-height", "11in"),
        ]));
        assert!((page.width - 215.9).abs() < 1e-9);
        assert_eq!(page.iso_name(), None);
        assert_eq!(
            PageGeometry::a4().landscape().iso_name().as_deref(),
            Some("A4")
        );
    }

    #[test]
    fn a_millimetre_is_about_two_point_eight_points() {
        assert!((pt(25.4) - 72.0).abs() < 1e-9);
        assert!((pt(210.0) - 595.2755905511812).abs() < 1e-9);
    }
}
