// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Evaluation: an AST plus a document → a [`Value`] (ODF 1.4 Part 4 §6.4 operators,
//! §6.3 conversions).
//!
//! **Recursion is the dependency graph.** A formula that reads a cell holding another
//! formula evaluates that one first, and a memo keeps each cell to one evaluation — which
//! is a topological order arrived at by asking rather than by sorting. The plan budgets a
//! `graph.rs` with dirty propagation and petgraph; that pays for itself when *incremental*
//! recalculation exists, and nothing needs it yet. A cell already being visited is a
//! circular reference (§3.5 describes recalculation but leaves cycles to the host's
//! iterative-calculation settings, which do not exist yet) and yields `#NUM!`.
//!
//! ponytail: whole-document recalculation, no dirty set, no incremental update. Upgrade to
//! a real graph when an edit has to repaint a sheet rather than a test having to check one.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::model::{CellValue, Document, Pos};

use super::funcs;
use super::lex::{CellRef, Op, Reference};
use super::parse::{Expr, parse};
use super::value::{FormulaError, Value};

/// A cell, document-wide.
///
/// Ordered so that a map keyed by one iterates in a stable, readable order — sheet by sheet,
/// then row by row. [`crate::graph`] is what asked for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address {
    pub sheet: usize,
    pub pos: Pos,
}

impl Address {
    pub fn new(sheet: usize, pos: Pos) -> Self {
        Self { sheet, pos }
    }
}

/// A resolved rectangle of cells — what a reference becomes once the sheet name is looked
/// up and the open axis of a whole-row or whole-column reference is bounded.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    pub sheet: usize,
    pub rows: Range<u32>,
    pub cols: Range<u32>,
}

impl Area {
    pub fn cells(&self) -> impl Iterator<Item = Address> + '_ {
        let sheet = self.sheet;
        self.rows.clone().flat_map(move |row| {
            self.cols
                .clone()
                .map(move |col| Address::new(sheet, Pos::new(row, col)))
        })
    }

    fn single(&self) -> Option<Address> {
        (self.rows.len() == 1 && self.cols.len() == 1)
            .then(|| Address::new(self.sheet, Pos::new(self.rows.start, self.cols.start)))
    }
}

/// What an expression evaluates to before conversion. A reference stays a reference for as
/// long as possible: §6.3.7's sequence conversions can only tell text from a *referenced*
/// cell apart from text passed as an argument while it is still one.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Value(Value),
    Area(Area),
}

impl From<Value> for Operand {
    fn from(v: Value) -> Self {
        Operand::Value(v)
    }
}

impl From<FormulaError> for Operand {
    fn from(e: FormulaError) -> Self {
        Operand::Value(Value::Error(e))
    }
}

/// How deep a chain of formulas may be before we call it a runaway.
///
/// A real chain of dependent cells is thousands long at worst, and each frame here is
/// small; this only exists so that a pathological document cannot take the stack with it.
const MAX_DEPTH: usize = 4096;

pub struct Engine<'a> {
    doc: &'a Document,
    /// One evaluation per cell. Also the reason a diamond dependency is linear rather than
    /// exponential.
    cache: HashMap<Address, Value>,
    visiting: HashSet<Address>,
    depth: usize,
}

impl<'a> Engine<'a> {
    pub fn new(doc: &'a Document) -> Self {
        Self {
            doc,
            cache: HashMap::new(),
            visiting: HashSet::new(),
            depth: 0,
        }
    }

    /// The document's epoch (§3.4 `HOST-NULL-DATE`). A setting rather than a cell, so it is
    /// read through here instead of handing `funcs` the whole document.
    pub fn null_date(&self) -> i64 {
        self.doc.null_date
    }

    /// `HOST-NULL-YEAR` (§3.4 item 7), the two-digit-year break point.
    pub fn null_year(&self) -> i64 {
        self.doc.null_year
    }

    /// The value of a cell: its formula recalculated, or its stored value if it has none.
    pub fn value(&mut self, at: Address) -> Value {
        if let Some(cached) = self.cache.get(&at) {
            return cached.clone();
        }
        let Some(sheet) = self.doc.sheet(at.sheet) else {
            return Value::Error(FormulaError::Ref);
        };
        let Some(formula) = sheet.formula(at.pos) else {
            return from_cell(sheet.get(at.pos));
        };
        let formula = formula.to_owned();
        let value = self.eval(&formula, at);
        self.cache.insert(at, value.clone());
        value
    }

