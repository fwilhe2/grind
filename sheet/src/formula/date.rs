// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Serial dates and times — ODF 1.4 Part 4 §4.3.2 Time, §4.3.3 Date, and the epoch.
//!
//! "Date is a subtype of Number" (§4.3.3): a date is an `f64` counting days from the epoch and a
//! time is its fractional part, so there is no date type, and a calendar appears only when a
//! function asks for a year or an hour.
//!
//! The epoch is `HOST-NULL-DATE` (§3.4 item 8, `table:null-date`), by default 1899-12-30: §4.3.3
//! Note 3's recommendation for an evaluator that, like this one, treats 1900 as a non-leap year,
//! and LibreOffice's default.
//!
//! Text is parsed as ISO 8601 only: text→date via `DATEVALUE` is locale-dependent (§6.3.15), and
//! §3.4 Note 2 names ISO 8601 as the interoperable form. Guessing a locale would make
//! `DAY("01/02/03")` mean different days in different countries.

/// Days from 1970-01-01 in the proleptic Gregorian calendar — Howard Hinnant's `days_from_civil`
/// (public domain), exact for every year §4.3.3 asks for and for the corpus's negative ones
/// (`-0001-11-25`).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The inverse of [`days_from_civil`] — Hinnant's `civil_from_days`.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + i64::from(m <= 2), m, d)
}

/// 1899-12-30 as days from 1970-01-01: the ODF default epoch, and §4.3.3 Note 3's.
pub const DEFAULT_NULL_DATE: i64 = -25569;

/// `HOST-NULL-YEAR`'s default (§3.4 item 7), which is LibreOffice's: a two-digit year means
/// the year at or after 1930 ending in those digits, so 30..=99 are 1930..=1999 and 0..=29
/// are 2000..=2029.
pub const DEFAULT_NULL_YEAR: i64 = 1930;

/// The first year §7.4 "Year 1583" requires an evaluator to get right. Earlier years are
/// implementation-defined; LibreOffice returns `#VALUE!` (the corpus's `DATE(100;1;1)`). This
/// bounds `DATE`, not [`days_from_civil`], since the reader must still load `-0001-11-25`.
pub const MIN_YEAR: i64 = 1583;

/// Seconds in a day — the denominator of every Time (§4.3.2).
const DAY: f64 = 86_400.0;

/// A two-digit year as the four-digit one it stands for (§3.4 item 7). Only 0..=99 are two-digit:
/// `DATE(1899;1;1)` is 1899.
pub fn expand_year(y: i64, null_year: i64) -> i64 {
    match (0..=99).contains(&y) {
        true => null_year + (y - null_year).rem_euclid(100),
        false => y,
    }
}

/// The calendar date a serial number names, under the epoch `null_date`. `floor`, so a negative
/// serial names the day it falls in.
pub fn ymd(serial: f64, null_date: i64) -> (i64, i64, i64) {
    civil_from_days(serial.floor() as i64 + null_date)
}

/// The serial number of a calendar date. Out-of-range months and days roll over, as §6.10.2
/// requires of `DATE`: month 0 is the previous December, day 0 the last of the previous month.
pub fn serial(y: i64, m: i64, d: i64, null_date: i64) -> f64 {
    let (y, m) = (y + (m - 1).div_euclid(12), (m - 1).rem_euclid(12) + 1);
    (days_from_civil(y, m, 1) + d - 1 - null_date) as f64
}

/// The clock time a serial number names, as whole seconds since midnight (0..86400). Rounded once,
/// as §6.10.17 says `SECOND` rounds, so `HOUR`, `MINUTE` and `SECOND` all describe one clock and
/// 23:59:59.6 is not hour 23 with second 0.
pub fn seconds_of_day(serial: f64) -> i64 {
    let fraction = serial - serial.floor();
    (fraction * DAY).round() as i64 % 86_400
}

/// Day of the week, 0 = Sunday — the raw axis §6.10.21's Table 6 is a relabelling of.
pub fn weekday(serial: f64, null_date: i64) -> i64 {
    // 1970-01-01 was a Thursday, which is index 4 when Sunday is 0.
    (serial.floor() as i64 + null_date + 4).rem_euclid(7)
}

