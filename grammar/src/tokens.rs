// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A paragraph cut into words, spaces and single marks — the words spelling's, so `don't` and
//! `E-Mail` are one word here too.

use std::ops::Range;

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
