// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How a cell's text is set in its box, as far as the document and the value decide it — where
//! it sits across and down, and whether it is bold or italic.
//!
//! **Rules about documents, not about a toolkit**, which is why they are here: "a number is
//! right-aligned unless the document says otherwise" is the kind of thing that quietly differs
//! between shells when each one re-decides it in its own painting code, and it had. The GNOME
//! window centred a boolean and an error, as `doc/sheet-shell.md` says a grid does; the Windows
//! pane right-aligned a boolean and left an error on the left. The macOS shell would have been
//! the third copy (`doc/macos-shell.md`, M1), so the rule the phase-9 plan wrote down is the one
//! every shell now draws.
//!
//! What a *colour* looks like is not here: the document's colour is an ODF value every shell
//! parses into its own colour type, and what it looks like on a dark page is
//! `grind_core::color`'s.

use crate::formula::value::FormulaError;
use crate::model::CellValue;
use crate::style::{CellStyle, EDGES, border_parts};
use grind_core::color::{self, Rgb};

/// Where a cell's text sits across its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Where a cell's text sits down its box. Middle unless the cell says otherwise — which is what
/// `style:vertical-align="automatic"` means for a value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// Where a value's type puts it, when the document does not say: **numbers right, text left, a
/// boolean or an error centred** (`doc/sheet-shell.md`).
///
/// The convention carries information. A column of figures lines up on its digits, and a number
/// that reads as text is visibly left-aligned, which is how somebody spots the import that went
/// wrong. Errors are centred as LibreOffice and Excel both draw them.
///
/// An error is a value spelled as **one of the error names**, exactly. `FormulaError::from_name`
/// reads anything that starts with `#` as an error, which is right for a document's value and
/// wrong for a label: the GNOME window centred `# of items`.
pub fn by_type(value: &CellValue) -> Align {
    match value {
        CellValue::Number(_) => Align::Right,
        CellValue::Bool(_) => Align::Center,
        CellValue::Text(text) if is_error(text) => Align::Center,
        CellValue::Text(_) | CellValue::Empty => Align::Left,
    }
}

/// Whether `text` is an error's own name — `#DIV/0!`, `#N/A` — rather than text that happens to
/// start with `#`.
fn is_error(text: &str) -> bool {
    FormulaError::from_name(text).is_some_and(|error| error.name().eq_ignore_ascii_case(text))
}

/// `fo:text-align` as a left-to-right grid draws it (§16.5 — the model keeps the ODF spelling
/// verbatim). `start` and `end` are relative to the writing direction, and this suite is
/// left-to-right by decision (`doc/text-layout.md`); `left` and `right` are what other producers
/// write. Anything else — `justify`, which means nothing for one line in a cell, or a value from
/// a newer ODF — is `None`, so the value's own rule applies rather than a guess (R5).
pub fn by_style(value: &str) -> Option<Align> {
    match value {
        "start" | "left" => Some(Align::Left),
        "center" => Some(Align::Center),
        "end" | "right" => Some(Align::Right),
        _ => None,
    }
}

/// Where a cell's text sits across its box: what the style says, else what the value's type
/// says.
pub fn align(value: &CellValue, style: Option<&CellStyle>) -> Align {
    style
        .and_then(|style| style.align.as_deref())
        .and_then(by_style)
        .unwrap_or_else(|| by_type(value))
}

/// Where a cell's text sits down its box: `style:vertical-align`'s `top` and `middle`, and the
/// bottom for everything else — `automatic`, no value at all, and a value from a newer ODF —
/// which is where LibreOffice puts it (`doc/ods-format.md` §5.1).
pub fn valign(style: Option<&CellStyle>) -> VAlign {
    match style.and_then(|style| style.vertical_align.as_deref()) {
        Some("top") => VAlign::Top,
        Some("middle") => VAlign::Middle,
        _ => VAlign::Bottom,
    }
}

/// Whether a cell's text is set bold: `fo:font-weight` is `bold`, `normal`, or a hundreds number,
/// and 600 and up is bold everywhere else, so it is bold here.
pub fn is_bold(style: Option<&CellStyle>) -> bool {
    bold_weight(style.and_then(|style| style.font_weight.as_deref()))
}

/// Whether a cell's text is set italic — `italic`, or `oblique`, which a grid draws the same.
pub fn is_italic(style: Option<&CellStyle>) -> bool {
    italic_style(style.and_then(|style| style.font_style.as_deref()))
}

