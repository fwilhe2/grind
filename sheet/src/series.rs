// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a fill-handle drag continues (`doc/spreadsheet-habits.md` §2): the cells of one line of
//! the selection as a series when they define a step, and otherwise as a pattern repeated.

use crate::formula::date;

/// One selected cell, as the rule sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Seed {
    /// The number a cell holds as a plain value; `None` for text, a truth value, an empty cell
    /// and anything a formula computes, none of which a series continues.
    pub number: Option<f64>,
    /// Whether it is shown as a date.
    pub date: bool,
}

/// How a line goes on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Series {
    /// The selected cells again, in order, a formula's references shifted.
    Repeat,
    /// `first + k × step`, rounded to the seeds' own decimals.
    Step {
        first: f64,
        step: f64,
        decimals: Option<usize>,
    },
    /// A date `step` months on per cell: on the same day of the month (the month's last when it
    /// is shorter), or on every month's last day.
    Months {
        year: i64,
        month: i64,
        day: i64,
        step: i64,
        month_end: bool,
    },
}

/// The series `seeds` define. A single number is copied, as everywhere; a single date steps by a
/// day. Two or more numbers or dates go on by their difference when it is the same between each
/// pair; dates the same day of the month apart, or each on its month's last day, go on by months.
pub(crate) fn of(seeds: &[Seed], null_date: i64) -> Series {
    let Some(numbers) = seeds.iter().map(|s| s.number).collect::<Option<Vec<f64>>>() else {
        return Series::Repeat;
    };
    let dates = seeds.iter().all(|s| s.date) && numbers.iter().all(|n| n.fract() == 0.0);
    match numbers.as_slice() {
        [first] if dates => Series::Step {
            first: *first,
            step: 1.0,
            decimals: Some(0),
        },
        [] | [_] => Series::Repeat,
        _ => {
            if dates && let Some(months) = months(&numbers, null_date) {
                return months;
            }
            step(&numbers).unwrap_or(Series::Repeat)
        }
    }
}

/// The value `k` cells on from the first seed, `k` negative before it.
pub(crate) fn at(series: Series, k: i64, null_date: i64) -> Option<f64> {
    match series {
        Series::Repeat => None,
        Series::Step {
            first,
            step,
            decimals,
        } => {
            let value = first + k as f64 * step;
            Some(decimals.map_or(value, |d| round(value, d)))
        }
        Series::Months {
            year,
            month,
            day,
            step,
            month_end,
        } => {
            let month = month + k * step;
            let last = last_day(year, month, null_date);
            let day = if month_end { last } else { day.min(last) };
            Some(date::serial(year, month, day, null_date))
        }
    }
}

fn step(numbers: &[f64]) -> Option<Series> {
    let decimals = numbers
        .iter()
        .map(|n| decimals(*n))
        .try_fold(0, |most, d| d.map(|d| most.max(d)));
    let differences: Vec<f64> = numbers
        .windows(2)
        .map(|pair| {
            let difference = pair[1] - pair[0];
            decimals.map_or(difference, |d| round(difference, d))
        })
        .collect();
    let first = differences[0];
    let close = |d: f64| {
        let scale = first.abs().max(d.abs()).max(f64::MIN_POSITIVE);
        (d - first).abs() <= scale * 1e-12
    };
    differences
        .iter()
        .all(|d| close(*d))
        .then_some(Series::Step {
            first: numbers[0],
            step: first,
            decimals,
        })
}

fn months(serials: &[f64], null_date: i64) -> Option<Series> {
    let days: Vec<(i64, i64, i64)> = serials.iter().map(|s| date::ymd(*s, null_date)).collect();
    let index = |(y, m, _): (i64, i64, i64)| y * 12 + m - 1;
    let step = index(days[1]) - index(days[0]);
    if step == 0 || days.windows(2).any(|p| index(p[1]) - index(p[0]) != step) {
        return None;
    }
    let (year, month, day) = days[0];
    let month_end = days.iter().all(|&(y, m, d)| d == last_day(y, m, null_date));
    let same_day = days.iter().all(|&(.., d)| d == day);
    (month_end || same_day).then_some(Series::Months {
        year,
        month,
        day,
        step,
        month_end,
    })
}

