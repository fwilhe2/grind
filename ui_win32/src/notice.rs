// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every sentence the notice bar says, as a pure function.
//!
//! **Portable, and tested on any host**, like [`crate::menu`] and `sheet/state.rs`. A window is
//! not needed to check whether a message reads well, counts correctly or says what to do next,
//! and the plurals are exactly the sort of thing that is wrong for a year in a shell nobody can
//! run on the development machine.
//!
//! The bar is for a **state the document is in**, never for an event that has happened —
//! `ui_sheet_gtk` draws the same line between its banner and its toasts, and the reason is the
//! same: a state stays true until something changes it, so it belongs in a surface that stays.
//! That is why a failed save is a message box here (it is over as soon as it is read) and a
//! recalculation this build refused to perform is a banner (it is true until F9).
//!
//! Every sentence ends by naming the way out, because a notice with no next action is one the
//! reader can only dismiss.

/// `1 thing` / `4 things` — the whole of the pluralisation, in one place.
fn counted(n: usize, singular: &str, plural: &str) -> String {
    match n {
        1 => format!("1 {singular}"),
        n => format!("{n} {plural}"),
    }
}

/// A recalculation that was **not** performed, because performing it would have replaced cached
/// values this build cannot reproduce (`grind_sheet::Recalc::spoiled`).
///
/// The honest shape of the problem: the document uses a function outside the 110 of
/// `doc/small-group.md`, so recalculating would turn a perfectly good saved number into
/// `#NAME?`. Refusing the *edit* would make such a document read-only, which is worse, so the
/// edit commits and this says what was skipped.
pub fn recalc_skipped(spoiled: usize) -> String {
    match spoiled {
        1 => "1 formula uses a function this build does not have — recalculating would replace \
              its saved value. F9 does it anyway; Ctrl+Z takes it back."
            .to_owned(),
        n => format!(
            "{n} formulas use functions this build does not have — recalculating would replace \
             their saved values. F9 does it anyway; Ctrl+Z takes it back."
        ),
    }
}

/// What a recalculation did, once one has been asked for.
pub fn recalculated(changed: usize, spoiled: usize) -> String {
    if spoiled > 0 {
        return format!(
            "{} — Ctrl+Z takes the recalculation back.",
            counted(spoiled, "cell became an error", "cells became errors")
        );
    }
    match changed {
        0 => "Every formula already holds what it computes.".to_owned(),
        n => format!(
            "{}. Ctrl+Z takes it back.",
            counted(n, "cell recalculated", "cells recalculated")
        ),
    }
}

/// A rename that carried references with it — `doc/dsl.md` §6.5's first refactoring, D10.
///
/// Worth saying out loud, because it is the one edit here whose reach is larger than the thing
/// that was edited: renaming a sheet rewrites every formula, named expression and chart range
/// that named it, and a user who did not expect that needs to know it is one Ctrl+Z.
pub fn references_renamed(count: usize) -> String {
    format!(
        "{} rewritten to follow the rename. Ctrl+Z takes it back.",
        counted(count, "reference", "references")
    )
}

/// A formula the parser would not take. The edit stays open, which is what the sentence has to
/// make obvious — otherwise it reads as though the cell had been stored broken.
pub fn bad_formula(message: &str) -> String {
    format!("Not a formula: {message}. Esc leaves the cell as it was.")
}

/// A CSV that landed. Says **where**, because an import goes to the cursor rather than to the
/// top-left corner, and somebody who meant the other one needs to see that in one glance.
pub fn imported(cells: usize, at: &str) -> String {
    format!(
        "{} imported at {at}. Ctrl+Z takes it back.",
        counted(cells, "cell", "cells")
    )
}

/// A range that went out as one. The odd sentence in this file: it names no way out because
/// there is nothing to undo — exporting writes a *file* and leaves the document exactly as it
/// was, which is the thing worth saying about it.
pub fn exported(start: &str, end: &str, name: &str) -> String {
    format!("{start}:{end} exported to {name}. The document is unchanged.")
}

/// Data ▸ Explain Formula, asked about a cell that holds no formula — or one this build cannot
/// parse, which from the reader's side is the same thing: there is no reading to show.
///
/// A notice rather than a message box, because the question was asked *about a cell* and this bar
/// is where this window says things about the cell it is on.
pub fn nothing_to_explain(address: &str) -> String {
    format!("{address} holds no formula to explain. F2 opens the cell to write one.")
}

/// Where a find landed: which of how many, and the two keys that go on from here.
pub fn found(index: usize, count: usize, needle: &str) -> String {
    format!(
        "{} of {} holding “{needle}”. F3 finds the next, Shift+F3 the previous.",
        index + 1,
        counted(count, "cell", "cells")
    )
}

/// A find with nothing to land on. Names the way out, which is the word itself.
pub fn not_found(needle: &str) -> String {
    format!("No cell holds “{needle}”. Ctrl+F asks for another word.")
}

/// What a replace did. `refused` is the first formula it left alone and how many it left, when
/// it left any — a cell that matched and did not change is worth a sentence, since otherwise it
/// reads as a replace that missed one.
pub fn replaced(cells: usize, refused: Option<(&str, usize)>) -> String {
    let done = match cells {
        0 => "Nothing replaced".to_owned(),
        n => format!("Replaced in {}", counted(n, "cell", "cells")),
    };
    let left = match refused {
        None => String::new(),
        Some((first, 1)) => format!("; {first} was left alone, since its formula would not parse"),
        Some((first, n)) => format!(
            "; {n} formulas were left alone, since they would not parse — the first is {first}"
        ),
    };
    match cells {
        0 => format!("{done}{left}."),
        _ => format!("{done}{left}. Ctrl+Z takes it back."),
    }
}

