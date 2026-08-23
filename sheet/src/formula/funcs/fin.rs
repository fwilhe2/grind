// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.12 Financial Functions — the Small Group's ten.
//!
//! Five of them (`PV`, `FV`, `PMT`, `NPER`, `RATE`) are one equation with a different
//! unknown each time, written once in [`annuity`] and rearranged per function. §6.12.41
//! states it, and §6.12.20, §6.12.36 and §6.12.29 each point back at it:
//!
//! ```text
//! 0 = Pv·(1+Rate)^Nper + Payment·(1+Rate·PayType)·((1+Rate)^Nper − 1)/Rate + Fv
//! ```
//!
//! `Rate = 0` is a removable singularity in that expression, not an error, and each section
//! spells the limit out separately (`Pv = −Fv − Payment·Nper`) — which is why every one of
//! these functions branches on it rather than letting the division produce an infinity.
//!
//! `RATE` and `IRR` have no closed form. §6.12.24 permits an iterative solution and permits
//! returning an Error when it does not converge, which is exactly what [`solve`] does.
//!
//! Three of the ten are depreciation and share nothing with the rest: `SLN`, `SYD` and
//! `DDB`, the last transcribed from §6.12.14's own pseudocode so that a non-integer `Period`
//! behaves as the spec says rather than as a guess.

use super::super::value::{FormulaError, Value};
use super::Args;

type Answer = Result<Value, FormulaError>;

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    Some(match name {
        "PV" => pv(args),
        "FV" => fv(args),
        "PMT" => pmt(args),
        "NPER" => nper(args),
        "RATE" => rate(args),
        "NPV" => npv(args),
        "IRR" => irr(args),
        "SLN" => sln(args),
        "SYD" => syd(args),
        "DDB" => ddb(args),
        _ => return None,
    })
}

/// The annuity equation of §6.12.41, as a residual that is zero at a solution.
fn annuity(rate: f64, nper: f64, payment: f64, pv: f64, fv: f64, pay_type: f64) -> f64 {
    if rate == 0.0 {
        pv + payment * nper + fv
    } else {
        pv * (1.0 + growth(rate, nper)) + payment * series(rate, nper, pay_type) + fv
    }
}

/// `(1+Rate)^Nper − 1`, without ever forming `1+Rate`.
///
/// That sum is where the digits go: a monthly rate of `0.0199/12` is 0.00166, and adding it
/// to 1 throws away the bottom three digits of it before `powf` has begun. `ln_1p` and
/// `exp_m1` are the same expression with neither cancellation, and the difference is visible
/// at the fifteen significant digits a cached value is compared at — `PMT(0.0199/12;36;25000)`
/// is `−715.955334437392`, and the naive spelling gets `−715.955334437399`.
fn growth(rate: f64, nper: f64) -> f64 {
    (nper * rate.ln_1p()).exp_m1()
}

/// The payment's coefficient: `(1 + Rate·PayType)·((1+Rate)^Nper − 1)/Rate`.
fn series(rate: f64, nper: f64, pay_type: f64) -> f64 {
    (1.0 + rate * pay_type) * growth(rate, nper) / rate
}

/// The trailing `[ ; Fv = 0 ] [ ; PayType = 0 ]` the five annuity functions share.
///
/// §6.12.41 and its siblings define PayType as a flag — "0 if payments are due at the end of
/// the period; 1 if they are due at the beginning" — and say nothing about any other number.
/// Reading it as the flag it is, rather than as a coefficient, is what makes `PayType = 2`
/// the same as `1` instead of an extra period's interest.
fn tail(args: &mut Args, first: usize) -> Result<(f64, f64), FormulaError> {
    let fv = if args.omitted(first) {
        0.0
    } else {
        args.number(first)?
    };
    let pay_type = if args.omitted(first + 1) {
        0.0
    } else {
        f64::from(args.number(first + 1)? != 0.0)
    };
    Ok((fv, pay_type))
}

/// §6.12.41 `PV( Rate ; Nper ; Payment [ ; Fv [ ; PayType ] ] )`.
fn pv(args: &mut Args) -> Answer {
    args.arity(3..=5)?;
    let (rate, nper, payment) = (args.number(0)?, args.number(1)?, args.number(2)?);
    let (fv, pay_type) = tail(args, 3)?;
    Ok(Value::number(if rate == 0.0 {
        -fv - payment * nper
    } else {
        -(fv + payment * series(rate, nper, pay_type)) / (1.0 + growth(rate, nper))
    }))
}

