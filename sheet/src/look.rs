// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How a cell's text is set in its box, as far as the document and the value decide: where it sits
//! across and down, and whether it is bold or italic — so no shell decides it in its painting
//! code. Colour is `grind_core::color`'s.

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

/// Where a cell's text sits down its box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// Where a value's type puts it when the document does not say: numbers right, text left, a
/// boolean or an error centred (`doc/sheet-shell.md`), so a column of figures lines up and a
/// number read as text shows. An error is a value spelled exactly as one of the error names — `#
/// of items` is text.
pub fn by_type(value: &CellValue) -> Align {
    match value {
        CellValue::Number(_) => Align::Right,
        CellValue::Bool(_) => Align::Center,
        CellValue::Text(text) if is_error(text) => Align::Center,
        CellValue::Text(_) | CellValue::Empty => Align::Left,
    }
}

/// Whether `text` is an error's own name (`#DIV/0!`, `#N/A`) rather than text starting with `#`.
fn is_error(text: &str) -> bool {
    FormulaError::from_name(text).is_some_and(|error| error.name().eq_ignore_ascii_case(text))
}

/// `fo:text-align` as a left-to-right grid draws it: `start`/`end` and the `left`/`right` other
/// producers write. Anything else — `justify`, or a value from a newer ODF — is `None`, and the
/// value's own rule applies (R5).
pub fn by_style(value: &str) -> Option<Align> {
    match value {
        "start" | "left" => Some(Align::Left),
        "center" => Some(Align::Center),
        "end" | "right" => Some(Align::Right),
        _ => None,
    }
}

/// Where a cell's text sits across its box: the style's alignment, else the value type's.
pub fn align(value: &CellValue, style: Option<&CellStyle>) -> Align {
    style
        .and_then(|style| style.align.as_deref())
        .and_then(by_style)
        .unwrap_or_else(|| by_type(value))
}

/// Where a cell's text sits down its box: `style:vertical-align`'s `top` and `middle`, else the
/// bottom, where LibreOffice puts it (`doc/ods-format.md` §5.1).
pub fn valign(style: Option<&CellStyle>) -> VAlign {
    match style.and_then(|style| style.vertical_align.as_deref()) {
        Some("top") => VAlign::Top,
        Some("middle") => VAlign::Middle,
        _ => VAlign::Bottom,
    }
}

/// Whether a cell's text is bold: `fo:font-weight` of `bold`, or 600 and up.
pub fn is_bold(style: Option<&CellStyle>) -> bool {
    bold_weight(style.and_then(|style| style.font_weight.as_deref()))
}

/// Whether a cell's text is set italic — `italic`, or `oblique`, which a grid draws the same.
pub fn is_italic(style: Option<&CellStyle>) -> bool {
    italic_style(style.and_then(|style| style.font_style.as_deref()))
}

/// [`is_bold`]'s rule over a raw `fo:font-weight`.
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

/// How tall a row must be for `laid` set in a cell of `style`: its height, or for turned text the
/// height of the turned box — `width·|sin θ| + height·|cos θ|`, LibreOffice's optimal height to
/// within a point (`doc/ods-format.md` §5.4).
pub fn needed_height(laid: &grind_core::layout::Layout, style: &CellStyle) -> f32 {
    let height = laid.height();
    match style.rotation_degrees().filter(|d| d % 180.0 != 0.0) {
        Some(degrees) => {
            let width = laid.lines().iter().map(|l| l.width).fold(0.0, f32::max);
            let turn = degrees.to_radians() as f32;
            width * turn.sin().abs() + height * turn.cos().abs()
        }
        None => height,
    }
}

/// Whether a cell's text wraps at its column's width rather than running on — `fo:wrap-option`.
pub fn wraps(style: Option<&CellStyle>) -> bool {
    style.is_some_and(|style| style.wrap.as_deref() == Some("wrap"))
}

/// The size a cell that names none is set in, in points — `style:default-style`'s `fo:font-size`
/// in every document LibreOffice writes.
pub const DEFAULT_FONT_PT: f64 = 10.0;

/// A cell's `fo:font-size` as a multiple of [`DEFAULT_FONT_PT`], or `None` for a size not read (a
/// percentage, another unit). A multiple, because a shell draws its default cell in its own UI
/// font, and a cell made twice as large must be twice that. Drawing and measuring share this, so a
/// cell is never drawn at one size and measured at another.
pub fn font_scale(font_size: Option<&str>) -> Option<f64> {
    font_size
        .and_then(|size| size.strip_suffix("pt"))
        .and_then(|points| points.trim().parse::<f64>().ok())
        .filter(|points| *points > 0.0)
        .map(|points| points / DEFAULT_FONT_PT)
}

