// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel number formats, translated onto `grind_sheet::numfmt::Format`.
//!
//! Excel spells a format as a **code string** — `#,##0.00;[Red]-#,##0.00;"—";@` — and no such
//! string may exist in the core: ODF has no such attribute, and a format there is an *ordered
//! sequence of parts* (`doc/xlsx-import.md` Part II §4). So the parser for one lives here, on
//! the import side of the seam, and its output is the model's own vocabulary.
//!
//! Two questions, answered by one reading of the code so they cannot disagree:
//!
//! 1. **Is this a date, a time, or neither?** — [`classify`], which X1 needed before it could
//!    put a serial in a cell, since Excel stores a date as a plain number and only the format
//!    says otherwise.
//! 2. **What does the cell display?** — [`of_code`] and [`of_builtin`], X3's work.
//!
//! **What cannot be spelled is named and counted** ([`Unspellable`]), never approximated into
//! something that would misstate the number. A format carrying a fraction, a scientific
//! exponent, an elapsed-time count or a ×1000 display factor is **dropped whole** and the cell
//! shows its plain value, because `3.75` shown as `4` is worse than `3.75` shown as `3.75`;
//! a format that merely loses a fill character or a colour is carried and the loss counted.
//! [`Unspellable::refuses`] is that line, and `Report::formats_lost` is where it surfaces.
//!
//! The shapes are ECMA-376's — built-in ids by §18.8.30's table, a custom code by §18.8.31's
//! grammar — and everything about *how the two models meet* is measured against the oracle and
//! recorded in `doc/xlsx-format.md` §3: which section becomes the style and which become its
//! `style:map` branches, and the positional rule for `m`.

use std::collections::BTreeSet;

use grind_sheet::model::NumberKind;
use grind_sheet::numfmt::{Format, Kind, Map, Op, Part};

/// A piece of an Excel format this model has no spelling for.
///
/// The admission rule is [`crate::report::Dropped`]'s: every variant names something a
/// `grind_sheet::numfmt::Format` **cannot express**, not something this filter has not got to.
/// Inventing a `Part` for a fraction or an exponent is a decision about the core's format
/// model, which is phase 5's and not an import filter's to take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unspellable {
    /// `# ?/?` — no `Part` holds a numerator and a denominator.
    Fraction,
    /// `0.00E+00` — no `Part` holds an exponent.
    Scientific,
    /// `[h]:mm` — hours that accumulate past 24. The core names the attribute that would
    /// carry it (`number:truncate-on-overflow="false"`) and deliberately does not model it.
    Elapsed,
    /// A trailing `,`, which divides the displayed number by a thousand per comma
    /// (`number:display-factor`).
    Scaling,
    /// A `General` section beside others: the model has no "as many digits as it takes" part,
    /// and guessing a decimal count would round a value the file shows in full.
    General,
    /// A built-in currency id (5–8, 41–44), whose symbol is the *user's* locale and appears
    /// nowhere in the file. Putting a `$` on it would put dollars on euros.
    LocaleCurrency,
    /// A built-in id §18.8.30 does not define — the East Asian ones, and anything else below
    /// 164 a producer invented. `doc/xlsx-format.md` §3.1: an unverified fact is not
    /// implemented.
    UnknownBuiltin,
    /// A section this build has no branch left for — a condition past the two-plus-default
    /// shape, or a fifth section.
    Section,
    /// `[Red]` — a colour belongs to a *branch* of the format, and a `Format` carries none.
    Colour,
    /// `*-` — the fill character that repeats to the width of the column.
    FillCharacter,
    /// `_)` — a blank as wide as the character after it, which is how Excel lines a bracketed
    /// negative up under a positive one.
    BlankWidth,
    /// `?` as a digit placeholder: a blank-padded digit, where `#` simply drops it.
    BlankDigit,
    /// `mmmmm`, the one-letter month. The oracle loses this one too, rendering `Mar`.
    NarrowMonth,
    /// `A/P`, the one-letter meridiem marker. `number:am-pm` has one spelling.
    NarrowAmPm,
    /// `[$€-407]`'s LCID: which decimal and grouping characters to punctuate with. The symbol
    /// itself comes through; the locale does not, since a `Format`'s locale is the document's
    /// own two characters (`core/src/locale.rs`) and an LCID table is not a fact this build
    /// has measured.
    Locale,
}

impl Unspellable {
    /// Whether losing this piece would **misstate the number**, and so costs the cell its
    /// whole format rather than one part of it.
    pub fn refuses(self) -> bool {
        matches!(
            self,
            Unspellable::Fraction
                | Unspellable::Scientific
                | Unspellable::Elapsed
                | Unspellable::Scaling
                | Unspellable::General
                | Unspellable::LocaleCurrency
                | Unspellable::UnknownBuiltin
        )
    }

    /// A plain-English name, for a report a person reads.
    pub fn label(self) -> &'static str {
        match self {
            Unspellable::Fraction => "fraction format",
            Unspellable::Scientific => "scientific format",
            Unspellable::Elapsed => "elapsed-time format",
            Unspellable::Scaling => "thousands display factor",
            Unspellable::General => "a General section beside others",
            Unspellable::LocaleCurrency => "built-in currency format (a locale's own symbol)",
            Unspellable::UnknownBuiltin => "undocumented built-in format id",
            Unspellable::Section => "number-format section",
            Unspellable::Colour => "number-format colour",
            Unspellable::FillCharacter => "fill character",
            Unspellable::BlankWidth => "blank-width padding",
            Unspellable::BlankDigit => "blank-padded digit",
            Unspellable::NarrowMonth => "one-letter month",
            Unspellable::NarrowAmPm => "one-letter AM/PM marker",
            Unspellable::Locale => "number-format locale",
        }
    }
}

