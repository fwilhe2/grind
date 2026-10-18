// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.18 Statistical Functions — the eight in the Small Group (`AVERAGEIF`'s matching lives
//! in `criterion.rs`), and `LARGE`, `SMALL`, `MEDIAN` and `RANK` from beyond it.
//!
//! All of them take a NumberSequence, so all of them inherit §6.3.7's filtering: text and
//! empty cells inside a range are *skipped*, not counted as zero. That is the difference
//! between `AVERAGE` over a column with a header and a wrong answer.

use super::super::value::{FormulaError, Value};
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        "AVERAGE" => average(args),
        "AVERAGEIF" => super::criterion::conditional(args, super::criterion::Mode::Average),
        "AVERAGEIFS" => super::criterion::several(args, super::criterion::Mode::Average),
        "MAX" => extreme(args, true),
        "MIN" => extreme(args, false),
        // §6.18.82/§6.18.84: the sample forms divide by n-1, the population forms by n.
        "VAR" => moment(args, true, false),
        "VARP" => moment(args, false, false),
        "STDEV" => moment(args, true, true),
        "STDEVP" => moment(args, false, true),
        "LARGE" => nth(args, true),
        "SMALL" => nth(args, false),
        "MEDIAN" => median(args),
        "RANK" => rank(args),
        _ => return None,
    })
}

/// §6.18.3. An average of nothing is a division by zero, and says so.
fn average(args: &mut Args) -> Answer {
    mean(&args.numbers()?)
}

/// [`average`] over numbers already in hand — `SUBTOTAL`'s too.
pub(super) fn mean(numbers: &[f64]) -> Answer {
    if numbers.is_empty() {
        return Err(FormulaError::DivZero);
    }
    Ok(Value::number(
        numbers.iter().sum::<f64>() / numbers.len() as f64,
    ))
}

/// §6.18.45 `MAX` and §6.18.48 `MIN`, which return 0 when handed no numbers at all — the
/// spec's answer, and the one that keeps `MAX(A1:A9)` over an empty column from erroring.
fn extreme(args: &mut Args, max: bool) -> Answer {
    extreme_of(&args.numbers()?, max)
}

/// [`extreme`] over numbers already in hand.
pub(super) fn extreme_of(numbers: &[f64], max: bool) -> Answer {
    Ok(Value::number(
        numbers
            .iter()
            .fold(None, |best: Option<f64>, &n| {
                Some(match best {
                    None => n,
                    Some(b) if max => b.max(n),
                    Some(b) => b.min(n),
                })
            })
            .unwrap_or(0.0),
    ))
}

/// Variance and standard deviation, sample or population.
fn moment(args: &mut Args, sample: bool, root: bool) -> Answer {
    moment_of(&args.numbers()?, sample, root)
}

/// [`moment`] over numbers already in hand.
///
/// Two passes rather than the `E[x²] - E[x]²` shortcut: that identity is exact in algebra
/// and catastrophically inexact in floating point for values with a large mean and a small
/// spread, which is a normal shape for a spreadsheet column.
pub(super) fn moment_of(numbers: &[f64], sample: bool, root: bool) -> Answer {
    let n = numbers.len() as f64;
    let divisor = if sample { n - 1.0 } else { n };
    if divisor <= 0.0 {
        // A sample variance needs two values; a population variance needs one.
        return Err(FormulaError::DivZero);
    }
    let mean = numbers.iter().sum::<f64>() / n;
    let variance = numbers.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / divisor;
    Ok(Value::number(if root { variance.sqrt() } else { variance }))
}

/// The numbers of a sequence, in ascending order — what `LARGE`, `SMALL` and `MEDIAN` all
/// rank. "Nothing to rank" is `#VALUE!` for each of them, as measured.
fn ranked(numbers: Vec<f64>) -> Result<Vec<f64>, FormulaError> {
    if numbers.is_empty() {
        return Err(FormulaError::Value);
    }
    let mut numbers = numbers;
    numbers.sort_by(f64::total_cmp);
    Ok(numbers)
}

/// §6.18.40 `LARGE( List ; N )` and §6.18.70 `SMALL( List ; N )`.
///
/// The two constraints are not symmetrical — `ROUNDUP(N;0) = N` for `LARGE`,
/// `ROUNDDOWN(N;0) = N` for `SMALL` — and the oracle applies each as the rounding it names,
/// so `LARGE(…;1.5)` is the second largest and `SMALL(…;1.9)` the smallest. The array form
/// of `N` stays out with the rest of arrays.
fn nth(args: &mut Args, largest: bool) -> Answer {
    args.arity(2..=2)?;
    if args.omitted(1) {
        return Err(FormulaError::Value);
    }
    let numbers = ranked(args.numbers_range(0..1)?)?;
    let n = args.number(1)?;
    let n = if largest { n.ceil() } else { n.floor() };
    if n < 1.0 || n > numbers.len() as f64 {
        return Err(FormulaError::Value);
    }
    let i = n as usize - 1;
    Ok(Value::Number(if largest {
        numbers[numbers.len() - 1 - i]
    } else {
        numbers[i]
    }))
}

/// §6.18.47 `MEDIAN`: the middle number, or the mean of the middle two.
fn median(args: &mut Args) -> Answer {
    args.arity(1..)?;
    let numbers = ranked(args.numbers()?)?;
    let half = numbers.len() / 2;
    Ok(Value::number(if numbers.len() % 2 == 1 {
        numbers[half]
    } else {
        (numbers[half - 1] + numbers[half]) / 2.0
    }))
}

/// §6.18.65 `RANK( Value ; Data [ ; Order = 0 ] )`: one plus how many numbers come before
/// `Value` — larger ones by default, smaller ones for any non-zero `Order` — so equal values
/// share a rank and the next one skips past them.
///
/// "Value shall exist in Data": a value that does not is `#N/A`, compared exactly, while a
/// `Data` with no numbers in it at all is `#VALUE!` — two different errors, both measured.
fn rank(args: &mut Args) -> Answer {
    args.arity(2..=3)?;
    let value = args.number(0)?;
    let data = args.numbers_range(1..2)?;
    let ascending = args.len() > 2 && !args.omitted(2) && args.number(2)? != 0.0;
    if data.is_empty() {
        return Err(FormulaError::Value);
    }
    if !data.contains(&value) {
        return Err(FormulaError::NA);
    }
    let ahead = data
        .iter()
        .filter(|&&x| if ascending { x < value } else { x > value })
        .count();
    Ok(Value::number(ahead as f64 + 1.0))
}
