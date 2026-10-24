// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A document's own colours, made to read in whichever terminal draws them. Unlike a window, a
//! terminal does not say what ground it paints on — the reader's theme, light or dark, and asking
//! takes an escape sequence not every terminal or multiplexer answers — so every rule here holds
//! **on both**:
//!
//! - **A colour of the document's own, on no fill**, becomes the named colour of its **hue
//!   family** that reads on light *and* dark ([`READABLE`]): navy is LightBlue, not black, which
//!   vanished on every dark theme.
//! - **A grey, black or white of its own** is the terminal's own ink: it has no hue to keep, and
//!   black on dark is exactly what cannot be seen.
//! - **A fill** is the nearest named colour: the document's paper, whatever the terminal's ground.
//! - **No colour of its own, on a fill**, is ODF's automatic colour — black or white, whichever
//!   reads on the fill ([`grind_core::color::automatic_ink`]), so a silver heading row stays
//!   readable on a dark theme.
//! - **A colour of its own on a fill of its own** is the document's decision: its hue family's
//!   member, or the nearest named colour for a grey, black or white.
//!
//! Sixteen named colours, never 256 or true colour: a named colour is the reader's own theme's,
//! and an RGB escape is not universal. Nothing decided here is written.

use grind_core::color::{self, Rgb};
use ratatui::style::Color;

/// The sixteen named colours at **xterm's** defaults — what a fill is matched against. No table is
/// exact, since the theme decides; xterm's is what most themes stay near, and it matches a silver
/// fill to ANSI grey rather than bright white, which on a light theme would erase it.
const NAMED: [(Color, Rgb); 16] = [
    (Color::Black, (0, 0, 0)),
    (Color::Red, (205, 0, 0)),
    (Color::Green, (0, 205, 0)),
    (Color::Yellow, (205, 205, 0)),
    (Color::Blue, (0, 0, 238)),
    (Color::Magenta, (205, 0, 205)),
    (Color::Cyan, (0, 205, 205)),
    (Color::Gray, (229, 229, 229)),
    (Color::DarkGray, (127, 127, 127)),
    (Color::LightRed, (255, 0, 0)),
    (Color::LightGreen, (0, 255, 0)),
    (Color::LightYellow, (255, 255, 0)),
    (Color::LightBlue, (92, 92, 255)),
    (Color::LightMagenta, (255, 0, 255)),
    (Color::LightCyan, (0, 255, 255)),
    (Color::White, (255, 255, 255)),
];

/// The named colours a document's **text** colour may become, by hue family — each measured to
/// clear a contrast of 2.5 against both white and `#1e1e1e` at the classic VGA values; the nine
/// left out each fail one side. `(hue from, hue to, dark member, light member)`.
const READABLE: [(f64, f64, Color, Color); 6] = [
    (345.0, 15.0, Color::LightRed, Color::LightRed),
    (15.0, 75.0, Color::Yellow, Color::Yellow),
    (75.0, 165.0, Color::Green, Color::Green),
    (165.0, 200.0, Color::Cyan, Color::Cyan),
    (200.0, 265.0, Color::LightBlue, Color::LightBlue),
    (265.0, 345.0, Color::Magenta, Color::LightMagenta),
];

/// How far from grey a colour has to be to have a hue worth keeping — below this it is a grey, and
/// the terminal's own ink is the honest drawing of it.
const GREY: f64 = 0.15;

/// A colour a hair off black or white is still black or white, whatever its hue says.
const EXTREME: f64 = 0.06;

/// The foreground and background for a run or a cell: its own colour `own` and fill `fill`, ODF
/// `#rrggbb` (or `transparent`), as named terminal colours by the rules above. `None` is the
/// terminal's own ink or ground, as is anything that does not parse.
pub fn colours(own: Option<&str>, fill: Option<&str>) -> (Option<Color>, Option<Color>) {
    let own = own.and_then(parse);
    let fill = fill.and_then(parse);
    match (own, fill) {
        (None, None) => (None, None),
        (Some(own), None) => (text(own), None),
        (None, Some(fill)) => (
            Some(nearest(color::automatic_ink(fill, (0, 0, 0)))),
            Some(nearest(fill)),
        ),
        // Its hue family where it has one, since raw distance at xterm's values sends a blue to
        // cyan; a grey, black or white is simply the nearest.
        (Some(own), Some(fill)) => (Some(text(own).unwrap_or(nearest(own))), Some(nearest(fill))),
    }
}

/// A document's text colour on the terminal's own ground — its hue family's readable member, or
/// the terminal's ink for a grey.
fn text(rgb: Rgb) -> Option<Color> {
    let (hue, saturation, lightness) = color::hsl(rgb);
    if saturation < GREY || !(EXTREME..=1.0 - EXTREME).contains(&lightness) {
        return None;
    }
    READABLE.iter().find_map(|&(from, to, dark, light)| {
        let within = match from < to {
            true => (from..to).contains(&hue),
            // The one family that wraps round zero.
            false => hue >= from || hue < to,
        };
        within.then_some(match lightness < 0.5 {
            true => dark,
            false => light,
        })
    })
}