/// What one Excel format becomes.
#[derive(Clone, Debug, Default)]
pub struct Translation {
    /// The format to put on a cell, or `None` when there is none to put — `General`, or a
    /// code carrying a piece [`Unspellable::refuses`].
    pub format: Option<Format>,
    /// Whether this format makes a number a date or a time, which is a different question:
    /// it is answered for a code this build refuses as well, so an elapsed-time cell is still
    /// a time even though nothing formats it.
    pub kind: Option<NumberKind>,
    /// Everything about the code that did not come through, by class — sorted and each
    /// class named once, however many pieces of the code fell in it.
    pub lost: Vec<Unspellable>,
}

impl Translation {
    fn nothing() -> Self {
        Translation::default()
    }

    fn refused(kind: Option<NumberKind>, lost: Vec<Unspellable>) -> Self {
        Translation {
            format: None,
            kind,
            lost,
        }
    }
}

/// What a built-in format id means, for the value reader. `SPEC`, ECMA-376 §18.8.30:
///
/// - 14–17 are dates (`mm-dd-yy`, `d-mmm-yy`, `d-mmm`, `mmm-yy`) and 22 is a date with a
///   time (`m/d/yy h:mm`) — a date either way, because the value is a point in time.
/// - 18–21 and 45–47 are clock and elapsed times (`h:mm AM/PM`, `h:mm:ss`, `mm:ss`,
///   `[h]:mm:ss`, `mmss.0`) — a duration.
///
/// Every other id below 164 is a number, text or currency format, and so is an id the table
/// does not list. The East Asian date ids (27–36, 50–58) are **not** here: §18.8.30 does not
/// define them, they are locale-dependent in Excel, and `doc/xlsx-format.md`'s rule is that
/// an unverified fact is not implemented. A file whose cell uses one carries its serial as a
/// number, which is visible and recoverable rather than a wrong date.
pub fn builtin(id: u32) -> Option<NumberKind> {
    match id {
        14..=17 | 22 => Some(NumberKind::Date),
        18..=21 | 45..=47 => Some(NumberKind::Time),
        _ => None,
    }
}

