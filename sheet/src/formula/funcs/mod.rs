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
//! All 110 of the Small Group are here. What is left is not a missing function but two
//! known gaps *inside* them, both named where they live: `criterion.rs` matches no
//! wildcards or regular expressions, and array formulas are read as ordinary ones.

use super::eval::{Address, Area, Engine, Operand};
use super::parse::Expr;
use super::value::{FormulaError, Value};

mod catalog;
mod criterion;
mod datetime;
mod db;
mod fin;
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
        datetime::call,
        db::call,
        fin::call,
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

pub use catalog::{FuncInfo, catalog, category};

/// Every function a formula calls, outermost first, each named once and upper-cased.
///
/// The names are what the formula *says*, not what this build implements: a document may
/// call a function the evaluator does not have, and an explorer that hid those would hide
/// exactly the ones worth finding. `None` when the formula will not parse.
pub fn used(formula: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    collect(&super::parse::parse(formula).ok()?, &mut names);
    Some(names)
}

fn collect(expr: &Expr, names: &mut Vec<String>) {
    match expr {
        Expr::Call { name, args } => {
            let name = name.to_uppercase();
            if !names.contains(&name) {
                names.push(name);
            }
            args.iter().for_each(|arg| collect(arg, names));
        }
        Expr::Prefix(_, inner) | Expr::Postfix(_, inner) | Expr::Paren(inner) => {
            collect(inner, names)
        }
        Expr::Binary(_, left, right) => {
            collect(left, names);
            collect(right, names);
        }
        _ => {}
    }
}

/// How many functions §2.3.2 E) enumerates. The conformance claim's denominator, and the
/// only number a coverage report may compare against — checked against
/// `doc/small-group.md` by `nothing_outside_the_small_group_gets_implemented`.
pub const SMALL_GROUP: usize = 110;

/// The functions implemented **beyond** the Small Group — the plan's one-at-a-time escape
/// hatch, and the second half of `doc/small-group.md`.
///
/// Reported apart from [`implemented`] because "112 of 110" is not a sentence. A caller
/// saying how much of the Small Group it covers needs the extras counted separately, or the
/// claim reads as broken arithmetic and stops meaning anything.
pub fn beyond_small_group() -> &'static [&'static str] {
    BEYOND
}

