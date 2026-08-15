// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The function library — ODF 1.4 Part 4 §6, one module per category, in the order
//! `doc/small-group.md` lists them.
//!
//! Functions receive their arguments **unevaluated**. `IF` is the reason: §6.15.4 wants the
//! branch not taken left alone, so `IF([.A1]=0;0;1/[.A1])` cannot divide by zero. Anything
//! that just wants a number asks [`Args`] for one and never sees the difference.
//!
//! Conversions are not each function's business either: [`Args::number`] is §6.3.5,
//! [`Args::numbers`] is §6.3.7, and the asymmetry between them — a referenced `"7"` is
//! skipped, an argument `"7"` converts — is the specified behaviour rather than an
//! inconsistency to iron out.
//!
//! Still unimplemented, and each for a reason rather than an oversight: the `*IF` family
//! and the lookups need §4.11.8 Criterion matching, date and time need `table:null-date`
//! (deferred until phase 4 has an epoch to be right about), and the database and financial
//! groups are the two least-used corners of the Small Group. Loop B's scoreboard sets the
//! order.

use super::eval::{Address, Area, Engine, Operand};
use super::parse::Expr;
use super::value::{FormulaError, Value};

mod criterion;
mod info;
mod logical;
mod lookup;
mod math;
mod stat;
mod text;

/// Dispatch. §5.6: function names are case-insensitive.
pub fn call(engine: &mut Engine, name: &str, args: &[Expr], at: Address) -> Value {
    let name = name.to_uppercase();
    let mut args = Args { engine, args, at };
    for category in [
        logical::call,
        math::call,
        info::call,
        stat::call,
        text::call,
        lookup::call,
    ] {
        if let Some(answer) = category(&name, &mut args) {
            return answer.unwrap_or_else(Value::Error);
        }
    }
    // §5.7: an evaluator that does not support a function computes "some Error value other
    // than #N/A", and an unrecognised name is exactly what #NAME? reports.
    Value::Error(FormulaError::Name)
}

/// Every function name the evaluator answers to — the scoreboard's denominator, and what
/// the CLI will list.
pub fn implemented() -> Vec<&'static str> {
    NAMES.to_vec()
}

const NAMES: &[&str] = &[
    // logical (§6.15)
    "AND",
    "FALSE",
    "IF",
    "NOT",
    "OR",
    "TRUE",
    // mathematical (§6.16) and rounding (§6.17)
    "ABS",
    "ACOS",
    "ASIN",
    "ATAN",
    "ATAN2",
    "COS",
    "DEGREES",
    "EVEN",
    "EXP",
    "FACT",
    "INT",
    "LN",
    "LOG",
    "LOG10",
    "MOD",
    "ODD",
    "PI",
    "POWER",
    "PRODUCT",
    "RADIANS",
    "ROUND",
    "SIN",
    "SQRT",
    "SUM",
    "SUMIF",
    "TAN",
    "TRUNC",
    // information (§6.13)
    "COLUMNS",
    "COUNT",
    "COUNTA",
    "COUNTBLANK",
    "COUNTIF",
    "ISBLANK",
    "ISERR",
    "ISERROR",
    "ISLOGICAL",
    "ISNA",
    "ISNONTEXT",
    "ISNUMBER",
    "ISTEXT",
    "N",
    "NA",
    "ROWS",
    "VALUE",
    // statistical (§6.18)
    "AVERAGE",
    "AVERAGEIF",
    "MAX",
    "MIN",
    "STDEV",
    "STDEVP",
    "VAR",
    "VARP",
    // text (§6.20)
    "EXACT",
    "FIND",
    "LEFT",
    "LEN",
    "LOWER",
    "MID",
    "PROPER",
    "REPLACE",
    "REPT",
    "RIGHT",
    "SUBSTITUTE",
    "T",
    "TRIM",
    "UPPER",
    // lookup (§6.14)
    "CHOOSE",
    "HLOOKUP",
    "INDEX",
    "MATCH",
    "VLOOKUP",
];

