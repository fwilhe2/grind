// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Find and replace over cells — what a match is, with no document in sight.
//!
//! A cell is searched through its input text ([`crate::App::input_text`]): a formula in display
//! form, a date in ISO, a number in the document's own spelling. So `SUM` finds `=SUM(B2:B9)` and
//! `2026` finds a typed date, and a replace goes back through the typing rule, so it can put
//! nothing in a cell typing could not.
//!
//! Case is folded one character at a time to its first lowercase character, as
//! `grind_text::App::find_ignoring_case` does, so a hit is as many characters as the needle and
//! `ß` does not match `SS`.

use std::ops::Range;

use crate::Pos;

/// What to look for, and where.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Search {
    pub needle: String,
    /// `Total` does not find `total`. Off by default: nobody searching a spreadsheet means it.
    pub match_case: bool,
    /// The needle must be the cell's whole input text — so `0` finds the zeros, not every number
    /// with a nought in it.
    pub whole_cell: bool,
    /// One sheet, or `None` for every sheet in the document.
    pub sheet: Option<usize>,
    /// Only the cells inside this rectangle, corners included, on whichever sheets are searched —
    /// how a replace stays inside a selection. `None` is the whole sheet.
    pub range: Option<(Pos, Pos)>,
}

impl Search {
    /// Every sheet, ignoring case, anywhere in a cell.
    pub fn new(needle: impl Into<String>) -> Self {
        Search {
            needle: needle.into(),
            ..Search::default()
        }
    }

    /// The byte ranges of `haystack` this search matches, left to right, never overlapping. An
    /// empty needle matches nothing.
    pub fn occurrences(&self, haystack: &str) -> Vec<Range<usize>> {
        if self.needle.is_empty() {
            return Vec::new();
        }
        let fold = |c: char| match self.match_case {
            true => c,
            false => c.to_lowercase().next().unwrap_or(c),
        };
        let needle: Vec<char> = self.needle.chars().map(fold).collect();
        let chars: Vec<(usize, char)> =
            haystack.char_indices().map(|(i, c)| (i, fold(c))).collect();
        let end_of = |i: usize| chars.get(i).map_or(haystack.len(), |(at, _)| *at);
        if self.whole_cell {
            return match chars.len() == needle.len()
                && chars.iter().zip(&needle).all(|((_, a), b)| a == b)
            {
                true => std::iter::once(0..haystack.len()).collect(),
                false => Vec::new(),
            };
        }
        let mut hits = Vec::new();
        let mut at = 0;
        while at + needle.len() <= chars.len() {
            match chars[at..at + needle.len()]
                .iter()
                .zip(&needle)
                .all(|((_, a), b)| a == b)
            {
                true => {
                    hits.push(chars[at].0..end_of(at + needle.len()));
                    at += needle.len();
                }
                false => at += 1,
            }
        }
        hits
    }

    /// `haystack` with every occurrence replaced by `with`, or `None` when there is none.
    pub fn replaced(&self, haystack: &str, with: &str) -> Option<String> {
        let hits = self.occurrences(haystack);
        if hits.is_empty() {
            return None;
        }
        let mut out = String::with_capacity(haystack.len());
        let mut from = 0;
        for hit in hits {
            out.push_str(&haystack[from..hit.start]);
            out.push_str(with);
            from = hit.end;
        }
        out.push_str(&haystack[from..]);
        Some(out)
    }
}

/// Which way a step through the hits goes. `Here` is the first hit at or after the cursor, so the
/// hit under it stays as the needle grows; `Next` and `Previous` are Enter and Shift+Enter, F3 and
/// Shift+F3, `n` and `N`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Towards {
    Here,
    Next,
    Previous,
}

/// Which of `hits` a step from `at` lands on, wrapping at either end; `None` only when there are
/// none. `hits` is `(sheet, cell)` in [`crate::App::find`]'s order.
pub fn step(hits: &[(usize, Pos)], at: (usize, Pos), towards: Towards) -> Option<usize> {
    let key = |(sheet, pos): (usize, Pos)| (sheet, pos.row, pos.col);
    let at = key(at);
    if hits.is_empty() {
        return None;
    }
    Some(match towards {
        Towards::Here => hits.iter().position(|hit| key(*hit) >= at).unwrap_or(0),
        Towards::Next => hits.iter().position(|hit| key(*hit) > at).unwrap_or(0),
        Towards::Previous => hits
            .iter()
            .rposition(|hit| key(*hit) < at)
            .unwrap_or(hits.len() - 1),
    })
}

