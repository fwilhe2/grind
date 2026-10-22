// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Score a finished run again under another reading, with no model: every candidate a1 saved
//! (`--out`), checked as it was and with Excel's syntax read as well ([`grind_assist::read`]).
//!
//! ```sh
//! cargo run --release -p grind-assist --example rescore -- RUN.jsonl [MORE.jsonl …]
//! ```
//!
//! The model's words are fixed; only the reading changes, so the difference between the two
//! columns is exactly what an Excel-syntax output filter is worth for that run.

use std::collections::HashMap;
use std::path::Path;

use grind_assist::Verdict;
use grind_assist::bench::Bench;

fn main() {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sheet/tests/data/functions");
    let bench = Bench::load(&samples).expect("the samples load");
    let tasks: HashMap<&str, _> = bench.pool.iter().map(|t| (t.id.as_str(), t)).collect();
    for path in std::env::args().skip(1) {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut rows = Vec::new();
        for line in text.lines() {
            let row: serde_json::Value = serde_json::from_str(line).expect("a JSON line");
            let id = row["id"].as_str().expect("an id");
            let task = tasks[id];
            let app = &bench.documents[task.document].1;
            let proposed: Vec<String> = row["proposed"]
                .as_array()
                .expect("candidates")
                .iter()
                .map(|c| c.as_str().unwrap_or_default().to_owned())
                .collect();
            let verdicts = |excel: bool| -> Vec<Verdict> {
                proposed
                    .iter()
                    .map(|c| {
                        grind_assist::check_with(app, task.sheet, task.at, c, &task.expected, excel)
                    })
                    .collect()
            };
            rows.push((verdicts(false), verdicts(true)));
        }
        let n = rows.first().map(|(v, _)| v.len()).unwrap_or(0);
        let old: Vec<&[Verdict]> = rows.iter().map(|(v, _)| v.as_slice()).collect();
        let new: Vec<&[Verdict]> = rows.iter().map(|(_, v)| v.as_slice()).collect();
        let ([g0, f0, a0, u0], [g1, f1, a1, u1]) = (summary(&old), summary(&new));
        let name = Path::new(&path)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or(path.clone());
        println!("\n### {name} ({} tasks × {n})\n", rows.len());
        println!("| | as run | + Excel read |\n|---|---|---|");
        println!("| greedy right | {g0} | {g1} |");
        println!("| first of {n} right | {f0} | {f1} |");
        println!("| any of {n} right | {a0} | {a1} |");
        println!("| greedy unparseable | {u0} | {u1} |");
    }
}

/// Greedy right, first surviving candidate right, any candidate right, greedy unparseable.
fn summary(rows: &[&[Verdict]]) -> [usize; 4] {
    let count = |f: &dyn Fn(&[Verdict]) -> bool| rows.iter().filter(|r| f(r)).count();
    [
        count(&|r| r[0] == Verdict::Right),
        count(&|r| r.iter().find(|v| v.survives()) == Some(&Verdict::Right)),
        count(&|r| r.contains(&Verdict::Right)),
        count(&|r| r[0] == Verdict::Unparseable),
    ]
}
