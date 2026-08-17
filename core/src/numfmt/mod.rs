// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Number formats — ODF 1.4 Part 3 §16.27, and doc/ods-format.md §5.2. **[ODS]**
//!
//! A number format is *display only*. The cell's value is the value (§5.2: "the number
//! style only controls display"), and nothing here ever changes it — which is why a
//! formatted cell still sums, compares and round-trips as the number it is.
//!
//! The model is the spec's own shape: a `number:*-style` is an **ordered sequence of
//! pieces**, literal text and value-bearing elements interleaved, and rendering is a walk
//! over that sequence. There is deliberately no format-code string (`"#,##0.00"`) anywhere
//! in the core — that spelling is Excel's, ODF has no such attribute, and inventing one
//! here would mean a translation layer in the one place this project exists not to have
//! one. A shell that wants to *show* a code can build it from the parts.
//!
//! No XML in this module: `odf::read` builds these from elements and `odf::write` puts
//! them back, so the format vocabulary has exactly one consumer on each side.
//!
//! Two things are deliberately not modelled, each named where it would go:
//!
//! * **Month and weekday names are English**, whatever the format's locale says. The
//!   separators a locale decides live in `locale.rs`; a name table is a different order of
//!   thing and wants CLDR, which is a dependency for two characters' worth of benefit.
//! * `number:truncate-on-overflow="false"` — a duration longer than a day. Hours wrap at 24,
//!   which is the attribute's default.

use serde::{Deserialize, Serialize};

use crate::formula::date;
use crate::locale::{self, Locale};
use crate::formula::value::format_number;
use crate::model::CellValue;

/// Which `number:*-style` element this is.
///
/// Not decoration: the family decides how the *value* reaches the parts. A percentage style
/// renders 0.5 as `50`, and the `%` is an ordinary text part beside it.
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
    },
    /// `number:currency-symbol` — the symbol is the element's content.
    Currency(String),
    /// `number:year`, long being four digits.
    Year { long: bool },
    /// `number:month`; `textual` is `number:textual="true"`, a name rather than a number.
    Month { long: bool, textual: bool },
    /// `number:day`, long being two digits.
    Day { long: bool },
    /// `number:day-of-week`, long being the full name.
    DayOfWeek { long: bool },
    Hours { long: bool },
    Minutes { long: bool },
    /// `number:seconds`, with `number:decimal-places` for sub-second precision.
    Seconds { long: bool, decimals: u8 },
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

/// One `style:map`: a condition and the format to use when it holds (§5.1).
///
/// This is how ODF spells a two-branch format — the red negative currency, the "show a dash
/// for zero" — and the reason there is no sign or zero handling anywhere else in a format.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Map {
    pub op: Op,
    /// The value compared against, as text, so `Format` stays `Eq` and `Hash` — a bit
    /// pattern is not a sensible map key and this is only ever parsed and printed back.
    pub value: String,
    pub format: Format,
}

impl Map {
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

/// `style:condition="value()>=0"` as an operator and the text of its operand.
///
/// Anything else is `None` and the map is dropped: §16.3 allows conditions this does not
/// model, and a condition we cannot evaluate must not silently become one we can.
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

    /// The [`preset`] arguments that come closest to this format — its kind, the digits and
    /// grouping of its first `number:number`, and its currency symbol.
    ///
    /// What a format *picker* shows as its current state, and in the core because both a GUI
    /// and `sheet format --show` ask it: two shells deriving "how many decimals is this"
    /// separately would eventually answer differently about the same cell. A format with no
    /// numeric part — a date, a boolean — has no digits, and says so as `(0, false)`.
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

    /// Whether [`preset`] — or [`datetime_preset`] — built from [`Format::preset_params`]
    /// *is* this format.
    ///
    /// False for a format a document brought that this vocabulary cannot spell: `DD.MM.YYYY`,
    /// a two-branch currency, a `number:min-decimal-places` short of its decimals. A picker
    /// showing such a format has to say so rather than offer parameters that would silently
    /// replace it with something else.
    pub fn is_preset(&self) -> bool {
        let (kind, decimals, grouping, symbol) = self.preset_params();
        let same = |built: Format| built.in_locale(self.locale.clone()) == *self;
        same(preset(kind, decimals, grouping, &symbol)) || same(datetime_preset())
    }

