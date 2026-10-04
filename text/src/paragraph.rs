// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Paragraph styles, **resolved for layout and never authored** (`doc/pdf-export.md` P5).
//!
//! `doc/text-core.md` keeps a named style as a name and never interprets it, and for *writing*
//! that is unchanged: nothing here is ever written back, and a document saved after being read
//! keeps its styles because R6 carries `office:styles` out untouched. What changed is that the
//! printed page has to look the way the document says, and in a Writer document almost all of
//! that lives in the style chain — `P1` (automatic) → `Standard` → the default paragraph style —
//! rather than on the paragraph. So the properties a page needs are read, verbatim, and
//! [`resolve`] walks the chain.
//!
//! **Inheritance is ODF's** (`style:parent-style-name`, rng:12081): a property a style does not
//! state is its parent's, and the default style (`style:default-style`, rng:12049) is under every
//! chain. The one property that is not simply inherited is a **percentage font size**, which is a
//! share of the parent's resolved size — `130%` of a 14 pt `Heading` is 18.2 pt.

use std::collections::HashMap;

use grind_core::style::length_mm;

use crate::style::CharStyle;

/// The properties of a paragraph style that decide how its text is laid out on a page — ODF
/// values kept verbatim, as every style property in this suite is.
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
    // `style:text-properties` — the four that change how wide text is
    pub font_family: Option<String>,
    pub font_size: Option<String>,
    pub font_weight: Option<String>,
    pub font_style: Option<String>,
}

impl ParagraphProps {
    /// Every property `over` states replaces this one's — except a percentage size, which is a
    /// share of the size already here.
    fn layer(&mut self, over: &ParagraphProps) {
        let size = match (over.font_size.as_deref(), self.font_size.as_deref()) {
            (Some(child), Some(base)) if child.trim().ends_with('%') => {
                relative_size(child, base).or_else(|| Some(child.to_owned()))
            }
            (Some(child), _) => Some(child.to_owned()),
            (None, base) => base.map(str::to_owned),
        };
        let fields: [(&mut Option<String>, &Option<String>); 15] = [
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
        self.font_size = size;
    }
}

/// `percent` (`"130%"`) of `base` (`"14pt"`, or any ODF length), as points — `None` when either
/// is not a number this can read.
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

/// A block's paragraph style, resolved: its properties, and whether the style it names is one
/// the document declares at all — a page lays out a block whose style the document defines by
/// that style, and one whose style it does not (or which names none) by the screen's own faces.
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
}

/// One named character style (`style:style style:family="text"` in `office:styles`): what it
/// formats, and the style it inherits the rest from — read for showing and never written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamedChar {
    pub parent: Option<String>,
    pub props: CharStyle,
}

/// What a run looks like: the named character styles it carries (`names`, outermost first,
/// space-separated as the model keeps them), each down its own chain, and the run's direct
/// formatting over all of them.
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

/// How deep a chain is followed before it is taken to be a cycle — far past anything Writer
/// writes (its deepest is four), and short enough that a document naming each style as the
/// other's parent costs nothing.
const MAX_DEPTH: usize = 32;

/// The properties of a paragraph styled `name`: the default style's, then each style of the
/// chain from its root down to `name` layered over them. A name nothing declares, or a chain
/// that loops, stops where it stops — R5's tolerance rather than an error.
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