/// [`is_bold`]'s rule over the raw `fo:font-weight`, for a shell resolving a font from a
/// [`grind_core::style::TextStyle`] rather than from a cell.
pub fn bold_weight(weight: Option<&str>) -> bool {
    match weight {
        Some("bold") => true,
        Some(other) => other.parse::<u32>().is_ok_and(|weight| weight >= 600),
        None => false,
    }
}

/// [`is_italic`]'s rule over the raw `fo:font-style`.
pub fn italic_style(style: Option<&str>) -> bool {
    matches!(style, Some("italic" | "oblique"))
}

/// Whether a cell's text wraps at its column's width rather than running on — `fo:wrap-option`.
pub fn wraps(style: Option<&CellStyle>) -> bool {
    style.is_some_and(|style| style.wrap.as_deref() == Some("wrap"))
}

/// The size a cell that names none is set in, in points — `style:default-style`'s `fo:font-size`
/// in every document LibreOffice writes, and what a spreadsheet has meant by "no size given"
/// since long before ODF.
pub const DEFAULT_FONT_PT: f64 = 10.0;

/// A cell's `fo:font-size` as a multiple of [`DEFAULT_FONT_PT`], or `None` when it names no size
/// this build reads (a percentage, a length in another unit).
///
/// **A multiple, not an absolute**, because a shell draws its default cell in its own UI font at
/// its own size, and a cell the document made twice as large has to come out twice as large as
/// *that* — which an absolute 20pt would not, beside a 13pt system font. One parse for drawing
/// and measuring alike, so a cell cannot be drawn at one size and measured at another.
pub fn font_scale(font_size: Option<&str>) -> Option<f64> {
    font_size
        .and_then(|size| size.strip_suffix("pt"))
        .and_then(|points| points.trim().parse::<f64>().ok())
        .filter(|points| *points > 0.0)
        .map(|points| points / DEFAULT_FONT_PT)
}

/// The four properties of a cell's style that change how *wide* its text is, as the text style
/// `grind_core::layout` measures with. No family: the model does not carry one for a cell
/// (LibreOffice rewrites it into a font-face reference).
pub fn text_style(style: Option<&CellStyle>) -> grind_core::style::TextStyle {
    grind_core::style::TextStyle {
        font_family: None,
        font_size: style.and_then(|s| s.font_size.clone()),
        font_weight: style.and_then(|s| s.font_weight.clone()),
        font_style: style.and_then(|s| s.font_style.clone()),
    }
}

/// One filled rectangle of a border — `(x, y, width, height)` in whatever unit the cell was
/// given in, and the colour to fill it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub rect: (f64, f64, f64, f64),
    pub color: Rgb,
}

