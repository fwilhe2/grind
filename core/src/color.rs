// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **\[GENERIC\]** Colour arithmetic every shell draws with — WCAG contrast, hue-keeping lightness
//! moves, and ODF's automatic ink — so that no two shells disagree about what can be read on what.
//! A colour is `(r, g, b)`; each shell converts at its edge. Nothing here is written into a
//! document.

/// A colour as three bytes, red, green, blue.
pub type Rgb = (u8, u8, u8);

/// WCAG's floor for small text — what a cell's ink has to reach against the ground it sits on.
pub const TEXT: f64 = 4.5;

/// WCAG's floor for a line or a shape a reader has to be able to see.
pub const VISIBLE: f64 = 3.0;

/// A colour parsed from `#rrggbb`, the only form ODF stores (§5.1).
pub fn parse(hex: &str) -> Option<Rgb> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some((byte(0)?, byte(2)?, byte(4)?))
}

/// WCAG relative luminance — the perceptual weight of a colour, on 0.0 to 1.0.
pub fn luminance((r, g, b): Rgb) -> f64 {
    let channel = |value: u8| {
        let v = f64::from(value) / 255.0;
        match v <= 0.040_45 {
            true => v / 12.92,
            false => ((v + 0.055) / 1.055).powf(2.4),
        }
    };
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// The WCAG contrast ratio between two colours, from 1.0 (identical) to 21.0 (black on white).
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    let (light, dark) = match a > b {
        true => (a, b),
        false => (b, a),
    };
    (light + 0.05) / (dark + 0.05)
}

