// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a tape line's number is — a plain number, a percentage or an amount of one currency —
//! carried through its arithmetic the way a spreadsheet carries a number format: `€ × 19%` is
//! euros, `€ ÷ €` a plain ratio, and euros plus dollars no answer at all (`doc/calc.md` §4).
//! The value stays a number; the kind only decides how it is shown and formatted.

use crate::formula::lex::Op;
use crate::formula::parse::Expr;
use crate::numfmt::{self, Format, Kind as Family};

/// One ISO 4217 currency: its code, the symbol it is shown with, and its minor-unit digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Currency {
    pub code: &'static str,
    symbol: &'static str,
    decimals: u8,
}

/// Every active ISO 4217 code. A currency with no symbol of its own is shown by its code.
const CURRENCIES: &[(&str, &str, u8)] = &[
    ("AED", "AED", 2),
    ("AFN", "AFN", 2),
    ("ALL", "ALL", 2),
    ("AMD", "AMD", 2),
    ("ANG", "ANG", 2),
    ("AOA", "AOA", 2),
    ("ARS", "ARS", 2),
    ("AUD", "AUD", 2),
    ("AWG", "AWG", 2),
    ("AZN", "AZN", 2),
    ("BAM", "BAM", 2),
    ("BBD", "BBD", 2),
    ("BDT", "BDT", 2),
    ("BGN", "BGN", 2),
    ("BHD", "BHD", 3),
    ("BIF", "BIF", 0),
    ("BMD", "BMD", 2),
    ("BND", "BND", 2),
    ("BOB", "BOB", 2),
    ("BRL", "BRL", 2),
    ("BSD", "BSD", 2),
    ("BTN", "BTN", 2),
    ("BWP", "BWP", 2),
    ("BYN", "BYN", 2),
    ("BZD", "BZD", 2),
    ("CAD", "CAD", 2),
    ("CDF", "CDF", 2),
    ("CHF", "CHF", 2),
    ("CLP", "CLP", 0),
    ("CNY", "CNY", 2),
    ("COP", "COP", 2),
    ("CRC", "CRC", 2),
    ("CUP", "CUP", 2),
    ("CVE", "CVE", 2),
    ("CZK", "CZK", 2),
    ("DJF", "DJF", 0),
    ("DKK", "DKK", 2),
    ("DOP", "DOP", 2),
    ("DZD", "DZD", 2),
    ("EGP", "EGP", 2),
    ("ERN", "ERN", 2),
    ("ETB", "ETB", 2),
    ("EUR", "€", 2),
    ("FJD", "FJD", 2),
    ("FKP", "FKP", 2),
    ("GBP", "£", 2),
    ("GEL", "GEL", 2),
    ("GHS", "GHS", 2),
    ("GIP", "GIP", 2),
    ("GMD", "GMD", 2),
    ("GNF", "GNF", 0),
    ("GTQ", "GTQ", 2),
    ("GYD", "GYD", 2),
    ("HKD", "HKD", 2),
    ("HNL", "HNL", 2),
    ("HTG", "HTG", 2),
    ("HUF", "HUF", 2),
    ("IDR", "IDR", 2),
    ("ILS", "ILS", 2),
    ("INR", "INR", 2),
    ("IQD", "IQD", 3),
    ("IRR", "IRR", 2),
    ("ISK", "ISK", 0),
    ("JMD", "JMD", 2),
    ("JOD", "JOD", 3),
    ("JPY", "¥", 0),
    ("KES", "KES", 2),
    ("KGS", "KGS", 2),
    ("KHR", "KHR", 2),
    ("KMF", "KMF", 0),
    ("KPW", "KPW", 2),
    ("KRW", "KRW", 0),
    ("KWD", "KWD", 3),
    ("KYD", "KYD", 2),
    ("KZT", "KZT", 2),
    ("LAK", "LAK", 2),
    ("LBP", "LBP", 2),
    ("LKR", "LKR", 2),
    ("LRD", "LRD", 2),
    ("LSL", "LSL", 2),
    ("LYD", "LYD", 3),
    ("MAD", "MAD", 2),
    ("MDL", "MDL", 2),
    ("MGA", "MGA", 2),
    ("MKD", "MKD", 2),
    ("MMK", "MMK", 2),
    ("MNT", "MNT", 2),
    ("MOP", "MOP", 2),
    ("MRU", "MRU", 2),
    ("MUR", "MUR", 2),
    ("MVR", "MVR", 2),
    ("MWK", "MWK", 2),
    ("MXN", "MXN", 2),
    ("MYR", "MYR", 2),
    ("MZN", "MZN", 2),
    ("NAD", "NAD", 2),
    ("NGN", "NGN", 2),
    ("NIO", "NIO", 2),
    ("NOK", "NOK", 2),
    ("NPR", "NPR", 2),
    ("NZD", "NZD", 2),
    ("OMR", "OMR", 3),
    ("PAB", "PAB", 2),
    ("PEN", "PEN", 2),
    ("PGK", "PGK", 2),
    ("PHP", "PHP", 2),
    ("PKR", "PKR", 2),
    ("PLN", "PLN", 2),
    ("PYG", "PYG", 0),
    ("QAR", "QAR", 2),
    ("RON", "RON", 2),
    ("RSD", "RSD", 2),
    ("RUB", "RUB", 2),
    ("RWF", "RWF", 0),
    ("SAR", "SAR", 2),
    ("SBD", "SBD", 2),
    ("SCR", "SCR", 2),
    ("SDG", "SDG", 2),
    ("SEK", "SEK", 2),
    ("SGD", "SGD", 2),
    ("SHP", "SHP", 2),
    ("SLE", "SLE", 2),
    ("SOS", "SOS", 2),
    ("SRD", "SRD", 2),
    ("SSP", "SSP", 2),
    ("STN", "STN", 2),
    ("SVC", "SVC", 2),
    ("SYP", "SYP", 2),
    ("SZL", "SZL", 2),
    ("THB", "THB", 2),
    ("TJS", "TJS", 2),
    ("TMT", "TMT", 2),
    ("TND", "TND", 3),
    ("TOP", "TOP", 2),
    ("TRY", "TRY", 2),
    ("TTD", "TTD", 2),
    ("TWD", "TWD", 2),
    ("TZS", "TZS", 2),
    ("UAH", "UAH", 2),
    ("UGX", "UGX", 0),
    ("USD", "$", 2),
    ("UYU", "UYU", 2),
    ("UZS", "UZS", 2),
    ("VES", "VES", 2),
    ("VND", "VND", 0),
    ("VUV", "VUV", 0),
    ("WST", "WST", 2),
    ("XAF", "XAF", 0),
    ("XCD", "XCD", 2),
    ("XOF", "XOF", 0),
    ("XPF", "XPF", 0),
    ("YER", "YER", 2),
    ("ZAR", "ZAR", 2),
    ("ZMW", "ZMW", 2),
    ("ZWG", "ZWG", 2),
];

