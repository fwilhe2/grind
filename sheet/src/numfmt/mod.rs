// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Number formats — ODF 1.4 Part 3 §16.27, and `doc/ods-format.md` §5.2. **\[ODS\]**
//!
//! Display only: the cell's value is the value, so a formatted cell still sums, compares and
//! round-trips as its number. A format is the spec's own shape, an ordered sequence of literal and
//! value-bearing pieces, and there is no format-code string (`"#,##0.00"`, Excel's spelling)
//! anywhere in the core. No XML here: `odf::read` builds these and `odf::write` puts them back.
//!
//! Not modelled: month and weekday names are English whatever the locale (a name table wants
//! CLDR), and `number:truncate-on-overflow="false"` — hours wrap at 24, the attribute's default.

use serde::{Deserialize, Serialize};

use crate::formula::date;
use crate::formula::value::format_number;
use crate::locale::{self, Locale};
use crate::model::CellValue;

/// Which `number:*-style` element this is. The family decides how the value reaches the parts: a
/// percentage style renders 0.5 as `50`, with `%` an ordinary text part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    Number,
    Percentage,
    Currency,
    Date,
    Time,
    Boolean,
    Text,
}

/// One `number:embedded-text`: text placed among a number's digits (rng:7242). `position` is ODF's
/// count, measured (`doc/ods-format.md` §5.2): zero or more is how many integer digits stand to
/// its right, and a negative one reaches into the decimals, `-p` standing after `p - 1` of them
/// (LibreOffice's extension).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Embedded {
    pub position: i32,
    pub text: String,
}

/// One piece of a format, in document order.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Part {
    /// `number:text` — a literal separator, unit or currency word.
    Text(String),
    /// `number:number` — the numeric value itself.
    Number {
        /// `number:decimal-places`: how many fraction digits at most.
        decimals: u8,
        /// `number:min-decimal-places`: how many are kept even when they are zeros.
        min_decimals: u8,
        /// `number:min-integer-digits`: left-padded with zeros to reach this.
        min_int: u8,
        /// `number:grouping`: thousands separators.
        grouping: bool,
        /// `number:embedded-text` — text set among the digits themselves, as a telephone
        /// number's `(###) ###-####` or `#,##0.000" "###` sets it (`doc/ods-format.md` §5.2).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        embedded: Vec<Embedded>,
    },
    /// `number:currency-symbol` — the symbol is the element's content.
    Currency(String),
    /// `number:year`, long being four digits.
    Year {
        long: bool,
    },
    /// `number:month`; `textual` is `number:textual="true"`, a name rather than a number.
    Month {
        long: bool,
        textual: bool,
    },
    /// `number:day`, long being two digits.
    Day {
        long: bool,
    },
    /// `number:day-of-week`, long being the full name.
    DayOfWeek {
        long: bool,
    },
    Hours {
        long: bool,
    },
    Minutes {
        long: bool,
    },
    /// `number:seconds`, with `number:decimal-places` for sub-second precision.
    Seconds {
        long: bool,
        decimals: u8,
    },
    /// `number:am-pm` — its presence is what makes the hours a 12-hour clock.
    AmPm,
    /// `number:boolean` — the logical value as a word.
    Boolean,
    /// `number:text-content` — where the string value goes in a `number:text-style`.
    Content,
}

/// How a [`Map`] decides whether its branch applies — `style:condition="value()>=0"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// One `style:map`: a condition and the format to use when it holds (§5.1) — how ODF spells a
/// two-branch format such as a red negative or a dash for zero.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Map {
    pub op: Op,
    /// The value compared against, as text, so `Format` stays `Eq` and `Hash`.
    pub value: String,
    pub format: Format,
}

impl Map {
    /// Whether this condition says by itself that the value is negative — `value()<0` and
    /// `value()<=0`, the spelling a negative branch is written in. The branch it points at
    /// then spells the sign, or chooses not to (see [`Format::render_as`]).
    fn negative(&self) -> bool {
        matches!(self.op, Op::Lt | Op::Le)
            && self.value.trim().parse::<f64>().is_ok_and(|v| v <= 0.0)
    }

    fn holds(&self, n: f64) -> bool {
        let Ok(against) = self.value.trim().parse::<f64>() else {
            return false;
        };
        match self.op {
            Op::Lt => n < against,
            Op::Le => n <= against,
            Op::Gt => n > against,
            Op::Ge => n >= against,
            Op::Eq => n == against,
            Op::Ne => n != against,
        }
    }
}

impl Op {
    /// The spelling in `style:condition`. Two characters before one, or `<` would swallow
    /// `<=` and `<>`.
    pub const SPELLINGS: [(&'static str, Op); 6] = [
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("<>", Op::Ne),
        ("<", Op::Lt),
        (">", Op::Gt),
        ("=", Op::Eq),
    ];

    pub fn spelling(self) -> &'static str {
        Op::SPELLINGS
            .iter()
            .find(|(_, op)| *op == self)
            .map(|(text, _)| *text)
            .expect("every operator has a spelling")
    }
}

/// `style:condition="value()>=0"` as an operator and its operand's text. Anything else is `None`
/// and the map is dropped, rather than evaluating a condition this does not model.
pub fn parse_condition(condition: &str) -> Option<(Op, String)> {
    let rest = condition.trim().strip_prefix("value()")?.trim_start();
    let (text, op) = Op::SPELLINGS
        .iter()
        .find(|(text, _)| rest.starts_with(text))?;
    Some((*op, rest[text.len()..].trim().to_owned()))
}

