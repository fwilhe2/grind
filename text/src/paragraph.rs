// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Paragraph styles, resolved for layout and never written (`doc/pdf-export.md`). A printed page
//! must look the way the document says, and in a Writer document that lives in the style chain —
//! `P1` (automatic) → `Standard` → the default paragraph style — so the properties a page needs
//! are read verbatim and [`resolve`] walks the chain. A save keeps the styles because the envelope
//! carries `office:styles` untouched.
//!
//! Inheritance is ODF's (`style:parent-style-name`, rng:12081): an unstated property is the
//! parent's, with the default style (rng:12049) under every chain. A percentage font size is a
//! share of the parent's resolved size — `130%` of a 14 pt `Heading` is 18.2 pt.

use std::collections::HashMap;

use grind_core::style::length_mm;

use crate::style::CharStyle;

/// The properties of a paragraph style that decide how its text is laid out on a page, ODF values
/// verbatim.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParagraphProps {
    // `style:paragraph-properties`
    pub margin_top: Option<String>,
    pub margin_bottom: Option<String>,
    pub margin_left: Option<String>,
    pub margin_right: Option<String>,
    pub text_indent: Option<String>,
    pub line_height: Option<String>,
    pub text_align: Option<String>,
    pub break_before: Option<String>,
    pub break_after: Option<String>,
    pub keep_with_next: Option<String>,
    pub widows: Option<String>,
    pub orphans: Option<String>,
    /// `style:master-page-name` on the style: the master page a paragraph in it starts. Empty is
    /// none.
    pub master_page: Option<String>,
    /// `style:page-number` on the style: `auto`, or the number the page it starts restarts the
    /// counting at (`doc/pdf-export.md`).
    pub page_number: Option<String>,
    /// `style:tab-stop-distance` — the interval between the default tab stops.
    pub tab_stop_distance: Option<String>,
    /// `style:tab-stops`, in order. A style stating its own replaces its parent's set whole.
    pub tab_stops: Option<Vec<TabStop>>,
    // `style:text-properties` — the four that change how wide text is
    pub font_family: Option<String>,
    pub font_size: Option<String>,
    pub font_weight: Option<String>,
    pub font_style: Option<String>,
    /// `fo:color`: the ink of the paragraph's text where a run states none.
    pub color: Option<String>,
    /// `fo:language` and `fo:country`: the language spelling is checked against
    /// (`doc/spelling.md`).
    pub language: Option<String>,
    pub country: Option<String>,
}

impl ParagraphProps {
    /// Every property `over` states replaces this one's, except a percentage size, a share of the
    /// size here.
    fn layer(&mut self, over: &ParagraphProps) {
        let size = match (over.font_size.as_deref(), self.font_size.as_deref()) {
            (Some(child), Some(base)) if child.trim().ends_with('%') => {
                relative_size(child, base).or_else(|| Some(child.to_owned()))
            }
            (Some(child), _) => Some(child.to_owned()),
            (None, base) => base.map(str::to_owned),
        };
        let fields: [(&mut Option<String>, &Option<String>); 18] = [
            (&mut self.color, &over.color),
            (&mut self.language, &over.language),
            (&mut self.country, &over.country),
            (&mut self.margin_top, &over.margin_top),
            (&mut self.margin_bottom, &over.margin_bottom),
            (&mut self.margin_left, &over.margin_left),
            (&mut self.margin_right, &over.margin_right),
            (&mut self.text_indent, &over.text_indent),
            (&mut self.line_height, &over.line_height),
            (&mut self.text_align, &over.text_align),
            (&mut self.break_before, &over.break_before),
            (&mut self.break_after, &over.break_after),
            (&mut self.keep_with_next, &over.keep_with_next),
            (&mut self.widows, &over.widows),
            (&mut self.orphans, &over.orphans),
            (&mut self.font_family, &over.font_family),
            (&mut self.font_weight, &over.font_weight),
            (&mut self.font_style, &over.font_style),
        ];
        for (mine, theirs) in fields {
            if theirs.is_some() {
                mine.clone_from(theirs);
            }
        }
        if over.master_page.is_some() {
            self.master_page.clone_from(&over.master_page);
        }
        if over.page_number.is_some() {
            self.page_number.clone_from(&over.page_number);
        }
        if over.tab_stop_distance.is_some() {
            self.tab_stop_distance.clone_from(&over.tab_stop_distance);
        }
        if over.tab_stops.is_some() {
            self.tab_stops.clone_from(&over.tab_stops);
        }
        self.font_size = size;
    }
}

/// One `style:tab-stop` (rng:13905), its attributes verbatim.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabStop {
    /// `style:position` — a length from the paragraph's left edge.
    pub position: String,
    /// `style:type` — `left` (the default), `center`, `right` or `char`.
    pub kind: Option<String>,
    /// `style:char` — the character a `char` stop lines up, `.` when unstated.
    pub char: Option<String>,
    /// `style:leader-text`, else what `style:leader-style` draws — the gap's filling.
    pub leader: Option<String>,
}

