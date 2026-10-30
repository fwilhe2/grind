// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A paragraph cut into words, spaces and single marks — the words spelling's, so `don't` and
//! `E-Mail` are one word here too.

use std::ops::Range;

use grind_core::grammar::{Finding, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Word,
    Space,
    /// One character that is neither: punctuation, a symbol, a line break.
    Mark,
    /// A range the paragraph says is not prose — a run of code, a link — taken whole, so no rule
    /// reads inside it or matches it as a word.
    Opaque,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Characters into the paragraph.
    pub range: Range<usize>,
    pub sort: Sort,
    pub text: String,
}

impl Token {
    pub fn is_word(&self) -> bool {
        self.sort == Sort::Word
    }

    /// One or more spaces and nothing else — not a line break or a tab.
    pub fn is_plain_space(&self) -> bool {
        self.sort == Sort::Space && self.text.chars().all(|c| c == ' ')
    }

    pub fn lower(&self) -> String {
        self.text.to_lowercase()
    }
}

pub fn tokens(text: &str, skip: &[Range<usize>]) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let skipped = |range: &Range<usize>| {
        skip.iter()
            .any(|s| s.start < range.end && range.start < s.end)
    };
    let words: Vec<Range<usize>> = grind_core::spell::words(text)
        .into_iter()
        .filter(|word| !skipped(word))
        .collect();
    let mut out = Vec::new();
    let mut at = 0;
    let mut next_word = words.iter().peekable();
    let piece = |range: Range<usize>, sort| Token {
        text: chars[range.clone()].iter().collect(),
        range,
        sort,
    };
    while at < chars.len() {
        if let Some(opaque) = skip.iter().find(|s| s.start <= at && at < s.end) {
            let end = opaque.end.min(chars.len());
            out.push(piece(at..end, Sort::Opaque));
            at = end;
            continue;
        }
        if let Some(word) = next_word.peek()
            && word.start == at
        {
            out.push(piece((*word).clone(), Sort::Word));
            at = word.end;
            next_word.next();
            continue;
        }
        if chars[at].is_whitespace() && chars[at] != '\n' {
            let from = at;
            while at < chars.len() && chars[at].is_whitespace() && chars[at] != '\n' {
                at += 1;
            }
            out.push(piece(from..at, Sort::Space));
            continue;
        }
        out.push(piece(at..at + 1, Sort::Mark));
        at += 1;
    }
    out
}

/// The index of the word after `tokens[at]`, when one or two plain spaces are all between.
pub fn next_word(tokens: &[Token], at: usize) -> Option<usize> {
    let space = tokens.get(at + 1)?;
    let word = tokens.get(at + 2)?;
    (space.is_plain_space() && space.range.len() <= 2 && word.is_word()).then_some(at + 2)
}

/// The index of the word before `tokens[at]`, the same way.
pub fn previous_word(tokens: &[Token], at: usize) -> Option<usize> {
    let space = tokens.get(at.checked_sub(1)?)?;
    let word = tokens.get(at.checked_sub(2)?)?;
    (space.is_plain_space() && space.range.len() <= 2 && word.is_word()).then_some(at - 2)
}

/// Whether `tokens[at]` starts a sentence: nothing but spaces and opening marks before it back
/// to the paragraph's start or a sentence's end.
pub fn starts_sentence(tokens: &[Token], at: usize) -> bool {
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
pub fn cased(word: &str, like: &str) -> String {
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

/// Each word `first` followed by `second`, as `(index of first, index of second)`, both
/// compared in lowercase.
pub fn pairs<'a>(
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

pub fn replace(rule: &str, token: &Token, with: &str, message: &str, out: &mut Vec<Finding>) {
    out.push(crate::finding(
        rule,
        Kind::Grammar,
        (token, token),
        message.to_owned(),
        vec![cased(with, &token.text)],
    ));
}
