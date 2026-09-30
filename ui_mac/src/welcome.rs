// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The welcome window (decision 6, M9) — what a launch with nothing to open shows, and what a
//! Dock click with no window open brings back. Portable: what a choice is, where everything
//! sits, which thing a point is on, where a key moves, and a frame as [`Op`]s, so `--render-to`
//! draws it with no document named, in both appearances.
//!
//! It offers the *choice* rather than guessing — one application holds two kinds of document,
//! and a window that picks one before being asked is wrong half the time (`ui_win32`'s W11) —
//! and every card runs a menu item's action, so the window is never a second way of doing
//! anything. **The recent-documents list W11 declined to build comes free** here: it is
//! `NSDocumentController`'s, the system's list rather than a file of ours, handed to this file as
//! names and shown under the cards.

use grind_core::color::Rgb;
use grind_core::style::TextStyle;

use crate::menu::{Action, Command};
use crate::ops::Op;
use crate::sheet::geom::Rect;

/// One thing the window offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    NewSheet,
    NewText,
    Open,
}

impl Choice {
    /// Make something, make something else, open — the keyboard starts on the first.
    pub const ALL: [Choice; 3] = [Choice::NewSheet, Choice::NewText, Choice::Open];

    /// What choosing it does: the menu item's own action.
    pub fn action(self) -> Action {
        match self {
            Choice::NewSheet => Action::Command(Command::NewSheet),
            Choice::NewText => Action::Command(Command::NewText),
            Choice::Open => Action::Standard("openDocument:"),
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Choice::NewSheet => "New Spreadsheet",
            Choice::NewText => "New Text Document",
            Choice::Open => "Open a Document\u{2026}",
        }
    }

    /// What you get, in the words a person would use — and the extension, which is how somebody
    /// with a `.fodt` to make knows which of the two they want.
    pub fn detail(self) -> &'static str {
        match self {
            Choice::NewSheet => "An empty sheet of cells \u{2014} .fods",
            Choice::NewText => "An empty page to write on \u{2014} .fodt",
            Choice::Open => "A spreadsheet, a text document or a .grind projection",
        }
    }

    /// The menu item's key, as the menu draws it.
    pub fn keys(self) -> &'static str {
        match self {
            Choice::NewSheet => "\u{2318}N",
            Choice::NewText => "\u{2325}\u{2318}N",
            Choice::Open => "\u{2318}O",
        }
    }
}

pub const TITLE: &str = "Grind";
pub const SUBTITLE: &str = "An ODF-native office suite";

/// The window's size, in points.
pub const WIDTH: f64 = 520.0;
pub const HEIGHT: f64 = 500.0;

const MARGIN: f64 = 40.0;
const HEADING_H: f64 = 64.0;
const CARD_H: f64 = 60.0;
const CARD_GAP: f64 = 8.0;
const CARD_PAD: f64 = 14.0;
const RECENT_GAP: f64 = 24.0;
const RECENT_H: f64 = 22.0;
/// How many recent documents are shown — the system keeps more; Open Recent has them all.
pub const RECENT_MAX: usize = 5;
const OUTLINE: f64 = 2.0;

/// What a point or a key is on: a card, or a recent document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Choice(usize),
    Recent(usize),
}

/// The window's content: which recent documents there are, by name, and which target has the
/// keyboard.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Welcome {
    pub recent: Vec<String>,
    pub focus: usize,
}

impl Welcome {
    /// Everything a key can land on, in order: the three cards, then each recent document.
    pub fn targets(&self) -> Vec<Target> {
        (0..Choice::ALL.len())
            .map(Target::Choice)
            .chain((0..self.recent.len().min(RECENT_MAX)).map(Target::Recent))
            .collect()
    }

    pub fn focused(&self) -> Target {
        self.targets()
            .get(self.focus)
            .copied()
            .unwrap_or(Target::Choice(0))
    }

    /// ↑ and ↓, Tab and ⇧Tab: the next target or the one before, wrapping.
    pub fn step(&mut self, by: i32) {
        let count = self.targets().len() as i32;
        self.focus = (self.focus as i32 + by).rem_euclid(count.max(1)) as usize;
    }

