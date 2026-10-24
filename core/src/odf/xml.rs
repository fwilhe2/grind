// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Raw XML, at the two places a document type cannot avoid it: escaping ([`esc`]) and finding
//! where an element ends ([`element_extent`]). **\[GENERIC\]**

use std::ops::Range;

/// Escape a string for XML content or an attribute value, dropping what XML cannot carry.
///
/// XML 1.0 cannot represent most control characters, literally or as a reference, so they are
/// dropped, as are surrogates and `U+FFFE`/`U+FFFF`. Tab, newline and carriage return stay, the
/// last as `&#13;`, since a literal one is normalised away on reading (XML 1.0 §2.11, §3.3.3).
pub fn esc(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| matches!(*c, '\t' | '\n' | '\r' | ' '..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..))
        .collect();
    quick_xml::escape::escape(clean.as_str()).into_owned()
}

/// The qualified name in an element's start tag, as the document spelled it — `text:p` out of
/// `<text:p …>` — for finding the matching close tag in the raw bytes.
fn qualified_name(start_tag: &[u8]) -> Option<&[u8]> {
    let rest = start_tag.strip_prefix(b"<")?;
    let end = rest
        .iter()
        .position(|c| c.is_ascii_whitespace() || *c == b'/' || *c == b'>')
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Whether `bytes` at `at` is a tag for exactly this element — `<text:p>` but not `<text:page>`.
fn tag_at(bytes: &[u8], at: usize, tag: &[u8]) -> bool {
    if !bytes[at..].starts_with(tag) {
        return false;
    }
    // A qualified name ends where the tag does; without this, `<text:p` matches `<text:page`
    // and the extent runs to the wrong close.
    matches!(
        bytes.get(at + tag.len()),
        Some(c) if c.is_ascii_whitespace() || *c == b'/' || *c == b'>'
    )
}

/// How many bytes of `bytes` are a UTF-8 byte-order mark: 3 or 0. `quick-xml` skips a leading BOM
/// and reports positions from the byte after it, so anything recording where an element was must
/// add this back (`tdf117948_CollapseBeforeShape.ods`).
pub fn bom_len(bytes: &[u8]) -> usize {
    if bytes.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    }
}

/// The full extent of an element in the bytes it was parsed from, given the span of its start tag.
/// **\[GENERIC\]**
///
/// The reader knows where an element began (`Attrs::span`); a writer replacing it in place needs
/// its end, found by scanning that one element. Nesting of the same name is counted. `None` when
/// there is no matching close, and the caller regenerates.
pub fn element_extent(bytes: &[u8], start_tag: Range<usize>) -> Option<Range<usize>> {
    // Self-closed — `<text:p/>` — is the whole element already.
    if bytes.get(start_tag.end.checked_sub(2)?..start_tag.end)? == b"/>" {
        return Some(start_tag);
    }
    let name = qualified_name(bytes.get(start_tag.clone())?)?;
    let open: Vec<u8> = [b"<", name].concat();
    let close: Vec<u8> = [b"</", name].concat();

    let mut depth = 1usize;
    let mut at = start_tag.end;
    while at < bytes.len() {
        if tag_at(bytes, at, &close) {
            depth -= 1;
            if depth == 0 {
                // Past the `>` that ends the close tag.
                let end = bytes[at..].iter().position(|c| *c == b'>')? + at + 1;
                return Some(start_tag.start..end);
            }
            at += close.len();
            continue;
        }
        if tag_at(bytes, at, &open) {
            // A self-closed nested element opens and closes at once.
            let tag_end = bytes[at..].iter().position(|c| *c == b'>')? + at;
            if bytes.get(tag_end - 1) != Some(&b'/') {
                depth += 1;
            }
            at = tag_end + 1;
            continue;
        }
        at += 1;
    }
    None
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
        // The three XML does allow survive — the carriage return as a numeric reference,
        // since a literal one is normalized away on the way back in.
        assert_eq!(esc("a\tb\nc\rd"), "a\tb\nc&#13;d");
    }

    /// `start` is the span of the first start tag in `xml`, found the crude way a test may.
    fn extent(xml: &str) -> Option<(usize, usize)> {
        let bytes = xml.as_bytes();
        let end = xml.find('>')? + 1;
        let range = element_extent(bytes, 0..end)?;
        Some((range.start, range.end))
    }

    // The fixtures use a made-up `x:` vocabulary: this function knows no document type (R8).

    #[test]
    fn a_self_closed_element_is_its_own_extent() {
        let xml = "<x:one/>after";
        assert_eq!(extent(xml), Some((0, 8)));
        assert_eq!(&xml[0..8], "<x:one/>");
    }

    #[test]
    fn an_element_runs_to_its_own_close_tag() {
        let xml = "<x:one>hello</x:one>after";
        let (a, b) = extent(xml).expect("found");
        assert_eq!(&xml[a..b], "<x:one>hello</x:one>");
    }

    /// The bug a bare `starts_with` would have: `<x:one` is a prefix of `<x:oneness`.
    #[test]
    fn a_longer_name_that_starts_the_same_is_not_a_match() {
        let xml = "<x:one>a<x:oneness/>b</x:one>tail";
        let (a, b) = extent(xml).expect("found");
        assert_eq!(&xml[a..b], "<x:one>a<x:oneness/>b</x:one>");
    }

    #[test]
    fn nesting_of_the_same_element_is_counted_rather_than_refused() {
        let xml = "<x:one>a<x:one>b</x:one>c</x:one>tail";
        let (a, b) = extent(xml).expect("found");
        assert_eq!(&xml[a..b], "<x:one>a<x:one>b</x:one>c</x:one>");
    }

    #[test]
    fn an_unclosed_element_is_none_rather_than_a_guess() {
        // Malformed enough that there is no matching close: the caller regenerates instead of
        // splicing tangled bytes.
        assert_eq!(extent("<x:one>never closed"), None);
    }

    #[test]
    fn a_prefix_is_whatever_the_document_used() {
        // §8.1 in reverse: dispatch resolves prefixes, this one has to match the bytes.
        let xml = "<zz:one>text</zz:one>tail";
        let (a, b) = extent(xml).expect("found");
        assert_eq!(&xml[a..b], "<zz:one>text</zz:one>");
    }
}
