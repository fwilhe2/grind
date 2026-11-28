// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tape: a scratch calculator whose lines become a spreadsheet's rows (`doc/calc.md`). A line is `[name =] expression`, `#n` reads line n, and numbers are read in the
//! tape's locale with the CSV import's rule.

use std::ops::Range;
use std::path::Path;

use crate::formula::assist::{Assist, Candidate};
use crate::formula::display;
use crate::formula::eval::to_cell;
use crate::formula::lex::Op;
use crate::formula::list::replace_literal;
use crate::formula::parse::{Expr, parse};
use crate::formula::serialize::Bare;
use crate::formula::value::Value;
use crate::formula::{Entry, ExprList};
use crate::kinds::{self, Currency, Kind};
use crate::locale::{self, Locale};
use crate::model::{CellValue, Document, Pos};
use crate::{App, Error, Result};

/// One line as it is shown.
#[derive(Clone, Debug, PartialEq)]
pub struct TapeLine {
    /// 1-based, as `#n` spells it.
    pub handle: usize,
    pub name: Option<String>,
    /// The line as it would be typed again, in the tape's locale.
    pub text: String,
    /// The value in the tape's locale; an error by its name.
    pub value: String,
    pub error: bool,
}

/// What a run of a typed line is, for colouring it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapeInk {
    /// The name a line gives itself, before its `=`.
    Definition,
    /// A name some line has.
    Name,
    /// `#n` for a line that exists.
    Handle,
    Function,
    Number,
    Text,
    /// A constant error, `#N/A`.
    Error,
    /// A name no line has, a function the catalog does not know, a cell, a `#n` past the end.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapeSpan {
    /// Bytes of the text that was scanned.
    pub range: Range<usize>,
    pub ink: TapeInk,
}

#[derive(Clone, Debug, Default)]
pub struct Tape {
    list: ExprList,
    /// Per line, the currency each number literal was typed with, in the order they stand.
    money: Vec<Vec<Option<Currency>>>,
    locale: Option<Locale>,
    undo: Vec<State>,
    redo: Vec<State>,
}

/// What an undo step restores.
type State = (ExprList, Vec<Vec<Option<Currency>>>);

/// A line as read: its name, its canonical formula, and its literals' currencies.
type Read = (Option<String>, String, Vec<Option<Currency>>);

