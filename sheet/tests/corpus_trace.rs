// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The workbench's trace, held to loop B's corpus (`doc/formula-workbench.md` F1).
//!
//! Two claims, over every formula the world wrote that survives display form:
//!
//! 1. **The spans are right.** Every node's text, cut out of the display formula and parsed
//!    on its own, is that node's subtree exactly — so selecting a node in the workbench
//!    selects the text that means it, and nothing else.
//! 2. **The trace is the evaluator.** Its root is what the engine computes for the cell, so
//!    the workbench can never show one answer and the cell hold another.
//!
//! Point it at a LibreOffice checkout, as for loop B:
//!
//!     GRIND_LO_CORPUS=/path/to/libreoffice/core cargo test -p grind-sheet --test corpus_trace

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use grind_sheet::formula::display::{from_display, parse_display, to_display};
use grind_sheet::formula::eval::{Address, Engine};
use grind_sheet::formula::parse::{Expr, Shape, parse};
use grind_sheet::formula::trace::trace;

const DEFAULT_CHECKOUT: &str = "/home/florian/code/github.com/LibreOffice/core";
const CORPUS: &str = "sc/qa/unit/data";
const DIRS: [&str; 3] = ["functions", "ods", "fods"];

/// The functions whose answer changes between two evaluations of the same formula — the
/// clock and the dice (§6.10.15, §6.10.19, §6.16.50, §6.16.51). A document that calls one
/// can differ between the trace and the evaluator by the time it took to get there, which
/// is the function working rather than the trace failing.
const VOLATILE: [&str; 4] = ["NOW", "TODAY", "RAND", "RANDBETWEEN"];

/// How many formulas per document are *evaluated* (claim 2). Every trace builds its own
/// engine, as the workbench's does, so a workbook of 300,000 formulas would evaluate each
/// one's whole precedent cone afresh; the spans (claim 1) are checked for every formula.
const EVALUATED_PER_DOCUMENT: usize = 50;

fn corpus_root() -> Option<PathBuf> {
    let root = PathBuf::from(
        std::env::var("GRIND_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CHECKOUT.to_owned()),
    )
    .join(CORPUS);
    root.is_dir().then_some(root)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ods" | "fods")
        ) {
            out.push(path);
        }
    }
}

fn children(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::Call { args, .. } => args.iter().collect(),
        Expr::Prefix(_, inner) | Expr::Postfix(_, inner) | Expr::Paren(inner) => vec![inner],
        Expr::Binary(_, left, right) => vec![left, right],
        _ => Vec::new(),
    }
}

/// Claim 1 for one node and everything under it; the first mismatch, if any.
fn spans_hold(display: &str, expr: &Expr, shape: &Shape) -> Result<(), String> {
    if children(expr).len() != shape.children.len() {
        return Err(format!(
            "{} children, {} shapes",
            children(expr).len(),
            shape.children.len()
        ));
    }
    if matches!(expr, Expr::Empty) {
        if !shape.range.is_empty() {
            return Err(format!("an omitted parameter spans {:?}", shape.range));
        }
    } else {
        let text = display
            .get(shape.range.clone())
            .ok_or_else(|| format!("{:?} is not on a char boundary", shape.range))?;
        match parse_display(text) {
            Ok((alone, _)) if alone == *expr => {}
            Ok((alone, _)) => return Err(format!("`{text}` alone is {alone}, in place {expr}")),
            Err(e) => return Err(format!("`{text}` alone does not parse: {e}")),
        }
    }
    children(expr)
        .into_iter()
        .zip(&shape.children)
        .try_for_each(|(child, shape)| spans_hold(display, child, shape))
}

#[test]
fn every_corpus_formula_traces_with_true_spans_and_the_evaluators_answer() {
    let Some(root) = corpus_root() else {
        eprintln!(
            "skipping: no LibreOffice checkout at {DEFAULT_CHECKOUT}; \
             set GRIND_LO_CORPUS to its root to run the trace's corpus check"
        );
        return;
    };
    let mut files = Vec::new();
    for dir in DIRS {
        collect(&root.join(dir), &mut files);
    }
    files.sort();
    assert!(!files.is_empty(), "corpus at {} is empty", root.display());

    let (mut spanned, mut evaluated, mut excused) = (0usize, 0usize, 0usize);
    let mut failures: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    let mut fail = |kind: &str, example: String| {
        let entry = failures.entry(kind.to_owned()).or_default();
        entry.0 += 1;
        if entry.1.len() < 5 {
            entry.1.push(example);
        }
    };
    for path in &files {
        let Ok(doc) = grind_sheet::read_file(path) else {
            continue;
        };
        let mut budget = EVALUATED_PER_DOCUMENT;
        let volatile = doc.sheets.iter().any(|sheet| {
            sheet.formulas().any(|(_, formula)| {
                grind_sheet::formula::funcs::used(formula)
                    .is_some_and(|names| names.iter().any(|n| VOLATILE.contains(&n.as_str())))
            })
        });
        for (index, sheet) in doc.sheets.iter().enumerate() {
            for (pos, formula) in sheet.formulas() {
                let Ok(expr) = parse(formula) else {
                    continue; // loop B's parse half owns these
                };
                let canonical = format!("={expr}");
                let display = to_display(&canonical).expect("it just parsed");
                if from_display(&display).as_deref() != Ok(canonical.as_str()) {
                    continue; // loop B's named ambiguities, which display form cannot spell
                }
                spanned += 1;
                match parse_display(&display) {
                    Ok((again, shape)) if again == expr => {
                        if let Err(why) = spans_hold(&display, &expr, &shape) {
                            fail("a span is wrong", format!("{display}: {why}"));
                        }
                    }
                    Ok((again, _)) => fail("parsed differently", format!("{display} -> {again}")),
                    Err(e) => fail("did not parse", format!("{display}: {e}")),
                }
                if budget == 0 {
                    continue;
                }
                budget -= 1;
                evaluated += 1;
                let at = Address::new(index, pos);
                let want = Engine::new(&doc).eval(&canonical, at);
                match trace(&doc, index, pos, &display, &Default::default()) {
                    Ok(t) if t.value == want => {}
                    Ok(_) if volatile => excused += 1,
                    Ok(t) => fail(
                        "root differs from the evaluator",
                        format!("{}: {display}: {:?} vs {want:?}", path.display(), t.value),
                    ),
                    Err(e) => fail("did not trace", format!("{display}: {e}")),
                }
            }
        }
    }

    let failed: usize = failures.values().map(|(n, _)| n).sum();
    eprintln!(
        "trace: {spanned} formulas spanned, {evaluated} evaluated ({excused} differing in a \
         document with a volatile function), in {} documents; {failed} failed",
        files.len()
    );
    for (kind, (count, examples)) in &failures {
        eprintln!("  {count:>5}  {kind}");
        for example in examples {
            eprintln!("           {example}");
        }
    }
    assert!(spanned > 0, "nothing was checked");
    assert_eq!(
        failed, 0,
        "{failed} formulas failed the trace's corpus check"
    );
}
