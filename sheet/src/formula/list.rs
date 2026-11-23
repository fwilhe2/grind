// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A list of expressions that read each other by position or by name: a column of cells nobody
//! has laid out yet (`doc/calc.md` §3). Entry `i` is cell `B(i+1)` of a private one-sheet
//! document, so the engine evaluates it, and a name is a named range over that cell.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::formula::eval::{Address, Engine, to_cell};
use crate::formula::lex::{Axis, CellRef, Op, Reference};
use crate::formula::parse::{Expr, parse};
use crate::formula::value::{FormulaError, Value};
use crate::model::{Document, Pos};
use crate::{Error, Result};

/// The column every entry sits in.
const COLUMN: u32 = 1;

/// One expression: canonical OpenFormula without its `of:=`, reading another entry as `[.Bn]` or
/// by its name.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: Option<String>,
    pub formula: String,
}

#[derive(Clone, Debug, Default)]
pub struct ExprList {
    entries: Vec<Entry>,
    values: Vec<Value>,
    cycles: Vec<Vec<usize>>,
}

impl ExprList {
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    #[must_use]
    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// Every circular group of entries, each in ascending order. Each member evaluates to `#NUM!`,
    /// as a circular reference in a sheet does.
    #[must_use]
    pub fn cycles(&self) -> &[Vec<usize>] {
        &self.cycles
    }

    /// The entry a name belongs to, case-insensitively.
    #[must_use]
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| {
            e.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    }

    /// How a formula in the list spells a reference to entry `i`.
    #[must_use]
    pub fn reference(i: usize) -> String {
        format!("[.B{}]", i + 1)
    }

    /// Put an entry at `at`, moving the ones from there down and every reference to them. Returns
    /// the entries whose value changed.
    pub fn insert(&mut self, at: usize, name: Option<&str>, formula: &str) -> Result<Vec<usize>> {
        if at > self.entries.len() {
            return Err(Error::Formula(format!("there is no position {}", at + 1)));
        }
        if let Some(name) = name {
            self.check_new_name(name, None)?;
        }
        check(formula)?;
        let len = u32::try_from(self.entries.len()).unwrap_or(u32::MAX);
        let at32 = u32::try_from(at).unwrap_or(u32::MAX);
        self.remap(|row| {
            Some(if row >= at32 && row < len {
                row + 1
            } else {
                row
            })
        });
        self.entries.insert(
            at,
            Entry {
                name: name.map(str::to_owned),
                formula: formula.to_owned(),
            },
        );
        self.values.insert(at, Value::Empty);
        Ok(self.evaluate(None))
    }

    /// Change what entry `at` says, and what it is called. Uses of an old name move to the new
    /// one, or to a reference when the name goes.
    pub fn set(&mut self, at: usize, name: Option<&str>, formula: &str) -> Result<Vec<usize>> {
        let Some(old) = self.entries.get(at).map(|e| e.name.clone()) else {
            return Err(Error::Formula(format!("there is no position {}", at + 1)));
        };
        if let Some(name) = name {
            self.check_new_name(name, Some(at))?;
        }
        check(formula)?;
        if let Some(old) = old.as_deref()
            && name.is_none_or(|n| !n.eq_ignore_ascii_case(old))
        {
            let with = match name {
                Some(n) => Expr::Name(n.to_owned()),
                None => Expr::Ref(cell(u32::try_from(at).unwrap_or(u32::MAX))),
            };
            for entry in &mut self.entries {
                if let Ok(expr) = parse(&entry.formula) {
                    entry.formula = rename(&expr, old, &with).to_string();
                }
            }
        }
        self.entries[at] = Entry {
            name: name.map(str::to_owned),
            formula: formula.to_owned(),
        };
        Ok(self.evaluate(Some(at)))
    }

