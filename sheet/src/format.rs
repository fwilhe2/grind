// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a formatting control reads off the active cell, and what it writes over the selection —
//! the half of a format strip that is not a toolbar.
//!
//! Every shell with a format strip had its own: `ui_win32/src/sheet/format.rs`, the GNOME
//! window's `formatting.rs` and the browser's tool row, and the Mac's toolbar would have been the
//! fourth (`doc/macos-shell.md`, M1). They had drifted where it shows. The browser un-bolding the
//! only bold cell left an empty `style:style` behind, read an `oblique` cell as not italic, and
//! stepped the decimals of a date — or of a format this build did not write — into a number
//! format nobody asked for. This is the Windows copy, whose rules were already the GNOME
//! window's, and it adds nothing to the core's vocabulary: a toggle is one field of
//! [`CellStyle`], a step is one [`numfmt::preset`], and a cell formatted from any shell and one
//! formatted by `grind sheet style` / `grind sheet format` with the same arguments are the same
//! document.
//!
//! Three rules carry the weight:
//!
//! * **Every read is of the active cell**, which is what every spreadsheet's toolbar shows and
//!   what makes a toggle over a mixed selection predictable — Bold on a range whose first cell is
//!   plain makes the whole range bold. The shell does the reading; everything here is handed the
//!   active cell's style or format.
//! * **A write is read, change one field, write** ([`restyled`]), because
//!   [`crate::App::set_style`] replaces rather than merges, deliberately.
//! * **A style that sets nothing is no style**, so un-bolding the only bold cell leaves no empty
//!   `style:style` behind.

use crate::locale::Locale;
use crate::numfmt::{self, Format, Kind};
use crate::style::CellStyle;

/// A formatting control that is on or off for a cell — one field of [`CellStyle`] with one value
/// that means "on".
///
/// The three alignments are one field with three answers: turning one on takes the others out,
/// and turning the one that is on off leaves the cell aligned by its content again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Toggle {
    Bold,
    Italic,
    /// `fo:wrap-option` — lines broken at the column's width.
    Wrap,
    AlignStart,
    AlignCenter,
    AlignEnd,
}

impl Toggle {
    pub const ALL: [Toggle; 6] = [
        Toggle::Bold,
        Toggle::Italic,
        Toggle::Wrap,
        Toggle::AlignStart,
        Toggle::AlignCenter,
        Toggle::AlignEnd,
    ];

    /// Whether the control is in for a cell styled `style`.
    ///
    /// §16.5's alignment values are relative to the writing direction, which is why `start` and
    /// `end` are what this build *writes*; `left` and `right` are what other producers write, and
    /// read the same here. `oblique` reads as italic for the same reason.
    pub fn is_on(self, style: &CellStyle) -> bool {
        let align = style.align.as_deref();
        match self {
            Toggle::Bold => style.font_weight.as_deref() == Some("bold"),
            Toggle::Italic => matches!(style.font_style.as_deref(), Some("italic" | "oblique")),
            Toggle::Wrap => style.wrap.as_deref() == Some("wrap"),
            Toggle::AlignStart => matches!(align, Some("start" | "left")),
            Toggle::AlignCenter => align == Some("center"),
            Toggle::AlignEnd => matches!(align, Some("end" | "right")),
        }
    }

    /// The style to write over a cell styled `style` to turn the control `on` or off: the one
    /// field set or removed, everything else kept, and `None` when nothing is left set — which is
    /// how the core spells no style.
    ///
    /// For a shell whose control already knows its new state, as a GTK toggle button does.
    pub fn set(self, style: &CellStyle, on: bool) -> Option<CellStyle> {
        let value = |value: &str| on.then(|| value.to_owned());
        restyled(style, |style| match self {
            Toggle::Bold => style.font_weight = value("bold"),
            Toggle::Italic => style.font_style = value("italic"),
            Toggle::Wrap => style.wrap = value("wrap"),
            Toggle::AlignStart => style.align = value("start"),
            Toggle::AlignCenter => style.align = value("center"),
            Toggle::AlignEnd => style.align = value("end"),
        })
    }

    /// The style a press writes — [`Toggle::set`] to the opposite of [`Toggle::is_on`].
    pub fn flipped(self, style: &CellStyle) -> Option<CellStyle> {
        self.set(style, !self.is_on(style))
    }
}

