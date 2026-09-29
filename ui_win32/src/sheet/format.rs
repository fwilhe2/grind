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

use grind_sheet::format::{Toggle, own_locale};
use grind_sheet::locale::Locale;
use grind_sheet::numfmt::{self, Format, Kind};
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

/// The number kinds the picker offers, as (label, kind): [`numfmt::preset`]'s kinds, plus
/// *General* for no format at all and *Date and Time*, which is `numfmt::datetime_preset`.
///
/// The same nine, in the same order, as `ui_sheet_gtk`'s menu and `grind sheet format`'s
/// positional argument.
pub const KINDS: [(&str, Option<Kind>); 9] = [
    ("General", None),
    ("Number", Some(Kind::Number)),
    ("Percent", Some(Kind::Percentage)),
    ("Currency", Some(Kind::Currency)),
    ("Date", Some(Kind::Date)),
    ("Date and Time", None),
    ("Time", Some(Kind::Time)),
    ("Boolean", Some(Kind::Boolean)),
    ("Text", Some(Kind::Text)),
];

/// Where *General* and *Date and Time* sit in [`KINDS`] — the two entries that are not a `Kind`.
const GENERAL: usize = 0;
const DATETIME: usize = 5;

/// Which of [`KINDS`] a cell formatted `format` is — the row the picker opens on. `None` for a
/// format this build did not write (a document's own `#,##0.0 "kg"`), which no row may claim.
pub fn kind_of(format: Option<&Format>) -> Option<usize> {
    let Some(format) = format else {
        return Some(GENERAL);
    };
    if *format == numfmt::datetime_preset().in_locale(format.locale.clone()) {
        return Some(DATETIME);
    }
    if !format.is_preset() {
        return None;
    }
    let kind = format.preset_params().0;
    KINDS.iter().position(|(_, k)| *k == Some(kind))
}

/// What the picker's face says about a cell formatted `format`: `123` for a plain number or none
/// at all, `%`, the currency's own symbol, `Date`, `Time`, `T/F`, `Abc` — `ui_sheet_gtk`'s faces,
/// so a glance at the strip answers "what is this cell" without opening anything. A format this
/// build did not write says `Custom`, which is true and is not a guess.
pub fn face(format: Option<&Format>) -> String {
    match kind_of(format) {
        Some(GENERAL) => "General".to_owned(),
        Some(DATETIME) => "Date Time".to_owned(),
        Some(_) => {
            let Some((kind, _, _, symbol)) = format.map(Format::preset_params) else {
                return "General".to_owned();
            };
            match kind {
                Kind::Number => "123".to_owned(),
                Kind::Percentage => "%".to_owned(),
                Kind::Currency => match symbol.as_str() {
                    "" => numfmt::DEFAULT_CURRENCY.to_owned(),
                    symbol => symbol.to_owned(),
                },
                Kind::Date => "Date".to_owned(),
                Kind::Time => "Time".to_owned(),
                Kind::Boolean => "T/F".to_owned(),
                Kind::Text => "Abc".to_owned(),
            }
        }
        None => "Custom".to_owned(),
    }
}

/// The format picking row `index` of [`KINDS`] writes over a cell formatted `current`.
///
/// **One click is the whole request**, as the currency items are (`sheet/currency.rs`): there is
/// no dialog of decimals here, so a cell that already shows a number keeps its own decimals,
/// grouping and locale where the new kind has them, and a currency keeps its symbol. Anything
/// else gets what a spreadsheet's own buttons give — a number two decimals grouped, a percentage
/// none, a currency two grouped in the default currency. `locale` is used only when the cell has
/// no format to keep one from, which is `currency::format_for`'s rule too.
///
/// `None` is *General* — no format at all — and so is an index past the end.
pub fn format_for_kind(
    current: Option<&Format>,
    index: usize,
    locale: Option<Locale>,
) -> Option<Format> {
    let (_, kind) = KINDS.get(index)?;
    if index == DATETIME {
        return Some(numfmt::datetime_preset().in_locale(own_locale(current, locale)));
    }
    let kind = (*kind)?;
    let numeric = current
        .filter(|f| f.is_preset())
        .map(Format::preset_params)
        .filter(|(k, ..)| matches!(k, Kind::Number | Kind::Percentage | Kind::Currency));
    let (decimals, grouping, symbol) = match (kind, numeric) {
        (Kind::Number | Kind::Percentage | Kind::Currency, Some((_, d, g, s))) => (d, g, s),
        (Kind::Number, None) => (2, true, String::new()),
        (Kind::Percentage, None) => (0, false, String::new()),
        (Kind::Currency, None) => (2, true, String::new()),
        _ => (0, false, String::new()),
    };
    let symbol = match (kind, symbol.as_str()) {
        (Kind::Currency, "") => numfmt::DEFAULT_CURRENCY.to_owned(),
        (Kind::Currency, _) => symbol,
        _ => String::new(),
    };
    // A percentage keeps the grouping it had, but one arriving from nothing gets none: `1,250%`
    // is a figure nobody formats on purpose.
    Some(numfmt::preset(kind, decimals, grouping, &symbol).in_locale(own_locale(current, locale)))
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

    /// The strip's vocabulary is the core's: every kind the picker offers is one `numfmt` builds,
    /// and each reads back as the row that wrote it.
    #[test]
    fn every_kind_reads_back_as_the_row_that_wrote_it() {
        assert_eq!(kind_of(None), Some(GENERAL));
        assert_eq!(format_for_kind(None, GENERAL, None), None);
        for (index, (label, _)) in KINDS.iter().enumerate().skip(1) {
            let format = format_for_kind(None, index, None).expect("a format");
            assert_eq!(kind_of(Some(&format)), Some(index), "{label}");
        }
        assert_eq!(
            format_for_kind(None, KINDS.len(), None),
            None,
            "past the end"
        );
    }

    #[test]
    fn a_number_keeps_its_own_digits_when_it_changes_kind() {
        let de = Locale::parse("de-DE");
        let own = numfmt::preset(Kind::Number, 3, false, "").in_locale(de.clone());
        let percent = format_for_kind(Some(&own), 2, Locale::parse("en-US")).unwrap();
        assert_eq!(
            percent,
            numfmt::preset(Kind::Percentage, 3, false, "").in_locale(de.clone())
        );
        let currency = format_for_kind(Some(&percent), 3, None).unwrap();
        assert_eq!(currency.preset_params().3, numfmt::DEFAULT_CURRENCY);
        assert_eq!(currency.preset_params().1, 3);
        // A currency keeps its symbol when it becomes a currency again.
        let dollar = numfmt::preset(Kind::Currency, 0, true, "$");
        assert_eq!(
            format_for_kind(Some(&dollar), 3, None)
                .unwrap()
                .preset_params(),
            (Kind::Currency, 0, true, "$".to_owned())
        );
        // A date has no digits to keep: a number arriving from one gets the defaults.
        let date = numfmt::preset(Kind::Date, 0, false, "");
        assert_eq!(
            format_for_kind(Some(&date), 1, None)
                .unwrap()
                .preset_params(),
            (Kind::Number, 2, true, String::new())
        );
    }

    #[test]
    fn the_face_says_what_the_cell_is() {
        assert_eq!(face(None), "General");
        let faces: Vec<String> = (1..KINDS.len())
            .map(|index| face(format_for_kind(None, index, None).as_ref()))
            .collect();
        assert_eq!(
            faces,
            ["123", "%", "€", "Date", "Date Time", "Time", "T/F", "Abc"]
        );
        let pound = numfmt::preset(Kind::Currency, 2, true, "£");
        assert_eq!(face(Some(&pound)), "£");
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