/// One `number:*-style`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Format {
    pub kind: Kind,
    pub parts: Vec<Part>,
    /// `number:language` / `number:country` — which decides the decimal and grouping
    /// characters, and nothing else here (see `locale.rs`). `None` is an unmarked format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<Locale>,
    /// `style:map` branches, in document order — the first whose condition holds wins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub maps: Vec<Map>,
    /// `fo:color` on the style's own `style:text-properties`: the colour a value is drawn in
    /// through this format, over the cell's own — the red of a red negative belongs to the branch
    /// (`doc/ods-format.md` §5.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Serial 0 is 1899-12-30, a Saturday — the offset that makes `weekday` line up.
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

impl Format {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            parts: Vec::new(),
            locale: None,
            maps: Vec::new(),
            color: None,
        }
    }

    /// The colour `value` is drawn in through this format, if it says one: the branch's own for a
    /// number a `style:map` sends elsewhere, this style's otherwise. A text value takes only a
    /// text format's colour.
    pub fn color_of(&self, value: &CellValue) -> Option<&str> {
        match value {
            CellValue::Number(n) => match self.branch(*n) {
                Some(map) if !std::ptr::eq(&map.format, self) => map.format.color.as_deref(),
                _ => self.color.as_deref(),
            },
            CellValue::Text(_) if self.kind == Kind::Text => self.color.as_deref(),
            _ => None,
        }
    }

    /// The same format, in a locale — `1,234.50` becomes `1.234,50` without another part.
    #[must_use]
    pub fn in_locale(mut self, locale: Option<Locale>) -> Self {
        self.locale = locale;
        self
    }

    pub fn push(&mut self, part: Part) {
        self.parts.push(part);
    }

    /// The [`preset`] arguments closest to this format — its kind, its first `number:number`'s
    /// digits and grouping, its currency symbol — which a format picker and `sheet format --show`
    /// both show. A format with no numeric part reports `(0, false)`.
    pub fn preset_params(&self) -> (Kind, u8, bool, String) {
        let (decimals, grouping) = self
            .parts
            .iter()
            .find_map(|part| match part {
                Part::Number {
                    decimals, grouping, ..
                } => Some((*decimals, *grouping)),
                _ => None,
            })
            .unwrap_or((0, false));
        let symbol = self
            .parts
            .iter()
            .find_map(|part| match part {
                Part::Currency(symbol) => Some(symbol.clone()),
                _ => None,
            })
            .unwrap_or_default();
        (self.kind, decimals, grouping, symbol)
    }

    /// Whether [`preset`] (or [`datetime_preset`]) built from [`Format::preset_params`] is this
    /// format. False for a document's format the vocabulary cannot spell (`DD.MM.YYYY`, a
    /// two-branch currency), which a picker must show as such rather than offer parameters that
    /// would replace it.
    pub fn is_preset(&self) -> bool {
        let (kind, decimals, grouping, symbol) = self.preset_params();
        let same = |built: Format| built.in_locale(self.locale.clone()) == *self;
        // A dollar written after the amount is how older saves spelled it; still one of ours, and
        // re-applying it writes the leading form.
        let written_trailing = || {
            let mut old = preset(kind, decimals, grouping, &symbol);
            if let Some(Part::Number { .. }) = old.parts.get(1) {
                old.parts = trailing_currency(old.parts[1].clone(), &symbol);
            }
            old
        };
        same(preset(kind, decimals, grouping, &symbol))
            || (kind == Kind::Currency && symbol_leads(&symbol) && same(written_trailing()))
            || same(datetime_preset())
    }

    /// Whether the hours in this format are a 12-hour clock (§16.27.19: `number:am-pm`).
    fn twelve_hour(&self) -> bool {
        self.parts.contains(&Part::AmPm)
    }

    /// The branch that applies to `n` — the first `style:map` whose condition holds, one level
    /// deep, as LibreOffice writes them.
    fn branch(&self, n: f64) -> Option<&Map> {
        self.maps.iter().find(|map| map.holds(n))
    }

    /// The display text for `value`. Never fails: a format that does not fit its value falls back
    /// to the value's plain spelling, since a `date` style on a string is a real document.
    pub fn render(&self, value: &CellValue, null_date: i64) -> String {
        self.render_as(value, null_date, false, None)
    }

    /// [`Format::render`] in a document whose locale is `fallback`, which a format with no locale
    /// of its own uses ([`crate::Document::locale`]).
    pub fn render_in(
        &self,
        value: &CellValue,
        null_date: i64,
        fallback: Option<&Locale>,
    ) -> String {
        self.render_as(value, null_date, false, fallback)
    }

    /// [`Format::render`], plus whether this format was reached through a negative `style:map` —
    /// one whose condition says on its own that the value is not positive. Such a branch supplies
    /// no sign; it spells its own `-`, brackets, or nothing.
    ///
    /// Measured (`doc/ods-format.md` §5.2): LibreOffice converts Excel's
    /// `#,##0.00;[Red](#,##0.00);"—"` with the zero section as the style and maps to the other
    /// two, and renders −1234.5 as `(1,234.50)`, not `-(1,234.50)`. A branch reached by any other
    /// condition keeps its sign: `[<50][Red]0;[>500][Blue]0;[Green]0` renders −50 as `-50`.
    fn render_as(
        &self,
        value: &CellValue,
        null_date: i64,
        branch: bool,
        fallback: Option<&Locale>,
    ) -> String {
        // A branch with no locale of its own speaks this style's, and this style the
        // document's.
        let fallback = self.locale.as_ref().or(fallback);
        if let CellValue::Number(n) = value
            && let Some(map) = self.branch(*n)
            // Guard against a document mapping a style to itself: one level, then stop.
            && !std::ptr::eq(&map.format, self)
        {
            return map
                .format
                .render_as(value, null_date, map.negative(), fallback);
        }
        match (self.kind, value) {
            (_, CellValue::Empty) => String::new(),
            (Kind::Text, _) => self.render_parts(value, null_date, branch, fallback),
            (_, CellValue::Text(s)) => s.clone(),
            (Kind::Boolean, CellValue::Bool(_)) | (_, CellValue::Number(_)) => {
                self.render_parts(value, null_date, branch, fallback)
            }
            (_, CellValue::Bool(b)) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        }
    }

    fn render_parts(
        &self,
        value: &CellValue,
        null_date: i64,
        branch: bool,
        locale: Option<&Locale>,
    ) -> String {
        let n = match value {
            CellValue::Number(n) => *n,
            CellValue::Bool(b) => f64::from(u8::from(*b)),
            _ => 0.0,
        };
        // The value the numeric parts see. §16.27.11: a percentage style displays the value
        // multiplied by 100, and the `%` beside it is an ordinary literal.
        let scaled = match self.kind {
            Kind::Percentage => n * 100.0,
            _ => n,
        };
        // The minus is supplied only by a format with no branches, reached directly. A style with
        // a `style:map` spells its own sign (§5.1's red-negative currency has a literal `-`), and
        // one reached as a branch had its sign decided by its condition ([`Format::render_as`]).
        let mut out = String::new();
        let signed = !branch && self.maps.is_empty() && sign_carrying(&self.parts);
        if scaled < 0.0 && signed && !matches!(self.kind, Kind::Date | Kind::Time | Kind::Boolean) {
            out.push('-');
        }

        let (y, m, d) = date::ymd(n, null_date);
        let seconds = date::seconds_of_day(n);
        for part in &self.parts {
            match part {
                Part::Text(t) => out.push_str(t),
                Part::Currency(symbol) => out.push_str(symbol),
                Part::Number {
                    decimals,
                    min_decimals,
                    min_int,
                    grouping,
                    embedded,
                } => out.push_str(&embed(
                    &digits(
                        scaled.abs(),
                        *decimals,
                        *min_decimals,
                        *min_int,
                        // Text among or in front of the whole digits takes their grouping's place:
                        // LibreOffice draws `1234567G890` for a grouped format with text at
                        // position 3, and `1,234,567X` with it at 0, after them.
                        *grouping && embedded.iter().all(|e| e.position <= 0),
                        locale::separators(locale),
                    ),
                    embedded,
                    locale::separators(locale).0,
                )),
                Part::Year { long } => out.push_str(&match long {
                    true => format!("{y:04}"),
                    false => format!("{:02}", y.rem_euclid(100)),
                }),
                Part::Month { long, textual } => {
                    let index = (m.clamp(1, 12) - 1) as usize;
                    match (textual, long) {
                        (true, true) => out.push_str(MONTHS[index]),
                        (true, false) => out.push_str(&MONTHS[index][..3]),
                        (false, true) => out.push_str(&format!("{m:02}")),
                        (false, false) => out.push_str(&m.to_string()),
                    }
                }
                Part::Day { long } => out.push_str(&pad(d, *long)),
                Part::DayOfWeek { long } => {
                    let name = WEEKDAYS[date::weekday(n, null_date).clamp(1, 7) as usize - 1];
                    out.push_str(match long {
                        true => name,
                        false => &name[..3],
                    });
                }
                Part::Hours { long } => {
                    let h = seconds.div_euclid(3600).rem_euclid(24);
                    let h = match self.twelve_hour() {
                        true if h % 12 == 0 => 12,
                        true => h % 12,
                        false => h,
                    };
                    out.push_str(&pad(h, *long));
                }
                Part::Minutes { long } => {
                    out.push_str(&pad(seconds.div_euclid(60).rem_euclid(60), *long));
                }
                Part::Seconds { long, decimals } => {
                    // Whole seconds come from the same rounded total as the hours and minutes
                    // (`date::seconds_of_day`), or `09:15:00` stored as 46113.385416666664 prints
                    // its seconds as 60. A format asking for fractional seconds keeps the
                    // unrounded remainder, the digit it exists to show.
                    let s = match decimals {
                        0 => seconds.rem_euclid(60) as f64,
                        _ => {
                            let exact = (n - n.floor()) * 86_400.0;
                            exact - (exact / 60.0).floor() * 60.0
                        }
                    };
                    let width = usize::from(*long)
                        + 1
                        + usize::from(*decimals > 0)
                        + usize::from(*decimals);
                    out.push_str(&format!(
                        "{s:0width$.decimals$}",
                        decimals = usize::from(*decimals)
                    ));
                }
                Part::AmPm => out.push_str(match seconds.rem_euclid(86_400) < 43_200 {
                    true => "AM",
                    false => "PM",
                }),
                Part::Boolean => out.push_str(match n != 0.0 {
                    true => "TRUE",
                    false => "FALSE",
                }),
                // §16.27.28's text content is the cell's own text, and a number has one: 1234.5678
                // under a `number:text-style` shows 1234.5678, as the oracle renders a cell
                // formatted `@` (`doc/ods-format.md` §5.2).
                Part::Content => match value {
                    CellValue::Text(s) => out.push_str(s),
                    other => out.push_str(&general(other, None, null_date)),
                },
            }
        }
        out
    }
}