/// The four style properties that change how wide a cell's text is, for `grind_core::layout`. No
/// family: cells do not carry one.
pub fn text_style(style: Option<&CellStyle>) -> grind_core::style::TextStyle {
    grind_core::style::TextStyle {
        font_family: None,
        font_size: style.and_then(|s| s.font_size.clone()),
        font_weight: style.and_then(|s| s.font_weight.clone()),
        font_style: style.and_then(|s| s.font_style.clone()),
    }
}

/// One filled rectangle of a border — `(x, y, width, height)` in the cell's unit, and its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub rect: (f64, f64, f64, f64),
    pub color: Rgb,
}

/// A cell's own borders as rectangles, each centred on its grid line, so a cell's right border and
/// its neighbour's left one coincide. A border is ODF's three parts ([`border_parts`]), at least
/// one device pixel (`hairline`) thick, in the document's colour lifted for a dark page
/// (`color::document_ink`), and `double` as two lines with a gap. `none`, `hidden` and unreadable
/// borders draw nothing (R5). `cell` is `(x, y, w, h)`. `dashed` and `dotted` are runs of short
/// strokes in step from the origin, so one ruled line reads as one across many cells.
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
            "double" | "double-thin" => {
                let thin = (width / 3.0).max(hairline);
                let gap = thin.max(hairline);
                vec![(at - gap / 2.0 - thin, thin), (at + gap / 2.0, thin)]
            }
            _ => vec![(at - width / 2.0, width)],
        };
        let (start, length) = match vertical {
            true => (y, h),
            false => (x, w),
        };
        let runs = match line {
            // LibreOffice's own dash kinds drawn as the nearest ODF one.
            "dashed" | "fine-dashed" | "dash-dot" | "dash-dot-dot" => {
                dashes(start, start + length, 3.0 * width, 2.0 * width)
            }
            "dotted" => dashes(start, start + length, width, width),
            _ => vec![(start, length)],
        };
        for (from, thickness) in strokes {
            for &(along, run) in &runs {
                let rect = match vertical {
                    true => (from, along, thickness, run),
                    false => (along, from, run, thickness),
                };
                out.push(Stroke { rect, color });
            }
        }
    }
    out
}

/// The `on` lengths of an `on`/`off` pattern counted from zero that fall within `start..end`,
/// as `(from, length)`.
fn dashes(start: f64, end: f64, on: f64, off: f64) -> Vec<(f64, f64)> {
    let period = on + off;
    let mut out = Vec::new();
    let mut at = (start / period).floor() * period;
    while at < end {
        let (from, to) = (at.max(start), (at + on).min(end));
        if to > from {
            out.push((from, to - from));
        }
        at += period;
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

    /// A bottom border alone, on a 10 × 10 cell at `x`, a point a device pixel.
    fn bottom(x: f64, border: &str) -> Vec<(f64, f64)> {
        let style = styled(|s| s.borders[3] = Some(border.to_owned()));
        border_strokes((x, 0.0, 10.0, 10.0), &style, (255, 255, 255), false, 1.0)
            .into_iter()
            .map(|stroke| (stroke.rect.0, stroke.rect.2))
            .collect()
    }

    #[test]
    fn a_dashed_or_dotted_border_is_cut_in_step_across_cells() {
        assert_eq!(bottom(0.0, "1pt solid #000000"), [(0.0, 10.0)]);
        assert_eq!(bottom(0.0, "1pt double #000000").len(), 2);
        assert_eq!(bottom(0.0, "1pt dashed #000000"), [(0.0, 3.0), (5.0, 3.0)]);
        // The next cell carries on the pattern rather than starting it again.
        assert_eq!(
            bottom(12.0, "1pt dashed #000000"),
            [(12.0, 1.0), (15.0, 3.0), (20.0, 2.0)]
        );
        assert_eq!(bottom(0.0, "1pt dotted #000000").len(), 5);
        assert_eq!(bottom(0.0, "1pt dotted #000000")[1], (2.0, 1.0));
        assert!(bottom(0.0, "1pt none #000000").is_empty());
        assert_eq!(bottom(0.0, "1pt dash-dot #000000").len(), 2);
        assert_eq!(bottom(0.0, "1pt double-thin #000000").len(), 2);
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
