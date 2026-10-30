// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The rules for English. Each one looks at a word and the word after it, a single space
//! apart, and finds only the pairs that are wrong whatever the sentence around them says.

use grind_core::grammar::{Finding, Kind};

use crate::tokens::{Sort, Token};

/// The index of the word after `tokens[at]`, when one or two plain spaces are all between.
fn next_word(tokens: &[Token], at: usize) -> Option<usize> {
    let space = tokens.get(at + 1)?;
    let word = tokens.get(at + 2)?;
    (space.is_plain_space() && space.range.len() <= 2 && word.is_word()).then_some(at + 2)
}

/// The index of the word before `tokens[at]`, the same way.
fn previous_word(tokens: &[Token], at: usize) -> Option<usize> {
    let space = tokens.get(at.checked_sub(1)?)?;
    let word = tokens.get(at.checked_sub(2)?)?;
    (space.is_plain_space() && space.range.len() <= 2 && word.is_word()).then_some(at - 2)
}

/// Whether `tokens[at]` starts a sentence: nothing but spaces and opening marks before it back
/// to the paragraph's start or a sentence's end.
fn starts_sentence(tokens: &[Token], at: usize) -> bool {
    for token in tokens[..at].iter().rev() {
        match token.sort {
            Sort::Space => continue,
            Sort::Mark if matches!(token.text.as_str(), "(" | "\u{201c}" | "\u{2018}" | "\"") => {
                continue;
            }
            Sort::Mark => return matches!(token.text.as_str(), "." | "!" | "?" | "\n"),
            Sort::Word | Sort::Opaque => return false,
        }
    }
    true
}

/// `word` with its first letter as `like`'s: `An` for `an` after `A`.
fn cased(word: &str, like: &str) -> String {
    match like.chars().next().is_some_and(char::is_uppercase) {
        true => {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
        false => word.to_owned(),
    }
}

/// Whether `word` starts with a vowel sound, or `None` where the spelling does not say.
fn vowel_sound(word: &str) -> Option<bool> {
    let letters: Vec<char> = word.chars().collect();
    if letters.len() < 2 || !letters.iter().all(|c| c.is_alphabetic()) {
        return None;
    }
    // An abbreviation is said as letters (`an FOI`) or as a word (`a MAPPA meeting`), and its
    // spelling does not say which.
    if letters.iter().all(|c| c.is_uppercase()) {
        return None;
    }
    let lower = word.to_lowercase();
    // A word with no vowel is said as letters (`an rng`), like an abbreviation.
    if !lower.chars().any(|c| "aeiouy".contains(c)) {
        return None;
    }
    let starts = |prefixes: &[&str]| prefixes.iter().any(|p| lower.starts_with(p));
    // `un-` before a word (`unimportant`, `uninformed`) against `uni-` (`unicode`, `unit`).
    if starts(&["unim", "unin"]) {
        return Some(true);
    }
    if starts(&["unid"]) {
        return None;
    }
    if starts(&["hour", "honest", "honour", "honor", "heir"]) {
        return Some(true);
    }
    // `an historic` and `an hotel` are older usage, not mistakes.
    if starts(&["hist", "hotel", "hero", "herb", "x"]) {
        return None;
    }
    if starts(&[
        "one", "once", "eu", "ewe", "uni", "use", "usu", "uti", "ura", "ure", "uro", "ubiq",
    ]) {
        return Some(false);
    }
    Some(matches!(
        letters[0].to_ascii_lowercase(),
        'a' | 'e' | 'i' | 'o' | 'u'
    ))
}

/// `a apple`, `an test`: the article by the sound of the next word. A capital `A` counts only
/// at a sentence's start (`vitamin A and` is not an article), and a word an article cannot come
/// before (`option a or b`) means it is not one.
pub fn a_an(tokens: &[Token], out: &mut Vec<Finding>) {
    const NOT_AFTER_ARTICLE: &[&str] = &[
        "or", "and", "as", "at", "in", "is", "it", "of", "on", "if", "so", "to", "by", "for",
        "nor", "but", "the", "an", "a", "are", "was", "were", "above", "below", "under",
    ];
    for (at, article) in tokens.iter().enumerate() {
        let lower = article.text.to_lowercase();
        if !(lower == "a" || lower == "an") {
            continue;
        }
        if article.text.starts_with(char::is_uppercase) && !starts_sentence(tokens, at) {
            continue;
        }
        let Some(next) = next_word(tokens, at).map(|i| &tokens[i]) else {
            continue;
        };
        if NOT_AFTER_ARTICLE.contains(&next.text.to_lowercase().as_str()) {
            continue;
        }
        let Some(vowel) = vowel_sound(&next.text) else {
            continue;
        };
        let wanted = if vowel { "an" } else { "a" };
        if lower != wanted {
            out.push(crate::finding(
                "a-an",
                Kind::Grammar,
                (article, article),
                format!(
                    "\u{201c}{wanted}\u{201d} before \u{201c}{}\u{201d}",
                    next.text
                ),
                vec![cased(wanted, &article.text)],
            ));
        }
    }
}

/// Each word `first` followed by `second`, as `(index of first, index of second)`, both
/// compared in lowercase.
fn pairs<'a>(
    tokens: &'a [Token],
    first: &'a [&str],
    second: &'a [&str],
) -> impl Iterator<Item = (usize, usize)> + 'a {
    (0..tokens.len()).filter_map(move |at| {
        let word = &tokens[at];
        if !word.is_word() || !first.contains(&word.lower().replace('\u{2019}', "'").as_str()) {
            return None;
        }
        let next = next_word(tokens, at)?;
        second
            .contains(&tokens[next].lower().as_str())
            .then_some((at, next))
    })
}

