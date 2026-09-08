// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a formatting bar can ask for, as a pure change to a [`CharStyle`].
//!
//! Built for `grind-text-gtk`'s formatting bar and hoisted here the day a second shell
//! (`grind-win32`) wanted the same answers — the same move `sheet/src/formula/assist.rs` made
//! for the sheet's autocomplete. [`Change`] is the whole vocabulary of "what a control does" as
//! a value rather than eight closures, so it is answerable — and answered, by `apply`'s own
//! test — with no display of any kind, and two shells writing through it can never disagree
//! about what Bold means.

use crate::markdown::MONOSPACE;
use crate::style::CharStyle;

/// What one control asks for, as a change to the selection's common formatting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Bold(bool),
    Italic(bool),
    Underline(bool),
    Strike(bool),
    /// The monospace family, which is what the `` `code` `` notation sets — a *family* and not
    /// a fifth boolean, because that is what the document stores ([`crate::markdown`]).
    Code(bool),
    /// `fo:font-family`, verbatim. `None` clears it, which is what "the document's own font"
    /// means: the attribute is absent rather than set to a name.
    Family(Option<String>),
    /// `fo:font-size`, an ODF length such as `14pt`.
    Size(Option<String>),
    Color(Option<String>),
    Highlight(Option<String>),
    /// Every property at once, off. The one-shot the four toggles could only approximate.
    Clear,
}

impl Change {
    /// Lay this change over what the selection already agrees about.
    ///
    /// Everything but [`Change::Clear`] touches exactly one property and leaves the other seven
    /// alone, which is what makes italicising a bold run leave it bold.
    pub fn apply(&self, style: &mut CharStyle) {
        match self {
            Change::Bold(on) => style.set_bold(*on),
            Change::Italic(on) => style.set_italic(*on),
            Change::Underline(on) => style.set_underlined(*on),
            Change::Strike(on) => style.set_struck(*on),
            Change::Code(on) => {
                style.font_family = on.then(|| MONOSPACE.to_owned());
            }
            Change::Family(family) => style.font_family = family.clone(),
            Change::Size(size) => style.font_size = size.clone(),
            Change::Color(color) => style.color = color.clone(),
            Change::Highlight(color) => style.background = color.clone(),
            Change::Clear => *style = CharStyle::default(),
        }
    }
}

/// The sizes a drop-down or picker offers, in points.
///
/// A word processor's usual ladder. It is a **default and not a limit**, the same stance
/// `grind_core::style::PALETTE` takes for colours: `grind text format --size 13pt` writes one
/// that is not on this list, the document keeps it verbatim, and [`sizes`] puts it in the list
/// for as long as it is selected.
pub const SIZES: [u32; 12] = [8, 9, 10, 11, 12, 14, 16, 18, 24, 32, 48, 72];

/// What "no value set at all" is called in a picker. Not a size and not a family: the attribute
/// is absent and the block's own face decides, which is a different thing from setting the same
/// value the face happens to have.
pub const DEFAULT: &str = "Default";

/// The sizes as a document spells them — `"11pt"` — with the document's own size first when it
/// is not one of them, so a selection at `13pt` has something to be selected, and [`DEFAULT`]
/// first of all.
pub fn sizes(current: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = SIZES.iter().map(|pt| format!("{pt}pt")).collect();
    if let Some(current) = current
        && !out.iter().any(|size| size == current)
    {
        out.insert(0, current.to_owned());
    }
    out.insert(0, DEFAULT.to_owned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_toggle_touches_only_its_own_property() {
        let bold = CharStyle {
            font_weight: Some("bold".to_owned()),
            ..Default::default()
        };
        let mut italicised = bold.clone();
        Change::Italic(true).apply(&mut italicised);
        assert!(italicised.is_bold());
        assert!(italicised.is_italic());
    }

    #[test]
    fn code_sets_the_monospace_family_rather_than_a_boolean() {
        let mut style = CharStyle::default();
        Change::Code(true).apply(&mut style);
        assert_eq!(style.font_family.as_deref(), Some(MONOSPACE));
        Change::Code(false).apply(&mut style);
        assert_eq!(style.font_family, None);
    }

    #[test]
    fn clear_drops_every_property_at_once() {
        let mut style = CharStyle {
            font_weight: Some("bold".to_owned()),
            color: Some("#ff0000".to_owned()),
            ..Default::default()
        };
        Change::Clear.apply(&mut style);
        assert_eq!(style, CharStyle::default());
    }

    #[test]
    fn sizes_puts_default_first_and_an_off_ladder_value_second() {
        let list = sizes(Some("13pt"));
        assert_eq!(list[0], DEFAULT);
        assert_eq!(list[1], "13pt");
        assert!(sizes(Some("12pt")).iter().filter(|s| *s == "12pt").count() == 1);
    }
}
