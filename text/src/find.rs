// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Walking the hits of a find — the half of "find in a document" that is not a text box.
//!
//! The spreadsheet's twin is `grind_sheet::find::step`, hoisted out of its shells for the same
//! reason: the GNOME window's find bar had its own copy, and the second window to want one (the
//! Windows pane, the browser) would have written a third, and the three would then disagree about
//! where Shift+F3 goes from a caret that is not on a hit.
//!
//! **It ignores case**, through [`App::find_ignoring_case`] and not a second matcher: a person
//! typing `appendix` is looking for *Appendix*. [`App::replace`] stays exact, since it writes.

use crate::{App, Caret};

/// Which way a step goes: `Here` is "the first hit at or after where I am", which is what typing
/// into a find box wants — the hit under the selection stays selected as the word grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Towards {
    Here,
    Next,
    Previous,
}

/// Where every occurrence of `needle` starts, in document order. Asked for again at every step
/// by every shell, since the document may have been edited between two presses of Enter.
pub fn hits(app: &App, needle: &str) -> Vec<Caret> {
    app.find_ignoring_case(needle)
        .into_iter()
        .map(|hit| Caret {
            block: hit.index,
            offset: hit.offset,
        })
        .collect()
}

/// The far end of a hit that starts at `from` — a hit is exactly as many characters as the
/// needle ([`App::find_ignoring_case`] folds a character at a time).
pub fn end_of(from: Caret, needle: &str) -> Caret {
    Caret {
        block: from.block,
        offset: from.offset + needle.chars().count(),
    }
}

/// Which of `hits` (in document order) a step from `at` lands on, wrapping at either end.
/// `None` only when there are none.
pub fn step(hits: &[Caret], at: Caret, towards: Towards) -> Option<usize> {
    if hits.is_empty() {
        return None;
    }
    Some(match towards {
        Towards::Here => hits.iter().position(|hit| *hit >= at).unwrap_or(0),
        Towards::Next => hits.iter().position(|hit| *hit > at).unwrap_or(0),
        Towards::Previous => hits
            .iter()
            .rposition(|hit| *hit < at)
            .unwrap_or(hits.len() - 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(block: usize, offset: usize) -> Caret {
        Caret { block, offset }
    }

    #[test]
    fn a_step_finds_the_next_hit_and_wraps_at_either_end() {
        let hits = [at(0, 4), at(2, 0), at(2, 9)];
        assert_eq!(step(&hits, at(0, 0), Towards::Here), Some(0));
        assert_eq!(
            step(&hits, at(2, 0), Towards::Here),
            Some(1),
            "typing keeps the hit already under the selection"
        );
        assert_eq!(step(&hits, at(2, 0), Towards::Next), Some(2));
        assert_eq!(
            step(&hits, at(2, 9), Towards::Next),
            Some(0),
            "wraps forward"
        );
        assert_eq!(step(&hits, at(2, 0), Towards::Previous), Some(0));
        assert_eq!(
            step(&hits, at(0, 4), Towards::Previous),
            Some(2),
            "wraps back"
        );
        assert_eq!(step(&[], at(0, 0), Towards::Next), None);
    }

    #[test]
    fn hits_ignore_case_and_end_where_the_needle_does() {
        let app = App::new();
        app.insert(0, crate::BlockKind::Paragraph, "Appendix and appendix")
            .unwrap();
        let found = hits(&app, "APPENDIX");
        assert_eq!(found.len(), 2);
        assert_eq!(end_of(found[1], "APPENDIX").offset, found[1].offset + 8);
    }
}