    /// Evaluate a formula as if it sat in cell `at` — which matters: implied intersection
    /// (§6.3.3) and every relative reference are relative to that cell.
    pub fn eval(&mut self, formula: &str, at: Address) -> Value {
        if !self.visiting.insert(at) || self.depth >= MAX_DEPTH {
            return Value::Error(FormulaError::Num);
        }
        self.depth += 1;
        let value = match parse(formula) {
            Ok(expr) => {
                let operand = self.operand(&expr, at);
                self.scalar(operand, at)
            }
            // §5.7 asks for "some Error value other than #N/A" for what an evaluator cannot
            // handle; a formula that is not OpenFormula is the same situation.
            Err(_) => Value::Error(FormulaError::Name),
        };
        self.depth -= 1;
        self.visiting.remove(&at);
        // §4.7 keeps Empty distinct from zero *as a cell*, but a formula's result is a stored
        // value and there is no empty value type to store it as — `office:value-type` has
        // none. §6.3.5 says what an empty cell is worth once something asks for its value, so
        // `=[.A1]` pointing at nothing is the number 0, and a cell reading *that* sees 0 in
        // turn rather than a second empty cell.
        if matches!(value, Value::Empty) {
            return Value::Number(0.0);
        }
        value
    }

    /// Evaluate an expression to an operand — a value, or a reference still in one piece.
    pub fn operand(&mut self, expr: &Expr, at: Address) -> Operand {
        match expr {
            Expr::Number(n) => Value::number(*n).into(),
            Expr::Text(s) => Value::Text(s.clone()).into(),
            Expr::Error(e) => Value::Error(*e).into(),
            Expr::Empty => Value::Empty.into(),
            Expr::Paren(inner) => self.operand(inner, at),
            Expr::Ref(reference) => match self.area(reference, at) {
                Some(area) => Operand::Area(area),
                None => FormulaError::Ref.into(),
            },
            Expr::Name(name) => self.named(name, at),
            Expr::Call { name, args } => funcs::call(self, name, args, at).into(),
            Expr::Prefix(op, operand) => self.prefix(*op, operand, at).into(),
            Expr::Postfix(_, operand) => {
                // The only postfix operator is `%` (§6.4.14).
                let value = self.value_of(operand, at);
                match value.to_number() {
                    Ok(n) => Value::number(n / 100.0).into(),
                    Err(e) => e.into(),
                }
            }
            Expr::Binary(op, lhs, rhs) => self.binary(*op, lhs, rhs, at),
        }
    }

    /// §5.11: a named expression stands for whatever it was defined as, evaluated *here* —
    /// relative references in it are relative to the cell that used the name.
    ///
    /// The depth counter is what stops `X` defined as `X+1`: names resolve inside one
    /// cell's evaluation, so the `visiting` set — which is keyed by cell — never sees the
    /// loop.
    fn named(&mut self, name: &str, at: Address) -> Operand {
        let Some(expression) = self.doc.name(name).map(str::to_owned) else {
            // §5.12: an unrecognised name is exactly what #NAME? reports.
            return FormulaError::Name.into();
        };
        if self.depth >= MAX_DEPTH {
            return FormulaError::Num.into();
        }
        let Ok(expr) = parse(&expression) else {
            return FormulaError::Name.into();
        };
        self.depth += 1;
        let operand = self.operand(&expr, at);
        self.depth -= 1;
        operand
    }

    fn prefix(&mut self, op: Op, operand: &Expr, at: Address) -> Value {
        let value = self.value_of(operand, at);
        match op {
            // §6.4.15 is explicit that prefix `+` converts nothing at all.
            Op::Add => value,
            _ => match value.to_number() {
                Ok(n) => Value::number(-n),
                Err(e) => Value::Error(e),
            },
        }
    }

