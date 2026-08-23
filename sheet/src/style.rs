// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cell styling — the `style:style` of family `table-cell` (doc/ods-format.md §5.1). **\[ODS\]**
//!
//! The other half of §5: a number format says how a *value* is spelled, this says how the
//! cell looks around it. The two travel together on one `style:style` and are pooled
//! together, which is why the writer keys its pool on the pair.
//!
//! **The pieces a cell style is made of are not spreadsheet-specific and do not live here.**
//! `fo:font-weight`, an ODF length, a `#rrggbb` colour and a three-part border are the same
//! constructs a paragraph style and a text span style are built from, so they are
//! `grind_core::style` and are re-exported below. What is here is the one thing that is
//! genuinely about cells: which of those pieces a cell carries.
//!
//! **The values are ODF's own, kept verbatim** — see `grind_core::style` for why, and for
//! where validation lives instead.
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

/// The styling vocabulary, from the crate that has no opinion about document types.
pub use grind_core::style::{EDGES, PALETTE, border_parts, length_mm, mm_length, palette};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
