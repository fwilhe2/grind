// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.10 Date and Time Functions — the Small Group's 11, and `EOMONTH`, `EDATE`, `WEEKNUM`,
//! `ISOWEEKNUM`, `NETWORKDAYS` and `WORKDAY` from beyond it. The calendar arithmetic is
//! [`super::super::date`]'s; a date is a number (§4.3.3) and a time the fraction of a day
//! (§4.3.2).
//!
//! ponytail: a formula that returns a date (`DATE(…)`, `TODAY()`) cannot say so, so `App::recalc`
//! writes a plain float and the cell loses the date spelling a read-in one keeps (`Sheet::kind`).
//! LibreOffice takes it from the cell's number format instead.

use std::collections::BTreeSet;

use super::super::date;
use super::super::eval::Operand;
use super::super::value::{FormulaError, Value};
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        "DATE" => date_from_parts(args),
        "TIME" => time_from_parts(args),
        "NOW" => nullary(args, date::now),
        // §6.10.20: "This only returns the date, not the datetime value."
        "TODAY" => nullary(args, |null_date| date::now(null_date).floor()),

        // §6.10.5, §6.10.14, §6.10.24 — one serial, three ways of reading the calendar.
        "DAY" => from_date(args, |(_, _, d)| d),
        "MONTH" => from_date(args, |(_, m, _)| m),
        "YEAR" => from_date(args, |(y, _, _)| y),

        // §6.10.11, §6.10.13, §6.10.17 — one clock, read to the nearest second once.
        "HOUR" => from_clock(args, |s| s / 3600),
        "MINUTE" => from_clock(args, |s| s / 60 % 60),
        "SECOND" => from_clock(args, |s| s % 60),

        "WEEKDAY" => weekday(args),
        "EOMONTH" => months_on(args, true),
        "EDATE" => months_on(args, false),
        "WEEKNUM" => weeknum(args),
        "ISOWEEKNUM" => isoweeknum(args),
        "NETWORKDAYS" => networkdays(args),
        "WORKDAY" => workday(args),
        _ => return None,
    })
}

fn nullary(args: &mut Args, f: impl Fn(i64) -> f64) -> Answer {
    args.arity(0..=0)?;
    Ok(Value::number(f(args.null_date())))
}

/// §6.10.2 `DATE(Year; Month; Day)`. Truncation and roll-over are [`date::serial`]'s. A two-digit
/// year expands through `HOST-NULL-YEAR` first (`DATE(0;13;31)` is 2001-01-31), and a year before
/// §7.4's 1583 is `#VALUE!`.
fn date_from_parts(args: &mut Args) -> Answer {
    args.arity(3..=3)?;
    let (y, m, d) = (args.integer(0)?, args.integer(1)?, args.integer(2)?);
    let y = date::expand_year(y, args.null_year());
    if y < date::MIN_YEAR {
        return Err(FormulaError::Value);
    }
    Ok(Value::number(date::serial(y, m, d, args.null_date())))
}

/// §6.10.18 `TIME(Hours; Minutes; Seconds)`. The parameters are not limited to their ranges, so
/// they are summed and the total wrapped to a clock face: LibreOffice returns midnight for
/// `TIME(24;0;0)` and the corpus has `TIME(24;60;1)` as 01:00:01, where §6.10.18's formula would
/// make a whole day. Reduced in seconds, so `TIME(25;0;0)` is bit-identical to `TIME(1;0;0)`.
fn time_from_parts(args: &mut Args) -> Answer {
    args.arity(3..=3)?;
    let (h, m, s) = (args.number(0)?, args.number(1)?, args.number(2)?);
    let seconds = (h * 3600.0 + m * 60.0 + s).rem_euclid(86_400.0);
    Ok(Value::number(seconds / 86_400.0))
}

fn from_date(args: &mut Args, f: impl Fn((i64, i64, i64)) -> i64) -> Answer {
    args.arity(1..=1)?;
    let serial = args.serial(0)?;
    Ok(Value::number(f(date::ymd(serial, args.null_date())) as f64))
}

fn from_clock(args: &mut Args, f: impl Fn(i64) -> i64) -> Answer {
    args.arity(1..=1)?;
    let serial = args.serial(0)?;
    Ok(Value::number(f(date::seconds_of_day(serial)) as f64))
}

