// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The welcome screen — what a window with **no document** shows.
//!
//! This shell used to open an empty spreadsheet when it was started with no file, and `--text`
//! was the only way to say otherwise. That is a guess the suite has no business making: it holds
//! two applications, and a window that picks one of them before being asked is a window that is
//! wrong half the time. So a window with nothing to show shows the *choice* instead — new
//! spreadsheet, new text document, or open something that already exists.
//!
//! **A third pane, not a dialog.** `dialog::choose` was the cheap answer and the wrong one: a
//! modal in front of an empty grid is still a window that has already decided, and cancelling it
//! leaves the guess behind. A pane is `win.rs`'s existing shape — one window, one `Pane`, decided
//! by what was opened — and it extends to "nothing was opened" with no new machinery at all.
//!
//! **Portable, and tested on any host**, like every other geometry file in this crate: what a
//! card is, where it sits, which one a point is in and which one a key moves to are arithmetic,
//! and only putting pixels down needs Windows. The Windows half is at the bottom, behind
//! `#[cfg(windows)]`, exactly as `sheet/draw.rs` and `text/draw.rs` arrange the same split.

use crate::menu::Command;
use crate::sheet::geom::{Rect, scale};

/// One thing the welcome screen offers.
///
/// Each is a [`Command`] rather than a private verb, which is what keeps this pane from being a
/// second way of doing anything: the card runs the menu item, and `File ▸ New Spreadsheet` runs
/// the card. A choice with no command could not be reached from the menu bar, and
/// `doc/windows-shell.md`'s decision 4 says every verb is reachable from there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    NewSheet,
    NewText,
    Open,
}

impl Choice {
    /// In the order they are offered: make something, then make something else, then open. The
    /// keyboard starts on the first, so this order is also the default answer.
    pub const ALL: &'static [Choice] = &[Choice::NewSheet, Choice::NewText, Choice::Open];

    pub fn command(self) -> Command {
        match self {
            Choice::NewSheet => Command::NewSheet,
            Choice::NewText => Command::NewText,
            Choice::Open => Command::Open,
        }
    }

    /// The card's own heading. Deliberately **not** [`crate::menu::label_for`]'s string: a menu
    /// item carries a `&` mnemonic and a `\t` accelerator, and neither belongs on a card.
    pub fn title(self) -> &'static str {
        match self {
            Choice::NewSheet => "New Spreadsheet",
            Choice::NewText => "New Text Document",
            Choice::Open => "Open a Document…",
        }
    }

    /// The line under it — what you get, in the words a person would use for it. The extensions
    /// are named because they are the answer to "which of these two do I want": somebody who has
    /// a `.fodt` to make knows it by its name long before they know it is a *text document*.
    pub fn detail(self) -> &'static str {
        match self {
            Choice::NewSheet => "An empty sheet of cells — .fods",
            Choice::NewText => "An empty page to write on — .fodt",
            Choice::Open => "A spreadsheet, a text document or a .grind projection",
        }
    }

    /// The accelerator, spelled for a reader. The menu bar carries the same three and
    /// [`crate::menu::accelerator`] answers them; this is the reminder on the card.
    pub fn keys(self) -> &'static str {
        match self {
            Choice::NewSheet => "Ctrl+N",
            Choice::NewText => "Ctrl+Shift+N",
            Choice::Open => "Ctrl+O",
        }
    }
}

/// The suite's name and what it is, over the cards. Not a version string: that is About's job,
/// and a welcome screen that leads with a build stamp is a welcome screen written by a compiler.
pub const TITLE: &str = "Grind";
pub const SUBTITLE: &str = "An ODF-native office suite";

/// The line under the cards. Both halves of it are true of this shell and of nothing else here:
/// a document can be dropped on the window in no shell at all, so it is *not* offered.
pub const HINT: &str = "A document opens in this window, whichever kind it is.";

/// One card's width and height at 100%, and the gap between two.
///
/// Wide enough for [`Choice::detail`]'s longest line at body size, which is what decides it:
/// a card whose second line elides has lost the sentence that tells the two new-document
/// choices apart.
pub const CARD_W: f64 = 420.0;
pub const CARD_H: f64 = 64.0;
pub const CARD_GAP: f64 = 8.0;

/// Space inside a card, from its edge to its text.
pub const CARD_PAD: f64 = 16.0;

/// The gap between the heading block and the first card, and between the last card and the hint.
pub const BLOCK_GAP: f64 = 32.0;

/// The heading block's own height — two lines, [`TITLE`] over [`SUBTITLE`].
pub const HEADING_H: f64 = 56.0;

/// The hint line's height.
pub const HINT_H: f64 = 20.0;