    fn binary(&mut self, op: Op, lhs: &Expr, rhs: &Expr, at: Address) -> Operand {
        // The reference operators work on references, so they take their operands before
        // anything is converted to a scalar.
        match op {
            Op::Range => {
                let (Operand::Area(a), Operand::Area(b)) =
                    (self.operand(lhs, at), self.operand(rhs, at))
                else {
                    return FormulaError::Value.into();
                };
                if a.sheet != b.sheet {
                    // A cuboid across sheets (§4.8). Nothing in the Small Group evaluates
                    // one, and quietly using the first sheet would be a wrong answer.
                    return FormulaError::Ref.into();
                }
                // §6.4.11: the smallest rectangle containing both.
                Operand::Area(Area {
                    sheet: a.sheet,
                    rows: a.rows.start.min(b.rows.start)..a.rows.end.max(b.rows.end),
                    cols: a.cols.start.min(b.cols.start)..a.cols.end.max(b.cols.end),
                })
            }
            Op::Intersect => {
                let (Operand::Area(a), Operand::Area(b)) =
                    (self.operand(lhs, at), self.operand(rhs, at))
                else {
                    return FormulaError::Value.into();
                };
                match intersect(&a, &b) {
                    // §6.4.12: no cells in common is an Error, and §5.12 names it.
                    None => FormulaError::Null.into(),
                    Some(area) => Operand::Area(area),
                }
            }
            // §2.3.2 G leaves reference union out of the Small Group, and a ReferenceList
            // is the one value shape nothing else here can hold.
            Op::Union => FormulaError::Value.into(),
            _ => {
                let left = self.value_of(lhs, at);
                let right = self.value_of(rhs, at);
                scalar_binary(op, left, right).into()
            }
        }
    }

    /// Evaluate an expression and reduce it to a single value.
    pub fn value_of(&mut self, expr: &Expr, at: Address) -> Value {
        let operand = self.operand(expr, at);
        self.scalar(operand, at)
    }

    /// §6.3.2 Conversion to Scalar, including §6.3.3's implied intersection.
    pub fn scalar(&mut self, operand: Operand, at: Address) -> Value {
        match operand {
            Operand::Value(v) => v,
            Operand::Area(area) => {
                if let Some(cell) = area.single() {
                    return self.value(cell);
                }
                // Implied intersection: the row and the column the *formula* sits in.
                let rows = area.rows.contains(&at.pos.row);
                let cols = area.cols.contains(&at.pos.col);
                let cell = match (rows, cols, area.rows.len(), area.cols.len()) {
                    (true, _, _, 1) => Pos::new(at.pos.row, area.cols.start),
                    (_, true, 1, _) => Pos::new(area.rows.start, at.pos.col),
                    _ => return Value::Error(FormulaError::Value),
                };
                self.value(Address::new(area.sheet, cell))
            }
        }
    }

    /// §6.3.7 Conversion to NumberSequence over a whole area — the referenced-cells case,
    /// where text and empty cells are skipped rather than converted.
    pub fn numbers_in(&mut self, area: &Area, out: &mut Vec<f64>) -> Result<(), FormulaError> {
        for cell in area.cells().collect::<Vec<_>>() {
            match self.value(cell) {
                Value::Number(n) => out.push(n),
                Value::Error(e) => return Err(e),
                Value::Empty | Value::Text(_) | Value::Bool(_) => {}
            }
        }
        Ok(())
    }

    /// Every value in an area, in row-major order, for the functions that care about more
    /// than numbers (`COUNTA`, `ISBLANK`, the lookups).
    pub fn values_in(&mut self, area: &Area) -> Vec<Value> {
        area.cells()
            .collect::<Vec<_>>()
            .into_iter()
            .map(|cell| self.value(cell))
            .collect()
    }

    /// A reference (§5.8) resolved against this document. `None` when the sheet does not
    /// exist — the caller turns that into `#REF!`.
    ///
    /// Public because resolving a reference is a question separate from evaluating one, and
    /// [`crate::graph`] asks it without evaluating anything. That it is the *same* function
    /// the evaluator calls is the whole point: an index that resolved references its own way
    /// would disagree with what the document actually computes.
    pub fn area(&self, reference: &Reference, at: Address) -> Option<Area> {
        // An external document is not open, so nothing in it can be read (§5.8 Source).
        if reference.source.is_some() {
            return None;
        }
        let sheet = self.sheet_index(&reference.start, at)?;
        let end = reference.end.as_ref().unwrap_or(&reference.start);
        if self.sheet_index(end, at)? != sheet {
            return None; // a cuboid; see the range operator.
        }
        // The open axis of a whole-column or whole-row reference is bounded by what the
        // sheet actually uses. Cells past that are empty, and an empty cell contributes
        // nothing to any Small Group function — so this is the same answer as iterating to
        // the evaluator's limit, minus a million reads.
        let used = self.doc.sheet(sheet)?;
        let (used_rows, used_cols) = (used.used_rows(), used.used_cols());
        let axis = |a: Option<u32>, b: Option<u32>, used: u32| match (a, b) {
            (Some(a), Some(b)) => a.min(b)..a.max(b) + 1,
            _ => 0..used,
        };
        Some(Area {
            sheet,
            rows: axis(
                reference.start.row.map(|a| a.index),
                end.row.map(|a| a.index),
                used_rows,
            ),
            cols: axis(
                reference.start.col.map(|a| a.index),
                end.col.map(|a| a.index),
                used_cols,
            ),
        })
    }

