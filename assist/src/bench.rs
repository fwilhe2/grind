// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The benchmark (`doc/formula-assistant.md` §5): the function samples, read as tasks.
//!
//! `sheet/tests/data/functions/` holds one document per category of Part 4 §6, each with a
//! `Samples` sheet whose rows are a function, the formula as typed, the formula with the value
//! LibreOffice computed for it, and a sentence saying what it shows. Every row is a task: given
//! the document, the cell and the sentence, produce a formula that evaluates to the oracle's
//! value there.
//!
//! The model must not see the answers, so each task's document is the sample document with its
//! `Samples` sheet **replaced by an empty one of the same name**, at the same index — the cell
//! keeps its address, which `ROW()` and `COLUMN()` need, and nothing of any other sample leaks
//! through the glance. Every sample refers to the data sheets by name, so emptying the sheet it
//! sits on changes no answer.

use std::path::Path;

use grind_sheet::{App, CellValue, Pos};

use crate::fence::Fence;
use crate::{Backend, Catalog, Request, Verdict, check, prompt};

/// One task.
#[derive(Clone, Debug)]
pub struct Task {
    /// `math.fods:Samples.C2` — the document and the cell, stable across runs.
    pub id: String,
    /// Which of [`Bench::documents`] the task is set in.
    pub document: usize,
    pub sheet: usize,
    pub at: Pos,
    /// The function the sample was written for, for reporting only.
    pub function: String,
    /// What the person asks for: the sample's *What it shows*.
    pub request: String,
    /// The sample's formula as typed, in display syntax — the one known right answer.
    pub reference: String,
    /// The oracle's value for it.
    pub expected: CellValue,
}

/// Every task, and the documents they are set in.
pub struct Bench {
    /// `(file name, the document with its samples emptied)`.
    pub documents: Vec<(String, App)>,
    pub tasks: Vec<Task>,
}

/// The sheet the samples are on, in every document.
const SAMPLES: &str = "Samples";

impl Bench {
    /// Read every `.fods` in `dir` that has a `Samples` sheet.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == "fods"))
            .collect();
        paths.sort();
        let mut bench = Bench {
            documents: Vec::new(),
            tasks: Vec::new(),
        };
        for path in paths {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let app = App::new();
            app.open_file(&path).map_err(|e| format!("{name}: {e}"))?;
            let Some(sheet) =
                (0..app.sheet_count()).find(|&i| app.sheet_name(i).is_ok_and(|n| n == SAMPLES))
            else {
                continue;
            };
            let document = bench.documents.len();
            let mut tasks = samples(&app, sheet, document, &name)?;
            // The answers go before the model is shown anything: an empty sheet of the same
            // name, moved back to where the old one was.
            app.remove_sheet(sheet).map_err(|e| e.to_string())?;
            let added = app.add_sheet(SAMPLES).map_err(|e| e.to_string())?;
            if added != sheet {
                return Err(format!("{name}: {SAMPLES} is not the last sheet"));
            }
            bench.tasks.append(&mut tasks);
            bench.documents.push((name, app));
        }
        Ok(bench)
    }

    /// Ask `backend` for formulas for every task as `setup` says, and check each one.
    pub fn run(&self, backend: &mut dyn Backend, setup: Setup) -> Result<Score, String> {
        self.run_each(backend, setup, |_, _| {})
    }

    /// [`Bench::run`], calling `each` with every task and its outcome as it is scored — for a
    /// run long enough that somebody wants to watch it.
    pub fn run_each(
        &self,
        backend: &mut dyn Backend,
        setup: Setup,
        mut each: impl FnMut(&Task, &Outcome),
    ) -> Result<Score, String> {
        let mut outcomes = Vec::new();
        for task in &self.tasks {
            let request = self.request(task, setup)?;
            let app = &self.documents[task.document].1;
            let proposed = backend.propose(&request)?;
            let verdicts = proposed
                .iter()
                .take(setup.candidates)
                .map(|c| check(app, task.sheet, task.at, c, &task.expected))
                .collect();
            let outcome = Outcome {
                id: task.id.clone(),
                proposed,
                verdicts,
            };
            each(task, &outcome);
            outcomes.push(outcome);
        }
        Ok(Score { outcomes })
    }

    /// What a backend is asked for one task.
    pub fn request(&self, task: &Task, setup: Setup) -> Result<Request, String> {
        let app = &self.documents[task.document].1;
        let context = app
            .assist_context(task.sheet, task.at)
            .map_err(|e| e.to_string())?;
        Ok(Request {
            id: task.id.clone(),
            prompt: prompt(&context, &task.request, setup.catalog),
            candidates: setup.candidates,
            fence: setup.fence.then(|| Fence::for_context(&context).gbnf()),
        })
    }
}