/// [`TITLE`]'s size at 100%. The one place in this shell that sets type larger than prose, and
/// the reason is that this is the only screen with nothing else on it to be larger than.
pub const TITLE_PX: f64 = 28.0;

/// Where everything on the welcome screen goes, for a client area this size.
///
/// A `struct` rather than free functions so the arithmetic is done once per frame and every
/// answer below agrees with every other — the same shape `text::geom::Page` has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page {
    pub width: f64,
    pub height: f64,
    pub dpi: u32,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            width: 0.0,
            height: 0.0,
            dpi: 96,
        }
    }
}

impl Page {
    /// The whole arrangement — heading, cards, hint — as one column, centred both ways.
    ///
    /// Centred *vertically* rather than pinned to the top, which is what makes this read as a
    /// welcome screen rather than as a document that happens to be empty. The column is clamped to
    /// the window, so a window shorter than the content starts at the top and lets the bottom
    /// fall off rather than centring it out of sight in both directions.
    fn column(&self) -> Rect {
        let card_h = scale(CARD_H, self.dpi);
        let gap = scale(CARD_GAP, self.dpi);
        let block = scale(BLOCK_GAP, self.dpi);
        let cards = Choice::ALL.len() as f64;
        let height = scale(HEADING_H, self.dpi)
            + block
            + cards * card_h
            + (cards - 1.0) * gap
            + block
            + scale(HINT_H, self.dpi);
        let width = scale(CARD_W, self.dpi).min(self.width - scale(CARD_PAD * 2.0, self.dpi));
        Rect {
            x: ((self.width - width) / 2.0).max(0.0),
            y: ((self.height - height) / 2.0).max(0.0),
            w: width.max(0.0),
            h: height,
        }
    }

    /// [`TITLE`] and [`SUBTITLE`]'s box.
    pub fn heading(&self) -> Rect {
        let column = self.column();
        Rect {
            h: scale(HEADING_H, self.dpi),
            ..column
        }
    }

    /// One rectangle per [`Choice`], in [`Choice::ALL`]'s order.
    pub fn cards(&self) -> Vec<Rect> {
        let column = self.column();
        let card_h = scale(CARD_H, self.dpi);
        let gap = scale(CARD_GAP, self.dpi);
        let top = column.y + scale(HEADING_H, self.dpi) + scale(BLOCK_GAP, self.dpi);
        (0..Choice::ALL.len())
            .map(|at| Rect {
                x: column.x,
                y: top + at as f64 * (card_h + gap),
                w: column.w,
                h: card_h,
            })
            .collect()
    }

    /// [`HINT`]'s box, under the last card.
    pub fn hint(&self) -> Rect {
        let cards = self.cards();
        let last = cards.last().copied().unwrap_or(self.column());
        Rect {
            x: last.x,
            y: last.y + last.h + scale(BLOCK_GAP, self.dpi),
            w: last.w,
            h: scale(HINT_H, self.dpi),
        }
    }

    /// Which card a point is in, if any — the index into [`Choice::ALL`].
    pub fn hit(&self, x: f64, y: f64) -> Option<usize> {
        self.cards().iter().position(|card| card.contains(x, y))
    }
}

