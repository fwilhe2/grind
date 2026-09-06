// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reassembling a `WM_CHAR` surrogate pair — pure, and tested on any host.
//!
//! `WM_CHAR` carries one UTF-16 code unit per message, so a character outside the Basic
//! Multilingual Plane — an emoji, most of the rarer CJK ideographs, some IME output — arrives as
//! two consecutive messages, a high surrogate and then a low one, and `char::from_u32` refuses
//! each alone. `win.rs`'s `typed_char` is the one caller: it remembers a pending high half on
//! whichever pane is open (`Pane::surrogate_mut`) and asks this module to close the pair the
//! moment the low half arrives, which is the last place either shell needs to know surrogates
//! exist at all.

/// Whether this UTF-16 code unit is the first half of a pair — `0xD800`–`0xDBFF`.
pub fn is_high(unit: u16) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

/// Whether this UTF-16 code unit is the second half of a pair — `0xDC00`–`0xDFFF`.
pub fn is_low(unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

/// The scalar value two surrogate halves encode, or `None` if either half is not what it should
/// be — a lone low surrogate mistaken for the close of a pair, say. The arithmetic is the
/// standard one (UTF-16, §3.9's D91): `char::encode_utf16` run backwards.
pub fn combine(high: u16, low: u16) -> Option<char> {
    if !is_high(high) || !is_low(low) {
        return None;
    }
    let scalar = 0x10000 + (u32::from(high) - 0xD800) * 0x400 + (u32::from(low) - 0xDC00);
    char::from_u32(scalar)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// U+1F600 GRINNING FACE, the example every surrogate-pair explainer reaches for — high
    /// D83D, low DE00, checked against `char::encode_utf16`'s own answer rather than a number
    /// copied from somewhere.
    #[test]
    fn a_pair_combines_to_the_character_it_encodes() {
        let want = '😀';
        let mut units = [0u16; 2];
        want.encode_utf16(&mut units);
        assert!(is_high(units[0]));
        assert!(is_low(units[1]));
        assert_eq!(combine(units[0], units[1]), Some(want));
    }

    #[test]
    fn a_plain_bmp_character_is_neither_half() {
        let mut units = [0u16; 2];
        'A'.encode_utf16(&mut units);
        assert!(!is_high(units[0]));
        assert!(!is_low(units[0]));
    }

    #[test]
    fn two_high_halves_do_not_combine() {
        assert_eq!(combine(0xD83D, 0xD83D), None);
    }

    #[test]
    fn a_low_half_alone_does_not_combine_as_the_high_one() {
        assert_eq!(combine(0xDE00, 0xDE00), None);
    }
}