/// §6.12.20 `FV( Rate ; Nper ; Payment [ ; Pv [ ; PayType ] ] )`.
fn fv(args: &mut Args) -> Answer {
    args.arity(3..=5)?;
    let (rate, nper, payment) = (args.number(0)?, args.number(1)?, args.number(2)?);
    let (pv, pay_type) = tail(args, 3)?;
    Ok(Value::number(if rate == 0.0 {
        -pv - payment * nper
    } else {
        -(pv * (1.0 + growth(rate, nper)) + payment * series(rate, nper, pay_type))
    }))
}

/// §6.12.36 `PMT( Rate ; Nper ; Pv [ ; Fv [ ; PayType ] ] )`. Constraint: `Nper > 0`.
fn pmt(args: &mut Args) -> Answer {
    args.arity(3..=5)?;
    let (rate, nper, pv) = (args.number(0)?, args.number(1)?, args.number(2)?);
    let (fv, pay_type) = tail(args, 3)?;
    if nper <= 0.0 {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(if rate == 0.0 {
        (-pv - fv) / nper
    } else {
        -(fv + pv * (1.0 + growth(rate, nper))) / series(rate, nper, pay_type)
    }))
}

/// §6.12.29 `NPER( Rate ; Payment ; Pv [ ; Fv [ ; PayType ] ] )`.
///
/// Solved in closed form: with `k = Payment·(1+Rate·PayType)/Rate`, the equation is
/// `(1+Rate)^Nper·(Pv + k) = k − Fv`, so `Nper` is one logarithm. A non-positive ratio has
/// no real answer and is `#NUM!` rather than a `NaN`.
fn nper(args: &mut Args) -> Answer {
    args.arity(3..=5)?;
    let (rate, payment, pv) = (args.number(0)?, args.number(1)?, args.number(2)?);
    let (fv, pay_type) = tail(args, 3)?;
    if rate == 0.0 {
        if payment == 0.0 {
            return Err(FormulaError::DivZero);
        }
        return Ok(Value::number((-pv - fv) / payment));
    }
    let k = payment * (1.0 + rate * pay_type) / rate;
    // A ratio of zero, negative or NaN has no real logarithm — an investment that never
    // reaches its future value, which is `#NUM!` rather than a payment count.
    let ratio = (k - fv) / (pv + k);
    if matches!(
        ratio.partial_cmp(&0.0),
        None | Some(std::cmp::Ordering::Less) | Some(std::cmp::Ordering::Equal)
    ) {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(ratio.ln() / (1.0 + rate).ln()))
}

/// §6.12.42 `RATE( Nper ; Payment ; Pv [ ; Fv [ ; PayType [ ; Guess ] ] ] )`.
///
/// "If Nper is 0 or less than 0, the result is an Error" is the section's one constraint.
fn rate(args: &mut Args) -> Answer {
    args.arity(3..=6)?;
    let (nper, payment, pv) = (args.number(0)?, args.number(1)?, args.number(2)?);
    let (fv, pay_type) = tail(args, 3)?;
    let guess = if args.omitted(5) {
        0.1
    } else {
        args.number(5)?
    };
    if nper <= 0.0 {
        return Err(FormulaError::Num);
    }
    let scale = pv.abs() + fv.abs() + payment.abs() * nper;
    let rate = solve(guess, scale, |r| {
        annuity(r, nper, payment, pv, fv, pay_type)
    })?;
    // A rate at or below −100% is not an interest rate but the far side of the equation's
    // pole, where `(1+Rate)^Nper` changes sign. LibreOffice reports non-convergence there,
    // and a root the iteration only reached by crossing the pole is not the answer asked for.
    if rate <= -1.0 {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(rate))
}

/// §6.12.30 `NPV( Rate ; { Values }+ )`, discounting the first value by one period.
fn npv(args: &mut Args) -> Answer {
    args.arity(2..=usize::MAX)?;
    let rate = args.number(0)?;
    let values = args.numbers_range(1..args.len())?;
    let mut total = 0.0;
    for (i, value) in values.iter().enumerate() {
        total += value / (1.0 + rate).powi(i as i32 + 1);
    }
    Ok(Value::number(total))
}

