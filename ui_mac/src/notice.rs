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

/// A defined name renamed, and every use of it respelled with it (`App::rename_name`).
pub fn name_renamed(count: usize) -> String {
    format!(
        "{} rewritten to follow the rename. ⌘Z takes it back.",
        counted(count, "use", "uses")
    )
}

/// A defined name inlined into every use and taken away (`App::inline_name`).
pub fn name_inlined(name: &str, count: usize) -> String {
    format!(
        "{name} written out in {} and removed. ⌘Z takes it back.",
        counted(count, "place", "places")
    )
}

/// A delimited file read in at the cursor (`App::import_csv`).
pub fn imported(cells: usize, at: grind_sheet::Pos) -> String {
    format!(
        "{} imported at {}. ⌘Z takes it back.",
        counted(cells, "cell", "cells"),
        grind_sheet::a1::format(None, at)
    )
}

/// A sheet that cannot be deleted because it is the only one — a spreadsheet has at least one.
pub fn last_sheet() -> String {
    "A spreadsheet keeps at least one sheet. Add another first, then delete this one.".to_owned()
}

/// Where a find landed: which of how many, and the keys that go on from here.
pub fn found(index: usize, count: usize, needle: &str) -> String {
    format!(
        "{} of {} holding “{needle}”. ⌘G finds the next, ⇧⌘G the previous.",
        index + 1,
        counted(count, "cell", "cells")
    )
}

/// A find with nothing to land on. Names the way out, which is the word itself.
pub fn not_found(needle: &str) -> String {
    format!("No cell holds “{needle}”. ⌘F asks for another word.")
}

/// What Replace All did. `refused` is the first formula it left alone and how many it left —
/// a cell that matched and did not change is worth a sentence, or it reads as a replace that
/// missed one.
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
        _ => format!("{done}{left}. ⌘Z takes it back."),
    }
}

/// A page's counts, for its window's subtitle — what every word processor keeps in sight — and,
/// once there is something to read, how long it takes to.
pub fn counted_words(counts: &grind_text::Counts) -> String {
    let counted = format!(
        "{}, {}",
        counted(counts.words, "word", "words"),
        counted(counts.characters, "character", "characters")
    );
    match counts.reading_minutes() {
        0 => counted,
        minutes => format!("{counted} · {minutes} min read"),
    }
}

/// A find on a page that landed: which of how many, and how to go on.
pub fn found_text(index: usize, count: usize, needle: &str) -> String {
    format!(
        "{} of {} of “{needle}”. ⌘G finds the next, ⇧⌘G the previous.",
        index + 1,
        counted(count, "occurrence", "occurrences")
    )
}

/// A find on a page with nothing to land on.
pub fn not_found_text(needle: &str) -> String {
    format!("Nothing in this document reads “{needle}”. ⌘F asks for another word.")
}

/// What Replace All did on a page — one ⌘Z for every one of them.
pub fn replaced_text(count: usize) -> String {
    match count {
        0 => "Nothing replaced.".to_owned(),
        n => format!(
            "Replaced {}. ⌘Z takes it back.",
            counted(n, "occurrence", "occurrences")
        ),
    }
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
            found(0, 3, "rent"),
            not_found("rent"),
            replaced(0, None),
            replaced(4, Some(("B7", 2))),
            found_text(1, 3, "rent"),
            not_found_text("rent"),
            replaced_text(0),
            replaced_text(1),
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
        assert_eq!(
            found_text(1, 3, "rent"),
            "2 of 3 occurrences of “rent”. ⌘G finds the next, ⇧⌘G the previous."
        );
        assert_eq!(replaced_text(1), "Replaced 1 occurrence. ⌘Z takes it back.");
        let note = grind_text::Counts {
            words: 1,
            characters: 5,
            ..Default::default()
        };
        assert_eq!(counted_words(&note), "1 word, 5 characters · 1 min read");
        assert_eq!(
            counted_words(&grind_text::Counts::default()),
            "0 words, 0 characters"
        );
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