/// Whether a format leaves its sign to the renderer: not when it opens with a literal `-`, as a
/// negative branch named directly on a cell does.
fn sign_carrying(parts: &[Part]) -> bool {
    !matches!(parts.first(), Some(Part::Text(text)) if text.starts_with('-'))
}

fn pad(n: i64, long: bool) -> String {
    match long {
        true => format!("{n:02}"),
        false => n.to_string(),
    }
}

/// `number`, as [`digits`] spelled it, with each embedded text set among its digits at the
/// position ODF counts ([`Embedded`]).
fn embed(number: &str, embedded: &[Embedded], decimal_point: char) -> String {
    if embedded.is_empty() {
        return number.to_owned();
    }
    let (whole, fraction) = match number.split_once(decimal_point) {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (number, None),
    };
    let mut whole: Vec<String> = whole.chars().map(String::from).collect();
    let digits = whole.len();
    // Inserted from the right, so an earlier insertion never moves a later one's index; texts at
    // one position keep their document order.
    let mut ahead: Vec<(usize, usize, &str)> = embedded
        .iter()
        .enumerate()
        .filter(|(_, e)| e.position >= 0)
        .map(|(i, e)| {
            (
                digits.saturating_sub(e.position as usize),
                i,
                e.text.as_str(),
            )
        })
        .collect();
    ahead.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    for (at, _, text) in ahead {
        whole.insert(at, text.to_owned());
    }
    let mut out = whole.concat();
    if let Some(fraction) = fraction {
        let mut fraction: Vec<String> = fraction.chars().map(String::from).collect();
        let shown = fraction.len();
        let mut behind: Vec<(usize, usize, &str)> = embedded
            .iter()
            .enumerate()
            .filter(|(_, e)| e.position < 0)
            .map(|(i, e)| (((-e.position - 1) as usize).min(shown), i, e.text.as_str()))
            .collect();
        behind.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        for (at, _, text) in behind {
            fraction.insert(at, text.to_owned());
        }
        out.push(decimal_point);
        out.push_str(&fraction.concat());
    }
    out
}

