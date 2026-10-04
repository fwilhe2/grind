// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which face a block is set in on paper, and at what width.
//!
//! Until paragraph styles are resolved (`doc/pdf-export.md` P5), a page prints **what the screen
//! shows**: the face of a block is [`Role::of`]'s, the same choice every shell makes, at
//! [`BODY_PT`] times that role's scale. A run's own direct formatting wins over its block's
//! face, property by property, as it does on screen.
//!
//! [`Column`] is the Mac's `text::face::Column` for paper: each block measured in its role's
//! face, at its cell's width inside a table and the column less its indent everywhere else.

use std::collections::HashMap;

use grind_core::layout::Metrics;
use grind_core::style::TextStyle;
use grind_text::BlockKind;
use grind_text::flow::{Across, Spacing};
use grind_text::look::Role;

use crate::metrics::Typesetter;

/// The body's size in points: Writer's default.
pub const BODY_PT: f64 = 12.0;

/// How a printed page is spaced, in points. Writer's numbers where it has an obvious one — a list
/// level indents a quarter inch (0.635 cm) — and round ones where it does not.
pub const SPACING: Spacing = Spacing {
    top: 0.0,
    gap: 6.0,
    heading: 12.0,
    indent: 18.0,
    cell_pad: 4.0,
};

/// One block's face on paper: a [`Metrics`] that sets a run in its own formatting where it has
/// some, in its paragraph style's where the run says nothing (`stated`), and in the role's
/// where neither does.
pub struct RoleFace<'a> {
    setter: &'a Typesetter,
    role: Role,
    /// What the block's paragraph style states — family, size, weight, slant — resolved down its
    /// chain (`grind_text::paragraph`). Empty for a role's own face.
    stated: TextStyle,
    /// The block's own line height, when its paragraph style states one.
    line: Option<LineRule>,
}

/// `fo:line-height` (`doc/odt-format.md` §5c, fact 10).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineRule {
    /// A share of the face's natural line, the extra below the text.
    Proportional(f32),
    /// A line exactly this many points tall.
    Fixed(f32),
}

impl LineRule {
    /// `"150%"` or a length; `normal`, anything unreadable or nothing at all is the natural line.
    pub fn parse(value: Option<&str>) -> Option<LineRule> {
        let value = value?.trim();
        match value.strip_suffix('%') {
            Some(share) => share
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|share| *share > 0.0)
                .map(|share| LineRule::Proportional(share / 100.0)),
            None => grind_core::style::length_mm(value)
                .filter(|mm| *mm > 0.0)
                .map(|mm| LineRule::Fixed((mm * 72.0 / 25.4) as f32)),
        }
    }
}

impl<'a> RoleFace<'a> {
    /// The same face with its paragraph style's line height.
    pub fn with_line(mut self, line: Option<LineRule>) -> Self {
        self.line = line;
        self
    }

    pub fn new(setter: &'a Typesetter, role: Role) -> Self {
        RoleFace::stating(setter, role, TextStyle::default())
    }

    /// A role's face under what a paragraph style states.
    pub fn stating(setter: &'a Typesetter, role: Role, stated: TextStyle) -> Self {
        RoleFace {
            setter,
            role,
            stated,
            line: None,
        }
    }

    pub fn setter(&self) -> &'a Typesetter {
        self.setter
    }

    /// What a run formatted `run` is really set in, in a block of this role: every property the
    /// run leaves unsaid filled in from the role, and a percentage size made absolute.
    pub fn style(&self, run: &TextStyle) -> TextStyle {
        let role = BODY_PT * self.role.scale();
        let base = match self.stated.font_size.as_deref().map(str::trim) {
            Some(value) if value.ends_with('%') => percent_of(value, role),
            Some(value) => grind_core::style::length_mm(value)
                .filter(|mm| *mm > 0.0)
                .map_or(role, |mm| mm * 72.0 / 25.4),
            None => role,
        };
        let size = match run.font_size.as_deref().map(str::trim) {
            Some(value) if value.ends_with('%') => percent_of(value, base),
            Some(value) if grind_core::style::length_mm(value).is_some_and(|mm| mm > 0.0) => {
                return self.with_role(run, value.to_owned());
            }
            _ => base,
        };
        self.with_role(run, points(size))
    }

    /// `run` with the role's family, weight and slant wherever it states none, at `size`.
    fn with_role(&self, run: &TextStyle, size: String) -> TextStyle {
        let stated = &self.stated;
        TextStyle {
            font_family: run
                .font_family
                .clone()
                .or_else(|| {
                    self.role
                        .mono()
                        .then(|| grind_text::markdown::MONOSPACE.to_owned())
                })
                .or_else(|| stated.font_family.clone()),
            font_size: Some(size),
            font_weight: run
                .font_weight
                .clone()
                .or_else(|| stated.font_weight.clone())
                .or_else(|| self.role.bold().then(|| "bold".to_owned())),
            font_style: run
                .font_style
                .clone()
                .or_else(|| stated.font_style.clone())
                .or_else(|| self.role.italic().then(|| "italic".to_owned())),
        }
    }
}