/// §6.10.21 `WEEKDAY(D [; Type = 1])`. Every column of Table 6 counts from a first day of the
/// week, starting at 1 (or 0, for type 3); types 11..=17 name their first day from Monday to
/// Sunday, so `(type - 10) % 7` puts 17 on Sunday with type 1.
fn weekday(args: &mut Args) -> Answer {
    args.arity(1..=2)?;
    let serial = args.serial(0)?;
    let kind = if args.len() > 1 && !args.omitted(1) {
        args.integer(1)?
    } else {
        1
    };
    let (first, base) = match kind {
        1 => (0, 1),
        2 => (1, 1),
        3 => (1, 0),
        11..=17 => ((kind - 10) % 7, 1),
        _ => return Err(FormulaError::Num),
    };
    let day = date::weekday(serial, args.null_date());
    Ok(Value::number(((day - first).rem_euclid(7) + base) as f64))
}

/// §6.10.10 `EOMONTH` and §6.10.9 `EDATE`: truncate both arguments, add the months, and take the
/// month's last day (`EOMONTH`) or the same day, clamped to the month's length (`EDATE`).
/// [`date::serial`] rolls day 0 back to the end of the month before.
fn months_on(args: &mut Args, end_of_month: bool) -> Answer {
    args.arity(2..=2)?;
    let start = args.serial(0)?;
    let months = args.number(1)?.trunc() as i64;
    let null_date = args.null_date();
    let (y, m, d) = date::ymd(start, null_date);
    let last = date::serial(y, m + months + 1, 0, null_date);
    if end_of_month {
        return Ok(Value::number(last));
    }
    let (_, _, last_day) = date::ymd(last, null_date);
    Ok(Value::number(date::serial(
        y,
        m + months,
        d.min(last_day),
        null_date,
    )))
}

/// The weekday a week starts on for each of §6.10.22's `Mode`s (Sunday is 0), or `None` for
/// the two ISO 8601 modes.
fn week_start(mode: i64) -> Result<Option<i64>, FormulaError> {
    Ok(Some(match mode {
        1 | 17 => 0,
        2 | 11 => 1,
        12..=16 => mode - 10,
        21 | 150 => return Ok(None),
        _ => return Err(FormulaError::Num),
    }))
}

/// §6.10.22 `WEEKNUM( D [ ; Mode = 1 ] )`.
///
/// "The week containing January 1 is the first week of the year", read as written — so the
/// last days of December that share a week with the *next* January 1 are week 1, which is
/// also the oracle's answer and not Excel's 53 (`doc/ods-format.md` §4.1).
fn weeknum(args: &mut Args) -> Answer {
    args.arity(1..=2)?;
    let day = args.serial(0)?.floor();
    let mode = if args.len() > 1 && !args.omitted(1) {
        args.integer(1)?
    } else {
        1
    };
    let null_date = args.null_date();
    let Some(first) = week_start(mode)? else {
        return Ok(Value::number(iso_week(day, null_date) as f64));
    };
    let starts =
        |serial: f64| serial - (date::weekday(serial, null_date) - first).rem_euclid(7) as f64;
    let week = starts(day);
    let (year, _, _) = date::ymd(day, null_date);
    if week + 6.0 >= date::serial(year + 1, 1, 1, null_date) {
        return Ok(Value::Number(1.0));
    }
    let first_week = starts(date::serial(year, 1, 1, null_date));
    Ok(Value::number(((week - first_week) / 7.0).round() + 1.0))
}

/// §6.10.12 `ISOWEEKNUM( D )`.
fn isoweeknum(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    let day = args.serial(0)?.floor();
    Ok(Value::number(iso_week(day, args.null_date()) as f64))
}

/// ISO 8601's week number: a week runs Monday to Sunday and belongs to the year its Thursday
/// is in, so the week's number is where that Thursday falls in its own year.
fn iso_week(day: f64, null_date: i64) -> i64 {
    let monday_based = (date::weekday(day, null_date) + 6).rem_euclid(7);
    let thursday = day - monday_based as f64 + 3.0;
    let (year, _, _) = date::ymd(thursday, null_date);
    ((thursday - date::serial(year, 1, 1, null_date)) / 7.0).floor() as i64 + 1
}