/// [`found`] for the word processor: which of how many occurrences the selection is on.
pub fn text_found(index: usize, count: usize, needle: &str) -> String {
    format!(
        "{} of {} of “{needle}”. F3 finds the next, Shift+F3 the previous.",
        index + 1,
        count
    )
}

/// [`not_found`] for the word processor.
pub fn text_not_found(needle: &str) -> String {
    format!("“{needle}” is not in the document. Ctrl+F asks for another word.")
}

/// What a replace did over the page: how many paragraphs changed.
pub fn text_replaced(blocks: usize, needle: &str) -> String {
    match blocks {
        0 => text_not_found(needle),
        n => format!(
            "Replaced in {}. Ctrl+Z takes it back.",
            counted(n, "paragraph", "paragraphs")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_find_says_where_it_is_and_how_to_go_on() {
        assert_eq!(
            found(1, 7, "tax"),
            "2 of 7 cells holding “tax”. F3 finds the next, Shift+F3 the previous."
        );
        assert_eq!(
            found(0, 1, "tax"),
            "1 of 1 cell holding “tax”. F3 finds the next, Shift+F3 the previous."
        );
        assert_eq!(
            not_found("tax"),
            "No cell holds “tax”. Ctrl+F asks for another word."
        );
    }

    #[test]
    fn a_replace_counts_and_names_what_it_left_alone() {
        assert_eq!(
            replaced(3, None),
            "Replaced in 3 cells. Ctrl+Z takes it back."
        );
        assert_eq!(
            replaced(1, Some(("Sheet1.B3", 1))),
            "Replaced in 1 cell; Sheet1.B3 was left alone, since its formula would not parse. \
             Ctrl+Z takes it back."
        );
        assert_eq!(
            replaced(0, Some(("Sheet1.B3", 2))),
            "Nothing replaced; 2 formulas were left alone, since they would not parse — the \
             first is Sheet1.B3."
        );
    }

    #[test]
    fn one_is_singular_and_everything_else_is_not() {
        assert!(recalc_skipped(1).starts_with("1 formula uses a function"));
        assert!(recalc_skipped(3).starts_with("3 formulas use functions"));
        assert!(recalc_skipped(1).contains("its saved value."));
        assert!(recalc_skipped(3).contains("their saved values."));
        assert!(recalculated(1, 0).starts_with("1 cell recalculated."));
        assert!(recalculated(7, 0).starts_with("7 cells recalculated."));
        assert!(recalculated(9, 1).starts_with("1 cell became an error"));
        assert!(recalculated(9, 4).starts_with("4 cells became errors"));
        assert!(references_renamed(1).starts_with("1 reference rewritten"));
        assert!(references_renamed(9).starts_with("9 references rewritten"));
        assert!(imported(1, "A1").starts_with("1 cell imported at A1."));
        assert!(imported(84, "B3").starts_with("84 cells imported at B3."));
    }

    /// Exporting is the one verb here that writes a file and not the document, and the notice
    /// is where that is said: there is nothing to undo, so the sentence that would name Ctrl+Z
    /// names the absence instead. This is why the rule below has to allow it by name.
    #[test]
    fn an_export_says_the_document_was_not_touched() {
        assert_eq!(
            exported("A1", "D20", "budget.csv"),
            "A1:D20 exported to budget.csv. The document is unchanged."
        );
    }

    /// Nothing to report is still a sentence: F9 on an up-to-date document must not look like a
    /// key that did nothing. It is also the **one** notice with no action in it, which is why
    /// the rule below has to name it rather than test for it.
    #[test]
    fn a_recalculation_that_changed_nothing_still_says_so() {
        assert_eq!(
            recalculated(0, 0),
            "Every formula already holds what it computes."
        );
    }

    /// Every notice names the key that resolves it, which is the rule this module exists to
    /// keep — a banner the reader can only stare at is one they learn to ignore.
    ///
    /// The two exceptions are above and each has a test of its own: a recalculation that
    /// changed nothing, and an export, which leaves nothing behind to act on.
    #[test]
    fn every_notice_names_the_way_out() {
        for text in [
            recalc_skipped(2),
            recalculated(3, 0),
            recalculated(3, 1),
            bad_formula("unexpected end of input"),
            references_renamed(4),
            nothing_to_explain("B3"),
            imported(12, "A1"),
        ] {
            assert!(
                text.contains("F9")
                    || text.contains("Ctrl+Z")
                    || text.contains("Esc")
                    || text.contains("F2"),
                "{text}"
            );
            assert!(text.ends_with('.'), "{text}");
        }
    }

    #[test]
    fn a_broken_formula_says_the_cell_is_untouched() {
        let text = bad_formula("unexpected end of input");
        assert!(
            text.starts_with("Not a formula: unexpected end of input"),
            "{text}"
        );
        assert!(text.contains("leaves the cell as it was"), "{text}");
    }

    #[test]
    fn the_pages_find_sentences_name_the_word_and_the_way_out() {
        assert!(text_found(0, 3, "tax").starts_with("1 of 3 of “tax”"));
        assert!(text_not_found("tax").contains("Ctrl+F"));
        assert!(text_replaced(1, "tax").contains("1 paragraph."));
        assert_eq!(text_replaced(0, "tax"), text_not_found("tax"));
    }
}
