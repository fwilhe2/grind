// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Conditional formatting's one rule type — `doc/conditional-format.md`, normative.
//!
//! *This formula is true here, so draw the cell this way.* A [`Rule`] is a condition in
//! OpenFormula's own syntax, a set of ranges it applies to, the cell its relative references
//! are written from, and the [`CellStyle`] it draws **over** a cell's own while it holds. A
//! sheet keeps an ordered list of them, and the first that holds at a cell is the one drawn
//! (`doc/ods-format.md` §3.6, Q4, measured).
//!
//! **Nothing here is ever stored about a cell.** Whether a rule holds is derived as the sheet
//! is drawn ([`applied`]), the way `doc/view-modes.md` derives a role, so the answer cannot go
//! stale and no shell evaluates a formula of its own.
//!
//! The file's spelling is ODF's standard one: a `style:map` with `is-true-formula(…)` and a
//! `style:base-cell-address` on every cell style of every cell in the range, applying a named
//! style from `office:styles` (`doc/ods-format.md` §3.6). LibreOffice's `calcext:` copy is
//! never needed to read a rule, and never written.

use serde::{Deserialize, Serialize};

use crate::formula::eval::{Address, Engine};
use crate::formula::parse::{Expr, parse};
use crate::formula::shift::shift;
use crate::model::{Document, Pos};
use crate::style::CellStyle;

/// One rule: while `condition` holds at a cell of `ranges`, `style` is drawn over the cell's
/// own.
///
/// **Several ranges, one base**, which is the shape both formats give a rule: Excel's `sqref`
/// is a list of ranges with one formula, and ODF's `style:base-cell-address` is one cell
/// however many ranges carry the map. A rule over ten ranges is one rule, as it was to whoever
/// wrote it, and `condition` is written from `base` and moved to each cell from there — the
/// way a fill moves a formula ([`shift`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The cells it applies to, each range top-left and bottom-right inclusive.
    pub ranges: Vec<(Pos, Pos)>,
    /// The cell `condition` is written from — its relative references are relative to this.
    pub base: Pos,
    /// OpenFormula in ODF syntax, with no `of:=` intro: `[.$E5]=TRUE()`.
    pub condition: String,
    /// What it draws. Only what this sets is drawn over the cell ([`CellStyle::over`]).
    pub style: CellStyle,
}

impl Rule {
    /// A rule over one range, written from its top-left cell — what a person means by "this
    /// range, this condition".
    pub fn over(start: Pos, end: Pos, condition: impl Into<String>, style: CellStyle) -> Self {
        Rule {
            ranges: vec![(start, end)],
            base: start,
            condition: condition.into(),
            style,
        }
    }

    /// Whether `pos` is in one of its ranges.
    pub fn covers(&self, pos: Pos) -> bool {
        self.ranges
            .iter()
            .any(|(start, end)| within(pos, *start, *end))
    }

    /// The condition as it reads at `pos`: parsed, and every relative reference moved by the
    /// distance from [`Rule::base`]. `None` when it does not parse — such a rule never holds.
    pub fn condition_at(&self, pos: Pos) -> Option<Expr> {
        let expr = parse(&self.condition).ok()?;
        Some(self.moved(&expr, pos))
    }

    fn moved(&self, expr: &Expr, pos: Pos) -> Expr {
        let rows = i64::from(pos.row) - i64::from(self.base.row);
        let cols = i64::from(pos.col) - i64::from(self.base.col);
        match (rows, cols) {
            (0, 0) => expr.clone(),
            _ => shift(expr, rows, cols),
        }
    }

    /// Whether the condition holds at `pos` of `sheet`, asked of `engine`. A value that is not
    /// a logical is converted the way §6.3.6 converts one, and an error — or a condition that
    /// does not parse — is false: a rule that cannot be evaluated draws nothing.
    pub fn holds(&self, engine: &mut Engine, sheet: usize, pos: Pos) -> bool {
        let Some(expr) = self.condition_at(pos) else {
            return false;
        };
        let at = Address::new(sheet, pos);
        engine.value_of(&expr, at).to_logical().unwrap_or(false)
    }
}

fn within(pos: Pos, start: Pos, end: Pos) -> bool {
    (start.row..=end.row).contains(&pos.row) && (start.col..=end.col).contains(&pos.col)
}

