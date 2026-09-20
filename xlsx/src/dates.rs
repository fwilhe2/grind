// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The two date systems, and the day in 1900 that never happened.
//!
//! `workbookPr/@date1904` picks the epoch, and ODF carries an epoch per document
//! (`table:null-date`), so the **1904** system is a translation of the document's null date
//! (`workbook::document`) and its serials pass through unchanged.
//!
//! The **1900** system counts serial 1 as 1900-01-01 *and* believes 1900 was a leap year —
//! Lotus 1-2-3's mistake, kept by Excel for compatibility — so its serial 60 is 1900-02-29, a
//! day no calendar has. ODF's default epoch is 1899-12-30, which is chosen precisely so that
//! the two agree from 1900-03-01 onwards. Below that they are one day apart:
//!
//! | Excel serial | Excel means | ODF serial |
//! |---|---|---|
//! | 0 ≤ s < 60 | 1900-01-00 … 1900-02-28 | s **+ 1** |
//! | 60 | 1900-02-29, which does not exist | 60 — see [`correct`] |
//! | s ≥ 61 | 1900-03-01 onwards | **unchanged** |
//!
//! **Only a date is corrected.** Excel stores a date as a plain number and only the format
//! says otherwise, so the same `<v>45368.75</v>` is a date under `yyyy-mm-dd` and a number
//! under `0.00` (`values/date-vs-number.xlsx`). And a *time* is not corrected either: a time
//! format makes the whole serial a duration, so 1 under `[h]:mm:ss` is `PT24H` in both
//! systems, with no epoch involved (`values/times.xlsx`).

/// The 1900 system's serial for a **date** cell, as an ODF serial against 1899-12-30.
///
/// Serial 60 is left where it is, which makes it 1900-02-28 — a day early, and the same day
/// as serial 59. There is no right answer for a day that does not exist; this one is the
/// oracle's (`doc/xlsx-format.md` §2.1 records the measurement), it keeps the mapping
/// monotone, and a person who wrote 1900-02-29 into a spreadsheet will see a date beside the
/// one they meant rather than a refusal. Negative serials are not dates in this system at all,
/// and are carried unchanged.
pub fn correct(serial: f64) -> f64 {
    if (0.0..60.0).contains(&serial) {
        serial + 1.0
    } else {
        serial
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::formula::date;

    /// An Excel 1900 serial, corrected, read back as a calendar day against ODF's own epoch.
    fn day(excel: f64) -> (i64, i64, i64) {
        date::ymd(correct(excel), date::DEFAULT_NULL_DATE)
    }

    /// The boundary every implementation of this gets wrong once: 59, 60 and 61.
    #[test]
    fn the_phantom_day_and_both_sides_of_it() {
        assert_eq!(day(1.0), (1900, 1, 1), "the epoch, as Excel displays it");
        assert_eq!(day(59.0), (1900, 2, 28), "the last real day before the bug");
        assert_eq!(day(60.0), (1900, 2, 28), "1900-02-29 has nowhere to go");
        assert_eq!(day(61.0), (1900, 3, 1), "the first day both agree");
        assert_eq!(day(25569.0), (1970, 1, 1));
        assert_eq!(day(2958465.0), (9999, 12, 31), "Excel's last day");
    }

    #[test]
    fn the_time_of_day_survives_the_correction() {
        assert_eq!(correct(59.5), 60.5);
        assert_eq!(correct(45368.75), 45368.75);
    }
}