/// A cell's own borders as rectangles to fill, each centred on the grid line it stands on — so a
/// cell's right border and its neighbour's left one are the same line, and either covers the
/// hairline under it.
///
/// A border is ODF's three parts (`0.5pt solid #000000`, [`border_parts`]): never thinner than
/// one device pixel (`hairline`), in the document's colour lifted to read on a dark page
/// (`color::document_ink`), and `double` as two thin lines with a gap. `none`, `hidden` and a
/// border this build cannot read draw nothing — R5's tolerance. `cell` is `(x, y, w, h)`.
///
/// Hoisted out of the macOS shell when the Windows grid wanted borders too; both draw these.
///
/// ponytail: `dashed` and `dotted` are drawn solid. The trigger is a document whose dashes are
/// what distinguishes two of its tables; a dash is a run of fills along the edge.
pub fn border_strokes(
    cell: (f64, f64, f64, f64),
    style: &CellStyle,
    page: Rgb,
    dark: bool,
    hairline: f64,
) -> Vec<Stroke> {
    let (x, y, w, h) = cell;
    let mut out = Vec::new();
    if w <= 0.0 || h <= 0.0 {
        return out;
    }
    for (edge, border) in style.borders.iter().enumerate() {
        let Some((points, line, ink)) = border.as_deref().and_then(border_parts) else {
            continue;
        };
        if matches!(line, "none" | "hidden") || points <= 0.0 {
            continue;
        }
        let Some(own) = color::parse(ink) else {
            continue;
        };
        let color = color::document_ink(Some(own), None, page, own, dark);
        let width = points.max(hairline);
        let at = match EDGES[edge] {
            "left" => x,
            "right" => x + w,
            "top" => y,
            _ => y + h,
        } - hairline / 2.0;
        let vertical = matches!(EDGES[edge], "left" | "right");
        let strokes: Vec<(f64, f64)> = match line {
            "double" => {
                let thin = (width / 3.0).max(hairline);
                let gap = thin.max(hairline);
                vec![(at - gap / 2.0 - thin, thin), (at + gap / 2.0, thin)]
            }
            _ => vec![(at - width / 2.0, width)],
        };
        for (from, thickness) in strokes {
            let rect = match vertical {
                true => (from, y, thickness, h),
                false => (x, from, w, thickness),
            };
            out.push(Stroke { rect, color });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled(set: impl FnOnce(&mut CellStyle)) -> CellStyle {
        let mut style = CellStyle::default();
        set(&mut style);
        style
    }

    #[test]
    fn a_values_type_decides_where_it_sits() {
        assert_eq!(by_type(&CellValue::Number(1.5)), Align::Right);
        assert_eq!(by_type(&CellValue::Text("Rent".into())), Align::Left);
        assert_eq!(by_type(&CellValue::Bool(true)), Align::Center);
        assert_eq!(by_type(&CellValue::Text("#DIV/0!".into())), Align::Center);
        assert_eq!(by_type(&CellValue::Text("#N/A".into())), Align::Center);
        // A label that starts with `#` is a label.
        assert_eq!(by_type(&CellValue::Text("# of items".into())), Align::Left);
        assert_eq!(by_type(&CellValue::Text("#hashtag".into())), Align::Left);
        assert_eq!(by_type(&CellValue::Empty), Align::Left);
    }

    #[test]
    fn the_style_overrides_the_type_and_an_unknown_value_does_not() {
        let number = CellValue::Number(3.0);
        let centred = styled(|s| s.align = Some("center".into()));
        assert_eq!(align(&number, Some(&centred)), Align::Center);
        let start = styled(|s| s.align = Some("start".into()));
        assert_eq!(align(&number, Some(&start)), Align::Left);
        let right = styled(|s| s.align = Some("right".into()));
        assert_eq!(
            align(&CellValue::Text("x".into()), Some(&right)),
            Align::Right
        );
        let justify = styled(|s| s.align = Some("justify".into()));
        assert_eq!(
            align(&number, Some(&justify)),
            Align::Right,
            "tolerated, not obeyed"
        );
        assert_eq!(align(&number, None), Align::Right);
    }

    #[test]
    fn vertical_alignment_is_the_bottom_unless_the_cell_says() {
        assert_eq!(valign(None), VAlign::Bottom);
        let top = styled(|s| s.vertical_align = Some("top".into()));
        assert_eq!(valign(Some(&top)), VAlign::Top);
        let middle = styled(|s| s.vertical_align = Some("middle".into()));
        assert_eq!(valign(Some(&middle)), VAlign::Middle);
        let automatic = styled(|s| s.vertical_align = Some("automatic".into()));
        assert_eq!(valign(Some(&automatic)), VAlign::Bottom);
    }

    #[test]
    fn a_font_size_is_a_multiple_of_the_default() {
        assert_eq!(font_scale(Some("20pt")), Some(2.0));
        assert_eq!(font_scale(Some("10pt")), Some(1.0));
        assert_eq!(font_scale(Some("120%")), None, "not a size this reads");
        assert_eq!(font_scale(Some("0pt")), None);
        assert_eq!(font_scale(None), None);
        let big = styled(|s| s.font_size = Some("14pt".into()));
        assert_eq!(text_style(Some(&big)).font_size.as_deref(), Some("14pt"));
        assert_eq!(text_style(None), grind_core::style::TextStyle::default());
    }

    #[test]
    fn weight_six_hundred_and_up_is_bold_and_oblique_is_italic() {
        assert!(!is_bold(None));
        assert!(is_bold(Some(&styled(
            |s| s.font_weight = Some("bold".into())
        ))));
        assert!(is_bold(Some(&styled(
            |s| s.font_weight = Some("600".into())
        ))));
        assert!(!is_bold(Some(&styled(
            |s| s.font_weight = Some("500".into())
        ))));
        assert!(!is_bold(Some(&styled(
            |s| s.font_weight = Some("normal".into())
        ))));
        assert!(is_italic(Some(&styled(
            |s| s.font_style = Some("oblique".into())
        ))));
        assert!(!is_italic(Some(&styled(
            |s| s.font_style = Some("normal".into())
        ))));
        assert!(wraps(Some(&styled(|s| s.wrap = Some("wrap".into())))));
        assert!(!wraps(None));
    }
}
