// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The four ways SpreadsheetML says a colour, resolved to the `#rrggbb` ODF stores.
//!
//! ```text
//! <color rgb="FF0000FF"/>             ARGB; the alpha is dropped
//! <color indexed="12"/>               the legacy palette, `PALETTE` below
//! <color theme="4" tint="-0.25"/>     the workbook's theme, then ECMA's tint
//! <color auto="1"/>                   the system's ink: no colour to carry
//! ```
//!
//! **Only `rgb` is self-contained.** An index needs a 64-entry table, a theme slot needs the
//! theme part, and a tint needs arithmetic in HSL space. `doc/xlsx-format.md` §4.1 and §4.2
//! hold the measurements this module is built from: the palette was read back from the oracle
//! one index per cell rather than transcribed, and the theme's slot order is the one place the
//! obvious reading is wrong.
//!
//! **Automatic is an answer, not a failure.** `auto="1"`, index 64 and index 65 name the
//! system's foreground and background — ODF's *automatic* colour, which the model spells as
//! no colour at all. Writing `#000000` in its place would be a fixed black on a dark theme,
//! which is the bug `ui_win32`'s `theme::automatic_ink` exists to avoid. What cannot be
//! resolved — a theme slot with no theme to look it up in — is [`Resolved::Unknown`], and
//! the caller counts it.

use crate::xml::{Attrs, Handled, Reader};

/// A colour as the file spelled it, before anything is looked up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Color {
    pub spelling: Spelling,
    /// `tint`, −1 to 1: darker below zero, lighter above. Applies to every spelling, though
    /// Excel only writes it beside `theme`.
    pub tint: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Spelling {
    /// `auto="1"`, or a `<color/>` that says nothing.
    #[default]
    Auto,
    Rgb([u8; 3]),
    Indexed(u32),
    Theme(u32),
}

/// What a colour turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// `#rrggbb`, lower-case — the spelling every writer in this workspace uses.
    Hex(String),
    /// The system's ink or paper: ODF's automatic colour, which the model spells as absent.
    Automatic,
    /// A reference to something the workbook does not define.
    Unknown(Missing),
}

/// Why a colour could not be resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// A `theme` slot, and no theme part to look it up in — or a slot past the scheme's twelve.
    Theme,
    /// An `indexed` value past the palette.
    Index,
}

impl Resolved {
    /// The colour a style carries: `None` for automatic, and for anything unresolved, which
    /// the caller has already counted.
    pub fn hex(&self) -> Option<&str> {
        match self {
            Resolved::Hex(hex) => Some(hex),
            _ => None,
        }
    }
}

/// ECMA-376 §18.8.27's legacy palette, **measured** rather than transcribed: a workbook with
/// one cell per index, converted by LibreOffice 26.8 (not CI's pinned 26.2) and read
/// back, 2026-09-21 — `doc/xlsx-format.md` §4.1. The first eight repeat as the next eight;
/// 64 and 65 are the system's foreground and background and are not in the table.
pub const PALETTE: [[u8; 3]; 64] = [
    [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xFF],
    [0xFF, 0x00, 0x00],
    [0x00, 0xFF, 0x00],
    [0x00, 0x00, 0xFF],
    [0xFF, 0xFF, 0x00],
    [0xFF, 0x00, 0xFF],
    [0x00, 0xFF, 0xFF],
    [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xFF],
    [0xFF, 0x00, 0x00],
    [0x00, 0xFF, 0x00],
    [0x00, 0x00, 0xFF],
    [0xFF, 0xFF, 0x00],
    [0xFF, 0x00, 0xFF],
    [0x00, 0xFF, 0xFF],
    [0x80, 0x00, 0x00],
    [0x00, 0x80, 0x00],
    [0x00, 0x00, 0x80],
    [0x80, 0x80, 0x00],
    [0x80, 0x00, 0x80],
    [0x00, 0x80, 0x80],
    [0xC0, 0xC0, 0xC0],
    [0x80, 0x80, 0x80],
    [0x99, 0x99, 0xFF],
    [0x99, 0x33, 0x66],
    [0xFF, 0xFF, 0xCC],
    [0xCC, 0xFF, 0xFF],
    [0x66, 0x00, 0x66],
    [0xFF, 0x80, 0x80],
    [0x00, 0x66, 0xCC],
    [0xCC, 0xCC, 0xFF],
    [0x00, 0x00, 0x80],
    [0xFF, 0x00, 0xFF],
    [0xFF, 0xFF, 0x00],
    [0x00, 0xFF, 0xFF],
    [0x80, 0x00, 0x80],
    [0x80, 0x00, 0x00],
    [0x00, 0x80, 0x80],
    [0x00, 0x00, 0xFF],
    [0x00, 0xCC, 0xFF],
    [0xCC, 0xFF, 0xFF],
    [0xCC, 0xFF, 0xCC],
    [0xFF, 0xFF, 0x99],
    [0x99, 0xCC, 0xFF],
    [0xFF, 0x99, 0xCC],
    [0xCC, 0x99, 0xFF],
    [0xFF, 0xCC, 0x99],
    [0x33, 0x66, 0xFF],
    [0x33, 0xCC, 0xCC],
    [0x99, 0xCC, 0x00],
    [0xFF, 0xCC, 0x00],
    [0xFF, 0x99, 0x00],
    [0xFF, 0x66, 0x00],
    [0x66, 0x66, 0x99],
    [0x96, 0x96, 0x96],
    [0x00, 0x33, 0x66],
    [0x33, 0x99, 0x66],
    [0x00, 0x33, 0x00],
    [0x33, 0x33, 0x00],
    [0x99, 0x33, 0x00],
    [0x99, 0x33, 0x66],
    [0x33, 0x33, 0x99],
    [0x33, 0x33, 0x33],
];