/// §6.12.24 `IRR( Values [ ; Guess ] )` — the rate at which `NPV` of the same flows is zero.
///
/// The flows start at period 0 here, unlike `NPV`'s: `IRR`'s first value is the investment
/// itself, which is why the two functions are inverses only when `NPV` is handed the tail.
fn irr(args: &mut Args) -> Answer {
    args.arity(1..=2)?;
    let values = args.numbers_range(0..1)?;
    let guess = if args.omitted(1) {
        0.1
    } else {
        args.number(1)?
    };
    if values.is_empty() {
        return Err(FormulaError::Num);
    }
    let scale: f64 = values.iter().map(|v| v.abs()).sum();
    Ok(Value::number(solve(guess, scale, |r| {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| v / (1.0 + r).powi(i as i32))
            .sum()
    })?))
}

/// A rate at which `f` is zero, sought from `guess` and then, failing that, anywhere in the
/// domain. `scale` is the size of the cash flows involved, which is what makes "`f` is zero"
/// answerable at all: a residual of one currency unit is a solved equation for a mortgage and
/// a badly missed one for a ten-unit annuity.
///
/// [`secant`] alone would do if every equation had one root. §6.12.24's `Guess` exists because
/// they do not, so the iteration runs from there first and its answer wins when it is a root —
/// that is the one that agrees with LibreOffice. Only when it lands on nothing (the annuity
/// equation's pole at −100% attracts it) does [`scan`] sweep the domain for a sign change.
///
/// §6.12.24 permits returning an Error when no root is found, and both halves failing is that.
fn solve(guess: f64, scale: f64, f: impl Fn(f64) -> f64) -> Result<f64, FormulaError> {
    // Half a ULP of the flows themselves is what the arithmetic can distinguish; the margin
    // above it is for the cancellation inside the equation, not for a sloppy answer.
    let tolerance = 1e-7 * scale.max(1.0);
    let solved = |x: f64| f(x).abs() <= tolerance;
    let from_guess = secant(guess, &f);
    if solved(from_guess) {
        return Ok(from_guess);
    }
    match scan(&f) {
        Some(x) if solved(x) => Ok(x),
        _ => Err(FormulaError::Num),
    }
}

/// The secant method from `guess`, returning the closest it came to a root.
///
/// The stopping rule is the *step*, not the residual: a rate is compared against
/// LibreOffice's at fifteen significant digits (loop C's rule, `doc/ods-format.md` §3.4), so
/// stopping when the residual merely looks small leaves the last four digits wrong. Secant
/// converges superlinearly, so running it until the iterate stops moving costs a handful of
/// extra evaluations and buys the whole mantissa.
///
/// ponytail: secant rather than Newton, because the derivative of the annuity equation in
/// `Rate` is an expression nobody needs written twice, and a numerical slope converges just
/// as well here.
fn secant(guess: f64, f: &impl Fn(f64) -> f64) -> f64 {
    let nudge = if guess == 0.0 { 1e-6 } else { guess * 1e-6 };
    let (mut x0, mut x1) = (guess, guess + nudge);
    let (mut y0, mut y1) = (f(x0), f(x1));
    let mut best = (y1.abs(), x1);
    for _ in 0..100 {
        if y1 == 0.0 {
            return x1;
        }
        // A flat secant has nowhere to step to, and a non-finite one has left the domain —
        // `(1+Rate)^Nper` is NaN below −1 for a fractional Nper.
        if y1 == y0 || !y1.is_finite() {
            break;
        }
        let next = x1 - y1 * (x1 - x0) / (y1 - y0);
        if !next.is_finite() {
            break;
        }
        let step = (next - x1).abs();
        (x0, y0) = (x1, y1);
        (x1, y1) = (next, f(next));
        if y1.abs() < best.0 {
            best = (y1.abs(), x1);
        }
        if step <= f64::EPSILON * x1.abs().max(1.0) {
            return x1;
        }
    }
    best.1
}

