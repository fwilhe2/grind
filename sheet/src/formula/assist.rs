// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a shell needs to help somebody *while they are typing a formula*: which word could be
//! completed, what to offer for it, and which call the caret is inside.
//!
//! All three are questions about **display syntax and a caret offset**, which is why they are
//! here rather than in a shell. They were `ui_sheet_gtk`'s — `formula_ux::prefix_at`,
//! `formula_ux::candidates` and `state::call_at` — until the Windows shell wanted the same three
//! answers, and a second hand-written backwards scan over parentheses and string literals is
//! exactly the kind of near-copy this project moves into the core instead (the precedent is
//! `grind_core::search::score`, which came out of `ui_web` when a second shell wanted the same
//! ranking). Both shells now read one answer, so a signature hint cannot say `SUM`'s second
//! argument in one window and its third in another.
//!
//! Nothing here is a *capability* the CLI is missing (rule 4): what these functions offer comes
//! entirely from [`funcs::catalog`] and [`friendly::signature`], both of which `grind sheet
//! functions --long` already prints. What is added is the caret — where the user is — and a caret
//! is a UI's own state, not a document's.
//!
//! Pure, allocating only what it returns, and tested here rather than in any shell.

use std::ops::Range;

use super::{friendly, funcs};

/// One offer in a completion list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// What gets inserted, `(` included for a function — so accepting an offer leaves the caret
    /// where the first argument goes.
    pub insert: String,
    /// What the list shows: `SUM`, or the defined name as the document spells it.
    pub name: String,
    /// One line about it — the spec's own `Summary:` for a function.
    pub detail: String,
}

/// The identifier being typed at `caret`, if it is one an offer could replace.
///
/// A run of name characters that starts where a *function* could start — after `=`, `(`, `;` or
/// an operator — and is not already a call. Pure, and the reason a completion popup has no
/// opinion of its own about what a word is.
///
/// `caret` is a **byte** offset, clamped rather than trusted: a shell converts from whatever its
/// own control counts in (UTF-16 units, on Windows), and an offset one past the end is a
/// rounding error rather than a reason to panic.
pub fn prefix_at(text: &str, caret: usize) -> Option<Range<usize>> {
    if !text.starts_with('=') {
        return None;
    }
    let caret = caret.min(text.len());
    // Not a character boundary — a shell that converted from UTF-16 badly, or an offset into the
    // middle of a multi-byte character. Nothing to complete rather than a panic on the slice.
    if !text.is_char_boundary(caret) {
        return None;
    }
    let start = text[..caret]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_name_char(*c))
        .last()
        .map(|(i, _)| i)?;
    if start == 0 {
        return None;
    }
    // Already a call: `SUM(` is finished business, and so is `A1` — a cell address is not a
    // function name, and offering to turn one into a call would be wrong twice over.
    if text[caret..].starts_with('(') {
        return None;
    }
    let before = text[..start].trim_end().chars().next_back()?;
    "=(;+-*/^&<>:,".contains(before).then_some(start..caret)
}

/// Everything worth offering for `prefix`: functions first, then the document's own names.
///
/// Neither list is written here — the functions are [`funcs::catalog`]'s, so a shell cannot offer
/// one the evaluator does not have, and the names are the caller's read of `App::names`.
pub fn candidates(prefix: &str, names: &[String]) -> Vec<Candidate> {
    let upper = prefix.to_uppercase();
    let functions = funcs::catalog()
        .iter()
        .filter(|info| info.name.starts_with(&upper))
        .map(|info| Candidate {
            insert: format!("{}(", info.name),
            name: info.name.to_owned(),
            detail: info.brief.to_owned(),
        });
    let named = names
        .iter()
        .filter(|name| name.to_uppercase().starts_with(&upper))
        .map(|name| Candidate {
            insert: name.clone(),
            name: name.clone(),
            detail: "defined name".to_owned(),
        });
    functions.chain(named).collect()
}

/// Which call the caret is inside, and which argument of it — what a signature hint shows.
///
/// Scanned backwards over the display text, counting parentheses and skipping string literals,
/// because the caret is where the user is and the call that matters is the innermost one
/// containing it. A finished call (`=SUM(1;2)|`) is not one the caret is inside, and a bare
/// grouping parenthesis is not a call at all.
pub fn call_at(text: &str, caret: usize) -> Option<(String, usize)> {
    let head: Vec<char> = text[..caret.min(text.len())].chars().collect();
    let mut depth = 0i32;
    let mut argument = 0usize;
    let mut i = head.len();
    let mut in_string = false;
    while i > 0 {
        i -= 1;
        match head[i] {
            // Quotes are counted from the left, so a backwards scan flips at every one.
            '"' => in_string = !in_string,
            _ if in_string => {}
            ')' => depth += 1,
            ';' if depth == 0 => argument += 1,
            '(' if depth > 0 => depth -= 1,
            '(' => {
                // The name in front of it, if there is one — otherwise this was a grouping
                // parenthesis and the call, if any, is further out.
                let end = i;
                while i > 0 && is_name_char(head[i - 1]) {
                    i -= 1;
                }
                if i == end {
                    argument = 0;
                    continue;
                }
                return Some((head[i..end].iter().collect(), argument));
            }
            _ => {}
        }
    }
    None
}

