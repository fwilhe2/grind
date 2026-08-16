// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.10 Date and Time Functions — the Small Group's 11.
//!
//! All the calendar arithmetic lives in [`super::super::date`]; this file is the mapping
//! from a function name onto it, plus the two pseudotypes §4.11.3 and §4.11.4 define.
//!
//! The one thing worth knowing before reading further: a date **is** a number here
//! (§4.3.3) and a time is the fraction of a day (§4.3.2), so `DAY` and `HOUR` are two ways
//! of decomposing the same `f64` and `[.A1]+1` on a date is the next day without anything
//! being taught about calendars.
//!
//! ponytail: a formula that *returns* a date — `DATE(…)`, `TODAY()` — has no way to say so,
//! so `App::recalc` writes its result as a plain float and the cell loses the date spelling
//! a read-in one keeps (`Sheet::kind`). LibreOffice has the same shape of answer and gets
//! it from the cell's number format rather than from the function; phase 5 is where that
//! becomes possible, and doing it before then would mean guessing.

use super::super::date;
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
        _ => return None,
    })
}

fn nullary(args: &mut Args, f: impl Fn(i64) -> f64) -> Answer {
    args.arity(0..=0)?;
    Ok(Value::number(f(args.null_date())))
}

/// §6.10.2 `DATE(Year; Month; Day)`.
///
/// "Fractional values are truncated", and out-of-range months and days roll over — both
/// belong to [`date::serial`], which is also what the reader and the writer use.
///
/// The two rules on top of that are both about the *year*, and both are the corpus's:
/// a two-digit one expands through `HOST-NULL-YEAR` before anything else happens
/// (`DATE(0;13;31)` is 2001-01-31, so the expansion precedes the month roll-over), and a
/// year before §7.4's 1583 is `#VALUE!` rather than a proleptic guess.
fn date_from_parts(args: &mut Args) -> Answer {
    args.arity(3..=3)?;
    let (y, m, d) = (args.integer(0)?, args.integer(1)?, args.integer(2)?);
    let y = date::expand_year(y, args.null_year());
    if y < date::MIN_YEAR {
        return Err(FormulaError::Value);
    }
    Ok(Value::number(date::serial(y, m, d, args.null_date())))
}

/// §6.10.18 `TIME(Hours; Minutes; Seconds)`.
///
/// The three parameters "shall not be limited to the ranges 0..24, 0..59, or 0..60", so
/// they are summed as written and only the total is reduced — to a clock face rather than
/// to a count of days. §6.10.18's formula alone would make `TIME(24;0;0)` one whole day;
/// LibreOffice returns midnight, and `TIME(24;60;1)` in the corpus is 01:00:01, so the
/// wrap is the observed semantics and the oracle wins.
///
/// Reduced in *seconds* rather than in days, which keeps `TIME(25;0;0)` bit-identical to
/// `TIME(1;0;0)` instead of a rounding step away from it.
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

/// §6.10.21 `WEEKDAY(D [; Type = 1])`, whose Table 6 is ten columns of one rule.
///
/// Every column is "count from some first-day-of-week", so a type picks two things: which
/// weekday is first, and whether counting starts at 1 or at 0. Type 3 is the only one that
/// starts at 0, and types 11..=17 name their own first day — 11 is Monday through 17 is
/// Sunday, which is why `(type - 10) % 7` lands 17 back on Sunday alongside type 1.
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
