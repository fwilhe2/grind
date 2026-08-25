// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Markdown-shaped typing: what `**bold**` and `# ` mean while you write them.
//!
//! **The terminal's answer to a formatting toolbar.** A window has a row of buttons and a
//! selection to press them against; a terminal has a keyboard and a caret. So this shell takes
//! the notation everybody already types when they mean emphasis, and turns it into the
//! document's own formatting as the closing marker lands — the markers are ordinary characters
//! until then, so the caret, the line breaking and the undo history all stay honest. Nothing
//! here is a *display* convention: the document is drawn with the terminal's own bold and
//! italic (`app.rs`), never as source with markers in it, because markers on screen would be
//! characters the core never measured and every caret after one would sit in the wrong column.
//!
//! Pure functions over a block's text, so all of it unit-tests with no terminal attached.
//!
//! **One divergence from markdown, named**: `__x__` is *underline* here, where markdown reads
//! it as a second spelling of bold. ODF has underline and this build already spells bold
//! `**x**`, so the alternative was leaving a property of the model unreachable to spare a
//! notation nobody types twice. `_x_` alone means nothing at all, deliberately —
//! `snake_case` is not emphasis.

use grind_text::BlockKind;
use grind_text::style::CharStyle;

/// The four emphases this notation can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emphasis {
    Bold,
    Italic,
    Underline,
    Strike,
}

impl Emphasis {
    /// The formatting to write, as the character style the core takes.
    pub fn style(self) -> CharStyle {
        let mut style = CharStyle::default();
        match self {
            Emphasis::Bold => style.font_weight = Some("bold".to_owned()),
            Emphasis::Italic => style.font_style = Some("italic".to_owned()),
            Emphasis::Underline => style.underline = Some("solid".to_owned()),
            Emphasis::Strike => style.line_through = Some("solid".to_owned()),
        }
        style
    }

    /// How this emphasis is written — what the status line offers, and what the visual-mode
    /// keys are named after.
    pub fn markers(self) -> &'static str {
        match self {
            Emphasis::Bold => "**",
            Emphasis::Italic => "*",
            Emphasis::Underline => "__",
            Emphasis::Strike => "~~",
        }
    }
}

/// A completed pair of markers, in character offsets within the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Emphasised {
    /// Where the opening marker starts.
    pub open: usize,
    /// Where the content between the markers starts and ends.
    pub start: usize,
    pub end: usize,
    /// Where the closing marker ends — the caret's own position, since the last character of
    /// it is what was just typed.
    pub close: usize,
    pub emphasis: Emphasis,
}

/// Every notation, longest marker first: `**` has to be tried before `*`, or `**x**` would be
/// read as an italic `*x` the moment its third marker landed.
const NOTATION: [(&str, Emphasis); 4] = [
    ("**", Emphasis::Bold),
    ("__", Emphasis::Underline),
    ("~~", Emphasis::Strike),
    ("*", Emphasis::Italic),
];

/// What the character just typed at `caret` completed, if it completed anything.
///
/// `text` is the block as it now is, markers and all — this runs *after* the character is in
/// the document, so what it describes is a span to erase and re-format rather than something
/// to predict.
///
/// The rules that keep ordinary prose out of it:
///
/// - the content between the markers is not empty, and neither end of it is a space —
///   `** **` and `* x *` are not emphasis;
/// - the content holds no marker character of its own, so `2*3*4` is arithmetic;
/// - the opening marker starts a word — it is at the beginning of the block or follows a
///   space or an opening bracket — so `snake_case` and `a*b` are left alone.
pub fn emphasised(text: &str, caret: usize) -> Option<Emphasised> {
    let chars: Vec<char> = text.chars().collect();
    if caret > chars.len() {
        return None;
    }
    for (markers, emphasis) in NOTATION {
        let width = markers.chars().count();
        let marker = markers.chars().next()?;
        // The closing marker has to be what was just typed, sitting immediately behind the
        // caret.
        if caret < width * 2 || chars[caret - width..caret].iter().any(|c| *c != marker) {
            continue;
        }
        // The opening one is the nearest run of the same markers before it.
        let end = caret - width;
        let Some(open) = opening(&chars, end, marker, width) else {
            continue;
        };
        let start = open + width;
        let content = &chars[start..end];
        if content.is_empty()
            || content.first().is_some_and(|c| c.is_whitespace())
            || content.last().is_some_and(|c| c.is_whitespace())
            || content.contains(&marker)
        {
            continue;
        }
        if !starts_a_word(&chars, open) {
            continue;
        }
        return Some(Emphasised {
            open,
            start,
            end,
            close: caret,
            emphasis,
        });
    }
    None
}

/// Where the opening run of `width` copies of `marker` begins, searching back from `end`.
///
/// Requires *exactly* that many: a `***` is not the opening of a `**`, which is what keeps
/// `**x**` from matching as an italic with a stray asterisk in it.
fn opening(chars: &[char], end: usize, marker: char, width: usize) -> Option<usize> {
    let mut at = end;
    while at >= width {
        let start = at - width;
        if chars[start..at].iter().all(|c| *c == marker)
            && chars.get(start.wrapping_sub(1)).copied() != Some(marker)
            && chars.get(at).copied() != Some(marker)
        {
            return Some(start);
        }
        at -= 1;
    }
    None
}

