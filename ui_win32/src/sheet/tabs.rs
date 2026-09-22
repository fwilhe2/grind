// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sheet tabs, as arithmetic — where each one sits in the status bar, and which one a point
//! is on. **Portable**, and tested on Linux like the rest of `sheet/`.
//!
//! This window had no tabs. The only sign a document had a second sheet was *sheet 1 of 2* in
//! the status bar, and the only ways to reach it were Ctrl+PageDown and the Sheet menu — a
//! workbook whose second sheet a person never finds is a workbook with one sheet, as far as
//! they know. Every other window in the suite shows the sheets as tabs along the bottom, and so
//! does every spreadsheet anybody has used on this platform.
//!
//! So the status bar's left half is the tabs now, and the sentence they replace goes: the name
//! is on its tab, the count is the number of tabs, and the used extent was a fact about the file
//! nobody selecting cells was asking. A tab is a click away from being the sheet on screen; a
//! double-click renames it and a right-click is the tab's own menu, the same verbs the Sheet
//! menu already has (`menu.rs`), so the tab adds a way in and no new verb.
//!
//! The widths are the caller's — measured with the font the tabs are drawn in — so this file
//! never needs a DC, and what a click hits is exactly what was drawn: the window keeps the
//! layout the last paint produced and asks [`hit`] against that.

use super::geom::Rect;

/// Padding either side of a tab's name, the gap between two tabs, and the add button's width,
/// in pixels at 100%.
pub const PAD: f64 = 10.0;
pub const GAP: f64 = 2.0;
pub const ADD_W: f64 = 28.0;
/// The narrowest a tab is allowed to be, and the widest — a long name is elided, not a strip
/// that pushes the arithmetic off the bar.
pub const MIN_W: f64 = 48.0;
pub const MAX_W: f64 = 180.0;
/// What `draw_text` takes off each end beyond the padding, in device pixels.
const SLACK: f64 = 2.0;

/// What one piece of the strip is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Sheet(usize),
    /// The `+` after the last tab: Add Sheet.
    Add,
}

/// One tab, or the add button, where it was placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tab {
    pub target: Target,
    pub rect: Rect,
}

/// Lay the tabs out along `bar` from its left edge: one per name, each as wide as its name
/// (`widths`, already measured) plus padding within [`MIN_W`, `MAX_W`], then the add button.
/// Tabs that would not fit before `limit` are left out rather than squeezed — the Sheet menu and
/// Ctrl+PageDown still reach them, and the ones shown stay readable.
///
/// `scale` is the DPI factor (1.0 at 96 DPI).
pub fn layout(widths: &[f64], bar: Rect, limit: f64, scale: f64) -> Vec<Tab> {
    let inset = 3.0 * scale;
    let (top, height) = (bar.y + inset, (bar.h - 2.0 * inset).max(1.0));
    let mut x = bar.x + 4.0 * scale;
    let mut out = Vec::with_capacity(widths.len() + 1);
    let add_w = ADD_W * scale;
    for (index, width) in widths.iter().enumerate() {
        // Two pixels of slack beyond the padding: the painter's `draw_text` insets its rectangle
        // by the padding *and* one pixel at each end, and a name given exactly its measured width
        // comes back elided — the first frame drew `Budg…` for a tab with room for `Budget`.
        let w = (width + 2.0 * PAD * scale + 2.0 * SLACK).clamp(MIN_W * scale, MAX_W * scale);
        if x + w + add_w > limit {
            break;
        }
        out.push(Tab {
            target: Target::Sheet(index),
            rect: Rect {
                x,
                y: top,
                w,
                h: height,
            },
        });
        x += w + GAP * scale;
    }
    if x + add_w <= limit {
        out.push(Tab {
            target: Target::Add,
            rect: Rect {
                x,
                y: top,
                w: add_w,
                h: height,
            },
        });
    }
    out
}

/// The piece of the strip under a point, if any.
pub fn hit(tabs: &[Tab], x: f64, y: f64) -> Option<Target> {
    tabs.iter()
        .find(|tab| tab.rect.contains(x, y))
        .map(|tab| tab.target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: Rect = Rect {
        x: 0.0,
        y: 600.0,
        w: 1000.0,
        h: 28.0,
    };

    #[test]
    fn a_tab_per_sheet_then_the_add_button() {
        let tabs = layout(&[40.0, 45.0], BAR, 700.0, 1.0);
        let targets: Vec<Target> = tabs.iter().map(|t| t.target).collect();
        assert_eq!(targets, [Target::Sheet(0), Target::Sheet(1), Target::Add]);
        assert_eq!(
            tabs[0].rect.w, 64.0,
            "the name, its padding, and the painter's slack"
        );
        assert!(
            tabs[1].rect.x > tabs[0].rect.x + tabs[0].rect.w,
            "never touching"
        );
        assert!(
            tabs.iter()
                .all(|t| t.rect.y > BAR.y && t.rect.y + t.rect.h < BAR.y + BAR.h)
        );
    }

    /// A one-letter sheet is still a target a pointer can hit, and a paragraph for a name is
    /// elided rather than pushing everything else off the bar.
    #[test]
    fn a_tab_is_neither_a_sliver_nor_a_banner() {
        let tabs = layout(&[4.0, 900.0], BAR, 700.0, 1.0);
        assert_eq!(tabs[0].rect.w, MIN_W);
        assert_eq!(tabs[1].rect.w, MAX_W);
    }

    #[test]
    fn what_does_not_fit_is_left_out_not_squeezed() {
        let tabs = layout(&[60.0; 10], BAR, 300.0, 1.0);
        assert!(tabs.len() < 11);
        let last = tabs.last().expect("the add button at least");
        assert!(last.rect.x + last.rect.w <= 300.0);
        assert!(
            tabs.iter()
                .all(|t| t.rect.w >= MIN_W || t.target == Target::Add)
        );
    }

    #[test]
    fn a_click_finds_the_tab_it_landed_on() {
        let tabs = layout(&[40.0, 45.0], BAR, 700.0, 1.0);
        let middle = |t: &Tab| (t.rect.x + t.rect.w / 2.0, t.rect.y + t.rect.h / 2.0);
        let (x, y) = middle(&tabs[1]);
        assert_eq!(hit(&tabs, x, y), Some(Target::Sheet(1)));
        let (x, y) = middle(&tabs[2]);
        assert_eq!(hit(&tabs, x, y), Some(Target::Add));
        assert_eq!(hit(&tabs, 900.0, y), None);
        assert_eq!(hit(&tabs, x, 10.0), None, "the grid is not a tab");
    }

    #[test]
    fn it_scales_with_the_display() {
        let at_1x = layout(&[40.0], BAR, 700.0, 1.0);
        let at_2x = layout(&[80.0], BAR, 700.0, 2.0);
        // Everything but the painter's slack scales, which is a device pixel either way.
        assert_eq!(
            at_2x[0].rect.w - 2.0 * SLACK,
            (at_1x[0].rect.w - 2.0 * SLACK) * 2.0
        );
    }
}