impl ParagraphProps {
    /// Where this paragraph's tabs go, in points (`doc/odt-format.md` §5c fact 13): its own stops,
    /// then the default interval; unreadable ones are left out.
    pub fn tabs(&self) -> grind_core::layout::Tabs {
        use grind_core::layout::{TabAlign, Tabs};
        let points = |length: &str| length_mm(length).map(|mm| (mm * 72.0 / 25.4) as f32);
        let mut stops: Vec<grind_core::layout::TabStop> = self
            .tab_stops
            .iter()
            .flatten()
            .filter_map(|stop| {
                let align = match stop.kind.as_deref() {
                    Some("right") => TabAlign::Right,
                    Some("center") => TabAlign::Center,
                    Some("char") => TabAlign::Decimal(
                        stop.char
                            .as_deref()
                            .and_then(|c| c.chars().next())
                            .unwrap_or('.'),
                    ),
                    _ => TabAlign::Left,
                };
                Some(grind_core::layout::TabStop {
                    position: points(&stop.position)?,
                    align,
                    leader: stop.leader.as_deref().and_then(|l| l.chars().next()),
                })
            })
            .collect();
        stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        Tabs {
            stops,
            interval: self
                .tab_stop_distance
                .as_deref()
                .and_then(points)
                .filter(|p| *p > 0.0)
                .unwrap_or(0.0),
        }
    }
}

/// `percent` (`"130%"`) of `base` (`"14pt"` or any ODF length) as points, or `None` when either is
/// unreadable.
fn relative_size(percent: &str, base: &str) -> Option<String> {
    let share = percent
        .trim()
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()?
        / 100.0;
    let points = length_mm(base)? * 72.0 / 25.4 * share;
    let rounded = (points * 1000.0).round() / 1000.0;
    Some(format!("{rounded}pt"))
}

/// A block's paragraph style, resolved, and whether the document declares the style it names: a
/// page sets a block in a declared style by that style, and otherwise by the screen's faces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub props: ParagraphProps,
    pub declared: bool,
}

/// One `style:style` of family `paragraph`, as the document declared it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParagraphStyle {
    pub parent: Option<String>,
    pub props: ParagraphProps,
    /// Declared in `office:automatic-styles` — direct formatting, whose indents outrank its list
    /// level's where a named style's do not (`doc/odt-format.md` §5c fact 22).
    pub automatic: bool,
}

/// One named character style (`style:style style:family="text"` in `office:styles`): what it
/// formats and the style it inherits from. Read for showing, never written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamedChar {
    pub parent: Option<String>,
    pub props: CharStyle,
}

/// What a run looks like: its named character styles (`names`, outermost first, space-separated),
/// each down its chain, with the run's direct formatting over them.
pub fn shown(
    styles: &HashMap<String, NamedChar>,
    names: Option<&str>,
    direct: &CharStyle,
) -> CharStyle {
    let mut out = CharStyle::default();
    for name in names
        .unwrap_or_default()
        .split(' ')
        .filter(|n| !n.is_empty())
    {
        let mut chain: Vec<&NamedChar> = Vec::new();
        let mut at = Some(name);
        while let Some(style) = at.and_then(|n| styles.get(n)) {
            if chain.len() == MAX_DEPTH || chain.iter().any(|seen| std::ptr::eq(*seen, style)) {
                break;
            }
            chain.push(style);
            at = style.parent.as_deref();
        }
        for style in chain.iter().rev() {
            out.layer(&style.props);
        }
    }
    out.layer(direct);
    out
}

/// How deep a chain is followed before it counts as a cycle — far past Writer's deepest, four.
const MAX_DEPTH: usize = 32;

/// The properties of a paragraph styled `name`: the default style's, then each style from the
/// chain's root down to `name` laid over them. An undeclared name or a looping chain stops where
/// it stops (R5).
pub fn resolve(
    styles: &HashMap<String, ParagraphStyle>,
    default: &ParagraphProps,
    name: Option<&str>,
) -> ParagraphProps {
    let mut chain: Vec<&ParagraphStyle> = Vec::new();
    let mut at = name;
    while let Some(style) = at.and_then(|n| styles.get(n)) {
        if chain.len() == MAX_DEPTH || chain.iter().any(|seen| std::ptr::eq(*seen, style)) {
            break;
        }
        chain.push(style);
        at = style.parent.as_deref();
    }
    let mut out = default.clone();
    for style in chain.iter().rev() {
        out.layer(&style.props);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(size: Option<&str>, align: Option<&str>) -> ParagraphProps {
        ParagraphProps {
            font_size: size.map(str::to_owned),
            text_align: align.map(str::to_owned),
            ..ParagraphProps::default()
        }
    }

    #[test]
    fn a_percentage_is_of_the_size_beneath_it() {
        assert_eq!(relative_size("130%", "14pt").as_deref(), Some("18.2pt"));
        assert_eq!(relative_size("50%", "1cm").as_deref(), Some("14.173pt"));
        assert_eq!(relative_size("lots", "14pt"), None);
    }

    #[test]
    fn a_child_states_what_it_states_and_inherits_the_rest() {
        let mut base = props(Some("12pt"), Some("start"));
        base.layer(&props(None, Some("center")));
        assert_eq!(base, props(Some("12pt"), Some("center")));
        base.layer(&props(Some("200%"), None));
        assert_eq!(base.font_size.as_deref(), Some("24pt"));
    }

    #[test]
    fn a_percentage_with_nothing_beneath_it_is_kept_as_it_is() {
        let mut base = ParagraphProps::default();
        base.layer(&props(Some("130%"), None));
        assert_eq!(base.font_size.as_deref(), Some("130%"));
    }
}