/// A colour written into one of the two colour fields: `background` picks the fill rather than
/// the text. `None` is *Automatic* — the attribute removed, so the theme decides again.
pub fn coloured(style: &CellStyle, background: bool, value: Option<String>) -> Option<CellStyle> {
    restyled(style, |style| match background {
        true => style.background = value,
        false => style.color = value,
    })
}

/// The border a *Borders* control draws — LibreOffice's own hairline, in the three-part form ODF
/// stores (`doc/ods-format.md` §5.4), so a box drawn from any shell is the box a document already
/// full of them has. Four shells spelled it four times before this.
pub const BORDER: &str = "0.06pt solid #000000";

/// A hairline round the cell (`on`), or every edge taken away — *All Borders* and *No Borders*.
pub fn bordered(style: &CellStyle, on: bool) -> Option<CellStyle> {
    restyled(style, |style| {
        style.set_border(on.then(|| BORDER.to_owned()))
    })
}

/// Read, change, and hand back what to write — `None` meaning no style at all.
///
/// The read-merge-write [`crate::App::set_style`]'s own documentation promises its callers in
/// place of a merge policy in the core.
pub fn restyled(style: &CellStyle, change: impl FnOnce(&mut CellStyle)) -> Option<CellStyle> {
    let mut style = style.clone();
    change(&mut style);
    (!style.is_plain()).then_some(style)
}

/// The most decimals the steps go to. Past this a number is not being formatted, it is being
/// printed at the float's own precision, which *General* already does.
pub const MAX_DECIMALS: u8 = 10;

/// One decimal more (`step > 0`) or fewer over a cell formatted `current` — Excel's *Increase
/// Decimal* and *Decrease Decimal*.
///
/// A cell that already shows a number keeps its kind, grouping, symbol and locale and changes
/// only its decimal count, clamped to `0..=MAX_DECIMALS`. A cell with **no format** starts from
/// what it *shows* — `shown` is the number of decimals in its displayed text
/// ([`decimals_shown`]), so `1.5` becomes `1.50` and `1`, which is what the button means when you
/// press it over a plain number. A date, a time, a boolean, text or a format this build did not
/// write has no decimals to step, and answers `None`: nothing to write. `locale` is used only
/// when the cell has no format to keep one from.
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
/// caller's fallback — so a step or a change of kind never moves a cell out of its own locale.
pub fn own_locale(current: Option<&Format>, fallback: Option<Locale>) -> Option<Locale> {
    match current.and_then(|f| f.locale.clone()) {
        Some(own) => Some(own),
        None => fallback,
    }
}

/// The number formats a picker offers, each one click: [`numfmt::preset`]'s kinds, plus
/// *General* for no format at all and *Date and Time*, which is [`numfmt::datetime_preset`].
///
/// The same nine, in the same order, as `grind sheet format`'s positional argument and every
/// shell's picker. What each is *called* is the shell's — the GNOME window writes sentence case
/// and the Mac and Windows title case — and what each *writes* is this. Hoisted out of
/// `ui_win32/src/sheet/format.rs` the day the Mac's toolbar would have been its second copy
/// (`doc/macos-shell.md`, M7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    General,
    Number,
    Percent,
    Currency,
    Date,
    DateTime,
    Time,
    Boolean,
    Text,
}

impl Preset {
    pub const ALL: [Preset; 9] = [
        Preset::General,
        Preset::Number,
        Preset::Percent,
        Preset::Currency,
        Preset::Date,
        Preset::DateTime,
        Preset::Time,
        Preset::Boolean,
        Preset::Text,
    ];

    /// The `numfmt` kind this preset builds, for the seven that are one.
    pub fn kind(self) -> Option<Kind> {
        match self {
            Preset::General | Preset::DateTime => None,
            Preset::Number => Some(Kind::Number),
            Preset::Percent => Some(Kind::Percentage),
            Preset::Currency => Some(Kind::Currency),
            Preset::Date => Some(Kind::Date),
            Preset::Time => Some(Kind::Time),
            Preset::Boolean => Some(Kind::Boolean),
            Preset::Text => Some(Kind::Text),
        }
    }

