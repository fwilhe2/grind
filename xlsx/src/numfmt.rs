// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel's number formats: a custom code is read by `grind_sheet::numfmt::code`, which `TEXT`
//! shares; a **built-in id** (ECMA-376 §18.8.30) is this filter's to translate, by meaning.

use grind_sheet::model::NumberKind;
pub use grind_sheet::numfmt::code::{Translation, Unspellable, classify, of_code};

/// What a built-in format id means, for the value reader (ECMA-376 §18.8.30):
///
/// - 14–17 and 22 are dates (`mm-dd-yy`, `d-mmm-yy`, `d-mmm`, `mmm-yy`, `m/d/yy h:mm`) — a point
///   in time.
/// - 18–21 and 45–47 are clock and elapsed times — a duration.
///
/// The East Asian ids (27–36, 50–58) are listed once per UI language: those that are a date, or a
/// time, in all four count so (`doc/xlsx-format.md` §3.1). Every other id is a number, text or
/// currency format.
#[must_use]
pub fn builtin(id: u32) -> Option<NumberKind> {
    match id {
        14..=17 | 22 => Some(NumberKind::Date),
        18..=21 | 45..=47 => Some(NumberKind::Time),
        _ => east_asian(id),
    }
}

/// What an East Asian id (27–36, 50–58) is in all four of zh-TW, zh-CN, ja-JP and ko-KR
/// (§18.8.30). 34, 35, 52, 53, 55 and 56 are a date in one and a time in another, so `None`.
fn east_asian(id: u32) -> Option<NumberKind> {
    match id {
        27..=31 | 36 | 50 | 51 | 54 | 57 | 58 => Some(NumberKind::Date),
        32 | 33 => Some(NumberKind::Time),
        _ => None,
    }
}

/// The code §18.8.30 assigns a built-in id — by meaning, not always its literal spelling. 14
/// (`mm-dd-yy`) and 22 (`m/d/yy h:mm`) are shown by Excel in the reader's locale, so they map to
/// the ISO spelling `numfmt::preset` uses rather than putting an American date in a German
/// document. 47 (`mmss.0`) gains the separator the oracle gives it.
///
/// The East Asian ids [`east_asian`] can tell apart are spelled the same way: each locale words
/// them differently, and the file does not say which.
///
/// The currency ids (5–8, 41–44) are absent on purpose, [`Unspellable::LocaleCurrency`]: their
/// symbol is the reader's locale and appears nowhere in the file.
fn builtin_code(id: u32) -> Option<&'static str> {
    Some(match (id, east_asian(id)) {
        (_, Some(NumberKind::Date)) => "yyyy-mm-dd",
        (32, _) => "h:mm",
        (33, _) => "h:mm:ss",
        _ => match id {
            1 => "0",
            2 => "0.00",
            3 => "#,##0",
            4 => "#,##0.00",
            9 => "0%",
            10 => "0.00%",
            11 => "0.00E+00",
            12 => "# ?/?",
            13 => "# ??/??",
            14 => "yyyy-mm-dd",
            15 => "d-mmm-yy",
            16 => "d-mmm",
            17 => "mmm-yy",
            18 => "h:mm AM/PM",
            19 => "h:mm:ss AM/PM",
            20 => "h:mm",
            21 => "h:mm:ss",
            22 => "yyyy-mm-dd hh:mm",
            37 => "#,##0 ;(#,##0)",
            38 => "#,##0 ;[Red](#,##0)",
            39 => "#,##0.00;(#,##0.00)",
            40 => "#,##0.00;[Red](#,##0.00)",
            45 => "mm:ss",
            46 => "[h]:mm:ss",
            47 => "mm:ss.0",
            48 => "##0.0E+0",
            49 => "@",
            _ => return None,
        },
    })
}

/// The format a built-in id carries. Id 0 is `General`, which is the *absence* of a format
/// and not a loss.
#[must_use]
pub fn of_builtin(id: u32) -> Translation {
    match id {
        0 => Translation::nothing(),
        5..=8 | 41..=44 => Translation::refused(None, once(Unspellable::LocaleCurrency)),
        _ => match builtin_code(id) {
            Some(code) => of_code(code),
            None => Translation::refused(builtin(id), once(Unspellable::UnknownBuiltin)),
        },
    }
}

fn once(what: Unspellable) -> Vec<Unspellable> {
    vec![what]
}

#[cfg(test)]
mod tests {
    use super::*;

    use grind_sheet::formula::date::DEFAULT_NULL_DATE as EPOCH;
    use grind_sheet::model::CellValue;

    #[test]
    fn the_built_in_date_and_time_ids() {
        assert_eq!(builtin(0), None, "General");
        assert_eq!(builtin(14), Some(NumberKind::Date));
        assert_eq!(
            builtin(22),
            Some(NumberKind::Date),
            "a date with a time is a date"
        );
        assert_eq!(builtin(18), Some(NumberKind::Time));
        assert_eq!(builtin(46), Some(NumberKind::Time), "[h]:mm:ss");
        assert_eq!(builtin(49), None, "@");
        // An East Asian id is what it is in all four of its UI languages, or nothing.
        assert_eq!(
            builtin(58),
            Some(NumberKind::Date),
            "ggge年m月d日, m月d日, mm-dd"
        );
        assert_eq!(builtin(33), Some(NumberKind::Time), "h時mm分ss秒");
        assert_eq!(builtin(34), None, "a time in Chinese, a date in Japanese");
    }

    #[test]
    fn a_built_in_id_is_read_by_meaning() {
        assert!(of_builtin(0).format.is_none(), "General is no format");
        assert!(of_builtin(0).lost.is_empty(), "and not a loss");
        assert_eq!(
            of_builtin(4)
                .format
                .expect("a format")
                .render(&CellValue::Number(1234.5678), EPOCH),
            "1,234.57"
        );
        // 14 is `mm-dd-yy` in the table and a locale date in Excel; ISO is the spelling that
        // means the same day in every country.
        assert_eq!(
            of_builtin(14)
                .format
                .expect("a format")
                .render(&CellValue::Number(45_368.0), EPOCH),
            "2024-03-17"
        );
        assert_eq!(
            of_builtin(15)
                .format
                .expect("a format")
                .render(&CellValue::Number(45_368.0), EPOCH),
            "17-Mar-24"
        );
        // The currency ids and the undocumented ones are named losses, not guesses.
        assert_eq!(of_builtin(5).lost, once(Unspellable::LocaleCurrency));
        assert_eq!(of_builtin(34).lost, once(Unspellable::UnknownBuiltin));
        assert_eq!(of_builtin(23).lost, once(Unspellable::UnknownBuiltin));
        let shown = |id| {
            of_builtin(id)
                .format
                .expect("a format")
                .render(&CellValue::Number(45_368.75), EPOCH)
        };
        assert_eq!(shown(31), "2024-03-17", "yyyy年m月d日 in every locale");
        assert_eq!(shown(32), "18:00");
        assert!(of_builtin(12).lost.contains(&Unspellable::Fraction));
    }
}
