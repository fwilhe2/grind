// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop G′ (`doc/grammar.md` §6): the built-in checker over `tests/data/samples.txt`, sentences
//! with one deliberate slip each and correct ones meant to tempt a checker. The `FLOOR` is a
//! ratchet: raise it, never lower it.

use grind_core::grammar::{Checker, Finding, Paragraph};
use grind_grammar::Builtin;

const SAMPLES: &str = include_str!("data/samples.txt");

/// How many of the sentences with a slip the built-in checker finds something in.
const FLOOR: usize = 17;

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
    paragraphs.iter().map(|p| checker.check(p)).collect()
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
