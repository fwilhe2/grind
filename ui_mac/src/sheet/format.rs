// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a Format command does to the grid (M7), and whether its menu item is ticked — portable.
//!
//! Nothing is decided here that `grind_sheet::format` has not already decided for every shell: a
//! toggle is one field of a `CellStyle` ([`Toggle`]), a colour is `coloured`, a number format is
//! a [`Preset`], a decimal step is `stepped`. The three rules that module carries hold here by
//! construction — **every read is of the active cell**, a write is read-change-write, and a style
//! that sets nothing is no style. What is left is the Mac's vocabulary: which [`Command`] is
//! which control, and what a tick in the menu means for each.

use grind_core::style::PALETTE;
use grind_sheet::format::{self, Preset, Toggle};
use grind_sheet::locale::Locale;
use grind_sheet::numfmt::Format;
use grind_sheet::style::CellStyle;
use grind_text::markdown::Emphasis;

use crate::menu::{Align, Command};
use crate::text::format::{Facts, points};

/// The active cell, read once for a command: its style, its format, and how many decimals the
/// text it shows has — what a decimal step starts a plain cell from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Active {
    pub style: CellStyle,
    pub format: Option<Format>,
    pub shown: u8,
}

/// What a command writes over the selection.
// One of these lives for the length of one command and is never stored, so boxing the style to
// even the variants out would cost an allocation per click to save bytes nobody keeps.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    /// `App::set_style` — `None` is no style at all.
    Style(Option<CellStyle>),
    /// `App::set_format` — `None` is *General*.
    Format(Option<Format>),
    /// Both of the core's "plain again" calls, in one undo step each.
    Clear,
}

/// The core's toggle a command is, when it is one.
fn toggle(command: Command) -> Option<Toggle> {
    match command {
        Command::Mark(Emphasis::Bold) => Some(Toggle::Bold),
        Command::Mark(Emphasis::Italic) => Some(Toggle::Italic),
        Command::Align(Align::Left) => Some(Toggle::AlignStart),
        Command::Align(Align::Center) => Some(Toggle::AlignCenter),
        Command::Align(Align::Right) => Some(Toggle::AlignEnd),
        Command::Wrap => Some(Toggle::Wrap),
        _ => None,
    }
}

/// A palette entry as the colour a document stores.
pub fn color(index: Option<u8>) -> Option<String> {
    index
        .and_then(|index| PALETTE.get(usize::from(index)))
        .map(|(_, hex)| (*hex).to_owned())
}

/// What `command` writes over the selection, given the active cell — `None` when it writes
/// nothing: a command that is not the grid's, or a step over a date, which has no decimals.
/// `locale` is the one a new format is written in when the cell has none of its own.
pub fn write(command: Command, active: &Active, locale: Option<Locale>) -> Option<Write> {
    if let Some(toggle) = toggle(command) {
        return Some(Write::Style(toggle.flipped(&active.style)));
    }
    match command {
        Command::TextColor(index) => Some(Write::Style(format::coloured(
            &active.style,
            false,
            color(index),
        ))),
        Command::Background(index) => Some(Write::Style(format::coloured(
            &active.style,
            true,
            color(index),
        ))),
        Command::Number(preset) => {
            Some(Write::Format(preset.format(active.format.as_ref(), locale)))
        }
        Command::Decimals(step) => {
            format::stepped(active.format.as_ref(), step, active.shown, locale)
                .map(|format| Write::Format(Some(format)))
        }
        Command::Borders(on) => Some(Write::Style(format::bordered(&active.style, on))),
        Command::ClearFormatting => Some(Write::Clear),
        _ => None,
    }
}

/// Whether the command's menu item is ticked for the active cell: a toggle that is on, the
/// number format the cell is, the colour it is.
pub fn checked(command: Command, active: &Active) -> bool {
    if let Some(toggle) = toggle(command) {
        return toggle.is_on(&active.style);
    }
    match command {
        Command::Number(preset) => Preset::of(active.format.as_ref()) == Some(preset),
        Command::TextColor(index) => active.style.color == color(index),
        Command::Background(index) => active.style.background == color(index),
        Command::Borders(true) => active.style.uniform_border().is_some(),
        _ => false,
    }
}