/// The `number:number` piece: round to `decimals`, keep at least `min_decimals` of them,
/// pad the integer part to `min_int`, and group it if asked.
fn digits(
    n: f64,
    decimals: u8,
    min_decimals: u8,
    min_int: u8,
    grouping: bool,
    (decimal_point, thousands): (char, char),
) -> String {
    let decimals = decimals.max(min_decimals);
    // Rounded before formatting: `{:.2}` rounds half to even, a spreadsheet half away from zero.
    let scale = 10f64.powi(i32::from(decimals));
    let n = match scale.is_finite() && scale > 0.0 {
        true => (n * scale).round() / scale,
        false => n,
    };
    let text = format!("{n:.*}", usize::from(decimals));
    let (whole, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));

    // Trailing zeros come off only down to the minimum the style asks for.
    let keep = fraction
        .trim_end_matches('0')
        .len()
        .max(usize::from(min_decimals));
    let fraction = &fraction[..keep.min(fraction.len())];

    let mut whole = whole.to_owned();
    while whole.len() < usize::from(min_int) {
        whole.insert(0, '0');
    }
    if grouping {
        let mut i = whole.len();
        while i > 3 {
            i -= 3;
            whole.insert(i, thousands);
        }
    }
    match fraction.is_empty() {
        true => whole,
        false => format!("{whole}{decimal_point}{fraction}"),
    }
}

/// The currency a format gets when nobody named one: the euro, by the project's decision — one
/// constant for the CLI, every shell's currency button and the generator.
pub const DEFAULT_CURRENCY: &str = "€";

/// The currencies a shell offers one click away, most common first, as (symbol, name). Quick
/// choices, not a limit.
pub const CURRENCIES: [(&str, &str); 3] =
    [("€", "Euro"), ("$", "US dollar"), ("£", "Pound sterling")];

/// The currency a word names — `eur`/`euro`, `usd`/`dollar`, `gbp`/`pound`, or a symbol — ignoring
/// case; anything else is `None`.
pub fn currency_named(word: &str) -> Option<&'static str> {
    match word.to_lowercase().as_str() {
        "€" | "eur" | "euro" | "euros" => Some("€"),
        "$" | "usd" | "dollar" | "dollars" => Some("$"),
        "£" | "gbp" | "pound" | "pounds" => Some("£"),
        _ => None,
    }
}

/// Whether a currency symbol goes before the amount (`$1,234.50`, `£1,234.50`) rather than after
/// it with a no-break space (`1,234.50 €`), as every other symbol is spelled here.
pub fn symbol_leads(symbol: &str) -> bool {
    matches!(symbol, "$" | "£")
}

/// Build one of the formats a shell can ask for by name — the whole "set a format" vocabulary, so
/// a GUI's picker and `sheet format` produce the same `Format`. Dates and times are ISO, which
/// reads the same in every locale; a caller wanting `DD.MM.YYYY` builds the parts.
pub fn preset(kind: Kind, decimals: u8, grouping: bool, symbol: &str) -> Format {
    // Two decimals means two, always; `min_decimals` short of `decimals` is the "up to" form.
    let number = Part::Number {
        decimals,
        min_decimals: decimals,
        min_int: 1,
        grouping,
        embedded: Vec::new(),
    };
    let date = || {
        vec![
            Part::Year { long: true },
            Part::Text("-".into()),
            Part::Month {
                long: true,
                textual: false,
            },
            Part::Text("-".into()),
            Part::Day { long: true },
        ]
    };
    let time = || {
        vec![
            Part::Hours { long: true },
            Part::Text(":".into()),
            Part::Minutes { long: true },
            Part::Text(":".into()),
            Part::Seconds {
                long: true,
                decimals: 0,
            },
        ]
    };

    #[allow(clippy::items_after_statements)]
    let parts = match kind {
        Kind::Number => vec![number],
        Kind::Percentage => vec![number, Part::Text("%".into())],
        // `$1,234.50`: the symbol against the amount, as it is written.
        Kind::Currency if symbol_leads(symbol) => vec![Part::Currency(symbol.to_owned()), number],
        // A no-break space before the symbol, so the amount and its unit never wrap apart.
        Kind::Currency => trailing_currency(number, symbol),
        Kind::Date => date(),
        Kind::Time => time(),
        Kind::Boolean => vec![Part::Boolean],
        Kind::Text => vec![Part::Content],
    };
    Format {
        kind,
        parts,
        locale: None,
        maps: Vec::new(),
        color: None,
    }
}

