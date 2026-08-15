// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.18 Statistical Functions — the eight in the Small Group, minus `AVERAGEIF` (which
//! needs §4.11.8 Criterion matching, still unbuilt).
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
        "MAX" => extreme(args, true),
        "MIN" => extreme(args, false),
        // §6.18.82/§6.18.84: the sample forms divide by n-1, the population forms by n.
        "VAR" => moment(args, true, false),
        "VARP" => moment(args, false, false),
        "STDEV" => moment(args, true, true),
        "STDEVP" => moment(args, false, true),
        _ => return None,
    })
}

/// §6.18.3. An average of nothing is a division by zero, and says so.
fn average(args: &mut Args) -> Answer {
    let numbers = args.numbers()?;
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
    let numbers = args.numbers()?;
    Ok(Value::number(
        numbers
            .into_iter()
            .fold(None, |best: Option<f64>, n| {
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
///
/// Two passes rather than the `E[x²] - E[x]²` shortcut: that identity is exact in algebra
/// and catastrophically inexact in floating point for values with a large mean and a small
/// spread, which is a normal shape for a spreadsheet column.
fn moment(args: &mut Args, sample: bool, root: bool) -> Answer {
    let numbers = args.numbers()?;
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