    pub fn heading(&self) -> Rect {
        Rect::new(MARGIN, MARGIN, WIDTH - 2.0 * MARGIN, HEADING_H)
    }

    /// Where each card is.
    pub fn cards(&self) -> Vec<Rect> {
        let top = MARGIN + HEADING_H + CARD_GAP;
        (0..Choice::ALL.len())
            .map(|at| {
                Rect::new(
                    MARGIN,
                    top + at as f64 * (CARD_H + CARD_GAP),
                    WIDTH - 2.0 * MARGIN,
                    CARD_H,
                )
            })
            .collect()
    }

    /// Where the Recent heading is, when there are recent documents.
    pub fn recent_heading(&self) -> Option<Rect> {
        let last = *self.cards().last()?;
        (!self.recent.is_empty()).then(|| {
            Rect::new(
                MARGIN,
                last.bottom() + RECENT_GAP,
                WIDTH - 2.0 * MARGIN,
                RECENT_H,
            )
        })
    }

    /// Where each recent document's row is.
    pub fn recents(&self) -> Vec<Rect> {
        let Some(heading) = self.recent_heading() else {
            return Vec::new();
        };
        (0..self.recent.len().min(RECENT_MAX))
            .map(|at| {
                Rect::new(
                    MARGIN,
                    heading.bottom() + at as f64 * RECENT_H,
                    WIDTH - 2.0 * MARGIN,
                    RECENT_H,
                )
            })
            .collect()
    }

    /// Which target a point is on — `None` between them, where a click does nothing.
    pub fn hit(&self, x: f64, y: f64) -> Option<Target> {
        let inside =
            |rect: &Rect| x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom();
        if let Some(at) = self.cards().iter().position(inside) {
            return Some(Target::Choice(at));
        }
        self.recents().iter().position(inside).map(Target::Recent)
    }

    /// Move the keyboard to `target`.
    pub fn focus_on(&mut self, target: Target) {
        if let Some(at) = self.targets().iter().position(|t| *t == target) {
            self.focus = at;
        }
    }
}

/// The colours the window is drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The window's ground.
    pub ground: Rgb,
    /// A card, a step off the ground.
    pub card: Rgb,
    pub ink: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        ground: (0xf5, 0xf5, 0xf7),
        card: (0xff, 0xff, 0xff),
        ink: (0x1d, 0x1d, 0x1f),
        muted: (0x6e, 0x6e, 0x73),
        accent: (0x00, 0x7a, 0xff),
    };
    pub const DARK: Palette = Palette {
        ground: (0x1e, 0x1e, 0x1e),
        card: (0x2c, 0x2c, 0x2e),
        ink: (0xf5, 0xf5, 0xf7),
        muted: (0x98, 0x98, 0x9d),
        accent: (0x0a, 0x84, 0xff),
    };
}

fn size(points: u32) -> TextStyle {
    TextStyle {
        font_size: Some(format!("{points}pt")),
        ..TextStyle::default()
    }
}

fn bold(points: u32) -> TextStyle {
    TextStyle {
        font_weight: Some("bold".to_owned()),
        ..size(points)
    }
}

fn text(x: f64, top: f64, text: &str, style: TextStyle, color: Rgb, clip: Rect) -> Op {
    Op::Text {
        x,
        top,
        text: text.to_owned(),
        style,
        color,
        clip,
    }
}