/// What one workbook's colours are looked up in: its palette (the legacy one unless
/// `<colors><indexedColors>` replaces it) and its theme's twelve slots.
#[derive(Clone, Debug)]
pub struct Palette {
    pub indexed: Vec<[u8; 3]>,
    /// In **`theme` attribute order**, which is not the scheme's document order — see
    /// [`crate::theme`]. Empty when the workbook has no theme part this build can read.
    pub theme: Vec<[u8; 3]>,
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            indexed: PALETTE.to_vec(),
            theme: Vec::new(),
        }
    }
}

impl Palette {
    pub fn resolve(&self, color: &Color) -> Resolved {
        let base = match color.spelling {
            Spelling::Auto => return Resolved::Automatic,
            Spelling::Rgb(rgb) => rgb,
            // 64 and 65 are the system's foreground and background (§18.8.27): automatic,
            // whatever a replaced palette says about them.
            Spelling::Indexed(64 | 65) => return Resolved::Automatic,
            Spelling::Indexed(i) => match self.indexed.get(i as usize) {
                Some(rgb) => *rgb,
                None => return Resolved::Unknown(Missing::Index),
            },
            Spelling::Theme(i) => match self.theme.get(i as usize) {
                Some(rgb) => *rgb,
                None => return Resolved::Unknown(Missing::Theme),
            },
        };
        Resolved::Hex(hex(tint(base, color.tint)))
    }
}

/// Read a `<color>`-shaped element's attributes (`<color>`, `<fgColor>`, `<bgColor>` — one
/// shape under three names). The first spelling present wins, in the order the spec lists
/// them; [`Spelling::Auto`] when the element names no colour at all.
pub fn read(attrs: &Attrs) -> Color {
    let tint = attrs
        .plain("tint")
        .and_then(|t| t.trim().parse::<f64>().ok())
        .filter(|t| t.is_finite())
        .map_or(0.0, |t| t.clamp(-1.0, 1.0));
    let spelling = if attrs.flag("auto") {
        Spelling::Auto
    } else if let Some(rgb) = attrs.plain("rgb").and_then(argb) {
        Spelling::Rgb(rgb)
    } else if let Some(i) = attrs.plain("indexed").and_then(|i| i.trim().parse().ok()) {
        Spelling::Indexed(i)
    } else if let Some(i) = attrs.plain("theme").and_then(|i| i.trim().parse().ok()) {
        Spelling::Theme(i)
    } else {
        Spelling::Auto
    };
    Color { spelling, tint }
}

/// `<colors><indexedColors><rgbColor rgb="…"/>…` — a workbook's own palette, which replaces
/// the legacy one entry for entry. `None` when the element held no colour at all, so that an
/// empty override does not empty the palette.
pub fn read_indexed(reader: &mut Reader<'_>) -> crate::Result<Option<Vec<[u8; 3]>>> {
    let mut out = Vec::new();
    reader.children(|reader, name, _| {
        if !name.is("indexedColors") {
            return Ok(Handled::No);
        }
        reader.children(|_, name, attrs| {
            if !name.is("rgbColor") {
                return Ok(Handled::No);
            }
            // A slot that will not parse keeps its place, as black, so the indices after it
            // still mean what the file meant.
            out.push(attrs.plain("rgb").and_then(argb).unwrap_or([0, 0, 0]));
            Ok(Handled::Yes)
        })?;
        Ok(Handled::Yes)
    })?;
    Ok((!out.is_empty()).then_some(out))
}