/// The `Holidays` parameter of §6.10.15 and §6.10.23: a DateSequence, each date's time
/// dropped. Text and empty cells inside a range are nothing, as in any sequence; a text
/// *argument* is a date, as any DateParam is.
fn holidays(args: &mut Args, i: usize) -> Result<BTreeSet<i64>, FormulaError> {
    if i >= args.len() || args.omitted(i) {
        return Ok(BTreeSet::new());
    }
    let dates = match args.operand(i) {
        Operand::Area(_) => args.numbers_range(i..i + 1)?,
        Operand::Value(Value::Empty) => Vec::new(),
        Operand::Value(_) => vec![args.serial(i)?],
    };
    Ok(dates.into_iter().map(|d| d.floor() as i64).collect())
}

/// The `Workdays` parameter: seven flags, Sunday first, non-zero meaning "not a working day"
/// — or the Saturday-and-Sunday weekend when it is left out. Anything but seven numbers or
/// logicals is `#VALUE!`.
fn weekend(args: &mut Args, i: usize) -> Result<[bool; 7], FormulaError> {
    if i >= args.len() || args.omitted(i) {
        return Ok([true, false, false, false, false, false, true]);
    }
    let values = match args.operand(i) {
        Operand::Area(area) => args.values_in(&area),
        Operand::Value(value) => vec![value],
    };
    if values.len() != 7 {
        return Err(FormulaError::Value);
    }
    let mut off = [false; 7];
    for (flag, value) in off.iter_mut().zip(values) {
        *flag = match value {
            Value::Number(n) => n != 0.0,
            Value::Bool(b) => b,
            Value::Empty => false,
            Value::Error(e) => return Err(e),
            Value::Text(_) => return Err(FormulaError::Value),
        };
    }
    Ok(off)
}

/// The calendar `NETWORKDAYS` and `WORKDAY` walk: which weekdays are off, and which dates.
struct Calendar {
    off: [bool; 7],
    holidays: BTreeSet<i64>,
    null_date: i64,
}

impl Calendar {
    fn works(&self, day: i64) -> bool {
        !self.off[date::weekday(day as f64, self.null_date) as usize]
            && !self.holidays.contains(&day)
    }

    /// Working days in a week, holidays aside.
    fn per_week(&self) -> i64 {
        self.off.iter().filter(|off| !**off).count() as i64
    }

    /// Working days from `from` to `to`, both counted, `from <= to` — in whole weeks and a
    /// remainder, then holidays on working weekdays taken off once each.
    fn count(&self, from: i64, to: i64) -> i64 {
        let days = to - from + 1;
        let tail = from + days / 7 * 7;
        let plain = |day: i64| !self.off[date::weekday(day as f64, self.null_date) as usize];
        let weekdays =
            days / 7 * self.per_week() + (tail..=to).filter(|d| plain(*d)).count() as i64;
        let holidays = self
            .holidays
            .range(from..=to)
            .filter(|d| plain(**d))
            .count() as i64;
        weekdays - holidays
    }
}

/// §6.10.15 `NETWORKDAYS( Date1 ; Date2 [ ; [ Holidays ] [ ; Workdays ] ] )`: the working
/// days from one date to the other, both counted, negative when the second is earlier.
fn networkdays(args: &mut Args) -> Answer {
    args.arity(2..=4)?;
    let (a, b) = (
        args.serial(0)?.floor() as i64,
        args.serial(1)?.floor() as i64,
    );
    let calendar = Calendar {
        holidays: holidays(args, 2)?,
        off: weekend(args, 3)?,
        null_date: args.null_date(),
    };
    Ok(Value::number(if a <= b {
        calendar.count(a, b) as f64
    } else {
        -calendar.count(b, a) as f64
    }))
}