/// A call's arguments, plus the engine needed to evaluate them.
pub struct Args<'a, 'd> {
    engine: &'a mut Engine<'d>,
    args: &'a [Expr],
    at: Address,
}

impl Args<'_, '_> {
    pub fn len(&self) -> usize {
        self.args.len()
    }

    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    /// Reject a call whose argument count the function does not accept.
    ///
    /// `#VALUE!` rather than a parse failure: the formula is well formed, its parameters
    /// are not (§5.12, "parameter is wrong type").
    pub fn arity(&self, accepted: impl std::ops::RangeBounds<usize>) -> Result<(), FormulaError> {
        if accepted.contains(&self.args.len()) {
            Ok(())
        } else {
            Err(FormulaError::Value)
        }
    }

    /// The `i`th argument, unreduced. A missing one is [`Value::Empty`], which is what an
    /// omitted optional parameter means.
    pub fn operand(&mut self, i: usize) -> Operand {
        match self.args.get(i) {
            Some(expr) => self.engine.operand(expr, self.at),
            None => Operand::Value(Value::Empty),
        }
    }

    /// §6.3.2 Conversion to Scalar.
    pub fn value(&mut self, i: usize) -> Value {
        let operand = self.operand(i);
        self.engine.scalar(operand, self.at)
    }

    /// §6.3.5 Conversion to Number.
    pub fn number(&mut self, i: usize) -> Result<f64, FormulaError> {
        self.value(i).to_number()
    }

    /// §6.3.6 Conversion to Integer, truncating toward zero.
    ///
    /// §6.3.6 leaves the rounding to each function and most of the Small Group's
    /// integer parameters — `LEFT`'s length, `ROUND`'s digits, `CHOOSE`'s index — are
    /// counts, where truncation is the only reading that cannot overshoot.
    pub fn integer(&mut self, i: usize) -> Result<i64, FormulaError> {
        Ok(self.number(i)?.trunc() as i64)
    }

    /// §6.3.14 Conversion to Text.
    pub fn text(&mut self, i: usize) -> Result<String, FormulaError> {
        self.value(i).to_text()
    }

    /// §6.3.12 Conversion to Logical.
    pub fn logical(&mut self, i: usize) -> Result<bool, FormulaError> {
        self.value(i).to_logical()
    }

    /// §6.3.8 Conversion to NumberSequenceList: every argument flattened into one list of
    /// numbers, references filtered and scalars converted.
    pub fn numbers(&mut self) -> Result<Vec<f64>, FormulaError> {
        let mut out = Vec::new();
        for i in 0..self.args.len() {
            match self.operand(i) {
                Operand::Area(area) => self.engine.numbers_in(&area, &mut out)?,
                // An omitted argument contributes nothing rather than a zero.
                Operand::Value(Value::Empty) => {}
                Operand::Value(v) => out.push(v.to_number()?),
            }
        }
        Ok(out)
    }

    /// Every argument flattened into values, referenced cells included — for the functions
    /// that count or inspect rather than compute (`COUNTA`, `MAX` on text, the lookups).
    pub fn values(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        for i in 0..self.args.len() {
            match self.operand(i) {
                Operand::Area(area) => out.extend(self.engine.values_in(&area)),
                Operand::Value(v) => out.push(v),
            }
        }
        out
    }

    /// The `i`th argument as an area, for the functions that ask about a reference's shape
    /// rather than its contents (`ROWS`, `COLUMNS`, `COUNTBLANK`).
    pub fn area(&mut self, i: usize) -> Option<Area> {
        match self.operand(i) {
            Operand::Area(area) => Some(area),
            Operand::Value(_) => None,
        }
    }

    /// Was this parameter written but left empty — the `;;` of §5.6?
    ///
    /// Several functions distinguish it from an absent trailing parameter: §6.15.4's seven
    /// shapes of `IF` turn on exactly this.
    pub fn omitted(&self, i: usize) -> bool {
        matches!(self.args.get(i), None | Some(Expr::Empty))
    }

