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
//!
//! [`here`] and [`apply`] are the other half of a bar — *where* a change is read from and
//! written to — hoisted the day the Mac's toolbar would have been the fourth copy of them
//! (`doc/macos-shell.md`, M7): `ui_text_gtk`, `ui_web` and `ui_win32` each had a `style_here`
//! and a write that held the style for the next character when nothing was selected.

use crate::markdown::{Emphasis, MONOSPACE};
use crate::style::CharStyle;
use crate::{App, Caret, Result};

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

impl Change {
    /// What pressing a toggle asks for over `style` — the selection's agreed formatting, or
    /// [`here`]'s: the emphasis on, unless every character already has it.
    pub fn toggle(emphasis: Emphasis, style: &CharStyle) -> Change {
        let on = !has(style, emphasis);
        match emphasis {
            Emphasis::Bold => Change::Bold(on),
            Emphasis::Italic => Change::Italic(on),
            Emphasis::Underline => Change::Underline(on),
            Emphasis::Strike => Change::Strike(on),
            Emphasis::Code => Change::Code(on),
        }
    }
}

/// Whether `style` carries `emphasis` — what a toggle shows pressed in.
pub fn has(style: &CharStyle, emphasis: Emphasis) -> bool {
    match emphasis {
        Emphasis::Bold => style.is_bold(),
        Emphasis::Italic => style.is_italic(),
        Emphasis::Underline => style.is_underlined(),
        Emphasis::Strike => style.is_struck(),
        Emphasis::Code => style.font_family.as_deref() == Some(MONOSPACE),
    }
}

/// What a formatting bar shows, and what its controls change: the formatting the `selection`
/// agrees about ([`App::char_style`]); with nothing selected, the style `pending` for the next
/// character typed — which a control or a closed markdown span left — and otherwise what the
/// character before the `caret` carries, which is what the next keystroke would produce.
///
/// A document has no style *at* an empty caret, only on either side of it, and the one already
/// typed is the one a bar showing the current state means.
pub fn here(
    app: &App,
    selection: Option<(Caret, Caret)>,
    caret: Caret,
    pending: Option<&CharStyle>,
) -> CharStyle {
    if let Some((from, to)) = selection {
        return app.char_style(from, to).unwrap_or_default();
    }
    if let Some(pending) = pending {
        return pending.clone();
    }
    app.char_style(caret, caret).unwrap_or_default()
}

/// Where a bar's change landed.
// One is returned per click on a control, and every shell matches it by value; boxing the
// formatting would cost each of them a dereference to save bytes nobody keeps.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Landed {
    /// Written over the selection — one undo step.
    Written,
    /// Nothing was selected, so nothing was written: this is the style the next character typed
    /// at the caret takes, for the shell to hold as the `resume` [`App::type_markdown`] carries.
    /// Bold, then type, is bold in every word processor, and moving the caret forgets it.
    Pending(CharStyle),
}

/// One control of a bar, applied: `change` laid over what [`here`] reads, and written over the
/// selection — or, with none, handed back as [`Landed::Pending`].
pub fn apply(
    app: &App,
    selection: Option<(Caret, Caret)>,
    caret: Caret,
    pending: Option<&CharStyle>,
    change: &Change,
) -> Result<Landed> {
    apply_all(app, selection, caret, pending, std::slice::from_ref(change))
}

/// Several changes as one — what a font panel's single answer is, when it changed a size and a
/// weight at once — laid over [`here`] in order and written once, so they are one undo step.
pub fn apply_all(
    app: &App,
    selection: Option<(Caret, Caret)>,
    caret: Caret,
    pending: Option<&CharStyle>,
    changes: &[Change],
) -> Result<Landed> {
    let mut style = here(app, selection, caret, pending);
    for change in changes {
        change.apply(&mut style);
    }
    match selection {
        Some((from, to)) => app
            .set_char_style(from, to, &style)
            .map(|_| Landed::Written),
        None => Ok(Landed::Pending(style)),
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
    fn app(text: &str) -> App {
        let app = App::new();
        app.set_text(0, text).unwrap();
        app
    }

    fn at(offset: usize) -> Caret {
        Caret { block: 0, offset }
    }

    /// Over a selection: what every character agrees about, and a write there.
    #[test]
    fn a_change_over_a_selection_is_written_there() {
        let app = app("plain words");
        let selection = Some((at(0), at(5)));
        assert!(!here(&app, selection, at(5), None).is_bold());
        let landed = apply(&app, selection, at(5), None, &Change::Bold(true)).unwrap();
        assert_eq!(landed, Landed::Written);
        assert!(app.char_style(at(0), at(5)).unwrap().is_bold());
        assert!(!app.char_style(at(5), at(11)).unwrap().is_bold());
        let toggled = Change::toggle(Emphasis::Bold, &here(&app, selection, at(5), None));
        assert_eq!(toggled, Change::Bold(false), "a second press takes it off");
    }

    /// With nothing selected nothing is written, and what comes back is what to type next: the
    /// character before the caret's formatting, or what was already pending, with the change.
    #[test]
    fn with_nothing_selected_a_change_waits_for_the_next_character() {
        let app = app("bold then");
        app.set_char_style(at(0), at(4), &Emphasis::Bold.style())
            .unwrap();
        assert!(
            here(&app, None, at(4), None).is_bold(),
            "the character behind"
        );
        assert!(!here(&app, None, at(9), None).is_bold());
        let landed = apply(&app, None, at(4), None, &Change::Italic(true)).unwrap();
        let Landed::Pending(style) = landed else {
            panic!("nothing selected, nothing written")
        };
        assert!(style.is_bold() && style.is_italic());
        assert!(!app.char_style(at(0), at(4)).unwrap().is_italic());
        let pending = CharStyle::default();
        assert!(
            !here(&app, None, at(4), Some(&pending)).is_bold(),
            "what is pending wins"
        );
    }

    #[test]
    fn several_changes_are_one_write() {
        let app = app("some words");
        let selection = Some((at(0), at(4)));
        let changes = [Change::Bold(true), Change::Size(Some("18pt".into()))];
        apply_all(&app, selection, at(4), None, &changes).unwrap();
        let style = app.char_style(at(0), at(4)).unwrap();
        assert!(style.is_bold());
        assert_eq!(style.font_size.as_deref(), Some("18pt"));
        assert!(app.undo());
        assert!(!app.char_style(at(0), at(4)).unwrap().is_bold(), "one step");
    }

    #[test]
    fn a_toggle_is_pressed_when_its_emphasis_is_there() {
        for emphasis in [
            Emphasis::Bold,
            Emphasis::Italic,
            Emphasis::Underline,
            Emphasis::Strike,
            Emphasis::Code,
        ] {
            let plain = CharStyle::default();
            assert!(!has(&plain, emphasis), "{emphasis:?}");
            let mut on = plain.clone();
            Change::toggle(emphasis, &plain).apply(&mut on);
            assert!(has(&on, emphasis), "{emphasis:?}");
            let mut off = on.clone();
            Change::toggle(emphasis, &on).apply(&mut off);
            assert!(!has(&off, emphasis), "{emphasis:?}");
            assert!(off.is_plain(), "{emphasis:?}: off leaves nothing behind");
        }
    }
}
