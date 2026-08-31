// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The styling vocabulary every document type is made of. **\[GENERIC\]**
//!
//! ODF builds a cell style, a paragraph style and a text span style out of the *same* pieces:
//! XSL-FO properties (`fo:font-weight`, `fo:color`, `fo:border-left`), ODF lengths, and
//! `#rrggbb` colours. What differs per document type is which pieces a style carries and what
//! element it hangs off — so the pieces live here and the structs live with their document
//! type (`grind_sheet::style::CellStyle` is the worked example).
//!
//! **The values are ODF's own, kept verbatim.** `fo:font-weight` is `"bold"`, a border is
//! `"0.5pt solid #000000"`, a colour is `"#ffff00"` — these are XSL-FO strings, and parsing
//! them into a typed model would buy nothing until something *renders* them. What matters is
//! that a document keeps what it came with, and a string does that exactly. When a renderer
//! needs a colour as three bytes, it parses one string in one place.
//!
//! Validation therefore lives at the edge that has a user, not here: a CLI takes an enum for
//! alignment and checks a colour's syntax, and a *document's* value is whatever the document
//! said, because rejecting it would lose the element rather than the attribute.

/// Which edge a border is on. The array order of a style's four border slots.
pub const EDGES: [&str; 4] = ["left", "right", "top", "bottom"];

/// The colours a shell offers by default, and the names a person may spell them by — the
/// palette at <https://clrs.cc/>.
///
/// A **default, never a limit**: ODF takes any `#rrggbb`, a style keeps whatever the document
/// said, and a shell may still ask for a colour that is not here. What this fixes is the two
/// places a colour is *chosen* rather than read — a GUI's swatches and a CLI's `--color` — so
/// that a document coloured from one shell and one coloured from another do not need a second
/// table to agree.
///
/// The hexes are held against a real document rather than against this comment:
/// `sheet/tests/data/samples/custom-colors.fods` is this palette as LibreOffice wrote it, with
/// each colour's name in the cell it fills, and a test reads it back.
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

/// A colour as a *person* spelled it, resolved to what ODF stores.
///
/// A [`PALETTE`] name first, so `--color navy`, a GUI's navy swatch and a generator's
/// `style().color("navy")` all write the same attribute; a `#rrggbb` or `transparent` is kept
/// as it is; anything else is an error naming what was allowed.
///
/// **This is where a user's typing enters, which is not where a document's value does.** The
/// module comment above is the rule: a colour read out of a file is whatever the file said and
/// is never checked, because rejecting it would lose the element rather than the attribute. So
/// this is deliberately not called by any reader — only by the surfaces where somebody
/// *chooses* a colour, and it lives here so that those surfaces cannot disagree about what
/// `navy` is.
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

/// A border as its three parts — width in points, line style, colour.
///
/// Not how a border is *stored*: this exists because LibreOffice re-quantises the width
/// (doc/ods-format.md §5.4), so anything comparing two borders across a round trip has to
/// compare the number rather than the text. `None` for anything that is not the three-part
/// form, including ODF's `"none"`.
pub fn border_parts(border: &str) -> Option<(f64, &str, &str)> {
    let mut fields = border.split_whitespace();
    let width = fields.next()?;
    let style = fields.next()?;
    let color = fields.next()?;
    let points = width.strip_suffix("pt")?.parse::<f64>().ok()?;
    Some((points, style, color))
}

/// An ODF length (`"2.258cm"`, `"0.889in"`, `"64pt"`) in millimetres.
///
/// The one parser, in one place — a column width, a row height, a page margin and a paragraph
/// indent are all stored as the strings the document wrote, and everything that has to
/// *measure* one comes through here: a renderer laying out a grid, and loop C comparing a
/// width across a round trip that respells `2.258cm` as `22.58mm`.
///
/// `None` for anything that is not a number and one of §18's units. `px` is deliberately
/// absent: ODF allows it, but a pixel has no defined size in a document, and nothing in the
/// corpus writes one.
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

/// A measurement back as an ODF length. Millimetres, three decimals — enough that a
/// round trip through LibreOffice's own `cm` spelling stays inside loop C's tolerance, and
/// short enough not to write `12.000000000000002mm`.
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
        assert_eq!(border_parts("0.5mm solid #000000"), None);
    }

    #[test]
    fn a_palette_name_resolves_case_insensitively_and_nothing_else_does() {
        assert_eq!(palette("navy"), Some("#001f3f"));
        assert_eq!(palette(" Navy "), Some("#001f3f"));
        assert_eq!(palette("#001f3f"), None, "a hex is kept, not looked up");
        assert_eq!(palette("chartreuse"), None);
    }
}

/// The text properties that decide how *wide* a piece of text is.
///
/// The metrics half of a character style, and nothing else: no colour, no underline, no
/// background — those change how text looks and not how much room it takes, so a layout engine
/// has no use for them. Values are ODF's own and verbatim, exactly as everywhere else in this
/// module; the shell's [`crate::layout::Metrics`] implementation is what turns
/// `Some("10.5pt")` into a number, because only it knows what unit it is answering in.
///
/// Deliberately **not** a document type's style struct. `grind_sheet::style::CellStyle` carries
/// these four fields among a dozen others and can hand one of these out; a text run's character
/// style will do the same once style *definitions* are read (`doc/text-core.md` — until then a
/// run measures with the default, which is the one honest thing to do with a style name whose
/// properties nobody has looked up).
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