/// Kept in step with `doc/small-group.md`'s second half by the same test that checks the
/// first, so this cannot quietly grow either.
const BEYOND: &[&str] = &["COLUMN", "ROW"];

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
    "COLUMN",
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
    "ROW",
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
    // date and time (§6.10)
    "DATE",
    "DAY",
    "HOUR",
    "MINUTE",
    "MONTH",
    "NOW",
    "SECOND",
    "TIME",
    "TODAY",
    "WEEKDAY",
    "YEAR",
    // lookup (§6.14)
    "CHOOSE",
    "HLOOKUP",
    "INDEX",
    "MATCH",
    "VLOOKUP",
    // database (§6.9)
    "DAVERAGE",
    "DCOUNT",
    "DCOUNTA",
    "DGET",
    "DMAX",
    "DMIN",
    "DPRODUCT",
    "DSTDEV",
    "DSTDEVP",
    "DSUM",
    "DVAR",
    "DVARP",
    // financial (§6.12)
    "DDB",
    "FV",
    "IRR",
    "NPER",
    "NPV",
    "PMT",
    "PV",
    "RATE",
    "SLN",
    "SYD",
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

    /// The document's epoch (§3.4 `HOST-NULL-DATE`) — what makes a serial number a date.
    pub fn null_date(&self) -> i64 {
        self.engine.null_date()
    }

    /// `HOST-NULL-YEAR` (§3.4 item 7) — how `DATE` reads a two-digit year.
    pub fn null_year(&self) -> i64 {
        self.engine.null_year()
    }

    /// §6.3.15 Conversion to DateParam and §6.3.16 Conversion to TimeParam.
    ///
    /// One method for both, because both pseudotypes produce the same thing: a serial
    /// number. §4.3.4 is why — a DateTime "is a Date plus Time", so the two differ only in
    /// which half the *function* then reads, never in how the argument converts.
    ///
    /// Text goes to the ISO parsers first and falls back to §6.3.5's ordinary number
    /// conversion, which is the "may attempt to convert to a Number in other ways" both
    /// sections leave open — and is what makes `YEAR("2")` agree with `YEAR(2)`.
    pub fn serial(&mut self, i: usize) -> Result<f64, FormulaError> {
        match self.value(i) {
            Value::Text(s) => super::date::parse_date(&s, self.null_date())
                .or_else(|| super::date::parse_time(&s))
                .map_or_else(|| Value::Text(s).to_number(), Ok),
            other => other.to_number(),
        }
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
        self.numbers_range(0..self.args.len())
    }

    /// [`Args::numbers`] over some of the arguments — for the functions whose first
    /// parameter is not part of the sequence (`NPV`'s rate) or whose sequence is one
    /// argument (`IRR`'s values).
    pub fn numbers_range(
        &mut self,
        range: std::ops::Range<usize>,
    ) -> Result<Vec<f64>, FormulaError> {
        let mut out = Vec::new();
        for i in range {
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

    /// The catalog is what a shell offers a user, so it has to be the same list the
    /// evaluator answers to — a function missing from it is unreachable from an
    /// autocomplete, and one that is only in it is a promise nothing keeps.
    #[test]
    fn the_catalog_names_exactly_what_is_implemented() {
        let mut catalogued: Vec<&str> = super::catalog().iter().map(|f| f.name).collect();
        let mut implemented = super::implemented();
        catalogued.sort_unstable();
        implemented.sort_unstable();
        assert_eq!(catalogued, implemented);

        // Extracted from the spec rather than written about it, so each entry has to look
        // like what it was extracted from.
        for info in super::catalog() {
            assert!(
                info.signature.starts_with(info.name)
                    && info.signature[info.name.len()..].starts_with('('),
                "{}'s signature does not start with a call: {}",
                info.name,
                info.signature
            );
            assert!(!info.brief.is_empty(), "{} has no summary", info.name);
        }
    }

    /// [`category`] derives the group from the section number rather than storing one per
    /// entry, so the thing worth checking is that every section prefix this build actually
    /// uses is one the match arm recognises — an unrecognised one would silently file a
    /// function under "Other" in a help browser instead of failing a build.
    #[test]
    fn every_catalogued_function_has_a_known_category() {
        for info in super::catalog() {
            assert_ne!(
                super::category(info),
                "Other",
                "{} (§{}) has no category match arm",
                info.name,
                info.section
            );
        }
    }

    /// The section number is the citation, and a wrong one sends a reader to the wrong
    /// definition — which is worse than none. `doc/small-group.md` carries the same
    /// numbers, extracted the same way, so the two are checked against each other.
    #[test]
    fn every_catalogued_section_matches_the_small_group_document() {
        let doc = include_str!("../../../../doc/small-group.md");
        let documented: Vec<(&str, &str)> = doc
            .lines()
            .filter_map(|line| line.trim().strip_prefix("- `"))
            .filter_map(|line| line.split_once("` — §"))
            .collect();
        for info in super::catalog() {
            let (_, section) = documented
                .iter()
                .find(|(name, _)| *name == info.name)
                .unwrap_or_else(|| panic!("{} is not in doc/small-group.md", info.name));
            assert_eq!(
                *section, info.section,
                "{} cites §{} here and §{section} in doc/small-group.md",
                info.name, info.section
            );
        }
    }

    /// The feature line, checked by a machine rather than by discipline (CLAUDE.md).
    ///
    /// `doc/small-group.md` is the 110 functions §2.3.2 E) enumerates, and it is the whole
    /// scope of the evaluator. A function outside it is bloat by definition — so adding one
    /// fails here, and moving the line means editing the list on purpose.
    ///
    /// The document has a second half, after a `# Beyond the Small Group` heading, for
    /// functions moved in by the plan's explicit one-at-a-time decision. It is split on that
    /// heading rather than read as one list, so the §2.3.2 E) extract stays verbatim and
    /// exactly 110 — an addition cannot be smuggled in by growing the spec's own list, and
    /// the conformance claim keeps meaning what it says.
    #[test]
    fn nothing_outside_the_small_group_gets_implemented() {
        let doc = include_str!("../../../../doc/small-group.md");
        let (spec, beyond) = doc
            .split_once("\n# Beyond the Small Group")
            .expect("small-group.md's two halves");
        let names = |section: &'static str| -> Vec<&'static str> {
            section
                .lines()
                .filter_map(|line| line.strip_prefix("- `"))
                .filter_map(|line| line.split('`').next())
                .collect()
        };
        let (listed, extra) = (names(spec), names(beyond));
        assert_eq!(
            listed.len(),
            super::SMALL_GROUP,
            "the Small Group is {} functions (§2.3.2 E)",
            super::SMALL_GROUP
        );
        // The escape hatch is a constant *and* a document, and a reader believes whichever
        // it met first — so they are checked against each other rather than maintained in
        // parallel and hoped about.
        let mut declared = super::beyond_small_group().to_vec();
        let mut documented = extra.clone();
        declared.sort_unstable();
        documented.sort_unstable();
        assert_eq!(
            declared, documented,
            "funcs::beyond_small_group() and doc/small-group.md's second half disagree"
        );
        for name in super::implemented() {
            assert!(
                listed.contains(&name) || extra.contains(&name),
                "{name} is in neither half of doc/small-group.md"
            );
        }
        eprintln!(
            "small group: {} implemented, {} of them beyond §2.3.2 E)'s {}",
            super::implemented().len(),
            super::implemented()
                .iter()
                .filter(|n| !listed.contains(n))
                .count(),
            listed.len(),
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

    #[test]
    fn used_finds_every_call_however_deep() {
        assert_eq!(
            super::used("=ROUND(SUM([.A1:.A9])/COUNT([.A1:.A9]);2)"),
            Some(vec![
                "ROUND".to_owned(),
                "SUM".to_owned(),
                "COUNT".to_owned()
            ])
        );
        // Arithmetic calls nothing, a name is not a call, and one function twice is one
        // answer.
        assert_eq!(super::used("=[.A1]/2"), Some(Vec::new()));
        assert_eq!(super::used("=expenses"), Some(Vec::new()));
        assert_eq!(
            super::used("=sum([.A1])+SUM([.B1])"),
            Some(vec!["SUM".to_owned()])
        );
        // A function this build does not have is still one the document calls.
        assert_eq!(
            super::used("=WEBSERVICE(\"x\")"),
            Some(vec!["WEBSERVICE".to_owned()])
        );
        assert_eq!(super::used("=SUM("), None);
    }
}
