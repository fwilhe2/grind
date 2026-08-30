// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! D6's exit criterion, made mechanical: **every rule named in a table and covered by a test.**
//!
//! `doc/dsl.md` §4.3 holds the table, and this reads it — the same mechanism
//! `doc/small-group.md` uses against `funcs::implemented()` and `cli/tests/parity.rs` uses
//! against each `App`. A rule with no row fails the build; a row naming a rule nothing
//! implements fails it too.
//!
//! It lives here rather than in either application because the table is one table for the
//! suite: a rule that "applies to both" is one row and two `RULES` entries, and only a test
//! that can see both crates can check that.

use grind_core::lint::Rule;

/// The section, read at compile time so this cannot pass by looking in the wrong place.
const DSL: &str = include_str!("../../doc/dsl.md");

/// Every id §4.3's table names, in the order it names them.
///
/// The table is markdown, so a row is a line of `|`-separated cells and the id is the one in
/// backticks — parsed rather than duplicated here, because a copy of the list would be a third
/// place for it to be wrong.
fn documented() -> Vec<String> {
    let section = DSL
        .split_once("### 4.3 Linting")
        .expect("doc/dsl.md still has a §4.3")
        .1;
    let section = section
        .split_once("### 4.4")
        .expect("§4.3 ends where §4.4 begins")
        .0;
    section
        .lines()
        .filter(|line| line.starts_with('|'))
        .filter_map(|line| line.split('|').nth(2))
        .map(str::trim)
        .filter_map(|cell| cell.strip_prefix('`')?.strip_suffix('`'))
        .map(str::to_owned)
        .collect()
}

/// Every rule both applications implement, deduplicated: `off-palette` and `unspellable` are one
/// row of the table and one rule in each crate, which is the shape "applies to both" has.
fn implemented() -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = Vec::new();
    let rules: Vec<Rule> = grind_sheet::lint::RULES
        .into_iter()
        .chain(grind_text::lint::RULES)
        .collect();
    for rule in rules {
        if !ids.contains(&rule.id) {
            ids.push(rule.id);
        }
    }
    ids
}

#[test]
fn every_rule_the_table_names_is_implemented() {
    let documented = documented();
    assert!(
        documented.len() >= 5,
        "§4.3's table parsed to {documented:?}, which is too few rows to be the real table"
    );
    for id in &documented {
        assert!(
            implemented().contains(&id.as_str()),
            "doc/dsl.md §4.3 names the rule `{id}`, which neither `grind_sheet::lint::RULES` \
             nor `grind_text::lint::RULES` implements"
        );
    }
}

#[test]
fn every_rule_implemented_is_named_in_the_table() {
    let documented = documented();
    for id in implemented() {
        assert!(
            documented.iter().any(|row| row == id),
            "the rule `{id}` is implemented and doc/dsl.md §4.3's table does not name it — \
             a rule nobody can look up is a rule nobody can turn off"
        );
    }
}

/// Ids are the vocabulary `--off` and `--format json` share, so two rules may not answer to one
/// name — including across the two applications, where a shared id has to mean the same thing.
#[test]
fn a_shared_id_means_the_same_rule_in_both_applications() {
    for sheet in grind_sheet::lint::RULES {
        for text in grind_text::lint::RULES {
            if sheet.id == text.id {
                assert_eq!(
                    (sheet.severity, sheet.what),
                    (text.severity, text.what),
                    "`{}` is one row of §4.3's table and must not be two different rules",
                    sheet.id
                );
            }
        }
    }
}
