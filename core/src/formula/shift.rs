// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Moving a formula to a different cell — a fill or a drag, the way every spreadsheet's
//! "extend this into the next cell" works.
//!
//! A relative axis (`A1`) moves by the offset; an absolute one (`$A$1`, or one half of
//! `$A1`/`A$1`) does not, which is the entire reason `$` exists (§5.8). A reference that
//! would move off the sheet becomes `#REF!` — the same error a delete produces, and the
//! same one this AST already has a place to hold ([`Expr::Error`]).

use super::lex::{Axis, CellRef, Reference};
use super::parse::Expr;
use super::value::FormulaError;

fn shift_axis(axis: Axis, delta: i64, max: u32) -> Option<Axis> {
    if axis.absolute {
        return Some(axis);
    }
    let shifted = i64::from(axis.index) + delta;
    (0..i64::from(max)).contains(&shifted).then_some(Axis {
        index: shifted as u32,
        absolute: false,
    })
}

/// `None` means the reference no longer fits on the sheet.
fn shift_cellref(cell: &CellRef, rows: i64, cols: i64) -> Option<CellRef> {
    let row = match cell.row {
        Some(axis) => Some(shift_axis(axis, rows, crate::MAX_ROWS)?),
        None => None,
    };
    let col = match cell.col {
        Some(axis) => Some(shift_axis(axis, cols, crate::MAX_COLS)?),
        None => None,
    };
    Some(CellRef {
        row,
        col,
        ..cell.clone()
    })
}

fn shift_reference(r: &Reference, rows: i64, cols: i64) -> Option<Reference> {
    let end = match &r.end {
        Some(e) => Some(shift_cellref(e, rows, cols)?),
        None => None,
    };
    Some(Reference {
        source: r.source.clone(),
        start: shift_cellref(&r.start, rows, cols)?,
        end,
    })
}

/// Every reference in `expr`, moved by `rows` rows and `cols` columns.
pub fn shift(expr: &Expr, rows: i64, cols: i64) -> Expr {
    match expr {
        Expr::Ref(r) => match shift_reference(r, rows, cols) {
            Some(r) => Expr::Ref(r),
            None => Expr::Error(FormulaError::Ref),
        },
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(|a| shift(a, rows, cols)).collect(),
        },
        Expr::Prefix(op, e) => Expr::Prefix(*op, Box::new(shift(e, rows, cols))),
        Expr::Postfix(op, e) => Expr::Postfix(*op, Box::new(shift(e, rows, cols))),
        Expr::Binary(op, l, r) => Expr::Binary(
            *op,
            Box::new(shift(l, rows, cols)),
            Box::new(shift(r, rows, cols)),
        ),
        Expr::Paren(e) => Expr::Paren(Box::new(shift(e, rows, cols))),
        Expr::Number(_) | Expr::Text(_) | Expr::Error(_) | Expr::Name(_) | Expr::Empty => {
            expr.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::parse::parse;

    fn shifted(formula: &str, rows: i64, cols: i64) -> String {
        shift(&parse(formula).unwrap(), rows, cols).to_string()
    }

    #[test]
    fn a_relative_reference_moves_with_the_fill() {
        assert_eq!(shifted("=[.A1]", 1, 0), "[.A2]");
        assert_eq!(shifted("=[.A1]", 0, 1), "[.B1]");
        assert_eq!(shifted("=SUM([.A1:.A2])", 2, 0), "SUM([.A3:.A4])");
    }

    #[test]
    fn an_absolute_axis_does_not_move_but_its_relative_twin_does() {
        assert_eq!(shifted("=[.$A1]", 1, 1), "[.$A2]");
        assert_eq!(shifted("=[.A$1]", 1, 1), "[.B$1]");
        assert_eq!(shifted("=[.$A$1]", 5, 5), "[.$A$1]");
    }

    #[test]
    fn moving_off_the_sheet_becomes_ref_error() {
        assert_eq!(shifted("=[.A1]", -1, 0), "#REF!");
        assert_eq!(shifted("=[.A1]+1", 0, -1), "#REF!+1");
    }
}
