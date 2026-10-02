// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Offsets into a string in UTF-16 code units, and back to UTF-8 bytes.**
//!
//! Every string in this suite is Rust's, so every offset the core hands out or takes — a parse
//! error's position, a completion's span, a formula-bar caret — is a **byte** offset into UTF-8.
//! Two platforms' text controls count something else: Win32's `EM_GETSEL`/`EM_SETSEL` count
//! `WCHAR`s and AppKit's `NSRange` counts `unichar`s, both UTF-16 code units. `ä` is two bytes
//! and one unit, and an emoji four bytes and two units, so a shell that passed one where the
//! other was meant would put the caret in the wrong place on the first non-ASCII formula.
//!
//! `ui_win32/src/sheet/state.rs` wrote the conversion first; the macOS shell would have been the
//! second copy (`doc/macos-shell.md`, M1). It is here rather than in `grind-sheet` because
//! nothing about it is a spreadsheet's, and the word processor's shells ask the same question of
//! a block's text.

/// How many UTF-16 code units come before `byte` in `text` — where a caret goes in a control
/// that counts units, for a byte offset the core reported.
///
/// A byte past the end is the end, and one that falls *inside* a character is that character's
/// own start: an offset from the core is a hint about where to look (a parse error's position,
/// say), not an invariant worth a panic, and there is no place inside a character for a caret.
pub fn units_before(text: &str, byte: usize) -> usize {
    text.char_indices()
        .take_while(|&(at, c)| at + c.len_utf8() <= byte)
        .map(|(_, c)| c.len_utf16())
        .sum()
}

/// The byte offset of the place `units` UTF-16 code units into `text` — where a control's caret
/// is, as the core counts it.
///
/// A unit past the end is the end, and one that falls *inside* a surrogate pair lands on the
/// character it is half of rather than between its bytes, for the same reason
/// [`units_before`] snaps: a caret is a place in text, and a character has no inside.
pub fn byte_of(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (byte, c) in text.char_indices() {
        // Past it, or *inside* it: either way this character's own start is the answer.
        if seen + c.len_utf16() > units {
            return byte;
        }
        seen += c.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_offset_is_counted_in_units() {
        assert_eq!(units_before("=SUM(B2", 5), 5, "ASCII: the same number");
        assert_eq!(units_before("=ä+1", 3), 2, "`ä` is two bytes and one unit");
        let emoji = "=\u{1f600}&A1";
        assert_eq!(
            units_before(emoji, 5),
            3,
            "an emoji is four bytes and two units"
        );
        assert_eq!(units_before("=A1", 99), 3, "past the end is the end");
        assert_eq!(units_before("", 4), 0);
    }

    /// The case the Windows copy got wrong: a byte in the middle of a character counted the
    /// whole string, which put the caret at the end of the formula rather than on the problem.
    #[test]
    fn a_byte_inside_a_character_is_that_characters_start() {
        assert_eq!(units_before("=ä+1", 2), 1, "the second byte of `ä`");
        assert_eq!(units_before("=\u{1f600}&A1", 3), 1, "inside the emoji");
    }

    /// Every place a caret can really be round-trips; the ones it cannot land on a boundary.
    #[test]
    fn every_place_in_the_text_round_trips() {
        for text in ["=SUM(B2)", "=ä+1", "=\"\u{1f600}\"&A1", "", "\u{1f600}"] {
            for (byte, _) in text
                .char_indices()
                .chain(std::iter::once((text.len(), ' ')))
            {
                let units = units_before(text, byte);
                assert_eq!(byte_of(text, units), byte, "{text:?} at {byte}");
            }
        }
    }

    /// An emoji is one character and *two* units, and the unit between its halves is not a
    /// place — it lands on the character rather than between its bytes.
    #[test]
    fn a_unit_inside_a_surrogate_pair_is_the_characters_start() {
        let text = "=\u{1f600}";
        assert_eq!(byte_of(text, 1), 1);
        assert_eq!(byte_of(text, 2), 1, "between the halves");
        assert_eq!(byte_of(text, 3), text.len());
        assert_eq!(byte_of("=A1", 99), 3, "past the end is the end");
    }
}