/// The code §18.8.30 assigns a built-in id — **by meaning**, which is not always its literal
/// spelling.
///
/// Two ids are deliberately not their own code. 14 (`mm-dd-yy`) and 22 (`m/d/yy h:mm`) are
/// rendered by Excel in the *reader's* locale rather than in the US order the table prints,
/// so translating them literally would put an American date in a German document; they map
/// onto the ISO spelling `numfmt::preset` uses, for `date.rs`'s reason — it is the one
/// spelling that means the same day everywhere. 47 (`mmss.0`) gains the separator the oracle
/// gives it, since minutes and seconds run together are a spelling nothing else in this model
/// produces.
///
/// The currency ids (5–8, 41–44) are **absent on purpose** and answer
/// [`Unspellable::LocaleCurrency`]: their symbol is the reader's locale and appears nowhere
/// in the file.
fn builtin_code(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "yyyy-mm-dd",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "yyyy-mm-dd hh:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// The format a built-in id carries. Id 0 is `General`, which is the *absence* of a format
/// and not a loss.
pub fn of_builtin(id: u32) -> Translation {
    match id {
        0 => Translation::nothing(),
        5..=8 | 41..=44 => Translation::refused(None, once(Unspellable::LocaleCurrency)),
        _ => match builtin_code(id) {
            Some(code) => of_code(code),
            None => Translation::refused(builtin(id), once(Unspellable::UnknownBuiltin)),
        },
    }
}

fn once(what: Unspellable) -> Vec<Unspellable> {
    vec![what]
}

/// What a custom format code means, for the value reader. §18.8.31.
///
/// Only the **first section** is read — it is the one a positive number is shown in, and a
/// code whose sections disagree about being a date is not something any producer writes.
///
/// - any `y` or `d`, or an `m` that is a **month**, → a **date**. `e` is deliberately not
///   counted: in a format code it is an exponent (`0.00E+00`) before it is ever an era year.
/// - otherwise any `h`, `s`, an `m` that is **minutes**, an elapsed bracket, or `AM/PM` → a
///   **time**.
/// - otherwise, a number.
///
/// **Which `m` is which** is positional, and it is what a first version of this function got
/// wrong: `m` and `mm` are minutes when the nearest token before them is an hour or the nearest
/// after is a second (`h:mm`, `mm:ss`), and a month everywhere else — so `mm` alone is a month,
/// and so is `mmm` and longer, always. `numfmt/datetime-codes.xlsx` has one cell per spelling
/// and is where the first version was caught. `doc/xlsx-format.md` §3.2 records the rule.
///
/// Case-insensitive, as Excel reads them. `General` has none of the letters, which is not a
/// coincidence anybody should rely on and is tested.
pub fn classify(code: &str) -> Option<NumberKind> {
    let first = sections(code).into_iter().next().unwrap_or_default();
    section(&first).kind()
}

/// One Excel format code, translated.
pub fn of_code(code: &str) -> Translation {
    if code.trim().is_empty() || code.trim().eq_ignore_ascii_case("general") {
        return Translation::nothing();
    }
    let mut parsed: Vec<Section> = sections(code).iter().map(|s| section(s)).collect();
    let kind = parsed.first().and_then(Section::kind);
    let mut lost: BTreeSet<Unspellable> = parsed.iter().flat_map(|s| s.lost.clone()).collect();
    if parsed.len() > 4 {
        parsed.truncate(4);
        lost.insert(Unspellable::Section);
    }
    if lost.iter().copied().any(Unspellable::refuses) {
        return Translation::refused(kind, lost.into_iter().collect());
    }
    let format = assemble(&parsed, &mut lost);
    Translation {
        format,
        kind,
        lost: lost.into_iter().collect(),
    }
}

/// The sections of a code, as ODF's one style plus its `style:map` branches.
///
/// **Measured, not specified** (`doc/xlsx-format.md` §3.4): this is the shape LibreOffice
/// writes when it converts the same workbook, read out of its conversion of
/// `numfmt/sections.xlsx` and `numfmt/conditions.xlsx` on 2026-09-20. The style itself is
/// always the *fallback* — the section that applies when no condition holds — and every other
/// section is a map, because that is exactly what §16.3's first-match-wins means:
///
/// | sections | the style | the maps |
/// |---|---|---|
/// | `pos` | `pos` | — |
/// | `pos;neg` | `neg` | `value()>=0` → `pos` |
/// | `pos;neg;zero` | `zero` | `value()>0` → `pos`, `value()<0` → `neg` |
/// | `pos;neg;zero;text` | `text` | the three above, plus `value()=0` → `zero` |
/// | any with `[>=100]`-style conditions | the last section carrying none | each conditioned section, in order |
fn assemble(parsed: &[Section], lost: &mut BTreeSet<Unspellable>) -> Option<Format> {
    if parsed.is_empty() {
        return None;
    }
    let conditioned = parsed.iter().any(|s| s.cond.is_some());
    let (base, maps): (usize, Vec<(Op, String, usize)>) = if conditioned {
        // The fallback is the last section with no condition of its own; a code where every
        // section carries one has no fallback to be, and the last one stands in for it.
        let base = parsed
            .iter()
            .rposition(|s| s.cond.is_none())
            .unwrap_or(parsed.len() - 1);
        let maps = parsed
            .iter()
            .enumerate()
            .filter(|(i, s)| *i != base && s.cond.is_some())
            .map(|(i, s)| {
                let (op, value) = s.cond.clone().expect("filtered");
                (op, value, i)
            })
            .collect();
        // A section that is neither the fallback nor a condition has nothing to be reached by.
        if parsed
            .iter()
            .enumerate()
            .any(|(i, s)| i != base && s.cond.is_none())
        {
            lost.insert(Unspellable::Section);
        }
        (base, maps)
    } else {
        match parsed.len() {
            1 => (0, Vec::new()),
            2 => (1, vec![(Op::Ge, "0".to_owned(), 0)]),
            3 => (
                2,
                vec![(Op::Gt, "0".to_owned(), 0), (Op::Lt, "0".to_owned(), 1)],
            ),
            _ => (
                3,
                vec![
                    (Op::Gt, "0".to_owned(), 0),
                    (Op::Lt, "0".to_owned(), 1),
                    (Op::Eq, "0".to_owned(), 2),
                ],
            ),
        }
    };
    let mut format = parsed[base].format();
    format.maps = maps
        .into_iter()
        .map(|(op, value, i)| Map {
            op,
            value,
            format: parsed[i].format(),
        })
        .collect();
    Some(format)
}

/// One section of a code, parsed.
#[derive(Debug, Default)]
struct Section {
    /// `[>=100]` — the condition this section applies under, where it states one.
    cond: Option<(Op, String)>,
    parts: Vec<Part>,
    date: bool,
    time: bool,
    percent: bool,
    currency: bool,
    text: bool,
    lost: BTreeSet<Unspellable>,
}

impl Section {
    /// Whether this section makes a number a date or a time.
    fn kind(&self) -> Option<NumberKind> {
        match (self.date, self.time) {
            (true, _) => Some(NumberKind::Date),
            (false, true) => Some(NumberKind::Time),
            (false, false) => None,
        }
    }

    /// The family this section belongs to. A date wins over a time because a value carrying
    /// both is a point in time (§4.3.4), and `%` wins over a currency symbol because it is
    /// the one that changes the *value* the parts see.
    fn family(&self) -> Kind {
        match self {
            _ if self.date => Kind::Date,
            _ if self.time => Kind::Time,
            _ if self.percent => Kind::Percentage,
            _ if self.currency => Kind::Currency,
            _ if self.text => Kind::Text,
            _ => Kind::Number,
        }
    }

    fn format(&self) -> Format {
        let mut format = Format::new(self.family());
        format.parts = self.parts.clone();
        format
    }
}

/// A date or time token of a format code, with everything between them — separators, digits,
/// literals — already gone, so "the token before" is a question about this list.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Year,
    /// A run of `m`, and its length: the one letter whose meaning depends on its neighbours.
    M(usize),
    Day,
    Hour,
    Second,
    /// `[h]`, `[mm]`, `[ss]` — an elapsed count, which is a duration however it is spelled.
    Elapsed,
    /// `AM/PM` or `A/P`, consumed whole so that their `M` and `P` are not read as tokens.
    AmPm,
}