    fn sheet_index(&self, cell: &CellRef, at: Address) -> Option<usize> {
        match &cell.sheet {
            // §5.8: no sheet locator means the sheet the formula is evaluated on.
            None => Some(at.sheet),
            // Sheet names are matched case-insensitively, as names are throughout §5.11.
            Some(name) => self
                .doc
                .sheets
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(name)),
        }
    }
}

pub(crate) fn intersect(a: &Area, b: &Area) -> Option<Area> {
    if a.sheet != b.sheet {
        return None;
    }
    let rows = a.rows.start.max(b.rows.start)..a.rows.end.min(b.rows.end);
    let cols = a.cols.start.max(b.cols.start)..a.cols.end.min(b.cols.end);
    (!rows.is_empty() && !cols.is_empty()).then_some(Area {
        sheet: a.sheet,
        rows,
        cols,
    })
}

/// A stored cell as a value. The reader keeps a formula's cached result here too, which is
/// what recalculation replaces.
pub fn from_cell(cell: CellValue) -> Value {
    match cell {
        CellValue::Empty => Value::Empty,
        CellValue::Number(n) => Value::Number(n),
        CellValue::Text(s) => Value::Text(s),
        CellValue::Bool(b) => Value::Bool(b),
    }
}

/// A computed value on its way back into a cell — the inverse of [`from_cell`], and what a
/// recalculation stores.
///
/// An error becomes its name as text because that is the only shape [`CellValue`] has for
/// one, and it is also what LibreOffice writes: an error cell carries an empty
/// `office:string-value` and the error name in `text:p` (doc/ods-format.md §6). Our reader
/// already takes that display text as the value, so this round-trips.
pub fn to_cell(value: Value) -> CellValue {
    match value {
        Value::Empty => CellValue::Empty,
        Value::Number(n) => CellValue::Number(n),
        Value::Text(s) => CellValue::Text(s),
        Value::Bool(b) => CellValue::Bool(b),
        Value::Error(e) => CellValue::Text(e.name().to_owned()),
    }
}

/// The arithmetic, comparison and concatenation operators (§6.4.2–§6.4.10).
fn scalar_binary(op: Op, left: Value, right: Value) -> Value {
    // §4.6: an operator given an error returns that error. Left first, arbitrarily but
    // consistently.
    if let Some(e) = left.error().or_else(|| right.error()) {
        return Value::Error(e);
    }
    match op {
        Op::Concat => match (left.to_text(), right.to_text()) {
            (Ok(a), Ok(b)) => Value::Text(a + &b),
            (Err(e), _) | (_, Err(e)) => Value::Error(e),
        },
        Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => compare(op, &left, &right),
        _ => {
            let (a, b) = match (left.to_number(), right.to_number()) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => return Value::Error(e),
            };
            match op {
                Op::Add => Value::number(a + b),
                Op::Sub => Value::number(a - b),
                Op::Mul => Value::number(a * b),
                // §6.4.5: "Dividing by zero returns an Error", and §5.12 names which.
                Op::Div if b == 0.0 => Value::Error(FormulaError::DivZero),
                Op::Div => Value::number(a / b),
                Op::Pow => Value::number(a.powf(b)),
                _ => unreachable!("{op:?} is not an arithmetic operator"),
            }
        }
    }
}

/// §6.4.7 `=`, §6.4.8 `<>` and §6.4.9 the ordered comparisons.
///
/// Two rules the spec leaves to the host, chosen here and named:
///
/// * **Case-insensitive text.** §6.4.7 defers to the `HOST-CASE-SENSITIVE` calculation
///   setting; LibreOffice's default is off, and matching the oracle beats matching an
///   opinion.
/// * **Ordering across types** is "implementation-defined" in §6.4.9. Numbers sort before
///   text, text before logicals — the order every implementation with an opinion uses.
///
/// Empty is not a type here: §4.7 keeps it distinct as a *value*, but a comparison converts
/// it to whatever the other side is, so `[.A1]=0` and `[.A1]=""` are both true of a blank
/// cell, as they are everywhere else.
fn compare(op: Op, left: &Value, right: &Value) -> Value {
    use std::cmp::Ordering;
    let Some(ordering) = order(left, right) else {
        // A NaN cannot reach a cell (`Value::number`), so this is unreachable in practice.
        return Value::Error(FormulaError::Num);
    };
    Value::Bool(match op {
        Op::Eq => ordering == Ordering::Equal,
        Op::Ne => ordering != Ordering::Equal,
        Op::Lt => ordering == Ordering::Less,
        Op::Le => ordering != Ordering::Greater,
        Op::Gt => ordering == Ordering::Greater,
        _ => ordering != Ordering::Less,
    })
}

