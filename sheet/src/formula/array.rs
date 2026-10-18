// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ForceArray (§6.3.4): one parameter's expression evaluated in array mode, for a function
//! such as `SUMPRODUCT` that declares one.
//!
//! **This is not array formulas.** `doc/not-doing.md` keeps those out — a formula whose
//! *result* is a matrix spilling over a range of cells, `table:number-matrix-*-spanned`, a
//! second evaluation mode for the whole sheet. What is here stays inside one argument and
//! ends there: the function reduces the [`Matrix`] to one value before anything else sees
//! it, so no cell ever holds an array and nothing outside the function changes.
//!
//! Within that argument, §3.3 2.2's rules: an operator (other than the reference operators)
//! is applied element by element, a function given an array where it expects one value is
//! evaluated once per element ([`super::funcs::takes_one_value`]), and a function whose
//! parameter is a range — `SUM`'s, `COUNTIF`'s — takes it whole. Shapes combine by §3.3
//! 2.2.2–2.2.3: a single value is repeated, a single row or column is repeated across the
//! other axis, and a place one array does not reach is `#N/A`.

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

/// The most elements one combination may produce. Two references are bounded by the sheet,
/// but a column times a row is their product, and `[.A:.A]*[.1:.1]` must be an error rather
/// than an allocation the size of the grid.
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