    /// §6.3.13 Conversion to LogicalSequence, over every argument.
    ///
    /// Numbers *are* included: §6.3.13 excludes them only where Logical is a distinguished
    /// type, and while ours is, LibreOffice's is not — `AND([.A1:.A3])` over a column of 1s
    /// and 0s is TRUE there, and the oracle wins over the taxonomy.
    pub fn logicals(&mut self) -> Result<Vec<bool>, FormulaError> {
        let mut out = Vec::new();
        for i in 0..self.args.len() {
            match self.operand(i) {
                Operand::Area(area) => {
                    for value in self.engine.values_in(&area) {
                        match value {
                            Value::Bool(b) => out.push(b),
                            Value::Number(n) => out.push(n != 0.0),
                            Value::Error(e) => return Err(e),
                            // Text and empty cells contribute nothing, as in a sequence.
                            Value::Empty | Value::Text(_) => {}
                        }
                    }
                }
                Operand::Value(Value::Empty) => {}
                Operand::Value(v) => out.push(v.to_logical()?),
            }
        }
        Ok(out)
    }

    /// Every value in an area — for the few functions that resolve a reference themselves.
    pub fn values_in(&mut self, area: &Area) -> Vec<Value> {
        self.engine.values_in(area)
    }

    /// §6.3.2 Conversion to Scalar, applied to an operand a function already holds.
    pub fn scalar(&mut self, operand: Operand) -> Value {
        self.engine.scalar(operand, self.at)
    }

    /// One cell, by address — for the functions that compute *which* cell they want
    /// (§6.14's lookups, §6.16.62's parallel sum range) rather than reading a whole area.
    pub fn value_at(&mut self, address: Address) -> Value {
        self.engine.value(address)
    }
}

#[cfg(test)]
mod tests {
    use super::super::eval::Engine;
    use super::super::value::{FormulaError, Value};
    use crate::model::{CellValue, Document, Pos, Sheet};

    /// A sheet with A1:A4 = 1, "7", <blank>, 3 and B1 = TRUE — one fixture, because most of
    /// what needs testing here is how a function treats the values it did *not* get a
    /// number for.
    fn document() -> Document {
        let mut sheet = Sheet::new("Sheet1");
        sheet.set(Pos::new(0, 0), CellValue::Number(1.0));
        sheet.set(Pos::new(1, 0), CellValue::Text("7".into()));
        sheet.set(Pos::new(3, 0), CellValue::Number(3.0));
        sheet.set(Pos::new(0, 1), CellValue::Bool(true));
        Document {
            sheets: vec![sheet],
            ..Default::default()
        }
    }

    fn eval(formula: &str) -> Value {
        let document = document();
        // Row 20 keeps the formula clear of the fixture's own rows, so an implied
        // intersection cannot accidentally succeed.
        Engine::new(&document).eval(formula, super::Address::new(0, Pos::new(20, 20)))
    }

    fn number(formula: &str) -> f64 {
        match eval(formula) {
            Value::Number(n) => n,
            other => panic!("{formula} evaluated to {other:?}"),
        }
    }

    #[test]
    fn a_sequence_skips_what_a_scalar_converts() {
        // §6.3.7 against §6.3.5, the asymmetry the whole argument layer exists to keep.
        assert_eq!(number("=SUM([.A1:.A4])"), 4.0); // the referenced "7" is skipped
        assert_eq!(number("=SUM([.A1];\"7\";[.A4])"), 11.0); // the argument "7" converts
        assert_eq!(number("=COUNT([.A1:.A4])"), 2.0);
        assert_eq!(number("=COUNTA([.A1:.A4])"), 3.0); // text counts, the blank does not
        assert_eq!(number("=COUNTBLANK([.A1:.A4])"), 1.0);
    }