/// A root found by sweeping the whole domain for a sign change and bisecting the first one.
///
/// The probes are a geometric ladder in `1 + Rate` rather than an even spacing, because the
/// interesting rates are not evenly spaced: `−1 + 10^−9` and `10000` are both plausible, and
/// a linear sweep fine enough for the first would take a billion steps to reach the second.
fn scan(f: &impl Fn(f64) -> f64) -> Option<f64> {
    let probe = |k: i32| -1.0 + 10f64.powf(f64::from(k) / 10.0);
    let (mut lo, mut y_lo) = (probe(-90), f(probe(-90)));
    for k in -89..=40 {
        let (hi, y_hi) = (probe(k), f(probe(k)));
        if y_hi == 0.0 {
            return Some(hi);
        }
        if y_lo.is_finite() && y_hi.is_finite() && (y_lo < 0.0) != (y_hi < 0.0) {
            return Some(bisect(lo, hi, y_lo, f));
        }
        (lo, y_lo) = (hi, y_hi);
    }
    None
}

/// Bisection down to the last representable double between `lo` and `hi`, which bracket a root.
fn bisect(mut lo: f64, mut hi: f64, mut y_lo: f64, f: &impl Fn(f64) -> f64) -> f64 {
    loop {
        let mid = lo / 2.0 + hi / 2.0;
        if mid <= lo || mid >= hi {
            return mid;
        }
        let y_mid = f(mid);
        if y_mid == 0.0 || !y_mid.is_finite() {
            return mid;
        }
        if (y_mid < 0.0) == (y_lo < 0.0) {
            (lo, y_lo) = (mid, y_mid);
        } else {
            hi = mid;
        }
    }
}

/// §6.12.45 `SLN( Cost ; Salvage ; LifeTime )` — straight-line depreciation.
fn sln(args: &mut Args) -> Answer {
    args.arity(3..=3)?;
    let (cost, salvage, life) = (args.number(0)?, args.number(1)?, args.number(2)?);
    if life == 0.0 {
        return Err(FormulaError::DivZero);
    }
    Ok(Value::number((cost - salvage) / life))
}

/// §6.12.46 `SYD( Cost ; Salvage ; LifeTime ; Period )` — sum-of-the-years'-digits.
fn syd(args: &mut Args) -> Answer {
    args.arity(4..=4)?;
    let (cost, salvage) = (args.number(0)?, args.number(1)?);
    let (life, period) = (args.number(2)?, args.number(3)?);
    if life <= 0.0 {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(
        (cost - salvage) * (life + 1.0 - period) * 2.0 / ((life + 1.0) * life),
    ))
}

/// §6.12.14 `DDB( Cost ; Salvage ; LifeTime ; Period [ ; DeclinationFactor = 2 ] )`.
///
/// Transcribed from the section's own pseudocode rather than from the summing description
/// beside it: the two agree on integer periods, and only this one answers a fractional
/// `Period`. The `rate ≥ 1` branch is the degenerate case where the whole asset depreciates
/// in the first period.
fn ddb(args: &mut Args) -> Answer {
    args.arity(4..=5)?;
    let (cost, salvage) = (args.number(0)?, args.number(1)?);
    let (life, period) = (args.number(2)?, args.number(3)?);
    let factor = if args.omitted(4) {
        2.0
    } else {
        args.number(4)?
    };
    if cost < 0.0
        || salvage < 0.0
        || salvage > cost
        || period < 1.0
        || period > life
        || factor <= 0.0
    {
        return Err(FormulaError::Num);
    }
    let rate = factor / life;
    let (rate, old) = if rate >= 1.0 {
        (1.0, if period == 1.0 { cost } else { 0.0 })
    } else {
        (rate, cost * (1.0 - rate).powf(period - 1.0))
    };
    let new = cost * (1.0 - rate).powf(period);
    let depreciation = if new < salvage {
        old - salvage
    } else {
        old - new
    };
    Ok(Value::number(depreciation.max(0.0)))
}

#[cfg(test)]
mod tests {
    use super::super::super::eval::{Address, Engine};
    use super::super::super::value::{FormulaError, Value};
    use crate::model::{CellValue, Document, Pos, Sheet};

