// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.18 Statistical Functions — the Small Group's eight (`AVERAGEIF` is in `criterion.rs`), and
//! `LARGE`, `SMALL`, `MEDIAN` and `RANK` from beyond it. All take a `NumberSequence`, so text and
//! empty cells in a range are skipped, not counted as zero.

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
        "COM.MICROSOFT.MAXIFS" => super::criterion::several(args, super::criterion::Mode::Max),
        "COM.MICROSOFT.MINIFS" => super::criterion::several(args, super::criterion::Mode::Min),
        // §6.18.82/§6.18.84: the sample forms divide by n-1, the population forms by n; Excel's
        // 2010 names are the same four.
        "VAR" | "COM.MICROSOFT.VAR.S" => moment(args, true, false),
        "VARP" | "COM.MICROSOFT.VAR.P" => moment(args, false, false),
        "STDEV" | "COM.MICROSOFT.STDEV.S" => moment(args, true, true),
        "STDEVP" | "COM.MICROSOFT.STDEV.P" => moment(args, false, true),
        "LARGE" => nth(args, true),
        "SMALL" => nth(args, false),
        "MEDIAN" => median(args),
        "RANK" | "COM.MICROSOFT.RANK.EQ" => rank(args, false),
        "COM.MICROSOFT.RANK.AVG" => rank(args, true),
        "PERCENTILE" | "COM.MICROSOFT.PERCENTILE.INC" => percentile(args),
        "COM.MICROSOFT.PERCENTILE.EXC" => percentile_exc(args),
        "QUARTILE" | "COM.MICROSOFT.QUARTILE.INC" => quartile(args),
        "COM.MICROSOFT.QUARTILE.EXC" => quartile_exc(args),
        "MODE" | "COM.MICROSOFT.MODE.SNGL" => mode(args),
        "PERCENTRANK" | "COM.MICROSOFT.PERCENTRANK.INC" => percent_rank(args, false),
        "COM.MICROSOFT.PERCENTRANK.EXC" => percent_rank(args, true),
        "SUMSQ" => fold_numbers(args, |n| Ok(n.iter().map(|x| x * x).sum())),
        "DEVSQ" => fold_numbers(args, |n| Ok(deviations(n).map(|d| d * d).sum())),
        "AVEDEV" => fold_numbers(args, |n| {
            if n.is_empty() {
                return Err(FormulaError::DivZero);
            }
            Ok(deviations(n).map(f64::abs).sum::<f64>() / n.len() as f64)
        }),
        "GEOMEAN" => fold_numbers(args, |n| {
            if n.is_empty() || n.iter().any(|x| *x <= 0.0) {
                return Err(FormulaError::Num);
            }
            Ok((n.iter().map(|x| x.ln()).sum::<f64>() / n.len() as f64).exp())
        }),
        "HARMEAN" => fold_numbers(args, |n| {
            if n.is_empty() || n.iter().any(|x| *x <= 0.0) {
                return Err(FormulaError::Num);
            }
            Ok(n.len() as f64 / n.iter().map(|x| 1.0 / x).sum::<f64>())
        }),
        "CORREL" | "PEARSON" => paired(args, 0, Pairs::correlation),
        "RSQ" => paired(args, 0, |p| p.correlation().map(|r| r * r)),
        "SLOPE" => paired(args, 0, Pairs::slope),
        "INTERCEPT" => paired(args, 0, Pairs::intercept),
        "FORECAST" | "COM.MICROSOFT.FORECAST.LINEAR" => forecast(args),
        "LEGACY.NORMSINV" | "COM.MICROSOFT.NORM.S.INV" => super::dist::normsinv(args),
        "LEGACY.NORMSDIST" => super::dist::normsdist(args),
        "NORMDIST" => super::dist::normdist(args, false),
        "COM.MICROSOFT.NORM.DIST" => super::dist::normdist(args, true),
        "NORMINV" | "COM.MICROSOFT.NORM.INV" => super::dist::norminv(args),
        "STANDARDIZE" => super::dist::standardize(args),
        "COM.MICROSOFT.NORM.S.DIST" => super::dist::norm_s_dist(args),
        "COM.MICROSOFT.GAMMA.DIST" => super::dist::gamma_dist(args),
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

/// [`moment`] over numbers already in hand, in two passes: `E[x²] - E[x]²` is catastrophically
/// inexact for a large mean and a small spread, a normal spreadsheet column.
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

/// §6.18.40 `LARGE( List ; N )` and §6.18.70 `SMALL( List ; N )`. Their constraints differ
/// (`ROUNDUP(N;0) = N` for `LARGE`, `ROUNDDOWN(N;0) = N` for `SMALL`) and the oracle applies each
/// as named: `LARGE(…;1.5)` is the second largest, `SMALL(…;1.9)` the smallest.
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
        f64::midpoint(numbers[half - 1], numbers[half])
    }))
}