/// Hue (degrees), saturation and lightness — the space a colour is moved in when it has to
/// stay recognisably itself.
pub fn hsl((r, g, b): Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = match l > 0.5 {
        true => d / (2.0 - max - min),
        false => d / (max + min),
    };
    let h = if (max - r).abs() < f64::EPSILON {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f64::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

/// [`hsl`]'s inverse.
pub fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let (h, s, l) = (h.rem_euclid(360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(r), byte(g), byte(b))
}

/// The ink for text the document did not colour, on a ground the document did choose — ODF's
/// automatic colour: the theme's `ink` where it reads, else black or white, whichever reads
/// better. Without the fallback, a dark theme's near-white text on a document's pale heading row
/// is unreadable.
pub fn automatic_ink(ground: Rgb, ink: Rgb) -> Rgb {
    if contrast(ink, ground) >= TEXT {
        return ink;
    }
    let (black, white) = ((0, 0, 0), (0xff, 0xff, 0xff));
    match contrast(black, ground) > contrast(white, ground) {
        true => black,
        false => white,
    }
}

/// The ink for a cell's or a run's text, drawn with `ink` on `page`:
///
/// - No colour of its own, no fill: `ink`.
/// - No colour of its own, on a fill of its own: [`automatic_ink`] on that fill.
/// - A colour of its own on a `dark` page: lifted along its own hue until it reads ([`legible`]).
/// - Anything else: the document's colour, as it is.
pub fn document_ink(own: Option<Rgb>, fill: Option<Rgb>, page: Rgb, ink: Rgb, dark: bool) -> Rgb {
    match (own, fill) {
        (None, None) => ink,
        (None, Some(fill)) => automatic_ink(fill, ink),
        (Some(own), None) if dark => legible(own, page, TEXT),
        (Some(own), _) => own,
    }
}

/// `color`, moved along its own hue — lighter on a dark ground, darker on a light one — until it
/// reaches `floor` against `ground`. Lightness rather than a blend towards white or black, which
/// would desaturate it. When no lightness on that hue reads, the nearest comes back.
pub fn legible(color: Rgb, ground: Rgb, floor: f64) -> Rgb {
    if contrast(color, ground) >= floor {
        return color;
    }
    let (h, s, l) = hsl(color);
    let step = match luminance(ground) < 0.5 {
        true => 0.02,
        false => -0.02,
    };
    let mut best = color;
    for n in 1..=50 {
        let candidate = from_hsl(h, s, l + step * f64::from(n));
        best = candidate;
        if contrast(candidate, ground) >= floor {
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_documents_ink_follows_the_four_rules() {
        let (page, ink) = (DARK, WHITE);
        let navy = (0x00, 0x1f, 0x3f);
        let silver = (0xdd, 0xdd, 0xdd);
        assert_eq!(
            document_ink(None, None, page, ink, true),
            ink,
            "the page's own"
        );
        assert_eq!(
            document_ink(None, Some(silver), page, ink, true),
            BLACK,
            "automatic on a pale fill, even on a dark page"
        );
        let lifted = document_ink(Some(navy), None, page, ink, true);
        assert!(
            contrast(lifted, page) >= TEXT,
            "the navy reads on the dark page"
        );
        assert_eq!(
            hsl(lifted).0.round(),
            hsl(navy).0.round(),
            "and is still navy"
        );
        assert_eq!(
            document_ink(Some(navy), Some(silver), page, ink, true),
            navy,
            "on its own fill, the document's decision"
        );
        assert_eq!(
            document_ink(Some(navy), None, WHITE, BLACK, false),
            navy,
            "on a light page, as it is"
        );
    }

    const WHITE: Rgb = (0xff, 0xff, 0xff);
    const BLACK: Rgb = (0, 0, 0);
    /// Adwaita's dark view background, roughly — the ground a dark GNOME sheet is drawn on.
    const DARK: Rgb = (0x1e, 0x1e, 0x1e);

    #[test]
    fn contrast_is_wcags() {
        assert!((contrast(BLACK, WHITE) - 21.0).abs() < 1e-9);
        assert!((contrast(WHITE, WHITE) - 1.0).abs() < 1e-9);
        assert_eq!(parse("#0074d9"), Some((0x00, 0x74, 0xd9)));
        assert_eq!(parse("navy"), None);
    }

    #[test]
    fn hsl_round_trips() {
        for color in [
            (0x00, 0x74, 0xd9),
            (0xff, 0x41, 0x36),
            (0x3d, 0x99, 0x70),
            (0x80, 0x80, 0x80),
        ] {
            let (h, s, l) = hsl(color);
            let back = from_hsl(h, s, l);
            let near = |a: u8, b: u8| a.abs_diff(b) <= 1;
            assert!(near(back.0, color.0) && near(back.1, color.1) && near(back.2, color.2));
        }
    }

    /// The theme's ink where it reads; black or white where the document's own fill makes it not.
    #[test]
    fn automatic_ink_reads_on_whatever_ground_the_document_chose() {
        let light_ink = (0xee, 0xee, 0xee);
        let silver = (0xdd, 0xdd, 0xdd);
        assert_eq!(automatic_ink(DARK, light_ink), light_ink);
        assert_eq!(automatic_ink(silver, light_ink), BLACK);
        assert_eq!(automatic_ink((0x00, 0x1f, 0x3f), BLACK), WHITE);
    }

    /// Every colour a document can pick from the suite's palette reads on a dark sheet once it is
    /// lifted — and keeps its hue doing so.
    #[test]
    fn every_palette_colour_is_lifted_until_it_reads_on_a_dark_sheet() {
        for (name, hex) in crate::style::PALETTE {
            let color = parse(hex).unwrap();
            let lifted = legible(color, DARK, TEXT);
            assert!(
                contrast(lifted, DARK) >= TEXT,
                "{name} lifted to {lifted:?} still does not read"
            );
            if contrast(color, DARK) >= TEXT {
                assert_eq!(lifted, color, "{name} already read and was moved anyway");
            } else if hsl(color).1 > 0.1 {
                let hue = |c: Rgb| hsl(c).0;
                let turned = (hue(lifted) - hue(color)).abs();
                assert!(turned.min(360.0 - turned) < 12.0, "{name} changed hue");
            }
        }
    }
}
