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
//! Three rules carry the weight, all of them the GNOME window's:
//!
//! * **Every read is of the active cell**, which is what every spreadsheet's toolbar shows and
//!   what makes a toggle over a mixed selection predictable — Bold on a range whose first cell is
//!   plain makes the whole range bold.
//! * **A write is read, change one field, write** ([`restyled`]), because `App::set_style`
//!   replaces rather than merges, deliberately.
//! * **A style that sets nothing is no style**, so un-bolding the only bold cell leaves no empty
//!   `style:style` behind.
//!
//! Two controls the GNOME strip has are **not** here, and each is a named gap in
//! `doc/windows-shell.md` rather than an omission: *Wrap*, because this window does not draw
//! wrapped text yet and a toggle with no visible effect is worse than none; and *Borders*, which
//! the GNOME strip lacks as well and this window does not draw either.

use grind_sheet::locale::Locale;
use grind_sheet::numfmt::{self, Format, Kind};
use grind_sheet::style::CellStyle;

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
    /// How this control is drawn — its row of [`CONTROLS`], so the shape is written down once.
    pub fn shape(self) -> Shape {
        CONTROLS
            .iter()
            .find(|(control, ..)| *control == self)
            .map_or(Shape::Button, |(_, shape, _)| *shape)
    }

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

/// Which of the toggles are pressed in for a cell styled `style` — Bold, Italic, then the three
/// alignments, in [`CONTROLS`]' order.
///
/// §16.5's alignment values are relative to the writing direction, which is why `start` and `end`
/// are what this build *writes*; `left` and `right` are what other producers write, and read the
/// same here.
pub fn pressed(style: &CellStyle, control: Control) -> bool {
    let align = style.align.as_deref();
    match control {
        Control::Bold => style.font_weight.as_deref() == Some("bold"),
        Control::Italic => matches!(style.font_style.as_deref(), Some("italic" | "oblique")),
        Control::AlignLeft => matches!(align, Some("start" | "left")),
        Control::AlignCenter => align == Some("center"),
        Control::AlignRight => matches!(align, Some("end" | "right")),
        _ => false,
    }
}

/// The style a toggle writes over a cell styled `style`: the one field flipped, everything else
/// kept, and `None` inside when nothing is left set — which is how the core spells no style.
///
/// The outer `None` is for a control that is not a toggle, so a caller cannot write a style for a
/// click this function has no answer to.
pub fn toggled(style: &CellStyle, control: Control) -> Option<Option<CellStyle>> {
    if control.shape() != Shape::Toggle {
        return None;
    }
    let on = !pressed(style, control);
    let value = |value: &str| on.then(|| value.to_owned());
    Some(restyled(style, |style| match control {
        Control::Bold => style.font_weight = value("bold"),
        Control::Italic => style.font_style = value("italic"),
        Control::AlignLeft => style.align = value("start"),
        Control::AlignCenter => style.align = value("center"),
        Control::AlignRight => style.align = value("end"),
        _ => {}
    }))
}

/// A colour written into one of the two colour fields: `background` picks the fill rather than
/// the text. `None` is *Automatic* — the attribute removed, so the theme decides again.
pub fn coloured(style: &CellStyle, background: bool, value: Option<String>) -> Option<CellStyle> {
    restyled(style, |style| match background {
        true => style.background = value,
        false => style.color = value,
    })
}

