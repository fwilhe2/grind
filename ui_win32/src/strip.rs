// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A **format strip** — a row of controls on a band, in groups — laid out, and drawn.
//!
//! Both panes have one now: the text pane's since W5b (`CharStyle`) and the grid's since the
//! format-strip milestone (`CellStyle` + `numfmt::Format`). They are one kind of surface under
//! `doc/windows-shell.md` decision 4 and they must look like one, so the two things a strip is made
//! of live here once: **where** its controls go ([`lay_out`], portable and tested on any host) and
//! **how** each shape of control is painted (the `cfg(windows)` half). What each control *means*
//! stays with its pane — `text/geom.rs`'s `StripHit` and `sheet/format.rs`'s `Control`.
//!
//! The layout is W10's, written down once rather than twice: a control is Fluent's 32-pixel height
//! standing centred on its band, the first starts one group's margin in (the name box's own), the
//! controls of a group sit half a gap apart, and a new group starts a whole group's margin on with
//! a separator in the middle of that gap — Fluent's `AppBarSeparator`, which says "a different kind
//! of thing follows" without a border round anything.

use crate::sheet::geom::{Rect, scale};
use crate::theme::space;

/// Where one control of a strip goes, and the separator before it when it starts a new group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub rect: Rect,
    pub separator: Option<Rect>,
}

/// Lay out a strip of controls on `band` at this monitor's `dpi`.
///
/// `controls` is each control's width **at 100%** and whether a new group starts before it; the
/// first control's flag is ignored, since nothing before it needs separating. The widths are scaled
/// here, so a caller never holds a pixel measured at one DPI and drawn at another.
pub fn lay_out(band: Rect, dpi: u32, controls: &[(f64, bool)]) -> Vec<Placed> {
    let h = scale(space::CONTROL_H, dpi).min(band.h);
    let y = band.y + ((band.h - h) / 2.0).max(0.0);
    let tight = scale(space::GAP / 2.0, dpi);
    let group = scale(space::GROUP, dpi);
    let mut x = band.x + group;
    let mut out = Vec::with_capacity(controls.len());
    for (index, &(width, starts_group)) in controls.iter().enumerate() {
        let separator = match (index, starts_group) {
            (0, _) | (_, false) => {
                if index > 0 {
                    x += tight;
                }
                None
            }
            (_, true) => {
                let rule_h = h / 2.0;
                let rule = Rect {
                    x: (x + group / 2.0).round(),
                    y: y + (h - rule_h) / 2.0,
                    w: 1.0,
                    h: rule_h,
                };
                x += group;
                Some(rule)
            }
        };
        let w = scale(width, dpi);
        out.push(Placed {
            rect: Rect { x, y, w, h },
            separator,
        });
        x += w;
    }
    out
}

/// Which control, by index into what [`lay_out`] was given, a point at `x, y` is on.
pub fn hit(placed: &[Placed], x: f64, y: f64) -> Option<usize> {
    placed.iter().position(|p| p.rect.contains(x, y))
}

/// The rows a colour picker offers, in order: *Automatic* — no colour of its own, so the theme
/// decides — and then `grind_core::style::PALETTE` by name, **capitalised** as a list row is.
/// Row `n` past the first is `PALETTE[n - 1]`, which is what both panes' pickers write.
///
/// One list for both strips, so the text pane's *Text Colour* and the grid's are the same list;
/// the palette's own names are the lower-case words `grind sheet style --color navy` takes.
pub fn colour_choices() -> Vec<String> {
    std::iter::once("Automatic".to_owned())
        .chain(grind_core::style::PALETTE.iter().map(|(name, _)| {
            let mut chars = name.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }))
        .collect()
}

/// The row [`colour_choices`] opens on for a cell or run whose colour is `current` — its palette
/// entry, or *Automatic* for none and for a colour the palette does not have.
pub fn colour_row(current: Option<&str>) -> usize {
    current
        .and_then(|hex| {
            grind_core::style::PALETTE
                .iter()
                .position(|(_, value)| value.eq_ignore_ascii_case(hex))
        })
        .map_or(0, |index| index + 1)
}

#[cfg(windows)]
pub use paint::{Look, button_ground, label, picker, separator, swatch};

/// The painting half: one function per shape of control, each given the rectangle [`lay_out`]
/// placed it at and the [`Look`] it is in. Every one of them is what the text pane's strip drew
/// before there were two strips, moved here unchanged so the two cannot drift.
#[cfg(windows)]
mod paint {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::HDC;