/// How two values sort, by the rules `compare` documents. Shared with the lookup functions
/// (§6.14), which sort by exactly the same order — "Numbers before Text, Text before
/// Logicals" is §6.14.9's wording for what `rank` encodes.
pub fn order(left: &Value, right: &Value) -> Option<std::cmp::Ordering> {
    match (coerce_empty(left, right), coerce_empty(right, left)) {
        (Value::Number(a), Value::Number(b)) => a.partial_cmp(&b),
        (Value::Text(a), Value::Text(b)) => Some(compare_text(&a, &b)),
        (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(&b)),
        (a, b) => Some(rank(&a).cmp(&rank(&b))),
    }
}

/// An empty cell takes the type of what it is compared against.
fn coerce_empty(value: &Value, other: &Value) -> Value {
    match (value, other) {
        (Value::Empty, Value::Number(_)) => Value::Number(0.0),
        (Value::Empty, Value::Text(_)) => Value::Text(String::new()),
        (Value::Empty, Value::Bool(_)) => Value::Bool(false),
        (v, _) => v.clone(),
    }
}

fn compare_text(a: &str, b: &str) -> std::cmp::Ordering {
    // ponytail: code-point order after case folding, not locale collation. §6.4.9 allows a
    // host-defined collation; a real one needs ICU and a locale the document does not carry
    // yet. Wrong only for accented text ordering, never for equality of ASCII.
    a.to_lowercase().cmp(&b.to_lowercase())
}

