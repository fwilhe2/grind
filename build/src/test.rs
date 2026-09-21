// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind test` — `doc/dsl.md` §4.4, D8: **a generated document's numbers, checked**.
//!
//! A test is a function in the model's own script whose name starts with `test_`, and which takes
//! the document:
//!
//! ```rhai
//! let s = sheet("Sales");
//! s.push(row(["North", 400]));
//! s.push(row(["South", 380]));
//! s.push(row(["Total", sum_above()]));
//! s;
//!
//! fn test_the_total_is_the_sum_of_the_regions(d) {
//!     assert_eq(d.cell("B3").value, 780);
//! }
//! ```
//!
//! `grind test model.rhai` builds the document exactly as `grind build` does, **recalculates it**
//! — the answer a test asks about is the one the formulas give, not whatever the script typed —
//! and calls every `test_` function with it, one at a time, each failure with the line of the
//! assertion that failed. `grind build` never calls them; a function nothing calls is inert, so
//! the model and its checks are one file, the way a Rust module and its `#[cfg(test)]` are. The
//! model's last statement is still its document — `s;` with a semicolon, since Rhai will not
//! take a function definition straight after a bare expression, and the value of a script is
//! its last statement's either way.
//!
//! **Why functions rather than §4.4's `test "name" { … }`.** That sketch is not Rhai, and making
//! it Rhai means custom syntax — a second grammar in a crate whose whole argument is that the
//! language is somebody else's. A function is already a name, a body and a line number, and a
//! Rhai function cannot see the script's own variables, so a test can only reach the document
//! through the one argument it is given: it checks what was **built**, never what the script
//! happened to leave lying around.
//!
//! **What a test can do that a build cannot, and the reverse.** A test gets the assertions and a
//! read-only [`Built`] document; it cannot change the document it is checking, because there is
//! no method here that would. The builders are still in scope, since the vocabulary is one
//! engine's, but a sheet built inside a test is a value that goes nowhere.

use std::rc::Rc;

use rhai::{Dynamic, Engine, EvalAltResult, ImmutableString};

use crate::hint::{hint, hint_get};
use crate::{Artifact, Error};

/// The document a test reads — the one the script built, recalculated. Read-only by
/// construction: every method registered below asks and none of them edits.
#[derive(Clone)]
pub struct Built(Rc<Artifact>);

/// One cell of a [`Built`] spreadsheet, as a test sees it.
#[derive(Clone)]
pub struct Cell {
    value: Dynamic,
    display: ImmutableString,
    formula: Dynamic,
}

/// What `grind test` found.
pub struct Tests {
    /// What the script built, for a report to name.
    pub kind: grind_core::kind::DocumentKind,
    /// Every `test_` function, by name, with its failure if it had one.
    pub outcomes: Vec<Outcome>,
}

pub struct Outcome {
    pub name: String,
    pub failure: Option<Error>,
}

impl Tests {
    pub fn passed(&self) -> bool {
        self.outcomes
            .iter()
            .all(|outcome| outcome.failure.is_none())
    }
}

/// Build the script, recalculate what it built, and run its tests.
///
/// `Err` is for a script that did not build — the same error `grind build` would give — or one
/// that defines no test at all, which is a mistake rather than a pass: a CI step that runs a
/// file with no checks in it must not go green.
pub fn run(source: &str, script: &str, data: Rc<dyn crate::Data>) -> Result<Tests, Error> {
    let mut engine = crate::engine::engine(data);
    register(&mut engine);
    let ast = engine
        .compile(source)
        .map_err(|e| crate::position(script, &e.into()))?;
    let value = engine
        .eval_ast::<Dynamic>(&ast)
        .map_err(|e| crate::position(script, &e))?;
    let artifact = crate::materialise(value, script)?;
    if let Artifact::Spreadsheet(app) = &artifact {
        // The answers a test asks about are the formulas', so they are computed first — the
        // same recalculation `grind build` does before it writes.
        app.recalc().map_err(|e| Error::at(script, e.to_string()))?;
    }
    let kind = artifact.kind();
    let built = Built(Rc::new(artifact));

    // By name: Rhai's function table is not in the order the script wrote them, and a report
    // that reorders itself between runs is one nobody can diff.
    let mut tests: Vec<(String, usize)> = ast
        .iter_functions()
        .filter(|f| f.name.starts_with("test_"))
        .map(|f| (f.name.to_owned(), f.params.len()))
        .collect();
    tests.sort();
    if tests.is_empty() {
        return Err(Error::at(
            script,
            "defines no tests — a test is a function whose name starts with `test_` and which \
             takes the document, `fn test_totals(d) { … }`",
        ));
    }

    let outcomes = tests
        .into_iter()
        .map(|(name, arity)| {
            let failure = match arity {
                1 => engine
                    .call_fn_with_options::<Dynamic>(
                        // The script's statements already ran, above: a test sees the document
                        // that built, not a second one built for it.
                        rhai::CallFnOptions::new().eval_ast(false),
                        &mut rhai::Scope::new(),
                        &ast,
                        &name,
                        (Dynamic::from(built.clone()),),
                    )
                    .err()
                    .map(|e| failure(script, unwrap_call(&e))),
                n => Some(Error::at(
                    script,
                    format!("`{name}` takes {n} parameters; a test takes one, the document"),
                )),
            };
            Outcome { name, failure }
        })
        .collect();
    Ok(Tests { kind, outcomes })
}