impl Metrics for RoleFace<'_> {
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
        self.setter.advances(text, &self.style(style), out)
    }

    fn line_height(&self, style: &TextStyle) -> f32 {
        let natural = self.setter.line_height(&self.style(style));
        match self.line {
            None => natural,
            Some(LineRule::Proportional(share)) => natural * share,
            Some(LineRule::Fixed(height)) => height,
        }
    }

    /// A proportional line keeps its text where a natural one has it; a fixed one shares the
    /// difference above and below the baseline as the face's ascent shares its natural line —
    /// the approximation `doc/odt-format.md` §5c fact 10 measures, within half a point.
    fn ascent(&self, style: &TextStyle) -> f32 {
        let set = self.style(style);
        let ascent = self.setter.ascent(&set);
        match self.line {
            Some(LineRule::Fixed(height)) => {
                let natural = self.setter.line_height(&set);
                ascent + (height - natural) * ascent / natural.max(f32::MIN_POSITIVE)
            }
            _ => ascent,
        }
    }
}

/// `value` (`"130%"`) of `base` points, or `base` itself when it is not a positive percentage.
fn percent_of(value: &str, base: f64) -> f64 {
    value
        .trim()
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|percent| *percent > 0.0)
        .map_or(base, |percent| base * percent / 100.0)
}

/// A size in points as an ODF length, without the float noise a scale leaves (`21.6pt`, not
/// `21.6000000000000014pt`).
fn points(size: f64) -> String {
    let rounded = (size * 1000.0).round() / 1000.0;
    format!("{rounded}pt")
}

/// One [`RoleFace`] per role, in [`Role::ALL`]'s order.
pub fn role_faces(setter: &Typesetter) -> Vec<RoleFace<'_>> {
    Role::ALL
        .iter()
        .map(|role| RoleFace::new(setter, *role))
        .collect()
}

/// The room a declared paragraph style gives its block on paper, in points: the flow's
/// [`grind_text::flow::Space`], and the right margin, which only narrows the measure.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Room {
    pub space: grind_text::flow::Space,
    pub right: f64,
}

/// The page's [`grind_text::Faces`]. The cell map is built before this is, because
/// `Faces::of` is called while `App` holds its read lock ([`grind_text::flow::across`]).
pub struct Column<'a> {
    pub faces: &'a [RoleFace<'a>],
    /// A face of its own for each block whose paragraph style states something about its text,
    /// by block index; every other block is set in its role's face.
    pub blocks: &'a HashMap<usize, RoleFace<'a>>,
    /// The space above and below each block whose declared paragraph style decides it, in
    /// points ([`grind_text::Faces::spacing`]); every other block is spaced as on screen.
    pub spacing: &'a HashMap<usize, Room>,
    /// What each block's paragraph style says about page breaks around it
    /// ([`grind_text::Faces::breaks`]); a block with nothing to say keeps the paginator's rules.
    pub breaks: &'a HashMap<usize, grind_text::page::Breaks>,
    /// Each block's first-line indent in points, where its paragraph style states one.
    pub indents: &'a HashMap<usize, f32>,
    /// The text area's width, in points.
    pub width: f64,
    pub across: &'a HashMap<usize, Across>,
}

impl Column<'_> {
    /// The face the block at `index`, of `kind` and `style`, is set in.
    pub fn face(&self, index: usize, kind: &BlockKind, style: Option<&str>) -> &RoleFace<'_> {
        if let Some(face) = self.blocks.get(&index) {
            return face;
        }
        let role = Role::of(kind, style);
        let slot = Role::ALL.iter().position(|each| *each == role).unwrap_or(0);
        &self.faces[slot]
    }
}