/// `1,234.50 €` — the amount, a no-break space, the symbol.
fn trailing_currency(number: Part, symbol: &str) -> Vec<Part> {
    vec![
        number,
        Part::Text("\u{a0}".into()),
        Part::Currency(symbol.to_owned()),
    ]
}

/// The ISO date-and-time format: [`preset`]'s `Date` with a clock after it. A DateTime is a Date
/// whose value carries a fraction (§4.3.4), so the style differs and the kind does not.
pub fn datetime_preset() -> Format {
    let mut format = preset(Kind::Date, 0, false, "");
    format.push(Part::Text(" ".into()));
    format.parts.extend(preset(Kind::Time, 0, false, "").parts);
    format
}

/// The display text for a cell with no format of its own. A date prints as a date from its value
/// type alone (§4.3.3), in the ISO spelling the reader accepts back.
pub fn general(
    value: &CellValue,
    kind: Option<crate::model::NumberKind>,
    null_date: i64,
) -> String {
    general_in(value, kind, null_date, None)
}

/// [`general`] in a document whose locale is `locale`: numbers take its decimal separator
/// ([`spell_number`]); dates and times stay ISO.
pub fn general_in(
    value: &CellValue,
    kind: Option<crate::model::NumberKind>,
    null_date: i64,
    locale: Option<&Locale>,
) -> String {
    match (value, kind) {
        (CellValue::Number(n), Some(crate::model::NumberKind::Date)) => {
            let (y, m, d) = date::ymd(*n, null_date);
            let day = format!("{y:04}-{m:02}-{d:02}");
            // A Date with a fraction is a DateTime (§4.3.4); shown with a space rather than `T`.
            match n.fract() == 0.0 {
                true => day,
                false => format!("{day} {}", clock(*n)),
            }
        }
        // The clock, not the `xsd:duration` the file stores — `PT12H00M00S` is a
        // serialisation, not something to show a user.
        (CellValue::Number(n), Some(crate::model::NumberKind::Time)) => clock(*n),
        (CellValue::Number(n), None) => spell_number(*n, locale),
        (CellValue::Text(s), _) => s.clone(),
        (CellValue::Bool(b), _) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        (CellValue::Empty, _) => String::new(),
    }
}

/// A number with no format, spelled in `locale`: [`format_number`]'s shortest round-trip spelling
/// with the locale's decimal separator, and no grouping. Both shown and put in front of somebody
/// editing it (`App::input_text`), so what is shown can be typed back.
pub fn spell_number(n: f64, locale: Option<&Locale>) -> String {
    let plain = format_number(n);
    match locale::separators(locale).0 {
        '.' => plain,
        decimal => plain.replace('.', &decimal.to_string()),
    }
}

/// The time of day of a serial number, as a 24-hour clock.
fn clock(serial: f64) -> String {
    let seconds = date::seconds_of_day(serial);
    format!(
        "{:02}:{:02}:{:02}",
        seconds.div_euclid(3600).rem_euclid(24),
        seconds.div_euclid(60).rem_euclid(60),
        seconds.rem_euclid(60)
    )
}

/// The shorter spellings a shell tries, in order, when [`spell_number`]'s does not fit the column
/// (`doc/ods-format.md` §5.2, measured): one decimal fewer each time, rounded, then scientific
/// notation from five decimals down to none. When none fits, [`overflow`]'s hashes are drawn. Only
/// a number in the General format narrows; one whose format states its decimals shows `###`
/// instead ([`crate::Viewport::narrower`]).
pub fn narrower(n: f64, locale: Option<&Locale>) -> Vec<String> {
    let decimal = locale::separators(locale).0;
    let spell = |text: String| match decimal {
        '.' => text,
        decimal => text.replace('.', &decimal.to_string()),
    };
    let mut out: Vec<String> = Vec::new();
    if !n.is_finite() || n == 0.0 {
        return out;
    }
    let mut push = |text: String| {
        if !out.contains(&text) {
            out.push(text);
        }
    };
    let full = crate::formula::value::format_number(n);
    if !full.contains('E') {
        let decimals = full
            .split_once('.')
            .map_or(0, |(_, fraction)| fraction.len());
        for keep in (0..decimals).rev() {
            let rounded = format!("{:.keep$}", round_away(n, keep));
            // A small number rounded away to nothing is not a shorter spelling of it.
            if rounded
                .trim_start_matches('-')
                .bytes()
                .all(|b| b == b'0' || b == b'.')
            {
                break;
            }
            push(spell(trim_fraction(&rounded).to_owned()));
        }
    }
    for keep in (0..=5).rev() {
        let mut exponent = n.abs().log10().floor() as i32;
        let mut mantissa = round_away(n / 10f64.powi(exponent), keep);
        // `9.999996` at five decimals is `10.00000`, which is `1E+01` and not `10E+00`.
        if mantissa.abs() >= 10.0 {
            exponent += 1;
            mantissa = round_away(n / 10f64.powi(exponent), keep);
        }
        let mantissa = format!("{mantissa:.keep$}");
        let sign = if exponent < 0 { '-' } else { '+' };
        push(spell(format!(
            "{}E{sign}{:02}",
            trim_fraction(&mantissa),
            exponent.abs()
        )));
    }
    out
}