/// Whether the `m` run at `index` is **minutes** rather than a month: the positional rule,
/// in one place, so `classify` and the part builder cannot read it two ways.
fn minutes(tokens: &[Token], index: usize, len: usize) -> bool {
    len <= 2
        && (matches!(
            tokens.get(index.wrapping_sub(1)),
            Some(Token::Hour | Token::Elapsed)
        ) || matches!(tokens.get(index + 1), Some(Token::Second)))
}

/// Split a code into its sections at top-level `;`, leaving quoted text, escapes and
/// bracketed modifiers alone.
fn sections(code: &str) -> Vec<String> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = vec![String::new()];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ';' => {
                out.push(String::new());
                i += 1;
            }
            '"' => {
                let end = find(&chars, i + 1, '"');
                out.last_mut().expect("never empty").extend(&chars[i..end]);
                i = end;
            }
            '[' => {
                let end = find(&chars, i + 1, ']');
                out.last_mut().expect("never empty").extend(&chars[i..end]);
                i = end;
            }
            '\\' | '_' | '*' => {
                out.last_mut()
                    .expect("never empty")
                    .extend(&chars[i..(i + 2).min(chars.len())]);
                i += 2;
            }
            _ => {
                out.last_mut().expect("never empty").push(c);
                i += 1;
            }
        }
    }
    out
}

/// One past the next `what` at or after `from`, or the end.
fn find(chars: &[char], from: usize, what: char) -> usize {
    chars[from.min(chars.len())..]
        .iter()
        .position(|&c| c == what)
        .map_or(chars.len(), |p| from + p + 1)
}

/// Parse one section into parts, the family it belongs to, and what it lost.
fn section(code: &str) -> Section {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Section::default();
    let mut literal = String::new();
    let mut tokens: Vec<Token> = Vec::new();
    // Where each `m` run landed: (index in `parts`, index in `tokens`, how many `m`).
    let mut months: Vec<(usize, usize, usize)> = Vec::new();
    let mut i = 0;

    macro_rules! flush {
        () => {
            if !literal.is_empty() {
                out.parts.push(Part::Text(std::mem::take(&mut literal)));
            }
        };
    }

    while i < chars.len() {
        let c = chars[i];
        let lower = c.to_ascii_lowercase();
        match lower {
            '"' => {
                let end = find(&chars, i + 1, '"');
                literal.extend(chars[i + 1..end.saturating_sub(1).max(i + 1)].iter());
                i = end;
            }
            '\\' => {
                if let Some(&next) = chars.get(i + 1) {
                    literal.push(next);
                }
                i += 2;
            }
            // `_x` is a blank as wide as `x`, `*x` fills the column with `x`. Both are about
            // where the ink sits rather than what it says, and neither has a `Part`.
            '_' | '*' => {
                out.lost.insert(match lower {
                    '_' => Unspellable::BlankWidth,
                    _ => Unspellable::FillCharacter,
                });
                i += 2;
            }
            '[' => {
                let end = find(&chars, i + 1, ']');
                let inner: String = chars[i + 1..end.saturating_sub(1).max(i + 1)]
                    .iter()
                    .collect();
                i = end;
                bracket(&inner, &mut out, &mut literal, &mut tokens);
            }
            '%' => {
                out.percent = true;
                literal.push('%');
                i += 1;
            }
            '@' => {
                flush!();
                out.text = true;
                out.parts.push(Part::Content);
                i += 1;
            }
            '#' | '0' | '?' => {
                flush!();
                i = number(&chars, i, &mut out);
            }
            'y' | 'd' | 'h' | 's' | 'm' => {
                let len = run(&chars, i, lower);
                flush!();
                letters(
                    lower,
                    len,
                    &chars,
                    &mut i,
                    &mut out,
                    &mut tokens,
                    &mut months,
                );
            }
            'a' => {
                let rest: String = chars[i..]
                    .iter()
                    .take(5)
                    .collect::<String>()
                    .to_ascii_lowercase();
                let eaten = match () {
                    () if rest.starts_with("am/pm") => 5,
                    () if rest.starts_with("a/p") => {
                        out.lost.insert(Unspellable::NarrowAmPm);
                        3
                    }
                    () => 0,
                };
                match eaten {
                    0 => literal.push(c),
                    _ => {
                        flush!();
                        out.time = true;
                        tokens.push(Token::AmPm);
                        out.parts.push(Part::AmPm);
                    }
                }
                i += eaten.max(1);
            }
            'g' if chars[i..]
                .iter()
                .take(7)
                .collect::<String>()
                .eq_ignore_ascii_case("general") =>
            {
                out.lost.insert(Unspellable::General);
                i += 7;
            }
            _ => {
                literal.push(c);
                i += 1;
            }
        }
    }
    flush!();

    // The `m` runs, decided now that their neighbours are known.
    for (part, token, len) in months {
        match minutes(&tokens, token, len) {
            true => {
                out.time = true;
                out.parts[part] = Part::Minutes { long: len >= 2 };
            }
            false => out.date = true,
        }
    }
    out
}

/// How long the run of `letter` starting at `i` is.
fn run(chars: &[char], i: usize, letter: char) -> usize {
    chars[i..]
        .iter()
        .take_while(|c| c.to_ascii_lowercase() == letter)
        .count()
}