    /// Remove entry `at`. What read it reads `#REF!`; what read it by name, `#NAME?`.
    pub fn remove(&mut self, at: usize) -> Result<Vec<usize>> {
        if at >= self.entries.len() {
            return Err(Error::Formula(format!("there is no position {}", at + 1)));
        }
        let at32 = u32::try_from(at).unwrap_or(u32::MAX);
        self.remap(|row| match row.cmp(&at32) {
            std::cmp::Ordering::Less => Some(row),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(row - 1),
        });
        self.entries.remove(at);
        self.values.remove(at);
        Ok(self.evaluate(None))
    }

    /// The list as a sheet: every entry a formula in column B with its value cached, every name a
    /// named range over its cell.
    #[must_use]
    pub fn document(&self) -> Document {
        let mut doc = Document::default();
        for (i, entry) in self.entries.iter().enumerate() {
            let pos = Pos::new(row(i), COLUMN);
            let sheet = &mut doc.sheets[0];
            sheet.set_formula(pos, format!("of:={}", entry.formula));
            sheet.set(
                pos,
                to_cell(self.values.get(i).cloned().unwrap_or(Value::Empty)),
            );
            if let Some(name) = &entry.name {
                doc.names
                    .insert(name.to_lowercase(), format!("[$Sheet1.$B${}]", i + 1));
                doc.name_spell(name);
            }
        }
        doc
    }

    fn check_new_name(&self, name: &str, except: Option<usize>) -> Result<()> {
        crate::validate_name(name)?;
        match self.index_of(name) {
            Some(i) if Some(i) != except => Err(Error::Formula(format!(
                "{name} is already #{}; edit that line instead",
                i + 1
            ))),
            _ => Ok(()),
        }
    }

    fn remap(&mut self, f: impl Fn(u32) -> Option<u32>) {
        for entry in &mut self.entries {
            if let Ok(expr) = parse(&entry.formula) {
                entry.formula = remap(&expr, &f).to_string();
            }
        }
    }

    /// Every row past the last entry that some entry reads on its own.
    fn past_the_end(&self) -> BTreeSet<u32> {
        fn walk(expr: &Expr, len: u32, out: &mut BTreeSet<u32>) {
            match expr {
                Expr::Ref(r) if r.end.is_none() => {
                    out.extend(r.start.row.map(|a| a.index).filter(|&i| i >= len));
                }
                Expr::Call { args, .. } => args.iter().for_each(|a| walk(a, len, out)),
                Expr::Prefix(_, e) | Expr::Postfix(_, e) | Expr::Paren(e) => walk(e, len, out),
                Expr::Binary(_, a, b) => {
                    walk(a, len, out);
                    walk(b, len, out);
                }
                _ => {}
            }
        }
        let len = row(self.entries.len());
        let mut out = BTreeSet::new();
        for entry in &self.entries {
            if let Ok(expr) = parse(&entry.formula) {
                walk(&expr, len, &mut out);
            }
        }
        out
    }

    /// Which entries each entry reads.
    fn reads(&self) -> Vec<BTreeSet<usize>> {
        self.entries
            .iter()
            .map(|e| {
                let mut out = BTreeSet::new();
                if let Ok(expr) = parse(&e.formula) {
                    self.collect(&expr, &mut out);
                }
                out
            })
            .collect()
    }

    fn collect(&self, expr: &Expr, out: &mut BTreeSet<usize>) {
        let len = self.entries.len();
        match expr {
            Expr::Ref(r) => {
                let end = r.end.as_ref().unwrap_or(&r.start);
                let cols = [r.start.col, end.col];
                if cols.iter().any(|c| c.is_some_and(|c| c.index != COLUMN)) {
                    return;
                }
                let from = r.start.row.map_or(0, |a| a.index as usize);
                let to = end.row.map_or(len.saturating_sub(1), |a| a.index as usize);
                out.extend((from.min(to)..=from.max(to)).filter(|&i| i < len));
            }
            Expr::Name(n) => out.extend(self.index_of(n)),
            Expr::Call { args, .. } => args.iter().for_each(|a| self.collect(a, out)),
            Expr::Prefix(_, e) | Expr::Postfix(_, e) | Expr::Paren(e) => self.collect(e, out),
            Expr::Binary(_, a, b) => {
                self.collect(a, out);
                self.collect(b, out);
            }
            Expr::Number(_) | Expr::Text(_) | Expr::Error(_) | Expr::Empty => {}
        }
    }

