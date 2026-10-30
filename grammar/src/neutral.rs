// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The rules that run on every paragraph, whatever its language.

use grind_core::grammar::{Finding, Kind};

use crate::Language;
use crate::tokens::Token;

/// Words a language rightly writes twice in a row: `that that`, `had had`, `die die`. Where the
/// language is unknown every list counts.
fn doubled(language: Option<Language>, word: &str) -> bool {
    const ENGLISH: &[&str] = &["that", "had", "is"];
    const GERMAN: &[&str] = &["die", "der", "das", "den", "dem", "sie"];
    let lists: &[&[&str]] = match language {
        Some(Language::English) => &[ENGLISH],
        Some(Language::German) => &[GERMAN],
        Some(_) => &[],
        None => &[ENGLISH, GERMAN],
    };
    lists.iter().any(|list| list.contains(&word))
}

/// `the the`: a word, plain spaces, the same word again. Covers the second word and the space
/// before it, so taking the replacement deletes both.
pub fn repeated_word(tokens: &[Token], language: Option<Language>, out: &mut Vec<Finding>) {
    for window in tokens.windows(3) {
        let [first, space, second] = window else {
            continue;
        };
        if !(first.is_word() && space.is_plain_space() && second.is_word()) {
            continue;
        }
        let word = second.lower();
        if word != first.lower()
            || word.chars().any(|c| c.is_numeric())
            || first.text.chars().next().is_some_and(char::is_uppercase)
                && second.text.chars().next().is_some_and(char::is_uppercase)
            || doubled(language, &word)
        {
            continue;
        }
        out.push(crate::finding(
            "repeated-word",
            Kind::Grammar,
            (space, second),
            format!("\u{201c}{}\u{201d} twice", second.text),
            vec![String::new()],
        ));
    }
}