fn replace(rule: &str, token: &Token, with: &str, message: &str, out: &mut Vec<Finding>) {
    out.push(crate::finding(
        rule,
        Kind::Grammar,
        (token, token),
        message.to_owned(),
        vec![cased(with, &token.text)],
    ));
}

/// `could of` for `could have` — but not `could of course`.
pub fn could_of(tokens: &[Token], out: &mut Vec<Finding>) {
    const MODALS: &[&str] = &["could", "should", "would", "must", "might"];
    for (_, of) in pairs(tokens, MODALS, &["of"]) {
        let after = next_word(tokens, of).map(|i| tokens[i].lower());
        if matches!(after.as_deref(), Some("course" | "necessity")) {
            continue;
        }
        replace(
            "could-of",
            &tokens[of],
            "have",
            "\u{201c}have\u{201d}, not \u{201c}of\u{201d}",
            out,
        );
    }
}

/// `better then` for `better than`.
pub fn than_then(tokens: &[Token], out: &mut Vec<Finding>) {
    const COMPARATIVES: &[&str] = &[
        "better", "worse", "more", "less", "rather", "other", "greater", "fewer", "larger",
        "smaller", "higher", "lower", "bigger", "longer", "shorter", "faster", "slower", "older",
        "younger",
    ];
    for (_, then) in pairs(tokens, COMPARATIVES, &["then"]) {
        replace(
            "than-then",
            &tokens[then],
            "than",
            "A comparison takes \u{201c}than\u{201d}",
            out,
        );
    }
}

/// `its a` for `it's a`, and `it's own` for `its own`.
pub fn its(tokens: &[Token], out: &mut Vec<Finding>) {
    for (word, _) in pairs(
        tokens,
        &["its"],
        &["a", "an", "the", "not", "been", "going"],
    ) {
        replace(
            "its-it's",
            &tokens[word],
            "it's",
            "\u{201c}it's\u{201d}: it is",
            out,
        );
    }
    for (word, _) in pairs(tokens, &["it's"], &["own"]) {
        replace(
            "its-it's",
            &tokens[word],
            "its",
            "\u{201c}its\u{201d}: belonging to it",
            out,
        );
    }
}

/// `your welcome` at a sentence's end, for `you're welcome`.
pub fn your_welcome(tokens: &[Token], out: &mut Vec<Finding>) {
    for (word, welcome) in pairs(tokens, &["your"], &["welcome"]) {
        let ends = tokens
            .get(welcome + 1)
            .is_none_or(|t| matches!(t.text.as_str(), "." | "!" | ","));
        if ends {
            replace(
                "your-you're",
                &tokens[word],
                "you're",
                "\u{201c}you're\u{201d}: you are",
                out,
            );
        }
    }
}

/// `their is` for `there is`.
pub fn their_is(tokens: &[Token], out: &mut Vec<Finding>) {
    for (word, _) in pairs(tokens, &["their"], &["is", "are", "was", "were"]) {
        replace(
            "there-their",
            &tokens[word],
            "there",
            "\u{201c}there\u{201d}, not \u{201c}their\u{201d}",
            out,
        );
    }
}

/// A lone `i` between words, for `I` — not `i.e.`, `(i)` or a label `i)`.
pub fn lowercase_i(tokens: &[Token], out: &mut Vec<Finding>) {
    for (at, token) in tokens.iter().enumerate() {
        if token.text != "i" || next_word(tokens, at).is_none() {
            continue;
        }
        let before_ok = at == 0
            || previous_word(tokens, at).is_some()
            || starts_sentence(tokens, at) && tokens[at - 1].sort == Sort::Space;
        if before_ok {
            replace(
                "lowercase-i",
                token,
                "I",
                "\u{201c}I\u{201d} is a capital",
                out,
            );
        }
    }
}
