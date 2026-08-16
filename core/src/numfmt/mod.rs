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
//! Three things are deliberately not modelled, each named where it would go:
//!
//! * `style:map` — the two-branch conditional (red negatives, §5.1). Ignored, so a negative
//!   value renders with a leading `-` in front of whatever the base style says.
//! * The locale. `number:language`/`number:country` are read as text and month and weekday
//!   names are English, because the document's locale is `HOST-LOCALE` (Part 4 §3.4 item 9)
//!   and nothing in the core has one yet.
//! * `number:truncate-on-overflow="false"` — a duration longer than a day. Hours wrap at 24,
//!   which is the attribute's default.

use crate::formula::date;
use crate::formula::value::format_number;
use crate::model::CellValue;

/// Which `number:*-style` element this is.
///
/// Not decoration: the family decides how the *value* reaches the parts. A percentage style
/// renders 0.5 as `50`, and the `%` is an ordinary text part beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

/// One `number:*-style`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Format {
    pub kind: Kind,
    pub parts: Vec<Part>,
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
        }
    }

    pub fn push(&mut self, part: Part) {
        self.parts.push(part);
    }

    /// Whether the hours in this format are a 12-hour clock (§16.27.19: `number:am-pm`).
    fn twelve_hour(&self) -> bool {
        self.parts.contains(&Part::AmPm)
    }

    /// The display text for `value`. Never fails: a format that does not fit the value it
    /// meets falls back to the value's plain spelling, because a cell whose style says
    /// `date` and whose value is a string is a real document, not an error.
    pub fn render(&self, value: &CellValue, null_date: i64) -> String {
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
        // ponytail: the sign is emitted here rather than by a negative sub-style, because
        // `style:map` is not modelled. A document whose base style already spells the minus
        // as a literal (§5.1's red-negative currency) therefore renders it twice. Resolving
        // the map at read time is the upgrade.
        let mut out = String::new();
        if scaled < 0.0 && !matches!(self.kind, Kind::Date | Kind::Time | Kind::Boolean) {
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

fn pad(n: i64, long: bool) -> String {
    match long {
        true => format!("{n:02}"),
        false => n.to_string(),
    }
}

/// The `number:number` piece: round to `decimals`, keep at least `min_decimals` of them,
/// pad the integer part to `min_int`, and group it if asked.
fn digits(n: f64, decimals: u8, min_decimals: u8, min_int: u8, grouping: bool) -> String {
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
        // ponytail: an ASCII comma, because grouping is `HOST-LOCALE` (Part 4 §3.4 item 9)
        // and the core has no locale. Take the separator from the document's language when
        // one exists.
        let mut i = whole.len();
        while i > 3 {
            i -= 3;
            whole.insert(i, ',');
        }
    }
    match fraction.is_empty() {
        true => whole,
        false => format!("{whole}.{fraction}"),
    }
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