/// ISO 8601 date or dateTime text → a serial number (§6.3.15). Accepts `YYYY-MM-DD`, a full
/// `1899-12-26T12:00:00`, a leading `-` for a negative year, and a space where ISO writes `T`.
pub fn parse_date(s: &str, null_date: i64) -> Option<f64> {
    let s = s.trim();
    let (date, clock) = match s.find(['T', ' ']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (negative, digits) = match date.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, date),
    };
    let mut parts = digits.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    // A date in text does not roll over: that is `DATE`'s property (§6.10.2), not parsing's.
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if negative { -y } else { y };
    let day = (days_from_civil(y, m, d) - null_date) as f64;
    match clock {
        Some(t) => Some(day + parse_clock(t)?),
        None => Some(day),
    }
}

/// Time text → a fraction of a day (§6.3.16): an `xsd:duration` as `office:time-value` holds one
/// (`PT17H20M00S`), or a clock as a formula's text holds one (`"17:20:00"`).
pub fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim();
    let (sign, rest) = match s.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, s),
    };
    match rest.strip_prefix("PT") {
        Some(duration) => Some(sign * parse_duration(duration)?),
        // Not a duration: the clock form carries its own sign nowhere, so parse the whole
        // string rather than the half the `-` was stripped from.
        None => parse_clock(s),
    }
}

/// The `PT…H…M…S` body of an `xsd:duration`, in days. Anything else (`P1DT6H`) is `None` and the
/// cell keeps its text (§9).
fn parse_duration(s: &str) -> Option<f64> {
    let mut seconds = 0.0;
    let mut digits = String::new();
    for c in s.chars() {
        let scale = match c {
            'H' => 3600.0,
            'M' => 60.0,
            'S' => 1.0,
            _ => {
                digits.push(c);
                continue;
            }
        };
        seconds += digits.parse::<f64>().ok()? * scale;
        digits.clear();
    }
    digits.is_empty().then_some(seconds / DAY)
}

/// `HH:MM[:SS[.fff]]` → a fraction of a day. The colon is required: `HOUR("17")` must agree with
/// `HOUR(17)`, which is 0 (§4.3.2).
fn parse_clock(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':');
    let h: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = parts.next()?.trim().parse().ok()?;
    let sec: f64 = match parts.next() {
        Some(s) => s.trim().parse().ok()?,
        None => 0.0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((h * 3600.0 + m * 60.0 + sec) / DAY)
}

/// `office:date-value` text for a serial number. A fractional serial is a DateTime (§4.3.4) and
/// takes the combined spelling, as LibreOffice's files do.
pub fn format_date(serial: f64, null_date: i64) -> String {
    let (y, m, d) = ymd(serial, null_date);
    let date = format!("{y:04}-{m:02}-{d:02}");
    // `{:04}` pads the digits and leaves the sign outside, which is the ISO 8601 spelling
    // for a negative year (`-0001-11-25`) once the minus is accounted for.
    let date = match y < 0 {
        true => format!("-{:04}-{m:02}-{d:02}", -y),
        false => date,
    };
    // Only the time *within* the day: the date half already carries the days.
    let (h, m, s) = hms((serial - serial.floor()) * DAY);
    match (h, m, s) {
        (0, 0, 0.0) => date,
        _ => format!("{date}T{h:02}:{m:02}:{}", seconds_text(s)),
    }
}

/// `office:time-value` text — an `xsd:duration`, the form LibreOffice writes. Not wrapped at 24
/// hours: the corpus holds `PT33H45M00S`, and wrapping would lose a day on every save.
pub fn format_time(fraction: f64) -> String {
    let sign = if fraction < 0.0 { "-" } else { "" };
    let (h, m, s) = hms(fraction.abs() * DAY);
    format!("{sign}PT{h:02}H{m:02}M{}S", seconds_text(s))
}

/// A span of seconds split into hours, minutes and seconds that keep their fraction — unlike
/// [`seconds_of_day`], which rounds, and would drop the `.2` of `PT05H35M31.2S`.
fn hms(total: f64) -> (i64, i64, f64) {
    let h = (total / 3600.0).floor() as i64;
    let m = ((total - h as f64 * 3600.0) / 60.0).floor() as i64;
    (h, m, total - h as f64 * 3600.0 - m as f64 * 60.0)
}