/// Read, change, and hand back what to write — `None` meaning no style at all.
fn restyled(style: &CellStyle, change: impl FnOnce(&mut CellStyle)) -> Option<CellStyle> {
    let mut style = style.clone();
    change(&mut style);
    (!style.is_plain()).then_some(style)
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

/// The most decimals the steps go to. Past this a number is not being formatted, it is being
/// printed at the float's own precision, which *General* already does.
pub const MAX_DECIMALS: u8 = 10;

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

/// One decimal more (`step > 0`) or fewer over a cell formatted `current` — Excel's *Increase
/// Decimal* and *Decrease Decimal*, and the web shell's more/fewer.
///
/// A cell that already shows a number keeps its kind, grouping, symbol and locale and changes only
/// its decimal count, clamped to `0..=MAX_DECIMALS`. A cell with **no format** starts from what it
/// *shows* — `shown` is the number of decimals in its displayed text, so `1.5` becomes `1.50` and
/// `1`, which is what the button means when you press it over a plain number. A date, a time, a
/// boolean, text or a format this build did not write has no decimals to step, and answers `None`:
/// nothing to write.
pub fn stepped(
    current: Option<&Format>,
    step: i8,
    shown: u8,
    locale: Option<Locale>,
) -> Option<Format> {
    let (kind, decimals, grouping, symbol) = match current {
        None => (Kind::Number, shown, false, String::new()),
        Some(format) if format.is_preset() => format.preset_params(),
        Some(_) => return None,
    };
    if !matches!(kind, Kind::Number | Kind::Percentage | Kind::Currency) {
        return None;
    }
    let next = (i16::from(decimals) + i16::from(step)).clamp(0, i16::from(MAX_DECIMALS)) as u8;
    if current.is_some() && next == decimals {
        return None;
    }
    Some(numfmt::preset(kind, next, grouping, &symbol).in_locale(own_locale(current, locale)))
}

/// How many decimals `text` shows, read the way `locale` spells a number — the digits after its
/// decimal character, up to the first that is not a digit. What [`stepped`] starts a plain cell
/// from. Zero for text with no decimal character, which is also the answer for a cell that is not
/// a number at all.
pub fn decimals_shown(text: &str, locale: Option<&Locale>) -> u8 {
    let point = locale.map_or('.', Locale::decimal);
    let Some((_, after)) = text.rsplit_once(point) else {
        return 0;
    };
    let digits = after.chars().take_while(char::is_ascii_digit).count();
    u8::try_from(digits)
        .unwrap_or(MAX_DECIMALS)
        .min(MAX_DECIMALS)
}

/// The locale a new format is written in: the one the cell's format already states, else the
/// caller's fallback.
fn own_locale(current: Option<&Format>, fallback: Option<Locale>) -> Option<Locale> {
    match current.and_then(|f| f.locale.clone()) {
        Some(own) => Some(own),
        None => fallback,
    }
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

    /// Bold as well, not bold instead: the write is a read-change-write, since `set_style`
    /// replaces.
    #[test]
    fn a_toggle_keeps_every_other_field() {
        let styled = CellStyle {
            color: Some("#ff0000".into()),
            align: Some("center".into()),
            ..CellStyle::default()
        };
        let written = toggled(&styled, Control::Italic).unwrap().unwrap();
        assert_eq!(written.font_style.as_deref(), Some("italic"));
        assert_eq!(written.color.as_deref(), Some("#ff0000"));
        assert_eq!(written.align.as_deref(), Some("center"));
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

    #[test]
    fn a_colour_goes_in_its_own_field_and_automatic_removes_it() {
        let red = coloured(&CellStyle::default(), false, Some("#ff0000".into())).unwrap();
        assert_eq!(red.color.as_deref(), Some("#ff0000"));
        let filled = coloured(&red, true, Some("#ffff00".into())).unwrap();
        assert_eq!(filled.background.as_deref(), Some("#ffff00"));
        assert_eq!(filled.color.as_deref(), Some("#ff0000"), "kept");
        assert_eq!(coloured(&red, false, None), None, "automatic: nothing left");
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

    #[test]
    fn decimals_step_one_at_a_time_and_stop_at_the_ends() {
        let two = numfmt::preset(Kind::Currency, 2, true, "$");
        let three = stepped(Some(&two), 1, 0, None).unwrap();
        assert_eq!(three.preset_params(), (Kind::Currency, 3, true, "$".into()));
        let one = stepped(Some(&two), -1, 0, None).unwrap();
        assert_eq!(one.preset_params().1, 1);
        let none = numfmt::preset(Kind::Number, 0, false, "");
        assert_eq!(stepped(Some(&none), -1, 0, None), None, "nothing to write");
        let most = numfmt::preset(Kind::Number, MAX_DECIMALS, false, "");
        assert_eq!(stepped(Some(&most), 1, 0, None), None);
    }

    /// Over a plain cell the step starts from what the cell shows, which is what the button means.
    #[test]
    fn a_plain_cell_steps_from_what_it_shows() {
        let more = stepped(None, 1, decimals_shown("1.5", None), None).unwrap();
        assert_eq!(
            more.preset_params(),
            (Kind::Number, 2, false, String::new())
        );
        let fewer = stepped(None, -1, decimals_shown("1.5", None), None).unwrap();
        assert_eq!(fewer.preset_params().1, 0);
        // A plain whole number stepped down is still a request — for no decimals, stated.
        assert!(stepped(None, -1, 0, None).is_some());
    }

    #[test]
    fn a_date_or_a_foreign_format_has_no_decimals_to_step() {
        let date = numfmt::preset(Kind::Date, 0, false, "");
        assert_eq!(stepped(Some(&date), 1, 0, None), None);
        let text = numfmt::preset(Kind::Text, 0, false, "");
        assert_eq!(stepped(Some(&text), -1, 0, None), None);
    }

    #[test]
    fn decimals_are_counted_the_way_the_locale_spells_a_number() {
        assert_eq!(decimals_shown("1,234.567", None), 3);
        assert_eq!(
            decimals_shown("1.234,5 €", Locale::parse("de-DE").as_ref()),
            1
        );
        assert_eq!(decimals_shown("12", None), 0);
        assert_eq!(decimals_shown("Groceries", None), 0);
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
