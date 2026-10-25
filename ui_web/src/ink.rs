// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A document's own colours, made to read on whichever page the browser paints. The stylesheet
//! follows the reader's light or dark setting (`Canvas`/`CanvasText`), and a colour the document
//! chose is inline and outranks it — so on a dark page a navy word vanished and a silver-filled
//! heading row kept white text. The rule is every window's, over [`grind_core::color`]; nothing it
//! decides is written.
//!
//! - **No colour, no fill**: say nothing; the page's ink applies.
//! - **No colour, on a fill**: ODF's automatic colour — the page's ink where it reads on the fill,
//!   black or white where not.
//! - **A colour on the page, when it is dark**: lifted along its hue until it reads
//!   ([`grind_core::color::legible`]).
//! - **Anything else** is the document's decision about its own paper.

use grind_core::color::{self, Rgb};

/// The page's ground and ink as the browser paints them — Chromium's and Firefox's `Canvas` and
/// `CanvasText` in each scheme, near enough that a contrast computed against them holds.
const LIGHT: (Rgb, Rgb) = ((255, 255, 255), (0, 0, 0));
const DARK: (Rgb, Rgb) = ((18, 18, 18), (255, 255, 255));

/// The CSS `color` a run or a cell is drawn in, or `None` for the page's own.
pub fn color(own: Option<&str>, fill: Option<&str>, dark: bool) -> Option<String> {
    let (ground, ink) = match dark {
        true => DARK,
        false => LIGHT,
    };
    match (own, own.and_then(parse), fill.and_then(parse)) {
        // A colour this shell cannot parse is still the document's, and CSS may know it.
        (Some(own), None, _) => Some(own.to_owned()),
        (None, _, None) => None,
        (Some(own), Some(_), Some(_)) => Some(own.to_owned()),
        (Some(own), Some(_), None) if !dark => Some(own.to_owned()),
        (_, rgb, fill) => Some(hex(color::document_ink(rgb, fill, ground, ink, dark))),
    }
}

/// Whether the page is being painted dark — `prefers-color-scheme`, which is what `Canvas`
/// follows. `false` where there is no window to ask, which is every host test.
pub fn page_is_dark() -> bool {
    web_sys::window()
        .and_then(|window| window.match_media("(prefers-color-scheme: dark)").ok())
        .flatten()
        .is_some_and(|query| query.matches())
}

fn parse(value: &str) -> Option<Rgb> {
    match value.trim() {
        "transparent" | "none" => None,
        value => color::parse(value),
    }
}

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dark-mode screenshot, as a test: navy on the dark page is lifted until it reads, the
    /// same navy on white is left alone, and a silver heading with no colour of its own gets
    /// dark ink rather than the dark theme's white.
    #[test]
    fn a_documents_colour_reads_on_the_page_it_is_drawn_on() {
        let lifted = color(Some("#001f3f"), None, true).expect("drawn");
        let rgb = color::parse(&lifted).expect("a hex colour");
        assert!(color::contrast(rgb, DARK.0) >= color::TEXT, "{lifted}");
        assert_eq!(
            color(Some("#001f3f"), None, false).as_deref(),
            Some("#001f3f")
        );
        assert_eq!(
            color(None, Some("#dddddd"), true).as_deref(),
            Some("#000000")
        );
        assert_eq!(
            color(None, Some("#dddddd"), false).as_deref(),
            Some("#000000")
        );
        assert_eq!(
            color(None, Some("#001f3f"), false).as_deref(),
            Some("#ffffff")
        );
        assert_eq!(color(None, None, true), None);
        assert_eq!(color(None, Some("transparent"), true), None);
        assert_eq!(
            color(Some("#ff0000"), Some("#ffffff"), true).as_deref(),
            Some("#ff0000"),
            "on its own fill the document decided"
        );
    }
}