/// §6.10.23 `WORKDAY( Date ; Offset [ ; [ Holidays ] [ ; Workdays ] ] )`: `Offset` working days on
/// from `Date`, which is not counted and is the answer when `Offset` is zero. The fourth parameter
/// follows the spec; LibreOffice 26.8 refuses it. A long offset jumps whole weeks, then owes back
/// the holidays it passed.
fn workday(args: &mut Args) -> Answer {
    args.arity(2..=4)?;
    let start = args.serial(0)?.floor() as i64;
    let offset = args.number(1)?.trunc() as i64;
    let calendar = Calendar {
        holidays: holidays(args, 2)?,
        off: weekend(args, 3)?,
        null_date: args.null_date(),
    };
    if offset == 0 {
        return Ok(Value::number(start as f64));
    }
    let per_week = calendar.per_week();
    if per_week == 0 {
        return Err(FormulaError::Value);
    }
    let step = offset.signum();
    let (mut day, mut owed) = (start, offset.abs());
    while owed > per_week {
        let weeks = (owed - 1) / per_week;
        let next = day + 7 * weeks * step;
        let (low, high) = if step > 0 {
            (day + 1, next)
        } else {
            (next, day - 1)
        };
        let skipped = calendar
            .holidays
            .range(low..=high)
            .filter(|d| !calendar.off[date::weekday(**d as f64, calendar.null_date) as usize]);
        owed -= weeks * per_week - skipped.count() as i64;
        day = next;
        // Past year 9999 is not a date this calendar can name.
        if day.abs() > 3_000_000 {
            return Err(FormulaError::Num);
        }
    }
    while owed > 0 {
        day += step;
        if calendar.works(day) {
            owed -= 1;
        }
    }
    Ok(Value::number(day as f64))
}

#[cfg(test)]
mod tests {
    use super::super::super::eval::{Address, Engine};
    use super::*;
    use crate::model::{Document, Pos};

    /// An empty document at the default epoch — these functions take their arguments, not
    /// the sheet, and the epoch is what they share with it.
    fn eval(formula: &str) -> Value {
        let document = Document::default();
        Engine::new(&document).eval(formula, Address::new(0, Pos::new(0, 0)))
    }

    #[test]
    fn date_and_time_are_built_from_their_parts() {
        // §6.10.2 against the epoch the corpus uses: serial 2 is 1900-01-01.
        assert_eq!(eval("=DATE(1900;1;1)"), Value::Number(2.0));
        assert_eq!(eval("=DATE(1899;12;30)"), Value::Number(0.0));
        // Roll-over, both directions (§6.10.2).
        assert_eq!(eval("=DATE(1983;0;31)"), eval("=DATE(1982;12;31)"));
        assert_eq!(eval("=DATE(2000;13;31)"), eval("=DATE(2001;1;31)"));
        // §6.10.18: a fraction of a day, and the parts are not range-limited.
        assert_eq!(eval("=TIME(12;0;0)"), Value::Number(0.5));
        assert_eq!(eval("=TIME(0;90;0)"), eval("=TIME(1;30;0)"));
        assert_eq!(eval("=TIME(25;0;0)"), eval("=TIME(1;0;0)"));
    }

    #[test]
    fn a_two_digit_year_is_expanded_before_anything_else_happens() {
        // §3.4 item 7, and every one of these is a cell in `date_time/fods/date.fods`.
        assert_eq!(eval("=DATE(0;12;31)"), eval("=DATE(2000;12;31)"));
        assert_eq!(eval("=DATE(1;10;1)"), eval("=DATE(2001;10;1)"));
        assert_eq!(eval("=DATE(10;1;1)"), eval("=DATE(2010;1;1)"));
        // The expansion happens first, so the month roll-over carries into 2001 rather
        // than into year 1.
        assert_eq!(eval("=DATE(0;13;31)"), eval("=DATE(2001;1;31)"));
        assert_eq!(eval("=DATE(0;0;0)"), eval("=DATE(1999;11;30)"));
        // Only *two-digit* years expand: 1899 is 1899.
        assert_eq!(eval("=YEAR(DATE(1899;1;1))"), Value::Number(1899.0));
        // §7.4: before 1583 is implementation-defined, and LibreOffice says #VALUE!.
        assert_eq!(eval("=DATE(100;1;1)"), Value::Error(FormulaError::Value));
        assert_eq!(eval("=DATE(1000;1;1)"), Value::Error(FormulaError::Value));
        assert_eq!(eval("=YEAR(DATE(1583;1;1))"), Value::Number(1583.0));
    }

