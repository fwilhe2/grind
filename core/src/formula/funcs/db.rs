// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §6.9 Database Functions — the Small Group's twelve, which are twelve aggregations over
//! one selection.
//!
//! §6.9.1 says the whole thing: each takes a Database (§4.11.9), a Field (§4.11.10) and a
//! Criteria (§4.11.11), and then "performs SUM/COUNT/MAX/… on the data records that match".
//! So there is one [`select`] here and a `match` on what to do with what it found.
//!
//! The three parameter types are each a small rule:
//!
//! * **Database** — a range whose *first row is field names*. Every row below it is a record.
//! * **Field** — Text naming a column (matched case-insensitively, §4.11.10's "should"), or
//!   a **1-based** column number. A selector naming no field is an Error, stated normatively.
//! * **Criteria** — a range of at least two rows. The first names fields; each row below it
//!   is a conjunction (every expression in the row must match), and the rows are a
//!   disjunction. That shape is why `select` is two nested loops and not a filter.
//!
//! An empty criterion cell constrains nothing. §4.11.11's "a reference to an empty cell is
//! interpreted as the numeric value 0" reads like the opposite, but it is about a criterion
//! that *is* a reference — reading it as "this field must be 0" would make every criteria
//! range select nothing outside its own filled corner, which is not what the shape is for.

use crate::model::Pos;

use super::super::eval::{Address, Area};
use super::super::value::{FormulaError, Value};
use super::Args;
use super::criterion::Criterion;

type Answer = Result<Value, FormulaError>;

/// Which aggregation §6.9 asks for — the only thing the twelve functions disagree about.
#[derive(Clone, Copy, PartialEq)]
enum Op {
    Average,
    Count,
    CountA,
    Get,
    Max,
    Min,
    Product,
    StDev,
    StDevP,
    Sum,
    Var,
    VarP,
}

pub fn call(name: &str, args: &mut Args) -> Option<Answer> {
    let op = match name {
        "DAVERAGE" => Op::Average,
        "DCOUNT" => Op::Count,
        "DCOUNTA" => Op::CountA,
        "DGET" => Op::Get,
        "DMAX" => Op::Max,
        "DMIN" => Op::Min,
        "DPRODUCT" => Op::Product,
        "DSTDEV" => Op::StDev,
        "DSTDEVP" => Op::StDevP,
        "DSUM" => Op::Sum,
        "DVAR" => Op::Var,
        "DVARP" => Op::VarP,
        _ => return None,
    };
    Some(database(args, op))
}

fn database(args: &mut Args, op: Op) -> Answer {
    // §6.9.3 and §6.9.4 are the two whose Field is optional — written `DCOUNT(D;;C)` or
    // left off entirely — because counting records needs no column to count in.
    let counts = matches!(op, Op::Count | Op::CountA);
    args.arity(if counts { 2..=3 } else { 3..=3 })?;
    let db = args.area(0).ok_or(FormulaError::Value)?;
    let criteria_at = if args.len() == 3 { 2 } else { 1 };
    let criteria = args.area(criteria_at).ok_or(FormulaError::Value)?;
    // §4.11.11: "at least one column and two rows" — the first row names the fields.
    if criteria.rows.len() < 2 || db.rows.is_empty() {
        return Err(FormulaError::Value);
    }
    let headers: Vec<Value> = (db.cols.clone())
        .map(|col| args.value_at(Address::new(db.sheet, Pos::new(db.rows.start, col))))
        .collect();

    let field = match (criteria_at == 2 && !args.omitted(1)).then(|| args.value(1)) {
        None => None,
        // Field 0 is not the first column — §4.11.10's numbering starts at 1, so zero selects
        // nothing, which for the two counting functions is the same as leaving it out.
        Some(Value::Number(n)) if n == 0.0 && counts => None,
        Some(selector) => Some(field(&selector, &headers)?),
    };

    let mut values = Vec::new();
    for row in db.rows.start + 1..db.rows.end {
        if !selects(args, &db, &headers, &criteria, row)? {
            continue;
        }
        values.push(match field {
            // A record with no field selected is still a record to count.
            None => Value::Empty,
            Some(col) => args.value_at(Address::new(db.sheet, Pos::new(row, db.cols.start + col))),
        });
    }

    // §6.13.6: COUNT and COUNTA report what they found rather than propagating an error in
    // it. Every other aggregation is a NumberSequence and does propagate.
    if counts {
        let count = match (op, field) {
            (_, None) => values.len(),
            (Op::CountA, _) => values.iter().filter(|v| **v != Value::Empty).count(),
            _ => values
                .iter()
                .filter(|v| matches!(v, Value::Number(_)))
                .count(),
        };
        return Ok(Value::number(count as f64));
    }
    if op == Op::Get {
        // §6.9.5: "If no records match, or more than one matches, it returns an Error."
        return match values.len() {
            1 => Ok(values.remove(0)),
            0 => Err(FormulaError::Value),
            _ => Err(FormulaError::Num),
        };
    }

    let mut numbers = Vec::new();
    for value in values {
        match value {
            Value::Number(n) => numbers.push(n),
            Value::Error(e) => return Err(e),
            // §6.3.7: text and empty cells inside a sequence are skipped, not zeroed.
            _ => {}
        }
    }
    aggregate(op, &numbers)
}