    /// Whether the hours in this format are a 12-hour clock (§16.27.19: `number:am-pm`).
    fn twelve_hour(&self) -> bool {
        self.parts.contains(&Part::AmPm)
    }

    /// The branch that applies to `n` — the first `style:map` whose condition holds, or
    /// this format itself.
    ///
    /// One level deep on purpose: a branch of a branch is not something LibreOffice writes,
    /// and following it would need a cycle guard for a case that does not exist.
    fn branch(&self, n: f64) -> &Format {
        self.maps
            .iter()
            .find(|map| map.holds(n))
            .map_or(self, |map| &map.format)
    }

    /// The display text for `value`. Never fails: a format that does not fit the value it
    /// meets falls back to the value's plain spelling, because a cell whose style says
    /// `date` and whose value is a string is a real document, not an error.
    pub fn render(&self, value: &CellValue, null_date: i64) -> String {
        if !self.maps.is_empty()
            && let CellValue::Number(n) = value
        {
            let branch = self.branch(*n);
            // Guard against a document mapping a style to itself: one level, then stop.
            if !std::ptr::eq(branch, self) {
                return branch.render(value, null_date);
            }
        }
        match (self.kind, value) {
            (_, CellValue::Empty) => String::new(),
            (Kind::Text, _) => self.render_parts(value, null_date),
            (_, CellValue::Text(s)) => s.clone(),
            (Kind::Boolean, CellValue::Bool(_)) | (_, CellValue::Number(_)) => {
                self.render_parts(value, null_date)
            }
            (_, CellValue::Bool(b)) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        }
    }

    fn render_parts(&self, value: &CellValue, null_date: i64) -> String {
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
        // The minus is supplied here only for a format that has no branches. A style with a
        // `style:map` spells its own sign — §5.1's red-negative currency carries a literal
        // `-` in the negative branch — and adding one on top renders `--19.99`.
        let mut out = String::new();
        let signed = self.maps.is_empty() && sign_carrying(&self.parts);
        if scaled < 0.0 && signed && !matches!(self.kind, Kind::Date | Kind::Time | Kind::Boolean)
        {
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
                } => out.push_str(&digits(
                    scaled.abs(),
                    *decimals,
                    *min_decimals,
                    *min_int,
                    *grouping,
                    locale::separators(self.locale.as_ref()),
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
                    let exact = (n - n.floor()) * 86_400.0;
                    let s = exact - (exact / 60.0).floor() * 60.0;
                    let width = usize::from(*long) + 1 + usize::from(*decimals > 0)
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
                Part::Content => {
                    if let CellValue::Text(s) = value {
                        out.push_str(s);
                    }
                }
            }
        }
        out
    }
}

/// Whether a format leaves its sign to the renderer.
///
/// A style that already opens with a literal `-` is spelling the sign itself, which is what
/// the negative branch of a two-branch format looks like when it is reached directly — a
/// document may name one on a cell without any map pointing at it.
fn sign_carrying(parts: &[Part]) -> bool {
    !matches!(parts.first(), Some(Part::Text(text)) if text.starts_with('-'))
}