    /// Which preset a cell formatted `format` is — what a picker opens on. `None` for a format
    /// this build did not write (a document's own `#,##0.0 "kg"`), which no preset may claim.
    pub fn of(format: Option<&Format>) -> Option<Preset> {
        let Some(format) = format else {
            return Some(Preset::General);
        };
        if *format == numfmt::datetime_preset().in_locale(format.locale.clone()) {
            return Some(Preset::DateTime);
        }
        if !format.is_preset() {
            return None;
        }
        let kind = format.preset_params().0;
        Preset::ALL
            .into_iter()
            .find(|preset| preset.kind() == Some(kind))
    }

    /// The format choosing this preset writes over a cell formatted `current` — `None` for
    /// *General*, which is no format at all.
    ///
    /// **One click is the whole request**: there is no dialog of decimals, so a cell that already
    /// shows a number keeps its own decimals, grouping and locale where the new kind has them,
    /// and a currency keeps its symbol. Anything else gets what a spreadsheet's own buttons give
    /// — a number two decimals grouped, a percentage none, a currency two grouped in the default
    /// currency. `locale` is used only when the cell has no format to keep one from.
    pub fn format(self, current: Option<&Format>, locale: Option<Locale>) -> Option<Format> {
        if self == Preset::DateTime {
            return Some(numfmt::datetime_preset().in_locale(own_locale(current, locale)));
        }
        let kind = self.kind()?;
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
        // A percentage keeps the grouping it had, but one arriving from nothing gets none:
        // `1,250%` is a figure nobody formats on purpose.
        Some(
            numfmt::preset(kind, decimals, grouping, &symbol)
                .in_locale(own_locale(current, locale)),
        )
    }

    /// What a picker's face says about a cell formatted `format`: `123` for a plain number,
    /// `%`, the currency's own symbol, `Date`, `Time`, `T/F`, `Abc` — so a glance at a toolbar
    /// answers "what is this cell" without opening anything. A format this build did not write
    /// says `Custom`, which is true and is not a guess.
    pub fn face(format: Option<&Format>) -> String {
        let preset = match Preset::of(format) {
            None => return "Custom".to_owned(),
            Some(Preset::General) => return "General".to_owned(),
            Some(Preset::DateTime) => return "Date Time".to_owned(),
            Some(preset) => preset,
        };
        let symbol = format.map(|f| f.preset_params().3).unwrap_or_default();
        match preset {
            Preset::Number => "123".to_owned(),
            Preset::Percent => "%".to_owned(),
            Preset::Currency if symbol.is_empty() => numfmt::DEFAULT_CURRENCY.to_owned(),
            Preset::Currency => symbol,
            Preset::Date => "Date".to_owned(),
            Preset::Time => "Time".to_owned(),
            Preset::Boolean => "T/F".to_owned(),
            _ => "Abc".to_owned(),
        }
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
        assert!(!Toggle::Bold.is_on(&plain));
        assert_eq!(Toggle::Bold.flipped(&plain), Some(bold()));
        assert!(Toggle::Bold.is_on(&bold()));
        // Un-bolding the only thing a style set leaves no style — not an empty one.
        assert_eq!(Toggle::Bold.flipped(&bold()), None);
        assert_eq!(Toggle::Bold.set(&bold(), true), Some(bold()), "already on");
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
        let written = Toggle::Italic.flipped(&styled).unwrap();
        assert_eq!(written.font_style.as_deref(), Some("italic"));
        assert_eq!(written.color.as_deref(), Some("#ff0000"));
        assert_eq!(written.align.as_deref(), Some("center"));
    }

    /// What other producers write reads the same as what this build writes, so pressing Italic
    /// on an oblique cell takes it out rather than making it italic.
    #[test]
    fn oblique_is_italic_and_left_is_start() {
        let oblique = CellStyle {
            font_style: Some("oblique".into()),
            ..CellStyle::default()
        };
        assert!(Toggle::Italic.is_on(&oblique));
        assert_eq!(Toggle::Italic.flipped(&oblique), None);
        let left = CellStyle {
            align: Some("left".into()),
            ..CellStyle::default()
        };
        assert!(Toggle::AlignStart.is_on(&left));
        let right = CellStyle {
            align: Some("right".into()),
            ..CellStyle::default()
        };
        assert!(Toggle::AlignEnd.is_on(&right));
    }