/// `n` rounded to `keep` decimals, half away from zero.
fn round_away(n: f64, keep: usize) -> f64 {
    let scale = 10f64.powi(keep as i32);
    match scale.is_finite() {
        true => (n * scale).round() / scale,
        false => n,
    }
}

/// A decimal's trailing zeros taken off, and its point with them when nothing is left after it.
fn trim_fraction(text: &str) -> &str {
    match text.contains('.') {
        true => text.trim_end_matches('0').trim_end_matches('.'),
        false => text,
    }
}

/// What a number that does not fit its column is drawn as, in every shell: as many `#` as the room
/// holds, never part of the number. A cut number is a different value (`2026-08-1`, `3,710.00`
/// without its currency); `###` is what LibreOffice and Excel draw. Text is not covered: a word
/// cut short is still the start of the right word.
///
/// `room` and `hash` (one `#`'s width in the cell's font) are in the shell's own unit. At least
/// one `#`, and bounded for a pathological width.
pub fn overflow(room: f64, hash: f64) -> String {
    let count = match hash > 0.0 && room.is_finite() {
        true => (room / hash).floor().clamp(1.0, 256.0) as usize,
        false => 1,
    };
    "#".repeat(count)
}

/// Whether a cell's value is one [`overflow`] covers: a number, which includes every date, time,
/// percentage and currency, since the model holds all of them as one.
pub fn is_number(value: &CellValue) -> bool {
    matches!(value, CellValue::Number(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH: i64 = date::DEFAULT_NULL_DATE;

    fn number(decimals: u8, min_decimals: u8, grouping: bool) -> Format {
        let mut f = Format::new(Kind::Number);
        f.push(Part::Number {
            decimals,
            min_decimals,
            min_int: 1,
            grouping,
            embedded: Vec::new(),
        });
        f
    }

    fn iso_date() -> Format {
        let mut f = Format::new(Kind::Date);
        f.push(Part::Year { long: true });
        f.push(Part::Text("-".into()));
        f.push(Part::Month {
            long: true,
            textual: false,
        });
        f.push(Part::Text("-".into()));
        f.push(Part::Day { long: true });
        f
    }

    fn render(f: &Format, n: f64) -> String {
        f.render(&CellValue::Number(n), EPOCH)
    }

    #[test]
    fn decimal_places_round_and_min_decimal_places_pad() {
        assert_eq!(render(&number(2, 2, false), 1.239), "1.24");
        assert_eq!(render(&number(2, 2, false), 3.0), "3.00");
        // Half away from zero, which is what a spreadsheet does — Rust's own formatter
        // would round this to 2.
        assert_eq!(render(&number(0, 0, false), 2.5), "3");
        // Two allowed, none required: the zeros come off but a real digit stays.
        assert_eq!(render(&number(2, 0, false), 3.0), "3");
        assert_eq!(render(&number(2, 0, false), 3.5), "3.5");
        assert_eq!(render(&number(0, 0, false), 3.7), "4");
    }

    #[test]
    fn grouping_starts_above_a_thousand_and_the_sign_stays_outside() {
        assert_eq!(render(&number(2, 2, true), 1234567.5), "1,234,567.50");
        assert_eq!(render(&number(0, 0, true), 1000.0), "1,000");
        assert_eq!(render(&number(0, 0, true), 999.0), "999");
        assert_eq!(render(&number(2, 2, true), -1234.5), "-1,234.50");
    }

    /// §5.2: the locale is on the style, and the same parts print differently under it.
    #[test]
    fn the_locale_swaps_the_decimal_and_grouping_characters() {
        let format = number(2, 2, true);
        assert_eq!(render(&format, 1234.5), "1,234.50");
        let german = format.clone().in_locale(Locale::parse("de-DE"));
        assert_eq!(render(&german, 1234.5), "1.234,50");
        assert_eq!(render(&german, -1234.5), "-1.234,50");
        // A locale the table does not know keeps the default separators rather than failing.
        assert_eq!(
            render(&format.in_locale(Locale::parse("zz")), 1234.5),
            "1,234.50"
        );
    }

    #[test]
    fn a_percentage_style_shows_the_value_times_a_hundred() {
        let mut f = Format::new(Kind::Percentage);
        f.push(Part::Number {
            decimals: 1,
            min_decimals: 1,
            min_int: 1,
            grouping: false,
            embedded: Vec::new(),
        });
        f.push(Part::Text("%".into()));
        assert_eq!(render(&f, 0.075), "7.5%");
    }

    #[test]
    fn a_date_style_walks_the_calendar_not_the_serial() {
        let serial = date::serial(1983, 1, 31, EPOCH);
        assert_eq!(render(&iso_date(), serial), "1983-01-31");
        // The epoch itself, and the 1900 non-leap-year that an off-by-one lands on.
        assert_eq!(render(&iso_date(), 0.0), "1899-12-30");
        assert_eq!(
            render(&iso_date(), date::serial(1900, 3, 1, EPOCH)),
            "1900-03-01"
        );
    }

    #[test]
    fn textual_months_and_weekdays_come_from_the_calendar() {
        let mut f = Format::new(Kind::Date);
        f.push(Part::DayOfWeek { long: false });
        f.push(Part::Text(", ".into()));
        f.push(Part::Month {
            long: true,
            textual: true,
        });
        f.push(Part::Text(" ".into()));
        f.push(Part::Day { long: false });
        // 2026-08-16 is a Sunday.
        assert_eq!(
            render(&f, date::serial(2026, 8, 16, EPOCH)),
            "Sun, August 16"
        );
    }

    #[test]
    fn am_pm_makes_the_hours_a_twelve_hour_clock() {
        let mut f = Format::new(Kind::Time);
        f.push(Part::Hours { long: false });
        f.push(Part::Text(":".into()));
        f.push(Part::Minutes { long: true });
        f.push(Part::Text(" ".into()));
        f.push(Part::AmPm);
        assert_eq!(render(&f, 0.5), "12:00 PM");
        assert_eq!(render(&f, 0.0), "12:00 AM");
        assert_eq!(render(&f, 13.0 / 24.0), "1:00 PM");
    }

    #[test]
    fn seconds_keep_the_fraction_the_style_asks_for() {
        let mut f = Format::new(Kind::Time);
        f.push(Part::Seconds {
            long: true,
            decimals: 1,
        });
        assert_eq!(render(&f, (35.0 * 60.0 + 31.25) / 86_400.0), "31.2");
    }

    /// §5.1's two-branch format: the base style is the negative one, with its own literal minus,
    /// and a `style:map` switches to the plain one otherwise. A renderer that also supplied the
    /// sign would print `--19.99`.
    #[test]
    fn a_style_map_picks_the_branch_and_the_branch_spells_its_own_sign() {
        let plain = number(2, 2, false);
        let mut negative = Format::new(Kind::Number);
        negative.push(Part::Text("-".into()));
        negative.push(Part::Number {
            decimals: 2,
            min_decimals: 2,
            min_int: 1,
            grouping: false,
            embedded: Vec::new(),
        });
        negative.maps.push(Map {
            op: Op::Ge,
            value: "0".into(),
            format: plain,
        });

        assert_eq!(render(&negative, -19.99), "-19.99");
        assert_eq!(render(&negative, 19.99), "19.99");
        assert_eq!(render(&negative, 0.0), "0.00");
        // Reached directly, without a map, the negative branch still must not double its
        // sign — a document may name it on a cell.
        let mut bare = Format::new(Kind::Number);
        bare.push(Part::Text("-".into()));
        bare.push(Part::Number {
            decimals: 0,
            min_decimals: 0,
            min_int: 1,
            grouping: false,
            embedded: Vec::new(),
        });
        assert_eq!(render(&bare, -5.0), "-5");
    }

    /// The three-branch shape: the base is the zero style and both other branches are reached
    /// through maps, so the negative branch's brackets are all the sign there is. LibreOffice's
    /// conversion of `xlsx/tests/data/corpus/numfmt/sections.xlsx`, rendered to CSV
    /// (`doc/ods-format.md` §5.2); a renderer that signed it would print `-(1,234.50)`.
    #[test]
    fn a_negative_branch_supplies_no_sign_of_its_own() {
        let mut bracketed = Format::new(Kind::Number);
        bracketed.push(Part::Text("(".into()));
        bracketed.push(Part::Number {
            decimals: 2,
            min_decimals: 2,
            min_int: 1,
            grouping: true,
            embedded: Vec::new(),
        });
        bracketed.push(Part::Text(")".into()));

        let mut zero = Format::new(Kind::Number);
        zero.push(Part::Text("—".into()));
        zero.maps.push(Map {
            op: Op::Gt,
            value: "0".into(),
            format: number(2, 2, true),
        });
        zero.maps.push(Map {
            op: Op::Lt,
            value: "0".into(),
            format: bracketed,
        });

        assert_eq!(render(&zero, 1234.5), "1,234.50");
        assert_eq!(render(&zero, -1234.5), "(1,234.50)");
        assert_eq!(render(&zero, 0.0), "—");
    }

    /// A branch whose condition is not about the sign keeps it:
    /// `[<50][Red]0;[>500][Blue]0;[Green]0` shows −50 as `-50`.
    #[test]
    fn a_branch_reached_by_any_other_condition_keeps_its_sign() {
        let mut format = number(0, 0, false);
        format.maps.push(Map {
            op: Op::Lt,
            value: "50".into(),
            format: number(0, 0, false),
        });
        assert_eq!(render(&format, -50.0), "-50");
        // And the style itself, reached because no condition held, does not sign: it is the
        // branch its own maps left over.
        assert_eq!(render(&format, 500.0), "500");
    }

    #[test]
    fn a_condition_parses_only_the_shapes_it_can_evaluate() {
        assert_eq!(
            parse_condition("value()>=0"),
            Some((Op::Ge, "0".to_owned()))
        );
        // Two-character operators must win over their first character.
        assert_eq!(
            parse_condition("value()<>0"),
            Some((Op::Ne, "0".to_owned()))
        );
        assert_eq!(
            parse_condition("value()<-1.5"),
            Some((Op::Lt, "-1.5".to_owned()))
        );
        assert_eq!(parse_condition("cellcontent()>0"), None);
        assert_eq!(parse_condition("value()"), None);
    }

    #[test]
    fn a_value_the_style_cannot_take_falls_back_to_its_own_spelling() {
        assert_eq!(
            iso_date().render(&CellValue::Text("#N/A".into()), EPOCH),
            "#N/A"
        );
        assert_eq!(iso_date().render(&CellValue::Empty, EPOCH), "");
    }

    #[test]
    fn without_a_format_a_date_still_prints_as_a_date() {
        use crate::model::NumberKind;
        let serial = date::serial(2026, 8, 16, EPOCH);
        assert_eq!(
            general(&CellValue::Number(serial), Some(NumberKind::Date), EPOCH),
            "2026-08-16"
        );
        assert_eq!(
            general(&CellValue::Number(0.75), Some(NumberKind::Time), EPOCH),
            "18:00:00"
        );
        assert_eq!(general(&CellValue::Number(1.5), None, EPOCH), "1.5");
    }

    #[test]
    fn a_currency_is_written_where_it_is_read() {
        let shown = |symbol: &str, n: f64| {
            preset(Kind::Currency, 2, true, symbol).render(&CellValue::Number(n), 0)
        };
        assert_eq!(shown("€", 1234.5), "1,234.50\u{a0}€");
        assert_eq!(shown("$", 1234.5), "$1,234.50");
        assert_eq!(
            shown("$", -1234.5),
            "-$1,234.50",
            "the sign before the symbol"
        );
        assert_eq!(shown("£", 0.5), "£0.50");
        assert_eq!(shown("CHF", 10.0), "10.00\u{a0}CHF");
        assert!(symbol_leads("$") && symbol_leads("£") && !symbol_leads("€"));
        assert_eq!(currency_named("USD"), Some("$"));
        assert_eq!(currency_named("euro"), Some("€"));
        assert_eq!(currency_named("£"), Some("£"));
        assert_eq!(currency_named("chf"), None);
        assert_eq!(CURRENCIES[0].0, DEFAULT_CURRENCY);
        // A dollar written after the amount is still one of ours.
        let mut old = preset(Kind::Currency, 2, true, "$");
        let number = old.parts[1].clone();
        old.parts = trailing_currency(number, "$");
        assert!(old.is_preset());
    }

    #[test]
    fn a_number_that_does_not_fit_is_hashes_as_wide_as_the_room() {
        assert_eq!(overflow(9.0, 1.0), "#########");
        // `number:embedded-text`, by LibreOffice's own renderings (`doc/ods-format.md` §5.2).
        let embedded = |min_int: u8, decimals: u8, grouping: bool, at: &[(i32, &str)]| {
            let mut f = Format::new(Kind::Number);
            f.push(Part::Number {
                decimals,
                min_decimals: decimals,
                min_int,
                grouping,
                embedded: at
                    .iter()
                    .map(|(position, text)| Embedded {
                        position: *position,
                        text: (*text).to_owned(),
                    })
                    .collect(),
            });
            f
        };
        let shows = |f: Format, n: f64| f.render(&CellValue::Number(n), EPOCH);
        let big = 1234567890.0;
        assert_eq!(
            shows(embedded(1, 0, false, &[(0, "A")]), big),
            "1234567890A"
        );
        assert_eq!(
            shows(embedded(1, 0, true, &[(0, "X")]), 1234567.0),
            "1,234,567X",
            "after the whole digits, the grouping stays"
        );
        assert_eq!(
            shows(embedded(1, 0, true, &[(9, "X")]), 1234567.0),
            "X1234567"
        );
        assert_eq!(
            shows(embedded(1, 0, false, &[(1, "B")]), big),
            "123456789B0"
        );
        assert_eq!(
            shows(embedded(1, 0, false, &[(3, "C"), (6, "D")]), big),
            "1234D567C890"
        );
        assert_eq!(
            shows(embedded(10, 0, false, &[(4, "-"), (7, ") ")]), big),
            "123) 456-7890"
        );
        assert_eq!(
            shows(embedded(1, 4, false, &[(-2, "E")]), 1.2345),
            "1.2E345"
        );
        assert_eq!(
            shows(embedded(1, 0, false, &[(12, "F")]), big),
            "F1234567890"
        );
        assert_eq!(shows(embedded(1, 0, true, &[(3, "G")]), big), "1234567G890");
        assert_eq!(
            narrower(154.719066107646, None)[..3],
            ["154.71906610765", "154.7190661076", "154.719066108"]
        );
        let all = narrower(154.719066107646, None);
        assert!(all.contains(&"154.719".to_owned()) && all.contains(&"155".to_owned()));
        assert_eq!(all.last().map(String::as_str), Some("2E+02"));
        assert_eq!(
            narrower(123456789012.0, None),
            [
                "1.23457E+11",
                "1.2346E+11",
                "1.235E+11",
                "1.23E+11",
                "1.2E+11",
                "1E+11"
            ]
        );
        let small = narrower(0.000123456, None);
        assert_eq!(
            small[..5],
            ["0.00012346", "0.0001235", "0.000123", "0.00012", "0.0001"]
        );
        assert!(
            !small.contains(&"0".to_owned()),
            "never rounded away to nothing"
        );
        assert_eq!(narrower(-4.56789123, None)[3], "-4.5679");
        let de = crate::locale::Locale::new("de", "DE");
        assert_eq!(narrower(1.25, Some(&de))[0], "1,3");
        assert!(narrower(0.0, None).is_empty() && narrower(5.0, None)[0] == "5E+00");
        assert_eq!(overflow(50.0, 7.0), "#######", "whole hashes only");
        assert_eq!(overflow(3.0, 7.0), "#", "never nothing");
        assert_eq!(overflow(1e9, 1.0).len(), 256, "never unbounded");
        assert_eq!(overflow(10.0, 0.0), "#", "a font that measured nothing");
        assert!(is_number(&CellValue::Number(45000.0)));
        assert!(!is_number(&CellValue::Text("3,710.00".into())));
    }
}