/// The same statistics §6.18 computes, over the records that matched.
fn aggregate(op: Op, numbers: &[f64]) -> Answer {
    let n = numbers.len() as f64;
    let mean = numbers.iter().sum::<f64>() / n;
    let sum_squares = || numbers.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
    Ok(Value::number(match op {
        Op::Sum => numbers.iter().sum(),
        Op::Product => numbers.iter().product(),
        // §6.18.45/§6.18.48: no numbers at all is 0, as for the undecorated MAX and MIN.
        Op::Max | Op::Min if numbers.is_empty() => 0.0,
        Op::Max => numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Op::Min => numbers.iter().copied().fold(f64::INFINITY, f64::min),
        Op::Average if numbers.is_empty() => return Err(FormulaError::DivZero),
        Op::Average => mean,
        Op::VarP | Op::StDevP if numbers.is_empty() => return Err(FormulaError::DivZero),
        Op::Var | Op::StDev if numbers.len() < 2 => return Err(FormulaError::DivZero),
        Op::VarP => sum_squares() / n,
        Op::StDevP => (sum_squares() / n).sqrt(),
        Op::Var => sum_squares() / (n - 1.0),
        Op::StDev => (sum_squares() / (n - 1.0)).sqrt(),
        Op::Count | Op::CountA | Op::Get => unreachable!("handled before the sequence"),
    }))
}

/// Does record `row` satisfy the criteria? Rows are ORed, cells within a row ANDed.
fn selects(
    args: &mut Args,
    db: &Area,
    headers: &[Value],
    criteria: &Area,
    row: u32,
) -> Result<bool, FormulaError> {
    let mut constrained = false;
    for crow in criteria.rows.start + 1..criteria.rows.end {
        let mut all = true;
        let mut constraints = 0usize;
        for ccol in criteria.cols.clone() {
            let test = args.value_at(Address::new(criteria.sheet, Pos::new(crow, ccol)));
            // A criterion cell holding nothing constrains nothing — and a formula that
            // returned `""` holds nothing too. The criteria range is a *shape*, usually as
            // wide as the database, so most of its cells are blank by design.
            if test == Value::Empty || test == Value::Text(String::new()) {
                continue;
            }
            let name = args.value_at(Address::new(
                criteria.sheet,
                Pos::new(criteria.rows.start, ccol),
            ));
            let col = field(&name, headers)?;
            let cell = args.value_at(Address::new(db.sheet, Pos::new(row, db.cols.start + col)));
            constraints += 1;
            if !Criterion::new(test).matches(&cell) {
                all = false;
                break;
            }
        }
        // A row of the criteria range with nothing in it is not "every record qualifies" — it
        // is a row of a rectangle drawn wide enough for the fields and left blank, which is
        // how these ranges are written. It contributes no disjunct at all.
        if constraints == 0 {
            continue;
        }
        constrained = true;
        if all {
            return Ok(true);
        }
    }
    // ... and a criteria range where *every* row is blank does constrain nothing, so every
    // record qualifies. That is the same rule, read at the other end.
    Ok(!constrained)
}