    #[test]
    fn a_date_decomposes_into_the_parts_it_was_built_from() {
        assert_eq!(eval("=YEAR(DATE(1983;1;31))"), Value::Number(1983.0));
        assert_eq!(eval("=MONTH(DATE(1983;1;31))"), Value::Number(1.0));
        assert_eq!(eval("=DAY(DATE(1983;1;31))"), Value::Number(31.0));
        // The corpus's own three, which pin the epoch from the other side.
        assert_eq!(eval("=YEAR(0)"), Value::Number(1899.0));
        assert_eq!(eval("=YEAR(1)"), Value::Number(1899.0));
        assert_eq!(eval("=YEAR(2)"), Value::Number(1900.0));
    }

    #[test]
    fn a_time_decomposes_the_same_way() {
        assert_eq!(eval("=HOUR(TIME(17;20;5))"), Value::Number(17.0));
        assert_eq!(eval("=MINUTE(TIME(17;20;5))"), Value::Number(20.0));
        assert_eq!(eval("=SECOND(TIME(17;20;5))"), Value::Number(5.0));
        // §4.3.2: a whole number is midnight — 17 is seventeen *days*, not five o'clock.
        assert_eq!(eval("=HOUR(17)"), Value::Number(0.0));
        // A date carries its time with it (§4.3.4).
        assert_eq!(
            eval("=HOUR(DATE(1983;1;31)+TIME(9;0;0))"),
            Value::Number(9.0)
        );
    }

    #[test]
    fn text_is_converted_by_the_pseudotypes() {
        // §6.3.15 and §6.3.16, both taken from the corpus's own fixtures.
        assert_eq!(eval("=WEEKDAY(\"2000-06-14\")"), Value::Number(4.0));
        assert_eq!(eval("=HOUR(\"17:20:00\")"), Value::Number(17.0));
        assert_eq!(eval("=YEAR(\"1983-01-31\")"), Value::Number(1983.0));
        // Text that is a plain number is still a serial (§6.3.15's fallback).
        assert_eq!(eval("=YEAR(\"2\")"), Value::Number(1900.0));
        assert_eq!(
            eval("=YEAR(\"nonsense\")"),
            Value::Error(FormulaError::Value)
        );
    }

    #[test]
    fn weekday_types_agree_with_table_6() {
        // 2016-07-24 was a Sunday, 2000-06-14 a Wednesday — the two the corpus uses.
        let sunday = "\"2016-07-24\"";
        assert_eq!(eval(&format!("=WEEKDAY({sunday};1)")), Value::Number(1.0));
        assert_eq!(eval(&format!("=WEEKDAY({sunday};2)")), Value::Number(7.0));
        assert_eq!(eval(&format!("=WEEKDAY({sunday};3)")), Value::Number(6.0));
        assert_eq!(eval(&format!("=WEEKDAY({sunday};11)")), Value::Number(7.0));
        assert_eq!(eval(&format!("=WEEKDAY({sunday};17)")), Value::Number(1.0));
        // Type 1 is the default, and Table 6's columns 1 and 17 are the same column.
        assert_eq!(eval(&format!("=WEEKDAY({sunday})")), Value::Number(1.0));
        assert_eq!(eval("=WEEKDAY(\"1996-07-24\";2)"), Value::Number(3.0));
        // Not a type in the table.
        assert_eq!(eval("=WEEKDAY(1;4)"), Value::Error(FormulaError::Num));
    }

    #[test]
    fn today_is_a_whole_day_and_now_is_not_before_it() {
        // The only two functions that are not a pure function of the document, so this
        // pins the relationship between them rather than a value.
        let Value::Number(today) = eval("=TODAY()") else {
            panic!("TODAY is a number");
        };
        let Value::Number(now) = eval("=NOW()") else {
            panic!("NOW is a number");
        };
        assert_eq!(today, today.floor());
        assert!(
            (now - today) >= 0.0 && (now - today) < 1.0,
            "{now} vs {today}"
        );
        assert_eq!(eval("=YEAR(TODAY())>=2026"), Value::Bool(true));
    }
}