impl grind_text::Faces for Column<'_> {
    fn of(&self, index: usize, kind: &BlockKind, style: Option<&str>) -> (f32, &dyn Metrics) {
        let width = match (self.across.get(&index), self.spacing.get(&index)) {
            (Some(cell), _) => cell.width,
            (None, Some(room)) => SPACING.measure(kind, self.width) - room.space.left - room.right,
            (None, None) => SPACING.measure(kind, self.width),
        };
        ((width as f32).max(1.0), self.face(index, kind, style))
    }

    fn spacing(&self, index: usize) -> Option<grind_text::flow::Space> {
        self.spacing.get(&index).map(|room| room.space)
    }

    fn breaks(&self, index: usize) -> Option<grind_text::page::Breaks> {
        self.breaks.get(&index).copied()
    }

    fn first_indent(&self, index: usize) -> f32 {
        self.indents.get(&index).copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::Fonts;
    use grind_text::Faces;

    fn run(family: Option<&str>, size: Option<&str>, bold: bool) -> TextStyle {
        TextStyle {
            font_family: family.map(str::to_owned),
            font_size: size.map(str::to_owned),
            font_weight: bold.then(|| "bold".to_owned()),
            font_style: None,
        }
    }

    #[test]
    fn a_plain_run_takes_its_blocks_face() {
        let setter = Typesetter::new(Fonts::bundled());
        let body = RoleFace::new(&setter, Role::Body).style(&TextStyle::default());
        assert_eq!(body.font_size.as_deref(), Some("12pt"));
        assert_eq!(body.font_weight, None);
        let heading = RoleFace::new(&setter, Role::Heading(1)).style(&TextStyle::default());
        assert_eq!(heading.font_size.as_deref(), Some("21.6pt"));
        assert_eq!(heading.font_weight.as_deref(), Some("bold"));
        let subtitle = RoleFace::new(&setter, Role::Subtitle).style(&TextStyle::default());
        assert_eq!(subtitle.font_style.as_deref(), Some("italic"));
        let code = RoleFace::new(&setter, Role::Code).style(&TextStyle::default());
        assert_eq!(code.font_family.as_deref(), Some("monospace"));
    }

    #[test]
    fn a_runs_own_formatting_wins_property_by_property() {
        let setter = Typesetter::new(Fonts::bundled());
        let heading = RoleFace::new(&setter, Role::Heading(2));
        let set = heading.style(&run(Some("Liberation Sans"), Some("9pt"), false));
        assert_eq!(set.font_family.as_deref(), Some("Liberation Sans"));
        assert_eq!(set.font_size.as_deref(), Some("9pt"));
        assert_eq!(
            set.font_weight.as_deref(),
            Some("bold"),
            "the heading's weight stays"
        );
    }

    #[test]
    fn a_percentage_size_is_of_the_blocks_own_size() {
        let setter = Typesetter::new(Fonts::bundled());
        let set = RoleFace::new(&setter, Role::Heading(1)).style(&run(None, Some("50%"), false));
        assert_eq!(set.font_size.as_deref(), Some("10.8pt"));
    }

    #[test]
    fn a_heading_measures_wider_than_a_paragraph_of_the_same_text() {
        let setter = Typesetter::new(Fonts::bundled());
        let faces = role_faces(&setter);
        let width = |role: usize| {
            let mut out = Vec::new();
            faces[role].advances("Grind", &TextStyle::default(), &mut out);
            *out.last().unwrap()
        };
        assert!(width(1) > width(0) * 1.7);
    }

    /// `doc/odt-format.md` §5c fact 10: a proportional line height scales the line and leaves
    /// the text where it was, the extra below; a fixed one is the line's whole height.
    #[test]
    fn a_line_height_scales_or_fixes_the_line() {
        let setter = Typesetter::new(Fonts::bundled());
        let natural = RoleFace::new(&setter, Role::Body);
        let doubled =
            RoleFace::new(&setter, Role::Body).with_line(Some(LineRule::Proportional(2.0)));
        let fixed = RoleFace::new(&setter, Role::Body).with_line(Some(LineRule::Fixed(28.35)));
        let plain = TextStyle::default();
        assert!((doubled.line_height(&plain) - 2.0 * natural.line_height(&plain)).abs() < 1e-4);
        assert_eq!(doubled.ascent(&plain), natural.ascent(&plain));
        assert!((fixed.line_height(&plain) - 28.35).abs() < 1e-4);
        let ascent = fixed.ascent(&plain);
        assert!(ascent > 21.0 && ascent < 23.0, "22.14 measured: {ascent}");
        assert_eq!(
            LineRule::parse(Some("200%")),
            Some(LineRule::Proportional(2.0))
        );
        assert_eq!(LineRule::parse(Some("1in")), Some(LineRule::Fixed(72.0)));
        assert_eq!(LineRule::parse(Some("normal")), None);
        assert_eq!(LineRule::parse(None), None);
    }

    #[test]
    fn a_list_item_is_set_narrower_and_a_cell_at_its_own_width() {
        let setter = Typesetter::new(Fonts::bundled());
        let faces = role_faces(&setter);
        let across = HashMap::from([(
            3,
            Across {
                left: 100.0,
                width: 120.0,
            },
        )]);
        let blocks = HashMap::new();
        let spacing = HashMap::new();
        let breaks = HashMap::new();
        let indents = HashMap::new();
        let column = Column {
            indents: &indents,
            faces: &faces,
            blocks: &blocks,
            spacing: &spacing,
            breaks: &breaks,
            width: 480.0,
            across: &across,
        };
        assert_eq!(column.of(0, &BlockKind::Paragraph, None).0, 480.0);
        assert_eq!(
            column.of(1, &BlockKind::ListItem { depth: 2 }, None).0,
            444.0
        );
        assert_eq!(column.of(3, &BlockKind::Paragraph, None).0, 120.0);
    }
}