/// The card the keyboard moves to from `current`, wrapping at both ends.
///
/// Wrapping rather than stopping, because there are three of them and the list is on screen
/// whole: there is no "further down" to be at the end of, so Down from the last card meaning
/// "back to the top" is shorter than an edge nobody can see the point of.
pub fn step(current: usize, delta: i32) -> usize {
    let count = Choice::ALL.len() as i32;
    let at = current as i32 + delta;
    (((at % count) + count) % count) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Page {
        Page {
            width: 1280.0,
            height: 800.0,
            dpi: 96,
        }
    }

    /// Every card the welcome screen offers has a menu item behind it, so nothing here is a verb
    /// this window can only do from one place. The complement — that the two new verbs are in a
    /// menu at all — is `menu.rs`'s own test.
    #[test]
    fn every_choice_names_a_command() {
        let commands: Vec<Command> = Choice::ALL.iter().map(|c| c.command()).collect();
        assert_eq!(
            commands,
            vec![Command::NewSheet, Command::NewText, Command::Open]
        );
        for choice in Choice::ALL {
            assert!(!choice.title().is_empty());
            assert!(!choice.detail().is_empty());
            assert!(crate::menu::label_for(choice.command()).is_some());
        }
    }

    /// The accelerator on a card is the one the menu bar carries, not a second claim about the
    /// keyboard — a card saying Ctrl+N over a menu saying something else would be this shell
    /// disagreeing with itself in two places on one screen.
    #[test]
    fn a_cards_keys_are_the_menus_own() {
        for choice in Choice::ALL {
            let label = crate::menu::label_for(choice.command()).expect("a menu item");
            let (_, keys) = label.split_once('\t').expect("an accelerator");
            assert_eq!(keys, choice.keys(), "{choice:?}");
        }
    }

    #[test]
    fn the_cards_are_stacked_in_order_and_do_not_overlap() {
        let cards = page().cards();
        assert_eq!(cards.len(), Choice::ALL.len());
        for pair in cards.windows(2) {
            assert!(
                pair[1].y >= pair[0].y + pair[0].h,
                "{:?} overlaps {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// The whole column is inside the window, which is the property a centred layout is easy to
    /// get wrong in one direction and never notice in the other.
    #[test]
    fn everything_is_inside_the_window() {
        let page = page();
        let mut boxes = vec![page.heading(), page.hint()];
        boxes.extend(page.cards());
        for rect in boxes {
            assert!(rect.x >= 0.0, "{rect:?}");
            assert!(rect.y >= 0.0, "{rect:?}");
            assert!(rect.x + rect.w <= page.width + 0.5, "{rect:?}");
            assert!(rect.y + rect.h <= page.height + 0.5, "{rect:?}");
        }
    }

    #[test]
    fn a_click_lands_on_the_card_it_is_in() {
        let page = page();
        for (at, card) in page.cards().iter().enumerate() {
            assert_eq!(page.hit(card.x + 1.0, card.y + 1.0), Some(at));
            assert_eq!(
                page.hit(card.x + card.w / 2.0, card.y + card.h / 2.0),
                Some(at)
            );
        }
        // The gaps between cards, and the heading above them, are not cards.
        assert_eq!(
            page.hit(page.heading().x + 1.0, page.heading().y + 1.0),
            None
        );
        assert_eq!(page.hit(-5.0, -5.0), None);
    }

    /// A window smaller than the content must still answer every question rather than producing a
    /// negative width or panicking — the first thing a user does to a welcome screen is make the
    /// window small.
    #[test]
    fn a_tiny_window_still_answers() {
        let page = Page {
            width: 120.0,
            height: 60.0,
            dpi: 96,
        };
        for rect in page.cards() {
            assert!(rect.w >= 0.0 && rect.h >= 0.0, "{rect:?}");
        }
        assert!(page.heading().w >= 0.0);
        assert!(page.hint().w >= 0.0);
        assert_eq!(page.hit(1.0, 1.0), None);
    }

    /// Scaling moves everything together: at 200% the cards are twice as far down and twice as
    /// tall, which is the property a DPI bug breaks in exactly one of the two.
    #[test]
    fn the_layout_scales_with_the_monitor() {
        let hundred = page();
        let two_hundred = Page {
            dpi: 192,
            ..hundred
        };
        let one = hundred.cards()[0];
        let two = two_hundred.cards()[0];
        assert!((two.h - one.h * 2.0).abs() < 0.5, "{one:?} {two:?}");
        assert!(two.w > one.w);
    }

    #[test]
    fn the_keyboard_wraps_at_both_ends() {
        assert_eq!(step(0, 1), 1);
        assert_eq!(step(2, 1), 0);
        assert_eq!(step(0, -1), 2);
        assert_eq!(step(1, -1), 0);
    }
}

#[cfg(windows)]
pub use windows_impl::{Frame, paint};

#[cfg(windows)]
mod windows_impl {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{HDC, SetBkMode, TRANSPARENT};

    use super::{CARD_PAD, Choice, HINT, Page, SUBTITLE, TITLE, TITLE_PX, scale};
    use crate::gdi::{self, Font, Selected};
    use crate::sheet::draw::{Align, draw_text};
    use crate::theme::{self, Interaction, Theme, control_fill};

    /// Everything one frame of the welcome screen needs, and nothing about the window.
    ///
    /// The same shape the other two panes' frames have, and for the same reason: [`paint`] takes
    /// an `HDC` and this, so `--render-to` is a second *caller* rather than a second drawing path
    /// (`doc/windows-shell.md`, decision 5).
    pub struct Frame<'a> {
        pub theme: Theme,
        pub face: &'a str,
        /// Which card the pointer is over, and which one is held down — the press acts on
        /// release, so it can be taken back by moving off the card, exactly as the text pane's
        /// format strip behaves.
        pub hover: Option<usize>,
        pub pressed: Option<usize>,
        /// Which card the keyboard is on. Always one: this pane has nothing else to focus, so
        /// there is no "nothing selected" state for Enter to mean nothing in.
        pub focus: usize,
    }

    /// The welcome screen, painted.
    pub fn paint(dc: HDC, page: &Page, frame: &Frame) {
        let theme = frame.theme;
        // Every label here stands on a ground somebody else painted — the window's backdrop, or a
        // card — so GDI must not fill a box behind it. Its default is `OPAQUE`, and a rendered
        // frame is what said so: the subtitle and the hint came back sitting in pale rectangles
        // the size of their own text, which is the same class of bug `doc/windows-shell.md` keeps
        // a list of because it is invisible in review.
        // SAFETY: the DC is live for the length of this call.
        unsafe {
            SetBkMode(dc, TRANSPARENT);
        }
        // The window's own ground, not the document's: there is no document. The cards are the
        // only surface here, which is what makes them read as the things to press.
        gdi::fill(
            dc,
            0,
            0,
            page.width.round() as i32,
            page.height.round() as i32,
            theme.backdrop,
        );

        let dpi = page.dpi;
        let heading = page.heading();
        let title_px = scale(TITLE_PX, dpi).round() as i32;
        let body_px = scale(theme::text::BODY, dpi).round() as i32;
        let caption_px = scale(theme::text::CAPTION, dpi).round() as i32;

        {
            let font = Font::new(frame.face, title_px, true);
            let _font = Selected::font(dc, &font);
            let (l, t, r, _) = heading.edges();
            draw_text(
                dc,
                TITLE,
                l,
                t,
                r,
                t + title_px + scale(8.0, dpi).round() as i32,
                Align::Center,
                theme.text,
                0.0,
            );
        }
        {
            let font = Font::new(frame.face, body_px, false);
            let _font = Selected::font(dc, &font);
            let (l, _, r, b) = heading.edges();
            draw_text(
                dc,
                SUBTITLE,
                l,
                b - body_px - scale(6.0, dpi).round() as i32,
                r,
                b,
                Align::Center,
                theme.text_secondary,
                0.0,
            );
        }

        let radius = scale(theme::space::RADIUS_SURFACE, dpi).round() as i32;
        let pad = scale(CARD_PAD, dpi);
        for (at, card) in page.cards().iter().enumerate() {
            let choice = Choice::ALL[at];
            let state = match (frame.pressed == Some(at), frame.hover == Some(at)) {
                (true, _) => Interaction::Pressed,
                (false, true) => Interaction::Hover,
                (false, false) => Interaction::Rest,
            };
            // A card is a *filled* control — a field you press, not a toggle in a row of them —
            // so it has a ground at rest. The focused one is outlined in the accent instead of
            // filled with it: a keyboard focus that changed the fill would be indistinguishable
            // from a hover, and the two have to be told apart on a screen whose whole purpose is
            // being answered by either.
            let fill = control_fill(theme, state, false, true).unwrap_or(theme.card);
            let focused = frame.focus == at;
            let border = match focused {
                true => theme.accent,
                false => theme.stroke,
            };
            let (l, t, r, b) = card.edges();
            gdi::round_rect(
                dc,
                RECT {
                    left: l,
                    top: t,
                    right: r,
                    bottom: b,
                },
                radius,
                fill,
                border,
            );
            // A second rectangle one pixel in, so the focused card's outline is visibly thicker
            // than a resting one's rather than a shade of grey somebody has to compare against
            // its neighbours.
            if focused {
                gdi::round_rect(
                    dc,
                    RECT {
                        left: l + 1,
                        top: t + 1,
                        right: r - 1,
                        bottom: b - 1,
                    },
                    radius,
                    fill,
                    theme.accent,
                );
            }

            let mid = (t + b) / 2;
            {
                let font = Font::new(frame.face, body_px, true);
                let _font = Selected::font(dc, &font);
                draw_text(
                    dc,
                    choice.title(),
                    l,
                    t,
                    r,
                    mid,
                    Align::Left,
                    theme.text,
                    pad,
                );
                // The accelerator, at the other end of the same line — the card says which key
                // does this without a second row of chrome to hold the answer.
                let font = Font::new(frame.face, caption_px, false);
                let _font = Selected::font(dc, &font);
                draw_text(
                    dc,
                    choice.keys(),
                    l,
                    t,
                    r,
                    mid,
                    Align::Right,
                    theme.text_tertiary,
                    pad,
                );
            }
            {
                let font = Font::new(frame.face, caption_px, false);
                let _font = Selected::font(dc, &font);
                draw_text(
                    dc,
                    choice.detail(),
                    l,
                    mid,
                    r,
                    b,
                    Align::Left,
                    theme.text_secondary,
                    pad,
                );
            }
        }

        let hint = page.hint();
        let font = Font::new(frame.face, caption_px, false);
        let _font = Selected::font(dc, &font);
        let (l, t, r, b) = hint.edges();
        draw_text(
            dc,
            HINT,
            l,
            t,
            r,
            b,
            Align::Center,
            theme.text_tertiary,
            0.0,
        );
    }
}
