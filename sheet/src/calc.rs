// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tape: a scratch calculator whose lines become a spreadsheet's rows (`doc/calc.md`). An
//! experiment. A line is `[name =] expression`, `#n` reads line n, and numbers are read in the
//! tape's locale with the CSV import's rule.

use std::path::Path;

use crate::formula::display;
use crate::formula::eval::to_cell;
use crate::formula::lex::Op;
use crate::formula::list::replace_literal;
use crate::formula::parse::{Expr, parse};
use crate::formula::serialize::Bare;
use crate::formula::value::Value;
use crate::formula::{Entry, ExprList};
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

#[derive(Clone, Debug, Default)]
pub struct Tape {
    list: ExprList,
    locale: Option<Locale>,
    undo: Vec<ExprList>,
    redo: Vec<ExprList>,
}

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
        self.list
            .entries()
            .iter()
            .zip(self.list.values())
            .enumerate()
            .map(|(i, (entry, value))| TapeLine {
                handle: i + 1,
                name: entry.name.clone(),
                text: self.shown(entry),
                value: self.value_text(value),
                error: matches!(value, Value::Error(_)),
            })
            .collect()
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

    /// Add a line at the end; returns its handle.
    pub fn push(&mut self, text: &str) -> Result<usize> {
        self.insert(self.len(), text)?;
        Ok(self.len())
    }

    /// Put a line before the one at `at` (0-based), renumbering those after it.
    pub fn insert(&mut self, at: usize, text: &str) -> Result<()> {
        let (name, formula) = self.read(text)?;
        self.edit_with(|list| list.insert(at, name.as_deref(), &formula))
    }

    /// Replace the line at `at`.
    pub fn edit(&mut self, at: usize, text: &str) -> Result<()> {
        let (name, formula) = self.read(text)?;
        self.edit_with(|list| list.set(at, name.as_deref(), &formula))
    }

    pub fn delete(&mut self, at: usize) -> Result<()> {
        self.edit_with(|list| list.remove(at))
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
        let value = self.number(literal.trim().trim_end_matches('%').trim())?;
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
        self.edit_with(|list| {
            list.insert(at, Some(name), &input.to_string())?;
            let line = &list.entries()[at + 1];
            let shifted = parse(&line.formula).map_err(|e| Error::Formula(e.to_string()))?;
            let (rewritten, _) =
                replace_literal(&shifted, value, nth, &with).map_err(Error::Formula)?;
            let name = line.name.clone();
            list.set(at + 1, name.as_deref(), &rewritten.to_string())
        })
    }

    pub fn undo(&mut self) -> bool {
        let Some(before) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.list, before));
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(after) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.list, after));
        true
    }

    /// The tape as a spreadsheet: line n is row n, its label (name, or `#n`) in A and its number
    /// or formula in B, a name a named range over its B cell, and `#k` the name of line k or a
    /// reference to it. Refused while there is a cycle.
    pub fn promote(&self) -> Result<Document> {
        if let Some(cycle) = self.cycles().first() {
            return Err(Error::Formula(format!(
                "{} read each other in a circle; break it before saving",
                handles(cycle)
            )));
        }
        let mut doc = self.list.document();
        doc.locale.clone_from(&self.locale);
        let entries = self.list.entries();
        for (i, (entry, value)) in entries.iter().zip(self.list.values()).enumerate() {
            let row = u32::try_from(i).unwrap_or(u32::MAX);
            let sheet = &mut doc.sheets[0];
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

    /// [`Tape::promote`], written: `.ods` a package, `.grind` the projection, anything else flat.
    pub fn save(&self, path: &Path) -> Result<()> {
        let app = App::new();
        app.open_document(self.promote()?)?;
        app.save_file(path)
    }

    fn edit_with(&mut self, f: impl FnOnce(&mut ExprList) -> Result<Vec<usize>>) -> Result<()> {
        let mut next = self.list.clone();
        f(&mut next)?;
        self.undo.push(std::mem::replace(&mut self.list, next));
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

    /// A typed line → its name and its canonical formula.
    fn read(&self, text: &str) -> Result<(Option<String>, String)> {
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
        let typed = self.canonical_numbers(expression)?;
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
        Ok((name, canonical.trim_start_matches('=').to_owned()))
    }

    /// `expression` with every number respelled in OpenFormula's, `#n` as a reference to line n,
    /// and bracketed text refused.
    fn canonical_numbers(&self, expression: &str) -> Result<String> {
        let (decimal, group) = self.separators();
        let chars: Vec<char> = expression.chars().collect();
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
            } else {
                out.push(c);
                i += 1;
            }
        }
        Ok(out)
    }

    /// An entry as it would be typed: display form, `#n` for a reference, the locale's decimal.
    fn shown(&self, entry: &Entry) -> String {
        let expression = parse(&entry.formula).map_or_else(
            |_| entry.formula.clone(),
            |expr| self.localised(&Bare(&expr).to_string()),
        );
        match &entry.name {
            Some(name) => format!("{name} = {expression}"),
            None => expression,
        }
    }

    /// Display form → the tape's spelling: `B3` as `#3`, `0.5` as `0,5` in de-DE.
    fn localised(&self, text: &str) -> String {
        let (decimal, _) = self.separators();
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
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                    out.push(if chars[i] == '.' { decimal } else { chars[i] });
                    i += 1;
                }
            } else {
                out.push(c);
                i += 1;
            }
        }
        out
    }

    fn value_text(&self, value: &Value) -> String {
        let (decimal, _) = self.separators();
        match value {
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

/// A number as typed, perhaps negated or a percentage: what promotion stores as a value.
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
        assert!(t.promote().unwrap_err().to_string().contains("#1, #2"));
    }
}