    #[test]
    fn if_short_circuits_and_knows_its_seven_shapes() {
        // §6.15.4. The guard is the point: the branch not taken is never evaluated.
        assert_eq!(
            eval("=IF(FALSE();1/0;\"safe\")"),
            Value::Text("safe".into())
        );
        assert_eq!(eval("=IF(1)"), Value::Bool(true)); // one parameter converts
        assert_eq!(eval("=IF(FALSE();1)"), Value::Bool(false)); // two: IfFalse is FALSE
        assert_eq!(eval("=IF(TRUE();;2)"), Value::Number(0.0)); // an empty branch is 0
        assert_eq!(eval("=IF(FALSE();1;)"), Value::Number(0.0));
    }

    #[test]
    fn and_or_and_not_fold_over_references_too() {
        assert_eq!(eval("=AND(TRUE();1)"), Value::Bool(true));
        assert_eq!(eval("=OR(FALSE();0)"), Value::Bool(false));
        assert_eq!(eval("=NOT([.B1])"), Value::Bool(false));
        // §4.6: an error in any parameter wins over a FALSE that would end the fold.
        assert_eq!(
            eval("=AND(FALSE();1/0)"),
            Value::Error(FormulaError::DivZero)
        );
    }

    #[test]
    fn the_maths_matches_section_6_16() {
        assert_eq!(number("=MOD(-3;2)"), 1.0); // §6.16.42: sign of the divisor
        assert_eq!(number("=MOD(3;-2)"), -1.0);
        assert_eq!(number("=ATAN2(1;1)"), std::f64::consts::FRAC_PI_4); // ATAN2(x;y)
        assert_eq!(number("=FACT(5)"), 120.0);
        assert_eq!(number("=EVEN(1.5)"), 2.0);
        assert_eq!(number("=EVEN(-1.5)"), -2.0);
        assert_eq!(number("=ODD(0)"), 1.0);
        assert_eq!(number("=INT(-1.5)"), -2.0); // §6.17.2: towards negative infinity
        assert_eq!(number("=TRUNC(-1.5)"), -1.0); // §6.17.8: towards zero
        assert_eq!(number("=ROUND(2.5)"), 3.0); // §6.17.5: halfway rounds away from zero
        assert_eq!(number("=ROUND(1.2345;2)"), 1.23);
        assert_eq!(number("=ROUND(1234;-2)"), 1200.0);
        assert_eq!(number("=LOG(8;2)"), 3.0);
        assert_eq!(eval("=SQRT(-1)"), Value::Error(FormulaError::Num));
        assert_eq!(eval("=FACT(-1)"), Value::Error(FormulaError::Num));
        assert_eq!(eval("=MOD(1;0)"), Value::Error(FormulaError::DivZero));
    }

    #[test]
    fn statistics_ignore_what_is_not_a_number() {
        assert_eq!(number("=AVERAGE([.A1:.A4])"), 2.0); // (1+3)/2, not /4
        assert_eq!(number("=MAX([.A1:.A4])"), 3.0);
        assert_eq!(number("=MIN([.B2:.B9])"), 0.0); // §6.18.48: no numbers is 0
        assert_eq!(number("=VARP(2;4)"), 1.0);
        assert_eq!(number("=VAR(2;4)"), 2.0);
        assert_eq!(number("=STDEVP(2;4)"), 1.0);
        assert_eq!(
            eval("=AVERAGE([.C1:.C9])"),
            Value::Error(FormulaError::DivZero)
        );
        assert_eq!(eval("=VAR(1)"), Value::Error(FormulaError::DivZero));
    }

    #[test]
    fn the_is_family_looks_at_errors_instead_of_propagating_them() {
        assert_eq!(eval("=ISERROR(1/0)"), Value::Bool(true));
        assert_eq!(eval("=ISERR(NA())"), Value::Bool(false)); // §6.13.15 excludes #N/A
        assert_eq!(eval("=ISNA(NA())"), Value::Bool(true));
        assert_eq!(eval("=ISBLANK([.A3])"), Value::Bool(true));
        assert_eq!(eval("=ISNUMBER([.A1])"), Value::Bool(true));
        assert_eq!(eval("=ISTEXT([.A2])"), Value::Bool(true));
        assert_eq!(eval("=ISLOGICAL([.B1])"), Value::Bool(true));
        assert_eq!(number("=N([.B1])"), 1.0);
        assert_eq!(number("=N(\"7\")"), 0.0); // §6.13.26: text is 0, however numeric
        assert_eq!(number("=VALUE(\"7\")"), 7.0); // ... which VALUE is the counterpart to
        assert_eq!(number("=ROWS([.A1:.B4])"), 4.0);
        assert_eq!(number("=COLUMNS([.A1:.B4])"), 2.0);
    }