impl Currency {
    /// The currency an ISO code names, in any case, or one of the four symbols a keyboard may
    /// have: `€`, `$` (US dollars), `£`, `¥` (yen).
    pub(crate) fn named(word: &str) -> Option<Currency> {
        let code = match word {
            "€" => "EUR",
            "$" => "USD",
            "£" => "GBP",
            "¥" => "JPY",
            _ => word,
        };
        CURRENCIES
            .iter()
            .find(|(c, _, _)| c.eq_ignore_ascii_case(code))
            .map(|&(code, symbol, decimals)| Currency {
                code,
                symbol,
                decimals,
            })
    }

    /// The format an amount of it is shown in: its symbol, its minor unit, grouped.
    pub(crate) fn format(self) -> Format {
        numfmt::preset(Family::Currency, self.decimals, true, self.symbol)
    }
}

/// What a line's number is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Plain,
    Percent,
    Money(Currency),
}

impl Kind {
    /// The format a value of this kind is shown and promoted in; `None` for a plain number, which
    /// keeps the tape's own fifteen digits. A percentage shows as many decimals as it has, up to
    /// `most`.
    pub(crate) fn format(self, value: f64, most: u8) -> Option<Format> {
        match self {
            Kind::Plain => None,
            Kind::Money(currency) => Some(currency.format()),
            Kind::Percent => {
                // Fifteen significant digits first, so 0.19 is 19 and not 19.000000000000004.
                let hundred: f64 = format!("{:.14e}", value * 100.0).parse().unwrap_or(0.0);
                let shown = format!("{hundred}");
                let decimals = shown
                    .split_once('.')
                    .map_or(0, |(_, fraction)| fraction.len())
                    .min(usize::from(most));
                Some(numfmt::preset(
                    Family::Percentage,
                    u8::try_from(decimals).unwrap_or(most),
                    false,
                    "",
                ))
            }
        }
    }
}

