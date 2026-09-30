// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which font a piece of the page is set in, and how wide each block is measured — the page's
//! [`grind_text::Faces`] (M6).
//!
//! Two layers, the way every page in the suite has them. The **block's** face is its
//! [`Role`] — `grind_text::look`'s decision, so a heading is the same size here as in every
//! other window. The **run's** own direct formatting goes over it: `**bold**` inside a heading
//! is bold *and* a heading, a run's own family replaces the face's, and its own size replaces
//! the face's size. The answer is a [`Font`], which the page's `Metrics` measures with and
//! `text/paint.rs` hands the renderer — one resolution, so a run is measured and drawn in one
//! font (decision 4).
//!
//! **Sizes are points**, and a document's are honoured as written: a run at `14pt` is fourteen
//! points on this page, and one at `120%` is a fifth bigger than its block. The body is
//! [`BODY_PT`], LibreOffice's own default, so a document that states no sizes and one that states
//! LibreOffice's look the same.

use std::collections::HashMap;

use grind_core::layout::Metrics;
use grind_core::style::TextStyle;
use grind_sheet::look::{bold_weight, italic_style};
use grind_text::flow::{Across, Flow, Spacing};
use grind_text::look::Role;
use grind_text::{App, BlockKind};

use super::geom;
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
    }
}

/// Which of [`Role::ALL`] a role is — the index a shell's face per role is kept at.
pub fn slot(role: Role) -> usize {
    Role::ALL
        .iter()
        .position(|each| *each == role)
        .expect("Role::ALL lists every role")
}

/// The page's [`grind_text::Faces`]: each block measured in its role's face, at the width it
/// is set — its cell's inside a table, the column less its indent everywhere else.
///
/// Generic over the face so the arithmetic is tested here against [`grind_text::Fixed`]; the Mac
/// hands it one `metrics::Face` per role. The cell map is built before this is, because
/// `Faces::of` is called while `App` holds its read lock and so must not ask the document
/// anything (`grind_text::flow::across`).
pub struct Column<'a, M> {
    /// One face per role, in [`Role::ALL`]'s order.
    pub faces: &'a [M],
    pub width: f64,
    pub spacing: Spacing,
    pub across: &'a HashMap<usize, Across>,
}

impl<M: Metrics> Column<'_, M> {
    /// The face a block of `kind` and `style` is set in.
    pub fn face(&self, kind: &BlockKind, style: Option<&str>) -> &M {
        &self.faces[slot(Role::of(kind, style))]
    }
}

impl<M: Metrics> grind_text::Faces for Column<'_, M> {
    fn of(&self, index: usize, kind: &BlockKind, style: Option<&str>) -> (f32, &dyn Metrics) {
        let width = match self.across.get(&index) {
            Some(cell) => cell.width,
            None => self.spacing.measure(kind, self.width),
        };
        ((width as f32).max(1.0), self.face(kind, style))
    }
}

/// A document laid out on a page: the flow, the cell map the page's [`Column`] reads, and where
/// the text column is — one answer for the view and for `--render-to`, so a frame is the window's.
pub struct Laid {
    pub flow: Flow,
    pub across: HashMap<usize, Across>,
    /// Where the text column starts on the page, and how wide it is (`geom::column`).
    pub column: (f64, f64),
}

impl Laid {
    /// Nothing laid out yet — a page with no width.
    pub fn empty() -> Laid {
        Laid {
            flow: Flow::new(geom::TOP, 0.0),
            across: HashMap::new(),
            column: (0.0, 0.0),
        }
    }

    /// The page's `Faces` for this layout, over one face per role.
    pub fn faces<'a, M>(&'a self, faces: &'a [M]) -> Column<'a, M> {
        Column {
            faces,
            width: self.column.1,
            spacing: geom::spacing(),
            across: &self.across,
        }
    }
}

/// Lay `app` out on a page `width` wide, with one face per role in [`Role::ALL`]'s order.
pub fn lay_out<M: Metrics>(app: &App, faces: &[M], width: f64) -> Laid {
    let column = geom::column(width);
    let spacing = geom::spacing();
    let mut laid = Laid {
        flow: Flow::new(geom::TOP, column.1),
        across: grind_text::flow::across(app, column.1, &spacing),
        column,
    };
    laid.flow =
        grind_text::flow::lay_out(app, &laid.faces(faces), column.1, &spacing, &|_, _| None);
    laid
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
                family: Family::System
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

    #[test]
    fn every_role_has_a_slot_of_its_own() {
        let mut slots: Vec<usize> = Role::ALL.iter().map(|role| slot(*role)).collect();
        slots.dedup();
        assert_eq!(slots, (0..Role::ALL.len()).collect::<Vec<_>>());
    }

    /// A list item is measured narrower by its indent and a cell's block at its cell's width —
    /// the same numbers `grind_text::flow` places them with.
    #[test]
    fn a_block_is_measured_at_the_width_it_is_set() {
        use grind_text::Faces as _;
        let faces = vec![grind_text::Fixed; Role::ALL.len()];
        let spacing = crate::text::geom::spacing();
        let mut across = HashMap::new();
        across.insert(
            3,
            Across {
                left: 10.0,
                width: 100.0,
            },
        );
        let column = Column {
            faces: &faces,
            width: 400.0,
            spacing,
            across: &across,
        };
        assert_eq!(column.of(0, &BlockKind::Paragraph, None).0, 400.0);
        let item = column.of(1, &BlockKind::ListItem { depth: 2 }, None).0;
        assert_eq!(f64::from(item), 400.0 - 2.0 * spacing.indent);
        assert_eq!(column.of(3, &BlockKind::Paragraph, None).0, 100.0);
    }

    /// M6's exit criterion, portably: with every face `Fixed`, a paragraph breaks where the
    /// core breaks it for `grind text view --width`, and its slot is as tall as its lines.
    #[test]
    fn the_page_breaks_where_the_cli_does() {
        let app = App::new();
        app.set_text(
            0,
            "the quick brown fox jumps over the lazy dog, and then over it again and again",
        )
        .unwrap();
        app.insert(
            1,
            BlockKind::ListItem { depth: 1 },
            "a list item long enough to wrap at a measure less its indent",
        )
        .unwrap();
        let faces = vec![grind_text::Fixed; Role::ALL.len()];
        let width = 60.0 + 2.0 * geom::MARGIN;
        let laid = lay_out(&app, &faces, width);
        assert_eq!(laid.column, (geom::MARGIN, 60.0));
        let column = laid.faces(&faces);
        for block in 0..2 {
            let view = app.get_viewport(block..block + 1);
            let view = view.get(block).unwrap();
            use grind_text::Faces as _;
            let (measure, metrics) = column.of(block, &view.kind, view.style.as_deref());
            let page = app.layout_block(block, measure, metrics).unwrap();
            let cli = app
                .layout_block(block, measure, &grind_text::Fixed)
                .unwrap();
            let breaks = |layout: &grind_core::layout::Layout| {
                layout
                    .lines()
                    .iter()
                    .map(|line| (line.start, line.end))
                    .collect::<Vec<_>>()
            };
            assert_eq!(breaks(&page), breaks(&cli), "block {block}");
            assert!(breaks(&page).len() > 1, "block {block} wraps");
            let slot = laid.flow.slot(block).unwrap();
            assert_eq!(slot.height, f64::from(page.height()));
        }
    }
}