/// The style the first rule holding at `pos` draws, if any rule covering it holds.
///
/// `engine` is the caller's so that a screenful of cells shares one cache; it should be
/// [`Engine::over_cached`], since a rule is about what the sheet shows.
pub fn applied<'d>(
    doc: &'d Document,
    engine: &mut Engine,
    sheet: usize,
    pos: Pos,
) -> Option<&'d CellStyle> {
    doc.sheet(sheet)?
        .rules()
        .iter()
        .filter(|rule| rule.covers(pos))
        .find(|rule| rule.holds(engine, sheet, pos))
        .map(|rule| &rule.style)
}

/// A rule's condition as ODF spells it on a `style:map` — `is-true-formula(…)`, the one
/// spelling this build writes (`doc/ods-format.md` §3.6).
pub fn odf_condition(condition: &str) -> String {
    format!("is-true-formula({})", strip_intro(condition))
}

fn strip_intro(condition: &str) -> &str {
    let c = condition.trim();
    let c = c.strip_prefix("of:").unwrap_or(c);
    c.strip_prefix('=').unwrap_or(c)
}

/// A `style:condition` read off a cell style's `style:map` → the model's condition, written
/// from `base`.
///
/// `is-true-formula(…)` is the rule itself. The value comparisons — `cell-content()>5`,
/// `cell-content-is-between(1,5)` — are each a formula about the cell, so they are read *as*
/// one, with the cell spelled as a relative reference to `base` (`doc/conditional-format.md`
/// §1: the model has no second vocabulary for what the first can say). Anything else —
/// `cell-content-is-whole-number()`, a condition this build cannot name — is `None`, and the
/// file's own map is kept where a save does not touch it.
pub fn from_odf_condition(condition: &str, base: Pos) -> Option<String> {
    let condition = condition.trim();
    if let Some(inner) = call(condition, "is-true-formula") {
        let inner = strip_intro(inner);
        parse(inner).ok()?;
        return Some(inner.to_owned());
    }
    let me = format!("[.{}]", crate::a1::format(None, base));
    for (name, negated) in [
        ("cell-content-is-between", false),
        ("cell-content-is-not-between", true),
    ] {
        if let Some(inner) = call(condition, name) {
            let (low, high) = split_pair(inner)?;
            parse(low).ok()?;
            parse(high).ok()?;
            return Some(match negated {
                false => format!("AND({me}>={low};{me}<={high})"),
                true => format!("OR({me}<{low};{me}>{high})"),
            });
        }
    }
    let rest = condition.strip_prefix("cell-content()")?.trim_start();
    // Longest operator first, so `<=` is not read as `<` and `=…`.
    let op = ["<=", ">=", "!=", "<", ">", "="]
        .into_iter()
        .find(|op| rest.starts_with(op))?;
    let value = rest[op.len()..].trim();
    parse(value).ok()?;
    let op = if op == "!=" { "<>" } else { op };
    Some(format!("{me}{op}{value}"))
}

/// An order of `n` rules honouring every `(a, b)` in `before` — `a` is tried before `b` — and
/// otherwise the order they are numbered in. What a file states is the order of the maps on each
/// cell style, a partial order, and this is the one total order a reader can derive from it. A
/// cycle (two styles disagreeing) is broken by numbering, which is all a reader can do.
pub fn priority(n: usize, before: &[(usize, usize)]) -> Vec<usize> {
    let mut pending: Vec<usize> = vec![0; n];
    for (_, b) in before {
        pending[*b] += 1;
    }
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let next = (0..n)
            .find(|i| !done[*i] && pending[*i] == 0)
            .or_else(|| (0..n).find(|i| !done[*i]))
            .expect("fewer than n placed");
        done[next] = true;
        order.push(next);
        for (a, b) in before {
            if *a == next && pending[*b] > 0 {
                pending[*b] -= 1;
            }
        }
    }
    order
}