/// One configuration of the benchmark: one row group of §5's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    pub candidates: usize,
    pub catalog: Catalog,
    /// Whether every candidate is generated inside [`Fence`] (A2).
    pub fence: bool,
}

fn samples(app: &App, sheet: usize, document: usize, name: &str) -> Result<Vec<Task>, String> {
    let (rows, _) = app.used_extent(sheet).map_err(|e| e.to_string())?;
    let text = |row, col| app.value_text(sheet, Pos { row, col }).unwrap_or_default();
    let mut tasks = Vec::new();
    // Row 1 is the header.
    for row in 1..rows {
        let at = Pos { row, col: 2 };
        if app.formula(sheet, at).map_err(|e| e.to_string())?.is_none() {
            continue;
        }
        tasks.push(Task {
            id: format!("{name}:{}", grind_sheet::a1::format(Some(SAMPLES), at)),
            document,
            sheet,
            at,
            function: text(row, 0),
            request: text(row, 3),
            reference: text(row, 1),
            expected: app.get(sheet, at).map_err(|e| e.to_string())?,
        });
    }
    Ok(tasks)
}

/// What became of one task.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub id: String,
    /// What the backend proposed, as it said it.
    pub proposed: Vec<String>,
    /// One verdict per candidate checked, in the backend's order.
    pub verdicts: Vec<Verdict>,
}

impl Outcome {
    /// The candidate a person would be shown first: the first one that survives the checks.
    pub fn first(&self) -> Option<&Verdict> {
        self.verdicts.iter().find(|v| v.survives())
    }
}

/// A run's result.
#[derive(Clone, Debug)]
pub struct Score {
    pub outcomes: Vec<Outcome>,
}

impl Score {
    /// The same run as though only the first `candidates` had been asked for — so one run at five
    /// answers for one, three and five.
    pub fn at(&self, candidates: usize) -> Score {
        Score {
            outcomes: self
                .outcomes
                .iter()
                .map(|o| Outcome {
                    id: o.id.clone(),
                    proposed: o.proposed.iter().take(candidates).cloned().collect(),
                    verdicts: o.verdicts.iter().take(candidates).cloned().collect(),
                })
                .collect(),
        }
    }

    pub fn tasks(&self) -> usize {
        self.outcomes.len()
    }

    /// Tasks whose first surviving candidate is right — what a person is shown, with no help.
    pub fn first_right(&self) -> usize {
        self.count(|o| o.first() == Some(&Verdict::Right))
    }

    /// Tasks where *some* candidate is right — what a person who gives the expected answer gets
    /// (§4.4 step 4), and so the ceiling for this backend at this many candidates.
    pub fn any_right(&self) -> usize {
        self.count(|o| o.verdicts.contains(&Verdict::Right))
    }

    /// Tasks where no candidate survived, and the person is told so.
    pub fn unanswered(&self) -> usize {
        self.count(|o| o.first().is_none())
    }

    /// Candidates by verdict, over every task: unparseable, an error, wrong, right.
    pub fn verdicts(&self) -> [usize; 4] {
        let mut counts = [0; 4];
        for verdict in self.outcomes.iter().flat_map(|o| &o.verdicts) {
            counts[match verdict {
                Verdict::Unparseable => 0,
                Verdict::Error(_) => 1,
                Verdict::Wrong(_) => 2,
                Verdict::Right => 3,
            }] += 1;
        }
        counts
    }

    fn count(&self, f: impl Fn(&Outcome) -> bool) -> usize {
        self.outcomes.iter().filter(|o| f(o)).count()
    }
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [unparseable, error, wrong, right] = self.verdicts();
        writeln!(f, "tasks       {}", self.tasks())?;
        writeln!(f, "first right {}", self.first_right())?;
        writeln!(f, "any right   {}", self.any_right())?;
        writeln!(f, "unanswered  {}", self.unanswered())?;
        write!(
            f,
            "candidates  {unparseable} unparseable, {error} errors, {wrong} wrong, {right} right"
        )
    }
}
