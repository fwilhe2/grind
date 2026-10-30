// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop G′ (`doc/grammar.md` §6): the built-in checker over `tests/data/samples.txt`, sentences
//! with one deliberate slip each and correct ones meant to tempt a checker — always — and against
//! LanguageTool where `GRIND_LANGUAGETOOL_URL` names a server. The `FLOOR`s are a ratchet: raise
//! them, never lower them.

use std::ops::Range;

use grind_core::grammar::{Checker, Finding, Paragraph};
use grind_grammar::Builtin;

const SAMPLES: &str = include_str!("data/samples.txt");

/// How many of the sentences with a slip the built-in checker finds something in.
const FLOOR: usize = 17;

/// How many of those LanguageTool also finds something in, where it runs.
const AGREEMENT_FLOOR: usize = 12;

struct Sample {
    wrong: bool,
    language: &'static str,
    text: &'static str,
}

fn samples() -> Vec<Sample> {
    SAMPLES
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let (head, text) = line.split_once('|').expect("`E en|` or `C en|`");
            let (kind, language) = head.split_once(' ').expect("a kind and a language");
            Sample {
                wrong: kind == "E",
                language: match language {
                    "de" => "de-DE",
                    _ => "en-GB",
                },
                text,
            }
        })
        .collect()
}

fn check(checker: &dyn Checker, samples: &[Sample]) -> Vec<Vec<Finding>> {
    let paragraphs: Vec<Paragraph<'_>> = samples
        .iter()
        .map(|s| Paragraph {
            text: s.text,
            skip: &[],
            language: Some(s.language),
        })
        .collect();
    checker.check_all(&paragraphs)
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end || a == b
}

#[test]
fn the_built_in_checker_finds_slips_and_leaves_correct_sentences_alone() {
    let samples = samples();
    let found = check(&Builtin::default(), &samples);
    let mut caught = 0;
    for (sample, findings) in samples.iter().zip(&found) {
        match sample.wrong {
            true => caught += usize::from(!findings.is_empty()),
            false => assert!(
                findings.is_empty(),
                "a correct sentence drew {:?}: {:?}",
                findings.iter().map(|f| &f.rule).collect::<Vec<_>>(),
                sample.text
            ),
        }
    }
    let wrong = samples.iter().filter(|s| s.wrong).count();
    eprintln!("loop G′: the built-in checker caught {caught} of {wrong} slips");
    assert!(caught >= FLOOR, "caught {caught}, the floor is {FLOOR}");
}

#[test]
fn the_built_in_checker_against_languagetool() {
    let Ok(url) = std::env::var("GRIND_LANGUAGETOOL_URL") else {
        eprintln!("skipped: GRIND_LANGUAGETOOL_URL names no LanguageTool server");
        return;
    };
    let samples = samples();
    let ours = check(&Builtin::default(), &samples);
    let server =
        grind_languagetool::LanguageTool::new(grind_languagetool::Server::SelfHosted(url), false);
    let theirs = check(&server, &samples);
    assert_eq!(server.error(), None);
    let (mut both, mut only_ours, mut only_theirs) = (0, Vec::new(), Vec::new());
    for ((sample, ours), theirs) in samples.iter().zip(&ours).zip(&theirs) {
        for finding in ours {
            match theirs.iter().any(|t| overlaps(&t.range, &finding.range)) {
                true => both += usize::from(sample.wrong),
                false => only_ours.push(format!("{} in {:?}", finding.rule, sample.text)),
            }
        }
        if sample.wrong && ours.is_empty() && !theirs.is_empty() {
            only_theirs.push(format!(
                "{:?}: {:?}",
                theirs.iter().map(|f| &f.rule).collect::<Vec<_>>(),
                sample.text
            ));
        }
    }
    eprintln!("loop G′: {both} slips found by both");
    eprintln!("only the built-in checker (suspected false positives, or LanguageTool's gaps):");
    for line in &only_ours {
        eprintln!("  {line}");
    }
    eprintln!("only LanguageTool (the backlog):");
    for line in &only_theirs {
        eprintln!("  {line}");
    }
    assert!(
        both >= AGREEMENT_FLOOR,
        "{both} found by both, the floor is {AGREEMENT_FLOOR}"
    );
}