    use crate::gdi::{self, Font, Selected};
    use crate::sheet::draw::{Align, draw_text};
    use crate::sheet::geom::{Rect, scale};
    use crate::theme::{Interaction, Rgb, Theme, control_fill};

    /// What a control looks like right now, beyond where it is.
    #[derive(Clone, Copy, Debug)]
    pub struct Look {
        pub theme: Theme,
        pub dpi: u32,
        /// Pressed, pointed at, or neither — the pointer's state, which the pane keeps.
        pub state: Interaction,
    }

    /// A control's ground, if it has one in this state. `checked` is a toggle that is in; `filled`
    /// is the difference between a *toggle*, transparent until you point at it so a row of them
    /// reads as one group, and a *field or button*, which looks like one at rest.
    pub fn button_ground(dc: HDC, rect: Rect, look: Look, checked: bool, filled: bool) {
        let theme = look.theme;
        let Some(fill) = control_fill(theme, look.state, checked, filled) else {
            return;
        };
        let border = match (checked, filled) {
            (true, _) => theme.accent.blend(fill, 0.55),
            (false, true) => theme.stroke,
            (false, false) => fill,
        };
        let (left, top, right, bottom) = rect.edges();
        let radius = scale(crate::theme::space::RADIUS, look.dpi).round() as i32;
        gdi::round_rect(
            dc,
            RECT {
                left,
                top,
                right,
                bottom,
            },
            radius,
            fill,
            border,
        );
    }

    /// A label centred on a control, in `font` and the ink a toggle's state asks for: the accent
    /// when it is in, the ordinary ink otherwise — Fluent's `ToggleButton` does both, and one
    /// without the other is a button that looks selected and says nothing.
    pub fn label(dc: HDC, rect: Rect, text: &str, font: &Font, ink: Rgb) {
        let (left, top, right, bottom) = rect.edges();
        let _font = Selected::font(dc, font);
        draw_text(dc, text, left, top, right, bottom, Align::Center, ink, 0.0);
    }

    /// A picker: a field with its current value, or a placeholder in the tertiary ink when nothing
    /// is set, and a drawn chevron — drawn, not typed, because a rendered frame came back with a
    /// missing-glyph box where `▾` should have been (`gdi::triangle_down`).
    pub fn picker(dc: HDC, rect: Rect, look: Look, text: &str, set: bool) {
        let theme = look.theme;
        button_ground(dc, rect, look, false, true);
        let (left, top, right, bottom) = rect.edges();
        let chevron = scale(14.0, look.dpi).round() as i32;
        let pad = scale(crate::theme::space::GAP + 2.0, look.dpi);
        draw_text(
            dc,
            text,
            left,
            top,
            right - chevron,
            bottom,
            Align::Left,
            match set {
                true => theme.text,
                false => theme.text_tertiary,
            },
            pad,
        );
        gdi::triangle_down(
            dc,
            right - chevron / 2,
            (top + bottom) / 2 - scale(1.0, look.dpi).round() as i32,
            scale(7.0, look.dpi).round() as i32,
            theme.text_tertiary,
        );
    }

    /// The rule between two groups, where [`super::lay_out`] put it.
    pub fn separator(dc: HDC, rule: Rect, theme: Theme) {
        let (left, top, right, bottom) = rule.edges();
        gdi::fill(dc, left, top, right.max(left + 1), bottom, theme.divider);
    }

