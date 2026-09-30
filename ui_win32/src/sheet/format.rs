// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The grid's **format strip** — what each control reads off the active cell, and what it writes
//! over the selection. `doc/windows-shell.md` decision 4's admission test, applied at last to the
//! pane it was written for: a control goes here when it **reads and writes a property of the
//! selection**, and `CellStyle` + `numfmt::Format` bound what that can be.
//!
//! **Portable, and tested on any host**, like everything under `sheet/` but `draw`. The window's
//! half is the drawing and one `App::set_style` or `App::set_format` per click.
//!
//! It maps onto the core's vocabulary and adds nothing, which is `ui_sheet_gtk/src/formatting.rs`'s
//! rule carried across: a bold button is one field of [`CellStyle`], the number picker offers
//! exactly the kinds `numfmt::preset` builds, and a cell formatted from here and one formatted by
//! `grind sheet style` / `grind sheet format` with the same arguments are the same document.
//!
//! What a toggle, a colour or a decimal step *writes* is `grind_sheet::format`'s — this file's
//! own half until the macOS shell would have made a fourth copy of it (`doc/macos-shell.md`, M1),
//! and the rules it carries (every read is of the active cell, a write is read-change-write, a
//! style that sets nothing is no style) are written down there. What is left here is this
//! window's strip: which controls it has, in which order and shape, what each is called, and the
//! number picker's rows and face.
//!
//! Two controls the GNOME strip has are **not** here, and each is a named gap in
//! `doc/windows-shell.md` rather than an omission: *Wrap*, because this window does not draw
//! wrapped text yet and a toggle with no visible effect is worse than none; and *Borders*, which
//! the GNOME strip lacks as well and this window does not draw either.

use grind_sheet::format::{Preset, Toggle};
use grind_sheet::locale::Locale;
use grind_sheet::numfmt::Format;
use grind_sheet::style::CellStyle;

pub use grind_sheet::format::{coloured, decimals_shown, stepped};

/// One control on the strip, in the order it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Bold,
    Italic,
    AlignLeft,
    AlignCenter,
    AlignRight,
    /// `fo:color` — the text's own colour.
    Color,
    /// `fo:background-color` — the cell's fill.
    Background,
    /// The number format, as a picker whose face says what the cell is formatted as.
    Number,
    FewerDecimals,
    MoreDecimals,
    /// Both "plain again" calls the core has — `set_style(None)` and `set_format(None)`.
    Clear,
}

/// How a control is drawn, which is also how wide it is: a toggle is square, a picker is a field
/// with a chevron, a swatch is square and shows a colour, and a button carries a short label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Toggle,
    Swatch,
    Picker,
    Button,
}

/// Every control, left to right, with its shape and whether a **new group** starts before it.
///
/// Five groups, and the grouping is the point (W10's rule for the text strip): the *weight* of
/// the text, its *alignment*, its *colours*, the *number* it shows, and Clear on its own because
/// it undoes all four. A separator is drawn wherever a group starts.
pub const CONTROLS: [(Control, Shape, bool); 11] = [
    (Control::Bold, Shape::Toggle, false),
    (Control::Italic, Shape::Toggle, false),
    (Control::AlignLeft, Shape::Toggle, true),
    (Control::AlignCenter, Shape::Toggle, false),
    (Control::AlignRight, Shape::Toggle, false),
    (Control::Color, Shape::Swatch, true),
    (Control::Background, Shape::Swatch, false),
    (Control::Number, Shape::Picker, true),
    (Control::FewerDecimals, Shape::Button, false),
    (Control::MoreDecimals, Shape::Button, false),
    (Control::Clear, Shape::Button, true),
];

impl Control {
    /// What the control is called — the title of the chooser it opens, and the name its menu item
    /// carries. Title case, as every command label in this shell is.
    pub fn name(self) -> &'static str {
        match self {
            Control::Bold => "Bold",
            Control::Italic => "Italic",
            Control::AlignLeft => "Align Left",
            Control::AlignCenter => "Center",
            Control::AlignRight => "Align Right",
            Control::Color => "Text Colour",
            Control::Background => "Cell Background",
            Control::Number => "Number Format",
            Control::FewerDecimals => "Decrease Decimals",
            Control::MoreDecimals => "Increase Decimals",
            Control::Clear => "Clear Formatting",
        }
    }

    /// The short label a [`Shape::Button`] carries. Plain ASCII on purpose — a rendered frame
    /// under Wine's substitute face is how this shell learned that anything else may come back as
    /// a missing-glyph box (`gdi::triangle_down`'s own comment).
    pub fn label(self) -> &'static str {
        match self {
            // A sign rather than Excel's `.00`/`.0` pair, which only says which is which to
            // somebody who already knows the icons: this says *fewer* and *more* by itself.
            Control::FewerDecimals => "-.0",
            Control::MoreDecimals => "+.0",
            Control::Clear => "Clear",
            _ => "",
        }
    }
}

impl Control {
    /// The core's toggle this control is, when it is one.
    pub fn toggle(self) -> Option<Toggle> {
        match self {
            Control::Bold => Some(Toggle::Bold),
            Control::Italic => Some(Toggle::Italic),
            Control::AlignLeft => Some(Toggle::AlignStart),
            Control::AlignCenter => Some(Toggle::AlignCenter),
            Control::AlignRight => Some(Toggle::AlignEnd),
            _ => None,
        }
    }
}

