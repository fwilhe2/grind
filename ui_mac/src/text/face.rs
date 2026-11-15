// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which font a run is shown in where the page names one — the font panel's reading of the
//! selection. The **block's** face is its [`Role`] (`grind_text::look`); the **run's** direct
//! formatting goes over it — `**bold**` in a heading is bold *and* a heading, a run's family and
//! size replace the face's. The document itself is set by `grind_print`.
//!
//! **Sizes are points**, honoured as written: `14pt` is fourteen points, `120%` a fifth bigger
//! than the block. The body is [`BODY_PT`], LibreOffice's default, so a document stating no sizes
//! looks as one stating LibreOffice's does.

use grind_core::style::TextStyle;
use grind_sheet::look::{bold_weight, italic_style};
use grind_text::look::Role;

use crate::metrics::{Family, Font};

/// The body's size, in points — `metrics::BASE_PT`, the grid's, and LibreOffice's default.
pub const BODY_PT: f64 = crate::metrics::BASE_PT;

/// A `fo:font-size`, in points: absolute when it says `pt`, relative to `base` when it is a
/// percentage, and `base` for anything else — a length in `cm` is left alone rather than guessed
/// at, exactly as the GNOME window's `size_units` leaves it.
fn size(value: Option<&str>, base: f64) -> f64 {
    let Some(value) = value.map(str::trim) else {
        return base;
    };
    if let Some(points) = value
        .strip_suffix("pt")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|points| *points > 0.0)
    {
        return points;
    }
    value
        .strip_suffix('%')
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|percent| *percent > 0.0)
        .map_or(base, |percent| base * percent / 100.0)
}

/// The font a run formatted `run` is set in, in a block whose face is `role`.
pub fn font(role: Role, run: &TextStyle) -> Font {
    let family = match run.font_family.as_deref() {
        Some(grind_text::markdown::MONOSPACE) => Family::Mono,
        Some(named) if !named.trim().is_empty() => Family::Named(named.to_owned()),
        _ if role.mono() => Family::Mono,
        _ => Family::System,
    };
    Font {
        size: size(run.font_size.as_deref(), BODY_PT * role.scale()),
        bold: role.bold() || bold_weight(run.font_weight.as_deref()),
        italic: role.italic() || italic_style(run.font_style.as_deref()),
        family,
        tabular: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(size: Option<&str>, weight: Option<&str>, family: Option<&str>) -> TextStyle {
        TextStyle {
            font_size: size.map(str::to_owned),
            font_weight: weight.map(str::to_owned),
            font_family: family.map(str::to_owned),
            ..TextStyle::default()
        }
    }

    #[test]
    fn a_plain_run_takes_its_blocks_face() {
        let body = font(Role::Body, &TextStyle::default());
        assert_eq!(
            body,
            Font {
                size: BODY_PT,
                bold: false,
                italic: false,
                family: Family::System,
                tabular: false,
            }
        );
        let h1 = font(Role::Heading(1), &TextStyle::default());
        assert_eq!(h1.size, BODY_PT * 1.8);
        assert!(h1.bold);
        assert!(font(Role::Subtitle, &TextStyle::default()).italic);
        assert_eq!(font(Role::Code, &TextStyle::default()).family, Family::Mono);
    }

    /// Bold inside a heading is still a heading; a run's own family and size win over its block's.
    #[test]
    fn a_runs_own_formatting_goes_over_its_blocks_face() {
        let bold = font(Role::Body, &style(None, Some("bold"), None));
        assert!(bold.bold && bold.size == BODY_PT);
        let named = font(
            Role::Heading(2),
            &style(Some("14pt"), None, Some("Georgia")),
        );
        assert_eq!(named.size, 14.0, "points are points");
        assert_eq!(named.family, Family::Named("Georgia".into()));
        assert!(named.bold, "and a heading is still bold");
        let code = font(
            Role::Body,
            &style(None, None, Some(grind_text::markdown::MONOSPACE)),
        );
        assert_eq!(code.family, Family::Mono, "the generic is the user's own");
    }

    #[test]
    fn a_size_is_points_or_a_share_of_the_block_and_nothing_else() {
        assert_eq!(size(Some("18pt"), 12.0), 18.0);
        assert_eq!(size(Some(" 150% "), 12.0), 18.0);
        assert_eq!(size(Some("0.5cm"), 12.0), 12.0, "a length is left alone");
        assert_eq!(size(Some("-3pt"), 12.0), 12.0, "and so is nonsense");
        assert_eq!(size(None, 21.6), 21.6);
    }
}
