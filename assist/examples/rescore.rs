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
        let survives = |v: &&Verdict| v.survives();
        let greedy = |pick: fn(&(Vec<Verdict>, Vec<Verdict>)) -> &Vec<Verdict>| {
            rows.iter().filter(|r| pick(r)[0] == Verdict::Right).count()
        };
        let first = |pick: fn(&(Vec<Verdict>, Vec<Verdict>)) -> &Vec<Verdict>| {
            rows.iter()
                .filter(|r| pick(r).iter().find(survives) == Some(&Verdict::Right))
                .count()
        };
        let any = |pick: fn(&(Vec<Verdict>, Vec<Verdict>)) -> &Vec<Verdict>| {
            rows.iter()
                .filter(|r| pick(r).contains(&Verdict::Right))
                .count()
        };
        let unparseable = |pick: fn(&(Vec<Verdict>, Vec<Verdict>)) -> &Vec<Verdict>| {
            rows.iter()
                .filter(|r| pick(r)[0] == Verdict::Unparseable)
                .count()
        };
        let (old, new): (
            fn(&(Vec<Verdict>, Vec<Verdict>)) -> &Vec<Verdict>,
            fn(&(Vec<Verdict>, Vec<Verdict>)) -> &Vec<Verdict>,
        ) = (|r| &r.0, |r| &r.1);
        let name = Path::new(&path)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or(path.clone());
        println!("\n### {name} ({} tasks × {n})\n", rows.len());
        println!("| | as run | + Excel read |\n|---|---|---|");
        println!("| greedy right | {} | {} |", greedy(old), greedy(new));
        println!("| first of {n} right | {} | {} |", first(old), first(new));
        println!("| any of {n} right | {} | {} |", any(old), any(new));
        println!(
            "| greedy unparseable | {} | {} |",
            unparseable(old),
            unparseable(new)
        );
    }
}