/// The last day of month `month` of `year`, months past twelve rolling into later years.
fn last_day(year: i64, month: i64, null_date: i64) -> i64 {
    date::ymd(date::serial(year, month + 1, 0, null_date), null_date).2
}

/// How many decimals `n` is written with, shortest form; `None` for one only an exponent writes.
fn decimals(n: f64) -> Option<usize> {
    let written = n.to_string();
    if written.contains(['e', 'E']) {
        return None;
    }
    Some(
        written
            .split_once('.')
            .map_or(0, |(_, fraction)| fraction.len()),
    )
    .filter(|d| *d <= 15)
}

fn round(n: f64, decimals: usize) -> f64 {
    format!("{n:.decimals$}").parse().unwrap_or(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::date::DEFAULT_NULL_DATE as NULL;

    fn number(n: f64) -> Seed {
        Seed {
            number: Some(n),
            date: false,
        }
    }

    fn day(y: i64, m: i64, d: i64) -> Seed {
        Seed {
            number: Some(date::serial(y, m, d, NULL)),
            date: true,
        }
    }

    fn run(seeds: &[Seed], ks: impl IntoIterator<Item = i64>) -> Vec<Option<f64>> {
        let series = of(seeds, NULL);
        ks.into_iter().map(|k| at(series, k, NULL)).collect()
    }

    fn ymd(serials: Vec<Option<f64>>) -> Vec<(i64, i64, i64)> {
        serials
            .into_iter()
            .map(|s| date::ymd(s.unwrap(), NULL))
            .collect()
    }

    #[test]
    fn numbers_go_on_by_their_difference_both_ways() {
        assert_eq!(
            run(&[number(1.0), number(2.0)], [2, 3, -1]),
            [Some(3.0), Some(4.0), Some(0.0)]
        );
        assert_eq!(
            run(&[number(10.0), number(7.5), number(5.0)], [3, 4]),
            [Some(2.5), Some(0.0)]
        );
    }

    #[test]
    fn a_step_keeps_the_seeds_decimals() {
        assert_eq!(
            run(&[number(0.1), number(0.2)], [2, 9]),
            [Some(0.3), Some(1.0)]
        );
    }

    #[test]
    fn one_number_or_an_uneven_line_is_repeated() {
        assert_eq!(of(&[number(5.0)], NULL), Series::Repeat);
        assert_eq!(
            of(&[number(1.0), number(2.0), number(4.0)], NULL),
            Series::Repeat
        );
        let text = Seed {
            number: None,
            date: false,
        };
        assert_eq!(of(&[number(1.0), text], NULL), Series::Repeat);
    }

    #[test]
    fn one_date_steps_by_a_day() {
        assert_eq!(
            ymd(run(&[day(2026, 2, 27)], [1, 2, -27])),
            [(2026, 2, 28), (2026, 3, 1), (2026, 1, 31)]
        );
    }

    #[test]
    fn month_ends_stay_month_ends() {
        assert_eq!(
            ymd(run(&[day(2026, 1, 31), day(2026, 2, 28)], [2, 3, 13, -1])),
            [(2026, 3, 31), (2026, 4, 30), (2027, 2, 28), (2025, 12, 31)]
        );
        assert_eq!(
            ymd(run(&[day(2026, 3, 31), day(2026, 6, 30)], [2, 3])),
            [(2026, 9, 30), (2026, 12, 31)],
            "quarter ends"
        );
    }

    #[test]
    fn the_same_day_of_each_month_clamped_to_shorter_months() {
        assert_eq!(
            ymd(run(&[day(2026, 1, 15), day(2026, 2, 15)], [2, 12])),
            [(2026, 3, 15), (2027, 1, 15)]
        );
        assert_eq!(
            ymd(run(&[day(2025, 12, 30), day(2026, 1, 30)], [2, 3])),
            [(2026, 2, 28), (2026, 3, 30)]
        );
    }

    #[test]
    fn dates_a_week_apart_go_on_by_weeks() {
        assert_eq!(
            ymd(run(&[day(2026, 1, 5), day(2026, 1, 12)], [2, 4])),
            [(2026, 1, 19), (2026, 2, 2)]
        );
    }
}
