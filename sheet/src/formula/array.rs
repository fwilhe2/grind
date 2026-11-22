// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `ForceArray` (§6.3.4): one parameter's expression evaluated in array mode, for a function such as
//! `SUMPRODUCT`.
//!
//! Not array formulas, which `doc/not-doing.md` keeps out: the function reduces the [`Matrix`] to
//! one value, so no cell holds an array. Within the argument, §3.3 2.2's rules apply: operators
//! work element by element, a function expecting one value is evaluated per element
//! ([`super::funcs::takes_one_value`]), and a range parameter takes the range whole. A single
//! value or a single row or column is repeated to fit, and a place an array does not reach is
//! `#N/A` (§3.3 2.2.2–2.2.3).

use super::eval::{Address, Engine, Operand, percent, prefix, scalar_binary};
use super::funcs;
use super::lex::Op;
use super::parse::Expr;
use super::value::{FormulaError, Value};

/// A rectangle of values, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    cells: Vec<Value>,
}

/// The most elements one combination may produce: `[.A:.A]*[.1:.1]` must be an error, not an
/// allocation the size of the grid.
const MAX_CELLS: usize = 1 << 24;

impl Matrix {
    fn single(value: Value) -> Self {
        Self {
            rows: 1,
            cols: 1,
            cells: vec![value],
        }
    }

    /// Every element, row by row.
    #[must_use]
    pub fn values(&self) -> &[Value] {
        &self.cells
    }

    /// The element at `row`, `col` of a result this matrix is one argument to (§3.3
    /// 2.2.3): a single row or column stands for every row or column, and a place beyond
    /// it is `#N/A`.
    fn get(&self, row: usize, col: usize) -> Value {
        let row = if self.rows == 1 { 0 } else { row };
        let col = if self.cols == 1 { 0 } else { col };
        if row < self.rows && col < self.cols {
            self.cells[row * self.cols + col].clone()
        } else {
            Value::Error(FormulaError::NA)
        }
    }

    /// One result per place of the largest shape among `matrices` (§3.3 2.2.2), each built
    /// by `f` from the elements every matrix has there.
    fn combine(matrices: &[&Matrix], mut f: impl FnMut(&[Value]) -> Value) -> Self {
        let rows = matrices.iter().map(|m| m.rows).max().unwrap_or(1);
        let cols = matrices.iter().map(|m| m.cols).max().unwrap_or(1);
        if rows.saturating_mul(cols) > MAX_CELLS {
            return Self::single(Value::Error(FormulaError::Value));
        }
        let mut cells = Vec::with_capacity(rows * cols);
        let mut here = Vec::with_capacity(matrices.len());
        for row in 0..rows {
            for col in 0..cols {
                here.clear();
                here.extend(matrices.iter().map(|m| m.get(row, col)));
                cells.push(f(&here));
            }
        }
        Self { rows, cols, cells }
    }
}

/// `expr` evaluated in array mode, as if it sat in cell `at`.
pub fn evaluate(engine: &mut Engine, expr: &Expr, at: Address) -> Matrix {
    match expr {
        Expr::Paren(inner) => evaluate(engine, inner, at),
        Expr::Prefix(op, inner) => {
            let inner = evaluate(engine, inner, at);
            Matrix::combine(&[&inner], |v| prefix(*op, v[0].clone()))
        }
        Expr::Postfix(_, inner) => {
            let inner = evaluate(engine, inner, at);
            Matrix::combine(&[&inner], |v| percent(v[0].clone()))
        }
        Expr::Binary(op, lhs, rhs) if !matches!(op, Op::Range | Op::Intersect | Op::Union) => {
            let (lhs, rhs) = (evaluate(engine, lhs, at), evaluate(engine, rhs, at));
            Matrix::combine(&[&lhs, &rhs], |v| {
                scalar_binary(*op, v[0].clone(), v[1].clone())
            })
        }
        Expr::Call { name, args } => call(engine, name, args, at),
        _ => match engine.operand(expr, at) {
            Operand::Value(value) => Matrix::single(value),
            Operand::Area(area) => {
                let (rows, cols) = (area.rows.len(), area.cols.len());
                let cells = engine.values_in(&area);
                Matrix { rows, cols, cells }
            }
        },
    }
}

/// A call inside an array: once, when every argument that takes one value got one, and
/// otherwise once per element, each array argument replaced by its element there (§3.3
/// 2.2.3).
fn call(engine: &mut Engine, name: &str, args: &[Expr], at: Address) -> Matrix {
    let upper = name.to_uppercase();
    let mut arrays: Vec<(usize, Matrix)> = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if funcs::takes_one_value(&upper, i) && !matches!(arg, Expr::Empty) {
            let matrix = evaluate(engine, arg, at);
            if matrix.rows * matrix.cols != 1 {
                arrays.push((i, matrix));
            }
        }
    }
    if arrays.is_empty() {
        return Matrix::single(funcs::call(engine, name, args, at));
    }
    let matrices: Vec<&Matrix> = arrays.iter().map(|(_, m)| m).collect();
    let mut substituted = args.to_vec();
    Matrix::combine(&matrices, |here| {
        for ((i, _), value) in arrays.iter().zip(here) {
            substituted[*i] = literal(value);
        }
        funcs::call(engine, name, &substituted, at)
    })
}