/// §6.18.65 `RANK( Value ; Data [ ; Order = 0 ] )`: one plus how many numbers come before `Value`
/// (larger by default, smaller for a non-zero `Order`), so equal values share a rank. A value not
/// in `Data` is `#N/A`, compared exactly; a `Data` with no numbers is `#VALUE!` — both measured.
/// With `average`, Excel's `RANK.AVG`: ties share the mean of the ranks they span.
fn rank(args: &mut Args, average: bool) -> Answer {
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
    let tied = if average {
        data.iter().filter(|&&x| x == value).count()
    } else {
        1
    };
    Ok(Value::number(
        ahead as f64 + f64::midpoint(tied as f64, 1.0),
    ))
}

/// A whole number sequence in, one number out.
fn fold_numbers(args: &mut Args, f: impl Fn(&[f64]) -> Result<f64, FormulaError>) -> Answer {
    args.arity(1..)?;
    Ok(Value::number(f(&args.numbers()?)?))
}

/// Each number's distance from the mean.
fn deviations(numbers: &[f64]) -> impl Iterator<Item = f64> + '_ {
    let mean = numbers.iter().sum::<f64>() / numbers.len().max(1) as f64;
    numbers.iter().map(move |x| x - mean)
}

/// The sorted numbers of the first argument, `#VALUE!` when there are none (as the oracle
/// answers §6.18.57's `COUNT( Data ) > 0`).
fn sorted_data(args: &mut Args) -> Result<Vec<f64>, FormulaError> {
    let mut data = args.numbers_range(0..1)?;
    if data.is_empty() {
        return Err(FormulaError::Value);
    }
    data.sort_by(f64::total_cmp);
    Ok(data)
}

/// §6.18.57's interpolation over sorted data at rank `1 + x·(n − 1)`.
fn interpolate(data: &[f64], x: f64) -> f64 {
    let r = x * (data.len() - 1) as f64;
    let (i, d) = (r.floor() as usize, r - r.floor());
    match data.get(i + 1) {
        Some(next) => data[i] + d * (next - data[i]),
        None => data[i],
    }
}

/// §6.18.57 `PERCENTILE( Data ; X )`, `0 ≤ X ≤ 1`.
fn percentile(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let data = sorted_data(args)?;
    let x = args.number(1)?;
    if !(0.0..=1.0).contains(&x) {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(interpolate(&data, x)))
}

/// Excel's `PERCENTILE.EXC( Data ; X )`: the interpolation at rank `X·(n + 1)`, which must fall
/// within the data — `#NUM!` outside it, and for `X` of 0 or 1.
fn percentile_exc(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let data = sorted_data(args)?;
    exclusive(&data, args.number(1)?)
}

fn exclusive(data: &[f64], x: f64) -> Answer {
    let r = super::math::shown(x * (data.len() + 1) as f64);
    if !(x > 0.0 && x < 1.0) || r < 1.0 || r > data.len() as f64 {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(interpolate(
        data,
        (r - 1.0) / (data.len() - 1).max(1) as f64,
    )))
}

/// Excel's `QUARTILE.EXC( Data ; Quart )`: [`percentile_exc`] at a quarter step, 1 to 3.
fn quartile_exc(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let data = sorted_data(args)?;
    let quart = args.integer(1)?;
    if !(1..=3).contains(&quart) {
        return Err(FormulaError::Num);
    }
    exclusive(&data, quart as f64 / 4.0)
}

/// §6.18.64 `QUARTILE( Data ; Quart )`: the percentile at a quarter step, `Quart` truncated.
fn quartile(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let data = sorted_data(args)?;
    let quart = args.integer(1)?;
    if !(0..=4).contains(&quart) {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(interpolate(&data, quart as f64 / 4.0)))
}

/// §6.18.50 `MODE`: the most frequent number, the smallest of a tie; `#VALUE!` when no number
/// repeats, as the oracle answers.
fn mode(args: &mut Args) -> Answer {
    args.arity(1..)?;
    let mut data = args.numbers()?;
    data.sort_by(f64::total_cmp);
    let mut best: Option<(f64, usize)> = None;
    for run in data.chunk_by(|a, b| a == b) {
        if run.len() > 1 && best.is_none_or(|(_, count)| run.len() > count) {
            best = Some((run[0], run.len()));
        }
    }
    best.map(|(value, _)| Value::number(value))
        .ok_or(FormulaError::Value)
}

