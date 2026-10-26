// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The styling vocabulary every document type is made of. **\[GENERIC\]**
//!
//! A cell style, a paragraph style and a text style are built from the same pieces — XSL-FO
//! properties, ODF lengths, `#rrggbb` colours — so the pieces live here and each document type's
//! style struct lives with it.
//!
//! Values are ODF's own, kept verbatim as strings, so a document keeps what it came with; a
//! renderer that needs a number parses one string in one place. Validation lives where a user
//! types ([`color`], [`border`]): a document's value is whatever it said, since rejecting it would
//! lose the element.

/// Which edge a border is on. The array order of a style's four border slots.
pub const EDGES: [&str; 4] = ["left", "right", "top", "bottom"];

/// The colours a shell offers by default, and the names a person may spell them by
/// (<https://clrs.cc/>). A default, never a limit: any `#rrggbb` is accepted.
/// `sheet/tests/data/samples/custom-colors.fods` holds the hexes to what LibreOffice wrote.
pub const PALETTE: [(&str, &str); 17] = [
    ("navy", "#001f3f"),
    ("blue", "#0074d9"),
    ("aqua", "#7fdbff"),
    ("teal", "#39cccc"),
    ("purple", "#b10dc9"),
    ("fuchsia", "#f012be"),
    ("maroon", "#85144b"),
    ("red", "#ff4136"),
    ("orange", "#ff851b"),
    ("yellow", "#ffdc00"),
    ("olive", "#3d9970"),
    ("green", "#2ecc40"),
    ("lime", "#01ff70"),
    ("black", "#111111"),
    ("gray", "#aaaaaa"),
    ("silver", "#dddddd"),
    ("white", "#ffffff"),
];

/// A [`PALETTE`] name as the colour a document stores, case-insensitively. `None` for anything
/// else, including a hex a caller should keep as it is.
pub fn palette(name: &str) -> Option<&'static str> {
    let name = name.trim().to_ascii_lowercase();
    PALETTE
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, hex)| *hex)
}

/// A colour as a person spelled it, resolved to what ODF stores: a [`PALETTE`] name, a `#rrggbb`
/// or `transparent`, else an error naming what is allowed. Called only where somebody chooses a
/// colour, never by a reader.
pub fn color(value: &str) -> Result<String, String> {
    if let Some(hex) = palette(value) {
        return Ok(hex.to_owned());
    }
    let hex = value.strip_prefix('#').unwrap_or_default();
    if value == "transparent" || (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())) {
        return Ok(value.to_owned());
    }
    Err(format!(
        "{value}: expected #rrggbb, transparent, or a palette name ({})",
        PALETTE
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// A border as a person spelled it — `"0.5pt solid navy"` — with its colour resolved by
/// [`color`], so a typo becomes an error here rather than an attribute LibreOffice drops
/// silently. The width is kept exactly as it was typed; only the colour is rewritten.
pub fn border(value: &str) -> Result<String, String> {
    let malformed = || {
        format!(
            "{value}: expected a width, a line and a colour, \
             e.g. \"0.5pt solid #000000\""
        )
    };
    let fields: Vec<&str> = value.split_whitespace().collect();
    let [width, line, name] = fields[..] else {
        return Err(malformed());
    };
    let resolved = format!("{width} {line} {}", color(name)?);
    match border_parts(&resolved) {
        Some(_) => Ok(resolved),
        None => Err(malformed()),
    }
}

/// A border as its three parts — width in points, line style, colour — for drawing it and for
/// comparing borders across a LibreOffice round trip, which re-quantises the width
/// (`doc/ods-format.md` §5.4). The width may be in any ODF unit: OpenOffice wrote `0.002cm`.
/// `None` for anything else, including `"none"`.
pub fn border_parts(border: &str) -> Option<(f64, &str, &str)> {
    let mut fields = border.split_whitespace();
    let width = fields.next()?;
    let style = fields.next()?;
    let color = fields.next()?;
    let points = length_mm(width)? * 72.0 / 25.4;
    Some((points, style, color))
}

/// An ODF length (`"2.258cm"`, `"0.889in"`, `"64pt"`) in millimetres — the one parser everything
/// that measures a stored length goes through. `None` for anything that is not a number and one of
/// §18's units; `px` has no defined size in a document and is not accepted.
pub fn length_mm(length: &str) -> Option<f64> {
    let length = length.trim();
    let digits = length.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let per_mm = match &length[digits.len()..] {
        "mm" => 1.0,
        "cm" => 10.0,
        "in" => 25.4,
        "pt" => 25.4 / 72.0,
        "pc" => 25.4 / 6.0,
        _ => return None,
    };
    Some(digits.parse::<f64>().ok()? * per_mm)
}

/// A measurement back as an ODF length: millimetres, three decimals.
pub fn mm_length(mm: f64) -> String {
    format!("{:.3}mm", mm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_is_read_in_whatever_unit_it_was_written() {
        assert_eq!(length_mm("22.58mm"), Some(22.58));
        assert_eq!(length_mm("2.258cm"), Some(22.58));
        assert_eq!(length_mm("1in"), Some(25.4));
        assert_eq!(length_mm("72pt"), Some(25.4));
        assert_eq!(length_mm("6pc"), Some(25.4));
        assert_eq!(length_mm("2.5"), None, "a unit is not optional");
        assert_eq!(length_mm("wide"), None);
        assert_eq!(length_mm("2.5px"), None);
        assert_eq!(length_mm(&mm_length(22.58)), Some(22.58));
    }

    #[test]
    fn a_border_splits_into_a_width_a_style_and_a_colour() {
        assert_eq!(
            border_parts("0.51pt solid #000000"),
            Some((0.51, "solid", "#000000"))
        );
        assert_eq!(border_parts("none"), None);
        let (points, _, _) = border_parts("0.0138in solid #000000").unwrap();
        assert!((points - 0.99).abs() < 0.01, "{points}");
        assert_eq!(border_parts("0.5px solid #000000"), None);
    }

    #[test]
    fn a_palette_name_resolves_case_insensitively_and_nothing_else_does() {
        assert_eq!(palette("navy"), Some("#001f3f"));
        assert_eq!(palette(" Navy "), Some("#001f3f"));
        assert_eq!(palette("#001f3f"), None, "a hex is kept, not looked up");
        assert_eq!(palette("chartreuse"), None);
    }
}

/// The text properties that decide how wide a piece of text is — no colour, underline or
/// background, which change how text looks but not the room it takes. Values are ODF's own; a
/// shell's [`crate::layout::Metrics`] turns them into numbers. `CellStyle` and `CharStyle` each
/// hand one out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextStyle {
    /// `fo:font-family`, or `style:font-name` resolved through `office:font-face-decls`.
    pub font_family: Option<String>,
    /// `fo:font-size` — an ODF length, e.g. `"10pt"`.
    pub font_size: Option<String>,
    /// `fo:font-weight` — `normal`, `bold`, or a number.
    pub font_weight: Option<String>,
    /// `fo:font-style` — `normal`, `italic`, `oblique`.
    pub font_style: Option<String>,
}