/// A function's signature split into a head and one part per parameter — the two spellings a
/// hint can be drawn in, and the only difference between them.
///
/// `friendly` picks: [`friendly::signature`]'s plain-English labels (`Sum`, `Number…`), which are
/// the same names [`friendly::explain`] labels a finished formula with, or the spec's own
/// `Syntax:` line split on `;`, types and brackets and all. Two spellings of one signature rather
/// than two signatures, so what a user reads while typing and what they read afterwards agree.
///
/// `None` for a name the catalog does not know, which is a shell's cue to show nothing rather
/// than to invent a signature.
pub fn signature_parts(name: &str, friendly: bool) -> Option<(String, Vec<String>)> {
    if friendly {
        return friendly::signature(name);
    }
    let info = funcs::catalog()
        .iter()
        .find(|info| info.name.eq_ignore_ascii_case(name))?;
    let (head, rest) = info.signature.split_once('(')?;
    let rest = rest.strip_suffix(')').unwrap_or(rest);
    Some((
        head.trim().to_owned(),
        rest.split(';').map(|part| part.trim().to_owned()).collect(),
    ))
}

/// The one line saying what a function does — the spec's own `Summary:`, for a hint that has
/// room for it.
pub fn brief(name: &str) -> Option<&'static str> {
    funcs::catalog()
        .iter()
        .find(|info| info.name.eq_ignore_ascii_case(name))
        .map(|info| info.brief)
}

/// What a name is made of, in both a function name and a defined one.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_is_a_word_where_a_function_could_start() {
        assert_eq!(prefix_at("=SU", 3), Some(1..3));
        assert_eq!(prefix_at("=SUM(AV", 7), Some(5..7));
        assert_eq!(prefix_at("=1+VLO", 6), Some(3..6));
        // A word, and `B2` is offerable — a defined name can begin that way.
        assert_eq!(prefix_at("=SUM(B2", 7), Some(5..7));
        assert_eq!(prefix_at("SUM", 3), None); // not a formula
        assert_eq!(prefix_at("=SUM(", 5), None); // nothing typed yet
        assert_eq!(prefix_at("=SUM(1;2)", 9), None);
    }

    /// A caret a shell converted badly is nothing to complete, not a panic. `EM_GETSEL` counts
    /// UTF-16 units and this counts bytes, which is exactly where such an offset comes from.
    #[test]
    fn a_caret_off_a_character_boundary_offers_nothing() {
        let text = "=SÜM"; // `Ü` is two bytes, so 3 is inside it
        assert_eq!(prefix_at(text, 3), None);
        assert_eq!(prefix_at(text, 5), Some(1..5));
        assert_eq!(prefix_at(text, 99), Some(1..5), "past the end is the end");
    }

    #[test]
    fn candidates_come_from_the_catalog_and_the_document() {
        let names = vec!["expenses".to_owned(), "excess".to_owned()];
        let offers = candidates("su", &names);
        assert!(offers.iter().any(|c| c.name == "SUM" && c.insert == "SUM("));
        assert!(offers.iter().all(|c| c.name.starts_with("SU")));

        // Names come after functions, and are inserted without a parenthesis.
        let offers = candidates("ex", &names);
        assert_eq!(
            offers.last().map(|c| c.insert.clone()),
            Some("excess".to_owned())
        );
        assert!(offers.iter().any(|c| c.name == "EXP"));
    }

    /// What a signature hint needs: which call the caret is in, and which argument.
    #[test]
    fn the_caret_knows_which_argument_it_is_in() {
        assert_eq!(call_at("=SUM(", 5), Some(("SUM".to_owned(), 0)));
        assert_eq!(call_at("=SUM(1;2", 8), Some(("SUM".to_owned(), 1)));
        assert_eq!(call_at("=SUM(1;2;3", 10), Some(("SUM".to_owned(), 2)));
        // The innermost call wins, and a closed one is not it.
        assert_eq!(call_at("=IF(A1;SUM(1;2", 14), Some(("SUM".to_owned(), 1)));
        assert_eq!(call_at("=IF(A1;SUM(1;2);", 16), Some(("IF".to_owned(), 2)));
        // A `;` inside a string is not an argument separator, and a bare group is no call.
        assert_eq!(call_at("=SUM(\"a;b\";2", 12), Some(("SUM".to_owned(), 1)));
        assert_eq!(call_at("=(1+2", 5), None);
        assert_eq!(call_at("=A1", 3), None);
        // Dollars are part of a reference, not of anything the scan cares about.
        assert_eq!(call_at("=SUM($A$13:$A$15", 16), Some(("SUM".to_owned(), 0)));
    }

    /// The two spellings of one signature, and that they describe the same parameters.
    #[test]
    fn a_signature_comes_in_the_specs_words_and_in_plain_ones() {
        let (head, parts) = signature_parts("VLOOKUP", false).expect("in the catalog");
        assert_eq!(head, "VLOOKUP");
        assert!(parts[0].contains("Lookup"), "{parts:?}");
        assert!(parts.iter().any(|p| p.contains("Column")), "{parts:?}");

        let (head, labels) = signature_parts("vlookup", true).expect("case does not matter");
        assert_eq!(head, "Vertical Lookup");
        assert_eq!(labels.len(), parts.len(), "one label per parameter");

        // A repeating parameter carries the ellipsis in the friendly spelling.
        let (head, labels) = signature_parts("SUM", true).expect("in the catalog");
        assert_eq!((head.as_str(), labels.len()), ("Sum", 1));
        assert!(labels[0].ends_with('\u{2026}'), "{labels:?}");

        assert_eq!(signature_parts("NOSUCHFUNCTION", false), None);
        assert_eq!(signature_parts("NOSUCHFUNCTION", true), None);
    }

    #[test]
    fn a_brief_is_the_specs_own_summary() {
        let summary = brief("sum").expect("in the catalog");
        assert!(summary.to_lowercase().contains("sum"), "{summary}");
        assert_eq!(brief("NOSUCHFUNCTION"), None);
    }
}