/// The named colour nearest `rgb`, by squared distance at [`NAMED`]'s values.
fn nearest((r, g, b): Rgb) -> Color {
    let distance = |(nr, ng, nb): Rgb| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(r, nr) + d(g, ng) + d(b, nb)
    };
    NAMED
        .iter()
        .min_by_key(|(_, rgb)| distance(*rgb))
        .map_or(Color::Reset, |(named, _)| *named)
}

fn parse(value: &str) -> Option<Rgb> {
    match value.trim() {
        "transparent" | "none" => None,
        value => color::parse(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Navy text becomes a blue that reads on both grounds; a silver heading row with no colour of
    /// its own gets black ink on a light-grey fill.
    #[test]
    fn a_documents_colour_reads_in_a_light_terminal_and_a_dark_one() {
        assert_eq!(
            colours(Some("#001f3f"), None),
            (Some(Color::LightBlue), None)
        );
        assert_eq!(
            colours(None, Some("#dddddd")),
            (Some(Color::Black), Some(Color::Gray))
        );
        assert_eq!(
            colours(None, Some("#001f3f")),
            (Some(Color::White), Some(Color::Black)),
            "a dark fill gets light ink"
        );
    }

    /// Every text colour this shell can produce is one of the readable seven — measured here
    /// against both grounds at the VGA values a terminal is least kind to, so a family added to
    /// [`READABLE`] with a member that fails one side fails this test.
    #[test]
    fn every_text_colour_reads_on_both_grounds() {
        const VGA: [(Color, Rgb); 7] = [
            (Color::LightRed, (255, 85, 85)),
            (Color::Yellow, (170, 85, 0)),
            (Color::Green, (0, 170, 0)),
            (Color::Cyan, (0, 170, 170)),
            (Color::LightBlue, (85, 85, 255)),
            (Color::Magenta, (170, 0, 170)),
            (Color::LightMagenta, (255, 85, 255)),
        ];
        for (_, _, dark, light) in READABLE {
            for member in [dark, light] {
                let (_, rgb) = VGA
                    .iter()
                    .find(|(named, _)| *named == member)
                    .unwrap_or_else(|| panic!("{member:?} has no VGA value here"));
                for ground in [(255, 255, 255), (30, 30, 30)] {
                    assert!(
                        color::contrast(*rgb, ground) >= 2.5,
                        "{member:?} on {ground:?}"
                    );
                }
            }
        }
        // And every hue lands in exactly one family.
        for hue in (0..360).step_by(5) {
            let rgb = color::from_hsl(f64::from(hue), 0.8, 0.4);
            assert!(text(rgb).is_some(), "hue {hue}");
        }
    }

    #[test]
    fn the_palette_keeps_its_hues() {
        let text_of = |hex| colours(Some(hex), None).0;
        // `grind_core::style::PALETTE`, the colours every shell offers.
        assert_eq!(text_of("#0074d9"), Some(Color::LightBlue), "blue");
        assert_eq!(text_of("#39cccc"), Some(Color::Cyan), "teal");
        assert_eq!(text_of("#2ecc40"), Some(Color::Green), "green");
        assert_eq!(text_of("#ff4136"), Some(Color::LightRed), "red");
        assert_eq!(text_of("#ff851b"), Some(Color::Yellow), "orange");
        assert_eq!(text_of("#b10dc9"), Some(Color::Magenta), "purple");
        assert_eq!(
            text_of("#f012be"),
            Some(Color::LightMagenta),
            "fuchsia, the light one"
        );
    }

    /// A grey has no hue to keep, and black or white of its own is exactly the colour one of the
    /// two grounds erases: all of them are the terminal's own ink.
    #[test]
    fn a_grey_is_the_terminals_own_ink() {
        for grey in ["#000000", "#ffffff", "#aaaaaa", "#111111", "#dddddd"] {
            assert_eq!(colours(Some(grey), None), (None, None), "{grey}");
        }
    }

    /// On its own fill the document decided both, and both are drawn as it said.
    #[test]
    fn a_colour_on_its_own_fill_is_the_documents_decision() {
        assert_eq!(
            colours(Some("#ffffff"), Some("#001f3f")),
            (Some(Color::White), Some(Color::Black))
        );
        assert_eq!(
            colours(Some("#0074d9"), Some("#ffdc00")),
            (Some(Color::LightBlue), Some(Color::LightYellow))
        );
    }

    #[test]
    fn nothing_parseable_is_the_terminals_own() {
        assert_eq!(colours(None, None), (None, None));
        assert_eq!(colours(None, Some("transparent")), (None, None));
        assert_eq!(colours(Some("not a colour"), None), (None, None));
    }
}