    #[test]
    fn text_functions_count_characters_not_bytes() {
        assert_eq!(number("=LEN(\"héllo\")"), 5.0);
        assert_eq!(eval("=MID(\"héllo\";2;3)"), Value::Text("éll".into()));
        assert_eq!(eval("=LEFT(\"abc\")"), Value::Text("a".into()));
        assert_eq!(eval("=RIGHT(\"abc\";2)"), Value::Text("bc".into()));
        assert_eq!(eval("=LEFT(\"ab\";99)"), Value::Text("ab".into())); // clamps
        assert_eq!(
            eval("=PROPER(\"hello WORLD\")"),
            Value::Text("Hello World".into())
        );
        assert_eq!(eval("=TRIM(\"  a   b  \")"), Value::Text("a b".into()));
        assert_eq!(eval("=EXACT(\"a\";\"A\")"), Value::Bool(false)); // unlike `=`
        assert_eq!(number("=FIND(\"b\";\"abc\")"), 2.0);
        assert_eq!(
            eval("=FIND(\"z\";\"abc\")"),
            Value::Error(FormulaError::Value)
        );
        assert_eq!(
            eval("=SUBSTITUTE(\"aaa\";\"a\";\"b\";2)"),
            Value::Text("aba".into())
        );
        assert_eq!(
            eval("=REPLACE(\"abcd\";2;2;\"X\")"),
            Value::Text("aXd".into())
        );
        assert_eq!(eval("=T([.A1])"), Value::Text(String::new()));
    }

    #[test]
    fn an_unknown_function_is_name_and_a_misused_one_is_value() {
        assert_eq!(eval("=NOSUCHFUNCTION(1)"), Value::Error(FormulaError::Name));
        assert_eq!(
            eval("=COM.MICROSOFT.CUBEMEMBER(1)"),
            Value::Error(FormulaError::Name)
        );
        assert_eq!(eval("=PI(1)"), Value::Error(FormulaError::Value));
        assert_eq!(eval("=ABS()"), Value::Error(FormulaError::Value));
    }

    /// The feature line, checked by a machine rather than by discipline (CLAUDE.md).
    ///
    /// `doc/small-group.md` is the 110 functions §2.3.2 E) enumerates, and it is the whole
    /// scope of the evaluator. A function outside it is bloat by definition — so adding one
    /// fails here, and moving the line means editing the list on purpose.
    #[test]
    fn nothing_outside_the_small_group_gets_implemented() {
        let small_group = include_str!("../../../../doc/small-group.md");
        let listed: Vec<&str> = small_group
            .lines()
            .filter_map(|line| line.strip_prefix("- `"))
            .filter_map(|line| line.split('`').next())
            .collect();
        assert_eq!(
            listed.len(),
            110,
            "the Small Group is 110 functions (§2.3.2 E)"
        );
        for name in super::implemented() {
            assert!(listed.contains(&name), "{name} is not in the Small Group");
        }
        eprintln!(
            "small group: {} of {} implemented",
            super::implemented().len(),
            listed.len()
        );
    }

    #[test]
    fn every_listed_name_actually_dispatches() {
        // `implemented()` is what the scoreboard and the CLI will report; a name in the list
        // that no category answers to would be a lie told by a constant.
        let document = document();
        for name in super::implemented() {
            let mut engine = Engine::new(&document);
            let value = engine.eval(
                &format!("={name}()"),
                super::Address::new(0, Pos::new(20, 20)),
            );
            assert_ne!(
                value,
                Value::Error(FormulaError::Name),
                "{name} is listed but not implemented"
            );
        }
    }
}
