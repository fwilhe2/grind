// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Putting text back into XML safely. **\[GENERIC\]**
//!
//! One function, and it is here rather than in each writer because getting it wrong is the
//! same bug in every document type: a cell holding `<` and a paragraph holding `<` both
//! produce a file nothing can read.

/// Escape a string for XML content or an attribute value, dropping what XML cannot carry.
///
/// Two steps, and the first is the one a plain escaper does not do. **XML 1.0 has no way to
/// represent most control characters** — not literally, and not as a numeric reference either,
/// so `&#1;` is as ill-formed as a raw `\x01`. A document is free to hold one anyway (a user
/// pastes from somewhere strange), and the only choices are to drop it or to write a file no
/// reader will open. Dropping is what §9's tolerance looks like from the writing side.
///
/// Surrogates and the two non-characters `U+FFFE`/`U+FFFF` go for the same reason. Tab,
/// newline and carriage return are the three control characters XML does allow, and stay.
pub fn esc(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| matches!(*c, '\t' | '\n' | '\r' | ' '..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..))
        .collect();
    quick_xml::escape::escape(&clean).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_markup_characters_are_escaped() {
        assert_eq!(esc("a < b & c"), "a &lt; b &amp; c");
        assert_eq!(esc("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn control_characters_are_dropped_rather_than_referenced() {
        // `&#1;` is as ill-formed as a raw \x01, so there is nothing to escape *to*.
        assert_eq!(esc("a\u{1}b"), "ab");
        assert_eq!(esc("a\u{fffe}b"), "ab");
        // The three XML does allow survive.
        assert_eq!(esc("a\tb\nc\rd"), "a\tb\nc\rd");
    }
}