    /// Recompute `edited` and everything downstream of it, or everything for `None`, and return
    /// the entries whose value changed.
    fn evaluate(&mut self, edited: Option<usize>) -> Vec<usize> {
        let reads = self.reads();
        self.cycles = cycles(&reads);
        let cyclic: BTreeSet<usize> = self.cycles.iter().flatten().copied().collect();

        let affected: BTreeSet<usize> = match edited {
            None => (0..self.entries.len()).collect(),
            Some(start) => {
                let mut read_by: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
                for (i, r) in reads.iter().enumerate() {
                    for &j in r {
                        read_by.entry(j).or_default().push(i);
                    }
                }
                let mut seen = BTreeSet::from([start]);
                let mut todo = vec![start];
                while let Some(i) = todo.pop() {
                    for &j in read_by.get(&i).into_iter().flatten() {
                        if seen.insert(j) {
                            todo.push(j);
                        }
                    }
                }
                seen
            }
        };

        let at = |i: usize| Address::new(0, Pos::new(row(i), COLUMN));
        let mut overlay: HashMap<Address, Value> = HashMap::new();
        for (i, value) in self.values.iter().enumerate() {
            if cyclic.contains(&i) {
                overlay.insert(at(i), Value::Error(FormulaError::Num));
            } else if !affected.contains(&i) {
                overlay.insert(at(i), value.clone());
            }
        }
        // A single reference past the last entry names a line that is not there.
        for r in self.past_the_end() {
            overlay.insert(
                Address::new(0, Pos::new(r, COLUMN)),
                Value::Error(FormulaError::Ref),
            );
        }
        let doc = self.document();
        let mut engine = Engine::with_overlay(&doc, &overlay);
        let mut changed = Vec::new();
        for i in 0..self.entries.len() {
            if !affected.contains(&i) && !cyclic.contains(&i) {
                continue;
            }
            let value = engine.value(at(i));
            if self.values[i] != value {
                self.values[i] = value;
                changed.push(i);
            }
        }
        changed
    }
}

