// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which word a position is in — what a double-click selects.
//!
//! Here rather than in a shell for the reason `format.rs` is: four shells each deciding where a
//! word ends is four answers, and a person who double-clicks `don't` in one window and gets a
//! different selection in another has learned that the suite is four programs.
//!
//! **The rule is deliberately small.** A word is a run of characters that are alphanumeric or
//! an underscore, joined across a single apostrophe between two of them (`don't`, `l'eau`); a
//! run of whitespace is one unit, so a double-click between two words selects the gap rather
//! than nothing; and anything else — punctuation, a symbol — is a unit of one character. That is
//! not UAX #29, which would need a segmentation table this crate does not carry, and it is the
//! same answer for every script `char::is_alphanumeric` knows, which is the Latin, Cyrillic,
//! Greek and CJK a document here is likely to hold. The ceiling is a script written without
//! spaces between words (Thai), where the whole run is one "word".

/// The `[start, end)` character range of the unit containing the character at `offset` in
/// `text` — or the one just before it, when `offset` is at the end, since a click past the last
/// letter of a line means that word.
///
/// An empty `text` answers `(0, 0)`.
pub fn around(text: &str, offset: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let at = offset.min(chars.len() - 1);
    let class = |i: usize| unit(&chars, i);
    let here = class(at);
    if here == Unit::Other {
        return (at, at + 1);
    }
    let mut start = at;
    while start > 0 && class(start - 1) == here {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && class(end) == here {
        end += 1;
    }
    (start, end)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Unit {
    Word,
    Space,
    Other,
}

fn unit(chars: &[char], i: usize) -> Unit {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    match chars[i] {
        c if word(c) => Unit::Word,
        c if c.is_whitespace() => Unit::Space,
        // An apostrophe inside a word is part of it: `don't` is one word to anybody reading it.
        '\'' | '\u{2019}'
            if i > 0 && i + 1 < chars.len() && word(chars[i - 1]) && word(chars[i + 1]) =>
        {
            Unit::Word
        }
        _ => Unit::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(text: &str, offset: usize) -> &str {
        let (start, end) = around(text, offset);
        let from = text
            .char_indices()
            .nth(start)
            .map_or(text.len(), |(b, _)| b);
        let to = text.char_indices().nth(end).map_or(text.len(), |(b, _)| b);
        &text[from..to]
    }

    #[test]
    fn a_word_is_letters_and_digits_and_an_inner_apostrophe() {
        let text = "Don't panic, it's 42_000.";
        assert_eq!(pick(text, 0), "Don't");
        assert_eq!(
            pick(text, 3),
            "Don't",
            "the apostrophe itself is inside the word"
        );
        assert_eq!(pick(text, 8), "panic");
        assert_eq!(pick(text, 19), "42_000");
    }

    #[test]
    fn a_gap_is_one_unit_and_punctuation_is_one_character() {
        let text = "one   two, three";
        assert_eq!(pick(text, 4), "   ");
        assert_eq!(pick(text, 9), ",");
    }

    #[test]
    fn the_end_of_a_line_means_the_word_before_it() {
        assert_eq!(pick("naïve café", 10), "café");
        assert_eq!(pick("naïve café", 99), "café");
        assert_eq!(around("", 3), (0, 0));
    }

    #[test]
    fn a_closing_quote_is_not_an_apostrophe() {
        assert_eq!(pick("'quoted'", 3), "quoted");
        assert_eq!(pick("'quoted'", 0), "'");
    }
}