/// §4.11.10: a field selector, resolved to a 0-based offset within the database's columns.
fn field(selector: &Value, headers: &[Value]) -> Result<u32, FormulaError> {
    let position = match selector {
        Value::Text(name) => headers.iter().position(|header| match header {
            Value::Text(h) => h.eq_ignore_ascii_case(name),
            _ => false,
        }),
        Value::Number(n) => {
            let index = *n as i64;
            // `then` rather than `then_some`: field 0 is out of range, and the subtraction
            // that turns 1-based into 0-based must not run for it.
            (1..=headers.len() as i64)
                .contains(&index)
                .then(|| index as usize - 1)
        }
        _ => None,
    };
    // "shall … return an Error if the selected field does not exist" (§4.11.10).
    position.map(|p| p as u32).ok_or(FormulaError::Value)
}

#[cfg(test)]
mod tests {
    use super::super::super::eval::{Address, Engine};
    use super::super::super::value::{FormulaError, Value};
    use crate::model::{CellValue, Document, Pos, Sheet};

    /// A1:C4 is the database — `Name`, `Age`, `Fee` over three records — and E1:F2 is a
    /// criteria range the tests overwrite per case.
    fn document() -> Document {
        let mut sheet = Sheet::new("Sheet1");
        let rows: [(&str, f64, f64); 3] = [
            ("Ann", 30.0, 10.0),
            ("Bob", 40.0, 20.0),
            ("Cal", 50.0, 30.0),
        ];
        for (col, name) in ["Name", "Age", "Fee"].into_iter().enumerate() {
            sheet.set(Pos::new(0, col as u32), CellValue::Text(name.into()));
        }
        for (r, (name, age, fee)) in rows.into_iter().enumerate() {
            let row = r as u32 + 1;
            sheet.set(Pos::new(row, 0), CellValue::Text(name.into()));
            sheet.set(Pos::new(row, 1), CellValue::Number(age));
            sheet.set(Pos::new(row, 2), CellValue::Number(fee));
        }
        Document {
            sheets: vec![sheet],
            ..Default::default()
        }
    }

    /// `criteria` is written into E1:F2 before the formula runs.
    fn eval(criteria: &[(&str, &str)], formula: &str) -> Value {
        let mut document = document();
        let sheet = &mut document.sheets[0];
        for (i, (field, test)) in criteria.iter().enumerate() {
            let col = 4 + i as u32;
            sheet.set(Pos::new(0, col), CellValue::Text((*field).into()));
            // An empty criterion is a cell that was never written, not one holding "".
            if !test.is_empty() {
                sheet.set(Pos::new(1, col), CellValue::Text((*test).into()));
            }
        }
        Engine::new(&document).eval(formula, Address::new(0, Pos::new(20, 20)))
    }

    fn number(criteria: &[(&str, &str)], formula: &str) -> f64 {
        match eval(criteria, formula) {
            Value::Number(n) => n,
            other => panic!("{formula} evaluated to {other:?}"),
        }
    }

    #[test]
    fn a_criteria_row_is_a_conjunction_and_the_rows_are_a_disjunction() {
        let one = [("Age", ">35")];
        assert_eq!(number(&one, "=DSUM([.A1:.C4];\"Fee\";[.E1:.E2])"), 50.0);
        assert_eq!(number(&one, "=DCOUNT([.A1:.C4];\"Fee\";[.E1:.E2])"), 2.0);
        // Two criteria columns in the same row must both hold.
        let both = [("Age", ">35"), ("Fee", "<30")];
        assert_eq!(number(&both, "=DSUM([.A1:.C4];\"Fee\";[.E1:.F2])"), 20.0);
        // An empty criterion cell constrains nothing: F2 is blank, so only Age applies.
        let one_of_two = [("Age", ">35"), ("Fee", "")];
        assert_eq!(
            number(&one_of_two, "=DSUM([.A1:.C4];\"Fee\";[.E1:.F2])"),
            50.0
        );
    }