fn row(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

fn check(formula: &str) -> Result<()> {
    parse(formula)
        .map(|_| ())
        .map_err(|e| Error::Formula(e.to_string()))
}

/// A reference to one entry's cell.
fn cell(row: u32) -> Reference {
    Reference {
        source: None,
        start: CellRef {
            sheet: None,
            sheet_absolute: false,
            col: Some(Axis {
                index: COLUMN,
                absolute: false,
            }),
            row: Some(Axis {
                index: row,
                absolute: false,
            }),
        },
        end: None,
    }
}

/// Every strongly connected group that is a cycle: more than one member, or one that reads
/// itself (Tarjan's algorithm).
fn cycles(reads: &[BTreeSet<usize>]) -> Vec<Vec<usize>> {
    struct State<'a> {
        reads: &'a [BTreeSet<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(s: &mut State, v: usize) {
        s.index[v] = Some(s.next);
        s.low[v] = s.next;
        s.next += 1;
        s.stack.push(v);
        s.on_stack[v] = true;
        for &w in &s.reads[v] {
            match s.index[w] {
                None => {
                    visit(s, w);
                    s.low[v] = s.low[v].min(s.low[w]);
                }
                Some(iw) if s.on_stack[w] => s.low[v] = s.low[v].min(iw),
                Some(_) => {}
            }
        }
        if Some(s.low[v]) == s.index[v] {
            let mut group = Vec::new();
            while let Some(w) = s.stack.pop() {
                s.on_stack[w] = false;
                group.push(w);
                if w == v {
                    break;
                }
            }
            if group.len() > 1 || s.reads[v].contains(&v) {
                group.sort_unstable();
                s.out.push(group);
            }
        }
    }
    let n = reads.len();
    let mut s = State {
        reads,
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for v in 0..n {
        if s.index[v].is_none() {
            visit(&mut s, v);
        }
    }
    s.out.sort();
    s.out
}

/// `expr` with every row of every reference passed through `f`; `None` is `#REF!`. A range whose
/// end goes keeps the rows that stay.
fn remap(expr: &Expr, f: &impl Fn(u32) -> Option<u32>) -> Expr {
    let axis = |a: Option<Axis>, fallback: Option<u32>| -> Option<Option<Axis>> {
        match a {
            None => Some(None),
            Some(a) => f(a.index)
                .or(fallback)
                .map(|index| Some(Axis { index, ..a })),
        }
    };
    let go = |e: &Expr| Box::new(remap(e, f));
    match expr {
        Expr::Ref(r) => {
            let start_row = r.start.row.map(|a| a.index);
            let fixed = match &r.end {
                None => axis(r.start.row, None).map(|row| Reference {
                    start: CellRef {
                        row,
                        ..r.start.clone()
                    },
                    ..r.clone()
                }),
                Some(end) => {
                    let end_row = end.row.map(|a| a.index);
                    // A removed end shrinks the range onto its neighbour inside it.
                    let start = axis(r.start.row, start_row.and_then(|s| f(s + 1)));
                    let finish = axis(end.row, end_row.and_then(|e| e.checked_sub(1).and_then(f)));
                    match (start, finish) {
                        (Some(s), Some(e)) => Some(Reference {
                            start: CellRef {
                                row: s,
                                ..r.start.clone()
                            },
                            end: Some(CellRef {
                                row: e,
                                ..end.clone()
                            }),
                            ..r.clone()
                        }),
                        _ => None,
                    }
                }
            };
            fixed.map_or(Expr::Error(FormulaError::Ref), Expr::Ref)
        }
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(|a| remap(a, f)).collect(),
        },
        Expr::Prefix(op, e) => Expr::Prefix(*op, go(e)),
        Expr::Postfix(op, e) => Expr::Postfix(*op, go(e)),
        Expr::Binary(op, a, b) => Expr::Binary(*op, go(a), go(b)),
        Expr::Paren(e) => Expr::Paren(go(e)),
        Expr::Number(_) | Expr::Text(_) | Expr::Error(_) | Expr::Name(_) | Expr::Empty => {
            expr.clone()
        }
    }
}

/// `expr` with every use of the name `from` replaced by `with`.
fn rename(expr: &Expr, from: &str, with: &Expr) -> Expr {
    let go = |e: &Expr| Box::new(rename(e, from, with));
    match expr {
        Expr::Name(n) if n.eq_ignore_ascii_case(from) => with.clone(),
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(|a| rename(a, from, with)).collect(),
        },
        Expr::Prefix(op, e) => Expr::Prefix(*op, go(e)),
        Expr::Postfix(op, e) => Expr::Postfix(*op, go(e)),
        Expr::Binary(op, a, b) => Expr::Binary(*op, go(a), go(b)),
        Expr::Paren(e) => Expr::Paren(go(e)),
        _ => expr.clone(),
    }
}

/// A number literal found in a formula: its value, and whether a `%` follows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Literal {
    pub value: f64,
    pub percent: bool,
}

