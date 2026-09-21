// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind test` (D8), end to end below the CLI: a script builds, is recalculated, and its
//! `test_` functions are called with what it built.

use std::rc::Rc;

use grind_build::NoData;
use grind_build::test::run;

const MODEL: &str = r#"
let s = sheet("Sales");
s.push(row(["Region", "Units"]));
s.push(row(["North", 400]));
s.push(row(["South", 380]));
s.push(row(["Total", sum_above()]));
s;
"#;

fn outcomes(tests: &str) -> Vec<(String, Option<String>)> {
    let source = format!("{MODEL}\n{tests}");
    run(&source, "model.rhai", Rc::new(NoData))
        .expect("it builds")
        .outcomes
        .into_iter()
        .map(|o| (o.name, o.failure.map(|e| e.to_string())))
        .collect()
}

/// The recalculated answer is what a test sees: `sum_above()` wrote a formula, and the total is
/// the formula's.
#[test]
fn a_test_reads_the_recalculated_document() {
    let got = outcomes(
        r#"
fn test_the_total(d) {
    assert_eq(d.cell("B4").value, 780);
    assert_eq(d.cell("B4").formula, "=SUM([.B2:.B3])");
    assert_eq(d.cell("A1").value, "Region");
    assert_eq(d.cell("Sales.C9").value, ());
    assert(d.lint().is_empty());
}
"#,
    );
    assert_eq!(got, [("test_the_total".to_owned(), None)]);
}

const TESTS: &str = r#"
fn test_wrong(d) {
    let total = d.cell("B4").value;
    assert_eq(total, 781);
}

fn test_right(d) {
    assert_near(d.cell("B2").value, 400.004, 0.01);
}
"#;

/// A failure names the test, says what was expected, and points at the assertion's own line —
/// not at the runner's call, which is nowhere in the script. The other tests still run.
#[test]
fn a_failure_says_what_and_where() {
    let got = outcomes(TESTS);
    // In order of name, not of definition — Rhai's function table has no order of its own.
    let names: Vec<&str> = got.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["test_right", "test_wrong"]);
    assert_eq!(got[0].1, None);
    let failure = got[1].1.as_deref().expect("it fails");
    let source = format!("{MODEL}\n{TESTS}");
    let line = 1 + source
        .lines()
        .position(|l| l.contains("assert_eq(total, 781)"))
        .expect("the assertion is in the source");
    assert_eq!(
        failure,
        format!("model.rhai:{line}:5: expected 781, got 780"),
        "the assertion's own line, and the number as a person writes it"
    );
}

/// A file with no checks in it is a mistake, not a pass — a CI step running it must not go green.
#[test]
fn a_script_with_no_tests_is_an_error() {
    let error = run(MODEL, "model.rhai", Rc::new(NoData))
        .err()
        .expect("no tests is an error");
    assert!(error.message.contains("defines no tests"), "{error}");
}

#[test]
fn a_test_takes_the_document_and_nothing_else() {
    let got = outcomes("fn test_two(a, b) { }");
    assert!(
        got[0]
            .1
            .as_deref()
            .unwrap_or_default()
            .contains("takes one, the document"),
        "{got:?}"
    );
}

/// The text document's half: a block by any of its addresses.
#[test]
fn a_text_document_is_read_by_block() {
    let source = r#"
let d = text();
d.heading(1, "Report");
d.para("The first paragraph.");
d;

fn test_blocks(d) {
    assert_eq(d.block("p1"), "Report");
    assert_eq(d.block("§1"), "Report");
    assert_eq(d.block("p2"), "The first paragraph.");
}
"#;
    let tests = run(source, "report.rhai", Rc::new(NoData)).expect("it builds");
    assert!(
        tests.passed(),
        "{:?}",
        tests
            .outcomes
            .iter()
            .map(|o| &o.failure)
            .collect::<Vec<_>>()
    );
}

/// `grind build` never calls a test, so a model with tests in it builds exactly as it did.
#[test]
fn a_build_ignores_the_tests() {
    let source = format!("{MODEL}\nfn test_never_run(d) {{ assert(false); }}");
    assert!(grind_build::build(&source, "model.rhai").is_ok());
}