impl Tape {
    #[must_use]
    pub fn new(locale: Option<Locale>) -> Self {
        Self {
            locale,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.list.entries().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn lines(&self) -> Vec<TapeLine> {
        let kinds = self.kinds();
        self.list
            .entries()
            .iter()
            .zip(self.list.values())
            .enumerate()
            .map(|(i, (entry, value))| {
                let (value, error) = match &kinds[i] {
                    Ok(kind) => (
                        self.value_text(value, self.format(i, *kind, value)),
                        matches!(value, Value::Error(_)),
                    ),
                    Err(_) => ("#VALUE!".to_owned(), true),
                };
                TapeLine {
                    handle: i + 1,
                    name: entry.name.clone(),
                    text: self.shown(i, entry),
                    value,
                    error,
                }
            })
            .collect()
    }

    /// Each line's kind, or why its currencies do not go together.
    fn kinds(&self) -> Vec<std::result::Result<Kind, String>> {
        let entries = self.list.entries();
        let memo: std::cell::RefCell<Vec<Option<std::result::Result<Kind, String>>>> =
            std::cell::RefCell::new(vec![None; entries.len()]);
        let visiting = std::cell::RefCell::new(vec![false; entries.len()]);
        fn kind_of(
            tape: &Tape,
            i: usize,
            memo: &std::cell::RefCell<Vec<Option<std::result::Result<Kind, String>>>>,
            visiting: &std::cell::RefCell<Vec<bool>>,
        ) -> std::result::Result<Kind, String> {
            // `#9` on a shorter tape is #REF! already.
            if i >= memo.borrow().len() {
                return Ok(Kind::Plain);
            }
            if let Some(known) = memo.borrow()[i].clone() {
                return known;
            }
            // A circle is #NUM! already; its kind is nothing in particular.
            if std::mem::replace(&mut visiting.borrow_mut()[i], true) {
                return Ok(Kind::Plain);
            }
            let entries = tape.list.entries();
            let answer = match parse(&entries[i].formula) {
                Err(_) => Ok(Kind::Plain),
                Ok(expr) => {
                    let money = tape.money.get(i).cloned().unwrap_or_default();
                    let mut literals = money.into_iter();
                    let mut literal = || literals.next().flatten();
                    let line = |e: &Expr| -> Option<Kind> {
                        let read = |j: usize| kind_of(tape, j, memo, visiting).ok();
                        match e {
                            Expr::Name(name) => tape.list.index_of(name).and_then(read),
                            Expr::Ref(r) if r.start.sheet.is_none() => {
                                let first = r.start.row?.index as usize;
                                let last = r
                                    .end
                                    .as_ref()
                                    .and_then(|e| e.row)
                                    .map_or(first, |r| r.index as usize);
                                let mut shared = None;
                                for j in first.min(last)..=first.max(last) {
                                    let k = read(j)?;
                                    match shared {
                                        None => shared = Some(k),
                                        Some(s) if s == k => {}
                                        Some(_) => return Some(Kind::Plain),
                                    }
                                }
                                shared
                            }
                            _ => None,
                        }
                    };
                    kinds::of(&expr, &mut literal, &line)
                }
            };
            memo.borrow_mut()[i] = Some(answer.clone());
            answer
        }
        (0..entries.len())
            .map(|i| kind_of(self, i, &memo, &visiting))
            .collect()
    }

    /// The format line `i` is shown and promoted in: a percentage typed as a number keeps every
    /// decimal it was typed with, one computed shows two at most.
    fn format(&self, i: usize, kind: Kind, value: &Value) -> Option<crate::numfmt::Format> {
        let Value::Number(n) = value else {
            return None;
        };
        let typed = parse(&self.list.entries()[i].formula).is_ok_and(|e| is_input(&e));
        kind.format(*n, if typed { 15 } else { 2 })
    }

    /// Every circular group, by handle.
    #[must_use]
    pub fn cycles(&self) -> Vec<Vec<usize>> {
        self.list
            .cycles()
            .iter()
            .map(|c| c.iter().map(|i| i + 1).collect())
            .collect()
    }

    /// The circles in words, `None` when there are none: `#3 reads itself`, `#1, #2 read each
    /// other in a circle`, joined when there are several.
    #[must_use]
    pub fn circles(&self) -> Option<String> {
        let said: Vec<String> = self
            .cycles()
            .iter()
            .map(|cycle| match cycle.as_slice() {
                [one] => format!("#{one} reads itself"),
                many => format!("{} read each other in a circle", handles(many)),
            })
            .collect();
        (!said.is_empty()).then(|| said.join("; "))
    }

    /// What to offer while a line is typed, `caret` a byte offset into `text`: the tape's own
    /// names first, each with its handle and value, then the functions.
    pub fn assist(&self, assist: &mut Assist, text: &str, caret: usize) {
        let names: Vec<String> = self
            .list
            .entries()
            .iter()
            .filter_map(|entry| entry.name.clone())
            .collect();
        assist.refresh_bare(text, caret, &names);
        let lines = self.lines();
        let (mut named, functions): (Vec<Candidate>, Vec<Candidate>) =
            std::mem::take(&mut assist.offers)
                .into_iter()
                .partition(|offer| !offer.insert.ends_with('('));
        for offer in &mut named {
            if let Some(line) = self.list.index_of(&offer.name).and_then(|i| lines.get(i)) {
                offer.detail = format!("#{} = {}", line.handle, line.value);
            }
        }
        named.extend(functions);
        assist.offers = named;
    }

    /// `text`, a line as typed, in runs to colour: a known name apart from one no line has.
    /// Byte ranges; anything not listed is an operator, a separator or space. A word that ends at
    /// `caret` is still being typed, and is not called unknown yet.
    #[must_use]
    pub fn spans(&self, text: &str, caret: Option<usize>) -> Vec<TapeSpan> {
        let mut out = Vec::new();
        let lead = text.len() - text.trim_start().len();
        let trimmed = &text[lead..];
        let body = if let Some(rest) = trimmed.strip_prefix('=') {
            text.len() - rest.len()
        } else if let Some((name, rest)) = definition(trimmed) {
            out.push(TapeSpan {
                range: lead..lead + name.len(),
                ink: TapeInk::Definition,
            });
            text.len() - rest.len()
        } else {
            lead
        };
        // `#n` is the tape's own, so it is taken out before the display scanner, which would
        // read `#3/12` as one error constant.
        let mut masked = text[body..].to_owned();
        let mut handles = Vec::new();
        let bytes = &text.as_bytes()[body..];
        let mut quoted = false;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => quoted = !quoted,
                b'#' if !quoted && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                    let end = i
                        + 1
                        + bytes[i + 1..]
                            .iter()
                            .take_while(|b| b.is_ascii_digit())
                            .count();
                    let known = text[body + i + 1..body + end]
                        .parse::<usize>()
                        .is_ok_and(|n| (1..=self.len()).contains(&n));
                    handles.push((i..end, known));
                    masked.replace_range(i..end, &" ".repeat(end - i));
                    i = end;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        let at = |r: Range<usize>| body + r.start..body + r.end;
        for (range, known) in handles {
            out.push(TapeSpan {
                range: at(range),
                ink: if known {
                    TapeInk::Handle
                } else {
                    TapeInk::Unknown
                },
            });
        }
        for span in display::spans(&masked) {
            let word = &masked[span.range.clone()];
            // `1200 EUR`, `EUR 1200`: a currency beside a number is part of the number.
            let beside_number = || {
                let before = masked[..span.range.start].trim_end_matches(' ');
                let after = masked[span.range.end..].trim_start_matches(' ');
                before.ends_with(|c: char| c.is_ascii_digit())
                    || after.starts_with(|c: char| c.is_ascii_digit())
            };
            if matches!(
                span.kind,
                display::TokenKind::Name | display::TokenKind::Func
            ) && Currency::named(word).is_some()
                && beside_number()
            {
                out.push(TapeSpan {
                    range: at(span.range),
                    ink: TapeInk::Number,
                });
                continue;
            }
            let ink = match span.kind {
                display::TokenKind::Func if is_function(word) => TapeInk::Function,
                display::TokenKind::Name if self.list.index_of(word).is_some() => TapeInk::Name,
                display::TokenKind::Func | display::TokenKind::Name | display::TokenKind::Ref => {
                    TapeInk::Unknown
                }
                display::TokenKind::Number | display::TokenKind::Bool => TapeInk::Number,
                display::TokenKind::Text => TapeInk::Text,
                display::TokenKind::Error => TapeInk::Error,
            };
            out.push(TapeSpan {
                range: at(span.range),
                ink,
            });
        }
        out.retain(|span| !(span.ink == TapeInk::Unknown && Some(span.range.end) == caret));
        out.sort_by_key(|span| span.range.start);
        out
    }

    /// Add a line at the end; returns its handle.
    pub fn push(&mut self, text: &str) -> Result<usize> {
        self.insert(self.len(), text)?;
        Ok(self.len())
    }

    /// Put a line before the one at `at` (0-based), renumbering those after it.
    pub fn insert(&mut self, at: usize, text: &str) -> Result<()> {
        let (name, formula, money) = self.read(text)?;
        self.edit_with(|list, lines| {
            list.insert(at, name.as_deref(), &formula)?;
            lines.insert(at, money);
            Ok(())
        })
    }

    /// Replace the line at `at`.
    pub fn edit(&mut self, at: usize, text: &str) -> Result<()> {
        let (name, formula, money) = self.read(text)?;
        self.edit_with(|list, lines| {
            list.set(at, name.as_deref(), &formula)?;
            if let Some(line) = lines.get_mut(at) {
                *line = money;
            }
            Ok(())
        })
    }

    /// Whether `text` reads as line `at` already is, however it is spaced.
    pub(crate) fn reads_as(&self, at: usize, text: &str) -> bool {
        let Some(entry) = self.list.entries().get(at) else {
            return false;
        };
        self.read(text).is_ok_and(|(name, formula, money)| {
            name == entry.name && formula == entry.formula && Some(&money) == self.money.get(at)
        })
    }

    pub fn delete(&mut self, at: usize) -> Result<()> {
        self.edit_with(|list, lines| {
            list.remove(at)?;
            if at < lines.len() {
                lines.remove(at);
            }
            Ok(())
        })
    }

    /// Move a number out of line `at` into a named input line just before it, and have the line
    /// read the name. `literal` is the number as typed, `%` allowed; `nth` picks one of several.
    pub fn extract(
        &mut self,
        at: usize,
        literal: &str,
        nth: Option<usize>,
        name: &str,
    ) -> Result<()> {
        let (literal, _) = strip_currency(literal.trim());
        let value = self.number(literal.trim_end_matches('%').trim())?;
        let Some(entry) = self.list.entries().get(at) else {
            return Err(Error::Formula(format!("there is no line #{}", at + 1)));
        };
        let expr = parse(&entry.formula).map_err(|e| Error::Formula(e.to_string()))?;
        let with = Expr::Name(name.to_owned());
        let (_, found) = replace_literal(&expr, value, nth, &with)
            .map_err(|why| Error::Formula(format!("#{}: {why}", at + 1)))?;
        let input = if found.percent {
            Expr::Postfix(Op::Percent, Box::new(Expr::Number(value)))
        } else {
            Expr::Number(value)
        };
        // Which of the line's literals moves, so its currency moves with it.
        let mut numbers = Vec::new();
        literals(&expr, &mut numbers);
        let moved = numbers
            .iter()
            .enumerate()
            .filter(|(_, n)| **n == value)
            .nth(nth.unwrap_or(1).saturating_sub(1))
            .map(|(i, _)| i);
        self.edit_with(|list, lines| {
            list.insert(at, Some(name), &input.to_string())?;
            let line = &list.entries()[at + 1];
            let shifted = parse(&line.formula).map_err(|e| Error::Formula(e.to_string()))?;
            let (rewritten, _) =
                replace_literal(&shifted, value, nth, &with).map_err(Error::Formula)?;
            let name = line.name.clone();
            list.set(at + 1, name.as_deref(), &rewritten.to_string())?;
            let mut rest = lines.get(at).cloned().unwrap_or_default();
            let currency = moved
                .and_then(|i| (i < rest.len()).then(|| rest.remove(i)))
                .flatten();
            if let Some(line) = lines.get_mut(at) {
                *line = rest;
            }
            lines.insert(at.min(lines.len()), vec![currency]);
            Ok(())
        })
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(before) = self.undo.pop() else {
            return false;
        };
        let now = self.restore(before);
        self.redo.push(now);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(after) = self.redo.pop() else {
            return false;
        };
        let now = self.restore(after);
        self.undo.push(now);
        true
    }

    fn restore(&mut self, (list, money): State) -> State {
        (
            std::mem::replace(&mut self.list, list),
            std::mem::replace(&mut self.money, money),
        )
    }

    /// The tape as a spreadsheet: line n is row n, its label (name, or `#n`) in A and its number
    /// or formula in B, a name a named range over its B cell, and `#k` the name of line k or a
    /// reference to it. Refused while there is a cycle.
    pub fn promote(&self) -> Result<Document> {
        if let Some(circles) = self.circles() {
            return Err(Error::Formula(format!(
                "{circles}; break the circle before saving"
            )));
        }
        let mut doc = self.list.document();
        doc.locale.clone_from(&self.locale);
        let entries = self.list.entries();
        let kinds = self.kinds();
        for (i, (entry, value)) in entries.iter().zip(self.list.values()).enumerate() {
            let row = u32::try_from(i).unwrap_or(u32::MAX);
            let sheet = &mut doc.sheets[0];
            if let Some(format) = kinds[i]
                .as_ref()
                .ok()
                .and_then(|kind| self.format(i, *kind, value))
            {
                sheet.set_format(Pos::new(row, 1), format);
            }
            let label = entry.name.clone().unwrap_or_else(|| format!("#{}", i + 1));
            sheet.set(Pos::new(row, 0), CellValue::Text(label));
            let b = Pos::new(row, 1);
            let Ok(expr) = parse(&entry.formula) else {
                continue;
            };
            if is_input(&expr) {
                sheet.clear_formula(b);
                sheet.set(b, to_cell(value.clone()));
            } else {
                let named = named_references(&expr, entries);
                sheet.set_formula(b, format!("of:={named}"));
            }
        }
        doc.edits.names = true;
        Ok(doc)
    }

    /// A promoted tape read back: the spreadsheet [`Tape::promote`] would write, or why `doc` is not
    /// one. Nothing in a file says it was a tape, so this is the whole test: the lines it reads
    /// promote to the same cells, names and number formats. Anything else a sheet holds — widths,
    /// fonts, fills — a tape has nowhere to keep.
    pub(crate) fn from_sheet(doc: &Document) -> Result<Tape> {
        let not = |why: String| Error::Formula(format!("not a tape: {why}"));
        let [sheet] = doc.sheets.as_slice() else {
            return Err(not(format!(
                "it has {} sheets, and a tape is one",
                doc.sheets.len()
            )));
        };
        let held = |pos: Pos| sheet.formula(pos).is_some() || sheet.get(pos) != CellValue::Empty;
        let mut rows = 0;
        for row in 0..sheet.used_rows() {
            for col in 0..sheet.used_cols() {
                let pos = Pos::new(row, col);
                if !held(pos) {
                    continue;
                }
                if col > 1 {
                    return Err(not(format!(
                        "{} holds something, and a tape is columns A and B",
                        crate::a1::format(None, pos)
                    )));
                }
                rows = rows.max(row + 1);
            }
        }
        let mut tape = Tape::new(doc.locale.clone());
        for row in 0..rows {
            let line = row as usize + 1;
            let (a, b) = (Pos::new(row, 0), Pos::new(row, 1));
            let CellValue::Text(label) = sheet.get(a) else {
                return Err(not(format!(
                    "A{line} is not a name or #{line}, the label a tape writes"
                )));
            };
            let name = if label == format!("#{line}") {
                None
            } else {
                crate::validate_name(&label)
                    .map_err(|_| not(format!("A{line}, {label}, is not a name")))?;
                Some(label)
            };
            let (formula, money) = match (sheet.formula(b), sheet.get(b)) {
                (Some(formula), _) => {
                    let formula = formula.strip_prefix("of:=").unwrap_or(formula);
                    let expr = parse(formula)
                        .map_err(|_| not(format!("B{line}'s formula does not read")))?;
                    if let Some(cell) = outside_the_tape(&expr) {
                        return Err(not(format!(
                            "B{line} reads {cell}, and a tape reads only its own lines"
                        )));
                    }
                    let mut numbers = Vec::new();
                    literals(&expr, &mut numbers);
                    (formula.to_owned(), vec![None; numbers.len()])
                }
                (None, CellValue::Number(n)) => match sheet.format(b) {
                    None => (shortest(n), vec![None]),
                    Some(format) => {
                        let kind = std::iter::once(Kind::Percent)
                            .chain(Currency::every().map(Kind::Money))
                            .find(|kind| kind.format(n, 15).as_ref() == Some(format))
                            .ok_or_else(|| {
                                not(format!("B{line}'s number format is not one a tape writes"))
                            })?;
                        if let Kind::Money(currency) = kind {
                            (shortest(n), vec![Some(currency)])
                        } else {
                            let hundred: f64 = format!("{:.14e}", n * 100.0).parse().unwrap_or(0.0);
                            (format!("{}%", shortest(hundred)), vec![None])
                        }
                    }
                },
                _ => {
                    return Err(not(format!("B{line} is neither a number nor a formula")));
                }
            };
            let at = tape.len();
            tape.list
                .insert(at, name.as_deref(), &formula)
                .map_err(|e| not(format!("line {line}: {e}")))?;
            tape.money.push(money);
        }
        tape.read_kinds(sheet);
        tape.same_as(doc, sheet, rows).map_err(not)?;
        Ok(tape)
    }

    /// Where a formula line's format is a currency its kind does not come to by itself, some of
    /// its numbers were typed in that currency. Promotion keeps only the line's format, so the
    /// first reading that gives it back is taken: one number in it, then each in turn, then all.
    fn read_kinds(&mut self, sheet: &crate::model::Sheet) {
        for i in 0..self.len() {
            let b = Pos::new(u32::try_from(i).unwrap_or(u32::MAX), 1);
            let count = self.money[i].len();
            let (Some(_), Some(wanted)) = (sheet.formula(b), sheet.format(b)) else {
                continue;
            };
            if count == 0 || self.shown_format(i).as_ref() == Some(wanted) {
                continue;
            }
            let Some(currency) = Currency::every().find(|c| &c.format() == wanted) else {
                continue;
            };
            let readings = (0..count)
                .map(|k| {
                    let mut typed = vec![None; count];
                    typed[k] = Some(currency);
                    typed
                })
                .chain(std::iter::once(vec![Some(currency); count]));
            for typed in readings {
                self.money[i] = typed;
                if self.shown_format(i).as_ref() == Some(wanted) {
                    break;
                }
            }
        }
    }

    /// The format line `i` is promoted in, as the tape now reads it.
    fn shown_format(&self, i: usize) -> Option<crate::numfmt::Format> {
        let kind = *self.kinds()[i].as_ref().ok()?;
        self.format(i, kind, &self.list.values()[i])
    }

    /// Why promoting this tape would not give `doc` back, or nothing when it would.
    fn same_as(
        &self,
        doc: &Document,
        sheet: &crate::model::Sheet,
        rows: u32,
    ) -> std::result::Result<(), String> {
        let promoted = self.promote().map_err(|e| e.to_string())?;
        let again = &promoted.sheets[0];
        for row in 0..rows {
            let line = row + 1;
            for (col, letter) in [(0, 'A'), (1, 'B')] {
                let (pos, cell) = (Pos::new(row, col), format!("{letter}{line}"));
                if sheet.formula(pos) != again.formula(pos) {
                    return Err(format!(
                        "{cell}'s formula is not the one its line promotes to"
                    ));
                }
                if sheet.formula(pos).is_none() && sheet.get(pos) != again.get(pos) {
                    return Err(format!("{cell} is not what its line promotes to"));
                }
                if sheet.format(pos) != again.format(pos) {
                    return Err(format!(
                        "{cell}'s number format is not the one its line has"
                    ));
                }
            }
        }
        let named = |names: &std::collections::BTreeMap<String, String>, sheet_name: &str| {
            names
                .iter()
                .map(|(name, target)| (name.clone(), named_row(target, sheet_name)))
                .collect::<Vec<_>>()
        };
        let (have, want) = (
            named(&doc.names, &sheet.name),
            named(&promoted.names, &again.name),
        );
        if have != want {
            let stray = have
                .iter()
                .find(|entry| !want.contains(entry))
                .map_or_else(String::new, |(name, _)| format!(": {name}"));
            return Err(format!(
                "its named ranges are not the names its lines have{stray}"
            ));
        }
        Ok(())
    }

    /// A saved tape, from a file in any form a spreadsheet is read from; refused, with the reason,
    /// when it is not one: the lines read from it must promote to the same cells.
    pub fn open_file(path: &Path) -> Result<Tape> {
        Tape::from_sheet(&crate::read_file(path)?)
    }

    /// [`Tape::open_file`] from bytes.
    pub fn open_bytes(bytes: &[u8]) -> Result<Tape> {
        Tape::from_sheet(&crate::read_bytes("tape", bytes)?)
    }

    /// [`Tape::promote`], written: `.ods` a package, `.grind` the projection, anything else flat.
    pub fn save(&self, path: &Path) -> Result<()> {
        let app = App::new();
        app.open_document(self.promote()?)?;
        app.save_file(path)
    }

    /// Apply an edit to a copy, refused when it leaves a line adding one currency to another.
    fn edit_with(
        &mut self,
        f: impl FnOnce(&mut ExprList, &mut Vec<Vec<Option<Currency>>>) -> Result<()>,
    ) -> Result<()> {
        let (mut list, mut money) = (self.list.clone(), self.money.clone());
        money.resize(list.entries().len(), Vec::new());
        f(&mut list, &mut money)?;
        let before = self.restore((list, money));
        if let Some((i, why)) = self
            .kinds()
            .into_iter()
            .enumerate()
            .find_map(|(i, k)| k.err().map(|why| (i, why)))
        {
            self.restore(before);
            return Err(Error::Formula(format!("#{}: {why}", i + 1)));
        }
        self.undo.push(before);
        self.redo.clear();
        Ok(())
    }

    fn separators(&self) -> (char, char) {
        locale::separators(self.locale.as_ref())
    }

    fn locale_name(&self) -> String {
        self.locale
            .as_ref()
            .map_or_else(|| "this tape".to_owned(), Locale::tag)
    }

    fn number(&self, text: &str) -> Result<f64> {
        let (decimal, group) = self.separators();
        crate::csv::number(text, decimal, group)
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| {
                Error::Formula(format!(
                    "{text} is not a number in {}: write {}",
                    self.locale_name(),
                    locale::sample(self.locale.as_ref())
                ))
            })
    }

    /// A typed line → its name, its canonical formula and its literals' currencies.
    fn read(&self, text: &str) -> Result<Read> {
        let text = text.trim();
        let (name, expression) = match text.strip_prefix('=') {
            Some(rest) => (None, rest),
            None => match definition(text) {
                Some((name, rest)) => {
                    crate::validate_name(name)?;
                    (Some(name.to_owned()), rest)
                }
                None => (None, text),
            },
        };
        if expression.trim().is_empty() {
            return Err(Error::Formula("the line has no expression".into()));
        }
        let (typed, money) = self.canonical_numbers(expression)?;
        for span in display::spans(&typed) {
            if span.kind == display::TokenKind::Ref && !typed[span.range.clone()].starts_with('[') {
                return Err(Error::Formula(format!(
                    "{} is a cell, and a tape has no cells: name a line or write #n",
                    &typed[span.range]
                )));
            }
        }
        let canonical =
            display::from_display(&format!("={typed}")).map_err(|e| Error::Formula(e.message))?;
        Ok((name, canonical.trim_start_matches('=').to_owned(), money))
    }

    /// `expression` with every number respelled in OpenFormula's, `#n` as a reference to line n,
    /// and bracketed text refused; and the currency each number was typed with — an ISO code or
    /// one of `€ $ £ ¥` before or after it — in the order the numbers stand.
    fn canonical_numbers(&self, expression: &str) -> Result<(String, Vec<Option<Currency>>)> {
        let (decimal, group) = self.separators();
        let chars: Vec<char> = expression.chars().collect();
        let mut out = String::new();
        let mut money = Vec::new();
        let mut pending: Option<Currency> = None;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            // `$5`: the symbol was read as a currency, so the digits start a number.
            let after_word = i > 0 && is_word(chars[i - 1]) && pending.is_none();
            let next_digit = |from: usize| {
                chars[from..]
                    .iter()
                    .find(|c| **c != ' ')
                    .is_some_and(char::is_ascii_digit)
            };
            if !after_word && matches!(c, '€' | '$' | '£' | '¥') && next_digit(i + 1) {
                pending = Currency::named(&c.to_string());
                i += 1;
                while chars.get(i) == Some(&' ') {
                    i += 1;
                }
                continue;
            }
            if !after_word && c.is_ascii_alphabetic() {
                let start = i;
                let mut end = i;
                while end < chars.len() && is_word(chars[end]) {
                    end += 1;
                }
                let word: String = chars[start..end].iter().collect();
                // `EUR 1200`: a code, a space, a number.
                if let Some(currency) = Currency::named(&word)
                    .filter(|_| chars.get(end) == Some(&' ') && next_digit(end))
                {
                    pending = Some(currency);
                    i = end;
                    while chars.get(i) == Some(&' ') {
                        i += 1;
                    }
                    continue;
                }
                out.push_str(&word);
                i = end;
                continue;
            }
            if c == '"' {
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                    if chars[i - 1] == '"' {
                        if chars.get(i) == Some(&'"') {
                            out.push('"');
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }
            } else if c == '[' {
                return Err(Error::Formula(
                    "a tape has no cells: name a line or write #n".into(),
                ));
            } else if c == '#' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
                let digits: String = chars[i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                let n: usize = digits.parse().unwrap_or(0);
                if n == 0 {
                    return Err(Error::Formula("lines count from #1".into()));
                }
                out.push_str(&ExprList::reference(n - 1));
                i += 1 + digits.len();
            } else if c.is_ascii_digit() && !after_word {
                let start = i;
                while i < chars.len() {
                    let d = chars[i];
                    let spaced = matches!(d, ' ' | '\u{a0}' | '\u{202f}')
                        && group == '\u{a0}'
                        && chars.get(i + 1).is_some_and(char::is_ascii_digit);
                    let exponent = matches!(d, 'e' | 'E')
                        && chars[i + 1..]
                            .iter()
                            .find(|c| !matches!(c, '+' | '-'))
                            .is_some_and(char::is_ascii_digit);
                    if exponent {
                        i += 1;
                        if matches!(chars.get(i), Some('+' | '-')) {
                            i += 1;
                        }
                        continue;
                    }
                    if d.is_ascii_digit() || d == decimal || d == group || spaced {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let run: String = chars[start..i].iter().collect();
                let number = crate::csv::number(&run, decimal, group).ok_or_else(|| {
                    Error::Formula(format!(
                        "{run} is not a number in {}: write {}",
                        self.locale_name(),
                        locale::sample(self.locale.as_ref())
                    ))
                })?;
                out.push_str(&number);
                // `1200 EUR`, `1200EUR`, `5 €`: a currency after the number.
                let mut j = i;
                while chars.get(j) == Some(&' ') {
                    j += 1;
                }
                let mut end = j;
                while end < chars.len() && chars[end].is_ascii_alphabetic() {
                    end += 1;
                }
                let after = if end > j && !chars.get(end).is_some_and(|c| is_word(*c)) {
                    Currency::named(&chars[j..end].iter().collect::<String>()).map(|c| (c, end))
                } else {
                    chars
                        .get(j)
                        .and_then(|c| matches!(c, '€' | '$' | '£' | '¥').then(|| c.to_string()))
                        .and_then(|symbol| Currency::named(&symbol))
                        .map(|c| (c, j + 1))
                };
                let before = pending.take();
                money.push(match (before, after) {
                    (Some(_), Some(_)) => {
                        return Err(Error::Formula(format!(
                            "{run} has a currency on both sides: write one"
                        )));
                    }
                    (Some(c), None) => Some(c),
                    (None, Some((c, end))) => {
                        i = end;
                        Some(c)
                    }
                    (None, None) => None,
                });
            } else {
                out.push(c);
                i += 1;
            }
        }
        Ok((out, money))
    }

    /// Line `i` as it would be typed: display form, `#n` for a reference, the locale's decimal,
    /// and each number's currency code after it.
    fn shown(&self, i: usize, entry: &Entry) -> String {
        let money = self.money.get(i).map_or(&[][..], Vec::as_slice);
        let expression = parse(&entry.formula).map_or_else(
            |_| entry.formula.clone(),
            |expr| self.localised(&Bare(&expr).to_string(), money),
        );
        match &entry.name {
            Some(name) => format!("{name} = {expression}"),
            None => expression,
        }
    }

    /// Display form → the tape's spelling: `B3` as `#3`, `0.5` as `0,5` in de-DE, and `money[k]`
    /// after the k-th number.
    fn localised(&self, text: &str, money: &[Option<Currency>]) -> String {
        let (decimal, _) = self.separators();
        let mut money = money.iter();
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let after_word = i > 0 && is_word(chars[i - 1]);
            if c == '"' {
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                    if chars[i - 1] == '"' {
                        if chars.get(i) == Some(&'"') {
                            out.push('"');
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }
            } else if (c.is_ascii_alphabetic() || c == '_') && !after_word {
                let start = i;
                while i < chars.len() && is_word(chars[i]) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                match word.strip_prefix('B') {
                    Some(row) if chars.get(i) != Some(&'(') && row.parse::<usize>().is_ok() => {
                        out.push('#');
                        out.push_str(row);
                    }
                    _ => out.push_str(&word),
                }
            } else if c.is_ascii_digit() && !after_word {
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric()
                        || chars[i] == '.'
                        || (matches!(chars[i], '+' | '-') && matches!(chars[i - 1], 'E' | 'e')))
                {
                    out.push(if chars[i] == '.' { decimal } else { chars[i] });
                    i += 1;
                }
                if let Some(Some(currency)) = money.next() {
                    out.push(' ');
                    out.push_str(currency.code);
                }
            } else {
                out.push(c);
                i += 1;
            }
        }
        out
    }

    fn value_text(&self, value: &Value, format: Option<crate::numfmt::Format>) -> String {
        let (decimal, _) = self.separators();
        match value {
            Value::Number(n) if format.is_some() => format.map_or_else(String::new, |f| {
                f.in_locale(self.locale.clone())
                    .render(&CellValue::Number(*n), 0)
            }),
            Value::Number(n) => {
                // Fifteen significant digits, as a spreadsheet shows: 0.1+0.2 is 0.3.
                let rounded: f64 = format!("{n:.14e}").parse().unwrap_or(*n);
                rounded.to_string().replace('.', &decimal.to_string())
            }
            Value::Text(s) => s.clone(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
            Value::Error(e) => e.name().to_owned(),
            Value::Empty => String::new(),
        }
    }
}

fn is_function(name: &str) -> bool {
    crate::formula::funcs::catalog()
        .iter()
        .any(|info| info.name.eq_ignore_ascii_case(name))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '$'
}

fn handles(group: &[usize]) -> String {
    group
        .iter()
        .map(|h| format!("#{h}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `name = rest`, when a line starts with an identifier and one `=`.
fn definition(text: &str) -> Option<(&str, &str)> {
    let end = text
        .char_indices()
        .find(|&(_, c)| !(c.is_alphanumeric() || c == '_'))
        .map_or(text.len(), |(i, _)| i);
    let name = &text[..end];
    let first = name.chars().next()?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    let rest = text[end..].trim_start().strip_prefix('=')?;
    Some((name, rest))
}

/// `text` without a currency before or after it, and that currency.
fn strip_currency(text: &str) -> (&str, Option<Currency>) {
    let symbol = |t: &str| {
        t.chars()
            .next()
            .map(String::from)
            .and_then(|c| Currency::named(&c))
    };
    if let Some(c) = symbol(text).filter(|_| text.starts_with(['€', '$', '£', '¥'])) {
        return (
            text[text.chars().next().map_or(0, char::len_utf8)..].trim(),
            Some(c),
        );
    }
    if let Some(last) = text
        .chars()
        .last()
        .filter(|c| matches!(c, '€' | '$' | '£' | '¥'))
    {
        return (
            text[..text.len() - last.len_utf8()].trim(),
            Currency::named(&last.to_string()),
        );
    }
    let letters = text.len()
        - text
            .trim_end_matches(|c: char| c.is_ascii_alphabetic())
            .len();
    if letters > 0 {
        let (number, code) = text.split_at(text.len() - letters);
        if let Some(c) = Currency::named(code) {
            return (number.trim(), Some(c));
        }
    }
    if let Some((code, number)) = text.split_once(' ')
        && let Some(c) = Currency::named(code)
    {
        return (number.trim(), Some(c));
    }
    (text, None)
}

/// Every number literal of `expr`, in the order they stand.
fn literals(expr: &Expr, out: &mut Vec<f64>) {
    match expr {
        Expr::Number(n) => out.push(*n),
        Expr::Call { args, .. } => args.iter().for_each(|a| literals(a, out)),
        Expr::Prefix(_, e) | Expr::Postfix(_, e) | Expr::Paren(e) => literals(e, out),
        Expr::Binary(_, a, b) => {
            literals(a, out);
            literals(b, out);
        }
        _ => {}
    }
}

/// A number as typed, perhaps negated or a percentage: what promotion stores as a value.
/// The first cell `expr` reads that is not a line of a tape — anything but column B of its own
/// sheet — as A1.
fn outside_the_tape(expr: &Expr) -> Option<String> {
    let here = |r: &crate::formula::lex::CellRef| {
        r.sheet.is_none() && r.col.is_some_and(|c| c.index == 1) && r.row.is_some()
    };
    match expr {
        Expr::Ref(r) => {
            let inside = r.source.is_none() && here(&r.start) && r.end.as_ref().is_none_or(here);
            (!inside).then(|| {
                let at = |c: &crate::formula::lex::CellRef| {
                    crate::a1::format(
                        c.sheet.as_deref(),
                        Pos::new(c.row.map_or(0, |a| a.index), c.col.map_or(0, |a| a.index)),
                    )
                };
                match &r.end {
                    Some(end) => format!("{}:{}", at(&r.start), at(end)),
                    None => at(&r.start),
                }
            })
        }
        Expr::Call { args, .. } => args.iter().find_map(outside_the_tape),
        Expr::Prefix(_, e) | Expr::Postfix(_, e) | Expr::Paren(e) => outside_the_tape(e),
        Expr::Binary(_, a, b) => outside_the_tape(a).or_else(|| outside_the_tape(b)),
        _ => None,
    }
}

/// The row a named range promotion wrote covers, from `[$Sheet.$B$n]`, the sheet's name quoted or
/// not; `None` for anything else.
fn named_row(target: &str, sheet: &str) -> Option<u32> {
    let inner = target.strip_prefix("[$")?.strip_suffix(']')?;
    let quoted = format!("'{}'", sheet.replace('\'', "''"));
    let rest = inner
        .strip_prefix(&quoted)
        .or_else(|| inner.strip_prefix(sheet))?;
    rest.strip_prefix(".$B$")?.parse().ok()
}

/// The shortest spelling of `n` that reads back as `n`.
fn shortest(n: f64) -> String {
    format!("{n}")
}

fn is_input(expr: &Expr) -> bool {
    match expr {
        Expr::Number(_) => true,
        Expr::Prefix(Op::Sub | Op::Add, e) | Expr::Postfix(Op::Percent, e) => is_input(e),
        _ => false,
    }
}

/// `expr` with every reference to a named line spelled as the name.
fn named_references(expr: &Expr, entries: &[Entry]) -> Expr {
    let go = |e: &Expr| Box::new(named_references(e, entries));
    match expr {
        Expr::Ref(r) if r.end.is_none() && r.start.sheet.is_none() => r
            .start
            .row
            .and_then(|row| entries.get(row.index as usize))
            .and_then(|e| e.name.clone())
            .map_or_else(|| expr.clone(), Expr::Name),
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(|a| named_references(a, entries)).collect(),
        },
        Expr::Prefix(op, e) => Expr::Prefix(*op, go(e)),
        Expr::Postfix(op, e) => Expr::Postfix(*op, go(e)),
        Expr::Binary(op, a, b) => Expr::Binary(*op, go(a), go(b)),
        Expr::Paren(e) => Expr::Paren(go(e)),
        _ => expr.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape(lines: &[&str]) -> Tape {
        let mut t = Tape::default();
        for l in lines {
            t.push(l).unwrap();
        }
        t
    }

    fn shown(t: &Tape) -> Vec<(String, String)> {
        t.lines().into_iter().map(|l| (l.text, l.value)).collect()
    }

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn lines_read_each_other_by_handle_and_name() {
        let t = tape(&["vat = 0.19", "net = 1200", "net * (1 + vat)", "#3 / 12"]);
        assert_eq!(
            shown(&t),
            pairs(&[
                ("vat = 0.19", "0.19"),
                ("net = 1200", "1200"),
                ("net*(1+vat)", "1428"),
                ("#3/12", "119"),
            ])
        );
    }

    #[test]
    fn completion_offers_the_tapes_names_first_with_their_values() {
        let t = tape(&["foo = 3", "food = foo * 2"]);
        let mut assist = Assist::default();
        t.assist(&mut assist, "1 + fo", 6);
        let offered: Vec<(&str, &str)> = assist
            .offers
            .iter()
            .map(|o| (o.name.as_str(), o.detail.as_str()))
            .collect();
        assert_eq!(offered[..2], [("foo", "#1 = 3"), ("food", "#2 = 6")]);
        assert!(offered[2..].iter().any(|(name, _)| *name == "FORECAST"));
        assert_eq!(assist.accept(), Some((4..6, "foo".to_owned())));
        // A function is still a call, offered with its `(`.
        t.assist(&mut assist, "ROU", 3);
        assert_eq!(assist.offers[0].insert, "ROUND(");
    }

    #[test]
    fn a_line_is_coloured_by_what_each_word_is() {
        let t = tape(&["foo = 3", "2"]);
        let text = "bar = foo + #2 * ROUND(baz; 2) + #9 & \"#1\"";
        let inks: Vec<(&str, TapeInk)> = t
            .spans(text, None)
            .into_iter()
            .map(|s| (&text[s.range], s.ink))
            .collect();
        assert_eq!(
            inks,
            [
                ("bar", TapeInk::Definition),
                ("foo", TapeInk::Name),
                ("#2", TapeInk::Handle),
                ("ROUND", TapeInk::Function),
                ("baz", TapeInk::Unknown),
                ("2", TapeInk::Number),
                ("#9", TapeInk::Unknown),
                ("\"#1\"", TapeInk::Text),
            ]
        );
        // A comparison names nothing, and `#2/12` is a handle, then a number.
        let inks: Vec<TapeInk> = t
            .spans("=foo=#2/12", None)
            .into_iter()
            .map(|s| s.ink)
            .collect();
        assert_eq!(inks, [TapeInk::Name, TapeInk::Handle, TapeInk::Number]);
        assert_eq!(
            t.spans("  B2", None)[0].ink,
            TapeInk::Unknown,
            "a tape has no cells"
        );
        assert!(
            t.spans("2 * fo", Some(6))
                .iter()
                .all(|s| s.ink != TapeInk::Unknown),
            "the word at the caret is still being typed"
        );
    }

    #[test]
    fn a_leading_equals_sign_is_a_comparison_not_a_definition() {
        let t = tape(&["a = 2", "=a=2", "(a = 3)"]);
        assert_eq!(t.lines()[1].value, "TRUE");
        assert_eq!(t.lines()[2].value, "FALSE");
        assert_eq!(t.lines()[1].name, None);
    }

    #[test]
    fn numbers_are_read_and_shown_in_the_tapes_locale() {
        let mut t = Tape::new(Locale::parse("de-DE"));
        t.push("1.234,5 * 2").unwrap();
        t.push("ROUND(#1/3; 2)").unwrap();
        assert_eq!(
            shown(&t),
            pairs(&[("1234,5*2", "2469"), ("ROUND(#1/3;2)", "823")])
        );
        let refused = t.push("1.5 + 1").unwrap_err().to_string();
        assert!(refused.contains("not a number in de-DE"), "{refused}");

        let mut fr = Tape::new(Locale::parse("fr-FR"));
        fr.push("1 234,5 + 1").unwrap();
        assert_eq!(fr.lines()[0].value, "1235,5");
    }

    #[test]
    fn a_cell_reference_is_refused_and_a_handle_in_a_string_is_text() {
        let mut t = tape(&["1"]);
        assert!(
            t.push("B2 * 2")
                .unwrap_err()
                .to_string()
                .contains("no cells")
        );
        assert!(t.push("[.B1]").is_err());
        t.push("\"#1\" & #1").unwrap();
        assert_eq!(t.lines()[1].value, "#11");
    }

    #[test]
    fn floating_point_noise_is_not_shown() {
        assert_eq!(tape(&["0.1 + 0.2"]).lines()[0].value, "0.3");
    }

    #[test]
    fn a_handle_past_the_end_reads_as_an_error_and_a_cycle_is_reported() {
        let t = tape(&["#5", "#3 + 1", "#2 * 2"]);
        assert!(t.lines()[0].error);
        assert_eq!(t.cycles(), [vec![2, 3]]);
        assert_eq!(t.lines()[1].value, "#NUM!");
    }

    #[test]
    fn deleting_and_inserting_renumber_the_handles() {
        let mut t = tape(&["2", "3", "#1 * #2"]);
        t.insert(0, "10").unwrap();
        assert_eq!(t.lines()[3].text, "#2*#3");
        t.delete(0).unwrap();
        assert_eq!(t.lines()[2].text, "#1*#2");
        assert_eq!(t.lines()[2].value, "6");
    }

    #[test]
    fn extract_makes_a_named_input_before_the_line_and_undo_takes_it_back() {
        let mut t = tape(&["1200", "#1 * 1.19"]);
        t.extract(1, "1.19", None, "rate").unwrap();
        assert_eq!(
            shown(&t),
            pairs(&[
                ("1200", "1200"),
                ("rate = 1.19", "1.19"),
                ("#1*rate", "1428")
            ])
        );
        assert!(t.undo());
        assert_eq!(t.lines().len(), 2);
        assert!(t.redo());
        assert_eq!(t.lines().len(), 3);
    }

    #[test]
    fn extract_carries_a_percent_and_refuses_what_it_cannot_place() {
        let mut t = tape(&["100 * 5% + 5"]);
        assert!(t.extract(0, "5", None, "rate").is_err());
        t.extract(0, "5%", Some(1), "rate").unwrap();
        assert_eq!(t.lines()[0].text, "rate = 5%");
        assert_eq!(t.lines()[1].text, "100*rate+5");
        assert!(t.extract(1, "7", None, "x").is_err());
        assert!(t.extract(1, "5", None, "rate").is_err());
    }

    #[test]
    fn extract_reads_the_literal_in_the_tapes_locale() {
        let mut t = Tape::new(Locale::parse("de-DE"));
        t.push("100 * 0,19").unwrap();
        t.extract(0, "0,19", None, "vat").unwrap();
        assert_eq!(t.lines()[0].text, "vat = 0,19");
    }

    #[test]
    fn promotion_lays_lines_out_as_rows_with_names_as_named_ranges() {
        let t = tape(&["vat = 0.19", "1200", "#2 * (1 + #1)", "-5"]);
        let doc = t.promote().unwrap();
        let sheet = &doc.sheets[0];
        assert_eq!(sheet.get(Pos::new(0, 0)), CellValue::Text("vat".into()));
        assert_eq!(sheet.get(Pos::new(1, 0)), CellValue::Text("#2".into()));
        assert_eq!(sheet.formula(Pos::new(0, 1)), None);
        assert_eq!(sheet.get(Pos::new(0, 1)), CellValue::Number(0.19));
        assert_eq!(sheet.formula(Pos::new(2, 1)), Some("of:=[.B2]*(1+vat)"));
        assert_eq!(sheet.get(Pos::new(2, 1)), CellValue::Number(1428.0));
        assert_eq!(sheet.get(Pos::new(3, 1)), CellValue::Number(-5.0));
        assert_eq!(doc.name("VAT"), Some("[$Sheet1.$B$1]"));
    }

    #[test]
    fn a_tape_with_a_cycle_is_not_promoted() {
        let t = tape(&["#2", "#1"]);
        assert!(
            t.promote()
                .unwrap_err()
                .to_string()
                .contains("#1, #2 read each other")
        );
        assert_eq!(
            tape(&["#1 + 1"]).circles().as_deref(),
            Some("#1 reads itself")
        );
    }

    #[test]
    fn a_currency_is_typed_by_its_code_or_symbol_on_either_side() {
        let t = tape(&[
            "1200 eur", "EUR 1200", "1200EUR", "€1200", "$5", "5 £", "1200 jpy",
        ]);
        let values: Vec<String> = t.lines().into_iter().map(|l| l.value).collect();
        assert_eq!(
            values,
            [
                "1,200.00\u{a0}€",
                "1,200.00\u{a0}€",
                "1,200.00\u{a0}€",
                "1,200.00\u{a0}€",
                "$5.00",
                "£5.00",
                "1,200\u{a0}¥",
            ]
        );
        // Shown again as it would be typed: the code after the number.
        assert_eq!(t.lines()[1].text, "1200 EUR");
        assert_eq!(t.lines()[4].text, "5 USD");
        let de = {
            let mut t = Tape::new(Locale::parse("de-DE"));
            t.push("net = 1.200,5 eur").unwrap();
            t
        };
        assert_eq!(de.lines()[0].value, "1.200,50\u{a0}€");
        assert_eq!(de.lines()[0].text, "net = 1200,5 EUR");
    }

    #[test]
    fn a_kind_is_carried_through_the_arithmetic() {
        let t = tape(&[
            "vat = 19%",
            "net = 1200 eur",
            "net * vat",
            "net * (1 + vat)",
            "net / net",
            "SUM(#2:#4)",
            "ROUND(net / 7; 2)",
            "19% * 1200",
            "net / 12",
        ]);
        let values: Vec<String> = t.lines().into_iter().map(|l| l.value).collect();
        assert_eq!(
            values,
            [
                "19%",
                "1,200.00\u{a0}€",
                "228.00\u{a0}€",
                "1,428.00\u{a0}€",
                "1",
                "2,856.00\u{a0}€",
                "171.43\u{a0}€",
                "228",
                "100.00\u{a0}€",
            ]
        );
        assert_eq!(tape(&["12.5%"]).lines()[0].value, "12.5%");
        assert_eq!(tape(&["1/3 * 100%"]).lines()[0].value, "0.333333333333333");
        assert_eq!(tape(&["10% / 3"]).lines()[0].value, "3.33%");
    }

    #[test]
    fn two_currencies_do_not_add_up() {
        let mut t = tape(&["a = 100 eur"]);
        let refused = t.push("a + 50 usd").unwrap_err().to_string();
        assert!(refused.contains("EUR and USD do not add up"), "{refused}");
        assert!(t.push("a > 50 usd").is_err(), "nor compare");
        // A rate between them is a plain ratio, and converts.
        t.push("eur_per_usd = 92 eur / 100 usd").unwrap();
        t.push("a + 50 * eur_per_usd").unwrap();
        assert_eq!(t.lines()[2].value, "146.00\u{a0}€");
        // An edit that would make a later line mix is refused too, and changes nothing.
        let mut t = tape(&["a = 100", "a + 50 usd"]);
        let refused = t.edit(0, "a = 100 eur").unwrap_err().to_string();
        assert!(refused.starts_with("#2:"), "{refused}");
        assert_eq!(t.lines()[0].text, "a = 100");
    }

    #[test]
    fn a_currency_is_coloured_as_its_number_and_moves_with_an_extract() {
        let t = tape(&["x = 1"]);
        let spans = t.spans("1200 eur * 2", None);
        assert_eq!(spans[1].range, 5..8);
        assert_eq!(spans[1].ink, TapeInk::Number);
        let mut t = tape(&["1200 eur * 19%"]);
        t.extract(0, "1200 eur", None, "price").unwrap();
        assert_eq!(t.lines()[0].text, "price = 1200 EUR");
        assert_eq!(t.lines()[1].text, "price*19%");
        assert_eq!(t.lines()[1].value, "228.00\u{a0}€");
        assert!(t.undo());
        assert_eq!(t.lines()[0].text, "1200 EUR*19%");
    }

    #[test]
    fn promotion_formats_each_cell_for_its_kind() {
        let t = tape(&["vat = 19%", "net = 1200 eur", "net * vat", "2 + 2"]);
        let doc = t.promote().unwrap();
        let sheet = &doc.sheets[0];
        let shown = |row| {
            sheet.format(Pos::new(row, 1)).map(|f| {
                f.clone()
                    .render(&sheet.get(Pos::new(row, 1)), doc.null_date)
            })
        };
        assert_eq!(shown(0).as_deref(), Some("19%"));
        assert_eq!(shown(1).as_deref(), Some("1,200.00\u{a0}€"));
        assert_eq!(shown(2).as_deref(), Some("228.00\u{a0}€"));
        assert_eq!(shown(3), None);
    }
}

#[cfg(test)]
mod read_back {
    use super::*;
    use crate::locale::Locale;

    fn tape(locale: Option<&str>, lines: &[&str]) -> Tape {
        let mut t = Tape::new(locale.and_then(Locale::parse));
        for l in lines {
            t.push(l).unwrap();
        }
        t
    }

    fn written(doc: Document, form: Form) -> Document {
        let app = App::new();
        app.open_document(doc).unwrap();
        crate::read_bytes("tape", &app.save_bytes(form).unwrap()).unwrap()
    }

    fn shown(t: &Tape) -> Vec<(String, String)> {
        t.lines().into_iter().map(|l| (l.text, l.value)).collect()
    }

    /// The same document, changed by `change`, read as a tape: the reason it is not one.
    fn refused(change: impl FnOnce(&mut Document)) -> String {
        let mut doc = tape(None, &["vat = 0.19", "net = 1200 EUR", "net * (1 + vat)"])
            .promote()
            .unwrap();
        change(&mut doc);
        Tape::from_sheet(&doc).unwrap_err().to_string()
    }

    use grind_core::Form;

    #[test]
    fn every_kind_of_line_reads_back_from_either_form_as_it_was_typed() {
        let tapes = [
            tape(
                None,
                &["vat = 0.19", "net = 1200", "net * (1 + vat)", "#3 / 12"],
            ),
            tape(
                None,
                &["rent = 950 EUR", "fees = 12.5%", "rent * (1 + fees)", "-3"],
            ),
            tape(None, &["5 EUR * 3", "#1 + 2", "SUM(#1:#2)", "\"done\""]),
            tape(Some("de-DE"), &["preis = 1,5", "preis * 2", "#2 / 3"]),
            tape(None, &["1e21", "0.1 + 0.2", "x = 7%", "x * 100"]),
        ];
        for t in tapes {
            for form in [Form::Flat, Form::Package] {
                let back = Tape::from_sheet(&written(t.promote().unwrap(), form)).unwrap();
                assert_eq!(shown(&back), shown(&t), "{form:?}");
                assert_eq!(back.locale, t.locale);
            }
        }
    }

    /// `#1 + 2 EUR` promotes as `#1 + 2` would when `#1` is euros already: the same cells, and
    /// so the same tape, shown the shorter way.
    #[test]
    fn a_currency_the_line_has_anyway_reads_back_unspelled() {
        let t = tape(None, &["5 EUR", "#1 + 2 EUR"]);
        let back = Tape::from_sheet(&written(t.promote().unwrap(), Form::Flat)).unwrap();
        let values = |t: &Tape| t.lines().into_iter().map(|l| l.value).collect::<Vec<_>>();
        assert_eq!(values(&back), values(&t));
        assert_eq!(back.lines()[1].text, "#1+2");
    }

    #[test]
    fn a_renamed_sheet_is_still_a_tape() {
        let t = tape(None, &["a = 1", "a + 1"]);
        let mut doc = t.promote().unwrap();
        doc.sheets[0].name = "Quote".to_owned();
        doc.names.insert("a".to_owned(), "[$Quote.$B$1]".to_owned());
        assert_eq!(shown(&Tape::from_sheet(&doc).unwrap()), shown(&t));
    }

    /// What a case does to a promoted tape.
    type Change = Box<dyn FnOnce(&mut Document)>;

    #[test]
    fn a_sheet_a_tape_would_not_write_says_why_it_is_not_one() {
        let cases: [(&str, Change); 9] = [
            (
                "C2 holds something",
                Box::new(|d| d.sheets[0].set(Pos::new(1, 2), CellValue::Number(1.0))),
            ),
            (
                "A4 is not a name or #4",
                Box::new(|d| d.sheets[0].set(Pos::new(4, 1), CellValue::Number(1.0))),
            ),
            (
                "B3 reads A1",
                Box::new(|d| d.sheets[0].set_formula(Pos::new(2, 1), "of:=[.A1]*2".into())),
            ),
            (
                "B3 reads Other.B1",
                Box::new(|d| {
                    d.sheets[0].set_formula(Pos::new(2, 1), "of:=[$Other.B1]*2".into());
                }),
            ),
            (
                "it has 2 sheets",
                Box::new(|d| d.sheets.push(crate::model::Sheet::new("Other"))),
            ),
            (
                "B1 is neither a number nor a formula",
                Box::new(|d| d.sheets[0].set(Pos::new(0, 1), CellValue::Text("x".into()))),
            ),
            (
                "A2, net total, is not a name",
                Box::new(|d| d.sheets[0].set(Pos::new(1, 0), CellValue::Text("net total".into()))),
            ),
            (
                "its named ranges are not the names its lines have: tax",
                Box::new(|d| {
                    d.names.insert("tax".into(), "[$Sheet1.$B$3]".into());
                }),
            ),
            (
                "B2's number format is not one a tape writes",
                Box::new(|d| {
                    let format = Kind::Percent.format(0.5, 15).unwrap();
                    d.sheets[0].set_format(Pos::new(1, 1), format);
                    d.sheets[0].set(Pos::new(1, 1), CellValue::Number(0.123_456));
                }),
            ),
        ];
        for (why, change) in cases {
            let said = refused(change);
            assert!(said.starts_with("not a tape: "), "{said}");
            assert!(said.contains(why), "wanted {why:?}, got {said:?}");
        }
    }
}
