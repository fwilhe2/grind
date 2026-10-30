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
    const ENGLISH: &[&str] = &["that", "had", "is", "at"];
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
        if !(first.is_word()
            && space.is_plain_space()
            && space.range.len() <= 2
            && second.is_word())
        {
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
/// before `;:!?`, an ellipsis may stand apart, `.50` is a number, `.xlsx` a name and ` :)` a
/// smiley.
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
            // A smiley, or a ratio set with a space either side (`1825 : 443`).
            ":" => matches!(next_char, Some(')' | '(')) || after.is_some_and(Token::is_plain_space),
            // An ellipsis, a number (`.50`), or a name (`.xlsx`) right after.
            "." => next_char.is_some_and(|c| c == '.' || c.is_alphanumeric()),
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

/// Exactly two spaces between two words. More is a line laid out by hand (a form's `Day
/// Month Year`), and two after a sentence's end is a typing convention, so that one is a style
/// finding (`picky`).
pub fn double_space(tokens: &[Token], picky: bool, out: &mut Vec<Finding>) {
    for (at, window) in tokens.windows(3).enumerate() {
        let [before, space, after] = window else {
            continue;
        };
        if !(space.is_plain_space() && space.range.len() == 2 && after.is_word()) {
            continue;
        }
        let between_words = before.is_word() && !before.text.chars().any(char::is_numeric);
        let after_sentence = matches!(before.text.as_str(), "." | "!" | "?")
            && at > 0
            && tokens[at - 1].is_word()
            && !tokens[at - 1].text.chars().any(char::is_numeric);
        let kind = match (between_words, after_sentence && picky) {
            (true, _) => Kind::Typography,
            (false, true) => Kind::Style,
            (false, false) => continue,
        };
        out.push(crate::finding(
            "double-space",
            kind,
            (space, space),
            "More than one space".to_owned(),
            vec![" ".to_owned()],
        ));
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
        // `formula::trace`: a path, not doubled punctuation.
        let joining =
            at > 0 && tokens[at - 1].is_word() && tokens.get(at + run).is_some_and(Token::is_word);
        if run > 1 && !joining && matches!(first.text.as_str(), "," | ";" | ":" | "!" | "?") {
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

/// The whitespace-delimited chunk around `tokens[at]`, as text.
fn chunk(tokens: &[Token], at: usize) -> String {
    let from = tokens[..at]
        .iter()
        .rposition(|t| t.sort == Sort::Space || t.text == "\n")
        .map_or(0, |i| i + 1);
    let to = tokens[at..]
        .iter()
        .position(|t| t.sort == Sort::Space || t.text == "\n")
        .map_or(tokens.len(), |i| at + i);
    tokens[from..to].iter().map(|t| t.text.as_str()).collect()
}

fn letters_only(word: &str) -> bool {
    word.chars()
        .all(|c| c.is_alphabetic() || c == '\'' || c == '\u{2019}' || c == '-')
}

/// `ended.Then`, `red,green`: no space after a mark between two words. A full stop counts only
/// before a capital and after a word of two letters or more, so `e.g.`, `z.B.`, `lib.rs` and
/// `www.example.org` are left alone; nothing in an address, a path or a number counts.
pub fn missing_space_after(tokens: &[Token], out: &mut Vec<Finding>) {
    for (at, window) in tokens.windows(3).enumerate() {
        let [before, mark, after] = window else {
            continue;
        };
        if !(before.is_word()
            && mark.sort == Sort::Mark
            && after.is_word()
            && letters_only(&before.text)
            && letters_only(&after.text))
        {
            continue;
        }
        let around = chunk(tokens, at + 1);
        if around.contains('/') || around.contains('@') || around.contains('=') {
            continue;
        }
        // A one-letter side is a symbol's subscript or an initial (`LEP,d`, `J.Smith`).
        if before.text.chars().count() < 2 || after.text.chars().count() < 2 {
            continue;
        }
        let starts_upper = after.text.chars().next().is_some_and(char::is_uppercase);
        let all_caps = after.text.chars().all(|c| !c.is_lowercase());
        let wanted = match mark.text.as_str() {
            // `GOV.UK` is a name written so.
            "." => starts_upper && !all_caps,
            // `chart:axis` is a namespace; `Note:This` is not.
            ":" => starts_upper,
            "," | ";" | "!" | "?" => true,
            _ => false,
        };
        if wanted {
            out.push(crate::finding(
                "missing-space-after",
                Kind::Punctuation,
                (mark, mark),
                format!("A space after \u{201c}{}\u{201d}", mark.text),
                vec![format!("{} ", mark.text)],
            ));
        }
    }
}

/// Abbreviations a lowercase word may follow without starting a sentence, by language; all of
/// them when the language is unknown. A one-letter abbreviation (`e.g.`, `z. B.`) needs no list.
fn abbreviation(language: Option<Language>, word: &str) -> bool {
    const ENGLISH: &[&str] = &[
        "etc", "vs", "cf", "approx", "incl", "ca", "fig", "no", "nos", "vol", "pp", "ed", "eds",
        "al", "resp", "esp", "min", "max", "dept", "est", "inc", "ref", "cent", "para", "paras",
        "approx", "dr", "mr", "mrs", "ms", "st",
    ];
    const GERMAN: &[&str] = &[
        "bzw", "usw", "ggf", "vgl", "evtl", "bspw", "inkl", "ca", "nr", "abs", "bzgl", "sog", "ff",
        "zzgl", "ggü", "mind", "max", "min", "allg", "insb", "jh", "str", "tel",
    ];
    let lists: &[&[&str]] = match language {
        Some(Language::English) => &[ENGLISH],
        Some(Language::German) => &[GERMAN],
        _ => &[ENGLISH, GERMAN],
    };
    let word = word.to_lowercase();
    lists.iter().any(|list| list.contains(&word.as_str()))
}

/// `It rained. then`: a lowercase word after a sentence's end. Not after an abbreviation, a
/// number (`3. Oktober`) or an ellipsis, nor a word with a capital inside (`iPhone`).
pub fn sentence_start_lowercase(
    tokens: &[Token],
    language: Option<Language>,
    out: &mut Vec<Finding>,
) {
    for window in tokens.windows(4) {
        let [before, end, space, word] = window else {
            continue;
        };
        if !(before.is_word()
            && matches!(end.text.as_str(), "." | "!" | "?")
            && space.is_plain_space()
            && word.is_word()
            && letters_only(&word.text))
        {
            continue;
        }
        // A word with a full stop straight after it is an abbreviation's start (`e.g.`).
        if word.text.chars().count() < 2 {
            continue;
        }
        let mut chars = word.text.chars();
        let first = chars.next().unwrap_or(' ');
        // A letter with no capital (`ᵖ`, a footnote key) does not start a sentence.
        let has_capital = first.to_uppercase().next() != Some(first);
        if !first.is_lowercase() || !has_capital || chars.any(char::is_uppercase) {
            continue;
        }
        if end.text == "."
            && (before.text.chars().count() == 1
                || before.text.chars().any(char::is_numeric)
                || abbreviation(language, &before.text))
        {
            continue;
        }
        let capital: String = first
            .to_uppercase()
            .chain(word.text.chars().skip(1))
            .collect();
        out.push(crate::finding(
            "sentence-start-lowercase",
            Kind::Grammar,
            (word, word),
            "A sentence starts with a capital".to_owned(),
            vec![capital],
        ));
    }
}

/// A `(` never closed, or a `)` never opened, in one paragraph. Not a list label (`a)`, `1)`
/// starting the paragraph) and not a smiley (`:)`).
pub fn unpaired_bracket(tokens: &[Token], out: &mut Vec<Finding>) {
    let mut open: Vec<&Token> = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        let smiley = at > 0 && matches!(tokens[at - 1].text.as_str(), ":" | ";");
        match token.text.as_str() {
            "(" if !smiley => open.push(token),
            ")" if !smiley => {
                if open.pop().is_some() {
                    continue;
                }
                if !label(tokens, at) {
                    out.push(unpaired(token));
                }
            }
            _ => {}
        }
    }
    out.extend(open.into_iter().map(unpaired));
}

/// Whether the `)` at `at` closes a list label: a word of at most two characters, or a roman
/// numeral, standing on its own — at the paragraph's or a line's start, or after a space.
fn label(tokens: &[Token], at: usize) -> bool {
    let Some(word) = at.checked_sub(1).map(|i| &tokens[i]) else {
        return false;
    };
    let short = word.text.chars().count() <= 2
        || word
            .text
            .chars()
            .all(|c| matches!(c, 'i' | 'v' | 'x' | 'I' | 'V' | 'X'));
    let alone = match at.checked_sub(2).map(|i| &tokens[i]) {
        None => true,
        Some(before) => before.sort == Sort::Space || before.text == "\n",
    };
    word.is_word() && short && alone
}

fn unpaired(token: &Token) -> Finding {
    crate::finding(
        "unpaired-bracket",
        Kind::Punctuation,
        (token, token),
        format!("\u{201c}{}\u{201d} is not paired", token.text),
        Vec::new(),
    )
}
