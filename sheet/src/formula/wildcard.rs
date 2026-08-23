// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! §3.4 item 6, `HOST-USE-WILDCARDS` — "wildcards question mark '?' and asterisk '*' are
//! used for character-string comparisons and when searching. Wildcards may be escaped with
//! a tilde '~' character."
//!
//! That is the whole syntax the spec gives, and it is host-defined whether it is on at all.
//! It is on here because LibreOffice has it on by default, and loop B's largest single block
//! of disagreements was `COUNTIF`/`SUMIF` fixtures reading a pattern as a literal.
//!
//! Two neighbouring host properties are settled the same way — by what the oracle does:
//!
//! * `HOST-SEARCH-CRITERIA-MUST-APPLY-TO-WHOLE-CELL` is **true**, LibreOffice's default, so
//!   `"a*"` matches `"abc"` and not `"xabc"`. A pattern is anchored at both ends; the `*`
//!   the user wrote is the only one there is.
//! * `HOST-USE-REGULAR-EXPRESSIONS` is **false**, and mutually exclusive with wildcards in
//!   LibreOffice's own options. Only one of the two can be the reading of `"a.*"`.
//!
//! Case folding matches `eval::compare_text` — `=` is case-insensitive (§6.4.7), so a
//! pattern that reduces to no wildcards at all has to keep agreeing with plain equality.

/// Is this text a *pattern* rather than a literal?
///
/// A lone `~` counts, because escaping is part of the language: `"~*"` has no wildcard left
/// after it is read, and yet it is emphatically not the two-character string `"~*"`.
pub fn is_pattern(text: &str) -> bool {
    text.contains(['*', '?', '~'])
}

#[derive(PartialEq)]
enum Token {
    /// `*` — any run of characters, including none.
    Star,
    /// `?` — exactly one character.
    Any,
    Char(char),
}

fn tokens(pattern: &str) -> Vec<Token> {
    let mut chars = pattern.chars().peekable();
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        out.push(match c {
            '*' => Token::Star,
            '?' => Token::Any,
            // A tilde escapes the character after it — but only a character that would
            // otherwise have meant something. `"a~b"` is three literal characters.
            '~' if matches!(chars.peek(), Some('*' | '?' | '~')) => {
                Token::Char(chars.next().expect("peeked"))
            }
            c => Token::Char(c),
        });
    }
    out
}

fn same(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Does `text`, in its entirety, match `pattern`?
pub fn matches(pattern: &str, text: &str) -> bool {
    let tokens = tokens(pattern);
    let text: Vec<char> = text.chars().collect();
    // Greedy walk with one backtrack point: on a mismatch, the most recent `*` eats one more
    // character and the walk resumes from there. Linear in the common case and quadratic
    // only when a pattern is mostly stars, which no criterion in the corpus is.
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut eaten) = (None, 0usize);
    while t < text.len() {
        match tokens.get(p) {
            Some(Token::Star) => {
                star = Some(p);
                eaten = t;
                p += 1;
            }
            Some(Token::Any) => {
                p += 1;
                t += 1;
            }
            Some(Token::Char(c)) if same(*c, text[t]) => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some(s) => {
                    p = s + 1;
                    eaten += 1;
                    t = eaten;
                }
                None => return false,
            },
        }
    }
    // Whole-cell: the pattern has to be spent too, bar trailing stars matching nothing.
    tokens[p..].iter().all(|token| *token == Token::Star)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_is_anchored_at_both_ends() {
        assert!(matches("a*", "abc"));
        assert!(!matches("a*", "xabc"));
        assert!(matches("*c", "abc"));
        assert!(matches("*b*", "abc"));
        assert!(matches("a?c", "abc"));
        assert!(!matches("a?c", "ac"));
        assert!(!matches("a?c", "abbc"));
        assert!(matches("*", ""));
        assert!(matches("", ""));
        assert!(!matches("", "a"));
    }

    #[test]
    fn a_tilde_escapes_a_wildcard_and_nothing_else() {
        assert!(matches("~*", "*"));
        assert!(!matches("~*", "abc"));
        assert!(matches("~?", "?"));
        assert!(matches("~~", "~"));
        assert!(matches("a~b", "a~b")); // not an escape: `b` is not a wildcard
        assert!(is_pattern("~*"));
        assert!(!is_pattern("plain"));
    }

    #[test]
    fn matching_folds_case_like_the_equality_operator() {
        assert!(matches("A*", "apple"));
        assert!(matches("*PLE", "apple"));
    }

    #[test]
    fn backtracking_finds_a_match_the_greedy_first_pass_misses() {
        // The first `*` must give back the `b` it swallowed.
        assert!(matches("*b*d", "abcd"));
        assert!(matches("a*b*c", "aXbYbZc"));
        assert!(!matches("a*b*c", "aXbYbZ"));
    }
}