/// Why a line's kinds do not go together.
pub(crate) fn mixed(a: Currency, b: Currency) -> String {
    format!(
        "{} and {} do not add up: convert one first with a rate line, such as {}_per_{} = 0.9 {}",
        a.code,
        b.code,
        a.code.to_lowercase(),
        b.code.to_lowercase(),
        a.code
    )
}

/// The functions whose answer is the kind their numbers share: a sum of euros is euros.
const SAME_KIND: &[&str] = &[
    "ABS",
    "AVERAGE",
    "CEILING",
    "FLOOR",
    "INT",
    "LARGE",
    "MAX",
    "MEDIAN",
    "MIN",
    "MROUND",
    "ROUND",
    "ROUNDDOWN",
    "ROUNDUP",
    "SMALL",
    "SUM",
    "TRUNC",
];

/// The kind `expr` comes to. `literal` gives each number literal's currency, in the order the
/// literals stand in the text; `line` the kind of the line a reference or a name reads.
pub(crate) fn of(
    expr: &Expr,
    literal: &mut dyn FnMut() -> Option<Currency>,
    line: &dyn Fn(&Expr) -> Option<Kind>,
) -> Result<Kind, String> {
    Ok(match expr {
        Expr::Number(_) => literal().map_or(Kind::Plain, Kind::Money),
        Expr::Ref(_) | Expr::Name(_) => line(expr).unwrap_or(Kind::Plain),
        Expr::Paren(inner) | Expr::Prefix(_, inner) => of(inner, literal, line)?,
        Expr::Postfix(_, inner) => match of(inner, literal, line)? {
            Kind::Plain | Kind::Percent => Kind::Percent,
            money @ Kind::Money(_) => money,
        },
        Expr::Binary(op, a, b) => {
            let (a, b) = (of(a, literal, line)?, of(b, literal, line)?);
            binary(*op, a, b)?
        }
        Expr::Call { name, args } => {
            let mut kinds = Vec::new();
            for arg in args {
                kinds.push(of(arg, literal, line)?);
            }
            let name = name.to_uppercase();
            if SAME_KIND.contains(&name.as_str()) {
                // `ROUND(price; 2)`: the digits are not part of the amount.
                let counted = if name.starts_with("ROUND") || name == "MROUND" || name == "TRUNC" {
                    &kinds[..kinds.len().min(1)]
                } else {
                    &kinds[..]
                };
                counted
                    .iter()
                    .try_fold(None, |shared: Option<Kind>, k| match shared {
                        None => Ok(Some(*k)),
                        Some(s) => binary(Op::Add, s, *k).map(Some),
                    })?
                    .unwrap_or(Kind::Plain)
            } else {
                Kind::Plain
            }
        }
        _ => Kind::Plain,
    })
}

fn binary(op: Op, a: Kind, b: Kind) -> Result<Kind, String> {
    use Kind::{Money, Percent, Plain};
    Ok(match op {
        Op::Add | Op::Sub => match (a, b) {
            (Money(x), Money(y)) if x != y => return Err(mixed(x, y)),
            (Money(x), _) | (_, Money(x)) => Money(x),
            (Percent, Percent) => Percent,
            _ => Plain,
        },
        Op::Mul => match (a, b) {
            (Money(_), Money(_)) => Plain,
            (Money(x), _) | (_, Money(x)) => Money(x),
            (Percent, Percent) => Percent,
            _ => Plain,
        },
        Op::Div => match (a, b) {
            (Money(x), Plain | Percent) => Money(x),
            (Percent, Plain) => Percent,
            _ => Plain,
        },
        // `#2:#4`: the lines' shared kind.
        Op::Range if a == b => a,
        Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => match (a, b) {
            (Money(x), Money(y)) if x != y => return Err(mixed(x, y)),
            _ => Plain,
        },
        _ => Plain,
    })
}
