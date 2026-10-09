// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The page on paper, the portable half: what Print Preview and Print decide before a pixel
//! (`doc/pdf-export.md`).
//!
//! Both draw `grind_print::raster`'s pixels of the display list the PDF is written from, so the
//! preview, the printer and the exported file are one typesetting. What is decided here: how a
//! page fits a window, what the window's title says, the resolution a printer is sent, and the
//! one conversion between the raster's channel order and GDI's.

/// A page's room from the window's edge, in device-independent pixels.
pub const MARGIN: f64 = 16.0;

/// The highest resolution a page is rasterised at for a printer. A printer reporting 600 dpi
/// would otherwise be sent 140 MB of A4 per page; 300 dpi is what an office printer resolves of
/// text set at body size, and GDI scales the rest.
pub const PRINT_DPI: u32 = 300;

/// `grind_print::raster`'s premultiplied RGBA as the premultiplied BGRA `gdi::blit_image` takes.
pub fn bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for pixel in out.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    out
}

/// Where a page `page` points across sits in an area `area` pixels across, with [`MARGIN`]
/// (scaled to `dpi`) all round: the scale in pixels per point, and the page's left,
/// top, width and height. The whole page always fits; a window smaller than the margins shows a
/// page of one pixel rather than none.
pub fn fit(page: (f32, f32), area: (i32, i32), dpi: u32) -> (f32, (i32, i32, i32, i32)) {
    let margin = (MARGIN * f64::from(dpi.max(1)) / 96.0).round() as i32;
    let room = (
        (area.0 - 2 * margin).max(1) as f32,
        (area.1 - 2 * margin).max(1) as f32,
    );
    let (pw, ph) = (page.0.max(1.0), page.1.max(1.0));
    let scale = (room.0 / pw).min(room.1 / ph);
    let (w, h) = (
        ((pw * scale).round() as i32).max(1),
        ((ph * scale).round() as i32).max(1),
    );
    (scale, ((area.0 - w) / 2, margin, w, h))
}

/// The preview's title: which page of how many, and on what paper.
pub fn title(page: usize, pages: usize, paper: &grind_core::page::PageGeometry) -> String {
    let name = paper.iso_name().unwrap_or_else(|| {
        let mm = |v: f64| (v * 10.0).round() / 10.0;
        format!("{} × {} mm", mm(paper.width), mm(paper.height))
    });
    format!("Page {} of {pages} · {name}", page + 1)
}

/// Pixels per point to rasterise a page for a printer reporting `dpi`.
pub fn print_scale(dpi: u32) -> f32 {
    match dpi {
        0 => 1.0,
        dpi => dpi.min(PRINT_DPI) as f32 / 72.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::page::PageGeometry;

    #[test]
    fn red_and_blue_change_places_and_nothing_else_does() {
        assert_eq!(
            bgra(&[1, 2, 3, 4, 5, 6, 7, 8]),
            vec![3, 2, 1, 4, 7, 6, 5, 8]
        );
        assert_eq!(bgra(&[]), Vec::<u8>::new());
    }

    #[test]
    fn a_page_fits_the_window_whole_and_centred() {
        // A4 in a window 800 × 600 at 96 dpi: the height is what binds.
        let (scale, (x, y, w, h)) = fit((595.0, 842.0), (800, 600), 96);
        assert_eq!(y, 16);
        assert_eq!(h, 600 - 32);
        assert!((scale - 568.0 / 842.0).abs() < 1e-4, "{scale}");
        assert_eq!(w, (595.0 * scale).round() as i32);
        assert_eq!(x, (800 - w) / 2, "centred across");
        // At 192 dpi the margin is twice as many pixels.
        let (_, (_, y2, _, _)) = fit((595.0, 842.0), (800, 600), 192);
        assert_eq!(y2, 32);
        // And a window too small for the margins still shows a page.
        let (_, (_, _, w3, h3)) = fit((595.0, 842.0), (10, 10), 96);
        assert!(w3 >= 1 && h3 >= 1);
    }

    #[test]
    fn the_title_counts_pages_and_names_the_paper() {
        assert_eq!(title(0, 3, &PageGeometry::a4()), "Page 1 of 3 · A4");
        let letter = PageGeometry {
            width: 215.9,
            height: 279.4,
            ..PageGeometry::a4()
        };
        assert_eq!(title(1, 2, &letter), "Page 2 of 2 · 215.9 × 279.4 mm");
    }

    #[test]
    fn a_printer_is_sent_no_more_than_three_hundred_dpi() {
        assert!((print_scale(150) - 150.0 / 72.0).abs() < 1e-6);
        assert!((print_scale(600) - 300.0 / 72.0).abs() < 1e-6);
        assert!(
            (print_scale(0) - 1.0).abs() < 1e-6,
            "a printer that says nothing gets 72"
        );
    }
}
