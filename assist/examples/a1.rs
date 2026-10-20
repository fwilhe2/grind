// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A1: put the model a llama-server has loaded through the benchmark
//! (`doc/formula-assistant.md` §5, §8).
//!
//! ```sh
//! scripts/llama-server.sh serve                      # the recommended model, pinned
//! cargo run --release -p grind-assist --example a1   # every task, with and without the catalog
//! cargo run --release -p grind-assist --example a1 -- --catalog whole --limit 20 --out /tmp/a1
//! ```
//!
//! Options: `--url` (default `http://127.0.0.1:8080`), `--candidates` (5), `--catalog`
//! (`whole`, `none` or `both`, the default), `--limit` (the first N tasks), `--only` (tasks whose
//! id or function contains this), `--out DIR` (one JSON line per task and configuration, every
//! raw reply included, for reading the failures afterwards).
//!
//! One run at five candidates answers for one, three and five ([`Score::at`]), so the table's
//! rows cost one pass. The server keeps the system part of the prompt evaluated between
//! requests, which is why the catalog's 5,000-odd tokens are paid once per configuration rather
//! than once per task.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use grind_assist::bench::{Bench, Score};
use grind_assist::server::{Server, Timing};
use grind_assist::{Catalog, Verdict};
use serde_json::json;

struct Options {
    url: String,
    candidates: usize,
    catalogs: Vec<Catalog>,
    limit: Option<usize>,
    only: Option<String>,
    out: Option<PathBuf>,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        url: "http://127.0.0.1:8080".into(),
        candidates: 5,
        catalogs: vec![Catalog::Whole, Catalog::None],
        limit: None,
        only: None,
        out: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} wants a value"));
        match flag.as_str() {
            "--url" => options.url = value()?,
            "--candidates" => options.candidates = value()?.parse().map_err(|e| format!("{e}"))?,
            "--limit" => options.limit = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            "--only" => options.only = Some(value()?),
            "--out" => options.out = Some(value()?.into()),
            "--catalog" => {
                options.catalogs = match value()?.as_str() {
                    "whole" => vec![Catalog::Whole],
                    "none" => vec![Catalog::None],
                    "both" => vec![Catalog::Whole, Catalog::None],
                    other => return Err(format!("--catalog {other}: whole, none or both")),
                }
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(options)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("a1: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = options()?;
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sheet/tests/data/functions");
    let mut bench = Bench::load(&samples)?;
    if let Some(only) = &options.only {
        bench
            .tasks
            .retain(|t| t.id.contains(only.as_str()) || t.function == *only);
    }
    if let Some(limit) = options.limit {
        bench.tasks.truncate(limit);
    }
    let mut server = Server::new(&options.url)?;
    let model = server.model()?;
    if let Some(out) = &options.out {
        std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    }

    for &catalog in &options.catalogs {
        let label = match catalog {
            Catalog::Whole => "whole",
            Catalog::None => "none",
        };
        eprintln!("— {model}, catalog {label}, {} tasks", bench.tasks.len());
        let (from, started) = (server.timings.len(), Instant::now());
        let total = bench.tasks.len();
        let mut done = 0;
        let score = bench.run_each(&mut server, options.candidates, catalog, |task, outcome| {
            done += 1;
            let mark = match outcome.first() {
                Some(Verdict::Right) => "right",
                Some(_) => "wrong",
                None => "none ",
            };
            let first = outcome.proposed.first().map(String::as_str).unwrap_or("");
            eprintln!(
                "[{done}/{total}] {mark} {} {}  {first}",
                task.id, task.function
            );
        })?;
        let elapsed = started.elapsed();
        let timings = &server.timings[from..];
        print!(
            "{}",
            table(&model, label, &score, options.candidates, timings, elapsed)
        );
        if let Some(out) = &options.out {
            write_lines(
                &out.join(format!("{}-{label}.jsonl", slug(&model))),
                &bench,
                &score,
                &server.replies[from..],
                timings,
                options.candidates,
            )?;
        }
    }
    Ok(())
}

fn table(
    model: &str,
    catalog: &str,
    score: &Score,
    candidates: usize,
    timings: &[Timing],
    elapsed: std::time::Duration,
) -> String {
    let tasks = score.tasks();
    let row =
        |label: &str, n: usize| format!("| {label} | {n}/{tasks} ({:.0}%) |\n", pct(n, tasks));
    let greedy = score.at(1);
    let [unparseable, error, wrong, right] = greedy.verdicts();
    let mut out = format!("\n### {model}, catalog {catalog}\n\n| | |\n|---|---|\n");
    out += &row("greedy, right", right);
    for n in [3, 5].into_iter().filter(|n| *n <= candidates) {
        out += &row(
            &format!("+ check, first of {n} right"),
            score.at(n).first_right(),
        );
    }
    out += &row(
        &format!("+ expect, any of {candidates} right"),
        score.any_right(),
    );
    out += &format!(
        "| greedy, why not | {unparseable} unparseable, {error} an error, {wrong} wrong |\n"
    );
    let first: Vec<&Timing> = timings.iter().step_by(candidates.max(1)).collect();
    out += &format!(
        "| prompt read per task (median) | {} tokens, {:.1} s |\n",
        median(first.iter().map(|t| t.prompt_tokens as f64)),
        median(first.iter().map(|t| t.prompt_ms)) / 1000.0
    );
    out += &format!(
        "| reply per candidate (median) | {} tokens, {:.1} s |\n",
        median(timings.iter().map(|t| t.predicted_tokens as f64)),
        median(timings.iter().map(|t| t.predicted_ms)) / 1000.0
    );
    out += &format!(
        "| wall time | {:.0} min for {tasks} tasks × {candidates} |\n",
        elapsed.as_secs_f64() / 60.0
    );
    out
}

fn write_lines(
    path: &Path,
    bench: &Bench,
    score: &Score,
    replies: &[String],
    timings: &[Timing],
    candidates: usize,
) -> Result<(), String> {
    let say = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut file = std::fs::File::create(path).map_err(say)?;
    for (i, (task, outcome)) in bench.tasks.iter().zip(&score.outcomes).enumerate() {
        let span = i * candidates..((i + 1) * candidates).min(replies.len());
        let line = json!({
            "id": task.id,
            "function": task.function,
            "request": task.request,
            "reference": task.reference,
            "expected": format!("{:?}", task.expected),
            "proposed": outcome.proposed,
            "verdicts": outcome.verdicts.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>(),
            "replies": replies.get(span.clone()).unwrap_or_default(),
            "timings": timings.get(span).unwrap_or_default().iter().map(|t| json!({
                "prompt_tokens": t.prompt_tokens, "prompt_ms": t.prompt_ms,
                "predicted_tokens": t.predicted_tokens, "predicted_ms": t.predicted_ms,
            })).collect::<Vec<_>>(),
        });
        writeln!(file, "{line}").map_err(say)?;
    }
    eprintln!("wrote {}", path.display());
    Ok(())
}

fn pct(n: usize, of: usize) -> f64 {
    100.0 * n as f64 / of.max(1) as f64
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<f64> = values.collect();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn slug(model: &str) -> String {
    model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .trim_end_matches(".gguf")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}
