// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `doc/generator-spec.md` against the code, both directions.
//!
//! The mechanism is `doc/small-group.md`'s, pointed at a host API instead of a function
//! catalogue: a document that nothing checks drifts, and **the host API is the surface most
//! likely to drift**, because adding to it is three lines in `register()` and forgetting to
//! write them down costs nothing at the time.
//!
//! Rust cannot reflect over what an `Engine` was handed — not without Rhai's `metadata`
//! feature, which would put `serde_json` in a crate that has no use for it — so this reads the
//! source, exactly as `cli/tests/parity.rs` reads `impl App`. The registrations are string
//! literals in one function per document type, which is what makes that honest rather than
//! clever.
//!
//! Three checks:
//!
//! 1. every function the specification names is registered;
//! 2. every function registered is named in the specification;
//! 3. every limit in §2.4 carries the constant's real value.
//!
//! What none of them cover: argument types, return values and the prose. `tests/smoke.rs`
//! executes most of that, and `examples/*.rhai` the rest.

/// Read at *compile* time, so this cannot pass by looking in the wrong place at runtime.
const SPEC: &str = include_str!("../../doc/generator-spec.md");
const ENGINE: &str = include_str!("../src/engine.rs");

/// The two files that register anything, with the name the specification gives each half.
const HOSTS: [(&str, &str); 2] = [
    ("the spreadsheet (§4)", include_str!("../src/sheet.rs")),
    ("the word processor (§5)", include_str!("../src/text.rs")),
];

/// The smallest API that is plausibly complete — `cli/tests/parity.rs`'s `least`, for the same
/// reason: a scanner that matched nothing would pass vacuously and quietly retire the check.
const LEAST: usize = 25;

/// Every name handed to `register_fn` or `register_get`.
///
/// Both spellings of the call are read, because rustfmt breaks the longer ones over lines and a
/// scanner that only saw `register_fn("name"` would silently miss exactly the functions with
/// the most arguments.
fn registered() -> Vec<String> {
    let mut names = Vec::new();
    for (_, source) in HOSTS {
        for call in ["register_fn(", "register_get("] {
            for (at, _) in source.match_indices(call) {
                let rest = &source[at + call.len()..];
                let open = rest.find('"').expect("a registration names its function");
                let close = rest[open + 1..].find('"').expect("that name ends");
                names.push(rest[open + 1..open + 1 + close].to_owned());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Every function the specification names, from the first column of its **API tables**.
///
/// Two conventions, and the document says so in §8. The API is §4 and §5 and nothing else, so
/// the scan is bounded by those headings — otherwise §2.3's `eval` and §3.1's `true` would read
/// as promised functions, which is the opposite of what those tables say about them. Inside
/// them, a row's first cell is a code span holding the call as a script writes it —
/// `` `sheet(name)` ``, `` `s.push(row)` ``, `` `s.rows` `` — so the name is what is left after
/// dropping a receiver and stopping at the parenthesis.
fn documented() -> Vec<String> {
    let mut names = Vec::new();
    let mut inside = false;
    for line in SPEC.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            inside = heading.starts_with("4.") || heading.starts_with("5.");
            continue;
        }
        if !inside {
            continue;
        }
        let Some(rest) = line.trim().strip_prefix("| `") else {
            continue;
        };
        let Some((call, _)) = rest.split_once('`') else {
            continue;
        };
        // `s.push(row)` is a method and `s.rows` is the same method as a property.
        let call = call.rsplit('.').next().unwrap_or(call);
        let name = call.split('(').next().unwrap_or(call).trim();
        // A name is an identifier. Anything else in those tables is a row about a value rather
        // than a call, and there is nothing to check about it here.
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        {
            names.push(name.to_owned());
        }
    }
    names.sort();
    names.dedup();
    names
}

#[test]
fn every_registered_function_is_in_the_specification() {
    let documented = documented();
    let missing: Vec<String> = registered()
        .into_iter()
        .filter(|name| !documented.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "registered and undocumented: {missing:?}. Every function a script can call has a row \
         in doc/generator-spec.md §4 or §5 — that is the check the specification exists to \
         pass, since adding one is three lines in register()."
    );
}

#[test]
fn every_function_in_the_specification_is_registered() {
    let registered = registered();
    let extra: Vec<String> = documented()
        .into_iter()
        .filter(|name| !registered.contains(name))
        .collect();
    assert!(
        extra.is_empty(),
        "documented and not registered: {extra:?}. A specification promising a function that \
         does not exist is worse than one missing a function that does."
    );
}

/// The vacuity guard, in both directions at once.
#[test]
fn the_scanners_found_an_api() {
    assert!(
        registered().len() >= LEAST,
        "only {} registrations found — the scanner has stopped matching the source",
        registered().len()
    );
    assert!(
        documented().len() >= LEAST,
        "only {} calls found in the specification — its tables have changed shape",
        documented().len()
    );
    for (which, source) in HOSTS {
        assert!(
            source.contains("register_fn("),
            "{which} registers nothing, which cannot be right"
        );
    }
}

/// §2.4's table against `engine.rs`: the numbers are the constants' own.
///
/// A limit written down wrongly is worse than one not written down at all — it is what somebody
/// will quote when a script of theirs stops, and they will conclude the build is broken.
#[test]
fn every_limit_carries_the_value_the_engine_uses() {
    let mut found = 0;
    for line in ENGINE.lines() {
        let Some(rest) = line.trim().strip_prefix("const ") else {
            continue;
        };
        let (name, value) = rest
            .split_once(": ")
            .and_then(|(name, rest)| rest.split_once(" = ").map(|(_, v)| (name, v)))
            .expect("a constant is `const NAME: TYPE = VALUE;`");
        let value = value.trim_end_matches(';');
        let row = SPEC
            .lines()
            .find(|line| line.trim().starts_with(&format!("| `{name}` |")))
            .unwrap_or_else(|| {
                panic!("{name} is a limit with no row in doc/generator-spec.md §2.4")
            });
        assert!(
            row.contains(&format!("`{value}`")),
            "{name} is `{value}` in build/src/engine.rs, and §2.4 says: {row}"
        );
        found += 1;
    }
    assert!(found >= 4, "only {found} limits found in engine.rs");
}
