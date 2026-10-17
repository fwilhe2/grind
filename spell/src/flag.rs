// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The little flag a spelling picker shows for each [`Setting`] — drawn, not an emoji, because
//! an emoji flag is a font's business and Windows has none. Decided here, as shapes, so every
//! client draws the same flag: the browser turns them into SVG ([`svg`]), a native window into
//! its own drawing calls.
//!
//! A flag is 30 × 20 units. A national flag is simplified to what reads at sixteen pixels high:
//! stripes, and the United States' canton without its stars. *All languages* is no nation's: a
//! light blue field with a white globe, after the United Nations' flag. *Off* is a grey field
//! struck through.

use crate::{Language, Setting};

/// The width and height every flag is drawn in.
pub const SIZE: (f64, f64) = (30.0, 20.0);

/// One piece of a flag, in flag units, painted in order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A filled rectangle.
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        color: &'static str,
    },
    /// A circle's outline.
    Ring {
        cx: f64,
        cy: f64,
        r: f64,
        width: f64,
        color: &'static str,
    },
    /// A straight stroke.
    Line {
        from: (f64, f64),
        to: (f64, f64),
        width: f64,
        color: &'static str,
    },
}

const fn rect(x: f64, y: f64, w: f64, h: f64, color: &'static str) -> Shape {
    Shape::Rect { x, y, w, h, color }
}

/// Stripes across the flag, top to bottom, each as tall as its share.
fn across(stripes: &[(f64, &'static str)]) -> Vec<Shape> {
    let total: f64 = stripes.iter().map(|(share, _)| share).sum();
    let mut y = 0.0;
    stripes
        .iter()
        .map(|(share, color)| {
            let h = SIZE.1 * share / total;
            let shape = rect(0.0, y, SIZE.0, h, color);
            y += h;
            shape
        })
        .collect()
}

/// Stripes down the flag, left to right.
fn down(stripes: &[(f64, &'static str)]) -> Vec<Shape> {
    let total: f64 = stripes.iter().map(|(share, _)| share).sum();
    let mut x = 0.0;
    stripes
        .iter()
        .map(|(share, color)| {
            let w = SIZE.0 * share / total;
            let shape = rect(x, 0.0, w, SIZE.1, color);
            x += w;
            shape
        })
        .collect()
}

/// The flag for a setting.
pub fn shapes(setting: Setting) -> Vec<Shape> {
    const WHITE: &str = "#ffffff";
    match setting {
        Setting::Automatic => {
            let globe = "#ffffff";
            vec![
                rect(0.0, 0.0, SIZE.0, SIZE.1, "#4b92db"),
                Shape::Ring {
                    cx: 15.0,
                    cy: 10.0,
                    r: 6.0,
                    width: 1.2,
                    color: globe,
                },
                Shape::Ring {
                    cx: 15.0,
                    cy: 10.0,
                    r: 3.0,
                    width: 0.9,
                    color: globe,
                },
                Shape::Line {
                    from: (9.0, 10.0),
                    to: (21.0, 10.0),
                    width: 0.9,
                    color: globe,
                },
                Shape::Line {
                    from: (15.0, 4.0),
                    to: (15.0, 16.0),
                    width: 0.9,
                    color: globe,
                },
            ]
        }
        Setting::Off => vec![
            rect(0.0, 0.0, SIZE.0, SIZE.1, "#c4c4c4"),
            Shape::Line {
                from: (3.0, 17.0),
                to: (27.0, 3.0),
                width: 2.0,
                color: "#6e6e6e",
            },
        ],
        Setting::In(language) => match language {
            Language::English => {
                let mut flag: Vec<Shape> = (0..13)
                    .map(|i| {
                        let color = if i % 2 == 0 { "#b22234" } else { WHITE };
                        rect(0.0, f64::from(i) * SIZE.1 / 13.0, SIZE.0, SIZE.1 / 13.0, color)
                    })
                    .collect();
                flag.push(rect(0.0, 0.0, 12.0, SIZE.1 * 7.0 / 13.0, "#3c3b6e"));
                flag
            }
            Language::German => across(&[(1.0, "#000000"), (1.0, "#dd0000"), (1.0, "#ffce00")]),
            Language::French => down(&[(1.0, "#0055a4"), (1.0, WHITE), (1.0, "#ef4135")]),
            Language::Spanish => across(&[(1.0, "#aa151b"), (2.0, "#f1bf00"), (1.0, "#aa151b")]),
            Language::Italian => down(&[(1.0, "#009246"), (1.0, WHITE), (1.0, "#ce2b37")]),
            Language::Portuguese => down(&[(2.0, "#046a38"), (3.0, "#da291c")]),
            Language::Polish => across(&[(1.0, WHITE), (1.0, "#dc143c")]),
        },
    }
}

/// The flag as an SVG element `height` pixels high, with a hairline edge so a white stripe does
/// not disappear into a white page.
pub fn svg(setting: Setting, height: f64) -> String {
    let (w, h) = SIZE;
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{}" height="{height}" aria-hidden="true">"#,
        height * w / h
    );
    for shape in shapes(setting) {
        out.push_str(&match shape {
            Shape::Rect { x, y, w, h, color } => {
                format!(r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{color}"/>"#)
            }
            Shape::Ring {
                cx,
                cy,
                r,
                width,
                color,
            } => format!(
                r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{color}" stroke-width="{width}"/>"#
            ),
            Shape::Line {
                from,
                to,
                width,
                color,
            } => format!(
                r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{color}" stroke-width="{width}" stroke-linecap="round"/>"#,
                from.0, from.1, to.0, to.1
            ),
        });
    }
    out.push_str(&format!(
        r##"<rect x="0.25" y="0.25" width="{}" height="{}" fill="none" stroke="#00000033" stroke-width="0.5"/></svg>"##,
        w - 0.5,
        h - 0.5
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_has_a_flag_that_fills_its_field() {
        for setting in Setting::ALL {
            let shapes = shapes(setting);
            assert!(!shapes.is_empty(), "{setting:?}");
            // The first shape, or the stripes together, cover the whole field.
            let covered: f64 = shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Rect { w, h, .. } => Some(w * h),
                    _ => None,
                })
                .sum();
            assert!(covered >= SIZE.0 * SIZE.1 - 1e-6, "{setting:?}");
            assert!(svg(setting, 16.0).starts_with("<svg"));
        }
    }

    #[test]
    fn all_languages_is_blue_and_no_nations() {
        match shapes(Setting::Automatic)[0] {
            Shape::Rect { color, .. } => assert_eq!(color, "#4b92db"),
            other => panic!("{other:?}"),
        }
    }
}