fn pad(n: i64, long: bool) -> String {
    match long {
        true => format!("{n:02}"),
        false => n.to_string(),
    }
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
    // Rounded before formatting, not by the formatter: Rust's `{:.2}` rounds half to *even*
    // and a spreadsheet rounds half away from zero, so 2.5 at zero decimals is 3 here and
    // would be 2 there. The scale is bounded because a style asking for more decimals than
    // a double carries has nothing left to round anyway.
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

/// Build one of the formats a shell can ask for by name.
///
/// The whole of the "set a format" vocabulary, and it lives here rather than in a shell for
/// the usual reason: a GUI's format picker and `sheet format` must produce the same
/// `Format`, or a document formatted from one displays differently in the other.
///
/// Dates and times are the **ISO** spellings, for `date.rs`'s reason: the alternative is a
/// locale, `HOST-LOCALE` has no home in the core yet, and guessing one puts `01/02/03` in a
/// file that means different days in different countries. A caller that wants `DD.MM.YYYY`
/// builds the parts, which is three lines and exactly as expressive as ODF is.
pub fn preset(kind: Kind, decimals: u8, grouping: bool, symbol: &str) -> Format {
    // A user asking for two decimals means two, always — `min_decimals` short of `decimals`
    // is the "up to" form, which is a different request and not one a preset can guess.
    let number = Part::Number {
        decimals,
        min_decimals: decimals,
        min_int: 1,
        grouping,
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
        // A no-break space before the symbol, so the amount and its unit never wrap apart.
        Kind::Currency => vec![
            number,
            Part::Text("\u{a0}".into()),
            Part::Currency(symbol.to_owned()),
        ],
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
    }
}

/// The ISO date-and-time format — [`preset`]'s `Date` with a clock after it.
///
/// A separate call rather than an eighth `Kind`, because §4.3.4's DateTime is a Date whose
/// value carries a fraction and not a family of its own: the *style* differs, the value type
/// does not.
pub fn datetime_preset() -> Format {
    let mut format = preset(Kind::Date, 0, false, "");
    format.push(Part::Text(" ".into()));
    format.parts.extend(preset(Kind::Time, 0, false, "").parts);
    format
}

/// The display text for a cell that carries no format of its own.
///
/// This is where "a date prints as a date" comes from without any style in the document:
/// the value type is enough (§4.3.3), and the spelling is the ISO one the reader accepts,
/// so what a shell shows can always be typed back in.
pub fn general(value: &CellValue, kind: Option<crate::model::NumberKind>, null_date: i64) -> String {
    match (value, kind) {
        (CellValue::Number(n), Some(crate::model::NumberKind::Date)) => {
            let (y, m, d) = date::ymd(*n, null_date);
            let day = format!("{y:04}-{m:02}-{d:02}");
            // §4.3.4: a Date carrying a fraction is a DateTime, and showing only its date
            // half hides half the value. A space rather than `T` — this is a display, not
            // the `office:date-value` serialisation.
            match n.fract() == 0.0 {
                true => day,
                false => format!("{day} {}", clock(*n)),
            }
        }
        // The clock, not the `xsd:duration` the file stores — `PT12H00M00S` is a
        // serialisation, not something to show a user.
        (CellValue::Number(n), Some(crate::model::NumberKind::Time)) => clock(*n),
        (CellValue::Number(n), None) => format_number(*n),
        (CellValue::Text(s), _) => s.clone(),
        (CellValue::Bool(b), _) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        (CellValue::Empty, _) => String::new(),
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
        assert_eq!(render(&format.in_locale(Locale::parse("zz")), 1234.5), "1,234.50");
    }

    #[test]
    fn a_percentage_style_shows_the_value_times_a_hundred() {
        let mut f = Format::new(Kind::Percentage);
        f.push(Part::Number {
            decimals: 1,
            min_decimals: 1,
            min_int: 1,
            grouping: false,
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

    /// A cell whose value does not fit its style is a real document, not a bug: the value
    /// still has to reach the user.
    /// §5.1's two-branch format: the base style is the *negative* one, carrying its own
    /// literal minus, and a `style:map` switches to the plain one for everything else. The
    /// bug this pins is `--19.99` — a renderer that also supplies the sign.
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
        });
        assert_eq!(render(&bare, -5.0), "-5");
    }

    #[test]
    fn a_condition_parses_only_the_shapes_it_can_evaluate() {
        assert_eq!(
            parse_condition("value()>=0"),
            Some((Op::Ge, "0".to_owned()))
        );
        // Two-character operators must win over their first character.
        assert_eq!(parse_condition("value()<>0"), Some((Op::Ne, "0".to_owned())));
        assert_eq!(parse_condition("value()<-1.5"), Some((Op::Lt, "-1.5".to_owned())));
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
}