/// Cross-type ordering, low to high.
fn rank(value: &Value) -> u8 {
    match value {
        Value::Empty => 0,
        Value::Number(_) => 1,
        Value::Text(_) => 2,
        Value::Bool(_) => 3,
        Value::Error(_) => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Sheet;

    /// A one-sheet document from a grid of literals, `None` for an empty cell.
    fn doc(cells: &[(u32, u32, CellValue)], formulas: &[(u32, u32, &str)]) -> Document {
        let mut sheet = Sheet::new("Sheet1");
        for (row, col, value) in cells {
            sheet.set(Pos::new(*row, *col), value.clone());
        }
        for (row, col, formula) in formulas {
            sheet.set_formula(Pos::new(*row, *col), (*formula).to_owned());
        }
        Document {
            sheets: vec![sheet],
            ..Default::default()
        }
    }

    fn eval(formula: &str) -> Value {
        let document = Document::default();
        Engine::new(&document).eval(formula, Address::new(0, Pos::new(0, 0)))
    }

    #[test]
    fn arithmetic_is_arithmetic() {
        assert_eq!(eval("=1+2*3"), Value::Number(7.0));
        assert_eq!(eval("=(1+2)*3"), Value::Number(9.0));
        assert_eq!(eval("=-2^2"), Value::Number(4.0)); // §5.5 Note 1
        assert_eq!(eval("=7-2-3"), Value::Number(2.0));
        assert_eq!(eval("=50%"), Value::Number(0.5));
        assert_eq!(eval("=\"a\"&\"b\""), Value::Text("ab".into()));
    }

    #[test]
    fn dividing_by_zero_is_an_error_not_an_infinity() {
        assert_eq!(eval("=1/0"), Value::Error(FormulaError::DivZero));
        assert_eq!(eval("=1E300*1E300"), Value::Error(FormulaError::Num));
    }

    #[test]
    fn an_error_propagates_through_every_operator() {
        // §4.6, the rule the whole error model rests on.
        assert_eq!(eval("=1+#N/A"), Value::Error(FormulaError::NA));
        assert_eq!(eval("=#DIV/0!&\"x\""), Value::Error(FormulaError::DivZero));
        assert_eq!(eval("=-#REF!"), Value::Error(FormulaError::Ref));
    }

    #[test]
    fn comparisons_follow_6_4_7() {
        assert_eq!(eval("=1=1"), Value::Bool(true));
        assert_eq!(eval("=1<>2"), Value::Bool(true));
        assert_eq!(eval("=\"a\"=\"A\""), Value::Bool(true)); // case-insensitive, like LO
        assert_eq!(eval("=1<\"a\""), Value::Bool(true)); // numbers sort before text
        assert_eq!(eval("=2>=2"), Value::Bool(true));
    }

    #[test]
    fn a_reference_reads_a_cell_and_a_blank_one_is_neither_zero_nor_empty_string() {
        let document = doc(&[(0, 0, CellValue::Number(6.0))], &[]);
        let mut engine = Engine::new(&document);
        let at = Address::new(0, Pos::new(5, 5));
        assert_eq!(engine.eval("=[.A1]*7", at), Value::Number(42.0));
        // §4.7: B9 holds nothing, and nothing converts to 0 and to "" alike.
        assert_eq!(engine.eval("=[.B9]+1", at), Value::Number(1.0));
        assert_eq!(engine.eval("=[.B9]=0", at), Value::Bool(true));
        assert_eq!(engine.eval("=[.B9]=\"\"", at), Value::Bool(true));
    }

    #[test]
    fn a_formula_cell_is_recalculated_rather_than_read() {
        // A1 = 2, A2 = A1*3 with a *stale* cached value of 999. Reading A2 must recompute.
        let mut document = doc(
            &[
                (0, 0, CellValue::Number(2.0)),
                (1, 0, CellValue::Number(999.0)),
            ],
            &[(1, 0, "of:=[.A1]*3")],
        );
        document.sheets[0].set_formula(Pos::new(2, 0), "of:=[.A2]+1".into());
        let mut engine = Engine::new(&document);
        assert_eq!(
            engine.value(Address::new(0, Pos::new(2, 0))),
            Value::Number(7.0)
        );
    }

    #[test]
    fn a_circular_reference_is_an_error_rather_than_a_hang() {
        let document = doc(&[], &[(0, 0, "of:=[.A2]"), (1, 0, "of:=[.A1]")]);
        let mut engine = Engine::new(&document);
        assert_eq!(
            engine.value(Address::new(0, Pos::new(0, 0))),
            Value::Error(FormulaError::Num)
        );
    }

    #[test]
    fn a_range_reduced_to_a_scalar_intersects_with_the_formulas_own_row() {
        // §6.3.3. The formula sits in row 1, so `[.A1:.A3]` resolves to A2.
        let document = doc(
            &[
                (0, 0, CellValue::Number(1.0)),
                (1, 0, CellValue::Number(2.0)),
                (2, 0, CellValue::Number(3.0)),
            ],
            &[],
        );
        let mut engine = Engine::new(&document);
        let at = Address::new(0, Pos::new(1, 4));
        assert_eq!(engine.eval("=[.A1:.A3]+0", at), Value::Number(2.0));
        // No intersection: an error, not a guess at the first cell.
        let at = Address::new(0, Pos::new(9, 9));
        assert_eq!(
            engine.eval("=[.A1:.A3]+0", at),
            Value::Error(FormulaError::Value)
        );
    }

    #[test]
    fn the_reference_operators_compute_areas() {
        let document = doc(&[], &[]);
        let engine = Engine::new(&document);
        let at = Address::new(0, Pos::new(0, 0));
        let area = |engine: &mut Engine, formula: &str| match engine
            .operand(&parse(formula).unwrap(), at)
        {
            Operand::Area(a) => Some((a.rows, a.cols)),
            Operand::Value(_) => None,
        };
        let mut engine = engine;
        // §6.4.11: the smallest rectangle containing both ends.
        assert_eq!(area(&mut engine, "=[.B4:.B5]:[.C5]"), Some((3..5, 1..3)));
        // §6.4.12: the overlap, or #NULL! when there is none.
        assert_eq!(
            area(&mut engine, "=[.A1:.C4]![.B1:.B5]"),
            Some((0..4, 1..2))
        );
        assert_eq!(area(&mut engine, "=[.A1]![.B2]"), None);
    }

    #[test]
    fn a_reference_to_a_sheet_that_does_not_exist_is_ref() {
        let document = doc(&[], &[]);
        let mut engine = Engine::new(&document);
        assert_eq!(
            engine.eval("=[Nope.A1]", Address::new(0, Pos::new(0, 0))),
            Value::Error(FormulaError::Ref)
        );
    }

    #[test]
    fn a_formula_that_is_not_openformula_is_name_rather_than_a_panic() {
        assert_eq!(eval("=NOT(0)NOT(0)"), Value::Error(FormulaError::Name));
    }
}