/// One cell a [`Search`] matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub sheet: usize,
    pub sheet_name: String,
    pub pos: Pos,
    /// The cell's input text — what was searched, and what a replace would rewrite.
    pub text: String,
}

impl Hit {
    /// The cell as an address a user can type back in: `Sheet1.B2`, ODF's own spelling.
    pub fn address(&self) -> String {
        format!("{}.{}", self.sheet_name, crate::a1::format(None, self.pos))
    }
}

/// What one [`crate::App::replace`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Replaced {
    /// How many cells were rewritten.
    pub cells: usize,
    /// Cells whose replaced text would be a formula that does not parse, left as they were, with
    /// the reason.
    pub refused: Vec<(Hit, String)>,
    /// The recalculation, when one was asked for; [`crate::EnterOutcome::recalc`]'s meaning.
    pub recalc: Option<crate::Recalc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(needle: &str) -> Search {
        Search::new(needle)
    }

    #[test]
    fn an_empty_needle_matches_nothing() {
        assert!(search("").occurrences("anything").is_empty());
        assert_eq!(search("").replaced("anything", "x"), None);
    }

    #[test]
    fn case_is_ignored_unless_asked_for() {
        assert_eq!(search("sum").occurrences("=SUM(A1)+sum"), vec![1..4, 9..12]);
        let exact = Search {
            match_case: true,
            ..search("sum")
        };
        assert_eq!(exact.occurrences("=SUM(A1)+sum"), vec![9..12]);
    }

    #[test]
    fn matches_do_not_overlap() {
        assert_eq!(search("aa").occurrences("aaaa"), vec![0..2, 2..4]);
        assert_eq!(search("aa").replaced("aaa", "b").as_deref(), Some("ba"));
    }

    #[test]
    fn ranges_are_bytes_in_the_original_text() {
        // `É` is two bytes and folds to `é`, also two: the range lands on the original.
        let text = "Café ÉTÉ";
        let hits = search("été").occurrences(text);
        assert_eq!(hits, vec![6..11]);
        assert_eq!(&text[hits[0].clone()], "ÉTÉ");
        assert_eq!(
            search("été").replaced(text, "hiver").as_deref(),
            Some("Café hiver")
        );
    }

    #[test]
    fn a_step_walks_the_sheets_in_reading_order_and_wraps() {
        let at = |sheet, row, col| (sheet, Pos::new(row, col));
        let hits = [at(0, 0, 3), at(0, 2, 0), at(1, 0, 0)];
        assert_eq!(step(&hits, at(0, 0, 0), Towards::Here), Some(0));
        assert_eq!(
            step(&hits, at(0, 2, 0), Towards::Here),
            Some(1),
            "typing keeps the hit already under the cursor"
        );
        assert_eq!(
            step(&hits, at(0, 2, 0), Towards::Next),
            Some(2),
            "onto sheet two"
        );
        assert_eq!(
            step(&hits, at(1, 0, 0), Towards::Next),
            Some(0),
            "wraps forward"
        );
        assert_eq!(
            step(&hits, at(0, 0, 3), Towards::Previous),
            Some(2),
            "wraps back"
        );
        assert_eq!(
            step(&hits, at(0, 1, 9), Towards::Previous),
            Some(0),
            "a row before is before, whatever its column"
        );
        assert_eq!(step(&[], at(0, 0, 0), Towards::Next), None);
    }

    #[test]
    fn a_whole_cell_search_wants_all_of_it() {
        let whole = Search {
            whole_cell: true,
            ..search("0")
        };
        assert_eq!(whole.occurrences("0"), vec![Range { start: 0, end: 1 }]);
        assert!(whole.occurrences("10").is_empty());
        assert_eq!(whole.replaced("0", "zero").as_deref(), Some("zero"));
    }
}
