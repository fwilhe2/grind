// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.18's distributions: the standard normal and its inverse, and the gamma distribution under
//! Excel's name. `erfc` and the log-gamma function are `libm`'s; the rest is here.

use super::super::value::{FormulaError, Value};
use super::Args;

type Answer = Result<Value, FormulaError>;

/// §6.18.55 `LEGACY.NORMSINV(P)`, `0 < P < 1`.
pub(super) fn normsinv(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    let p = args.number(0)?;
    if !(p > 0.0 && p < 1.0) {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(inverse_normal(p)))
}

/// §6.18.54 `LEGACY.NORMSDIST(X)`: Φ(X).
pub(super) fn normsdist(args: &mut Args) -> Answer {
    args.arity(1..=1)?;
    Ok(Value::number(normal_cdf(args.number(0)?)))
}

/// §6.18.52 `NORMDIST( X ; Mean ; StdDev [ ; Cumulative = TRUE ] )`, and Excel's `NORM.DIST`
/// with `required` — the same with its last parameter required. A `StdDev` that is not positive
/// is `#NUM!`.
pub(super) fn normdist(args: &mut Args, required: bool) -> Answer {
    args.arity(if required { 4..=4 } else { 3..=4 })?;
    let (x, mean, sd) = (args.number(0)?, args.number(1)?, args.number(2)?);
    let cumulative = args.omitted(3) || args.logical(3)?;
    if sd <= 0.0 {
        return Err(FormulaError::Num);
    }
    let z = (x - mean) / sd;
    Ok(Value::number(if cumulative {
        normal_cdf(z)
    } else {
        (-z * z / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt() / sd
    }))
}

/// §6.18.53 `NORMINV( P ; Mean ; StdDev )`, `0 < P < 1` and a positive `StdDev`.
pub(super) fn norminv(args: &mut Args) -> Answer {
    args.arity(3..=3)?;
    let (p, mean, sd) = (args.number(0)?, args.number(1)?, args.number(2)?);
    if !(p > 0.0 && p < 1.0) || sd <= 0.0 {
        return Err(FormulaError::Num);
    }
    Ok(Value::number(mean + sd * inverse_normal(p)))
}

/// §6.18.71 `STANDARDIZE( X ; Mean ; StdDev )`: `(X − Mean) / StdDev`; a zero `StdDev` divides by
/// zero and a negative one is `#NUM!`, as the oracle answers.
pub(super) fn standardize(args: &mut Args) -> Answer {
    args.arity(3..=3)?;
    let (x, mean, sd) = (args.number(0)?, args.number(1)?, args.number(2)?);
    if sd == 0.0 {
        return Err(FormulaError::DivZero);
    }
    if sd < 0.0 {
        return Err(FormulaError::Num);
    }
    Ok(Value::number((x - mean) / sd))
}

/// Excel's `NORM.S.DIST(Z; Cumulative)`: §6.18.52's `NORMDIST(Z; 0; 1; Cumulative)`, with the
/// last parameter required.
pub(super) fn norm_s_dist(args: &mut Args) -> Answer {
    args.arity(2..=2)?;
    let z = args.number(0)?;
    Ok(Value::number(if args.logical(1)? {
        normal_cdf(z)
    } else {
        (-z * z / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt()
    }))
}

/// Excel's `GAMMA.DIST(X; α; β; Cumulative)`: §6.18.31's `GAMMADIST`, with the last parameter
/// required and a negative `X` refused rather than 0, as the oracle's function of that name does.
pub(super) fn gamma_dist(args: &mut Args) -> Answer {
    args.arity(4..=4)?;
    let x = args.number(0)?;
    let alpha = args.number(1)?;
    let beta = args.number(2)?;
    let cumulative = args.logical(3)?;
    if x < 0.0 || alpha <= 0.0 || beta <= 0.0 {
        return Err(FormulaError::Num);
    }
    let x = x / beta;
    if cumulative {
        return Ok(Value::number(lower_regularized_gamma(alpha, x)));
    }
    if x == 0.0 {
        // `x^(α-1)` at zero: infinite below α = 1, one at it, zero above.
        return match alpha.partial_cmp(&1.0) {
            Some(std::cmp::Ordering::Less) => Err(FormulaError::DivZero),
            Some(std::cmp::Ordering::Equal) => Ok(Value::Number(1.0 / beta)),
            _ => Ok(Value::Number(0.0)),
        };
    }
    Ok(Value::number(
        ((alpha - 1.0) * x.ln() - x - libm::lgamma(alpha)).exp() / beta,
    ))
}

/// Φ(x), accurate in both tails: `erfc` of the side that is small.
fn normal_cdf(x: f64) -> f64 {
    0.5 * libm::erfc(-x / std::f64::consts::SQRT_2)
}

/// Φ⁻¹(p) for `0 < p < 1`: Abramowitz and Stegun's 26.2.23 to four places, then Halley's method
/// on Φ, which triples the correct digits each step. Solved in the lower half, where Φ keeps
/// its precision, and mirrored; `1 - p` is exact for `p ≥ 0.5`.
fn inverse_normal(p: f64) -> f64 {
    if p > 0.5 {
        return -inverse_normal(1.0 - p);
    }
    if p == 0.5 {
        return 0.0;
    }
    let t = (-2.0 * p.ln()).sqrt();
    let mut x = -(t
        - (2.515_517 + 0.802_853 * t + 0.010_328 * t * t)
            / (1.0 + 1.432_788 * t + 0.189_269 * t * t + 0.001_308 * t * t * t));
    for _ in 0..8 {
        let density = (-x * x / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt();
        let step = (normal_cdf(x) - p) / density;
        let next = x - step / (1.0 + x * step / 2.0);
        if (next - x).abs() <= f64::EPSILON * x.abs() {
            return next;
        }
        x = next;
    }
    x
}

/// P(a, x), the regularized lower incomplete gamma function: its series below `a + 1`, and one
/// minus the continued fraction for Q above, where each converges fast.
fn lower_regularized_gamma(a: f64, x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let prefix = (a * x.ln() - x - libm::lgamma(a)).exp();
    if x < a + 1.0 {
        let (mut term, mut sum, mut n) = (1.0 / a, 1.0 / a, a);
        while term.abs() > sum.abs() * f64::EPSILON {
            n += 1.0;
            term *= x / n;
            sum += term;
        }
        return (prefix * sum).min(1.0);
    }
    // Lentz's method on Q's continued fraction.
    let tiny = f64::MIN_POSITIVE / f64::EPSILON;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / tiny;
    let mut d = 1.0 / b;
    let mut h = d;
    let mut i = 1.0;
    loop {
        let an = -i * (i - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < tiny {
            d = tiny;
        }
        c = b + an / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() <= f64::EPSILON {
            break;
        }
        i += 1.0;
    }
    (1.0 - prefix * h).max(0.0)
}
