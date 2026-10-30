// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The rules that run on every paragraph, whatever its language.

use grind_core::grammar::{Finding, Kind};

use crate::Language;
use crate::tokens::{Sort, Token};

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

/// `word ,`: plain spaces between a word and the punctuation after it. French sets a space
/// before `;:!?`, an ellipsis may stand apart, `.50` is a number and ` :)` a smiley.
pub fn space_before_punctuation(
    tokens: &[Token],
    language: Option<Language>,
    out: &mut Vec<Finding>,
) {
    for (at, window) in tokens.windows(3).enumerate() {
        let [word, space, mark] = window else {
            continue;
        };
        if !(word.is_word() && space.is_plain_space() && mark.sort == Sort::Mark) {
            continue;
        }
        let after = tokens.get(at + 3);
        let next_char = after.and_then(|t| t.text.chars().next());
        let belongs = match mark.text.as_str() {
            ";" | ":" | "!" | "?" if language == Some(Language::French) => true,
            ":" => matches!(next_char, Some(')' | '(')),
            "." => next_char.is_some_and(|c| c == '.' || c.is_numeric()),
            "," | ";" | "!" | "?" => false,
            _ => true,
        };
        if belongs {
            continue;
        }
        out.push(crate::finding(
            "space-before-punctuation",
            Kind::Punctuation,
            (space, space),
            format!("No space before \u{201c}{}\u{201d}", mark.text),
            vec![String::new()],
        ));
    }
}

/// Two or more spaces between words, or after a sentence: one is enough. Spaces that start or
/// end a line, and a tab, are left alone.
pub fn double_space(tokens: &[Token], out: &mut Vec<Finding>) {
    for window in tokens.windows(3) {
        let [before, space, after] = window else {
            continue;
        };
        if space.is_plain_space()
            && space.range.len() > 1
            && before.sort != Sort::Space
            && after.sort != Sort::Space
            && before.text != "\n"
            && after.text != "\n"
        {
            out.push(crate::finding(
                "double-space",
                Kind::Typography,
                (space, space),
                "More than one space".to_owned(),
                vec![" ".to_owned()],
            ));
        }
    }
}

/// `!!`, `,,`, `??`: the same mark twice or more. An ellipsis is three dots on purpose, and
/// `?!` is two different marks.
pub fn repeated_punctuation(tokens: &[Token], out: &mut Vec<Finding>) {
    let mut at = 0;
    while at < tokens.len() {
        let first = &tokens[at];
        let run = tokens[at..]
            .iter()
            .take_while(|t| t.sort == Sort::Mark && t.text == first.text)
            .count();
        if run > 1 && matches!(first.text.as_str(), "," | ";" | ":" | "!" | "?") {
            out.push(crate::finding(
                "repeated-punctuation",
                Kind::Punctuation,
                (first, &tokens[at + run - 1]),
                format!("\u{201c}{}\u{201d} more than once", first.text),
                vec![first.text.clone()],
            ));
        }
        at += run.max(1);
    }
}