    /// The three alignments are one field with three answers: pressing one takes the others out,
    /// and pressing the one that is in takes it out too.
    #[test]
    fn the_alignments_are_one_field() {
        let centred = Toggle::AlignCenter.flipped(&CellStyle::default()).unwrap();
        assert!(Toggle::AlignCenter.is_on(&centred));
        assert!(!Toggle::AlignStart.is_on(&centred));
        let right = Toggle::AlignEnd.flipped(&centred).unwrap();
        assert_eq!(right.align.as_deref(), Some("end"));
        assert!(!Toggle::AlignCenter.is_on(&right));
        assert_eq!(Toggle::AlignEnd.flipped(&right), None);
    }

    /// Every toggle turned on is on, and turned off again is no style at all.
    #[test]
    fn every_toggle_round_trips_to_no_style() {
        for toggle in Toggle::ALL {
            let on = toggle.set(&CellStyle::default(), true).expect("a style");
            assert!(toggle.is_on(&on), "{toggle:?}");
            assert_eq!(toggle.set(&on, false), None, "{toggle:?}");
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

    /// A step keeps the cell in the locale its format already states.
    #[test]
    fn a_step_keeps_the_cells_own_locale() {
        let de = Locale::parse("de-DE");
        let own = numfmt::preset(Kind::Number, 1, true, "").in_locale(de.clone());
        let next = stepped(Some(&own), 1, 0, Locale::parse("en-US")).unwrap();
        assert_eq!(next.locale, de);
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
    /// Every preset is one `numfmt` builds, and each reads back as the preset that wrote it.
    #[test]
    fn every_preset_reads_back_as_the_one_that_wrote_it() {
        assert_eq!(Preset::of(None), Some(Preset::General));
        assert_eq!(Preset::General.format(None, None), None);
        for preset in Preset::ALL.into_iter().skip(1) {
            let format = preset.format(None, None).expect("a format");
            assert_eq!(Preset::of(Some(&format)), Some(preset), "{preset:?}");
        }
    }

    #[test]
    fn a_number_keeps_its_own_digits_when_it_changes_preset() {
        let de = Locale::parse("de-DE");
        let own = numfmt::preset(Kind::Number, 3, false, "").in_locale(de.clone());
        let percent = Preset::Percent
            .format(Some(&own), Locale::parse("en-US"))
            .unwrap();
        assert_eq!(
            percent,
            numfmt::preset(Kind::Percentage, 3, false, "").in_locale(de)
        );
        let currency = Preset::Currency.format(Some(&percent), None).unwrap();
        assert_eq!(currency.preset_params().3, numfmt::DEFAULT_CURRENCY);
        let dollar = numfmt::preset(Kind::Currency, 0, true, "$");
        assert_eq!(
            Preset::Currency
                .format(Some(&dollar), None)
                .unwrap()
                .preset_params(),
            (Kind::Currency, 0, true, "$".to_owned())
        );
        let date = numfmt::preset(Kind::Date, 0, false, "");
        assert_eq!(
            Preset::Number
                .format(Some(&date), None)
                .unwrap()
                .preset_params(),
            (Kind::Number, 2, true, String::new())
        );
    }

    #[test]
    fn the_face_says_what_the_cell_is() {
        let faces: Vec<String> = Preset::ALL
            .iter()
            .map(|preset| Preset::face(preset.format(None, None).as_ref()))
            .collect();
        assert_eq!(
            faces,
            [
                "General",
                "123",
                "%",
                "€",
                "Date",
                "Date Time",
                "Time",
                "T/F",
                "Abc"
            ]
        );
        let pound = numfmt::preset(Kind::Currency, 2, true, "£");
        assert_eq!(Preset::face(Some(&pound)), "£");
    }

    #[test]
    fn borders_go_round_every_edge_and_back_to_no_style() {
        let boxed = bordered(&CellStyle::default(), true).expect("a style with borders");
        assert_eq!(boxed.uniform_border(), Some(BORDER));
        assert_eq!(bordered(&boxed, false), None, "nothing else set: no style");
    }
}