    #[test]
    fn the_field_may_be_a_name_a_number_or_omitted() {
        let all = [("Age", ">0")];
        assert_eq!(number(&all, "=DSUM([.A1:.C4];3;[.E1:.E2])"), 60.0);
        assert_eq!(number(&all, "=DSUM([.A1:.C4];\"fee\";[.E1:.E2])"), 60.0); // §4.11.10
        assert_eq!(number(&all, "=DCOUNT([.A1:.C4];;[.E1:.E2])"), 3.0); // records, not cells
        assert_eq!(number(&all, "=DCOUNTA([.A1:.C4];;[.E1:.E2])"), 3.0);
        // A selector naming no field is an Error, and so is one past the last column.
        assert_eq!(
            eval(&all, "=DSUM([.A1:.C4];\"Nope\";[.E1:.E2])"),
            Value::Error(FormulaError::Value)
        );
        assert_eq!(
            eval(&all, "=DSUM([.A1:.C4];4;[.E1:.E2])"),
            Value::Error(FormulaError::Value)
        );
    }

    #[test]
    fn the_aggregations_are_the_ones_section_6_9_names() {
        let all = [("Age", ">0")];
        assert_eq!(number(&all, "=DAVERAGE([.A1:.C4];\"Fee\";[.E1:.E2])"), 20.0);
        assert_eq!(number(&all, "=DMAX([.A1:.C4];\"Fee\";[.E1:.E2])"), 30.0);
        assert_eq!(number(&all, "=DMIN([.A1:.C4];\"Fee\";[.E1:.E2])"), 10.0);
        assert_eq!(
            number(&all, "=DPRODUCT([.A1:.C4];\"Fee\";[.E1:.E2])"),
            6000.0
        );
        assert_eq!(
            number(&all, "=DVARP([.A1:.C4];\"Fee\";[.E1:.E2])"),
            200.0 / 3.0
        );
        assert_eq!(number(&all, "=DVAR([.A1:.C4];\"Fee\";[.E1:.E2])"), 100.0);
        assert_eq!(number(&all, "=DSTDEV([.A1:.C4];\"Fee\";[.E1:.E2])"), 10.0);
        assert_eq!(
            number(&all, "=DSTDEVP([.A1:.C4];\"Fee\";[.E1:.E2])"),
            (200.0f64 / 3.0).sqrt()
        );
        // MAX and MIN over no matching record are 0, as §6.18.45 and §6.18.48 say.
        let none = [("Age", ">99")];
        assert_eq!(number(&none, "=DMAX([.A1:.C4];\"Fee\";[.E1:.E2])"), 0.0);
        assert_eq!(number(&none, "=DSUM([.A1:.C4];\"Fee\";[.E1:.E2])"), 0.0);
        assert_eq!(
            eval(&none, "=DAVERAGE([.A1:.C4];\"Fee\";[.E1:.E2])"),
            Value::Error(FormulaError::DivZero)
        );
    }

    #[test]
    fn dget_wants_exactly_one_record() {
        assert_eq!(
            eval(&[("Age", "40")], "=DGET([.A1:.C4];\"Name\";[.E1:.E2])"),
            Value::Text("Bob".into())
        );
        assert_eq!(
            eval(&[("Age", ">0")], "=DGET([.A1:.C4];\"Name\";[.E1:.E2])"),
            Value::Error(FormulaError::Num)
        );
        assert_eq!(
            eval(&[("Age", ">99")], "=DGET([.A1:.C4];\"Name\";[.E1:.E2])"),
            Value::Error(FormulaError::Value)
        );
    }
}