/// What the font panel's answer writes over a cell styled `style`: its size and its weight and
/// slant where they changed, and nothing where nothing did — `None` then. A family is the one
/// thing the panel can choose that a `CellStyle` cannot hold, and is left out rather than
/// approximated (a named gap in `doc/macos-shell.md`).
pub fn font_restyle(style: &CellStyle, before: &Facts, after: &Facts) -> Option<Option<CellStyle>> {
    let size = (points(after.size) != points(before.size)).then(|| points(after.size));
    let bold = (after.bold != before.bold).then_some(after.bold);
    let italic = (after.italic != before.italic).then_some(after.italic);
    if size.is_none() && bold.is_none() && italic.is_none() {
        return None;
    }
    let mut restyled = style.clone();
    if let Some(size) = size {
        restyled.font_size = Some(size);
    }
    if let Some(bold) = bold {
        restyled = Toggle::Bold.set(&restyled, bold).unwrap_or_default();
    }
    if let Some(italic) = italic {
        restyled = Toggle::Italic.set(&restyled, italic).unwrap_or_default();
    }
    Some((!restyled.is_plain()).then_some(restyled))
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::numfmt::{self, Kind};

    fn bold() -> Active {
        Active {
            style: CellStyle {
                font_weight: Some("bold".into()),
                ..CellStyle::default()
            },
            ..Active::default()
        }
    }

    #[test]
    fn bold_flips_the_active_cell_and_is_ticked_when_it_is_on() {
        let plain = Active::default();
        let command = Command::Mark(Emphasis::Bold);
        assert_eq!(
            write(command, &plain, None),
            Some(Write::Style(bold().style.into()))
        );
        assert!(checked(command, &bold()));
        assert_eq!(
            write(command, &bold(), None),
            Some(Write::Style(None)),
            "off leaves no style"
        );
        assert_eq!(
            write(Command::Mark(Emphasis::Underline), &plain, None),
            None,
            "a cell has no underline"
        );
    }

    #[test]
    fn a_colour_is_the_palettes_and_automatic_removes_it() {
        let red = write(Command::Background(Some(7)), &bold(), None);
        let Some(Write::Style(Some(style))) = red else {
            panic!("{red:?}")
        };
        assert_eq!(style.background.as_deref(), Some("#ff4136"));
        assert!(style.font_weight.is_some(), "bold as well, not instead");
        let active = Active {
            style,
            ..Active::default()
        };
        assert!(checked(Command::Background(Some(7)), &active));
        assert!(!checked(Command::Background(None), &active));
        assert!(checked(Command::TextColor(None), &active), "automatic");
    }

    /// A currency's decimals step from its own, and a plain cell's from what it shows.
    #[test]
    fn decimals_step_a_number_and_leave_a_date_alone() {
        let euro = Active {
            format: Some(numfmt::preset(Kind::Currency, 2, true, "€")),
            ..Active::default()
        };
        let Some(Write::Format(Some(more))) = write(Command::Decimals(1), &euro, None) else {
            panic!("a currency steps")
        };
        assert_eq!(more.preset_params(), (Kind::Currency, 3, true, "€".into()));
        assert!(checked(Command::Number(Preset::Currency), &euro));
        let date = Active {
            format: Some(numfmt::preset(Kind::Date, 0, false, "")),
            ..Active::default()
        };
        assert_eq!(write(Command::Decimals(-1), &date, None), None);
        let plain = Active {
            shown: 1,
            ..Active::default()
        };
        let Some(Write::Format(Some(two))) = write(Command::Decimals(1), &plain, None) else {
            panic!("a plain number steps from what it shows")
        };
        assert_eq!(two.preset_params().1, 2);
    }

    #[test]
    fn general_is_no_format_and_clear_is_both() {
        assert_eq!(
            write(Command::Number(Preset::General), &Active::default(), None),
            Some(Write::Format(None))
        );
        assert!(checked(
            Command::Number(Preset::General),
            &Active::default()
        ));
        assert_eq!(
            write(Command::ClearFormatting, &bold(), None),
            Some(Write::Clear)
        );
        assert_eq!(write(Command::GoTo, &bold(), None), None);
    }
    #[test]
    fn the_font_panel_sets_a_cells_size_and_weight_and_no_family() {
        let before = Facts {
            family: "Helvetica".into(),
            size: 12.0,
            bold: false,
            italic: false,
        };
        assert_eq!(font_restyle(&CellStyle::default(), &before, &before), None);
        let after = Facts {
            family: "Georgia".into(),
            size: 18.0,
            bold: true,
            ..before.clone()
        };
        let written = font_restyle(&CellStyle::default(), &before, &after)
            .unwrap()
            .unwrap();
        assert_eq!(written.font_size.as_deref(), Some("18pt"));
        assert!(Toggle::Bold.is_on(&written));
        let only_family = Facts {
            family: "Georgia".into(),
            ..before.clone()
        };
        assert_eq!(
            font_restyle(&CellStyle::default(), &before, &only_family),
            None,
            "a cell has no family to set"
        );
    }

    #[test]
    fn borders_go_on_and_off_leaving_the_rest_of_the_style() {
        let Some(Write::Style(Some(on))) = write(Command::Borders(true), &bold(), None) else {
            panic!("a style with borders");
        };
        assert_eq!(on.uniform_border(), Some(format::BORDER));
        assert_eq!(on.font_weight.as_deref(), Some("bold"));
        let active = Active {
            style: on,
            ..Active::default()
        };
        assert!(checked(Command::Borders(true), &active));
        assert_eq!(
            write(Command::Borders(false), &active, None),
            Some(Write::Style(Some(bold().style)))
        );
    }
}
