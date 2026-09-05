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

#[cfg(test)]
mod tests {
    use super::*;

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
    #[test]
    fn every_notice_names_the_way_out() {
        for text in [
            recalc_skipped(2),
            recalculated(3, 0),
            recalculated(3, 1),
            bad_formula("unexpected end of input"),
            references_renamed(4),
        ] {
            assert!(
                text.contains("F9") || text.contains("Ctrl+Z") || text.contains("Esc"),
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
}
