// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Ranking a list of short labels against something a person is typing — the command palettes in
//! the browser and the GNOME window share it, so a query ranks the same in both.

/// How well `needle` matches `haystack`, and where: a case-insensitive subsequence match that
/// scores a letter starting a word above one inside a word, and adjacent letters above scattered
/// ones. Positions are in `char`s. `None` when a letter of the needle is missing. Deliberately
/// stable, so the first row does not jump as more is typed.
pub fn score(haystack: &str, needle: &str) -> Option<(i32, Vec<usize>)> {
    let hay: Vec<char> = haystack.chars().flat_map(char::to_lowercase).collect();
    // Word starts, computed once: the beginning, and any letter after a space or a hyphen.
    let boundary = |at: usize| at == 0 || matches!(hay.get(at - 1), Some(' ' | '-' | ':' | '('));

    let mut score = 0;
    let mut hits = Vec::new();
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for want in needle.chars().flat_map(char::to_lowercase) {
        if want == ' ' {
            continue;
        }
        let found = hay[at..].iter().position(|c| *c == want)? + at;
        score += match () {
            // `checked_sub`, because the first character of the haystack is position zero and
            // `found - 1` there is not a position at all.
            _ if found.checked_sub(1) == previous && previous.is_some() => 6,
            _ if boundary(found) => 5,
            _ => 1,
        };
        hits.push(found);
        previous = Some(found);
        at = found + 1;
    }
    // A short title that matched is a better answer than a long one that also did: "Bold"
    // should beat "Bold the selection's borders" for `bol`.
    score = score * 100 - hay.len() as i32;
    Some((score, hits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_letter_at_the_start_of_a_word_beats_one_in_the_middle() {
        let (start, _) = score("Align left", "al").unwrap();
        let (middle, _) = score("Recalculate", "al").unwrap();
        assert!(start > middle, "{start} vs {middle}");
    }

    /// Adjacent letters outrank a scattering, which is what makes a typed prefix win.
    #[test]
    fn a_run_beats_a_scattering() {
        let (run, _) = score("Recalculate", "rec").unwrap();
        let (scattered, _) = score("Remove borders and clear", "rec").unwrap();
        assert!(run > scattered, "{run} vs {scattered}");
    }

    #[test]
    fn a_letter_that_is_not_there_matches_nothing() {
        assert_eq!(score("Bold", "bz"), None);
    }

    #[test]
    fn the_matched_letters_come_back_so_they_can_be_shown() {
        let (_, hits) = score("Align left", "alle").unwrap();
        assert_eq!(hits, vec![0, 1, 6, 7]);
    }

    /// Positions are `char`s, not bytes, because a palette slices its title by `char`s.
    #[test]
    fn positions_count_characters_rather_than_bytes() {
        let (_, hits) = score("Größe ändern", "än").unwrap();
        assert_eq!(hits, vec![6, 7]);
    }

    /// An empty needle matches everything identically, so a palette must not use it to order rows.
    #[test]
    fn an_empty_needle_says_nothing_about_order() {
        let (a, hits) = score("Bold", "").unwrap();
        let (b, _) = score("Bold", "").unwrap();
        assert_eq!(a, b);
        assert!(hits.is_empty());
    }
}