/// A value as the expression that evaluates to it, so an element can stand in for the
/// argument it came from. An empty cell becomes an empty parameter, which every function
/// here reads as the empty cell it was — `IF`'s branch is 0, a number is 0.
fn literal(value: &Value) -> Expr {
    match value {
        Value::Empty => Expr::Empty,
        Value::Number(n) => Expr::Number(*n),
        Value::Text(s) => Expr::Text(s.clone()),
        Value::Error(e) => Expr::Error(*e),
        Value::Bool(b) => Expr::Call {
            name: if *b { "TRUE" } else { "FALSE" }.into(),
            args: Vec::new(),
        },
    }
}

/// Whether `expr`, entered as an array formula over one cell, means what it means entered
/// plainly. Array mode iterates a range that stands where one value belongs — an operator's
/// operand, a one-value parameter's argument, the whole formula — and a plain formula meets it at
/// one cell; anywhere else a range is taken whole either way, and inside a `ForceArray` argument
/// both readings are array mode already. A name, a function the catalog does not know and an
/// `INDEX` with a zero row or column may be ranges, so they count as ranges.
#[must_use]
pub fn one_cell_array_is_plain(expr: &Expr) -> bool {
    !ranged(expr) && plain_inside(expr)
}

/// Whether nothing below `expr` puts a range where one value belongs.
fn plain_inside(expr: &Expr) -> bool {
    match expr {
        Expr::Binary(Op::Range | Op::Union | Op::Intersect, left, right) => {
            plain_inside(left) && plain_inside(right)
        }
        Expr::Binary(_, left, right) => {
            one_cell_array_is_plain(left) && one_cell_array_is_plain(right)
        }
        Expr::Prefix(_, inner) | Expr::Postfix(_, inner) => one_cell_array_is_plain(inner),
        Expr::Paren(inner) => plain_inside(inner),
        Expr::Call { name, args } => {
            let name = name.to_uppercase();
            let Some(info) = funcs::catalog().iter().find(|info| info.name == name) else {
                return !args.iter().any(ranged) && args.iter().all(plain_inside);
            };
            let (params, group) = funcs::parameters(info.signature);
            args.iter().enumerate().all(|(i, arg)| {
                let at = match i.checked_sub(params.len()) {
                    None => Some(i),
                    Some(_) if group == 0 => None,
                    Some(past) => Some(params.len() - group + past % group),
                };
                let force_array = at.is_some_and(|at| params[at].0 == "ForceArray");
                force_array
                    || if funcs::takes_one_value(&name, i) {
                        one_cell_array_is_plain(arg)
                    } else {
                        plain_inside(arg)
                    }
            })
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Error(_)
        | Expr::Ref(_)
        | Expr::Name(_)
        | Expr::Empty => true,
    }
}

/// Whether `expr` may stand for more than one cell.
fn ranged(expr: &Expr) -> bool {
    match expr {
        Expr::Ref(reference) => reference.end.is_some(),
        Expr::Name(_) | Expr::Binary(Op::Range | Op::Union | Op::Intersect, ..) => true,
        Expr::Paren(inner) => ranged(inner),
        Expr::Call { name, args } if name.eq_ignore_ascii_case("INDEX") => args
            .iter()
            .skip(1)
            .take(2)
            .any(|arg| matches!(arg, Expr::Empty | Expr::Number(0.0))),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::one_cell_array_is_plain;
    use crate::formula::parse::parse;

    fn plain(formula: &str) -> bool {
        one_cell_array_is_plain(&parse(formula).expect("parses"))
    }

    /// The shapes a spreadsheet service writes as one-cell array formulas, from a Monte Carlo
    /// workbook: every range is taken whole, or sits inside SUMPRODUCT's own array argument.
    #[test]
    fn a_range_taken_whole_reads_the_same_either_way() {
        assert!(plain("=INDEX([.$AL$18:.$AL$78];[.$B$4]+1)"));
        assert!(plain("=INDEX([.$CH$19:.$CH$78];[.$KK19])/12"));
        assert!(plain("=MAX([.$AX$19]:INDEX([.$AX$19:.$AX$78];[.$B$4]))"));
        assert!(plain(
            "=IF([.$C19]=0;0;IFERROR(MAX(0;[.$B$12]*EXP([.$A19])+SUMPRODUCT(([.$A$19:.$A$78]<=[.$A19])*[.$I$19:.$I$78]));0))"
        ));
        // MATCH's search region is ForceArray (§6.14.9): array mode in either reading.
        assert!(plain("=MATCH(TRUE();INDEX([.$Y$19:.$Y$78]>=0.1;0);0)"));
        assert!(plain("=1+2"));
        assert!(plain("=[.A1]*2"));
    }

    /// Where array mode iterates a range and a plain formula meets it at one cell, the two
    /// readings part.
    #[test]
    fn a_range_where_one_value_belongs_reads_differently() {
        // An operator's operand.
        assert!(!plain("=SUM([.A1:.A3]*2)"));
        assert!(!plain("=-[.A1:.A3]"));
        // A one-value parameter's argument, iterated in array mode.
        assert!(!plain("=SUM(LEN([.A1:.A3]))"));
        assert!(!plain("=COUNTIF([.A1:.A9];[.B1:.B3])"));
        // The whole formula: array mode takes its first cell.
        assert!(!plain("=[.A1:.A3]"));
        // INDEX's zero means a whole row or column.
        assert!(!plain("=INDEX([.A1:.B3];0;1)+1"));
        // A name may be a range, and a function this build does not know may take one value.
        assert!(!plain("=Prices*2"));
        assert!(!plain("=NOSUCH([.A1:.A3])"));
    }
}