/// `expr` with the `nth` (1-based) number literal equal to `value` replaced by `with`, a `%`
/// after it going too. Refused when there is none, or more than one and no `nth`.
pub(crate) fn replace_literal(
    expr: &Expr,
    value: f64,
    nth: Option<usize>,
    with: &Expr,
) -> std::result::Result<(Expr, Literal), String> {
    fn count(expr: &Expr, value: f64) -> usize {
        match expr {
            Expr::Postfix(Op::Percent, inner) if matches!(**inner, Expr::Number(v) if v == value) => {
                1
            }
            Expr::Number(v) if *v == value => 1,
            Expr::Call { args, .. } => args.iter().map(|a| count(a, value)).sum(),
            Expr::Prefix(_, e) | Expr::Postfix(_, e) | Expr::Paren(e) => count(e, value),
            Expr::Binary(_, a, b) => count(a, value) + count(b, value),
            _ => 0,
        }
    }
    fn go(
        expr: &Expr,
        value: f64,
        left: &mut usize,
        with: &Expr,
        found: &mut Option<Literal>,
    ) -> Expr {
        fn hit(percent: bool, value: f64, left: &mut usize, found: &mut Option<Literal>) -> bool {
            if *left == 0 {
                return false;
            }
            *left -= 1;
            if *left == 0 {
                *found = Some(Literal { value, percent });
            }
            *left == 0
        }
        match expr {
            Expr::Postfix(Op::Percent, inner) if matches!(**inner, Expr::Number(v) if v == value) => {
                if hit(true, value, left, found) {
                    with.clone()
                } else {
                    expr.clone()
                }
            }
            Expr::Number(v) if *v == value => {
                if hit(false, value, left, found) {
                    with.clone()
                } else {
                    expr.clone()
                }
            }
            Expr::Call { name, args } => Expr::Call {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| go(a, value, left, with, found))
                    .collect(),
            },
            Expr::Prefix(op, e) => Expr::Prefix(*op, Box::new(go(e, value, left, with, found))),
            Expr::Postfix(op, e) => Expr::Postfix(*op, Box::new(go(e, value, left, with, found))),
            Expr::Binary(op, a, b) => {
                let a = Box::new(go(a, value, left, with, found));
                Expr::Binary(*op, a, Box::new(go(b, value, left, with, found)))
            }
            Expr::Paren(e) => Expr::Paren(Box::new(go(e, value, left, with, found))),
            _ => expr.clone(),
        }
    }
    let shown = Expr::Number(value).to_string();
    let n = count(expr, value);
    let nth = match (n, nth) {
        (0, _) => return Err(format!("there is no {shown} in it")),
        (1, None) => 1,
        (_, None) => {
            return Err(format!("{shown} occurs {n} times: say which, 1 to {n}"));
        }
        (_, Some(k)) if k == 0 || k > n => {
            return Err(format!(
                "{shown} occurs {n} times, so there is no number {k}"
            ));
        }
        (_, Some(k)) => k,
    };
    let mut left = nth;
    let mut found = None;
    let out = go(expr, value, &mut left, with, &mut found);
    Ok((out, found.expect("counted")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(lines: &[(Option<&str>, &str)]) -> ExprList {
        let mut l = ExprList::default();
        for (i, (name, f)) in lines.iter().enumerate() {
            l.insert(i, *name, f).unwrap();
        }
        l
    }

    fn numbers(l: &ExprList) -> Vec<String> {
        l.values()
            .iter()
            .map(|v| match v {
                Value::Number(n) => n.to_string(),
                Value::Error(e) => e.name().to_owned(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn entries_read_each_other_by_reference_and_by_name() {
        let l = list(&[
            (Some("vat"), "0.19"),
            (None, "1200"),
            (None, "[.B2]*(1+vat)"),
            (None, "SUM([.B2:.B3])"),
        ]);
        assert_eq!(numbers(&l), ["0.19", "1200", "1428", "2628"]);
    }

    #[test]
    fn a_name_used_before_it_is_defined_is_ordered_by_the_graph() {
        let l = list(&[(None, "rate*2"), (Some("rate"), "3")]);
        assert_eq!(numbers(&l), ["6", "3"]);
    }

    #[test]
    fn a_name_is_defined_once_and_never_cell_shaped() {
        let mut l = list(&[(Some("vat"), "0.19")]);
        assert!(l.insert(1, Some("VAT"), "1").is_err());
        assert!(l.insert(1, Some("B2"), "1").is_err());
    }

    #[test]
    fn a_cycle_is_num_and_reported_and_what_reads_it_inherits_the_error() {
        let mut l = list(&[
            (Some("a"), "b+1"),
            (Some("b"), "a+1"),
            (None, "[.B3]"),
            (None, "[.B1]*2"),
            (None, "5"),
        ]);
        assert_eq!(l.cycles(), [vec![0, 1], vec![2]]);
        assert_eq!(numbers(&l), ["#NUM!", "#NUM!", "#NUM!", "#NUM!", "5"]);
        l.set(1, Some("b"), "1").unwrap();
        assert_eq!(l.cycles().len(), 1);
        assert_eq!(numbers(&l)[..2], ["2", "1"]);
    }

    #[test]
    fn inserting_moves_the_references_and_removing_breaks_only_what_read_it() {
        let mut l = list(&[(None, "2"), (None, "[.B1]*10"), (None, "[.B2]+1")]);
        l.insert(0, None, "7").unwrap();
        assert_eq!(l.entries()[2].formula, "[.B2]*10");
        assert_eq!(l.entries()[3].formula, "[.B3]+1");
        assert_eq!(numbers(&l), ["7", "2", "20", "21"]);
        l.remove(1).unwrap();
        assert_eq!(l.entries()[1].formula, "#REF!*10");
        assert_eq!(l.entries()[2].formula, "[.B2]+1");
        assert_eq!(numbers(&l), ["7", "#REF!", "#REF!"]);
    }

    #[test]
    fn removing_the_end_of_a_range_shrinks_it() {
        let mut l = list(&[
            (None, "1"),
            (None, "2"),
            (None, "3"),
            (None, "SUM([.B1:.B3])"),
        ]);
        l.remove(2).unwrap();
        assert_eq!(l.entries()[2].formula, "SUM([.B1:.B2])");
        assert_eq!(numbers(&l)[2], "3");
    }

    #[test]
    fn renaming_an_entry_carries_its_uses_and_unnaming_turns_them_into_references() {
        let mut l = list(&[(Some("x"), "4"), (None, "x*x")]);
        l.set(0, Some("side"), "4").unwrap();
        assert_eq!(l.entries()[1].formula, "side*side");
        l.set(0, None, "5").unwrap();
        assert_eq!(l.entries()[1].formula, "[.B1]*[.B1]");
        assert_eq!(numbers(&l), ["5", "25"]);
    }

    #[test]
    fn an_edit_reports_only_the_values_that_changed() {
        let mut l = list(&[
            (Some("a"), "1"),
            (None, "a+1"),
            (None, "10"),
            (None, "[.B3]*2"),
        ]);
        assert_eq!(l.set(0, Some("a"), "5").unwrap(), [0, 1]);
        assert_eq!(l.set(2, None, "10").unwrap(), Vec::<usize>::new());
    }

    #[test]
    fn incremental_evaluation_agrees_with_evaluating_from_nothing() {
        // A small linear congruential generator: deterministic, and no dependency for it.
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            usize::try_from(seed >> 33).unwrap() % n.max(1)
        };
        let mut l = ExprList::default();
        for step in 0..300 {
            let len = l.entries().len();
            let formula = match next(4) {
                0 => format!("{}", next(100)),
                1 if len > 0 => format!("[.B{}]+{}", next(len) + 1, next(9)),
                2 if len > 1 => format!("[.B{}]*[.B{}]", next(len) + 1, next(len) + 1),
                _ => format!("{step}"),
            };
            match next(5) {
                0 if len > 0 => {
                    l.remove(next(len)).unwrap();
                }
                1 | 2 if len > 0 => {
                    l.set(next(len), None, &formula).unwrap();
                }
                _ => {
                    l.insert(next(len + 1), None, &formula).unwrap();
                }
            }
            let mut fresh = l.clone();
            fresh.evaluate(None);
            assert_eq!(numbers(&l), numbers(&fresh), "after step {step}");
        }
    }

    #[test]
    fn a_literal_is_found_by_value_and_by_which_one() {
        let expr = parse("=[.B1]*0.19+0.19").unwrap();
        let with = Expr::Name("vat".into());
        assert!(replace_literal(&expr, 0.19, None, &with).is_err());
        let (out, _) = replace_literal(&expr, 0.19, Some(2), &with).unwrap();
        assert_eq!(out.to_string(), "[.B1]*0.19+vat");
        let (out, lit) = replace_literal(&parse("=100*5%").unwrap(), 5.0, None, &with).unwrap();
        assert_eq!(out.to_string(), "100*vat");
        assert!(lit.percent);
        assert!(replace_literal(&expr, 7.0, None, &with).is_err());
    }
}
