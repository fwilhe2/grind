// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cell styling — the `style:style` of family `table-cell` (doc/ods-format.md §5.1). **\[ODS\]**
//!
//! The other half of §5: a number format says how a *value* is spelled, this says how the
//! cell looks around it. The two travel together on one `style:style` and are pooled
//! together, which is why the writer keys its pool on the pair.
//!
//! **The values are ODF's own, kept verbatim.** `fo:font-weight` is `"bold"`, a border is
//! `"0.5pt solid #000000"`, a colour is `"#ffff00"` — these are XSL-FO strings, and parsing
//! them into a typed model would buy nothing until something *renders* them. Nothing does
//! yet: no shell exists, and a CLI cannot show a border. What matters today is that a
//! document keeps what it came with, and a string does that exactly. When a renderer needs
//! a colour as three bytes, it parses one string in one place.
//!
//! Validation therefore lives at the edge that has a user, not here: `sheet style` takes an
//! enum for alignment and checks a colour's syntax, and a *document's* value is whatever the
//! document said, because rejecting it would lose the cell rather than the attribute.
//!
//! Two things are deliberately not carried, both because LibreOffice does not give them
//! back as written (§5.4, measured):
//!
//! * **`fo:font-family`**, which LO replaces with a `style:font-name` pointing into
//!   `office:font-face-decls` — a second vocabulary, and one nothing can use until text is
//!   drawn.
//! * **Border widths are re-quantised** on the way through: `0.5pt` comes back `0.51pt`.
//!   The string is kept as the document wrote it, and loop C compares widths numerically
//!   rather than pretending the round trip is exact.

use serde::{Deserialize, Serialize};

/// Which edge a border is on. The array order in [`CellStyle::borders`].
pub const EDGES: [&str; 4] = ["left", "right", "top", "bottom"];

/// The colours a shell offers by default, and the names a person may spell them by — the
/// palette at <https://clrs.cc/>.
///
/// A **default, never a limit**: ODF takes any `#rrggbb`, `CellStyle` keeps whatever the
/// document said, and a shell may still ask for a colour that is not here. What this fixes is
/// the two places a colour is *chosen* rather than read — a GUI's swatches and `sheet style
/// --color` — for the same reason `numfmt::preset` lives in the core: a document coloured
/// from one shell and one coloured from another should not need a second table to agree.
///
/// The hexes are held against a real document rather than against this comment:
/// `core/tests/data/samples/custom-colors.fods` is this palette as LibreOffice wrote it, with
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

/// How one cell looks. Every field is an ODF attribute value, and `None` means the
/// attribute is absent — which is not the same as a value of `"none"`, the spelling ODF
/// uses for "explicitly no border".
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellStyle {
    /// `fo:font-weight` — `normal`, `bold`, or a hundreds number.
    pub font_weight: Option<String>,
    /// `fo:font-style` — `normal`, `italic`, `oblique`.
    pub font_style: Option<String>,
    /// `fo:font-size` — a length, e.g. `12pt`, or a percentage.
    pub font_size: Option<String>,
    /// `fo:color` — the text colour, `#rrggbb`.
    pub color: Option<String>,
    /// `fo:background-color` — `#rrggbb` or `transparent`.
    pub background: Option<String>,
    /// `fo:text-align`, on `style:paragraph-properties` rather than on the cell.
    pub align: Option<String>,
    /// `style:vertical-align` — `top`, `middle`, `bottom`, `automatic`.
    pub vertical_align: Option<String>,
    /// `fo:wrap-option` — `wrap` or `no-wrap`.
    pub wrap: Option<String>,
    /// `fo:border-{left,right,top,bottom}`, in [`EDGES`] order. The `fo:border` shorthand is
    /// expanded into all four on the way in and collapsed back when they agree, because
    /// that is what the shorthand *means* — keeping it as a fifth field would make two
    /// spellings of one style unequal, and the pool would emit both.
    pub borders: [Option<String>; 4],
}

impl CellStyle {
    /// Nothing set at all — a cell style that is worth neither writing nor pooling.
    pub fn is_plain(&self) -> bool {
        *self == CellStyle::default()
    }

    /// The `fo:border` shorthand, when every edge agrees and is present.
    pub fn uniform_border(&self) -> Option<&str> {
        let first = self.borders[0].as_deref()?;
        self.borders
            .iter()
            .all(|edge| edge.as_deref() == Some(first))
            .then_some(first)
    }

    pub fn set_border(&mut self, value: Option<String>) {
        self.borders = [value.clone(), value.clone(), value.clone(), value];
    }
}

/// A border as its three parts — width in points, line style, colour.
///
/// Not how a border is *stored*: this exists because LibreOffice re-quantises the width
/// (§5.4), so anything comparing two borders across a round trip has to compare the number
/// rather than the text. `None` for anything that is not the three-part form, including
/// ODF's `"none"`.
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
/// The one parser, in one place — column widths and row heights are stored as the strings
/// the document wrote (see [`crate::model::Sheet::col_width`]), and everything that has to
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
    fn the_border_shorthand_is_four_edges_that_agree() {
        let mut style = CellStyle::default();
        assert_eq!(style.uniform_border(), None);

        style.set_border(Some("0.5pt solid #000000".into()));
        assert_eq!(style.uniform_border(), Some("0.5pt solid #000000"));

        // One edge differing is no longer the shorthand.
        style.borders[1] = Some("1pt solid #ff0000".into());
        assert_eq!(style.uniform_border(), None);
        // Nor is a missing edge, which is not the same as an edge set to "none".
        style.borders[1] = None;
        assert_eq!(style.uniform_border(), None);
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
}