/// §6.18.58 `PERCENTRANK( Data ; X ; Significance )`: X's rank in the sorted data, interpolated
/// between neighbours, over `n − 1`, rounded to `Significance` significant digits (3 by default),
/// as the oracle rounds. An X outside the data is `#VALUE!`, as the oracle answers. `exclusive`
/// is Excel's `PERCENTRANK.EXC`: one more than the rank, over `n + 1`.
fn percent_rank(args: &mut Args, exclusive: bool) -> Answer {
    args.arity(2..=3)?;
    let data = sorted_data(args)?;
    let x = args.number(1)?;
    let digits = if args.len() > 2 && !args.omitted(2) {
        args.integer(2)?
    } else {
        3
    };
    if digits < 1 {
        return Err(FormulaError::Num);
    }
    let (first, last) = (data[0], data[data.len() - 1]);
    if x < first || x > last {
        return Err(FormulaError::Value);
    }
    if data.len() == 1 {
        return Ok(Value::Number(1.0));
    }
    let below = data.iter().filter(|v| **v < x).count();
    let rank = if data.contains(&x) {
        below as f64
    } else {
        let (y, z) = (data[below - 1], data[below]);
        (below - 1) as f64 + (x - y) / (z - y)
    };
    let share = if exclusive {
        (rank + 1.0) / (data.len() + 1) as f64
    } else {
        rank / (data.len() - 1) as f64
    };
    Ok(Value::number(significant(share, digits.min(15) as i32)))
}

/// `v` rounded to `digits` significant digits.
fn significant(v: f64, digits: i32) -> f64 {
    if v == 0.0 {
        return 0.0;
    }
    let scale = 10f64.powi(digits - 1 - v.abs().log10().floor() as i32);
    (v * scale).round() / scale
}

/// Pairs of numbers at the same place in two arrays of one shape — §6.18's paired statistics
/// skip a place where either is not a number.
struct Pairs {
    xs: Vec<f64>,
    ys: Vec<f64>,
}

impl Pairs {
    fn means(&self) -> (f64, f64) {
        let n = self.xs.len() as f64;
        (
            self.xs.iter().sum::<f64>() / n,
            self.ys.iter().sum::<f64>() / n,
        )
    }

    /// Σ(x−x̄)(y−ȳ), Σ(x−x̄)², Σ(y−ȳ)².
    fn sums(&self) -> (f64, f64, f64) {
        let (mx, my) = self.means();
        self.xs
            .iter()
            .zip(&self.ys)
            .fold((0.0, 0.0, 0.0), |(xy, xx, yy), (x, y)| {
                let (dx, dy) = (x - mx, y - my);
                (xy + dx * dy, xx + dx * dx, yy + dy * dy)
            })
    }

    /// §6.18.17 `CORREL`, §6.18.56 `PEARSON`.
    fn correlation(&self) -> Result<f64, FormulaError> {
        let (xy, xx, yy) = self.sums();
        let denominator = (xx * yy).sqrt();
        if denominator == 0.0 {
            return Err(FormulaError::DivZero);
        }
        Ok(xy / denominator)
    }

    /// §6.18.69 `SLOPE` of y on x.
    fn slope(&self) -> Result<f64, FormulaError> {
        let (xy, xx, _) = self.sums();
        if xx == 0.0 {
            return Err(FormulaError::DivZero);
        }
        Ok(xy / xx)
    }

    /// §6.18.38 `INTERCEPT`.
    fn intercept(&self) -> Result<f64, FormulaError> {
        let (mx, my) = self.means();
        Ok(my - self.slope()? * mx)
    }
}

/// The numbers paired from arguments `first` (y) and `first + 1` (x), `ForceArray` both; arrays of
/// different shapes are `#N/A`, and no pair of numbers at all `#VALUE!`, as the oracle answers.
fn pairs(args: &mut Args, first: usize) -> Result<Pairs, FormulaError> {
    let (y, x) = (args.array(first), args.array(first + 1));
    if (y.rows, y.cols) != (x.rows, x.cols) {
        return Err(FormulaError::NA);
    }
    let mut pairs = Pairs {
        xs: Vec::new(),
        ys: Vec::new(),
    };
    for (a, b) in y.values().iter().zip(x.values()) {
        if let Value::Error(e) = a {
            return Err(*e);
        }
        if let Value::Error(e) = b {
            return Err(*e);
        }
        if let (Value::Number(a), Value::Number(b)) = (a, b) {
            pairs.ys.push(*a);
            pairs.xs.push(*b);
        }
    }
    if pairs.xs.is_empty() {
        return Err(FormulaError::Value);
    }
    Ok(pairs)
}

/// A paired statistic of arguments `first` and `first + 1`.
fn paired(
    args: &mut Args,
    first: usize,
    f: impl Fn(&Pairs) -> Result<f64, FormulaError>,
) -> Answer {
    args.arity(first + 2..=first + 2)?;
    Ok(Value::number(f(&pairs(args, first)?)?))
}

/// §6.18.28 `FORECAST( Value ; Data_Y ; Data_X )`: the regression line at `Value`.
fn forecast(args: &mut Args) -> Answer {
    args.arity(3..=3)?;
    let at = args.number(0)?;
    let pairs = pairs(args, 1)?;
    Ok(Value::number(pairs.intercept()? + pairs.slope()? * at))
}
