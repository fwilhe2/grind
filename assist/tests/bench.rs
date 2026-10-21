// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A0's exit criterion (`doc/formula-assistant.md` §8): the benchmark is proved right before
//! any model is put through it. Two stand-ins, one at each end:
//!
//! - the **reference** backend answers every task with the sample's own formula, and must score
//!   every task — so a task the scoring gets wrong cannot be blamed on a model later;
//! - the **garbage** backend answers with one formula of each failure kind, and must score none
//!   and have each kind named — so the checks a person relies on actually reject.
//!
//! Neither needs anything but the vendored samples, so this never skips.

use std::collections::HashMap;
use std::path::Path;

use grind_assist::bench::{Bench, Setup};
use grind_assist::fence::Fence;
use grind_assist::{Backend, Catalog, Request, Verdict};
use grind_sheet::formula::funcs;

fn bench() -> Bench {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sheet/tests/data/functions");
    Bench::load(&dir).expect("the function samples load")
}

fn setup(candidates: usize) -> Setup {
    Setup {
        candidates,
        catalog: Catalog::Whole,
        fence: true,
        rules: true,
        examples: 12,
    }
}

struct Reference(HashMap<String, String>);

impl Backend for Reference {
    fn propose(&mut self, request: &Request) -> Result<Vec<String>, String> {
        Ok(vec![self.0[&request.id].clone()])
    }
}

struct Garbage;

impl Backend for Garbage {
    fn propose(&mut self, _: &Request) -> Result<Vec<String>, String> {
        Ok(vec![
            "=SUM(".into(),
            "=NO.SUCH.FUNCTION(1)".into(),
            "=31415926.5358979".into(),
        ])
    }
}

#[test]
fn every_implemented_function_is_a_task() {
    let bench = bench();
    for name in funcs::implemented() {
        assert!(
            bench.tasks.iter().any(|t| t.function == *name),
            "{name} has no task"
        );
    }
    assert!(bench.tasks.iter().all(|t| !t.request.is_empty()));
}

#[test]
fn the_reference_answers_score_every_task() {
    let bench = bench();
    let answers = bench
        .tasks
        .iter()
        .map(|t| (t.id.clone(), t.reference.clone()))
        .collect();
    let score = bench.run(&mut Reference(answers), setup(1)).unwrap();
    let missed: Vec<String> = score
        .outcomes
        .iter()
        .filter(|o| o.verdicts != [Verdict::Right])
        .map(|o| format!("{}: {:?} → {:?}", o.id, o.proposed, o.verdicts))
        .collect();
    assert!(missed.is_empty(), "{}", missed.join("\n"));
    assert_eq!(score.first_right(), bench.tasks.len());
}

#[test]
fn garbage_scores_nothing_and_every_kind_is_named() {
    let bench = bench();
    let score = bench.run(&mut Garbage, setup(3)).unwrap();
    let tasks = bench.tasks.len();
    assert_eq!(score.first_right(), 0);
    assert_eq!(score.any_right(), 0);
    assert_eq!(
        score.unanswered(),
        0,
        "the constant survives the checks, wrongly"
    );
    assert_eq!(score.verdicts(), [tasks, tasks, tasks, 0]);
    // Asked for one, garbage leaves every task unanswered: nothing it says survives.
    assert_eq!(score.at(1).unanswered(), tasks);
    assert_eq!(score.at(2).verdicts(), [tasks, tasks, 0, 0]);
}

#[test]
fn the_model_is_never_shown_an_answer() {
    let bench = bench();
    for task in &bench.tasks {
        let app = &bench.documents[task.document].1;
        let shown = app.assist_context(task.sheet, task.at).unwrap().to_string();
        assert!(
            !shown.contains(task.reference.trim_start_matches('=')),
            "{} shows its own answer",
            task.id
        );
        assert!(
            !shown.contains(&task.request),
            "{} shows its request",
            task.id
        );
    }
}

/// The fence lets every right answer through: each sample's own formula is accepted by the
/// grammar built for its document — the exact GBNF a server is sent. A fence that refused a
/// right answer would cap every model's score without anyone noticing.
#[test]
fn the_fence_lets_every_reference_answer_through() {
    let bench = bench();
    let refused: Vec<String> = bench
        .tasks
        .iter()
        .filter(|task| {
            let app = &bench.documents[task.document].1;
            let context = app.assist_context(task.sheet, task.at).unwrap();
            !Fence::for_context(&context).accepts(task.reference.trim_start_matches('='))
        })
        .map(|task| format!("{}: {}", task.id, task.reference))
        .collect();
    assert!(refused.is_empty(), "{}", refused.join("\n"));
}

/// A worked example never comes from the task's own document, and a sample is spread over all of
/// them.
#[test]
fn examples_come_from_elsewhere_and_a_sample_covers_every_document() {
    let mut bench = bench();
    for task in &bench.tasks {
        let own: Vec<&str> = bench
            .pool
            .iter()
            .filter(|t| t.document == task.document)
            .map(|t| t.reference.as_str())
            .collect();
        let examples = bench.examples(task.document, 12);
        assert_eq!(examples.len(), 12);
        assert!(examples.iter().all(|(_, f)| !own.contains(&f.as_str())));
    }
    bench.sample(40);
    assert_eq!(bench.tasks.len(), 40);
    let documents: std::collections::BTreeSet<usize> =
        bench.tasks.iter().map(|t| t.document).collect();
    assert_eq!(documents.len(), bench.documents.len());
    assert_eq!(bench.pool.len(), 188);
}
