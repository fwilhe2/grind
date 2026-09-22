// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A document's own colours, made to read on whichever page the browser is painting.
//!
//! The stylesheet follows the reader's light or dark setting through `Canvas`/`CanvasText`, and
//! a colour the *document* chose is written inline and outranks it — which is right on white
//! paper and was two bugs on a dark page, the same two `ui_sheet_gtk`'s `theme::ink` was written
//! to answer: a navy word vanished into the dark ground, and a heading row the document filled
//! silver kept the dark theme's white text across it.
//!
//! The rule is that one, over the same arithmetic ([`grind_core::color`]), so every window in the
//! suite draws a document's colours the same way. Nothing it decides is written.
//!
//! - **No colour of its own, no fill**: say nothing, and the page's own ink applies.
//! - **No colour of its own, on a fill of its own**: ODF's *automatic* colour — the page's ink
//!   where it reads on that fill, black or white where it does not.
//! - **A colour of its own on the page, when the page is dark**: lifted along its own hue until
//!   it reads ([`grind_core::color::legible`]).
//! - **Anything else** is the document's decision about its own paper, and is drawn as it is.

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
        (None, _, Some(fill)) => Some(hex(color::automatic_ink(fill, ink))),
        (Some(_), Some(rgb), None) if dark => Some(hex(color::legible(rgb, ground, color::TEXT))),
        (Some(own), Some(_), _) => Some(own.to_owned()),
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