/// A failure inside a test arrives wrapped in "error in function `test_…`", at the position of
/// the *call* — which this runner made, and which is nowhere in the script. The inner error is
/// the one with the assertion's line.
fn unwrap_call(error: &EvalAltResult) -> &EvalAltResult {
    match error {
        EvalAltResult::ErrorInFunctionCall(_, _, inner, _) => unwrap_call(inner),
        other => other,
    }
}

/// A failure as the runner reports it. An assertion's own message is the whole sentence — Rhai
/// would print `Runtime error: ` in front of it, which says nothing a failed test does not.
fn failure(script: &str, error: &EvalAltResult) -> Error {
    let mut out = crate::position(script, error);
    if let EvalAltResult::ErrorRuntime(message, _) = error {
        out.message = message.to_string();
    }
    out
}

/// A failed assertion, as the runtime error Rhai positions at the call that raised it.
fn failed(message: String) -> Box<EvalAltResult> {
    EvalAltResult::ErrorRuntime(message.into(), rhai::Position::NONE).into()
}

/// Two script values, compared the way a person reading `assert_eq(total, 780)` means: an
/// integer and a float are equal when they are the same number, since a cell holds only floats
/// and a script mostly writes integers.
fn same(a: &Dynamic, b: &Dynamic) -> bool {
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.type_name() == b.type_name() && a.to_string() == b.to_string(),
    }
}

fn number(value: &Dynamic) -> Option<f64> {
    value
        .as_float()
        .ok()
        .or_else(|| value.as_int().ok().map(|i| i as f64))
}

/// A value as a failure message prints it — a string quoted, so `"3"` and `3` read differently,
/// and a whole number without the `.0` a cell's float would otherwise carry into `got 780.0`.
fn shown(value: &Dynamic) -> String {
    if let Some(n) = number(value) {
        return grind_sheet::formula::value::format_number(n);
    }
    match value.clone().into_immutable_string() {
        Ok(text) => format!("{:?}", text.as_str()),
        Err(_) if value.is_unit() => "()".to_owned(),
        Err(_) => value.to_string(),
    }
}

fn cell_value(value: grind_sheet::CellValue) -> Dynamic {
    match value {
        grind_sheet::CellValue::Empty => Dynamic::UNIT,
        grind_sheet::CellValue::Number(n) => Dynamic::from_float(n),
        grind_sheet::CellValue::Text(t) => Dynamic::from(t),
        grind_sheet::CellValue::Bool(b) => Dynamic::from_bool(b),
    }
}

impl Built {
    fn sheet(&self) -> Result<&grind_sheet::App, Box<EvalAltResult>> {
        match &*self.0 {
            Artifact::Spreadsheet(app) => Ok(app),
            Artifact::Text(_) => Err(failed(
                "this document is a text document, and `cell` asks a spreadsheet".to_owned(),
            )),
        }
    }

    fn text(&self) -> Result<&grind_text::App, Box<EvalAltResult>> {
        match &*self.0 {
            Artifact::Text(app) => Ok(app),
            Artifact::Spreadsheet(_) => Err(failed(
                "this document is a spreadsheet, and `block` asks a text document".to_owned(),
            )),
        }
    }

    fn cell(&self, address: &str) -> Result<Cell, Box<EvalAltResult>> {
        let app = self.sheet()?;
        let reference = grind_sheet::a1::parse(address).map_err(|e| failed(e.to_string()))?;
        let (sheet, at, _) =
            grind_sheet::a1::resolve(app, &reference).map_err(|e| failed(e.to_string()))?;
        let viewport = app
            .get_viewport(sheet, at.row..at.row + 1, at.col..at.col + 1)
            .map_err(|e| failed(e.to_string()))?;
        Ok(Cell {
            value: cell_value(viewport.get(at.row, at.col).cloned().unwrap_or_default()),
            display: viewport.text(at.row, at.col).unwrap_or_default().into(),
            formula: app
                .formula(sheet, at)
                .map_err(|e| failed(e.to_string()))?
                .map_or(Dynamic::UNIT, Dynamic::from),
        })
    }

    fn block(&self, address: &str) -> Result<ImmutableString, Box<EvalAltResult>> {
        let app = self.text()?;
        let loc = grind_text::loc::parse(address).map_err(|e| failed(e.to_string()))?;
        let index = app.resolve(&loc).map_err(|e| failed(e.to_string()))?;
        let viewport = app.get_viewport(index..index + 1);
        Ok(viewport
            .get(index)
            .map(|block| block.text.as_str())
            .unwrap_or_default()
            .into())
    }