    /// One colour swatch: an **A** with the colour it carries shown the way that colour is used —
    /// as a bar under the letter for a text colour, and as the ground behind it for a fill (a
    /// highlight, a cell background). So two swatches are told apart by what they *do* rather than
    /// by a label.
    ///
    /// *Automatic* — no colour set — draws the bar in the theme's own ink, since that is what
    /// automatic resolves to, and a fill with nothing set draws a hollow ground, which is what no
    /// fill looks like while still reading as a control.
    #[allow(clippy::too_many_arguments)]
    pub fn swatch(
        dc: HDC,
        rect: Rect,
        theme: Theme,
        hex: Option<&str>,
        ground: bool,
        font_px: i32,
        face: &str,
    ) {
        let (left, top, right, bottom) = rect.edges();
        let inset = ((right - left) / 5).max(2);
        let bar = (rect.h / 6.0).round().max(3.0) as i32;
        let fill = hex.and_then(Rgb::parse);
        if ground {
            gdi::round_rect(
                dc,
                RECT {
                    left: left + inset,
                    top: top + inset,
                    right: right - inset,
                    bottom: bottom - bar - inset / 2,
                },
                2,
                fill.unwrap_or(theme.backdrop),
                fill.unwrap_or(theme.stroke),
            );
        }
        {
            let font = Font::new(face, font_px, false);
            let _font = Selected::font(dc, &font);
            // On a fill the letter has to read against the *document's* colour, which can be any
            // of `PALETTE`'s — so it is chosen for contrast rather than fixed.
            let ink = match (ground, fill) {
                (true, Some(colour)) => match colour.contrast(theme.text) > 3.0 {
                    true => theme.text,
                    false => Rgb(0, 0, 0),
                },
                _ => theme.text,
            };
            draw_text(
                dc,
                "A",
                left,
                top,
                right,
                bottom - bar,
                Align::Center,
                ink,
                0.0,
            );
        }
        if !ground {
            gdi::round_rect(
                dc,
                RECT {
                    left: left + inset,
                    top: bottom - bar - inset,
                    right: right - inset,
                    bottom: bottom - inset,
                },
                1,
                fill.unwrap_or(theme.text),
                fill.unwrap_or(theme.stroke),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band() -> Rect {
        Rect {
            x: 0.0,
            y: 10.0,
            w: 800.0,
            h: 44.0,
        }
    }

    #[test]
    fn controls_go_left_to_right_centred_on_the_band() {
        let placed = lay_out(band(), 96, &[(32.0, false), (32.0, false), (64.0, true)]);
        assert_eq!(placed[0].rect.x, space::GROUP, "one group's margin in");
        assert_eq!(placed[0].rect.y, 10.0 + (44.0 - space::CONTROL_H) / 2.0);
        assert_eq!(placed[1].rect.x, placed[0].rect.x + 32.0 + space::GAP / 2.0);
        assert_eq!(placed[2].rect.x, placed[1].rect.x + 32.0 + space::GROUP);
        assert_eq!(placed[2].rect.w, 64.0);
        for window in placed.windows(2) {
            assert!(window[0].rect.x + window[0].rect.w <= window[1].rect.x);
        }
    }

    #[test]
    fn a_separator_stands_in_the_middle_of_a_group_gap_and_nowhere_else() {
        let placed = lay_out(band(), 96, &[(32.0, true), (32.0, false), (32.0, true)]);
        assert_eq!(placed[0].separator, None, "nothing before the first");
        assert_eq!(placed[1].separator, None);
        let rule = placed[2].separator.expect("a new group");
        let gap_start = placed[1].rect.x + placed[1].rect.w;
        assert!(rule.x > gap_start && rule.x < placed[2].rect.x);
        assert_eq!(rule.h, placed[2].rect.h / 2.0, "half a control tall");
    }

    #[test]
    fn the_strip_scales_with_the_monitor() {
        let at_100 = lay_out(band(), 96, &[(32.0, false), (48.0, true)]);
        let big = Rect { h: 88.0, ..band() };
        let at_200 = lay_out(big, 192, &[(32.0, false), (48.0, true)]);
        assert_eq!(at_200[1].rect.w, 2.0 * at_100[1].rect.w);
        assert_eq!(at_200[1].rect.x, 2.0 * at_100[1].rect.x);
    }

    #[test]
    fn the_colour_rows_are_automatic_then_the_palette_capitalised() {
        let rows = colour_choices();
        assert_eq!(rows[0], "Automatic");
        assert_eq!(rows[1], "Navy");
        assert_eq!(rows.len(), grind_core::style::PALETTE.len() + 1);
        assert_eq!(colour_row(None), 0);
        let (_, navy) = grind_core::style::PALETTE[0];
        assert_eq!(colour_row(Some(navy)), 1);
        assert_eq!(
            colour_row(Some(&navy.to_uppercase())),
            1,
            "hex case is not a colour"
        );
        assert_eq!(colour_row(Some("#123456")), 0, "not in the palette");
    }

    #[test]
    fn a_click_finds_the_control_it_is_on() {
        let placed = lay_out(band(), 96, &[(32.0, false), (32.0, true)]);
        let second = placed[1].rect;
        assert_eq!(hit(&placed, second.x + 1.0, second.y + 1.0), Some(1));
        assert_eq!(hit(&placed, second.x - 1.0, second.y + 1.0), None, "a gap");
        assert_eq!(hit(&placed, 5.0, 5.0), None, "above the band");
    }
}