/// A frame of the window: the ground, the heading, the cards — the focused one outlined in the
/// accent — and the recent documents.
pub fn frame(welcome: &Welcome, palette: &Palette) -> Vec<Op> {
    let whole = Rect::new(0.0, 0.0, WIDTH, HEIGHT);
    let mut ops = vec![Op::Fill {
        rect: whole,
        color: palette.ground,
    }];
    let heading = welcome.heading();
    ops.push(text(
        heading.x,
        heading.y,
        TITLE,
        bold(26),
        palette.ink,
        heading,
    ));
    ops.push(text(
        heading.x,
        heading.y + 36.0,
        SUBTITLE,
        size(13),
        palette.muted,
        heading,
    ));
    let focused = welcome.focused();
    for (at, (rect, choice)) in welcome.cards().iter().zip(Choice::ALL).enumerate() {
        if focused == Target::Choice(at) {
            ops.push(Op::Fill {
                rect: Rect::new(
                    rect.x - OUTLINE,
                    rect.y - OUTLINE,
                    rect.w + 2.0 * OUTLINE,
                    rect.h + 2.0 * OUTLINE,
                ),
                color: palette.accent,
            });
        }
        ops.push(Op::Fill {
            rect: *rect,
            color: palette.card,
        });
        let inner = Rect::new(rect.x + CARD_PAD, rect.y, rect.w - 2.0 * CARD_PAD, rect.h);
        ops.push(text(
            inner.x,
            rect.y + 10.0,
            choice.title(),
            bold(14),
            palette.ink,
            inner,
        ));
        ops.push(text(
            inner.x,
            rect.y + 32.0,
            choice.detail(),
            size(12),
            palette.muted,
            inner,
        ));
        // The key, at the card's far end — the reminder the menu also carries.
        ops.push(text(
            inner.right() - 48.0,
            rect.y + 10.0,
            choice.keys(),
            size(12),
            palette.muted,
            inner,
        ));
    }
    if let Some(heading) = welcome.recent_heading() {
        ops.push(text(
            heading.x,
            heading.y,
            "Recent",
            bold(12),
            palette.muted,
            heading,
        ));
        for (at, (rect, name)) in welcome.recents().iter().zip(&welcome.recent).enumerate() {
            let color = match focused == Target::Recent(at) {
                true => palette.accent,
                false => palette.ink,
            };
            ops.push(text(rect.x, rect.y + 2.0, name, size(13), color, *rect));
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{Item, MENUS, every_item};

    /// Every card runs a menu item: nothing here is only here.
    #[test]
    fn every_choice_is_a_menu_item() {
        let actions: Vec<Action> = every_item(MENUS)
            .into_iter()
            .filter_map(|item| match item {
                Item::Entry { action, .. } => Some(*action),
                _ => None,
            })
            .collect();
        for choice in Choice::ALL {
            assert!(actions.contains(&choice.action()), "{choice:?}");
        }
    }

    #[test]
    fn the_keys_go_round_the_cards_and_the_recent_documents() {
        let mut welcome = Welcome {
            recent: vec!["budget.fods".into(), "notes.fodt".into()],
            focus: 0,
        };
        assert_eq!(welcome.targets().len(), 5);
        welcome.step(-1);
        assert_eq!(welcome.focused(), Target::Recent(1), "wraps backwards");
        welcome.step(1);
        assert_eq!(welcome.focused(), Target::Choice(0));
        welcome.focus_on(Target::Choice(2));
        assert_eq!(welcome.focus, 2);
    }

    #[test]
    fn a_point_is_on_a_card_a_recent_document_or_nothing() {
        let welcome = Welcome {
            recent: vec!["budget.fods".into()],
            focus: 0,
        };
        let card = welcome.cards()[1];
        assert_eq!(
            welcome.hit(card.x + 5.0, card.y + 5.0),
            Some(Target::Choice(1))
        );
        let row = welcome.recents()[0];
        assert_eq!(
            welcome.hit(row.x + 5.0, row.y + 5.0),
            Some(Target::Recent(0))
        );
        assert_eq!(welcome.hit(1.0, 1.0), None);
    }

    /// Everything fits in the window, recent documents included.
    #[test]
    fn it_fits_with_every_recent_document() {
        let welcome = Welcome {
            recent: (0..9).map(|n| format!("doc{n}.fods")).collect(),
            focus: 0,
        };
        let last = welcome.recents().last().copied().unwrap();
        assert_eq!(welcome.recents().len(), RECENT_MAX);
        assert!(last.bottom() <= HEIGHT - MARGIN / 2.0, "{last:?}");
    }

    #[test]
    fn the_focused_card_is_outlined_in_the_accent_and_only_it() {
        let welcome = Welcome::default();
        for palette in [Palette::LIGHT, Palette::DARK] {
            let accents = frame(&welcome, &palette)
                .iter()
                .filter(|op| matches!(op, Op::Fill { color, .. } if *color == palette.accent))
                .count();
            assert_eq!(accents, 1);
        }
        assert_ne!(
            frame(&welcome, &Palette::LIGHT),
            frame(&welcome, &Palette::DARK)
        );
    }
}