    /// Every diagnostic `grind lint` would print, errors and warnings (the hints are off, as
    /// they are by default on the command line).
    fn lint(&self) -> rhai::Array {
        let options = grind_core::lint::Options::default();
        let report = match &*self.0 {
            Artifact::Spreadsheet(app) => app.lint(&options),
            Artifact::Text(app) => app.lint(&options),
        };
        report
            .diagnostics
            .iter()
            .map(|d| Dynamic::from(d.to_string()))
            .collect()
    }
}

/// The test vocabulary: four assertions and a read-only document. Registered only for `grind
/// test` — `grind build` has no assertion to make and no built document to read — and through
/// [`hint`] like every other registration, so an editor documents these too.
pub fn register(engine: &mut Engine) {
    engine.register_type_with_name::<Built>("Built");
    engine.register_type_with_name::<Cell>("Cell");

    hint(
        engine,
        "assert",
        ["condition: bool", "()"],
        ["/// Fail this test unless `condition` is true."],
        |condition: bool| -> Result<(), Box<EvalAltResult>> {
            match condition {
                true => Ok(()),
                false => Err(failed("assertion failed".to_owned())),
            }
        },
    );
    hint(
        engine,
        "assert",
        ["condition: bool", "message: string", "()"],
        ["/// Fail this test with `message` unless `condition` is true."],
        |condition: bool, message: ImmutableString| -> Result<(), Box<EvalAltResult>> {
            match condition {
                true => Ok(()),
                false => Err(failed(format!("assertion failed: {message}"))),
            }
        },
    );
    hint(
        engine,
        "assert_eq",
        ["actual: ?", "expected: ?", "()"],
        [
            "/// Fail this test unless the two values are equal. A number compares as a number, so",
            "/// `assert_eq(d.cell(\"B6\").value, 15400)` holds for the float the cell holds.",
        ],
        |actual: Dynamic, expected: Dynamic| -> Result<(), Box<EvalAltResult>> {
            match same(&actual, &expected) {
                true => Ok(()),
                false => Err(failed(format!(
                    "expected {}, got {}",
                    shown(&expected),
                    shown(&actual)
                ))),
            }
        },
    );
    hint(
        engine,
        "assert_ne",
        ["actual: ?", "unexpected: ?", "()"],
        ["/// Fail this test if the two values are equal."],
        |actual: Dynamic, unexpected: Dynamic| -> Result<(), Box<EvalAltResult>> {
            match same(&actual, &unexpected) {
                false => Ok(()),
                true => Err(failed(format!(
                    "expected anything but {}",
                    shown(&unexpected)
                ))),
            }
        },
    );
    hint(
        engine,
        "assert_near",
        ["actual: float", "expected: float", "tolerance: float", "()"],
        [
            "/// Fail this test unless `actual` is within `tolerance` of `expected` — for a total of",
            "/// fractions, where `0.1 + 0.2` is not `0.3` to the last bit.",
        ],
        |actual: Dynamic,
         expected: Dynamic,
         tolerance: Dynamic|
         -> Result<(), Box<EvalAltResult>> {
            let (Some(a), Some(e), Some(t)) =
                (number(&actual), number(&expected), number(&tolerance))
            else {
                return Err(failed("assert_near compares numbers".to_owned()));
            };
            match (a - e).abs() <= t {
                true => Ok(()),
                false => Err(failed(format!("expected {e} ± {t}, got {a}"))),
            }
        },
    );

    hint(
        engine,
        "cell",
        ["document: Built", "address: string", "Cell"],
        [
            "/// One cell of the built spreadsheet, by address — `\"B6\"` on the first sheet, or",
            "/// `\"Summary.B6\"`. Read after the document was recalculated.",
        ],
        |document: &mut Built, address: ImmutableString| document.cell(&address),
    );
    hint(
        engine,
        "block",
        ["document: Built", "address: string", "string"],
        [
            "/// One block's text in the built text document, by address — `\"p3\"`, `\"#intro\"`,",
            "/// `\"§2.1\"`.",
        ],
        |document: &mut Built, address: ImmutableString| document.block(&address),
    );
    hint(
        engine,
        "lint",
        ["document: Built", "array"],
        [
            "/// Every error and warning `grind lint` would report, one string each —",
            "/// `assert(d.lint().is_empty())` is a document that contradicts itself nowhere.",
        ],
        |document: &mut Built| document.lint(),
    );
    hint_get(
        engine,
        "value",
        ["/// What the cell holds: a number, a string, a bool, or `()` when it is empty."],
        |cell: &mut Cell| cell.value.clone(),
    );
    hint_get(
        engine,
        "display",
        ["/// What the cell shows — its value through its number format."],
        |cell: &mut Cell| cell.display.clone(),
    );
    hint_get(
        engine,
        "formula",
        [
            "/// The cell's formula in OpenFormula's syntax, `=SUM([.B2:.B5])`, or `()` if it has none.",
        ],
        |cell: &mut Cell| cell.formula.clone(),
    );
}