/// Seconds as ODF writes them: two integer digits, and up to nine decimals only when there is a
/// fraction — finer than LibreOffice's 15 significant digits, so the fraction survives a round
/// trip.
fn seconds_text(seconds: f64) -> String {
    let text = format!("{seconds:012.9}");
    match text.split_once('.') {
        // Trim the fractional digits only. Trimming the whole string would turn `20.000`
        // into `2`.
        Some((whole, fraction)) => match fraction.trim_end_matches('0') {
            "" => whole.to_owned(),
            fraction => format!("{whole}.{fraction}"),
        },
        None => text,
    }
}

/// The current instant as a serial number, for `NOW` and `TODAY`.
///
/// ponytail: UTC, not local time (§6.10.16 says the current locale); `std` has no time zone. Off
/// by at most one zone offset. Give `Engine` a clock from the shell when one has a locale worth
/// respecting.
pub fn now(null_date: i64) -> f64 {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    seconds / DAY - null_date as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_epoch_is_1899_12_30() {
        // §4.3.3 Note 3; every other serial in these tests depends on it.
        assert_eq!(days_from_civil(1899, 12, 30), DEFAULT_NULL_DATE);
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn the_calendar_round_trips_across_four_centuries() {
        // Leap rules only diverge at century boundaries, so walking a range that spans
        // 1900 (not a leap year) and 2000 (one) is what actually exercises them.
        for z in -200_000..200_000 {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z, "{y}-{m}-{d}");
        }
    }

    #[test]
    fn nineteen_hundred_is_not_a_leap_year_and_two_thousand_is() {
        // 1900-02-29 does not exist: the day after 1900-02-28 is 1900-03-01 (Excel's bug, not
        // inherited).
        assert_eq!(
            days_from_civil(1900, 3, 1) - days_from_civil(1900, 2, 28),
            1
        );
        assert_eq!(
            days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28),
            2
        );
    }

    #[test]
    fn the_serials_libreoffice_cached_in_the_corpus_land_on_the_right_year() {
        // Straight from `functions/date_time/fods/year.fods`: YEAR(0) and YEAR(1) are 1899,
        // YEAR(2) is 1900. Anything that shifts the epoch by a day breaks exactly here.
        let e = DEFAULT_NULL_DATE;
        assert_eq!(ymd(0.0, e), (1899, 12, 30));
        assert_eq!(ymd(1.0, e), (1899, 12, 31));
        assert_eq!(ymd(2.0, e), (1900, 1, 1));
        // The corpus's `YEAR([.J4])` where J4 holds 33333.33 — it pins only the year, and
        // the day is what the calendar says.
        assert_eq!(ymd(33333.33, e), (1991, 4, 5));
    }

    #[test]
    fn out_of_range_months_and_days_roll_over() {
        // §6.10.2, and the corpus's `DATE(1983;0;31)` = 1982-12-31.
        let e = DEFAULT_NULL_DATE;
        assert_eq!(ymd(serial(1983, 0, 31, e), e), (1982, 12, 31));
        assert_eq!(ymd(serial(1983, 1, 31, e), e), (1983, 1, 31));
        assert_eq!(ymd(serial(2000, 13, 31, e), e), (2001, 1, 31));
        assert_eq!(ymd(serial(2000, 1, 0, e), e), (1999, 12, 31));
        // Day past the month's end carries into the next, which is the other half of the
        // sentence and the reason February needs no special case.
        assert_eq!(ymd(serial(2001, 2, 29, e), e), (2001, 3, 1));
        assert_eq!(ymd(serial(2000, 2, 29, e), e), (2000, 2, 29));
    }

    #[test]
    fn the_clock_is_read_to_the_nearest_second() {
        // §6.10.17: `SECOND` rounds rather than truncates, and the other two read the same
        // total, so 23:59:59.6 is one second short of a whole day rather than 23:59:59.
        assert_eq!(seconds_of_day(0.0), 0);
        assert_eq!(seconds_of_day(0.5), 43_200);
        assert_eq!(seconds_of_day(0.25), 21_600);
        // A day boundary reached by rounding wraps to 0 rather than reporting hour 24.
        assert_eq!(seconds_of_day(1.0 - 0.4 / DAY), 0);
        // The fraction is taken from the day the serial falls in, so a negative serial
        // still reads as a clock rather than running backwards.
        assert_eq!(seconds_of_day(-1.5), 43_200);
    }

    #[test]
    fn weekdays_are_counted_from_sunday() {
        let e = DEFAULT_NULL_DATE;
        // 2000-06-14 was a Wednesday; the corpus's WEEKDAY("2000-06-14") is 4 with Sunday
        // as day 1, which is index 3 here.
        assert_eq!(weekday(serial(2000, 6, 14, e), e), 3);
        assert_eq!(weekday(serial(2016, 7, 24, e), e), 0); // a Sunday
        assert_eq!(weekday(serial(1996, 7, 24, e), e), 3);
    }

    #[test]
    fn iso_text_parses_and_anything_else_does_not() {
        let e = DEFAULT_NULL_DATE;
        assert_eq!(parse_date("2000-06-14", e), Some(serial(2000, 6, 14, e)));
        assert_eq!(parse_date(" 1983-01-31 ", e), Some(serial(1983, 1, 31, e)));
        // The dateTime and negative-year forms, both taken from the corpus.
        assert_eq!(
            parse_date("1899-12-26T12:00:00", e),
            Some(serial(1899, 12, 26, e) + 0.5)
        );
        assert_eq!(parse_date("-0001-11-25", e), Some(serial(-1, 11, 25, e)));
        // A locale-shaped date is not guessed at — see the module docs.
        assert_eq!(parse_date("14/06/2000", e), None);
        assert_eq!(parse_date("2000-13-01", e), None);
        assert_eq!(parse_date("not a date", e), None);
    }

    #[test]
    fn both_time_spellings_parse() {
        assert_eq!(parse_time("17:20:00"), Some((17.0 * 60.0 + 20.0) / 1440.0));
        assert_eq!(
            parse_time("PT17H20M00S"),
            Some((17.0 * 60.0 + 20.0) / 1440.0)
        );
        assert_eq!(parse_time("12:00"), Some(0.5));
        assert_eq!(parse_time("PT00H00M08.25S"), Some(8.25 / DAY));
        assert_eq!(parse_time("-PT12H00M00S"), Some(-0.5));
        // A bare number is a number, not a o'clock.
        assert_eq!(parse_time("17"), None);
        assert_eq!(parse_time("P1DT6H"), None);
    }

    #[test]
    fn the_written_forms_read_back_as_themselves() {
        let e = DEFAULT_NULL_DATE;
        for (y, m, d) in [(1983, 1, 31), (1899, 12, 30), (2000, 2, 29), (-1, 11, 25)] {
            let s = serial(y, m, d, e);
            assert_eq!(parse_date(&format_date(s, e), e), Some(s), "{y}-{m}-{d}");
        }
        let noon = serial(1899, 12, 26, e) + 0.5;
        assert_eq!(format_date(noon, e), "1899-12-26T12:00:00");
        assert_eq!(parse_date(&format_date(noon, e), e), Some(noon));
        assert_eq!(format_time(0.5), "PT12H00M00S");
        assert_eq!(parse_time(&format_time(0.5)), Some(0.5));
        // Sub-second precision survives, which `formats.ods` in loop C's corpus needs: it
        // holds `PT05H35M31.2S`, and rounding that to a whole second changes the document.
        for text in ["PT05H35M31.2S", "PT10H11M12.6S", "PT00H00M08.999S"] {
            let parsed = parse_time(text).expect(text);
            assert_eq!(format_time(parsed), text, "{text}");
        }
        // A whole number of seconds keeps its two digits rather than losing them to the
        // zero-trimming.
        assert_eq!(format_time(20.0 / DAY), "PT00H00M20S");
        // A duration longer than a day keeps the day — `pivot-table-shared-cache-with-
        // group.ods` in loop C's corpus holds 1.40625, and 24-hour wrapping loses it.
        assert_eq!(format_time(1.40625), "PT33H45M00S");
        assert_eq!(parse_time("PT33H45M00S"), Some(1.40625));
        assert_eq!(format_time(-0.5), "-PT12H00M00S");
    }

    #[test]
    fn a_serial_and_its_text_agree_about_the_clock() {
        // The formatter splits exactly and `seconds_of_day` rounds (§6.10.17); both must
        // still describe the same instant, which is what this pins.
        for fraction in [0.0, 0.5, 0.233, 1.0 / 3.0, 8.999 / DAY] {
            let text = format_time(fraction);
            let back = parse_time(&text).expect(&text);
            assert!(
                (back - fraction).abs() < 1e-12,
                "{text}: {back} vs {fraction}"
            );
        }
    }
}