/// Whether a control is pressed in for a cell styled `style` — [`Toggle::is_on`], and never for
/// a control that is not a toggle.
pub fn pressed(style: &CellStyle, control: Control) -> bool {
    control.toggle().is_some_and(|toggle| toggle.is_on(style))
}

/// The style a toggle writes over a cell styled `style` — [`Toggle::flipped`], with `None` inside
/// when nothing is left set.
///
/// The outer `None` is for a control that is not a toggle, so a caller cannot write a style for a
/// click this function has no answer to.
pub fn toggled(style: &CellStyle, control: Control) -> Option<Option<CellStyle>> {
    control.toggle().map(|toggle| toggle.flipped(style))
}

/// The number formats the picker offers, as (label, preset) — `grind_sheet::format::Preset`,
/// whose order is `ui_sheet_gtk`'s menu and `grind sheet format`'s positional argument, in this
/// window's title case.
pub const KINDS: [(&str, Preset); 9] = [
    ("General", Preset::General),
    ("Number", Preset::Number),
    ("Percent", Preset::Percent),
    ("Currency", Preset::Currency),
    ("Date", Preset::Date),
    ("Date and Time", Preset::DateTime),
    ("Time", Preset::Time),
    ("Boolean", Preset::Boolean),
    ("Text", Preset::Text),
];

/// Which row of [`KINDS`] a cell formatted `format` is — the row the picker opens on. `None` for
/// a format this build did not write, which no row may claim ([`Preset::of`]).
pub fn kind_of(format: Option<&Format>) -> Option<usize> {
    let preset = Preset::of(format)?;
    KINDS.iter().position(|(_, p)| *p == preset)
}

/// What the picker's face says about a cell formatted `format` ([`Preset::face`]).
pub fn face(format: Option<&Format>) -> String {
    Preset::face(format)
}

/// The format picking row `index` of [`KINDS`] writes over a cell formatted `current`
/// ([`Preset::format`]) — `None` for *General*, and for an index past the end.
pub fn format_for_kind(
    current: Option<&Format>,
    index: usize,
    locale: Option<Locale>,
) -> Option<Format> {
    KINDS.get(index)?.1.format(current, locale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> CellStyle {
        CellStyle {
            font_weight: Some("bold".into()),
            ..CellStyle::default()
        }
    }

    #[test]
    fn a_toggle_reads_one_field_and_flips_it() {
        let plain = CellStyle::default();
        assert!(!pressed(&plain, Control::Bold));
        assert_eq!(toggled(&plain, Control::Bold), Some(Some(bold())));
        assert!(pressed(&bold(), Control::Bold));
        // Un-bolding the only thing a style set leaves no style — not an empty one.
        assert_eq!(toggled(&bold(), Control::Bold), Some(None));
    }

    /// The three alignments are one field with three answers: pressing one takes the others out,
    /// and pressing the one that is in takes it out too.
    #[test]
    fn the_alignments_are_one_field() {
        let centred = toggled(&CellStyle::default(), Control::AlignCenter)
            .unwrap()
            .unwrap();
        assert!(pressed(&centred, Control::AlignCenter));
        assert!(!pressed(&centred, Control::AlignLeft));
        let right = toggled(&centred, Control::AlignRight).unwrap().unwrap();
        assert_eq!(right.align.as_deref(), Some("end"));
        assert!(!pressed(&right, Control::AlignCenter));
        assert_eq!(toggled(&right, Control::AlignRight), Some(None));
        // What other producers write reads the same.
        let left = CellStyle {
            align: Some("left".into()),
            ..CellStyle::default()
        };
        assert!(pressed(&left, Control::AlignLeft));
    }

    #[test]
    fn only_a_toggle_answers_toggled() {
        for (control, shape, _) in CONTROLS {
            assert_eq!(
                toggled(&CellStyle::default(), control).is_some(),
                shape == Shape::Toggle,
                "{control:?}"
            );
        }
    }

    /// The picker's rows are the core's presets, all of them, in the core's order — and each
    /// reads back as the row that wrote it. What a preset writes is `grind_sheet::format`'s test.
    #[test]
    fn every_row_is_a_preset_and_reads_back_as_itself() {
        let presets: Vec<Preset> = KINDS.iter().map(|(_, preset)| *preset).collect();
        assert_eq!(presets, Preset::ALL);
        assert_eq!(kind_of(None), Some(0));
        for (index, (label, _)) in KINDS.iter().enumerate().skip(1) {
            let format = format_for_kind(None, index, None);
            assert_eq!(kind_of(format.as_ref()), Some(index), "{label}");
        }
        assert_eq!(
            format_for_kind(None, KINDS.len(), None),
            None,
            "past the end"
        );
        assert_eq!(face(None), "General");
    }

    /// Every control has a name, and the ones drawn as a labelled button have a label to draw.
    #[test]
    fn every_control_can_be_named_and_every_button_labelled() {
        for (control, shape, _) in CONTROLS {
            assert!(!control.name().is_empty());
            assert_eq!(
                !control.label().is_empty(),
                shape == Shape::Button,
                "{control:?}"
            );
        }
    }
}
