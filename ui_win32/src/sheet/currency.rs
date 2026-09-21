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
use grind_sheet::{MAX_COLS, MAX_ROWS, Pos};

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

/// The rectangle a format is written over: the selection, cut down to the part of the sheet in
/// use — `ui_sheet_gtk`'s `Grid::target`, for the reason `App::set_format` gives. A whole column
/// selected from its header is a million rows, and a format costs an entry per cell, so a click
/// on its header followed by `€` would otherwise be refused rather than doing what it means.
///
/// Only a whole row or column is cut: a rectangle somebody dragged out past the last value is
/// a request for exactly those cells, which may be about to be filled.
pub fn target(start: Pos, end: Pos, used: (u32, u32)) -> (Pos, Pos) {
    let (rows, cols) = used;
    let row = match start.row == 0 && end.row >= MAX_ROWS - 1 {
        true => end.row.min(rows.saturating_sub(1)).max(start.row),
        false => end.row,
    };
    let col = match start.col == 0 && end.col >= MAX_COLS - 1 {
        true => end.col.min(cols.saturating_sub(1)).max(start.col),
        false => end.col,
    };
    (start, Pos::new(row, col))
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

    #[test]
    fn only_a_whole_row_or_column_is_cut_to_the_sheet_in_use() {
        let used = (10, 4);
        let column = (Pos::new(0, 2), Pos::new(MAX_ROWS - 1, 2));
        assert_eq!(
            target(column.0, column.1, used),
            (Pos::new(0, 2), Pos::new(9, 2))
        );
        let row = (Pos::new(3, 0), Pos::new(3, MAX_COLS - 1));
        assert_eq!(target(row.0, row.1, used), (Pos::new(3, 0), Pos::new(3, 3)));
        let dragged = (Pos::new(0, 0), Pos::new(40, 7));
        assert_eq!(target(dragged.0, dragged.1, used), dragged);
        // An empty sheet still formats the cell the column starts at, rather than nothing.
        assert_eq!(
            target(column.0, column.1, (0, 0)),
            (Pos::new(0, 2), Pos::new(0, 2))
        );
    }
}