/// `FF0000FF` as three bytes. ARGB is what §18.8.19 says; six digits is what some producers
/// write (`styles/colors.xlsx` B5), and means the same colour with the alpha implied. The
/// alpha is dropped either way — ODF's colour is six digits.
pub fn argb(value: &str) -> Option<[u8; 3]> {
    let value = value.trim();
    let rgb = match value.len() {
        8 => &value[2..],
        6 => value,
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(rgb.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

pub fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// ECMA-376's tint (§18.8.19, `tint`): the colour's **HSL luminance** moved toward black by
/// the fraction below zero, or toward white by the fraction above it. Hue and saturation are
/// untouched — which is why tinting each channel by the same fraction, the common shortcut, is
/// wrong for every colour that is not a grey.
///
/// Computed in floating point and rounded half up per channel. Against the oracle's own
/// conversion of 75 tinted theme colours (`doc/xlsx-format.md` §4.2) this agrees exactly on
/// 69 and is one step off in a channel or two on the other six: the oracle quantises somewhere
/// the specification does not say to, and loop D names that difference rather than this
/// module imitating it — the integer HLS routines tried agree on far fewer.
pub fn tint(rgb: [u8; 3], tint: f64) -> [u8; 3] {
    if tint == 0.0 {
        return rgb;
    }
    let [r, g, b] = rgb.map(|c| f64::from(c) / 255.0);
    let (h, s, l) = to_hsl(r, g, b);
    let l = if tint < 0.0 {
        l * (1.0 + tint)
    } else {
        l * (1.0 - tint) + tint
    };
    let (r, g, b) = from_hsl(h, s, l.clamp(0.0, 1.0));
    [r, g, b].map(|c| (c * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8)
}

/// Hue in sixths of a turn, saturation and lightness in 0–1.
fn to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = if l <= 0.5 {
        d / (max + min)
    } else {
        d / (2.0 - max - min)
    };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(hex: &str) -> [u8; 3] {
        argb(hex).unwrap()
    }

    #[test]
    fn argb_drops_the_alpha_and_six_digits_imply_it() {
        assert_eq!(argb("FF0000FF"), Some([0, 0, 0xFF]));
        assert_eq!(
            argb("800000FF"),
            Some([0, 0, 0xFF]),
            "half alpha, same colour"
        );
        assert_eq!(argb("00FF00"), Some([0, 0xFF, 0]));
        assert_eq!(argb("red"), None);
        assert_eq!(argb("FF00"), None);
    }

    /// Greys have no hue, so a grey's tint is exact arithmetic — the case the formula is
    /// easiest to check by hand: black lightened by 0.4 is 40% grey.
    #[test]
    fn a_tint_moves_luminance_toward_black_or_white() {
        assert_eq!(tint([0, 0, 0], 0.4), rgb("666666"));
        assert_eq!(tint([0, 0, 0], 0.8), rgb("CCCCCC"));
        assert_eq!(tint([0xFF, 0xFF, 0xFF], -0.25), rgb("BFBFBF"));
        assert_eq!(tint([0xFF, 0xFF, 0xFF], -0.5), rgb("808080"));
        assert_eq!(tint(rgb("4472C4"), 0.0), rgb("4472C4"));
    }

    /// Against the oracle's conversion, `doc/xlsx-format.md` §4.2: accent1 under every tint
    /// the fixture and the measurement used, and a sample of the other slots. Each of these
    /// agrees exactly; the six that do not are in that section, one step off in one channel.
    #[test]
    fn a_tinted_theme_colour_is_what_the_oracle_draws() {
        for (base, t, want) in [
            ("4472C4", -0.25, "2f5597"),
            ("4472C4", 0.4, "8faadc"),
            ("4472C4", 0.6, "b4c7e7"),
            ("4472C4", 0.8, "dae3f3"),
            ("4472C4", -0.5, "203864"),
            ("4472C4", 0.2, "698ed0"),
            ("4472C4", 0.9999, "ffffff"),
            ("4472C4", -0.9999, "000000"),
            ("ED7D31", -0.25, "c55a11"),
            ("FFC000", 0.4, "ffd966"),
            ("5B9BD5", -0.5, "1f4e79"),
            ("E7E6E6", 0.4, "f1f0f0"),
        ] {
            assert_eq!(hex(tint(rgb(base), t)), format!("#{want}"), "{base} {t}");
        }
    }

    #[test]
    fn index_64_and_65_are_the_system_not_a_colour() {
        let palette = Palette::default();
        let indexed = |i| Color {
            spelling: Spelling::Indexed(i),
            tint: 0.0,
        };
        assert_eq!(
            palette.resolve(&indexed(12)),
            Resolved::Hex("#0000ff".into())
        );
        assert_eq!(
            palette.resolve(&indexed(8)),
            Resolved::Hex("#000000".into())
        );
        assert_eq!(palette.resolve(&indexed(64)), Resolved::Automatic);
        assert_eq!(palette.resolve(&indexed(65)), Resolved::Automatic);
        assert_eq!(
            palette.resolve(&indexed(66)),
            Resolved::Unknown(Missing::Index)
        );
    }

    #[test]
    fn a_theme_colour_with_no_theme_is_unknown_rather_than_guessed() {
        let color = Color {
            spelling: Spelling::Theme(4),
            tint: 0.0,
        };
        assert_eq!(
            Palette::default().resolve(&color),
            Resolved::Unknown(Missing::Theme)
        );
    }
}
