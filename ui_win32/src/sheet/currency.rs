// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The currency picker: which format a click on `€`, `$` or `£` writes, and which of the three
//! the active cell already wears.
//!
//! **Portable, and tested on any host**, like every other file under `sheet/` but `draw`. The
//! three choices are [`numfmt::CURRENCIES`] rather than a list of this shell's own, so the
//! Windows window offers what the GNOME window's buttons, the browser's menu and the terminal's
//! `:format currency usd` offer, in the same order, the euro first ([`numfmt::DEFAULT_CURRENCY`]).
//! Every format written is one `numfmt::preset` builds, which is what makes it the same format
//! `grind sheet format … currency --symbol` would have written.
//!
//! One click is the whole request, as in the GTK picker — there is no dialog of decimals and
//! grouping in this shell, so a cell that is *already* a currency keeps its own decimals,
//! grouping and locale and only changes its symbol, and anything else gets two decimals, grouped
//! thousands and the environment's locale: what the GTK picker writes with its fields untouched.

use grind_sheet::locale::Locale;
use grind_sheet::numfmt::{self, Format, Kind};

/// The format a currency choice writes over a cell whose format is `current`.
///
/// `locale` is used only when the cell has no currency of its own to keep one from — the caller
/// passes `grind_sheet::locale::from_environment()`, the same fallback the GTK picker and the CLI
/// take when nobody named a locale.
pub fn format_for(current: Option<&Format>, symbol: &str, locale: Option<Locale>) -> Format {
    match current.filter(|f| f.is_preset()) {
        Some(format) if format.preset_params().0 == Kind::Currency => {
            let (_, decimals, grouping, _) = format.preset_params();
            numfmt::preset(Kind::Currency, decimals, grouping, symbol)
                .in_locale(format.locale.clone())
        }
        _ => numfmt::preset(Kind::Currency, 2, true, symbol).in_locale(locale),
    }
}

/// Which of [`numfmt::CURRENCIES`] a cell is formatted as, by index — the one menu item that
/// carries a check. `None` for a cell with no currency, and for one whose currency is none of
/// the three (a document's own `CHF`), which no item may claim.
pub fn chosen(current: Option<&Format>) -> Option<usize> {
    let format = current.filter(|f| f.is_preset())?;
    let (kind, _, _, symbol) = format.preset_params();
    if kind != Kind::Currency {
        return None;
    }
    numfmt::CURRENCIES.iter().position(|(s, _)| *s == symbol)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn currency(symbol: &str, decimals: u8, grouping: bool) -> Format {
        numfmt::preset(Kind::Currency, decimals, grouping, symbol)
    }

    #[test]
    fn a_plain_cell_gets_two_decimals_grouped() {
        assert_eq!(format_for(None, "$", None), currency("$", 2, true));
        let percent = numfmt::preset(Kind::Percentage, 1, false, "");
        assert_eq!(
            format_for(Some(&percent), "£", None),
            currency("£", 2, true)
        );
        let de = Locale::parse("de-DE");
        assert_eq!(
            format_for(None, "€", de.clone()),
            currency("€", 2, true).in_locale(de)
        );
    }

    /// Changing the symbol changes the symbol and nothing else a person chose.
    #[test]
    fn a_currency_keeps_its_own_digits_and_locale() {
        let de = Locale::parse("de-DE");
        let own = currency("€", 0, false).in_locale(de.clone());
        let other = Locale::parse("en-GB");
        assert_eq!(
            format_for(Some(&own), "$", other),
            currency("$", 0, false).in_locale(de)
        );
    }

    #[test]
    fn the_check_follows_the_cell() {
        assert_eq!(chosen(None), None);
        assert_eq!(chosen(Some(&currency("€", 2, true))), Some(0));
        assert_eq!(chosen(Some(&currency("$", 0, false))), Some(1));
        assert_eq!(chosen(Some(&currency("£", 2, true))), Some(2));
        assert_eq!(chosen(Some(&currency("CHF", 2, true))), None);
        let number = numfmt::preset(Kind::Number, 2, true, "");
        assert_eq!(chosen(Some(&number)), None);
        // Whatever it writes, it then reads back as the choice that wrote it.
        for (index, (symbol, _)) in numfmt::CURRENCIES.iter().enumerate() {
            assert_eq!(chosen(Some(&format_for(None, symbol, None))), Some(index));
        }
    }
}