/// One `[…]` modifier: a currency tag, an elapsed-time token, a condition, or a colour.
fn bracket(inner: &str, out: &mut Section, literal: &mut String, tokens: &mut Vec<Token>) {
    let lower = inner.to_ascii_lowercase();
    if let Some(tag) = inner.strip_prefix('$') {
        // `[$€-407]`: the symbol, then the LCID that says how to punctuate it.
        let (symbol, locale) = match tag.split_once('-') {
            Some((symbol, locale)) => (symbol, Some(locale)),
            None => (tag, None),
        };
        if locale.is_some_and(|l| !l.is_empty()) {
            out.lost.insert(Unspellable::Locale);
        }
        if !symbol.is_empty() {
            if !literal.is_empty() {
                out.parts.push(Part::Text(std::mem::take(literal)));
            }
            out.currency = true;
            out.parts.push(Part::Currency(symbol.to_owned()));
        }
        return;
    }
    if !lower.is_empty() && lower.chars().all(|c| matches!(c, 'h' | 'm' | 's')) {
        out.lost.insert(Unspellable::Elapsed);
        out.time = true;
        tokens.push(Token::Elapsed);
        return;
    }
    if let Some((op, value)) = condition(&lower) {
        out.cond = Some((op, value));
        return;
    }
    if is_colour(&lower) {
        out.lost.insert(Unspellable::Colour);
    }
}

/// `[>=100]` as the `style:condition` it becomes. The operators are the same six.
fn condition(inner: &str) -> Option<(Op, String)> {
    let (text, op) = Op::SPELLINGS
        .iter()
        .find(|(text, _)| inner.starts_with(text))?;
    let value = inner[text.len()..].trim();
    value.parse::<f64>().ok()?;
    Some((*op, value.to_owned()))
}

/// The colour names §18.8.31 allows, plus `color N`'s indexed form.
fn is_colour(inner: &str) -> bool {
    const NAMES: [&str; 8] = [
        "black", "blue", "cyan", "green", "magenta", "red", "white", "yellow",
    ];
    NAMES.contains(&inner) || inner.starts_with("color")
}

/// A run of digit placeholders — `#`, `0`, `?`, their grouping commas and their decimal point
/// — as one `Part::Number`, and the index just past it.
///
/// The three counts ODF wants are counts of placeholders: `min_int` is the `0`s before the
/// point, `decimals` every placeholder after it and `min_decimals` the `0`s among those — so
/// `#.###` shows 3.7 as `3.7` where `0.000` shows it as `3.700`, which is the distinction the
/// two spellings exist to make.
fn number(chars: &[char], from: usize, out: &mut Section) -> usize {
    let mut i = from;
    let (mut min_int, mut decimals, mut min_decimals) = (0u8, 0u8, 0u8);
    let (mut point, mut grouping, mut digits) = (false, false, 0u32);
    let mut trailing_commas = 0u32;
    while i < chars.len() {
        match chars[i] {
            c @ ('#' | '0' | '?') => {
                if c == '?' {
                    out.lost.insert(Unspellable::BlankDigit);
                }
                digits += 1;
                // A comma that turned out to have placeholders after it was grouping, not a
                // scaling factor.
                if trailing_commas > 0 {
                    grouping = true;
                    trailing_commas = 0;
                }
                match point {
                    true => {
                        decimals = decimals.saturating_add(1);
                        if c == '0' {
                            min_decimals = min_decimals.saturating_add(1);
                        }
                    }
                    false if c == '0' => min_int = min_int.saturating_add(1),
                    false => {}
                }
                i += 1;
            }
            ',' if digits > 0 => {
                trailing_commas += 1;
                i += 1;
            }
            '.' if !point => {
                point = true;
                i += 1;
            }
            _ => break,
        }
    }
    // A comma with no placeholder after it divides by a thousand (`number:display-factor`).
    if trailing_commas > 0 {
        out.lost.insert(Unspellable::Scaling);
    }
    // An exponent or a solidus turns the whole run into a part this model does not have.
    let next = chars.get(i).map(|c| c.to_ascii_lowercase());
    if next == Some('e') && matches!(chars.get(i + 1), Some('+' | '-')) {
        out.lost.insert(Unspellable::Scientific);
    }
    if next == Some('/') {
        out.lost.insert(Unspellable::Fraction);
    }
    out.parts.push(Part::Number {
        decimals,
        min_decimals,
        min_int,
        grouping,
    });
    i
}