    /// A1:A3 is a cash flow — an investment of 100 and two years of 60 — because `IRR` takes
    /// a NumberSequence, so a second scalar argument would be its `Guess` and not a flow.
    fn eval(formula: &str) -> Value {
        let mut sheet = Sheet::new("Sheet1");
        for (row, value) in [-100.0, 60.0, 60.0].into_iter().enumerate() {
            sheet.set(Pos::new(row as u32, 0), CellValue::Number(value));
        }
        let document = Document {
            sheets: vec![sheet],
            ..Default::default()
        };
        Engine::new(&document).eval(formula, Address::new(0, Pos::new(20, 20)))
    }

    fn number(formula: &str) -> f64 {
        match eval(formula) {
            Value::Number(n) => n,
            other => panic!("{formula} evaluated to {other:?}"),
        }
    }

    /// Every leg of the one equation, checked against itself: a `PV` and its `FV`, and the
    /// `PMT`, `NPER` and `RATE` that reproduce the same annuity.
    #[test]
    fn the_five_annuity_functions_invert_each_other() {
        // 10 years at 5%, paying 1000 a year, is worth 7721.73 today (§6.12.41's equation).
        let pv = number("=PV(0.05;10;-1000)");
        assert!((pv - 7721.7349).abs() < 1e-4, "{pv}");
        assert!((number("=PMT(0.05;10;7721.7349)") + 1000.0).abs() < 1e-4);
        assert!((number("=NPER(0.05;-1000;7721.7349)") - 10.0).abs() < 1e-6);
        assert!((number("=RATE(10;-1000;7721.7349)") - 0.05).abs() < 1e-8);
        // Payments at the start of each period are worth one period's interest more.
        assert!((number("=PV(0.05;10;-1000;0;1)") - pv * 1.05).abs() < 1e-6);
        // Rate 0 is the limit each section states, not a division by zero.
        assert_eq!(number("=PV(0;10;-100)"), 1000.0);
        assert_eq!(number("=FV(0;10;-100)"), 1000.0);
        assert_eq!(number("=PMT(0;10;1000)"), -100.0);
        assert_eq!(number("=NPER(0;-100;1000)"), 10.0);
        assert_eq!(eval("=RATE(0;-100;1000)"), Value::Error(FormulaError::Num));
        assert_eq!(eval("=PMT(0.05;0;1000)"), Value::Error(FormulaError::Num));
    }

    #[test]
    fn npv_discounts_from_period_one_and_irr_from_period_zero() {
        // §6.12.30: the first value is already discounted once.
        assert!((number("=NPV(0.1;100)") - 90.909_090_9).abs() < 1e-6);
        // §6.12.24: IRR is the rate at which NPV of the same flows is zero — and the flows
        // start one period earlier, which is why the investment is not discounted.
        let irr = number("=IRR([.A1:.A3])");
        assert!((irr - 0.130_662).abs() < 1e-6, "{irr}");
        assert!(number(&format!("=NPV({irr};[.A2:.A3])-100")).abs() < 1e-6);
        // A flow that never crosses zero has no rate, and §6.12.24 permits saying so.
        assert_eq!(eval("=IRR([.A2:.A3])"), Value::Error(FormulaError::Num));
    }

    #[test]
    fn depreciation_follows_sections_45_46_and_14() {
        assert_eq!(number("=SLN(1000;100;10)"), 90.0);
        assert_eq!(number("=SYD(1000;100;10;1)"), 900.0 * 10.0 / 55.0);
        assert_eq!(number("=SYD(1000;100;10;10)"), 900.0 / 55.0);
        // Double declining: 20% of 1000, then 20% of what is left.
        assert_eq!(number("=DDB(1000;100;10;1)"), 200.0);
        assert!((number("=DDB(1000;100;10;2)") - 160.0).abs() < 1e-9);
        // The last periods stop at Salvage rather than depreciating through it.
        assert_eq!(number("=DDB(1000;900;10;2)"), 0.0);
        assert_eq!(number("=DDB(1000;0;1;1)"), 1000.0); // rate ≥ 1
        // Constraints of §6.12.14, each an Error rather than a negative depreciation.
        assert_eq!(
            eval("=DDB(1000;100;10;11)"),
            Value::Error(FormulaError::Num)
        );
        assert_eq!(eval("=DDB(100;1000;10;1)"), Value::Error(FormulaError::Num));
        assert_eq!(
            eval("=DDB(1000;100;10;1;0)"),
            Value::Error(FormulaError::Num)
        );
    }
}
