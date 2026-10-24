// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `--help` is written for the person running the command. Clap prints every `///` on a command
//! or an argument, so a developer note left there reaches the user; this renders the help of
//! every command and refuses the vocabulary of one. A reason belongs in a `//` comment beside it.

use std::process::Command;

/// What a developer note looks like, and why each is not for a user. `§2.1.3` stays allowed:
/// it is grind's own outline address. A spec citation is the one in brackets, `(§5.4)`.
const FORBIDDEN: &[(&str, &str)] = &[
    ("doc/", "a design document"),
    (".rs", "a source file"),
    ("rng:", "a schema line"),
    ("::", "a Rust path"),
    ("this build", "the developers' phrase for grind"),
    ("span map", "an internal data structure"),
    ("token map", "an internal data structure"),
    ("bijective", "a design argument"),
];

fn help(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_grind"))
        .args(args)
        .arg("--help")
        .output()
        .expect("grind runs");
    assert!(output.status.success(), "grind {args:?} --help failed");
    String::from_utf8(output.stdout).expect("help is UTF-8")
}

/// The subcommands a help page lists, `help` aside.
fn subcommands(page: &str) -> Vec<String> {
    page.lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_owned)
        .collect()
}

/// A spec citation: `(§` followed by a section number and a closing bracket.
fn cites_a_section(line: &str) -> bool {
    line.match_indices("(§").any(|(at, _)| {
        let rest = &line[at + "(§".len()..];
        let number: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        !number.is_empty() && rest[number.len()..].starts_with(')')
    })
}

#[test]
fn no_help_page_speaks_to_developers() {
    let mut pages = vec![(String::new(), help(&[]))];
    for top in subcommands(&pages[0].1) {
        let page = help(&[&top]);
        for sub in subcommands(&page) {
            pages.push((format!("{top} {sub}"), help(&[&top, &sub])));
        }
        pages.push((top, page));
    }
    assert!(pages.len() > 50, "only {} help pages found", pages.len());

    let mut found = Vec::new();
    for (command, page) in &pages {
        for line in page.lines() {
            for (word, what) in FORBIDDEN {
                if line.contains(word) {
                    found.push(format!("grind {command}: {what} ({word}): {}", line.trim()));
                }
            }
            if cites_a_section(line) {
                found.push(format!("grind {command}: a spec section: {}", line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "help text written for developers:\n{}",
        found.join("\n")
    );
}

#[test]
fn a_section_citation_is_told_from_an_outline_address() {
    assert!(cites_a_section("Set the width of a column (§5.4)"));
    assert!(!cites_a_section("by outline path (§2.1.3, or s2.1.3)"));
    assert!(!cites_a_section("e.g. p3, #intro or §2.1"));
}