/// One run of `y`, `m`, `d`, `h` or `s`, as the part and the token it is.
///
/// The lengths are §18.8.31's and the parts LibreOffice's, measured from its conversion of
/// `numfmt/datetime-codes.xlsx` (`doc/xlsx-format.md` §3.5): `ddd` is a weekday rather than a
/// long day, `mmm` a month's name rather than a wide number, and a `.0` after a seconds run
/// belongs to the seconds rather than being a literal point.
fn letters(
    letter: char,
    len: usize,
    chars: &[char],
    i: &mut usize,
    out: &mut Section,
    tokens: &mut Vec<Token>,
    months: &mut Vec<(usize, usize, usize)>,
) {
    *i += len;
    match letter {
        'y' => {
            out.date = true;
            tokens.push(Token::Year);
            out.parts.push(Part::Year { long: len >= 3 });
        }
        'd' => {
            out.date = true;
            tokens.push(Token::Day);
            out.parts.push(match len {
                1 => Part::Day { long: false },
                2 => Part::Day { long: true },
                3 => Part::DayOfWeek { long: false },
                _ => Part::DayOfWeek { long: true },
            });
        }
        'h' => {
            out.time = true;
            tokens.push(Token::Hour);
            out.parts.push(Part::Hours { long: len >= 2 });
        }
        's' => {
            out.time = true;
            tokens.push(Token::Second);
            // `ss.000` — the point and the zeros after it are this part's precision.
            let mut decimals = 0u8;
            if chars.get(*i) == Some(&'.') && chars.get(*i + 1) == Some(&'0') {
                decimals = u8::try_from(run(chars, *i + 1, '0')).unwrap_or(u8::MAX);
                *i += 1 + usize::from(decimals);
            }
            out.parts.push(Part::Seconds {
                long: len >= 2,
                decimals,
            });
        }
        // A month, provisionally: whether this run is minutes instead is a question about the
        // tokens on either side of it, and the one after it has not been read yet.
        _ => {
            if len >= 5 {
                out.lost.insert(Unspellable::NarrowMonth);
            }
            months.push((out.parts.len(), tokens.len(), len));
            tokens.push(Token::M(len));
            out.parts.push(Part::Month {
                // `mmmmm` is the month's *initial*, which `number:month` has no spelling for;
                // the oracle loses it the same way, rendering the short name.
                long: len == 2 || len == 4,
                textual: len >= 3,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use grind_sheet::formula::date::DEFAULT_NULL_DATE as EPOCH;
    use grind_sheet::model::CellValue;

    /// What a code displays for a number — the whole point of the translation, checked the
    /// way a person would check it.
    fn shows(code: &str, value: f64) -> String {
        of_code(code)
            .format
            .unwrap_or_else(|| panic!("{code} has a format"))
            .render(&CellValue::Number(value), EPOCH)
    }

    fn lost(code: &str) -> Vec<Unspellable> {
        of_code(code).lost
    }

    #[test]
    fn the_built_in_date_and_time_ids() {
        assert_eq!(builtin(0), None, "General");
        assert_eq!(builtin(14), Some(NumberKind::Date));
        assert_eq!(
            builtin(22),
            Some(NumberKind::Date),
            "a date with a time is a date"
        );
        assert_eq!(builtin(18), Some(NumberKind::Time));
        assert_eq!(builtin(46), Some(NumberKind::Time), "[h]:mm:ss");
        assert_eq!(builtin(49), None, "@");
        // East Asian ids are not in §18.8.30 and are not guessed at.
        assert_eq!(builtin(31), None);
    }

    #[test]
    fn a_code_is_a_date_when_it_has_a_year_or_a_day() {
        assert_eq!(classify("yyyy-mm-dd"), Some(NumberKind::Date));
        assert_eq!(classify("yyyy-mm-dd hh:mm:ss"), Some(NumberKind::Date));
        assert_eq!(classify("DD.MM.YYYY"), Some(NumberKind::Date));
        assert_eq!(classify("mmm-yy"), Some(NumberKind::Date));
    }

    #[test]
    fn a_code_is_a_time_when_it_only_has_a_clock() {
        assert_eq!(classify("hh:mm:ss"), Some(NumberKind::Time));
        assert_eq!(classify("[h]:mm:ss"), Some(NumberKind::Time));
        assert_eq!(classify("[mm]:ss"), Some(NumberKind::Time));
        assert_eq!(classify("h AM/PM"), Some(NumberKind::Time));
        assert_eq!(classify("h:mm A/P"), Some(NumberKind::Time));
        assert_eq!(
            classify("mm:ss.0"),
            Some(NumberKind::Time),
            "minutes, before a second"
        );
        assert_eq!(
            classify("h:mm"),
            Some(NumberKind::Time),
            "minutes, after an hour"
        );
        assert_eq!(
            classify("[h]:mm"),
            Some(NumberKind::Time),
            "after an elapsed hour"
        );
    }

    /// The positional rule, over `numfmt/datetime-codes.xlsx`'s own spellings: an `m` with no
    /// hour before it and no second after it is a month, at any length.
    #[test]
    fn an_m_on_its_own_is_a_month() {
        for code in [
            "m",
            "mm",
            "mmm",
            "mmmm",
            "mmmmm",
            "[$-409]mmmm\\ d\\,\\ yyyy",
        ] {
            assert_eq!(classify(code), Some(NumberKind::Date), "{code}");
        }
        // German letters for day and year are not tokens, so what is left is a lone `MM`.
        assert_eq!(classify("[$-407]TT.MM.JJJJ"), Some(NumberKind::Date));
        // A month, however long, is never minutes — even after an hour.
        assert_eq!(classify("h mmm"), Some(NumberKind::Date));
    }

    #[test]
    fn literals_and_modifiers_are_not_tokens() {
        assert_eq!(classify("General"), None);
        assert_eq!(classify("0.00"), None);
        assert_eq!(classify("#,##0.00;[Red]-#,##0.00"), None);
        assert_eq!(classify("0.00E+00"), None, "an exponent, not an era");
        assert_eq!(classify(r#""days: "0"#), None, "a quoted d is a letter");
        assert_eq!(classify(r"0\d"), None, "an escaped d is a letter");
        assert_eq!(classify("[$€-407] #,##0.00"), None, "a currency bracket");
        assert_eq!(
            classify("_(* #,##0_);_(* (#,##0)"),
            None,
            "widths and fills"
        );
        assert_eq!(classify("@"), None);
    }

    #[test]
    fn only_the_first_section_decides() {
        assert_eq!(classify(r#"0;"d""#), None);
        assert_eq!(classify("yyyy;0"), Some(NumberKind::Date));
    }

    /// `numfmt/custom-numeric.xlsx`'s own claims, cell by cell.
    #[test]
    fn the_digit_placeholders_are_what_they_say() {
        assert_eq!(shows("0", 3.7), "4", "rounded for display, not changed");
        assert_eq!(shows("0.000", 3.7), "3.700", "trailing zeros are forced");
        assert_eq!(
            shows("#.###", 3.7),
            "3.7",
            "`#` drops a digit that is not there"
        );
        assert_eq!(shows("#,##0", 1_234_567.0), "1,234,567");
        assert_eq!(shows("000000", 42.0), "000042");
        assert_eq!(shows("0%", 0.1234), "12%");
        assert_eq!(shows("0.00%", 0.1234), "12.34%");
        assert_eq!(shows(r"\$0.00", 5.0), "$5.00", "an escaped literal");
        assert_eq!(shows(r#""total: "0"#, 5.0), "total: 5", "a quoted one");
        // A number under a text format shows its own plain spelling, which is what the
        // oracle renders for a cell formatted `@` and what `Part::Content` now says.
        assert_eq!(shows(r#""<"@">""#, 0.0), "<0>", "literals around the text");
    }

    /// The text placeholder renders the *string* a cell holds, which is the one case where a
    /// number format is not about a number.
    #[test]
    fn the_text_section_shows_the_string() {
        let format = of_code(r#""<"@">""#).format.expect("a format");
        assert_eq!(format.kind, Kind::Text);
        assert_eq!(
            format.render(&CellValue::Text("x".into()), EPOCH),
            "<x>",
            "literals around the text placeholder"
        );
    }

    /// `numfmt/sections.xlsx`, which is the fixture the map shape comes from.
    #[test]
    fn sections_become_a_style_and_its_branches() {
        assert_eq!(shows("0.00", 1234.5), "1234.50");
        assert_eq!(
            shows("0.00", -1234.5),
            "-1234.50",
            "the renderer's own sign"
        );
        assert_eq!(shows("0.00", 0.0), "0.00");

        assert_eq!(shows("0.00;(0.00)", 1234.5), "1234.50");
        assert_eq!(
            shows("0.00;(0.00)", -1234.5),
            "(1234.50)",
            "the negative section spells its own sign, brackets and all"
        );

        let three = r#"#,##0.00;[Red](#,##0.00);"—""#;
        assert_eq!(shows(three, 1234.5), "1,234.50");
        assert_eq!(shows(three, -1234.5), "(1,234.50)");
        assert_eq!(shows(three, 0.0), "—");
        assert_eq!(lost(three), [Unspellable::Colour]);

        let four = r#"#,##0.00;[Red](#,##0.00);"—";[Blue]@"#;
        assert_eq!(shows(four, 0.0), "—");
        let format = of_code(four).format.expect("a format");
        assert_eq!(format.kind, Kind::Text, "the fourth section is the style");
        assert_eq!(
            format.render(&CellValue::Text("text".into()), EPOCH),
            "text"
        );

        // Empty sections hide what they cover, which is a format with no parts at all.
        assert_eq!(shows("0.0;;", 0.0), "");
        assert_eq!(shows("0.0;;", 1.25), "1.3");
    }

    /// `numfmt/conditions.xlsx`. The plan expected to lose these; `style:map` turns out to
    /// spell them exactly, because §16.3's condition is the same six operators.
    #[test]
    fn a_condition_becomes_the_map_it_already_is() {
        assert_eq!(shows("[>=100]#,##0;0.00", 50.0), "50.00");
        assert_eq!(shows("[>=100]#,##0;0.00", 500.0), "500");
        // A style carrying maps spells its own sign, so the fallback shows −50 unsigned —
        // which is what the oracle renders from its own conversion of this code, measured on
        // 2026-09-20 (`doc/xlsx-format.md` §3.4). Excel signs it; ODF's renderers do not.
        assert_eq!(shows("[>=100]#,##0;0.00", -50.0), "50.00");
        assert!(lost("[>=100]#,##0;0.00").is_empty(), "nothing is lost");

        let two = r#"[>=100]"big";[<=-100]"small";0.00"#;
        assert_eq!(shows(two, 500.0), "big");
        assert_eq!(shows(two, -500.0), "small");
        assert_eq!(shows(two, 50.0), "50.00");

        // A `General` section has no spelling, so the cell keeps its plain display.
        let general = r#"[=0]"zero";General"#;
        assert!(of_code(general).format.is_none());
        assert_eq!(lost(general), [Unspellable::General]);
    }

    /// `numfmt/datetime-codes.xlsx`, one claim per spelling, against the serial that fixture
    /// uses: 2024-03-17 18:05:59.
    #[test]
    fn the_date_and_time_pieces() {
        let when = 45_368.0 + (18.0 * 3600.0 + 5.0 * 60.0 + 59.0) / 86_400.0;
        for (code, want) in [
            ("yyyy", "2024"),
            ("yy", "24"),
            ("m", "3"),
            ("mm", "03"),
            ("mmm", "Mar"),
            ("mmmm", "March"),
            ("d", "17"),
            ("dd", "17"),
            ("ddd", "Sun"),
            ("dddd", "Sunday"),
            ("yyyy-mm-dd", "2024-03-17"),
            ("h", "18"),
            ("hh", "18"),
            ("h:mm", "18:05"),
            ("mm:ss", "05:59"),
            ("h:mm:ss", "18:05:59"),
            ("h:mm AM/PM", "6:05 PM"),
            ("ss.0", "59.0"),
            ("ss.000", "59.000"),
            ("yyyy-mm-dd hh:mm:ss", "2024-03-17 18:05:59"),
        ] {
            assert_eq!(shows(code, when), want, "{code}");
        }
    }

    /// A currency tag gives the symbol a `Part` of its own; a quoted one is a literal, which
    /// renders the same and says less.
    #[test]
    fn a_currency_tag_carries_its_symbol_and_loses_its_locale() {
        assert_eq!(shows(r##""$"#,##0.00"##, 1234.5), "$1,234.50");
        assert_eq!(shows("[$$-409]#,##0.00", 1234.5), "$1,234.50");
        assert_eq!(shows("[$€-407]#,##0.00", 1234.5), "€1,234.50");
        assert_eq!(shows(r"#,##0.00\ [$€-407]", 1234.5), "1,234.50 €");
        assert_eq!(shows(r"[$CHF-807]\ #,##0.00", 1234.5), "CHF 1,234.50");
        assert_eq!(
            of_code("[$€-407]#,##0.00").format.expect("a format").kind,
            Kind::Currency
        );
        assert_eq!(lost("[$€-407]#,##0.00"), [Unspellable::Locale]);
    }

    /// The classes that cost a cell its whole format, because carrying half of one would
    /// misstate the number it shows.
    #[test]
    fn what_cannot_be_spelled_is_refused_whole() {
        for (code, class) in [
            ("# ?/?", Unspellable::Fraction),
            ("# ??/??", Unspellable::Fraction),
            ("# ?/8", Unspellable::Fraction),
            ("0.00E+00", Unspellable::Scientific),
            ("##0.0E+0", Unspellable::Scientific),
            ("[h]:mm", Unspellable::Elapsed),
            ("[m]", Unspellable::Elapsed),
            ("[s]", Unspellable::Elapsed),
            ("#,##0,", Unspellable::Scaling),
            (r#"#,##0,," M""#, Unspellable::Scaling),
        ] {
            let got = of_code(code);
            assert!(got.format.is_none(), "{code} keeps a format");
            assert!(got.lost.contains(&class), "{code}: {:?}", got.lost);
            assert!(class.refuses());
        }
        // And the kind survives the refusal: an elapsed-time cell is still a time.
        assert_eq!(of_code("[h]:mm").kind, Some(NumberKind::Time));
    }

    /// The classes that cost a piece and not the format.
    #[test]
    fn a_cosmetic_loss_keeps_the_format() {
        assert_eq!(shows("*-0", 5.0), "5");
        assert_eq!(lost("*-0"), [Unspellable::FillCharacter]);
        assert_eq!(shows("0.00_);(0.00)", 5.0), "5.00");
        assert_eq!(shows("0.00_);(0.00)", -5.0), "(5.00)");
        assert_eq!(lost("0.00_);(0.00)"), [Unspellable::BlankWidth]);
        assert_eq!(shows("mmmmm", 45_368.0), "Mar", "the oracle loses this too");
        assert_eq!(lost("mmmmm"), [Unspellable::NarrowMonth]);
        assert_eq!(shows("h:mm A/P", 0.75), "6:00 PM");
        assert_eq!(lost("h:mm A/P"), [Unspellable::NarrowAmPm]);
    }

    #[test]
    fn a_built_in_id_is_read_by_meaning() {
        assert!(of_builtin(0).format.is_none(), "General is no format");
        assert!(of_builtin(0).lost.is_empty(), "and not a loss");
        assert_eq!(
            of_builtin(4)
                .format
                .expect("a format")
                .render(&CellValue::Number(1234.5678), EPOCH),
            "1,234.57"
        );
        // 14 is `mm-dd-yy` in the table and a locale date in Excel; ISO is the spelling that
        // means the same day in every country.
        assert_eq!(
            of_builtin(14)
                .format
                .expect("a format")
                .render(&CellValue::Number(45_368.0), EPOCH),
            "2024-03-17"
        );
        assert_eq!(
            of_builtin(15)
                .format
                .expect("a format")
                .render(&CellValue::Number(45_368.0), EPOCH),
            "17-Mar-24"
        );
        // The currency ids and the undocumented ones are named losses, not guesses.
        assert_eq!(of_builtin(5).lost, once(Unspellable::LocaleCurrency));
        assert_eq!(of_builtin(31).lost, once(Unspellable::UnknownBuiltin));
        assert!(of_builtin(12).lost.contains(&Unspellable::Fraction));
    }

    /// Every class this build knows is either a refusal or an approximation, and says which.
    #[test]
    fn every_class_has_a_label_and_a_severity() {
        for class in [
            Unspellable::Fraction,
            Unspellable::Scientific,
            Unspellable::Elapsed,
            Unspellable::Scaling,
            Unspellable::General,
            Unspellable::LocaleCurrency,
            Unspellable::UnknownBuiltin,
            Unspellable::Section,
            Unspellable::Colour,
            Unspellable::FillCharacter,
            Unspellable::BlankWidth,
            Unspellable::BlankDigit,
            Unspellable::NarrowMonth,
            Unspellable::NarrowAmPm,
            Unspellable::Locale,
        ] {
            assert!(!class.label().is_empty());
        }
    }

    #[test]
    fn a_code_that_says_nothing_is_no_format_and_no_loss() {
        for code in ["", "   ", "General", "GENERAL"] {
            let got = of_code(code);
            assert!(got.format.is_none(), "{code}");
            assert!(got.lost.is_empty(), "{code}");
        }
    }
}