/// Rectangles of cells — in the order they were met — joined where they can be: side by side
/// on the same rows, then one above the other on the same columns. What a file spells as one
/// run of cells per row comes back as the range somebody chose.
pub fn rectangles(mut rects: Vec<(Pos, Pos)>) -> Vec<(Pos, Pos)> {
    let join = |rects: &mut Vec<(Pos, Pos)>, beside: fn(&(Pos, Pos), &(Pos, Pos)) -> bool| {
        let mut out: Vec<(Pos, Pos)> = Vec::with_capacity(rects.len());
        for rect in rects.drain(..) {
            match out.iter_mut().find(|r| beside(r, &rect)) {
                Some(r) => {
                    r.1 = Pos::new(r.1.row.max(rect.1.row), r.1.col.max(rect.1.col));
                    r.0 = Pos::new(r.0.row.min(rect.0.row), r.0.col.min(rect.0.col));
                }
                None => out.push(rect),
            }
        }
        *rects = out;
    };
    // Same rows, touching columns.
    join(&mut rects, |a, b| {
        a.0.row == b.0.row
            && a.1.row == b.1.row
            && (b.0.col <= a.1.col.saturating_add(1) && a.0.col <= b.1.col.saturating_add(1))
    });
    // Same columns, touching rows — repeated until nothing more joins, since a join can make a
    // rectangle that now touches one already passed.
    loop {
        let before = rects.len();
        join(&mut rects, |a, b| {
            a.0.col == b.0.col
                && a.1.col == b.1.col
                && (b.0.row <= a.1.row.saturating_add(1) && a.0.row <= b.1.row.saturating_add(1))
        });
        if rects.len() == before {
            break;
        }
    }
    rects
}

/// The argument text of `name(…)`, when `text` is exactly one call of it.
fn call<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    text.strip_prefix(name)?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')
}

/// `a,b` split at its one top-level comma — outside quotes and brackets.
fn split_pair(text: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '(' | '[' | '{' if !quoted => depth += 1,
            ')' | ']' | '}' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                return Some((text[..i].trim(), text[i + 1..].trim()));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CellValue, Sheet};

    fn green() -> CellStyle {
        CellStyle {
            background: Some("#97e8ca".into()),
            ..CellStyle::default()
        }
    }

    #[test]
    fn a_relative_reference_moves_from_the_base_and_an_absolute_one_does_not() {
        let rule = Rule::over(Pos::new(4, 1), Pos::new(6, 3), "[.$E5]=TRUE()", green());
        let at = rule.condition_at(Pos::new(6, 3)).unwrap();
        assert_eq!(at.to_string(), "[.$E7]=TRUE()");
        assert!(rule.covers(Pos::new(5, 2)));
        assert!(!rule.covers(Pos::new(7, 2)));
    }

    #[test]
    fn the_first_rule_that_holds_is_the_one_drawn() {
        let mut doc = Document::default();
        let mut sheet = Sheet::new("Sheet1");
        sheet.set(Pos::new(0, 0), CellValue::Number(5.0));
        let red = CellStyle {
            background: Some("#ff0000".into()),
            ..CellStyle::default()
        };
        sheet.set_rules(vec![
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "[.A1]>0", red.clone()),
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "[.A1]>1", green()),
        ]);
        doc.sheets = vec![sheet];
        let mut engine = Engine::over_cached(&doc);
        assert_eq!(applied(&doc, &mut engine, 0, Pos::new(0, 0)), Some(&red));
        assert_eq!(applied(&doc, &mut engine, 0, Pos::new(1, 0)), None);
    }

    #[test]
    fn an_error_or_a_condition_that_does_not_parse_draws_nothing() {
        let mut doc = Document::default();
        let mut sheet = Sheet::new("Sheet1");
        sheet.set_rules(vec![
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "1/0", green()),
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "((", green()),
        ]);
        doc.sheets = vec![sheet];
        let mut engine = Engine::over_cached(&doc);
        assert_eq!(applied(&doc, &mut engine, 0, Pos::new(0, 0)), None);
    }

    #[test]
    fn the_value_comparisons_are_read_as_formulas_about_the_cell() {
        let b2 = Pos::new(1, 1);
        assert_eq!(
            from_odf_condition("is-true-formula([.$A2]>0)", b2).as_deref(),
            Some("[.$A2]>0")
        );
        assert_eq!(
            from_odf_condition("cell-content()>=5", b2).as_deref(),
            Some("[.B2]>=5")
        );
        assert_eq!(
            from_odf_condition("cell-content()!=\"x\"", b2).as_deref(),
            Some("[.B2]<>\"x\"")
        );
        assert_eq!(
            from_odf_condition("cell-content-is-between(1,[.$C$1])", b2).as_deref(),
            Some("AND([.B2]>=1;[.B2]<=[.$C$1])")
        );
        assert_eq!(
            from_odf_condition("cell-content-is-not-between(1,5)", b2).as_deref(),
            Some("OR([.B2]<1;[.B2]>5)")
        );
        assert_eq!(
            from_odf_condition("cell-content-is-whole-number()", b2),
            None
        );
        assert_eq!(odf_condition("of:=[.A1]>0"), "is-true-formula([.A1]>0)");
    }
}
