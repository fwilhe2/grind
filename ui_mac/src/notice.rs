// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every sentence the notice banner says, as a pure function (M4). Portable, and tested on any
//! host — whether a message reads well, counts right and says what to do next needs no window.
//!
//! The banner, under the titlebar accessory (decision 3), is for **a state the document is in**,
//! never an event: a recalculation this build declined to perform stays true until somebody
//! recalculates, so it stays on screen; an error in a save is over once it is read, and is an
//! alert instead. `ui_win32/src/notice.rs` draws the same line, and says the same things in its
//! own keys. Here every key is spelled ⌘, ⌥, ⇧ or ⌃ — never "Ctrl" — which a test holds.
//!
//! Every sentence ends by naming the way out; a notice with no next action can only be dismissed.

/// `1 thing` / `4 things` — the whole of the pluralisation, in one place.
fn counted(n: usize, singular: &str, plural: &str) -> String {
    match n {
        1 => format!("1 {singular}"),
        n => format!("{n} {plural}"),
    }
}

/// A recalculation that was **not** performed, because performing it would have replaced saved
/// values this build cannot reproduce (`grind_sheet::Recalc::spoiled`). The banner's button is
/// [`RECALCULATE_ANYWAY`]; the edit itself committed, since refusing it would make such a
/// document read-only.
pub fn recalc_skipped(spoiled: usize) -> String {
    format!(
        "{} this build does not have — recalculating would replace {} saved {}.",
        match spoiled {
            1 => "1 formula uses a function".to_owned(),
            n => format!("{n} formulas use functions"),
        },
        if spoiled == 1 { "its" } else { "their" },
        if spoiled == 1 { "value" } else { "values" },
    )
}

/// The banner's button when a recalculation was skipped.
pub const RECALCULATE_ANYWAY: &str = "Recalculate Anyway";

/// What a recalculation asked for did.
pub fn recalculated(changed: usize, spoiled: usize) -> String {
    match (changed, spoiled) {
        (0, 0) => "Every formula was already up to date.".to_owned(),
        (_, 0) => format!(
            "{} recalculated. ⌘Z takes it back.",
            counted(changed, "cell", "cells")
        ),
        (_, spoiled) => format!(
            "{} — ⌘Z takes the recalculation back.",
            counted(spoiled, "cell became an error", "cells became errors")
        ),
    }
}

/// A formula that would not parse. Nothing was stored, the edit is still open with the caret on
/// the problem, and this says both.
pub fn bad_formula(message: &str) -> String {
    format!("That formula does not parse: {message}. Fix it, or press Esc to put the cell back.")
}

/// A sheet renamed, and every reference that named it rewritten with it (`App::rename_sheet`).
pub fn references_renamed(count: usize) -> String {
    format!(
        "{} rewritten to follow the rename. ⌘Z takes it back.",
        counted(count, "reference", "references")
    )
}

/// A sheet that cannot be deleted because it is the only one — a spreadsheet has at least one.
pub fn last_sheet() -> String {
    "A spreadsheet keeps at least one sheet. Add another first, then delete this one.".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_sentence() -> Vec<String> {
        vec![
            recalc_skipped(1),
            recalc_skipped(3),
            recalculated(0, 0),
            recalculated(4, 0),
            recalculated(4, 2),
            bad_formula("expected ) at byte 7"),
            references_renamed(1),
            references_renamed(4),
            last_sheet(),
        ]
    }

    #[test]
    fn one_and_many_read_right() {
        assert_eq!(
            recalc_skipped(1),
            "1 formula uses a function this build does not have — recalculating would replace \
             its saved value."
        );
        assert!(recalc_skipped(3).starts_with("3 formulas use functions"));
        assert!(recalc_skipped(3).ends_with("their saved values."));
        assert!(recalculated(1, 0).starts_with("1 cell recalculated"));
        assert!(references_renamed(4).starts_with("4 references"));
    }

    /// A Mac spells its keys with its own symbols.
    #[test]
    fn every_key_is_spelled_the_mac_way() {
        for sentence in every_sentence() {
            for pc in ["Ctrl", "Alt+", "F9", "Backspace"] {
                assert!(!sentence.contains(pc), "{sentence}");
            }
        }
    }

    #[test]
    fn every_sentence_ends_as_a_sentence() {
        for sentence in every_sentence() {
            assert!(sentence.ends_with('.'), "{sentence}");
            assert!(sentence.starts_with(|c: char| c.is_uppercase() || c.is_ascii_digit()));
        }
    }
}