/// Whether a marker at `at` starts a word rather than sitting inside one.
fn starts_a_word(chars: &[char], at: usize) -> bool {
    match at.checked_sub(1).map(|before| chars[before]) {
        None => true,
        Some(c) => c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\'' | '—' | '-'),
    }
}

/// The block kind a line-opening prefix asks for — `# `, `## `, `- ` — and how many characters
/// of it to take back out.
///
/// The other half of the same idea: markdown's *block* notation, recognised when its trailing
/// space lands. `#` up to six deep, which is the outline this build carries
/// (`doc/text-core.md`); `-` and `*` both open a list item, as both do in markdown.
pub fn block_prefix(text: &str, caret: usize) -> Option<(usize, BlockKind)> {
    let chars: Vec<char> = text.chars().collect();
    // Only at the very start of a block, and only as the space lands: `a # b` is prose.
    if caret > chars.len() || chars.get(caret.checked_sub(1)?) != Some(&' ') {
        return None;
    }
    let prefix: String = chars[..caret - 1].iter().collect();
    let kind = match prefix.as_str() {
        "-" | "*" | "+" => BlockKind::ListItem { depth: 1 },
        // A run of hashes, and nothing else.
        hashes if !hashes.is_empty() && hashes.chars().all(|c| c == '#') && hashes.len() <= 6 => {
            BlockKind::Heading {
                level: hashes.len() as u32,
            }
        }
        _ => return None,
    };
    Some((caret, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type `text` one character at a time, reporting what the last one completed.
    fn typing(text: &str) -> Option<Emphasised> {
        emphasised(text, text.chars().count())
    }

    #[test]
    fn the_four_notations_complete_on_their_closing_marker() {
        for (source, emphasis) in [
            ("**bold**", Emphasis::Bold),
            ("*italic*", Emphasis::Italic),
            ("__under__", Emphasis::Underline),
            ("~~struck~~", Emphasis::Strike),
        ] {
            let found = typing(source).unwrap_or_else(|| panic!("{source}"));
            assert_eq!(found.emphasis, emphasis, "{source}");
            assert_eq!(found.open, 0, "{source}");
            assert_eq!(
                source.chars().count(),
                found.close,
                "the caret is past the closing marker: {source}"
            );
        }
    }

    /// The ordering rule: `**` is tried before `*`, so a bold's third marker does not read as
    /// an italic with an asterisk inside it.
    #[test]
    fn a_bold_is_not_read_as_an_italic_halfway_through() {
        assert_eq!(typing("**bold*"), None, "three markers is not a pair yet");
        assert_eq!(typing("**bold**").unwrap().emphasis, Emphasis::Bold);
    }

    #[test]
    fn the_content_and_the_text_around_it_come_back_as_offsets() {
        let found = typing("say **this**").unwrap();
        assert_eq!(
            (found.open, found.start, found.end, found.close),
            (4, 6, 10, 12)
        );
    }

    /// The three rules that keep prose out of it.
    #[test]
    fn ordinary_prose_is_not_emphasis() {
        assert_eq!(typing("2*3*4"), None, "arithmetic: the marker is mid-word");
        assert_eq!(
            typing("snake_case_name"),
            None,
            "and `_` alone means nothing"
        );
        assert_eq!(
            typing("a**b**"),
            None,
            "the opening marker must start a word"
        );
        assert_eq!(typing("** **"), None, "empty content");
        assert_eq!(typing("* x *"), None, "content padded with spaces");
        assert_eq!(typing("*a*b*"), None, "a marker inside the content");
    }

    /// `_x_` is deliberately nothing — the divergence the module docs name.
    #[test]
    fn a_single_underscore_is_not_a_notation() {
        assert_eq!(typing("_under_"), None);
        assert_eq!(typing("__under__").unwrap().emphasis, Emphasis::Underline);
    }

    #[test]
    fn a_marker_typed_anywhere_but_the_end_completes_nothing() {
        // The caret is inside the text rather than past the closing marker.
        assert_eq!(emphasised("**bold** and more", 4), None);
    }

    #[test]
    fn a_bracket_or_a_dash_opens_a_word_too() {
        assert!(typing("(**bold**").is_some());
        assert!(typing("—*aside*").is_some());
    }

    #[test]
    fn the_block_prefixes_are_the_outline_and_a_list() {
        assert_eq!(
            block_prefix("# ", 2),
            Some((2, BlockKind::Heading { level: 1 }))
        );
        assert_eq!(
            block_prefix("### ", 4),
            Some((4, BlockKind::Heading { level: 3 }))
        );
        for bullet in ["- ", "* ", "+ "] {
            assert_eq!(
                block_prefix(bullet, 2),
                Some((2, BlockKind::ListItem { depth: 1 })),
                "{bullet}"
            );
        }
    }

    #[test]
    fn a_prefix_only_counts_at_the_start_of_a_block() {
        assert_eq!(block_prefix("a # ", 4), None);
        assert_eq!(
            block_prefix("####### ", 8),
            None,
            "seven is past the outline"
        );
        assert_eq!(block_prefix("#x ", 3), None);
        assert_eq!(block_prefix("#", 1), None, "no space yet");
    }
}
