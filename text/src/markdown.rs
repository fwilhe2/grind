// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Markdown-shaped typing: what `**bold**` and `# ` mean while you write them — a notation for
//! edits, not a document format; the model stays ODF. [`crate::App::type_markdown`] is the edit,
//! and this decides what one means, in the core so every shell reads `**` the same way.
//! `[label](target)` sets a run's link ([`linked`]).
//!
//! Never a display convention: a document is drawn with the shell's own bold and italic, since
//! markers on screen would be characters the layout never measured. Pure functions over a block's
//! text.
//!
//! One divergence from Markdown: `__x__` is underline here, not a second bold, so ODF's underline
//! is reachable. `_x_` alone means nothing, so `snake_case` is not emphasis.

// TODO: `` `code` `` and ``` were reported not to work in any text shell. Each shell's cause was
// real and is fixed: `grind-web` dropped dead keys (`ui_web/src/text/keymap.rs` now takes text
// from `compositionend`), `grind-text-gtk` neither measured nor drew a family
// (`ui_text_gtk/src/metrics.rs`), and `grind-tui` shows code with `Modifier::DIM` and `pre` in the
// gutter. None of it has been typed on a real keyboard on the reporter's layout — the tests use
// constructed composition events, not dead keys — so ask the reporter before calling this closed.
//
// TODO: LibreOffice may not resolve `monospace`. Measured: `soffice --convert-to` keeps
// `fo:font-family="monospace"` but declares no `style:font-face` for it. Whether it renders
// monospace there is unchecked; if not, put a real face ahead of the generic (XSL-FO list quoting
// applies). A fenced block's `Preformatted Text` is also a style name with no definition here.

use crate::BlockKind;
use crate::style::CharStyle;

/// The five emphases this notation can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emphasis {
    Bold,
    Italic,
    Underline,
    Strike,
    /// `` `code` `` — a monospace family on the run. Not a switch: it sets a family, so turning it
    /// off is "no family of my own".
    Code,
}

/// What `` `code` `` sets a run's family to: the generic, since which monospace face a reader has
/// is theirs.
pub const MONOSPACE: &str = "monospace";

/// The named paragraph style a fenced block gets — ODF's code paragraph, as LibreOffice writes it.
pub const PREFORMATTED: &str = "Preformatted Text";

impl Emphasis {
    /// The formatting to write, as the character style the core takes.
    pub fn style(self) -> CharStyle {
        let mut style = CharStyle::default();
        match self {
            Emphasis::Bold => style.font_weight = Some("bold".to_owned()),
            Emphasis::Italic => style.font_style = Some("italic".to_owned()),
            Emphasis::Underline => style.underline = Some("solid".to_owned()),
            Emphasis::Strike => style.line_through = Some("solid".to_owned()),
            Emphasis::Code => style.font_family = Some(MONOSPACE.to_owned()),
        }
        style
    }

    /// How this emphasis is written — what the status line offers and the visual-mode keys are
    /// named after.
    pub fn markers(self) -> &'static str {
        match self {
            Emphasis::Bold => "**",
            Emphasis::Italic => "*",
            Emphasis::Underline => "__",
            Emphasis::Strike => "~~",
            Emphasis::Code => "`",
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
    /// Where the closing marker ends: the caret, since its last character was just typed.
    pub close: usize,
    pub emphasis: Emphasis,
}

/// Every notation, longest marker first, or `**x**` would read as an italic `*x` at its third
/// marker.
const NOTATION: [(&str, Emphasis); 5] = [
    ("**", Emphasis::Bold),
    ("__", Emphasis::Underline),
    ("~~", Emphasis::Strike),
    ("*", Emphasis::Italic),
    ("`", Emphasis::Code),
];

/// What the character just typed at `caret` completed, if anything. `text` is the block as it now
/// is, markers and all. Ordinary prose is left alone:
///
/// - the content is not empty and does not begin or end with a space (`** **`, `* x *`);
/// - it holds no marker character of its own (`2*3*4`);
/// - the opening marker starts a word — at the block's start or after a space or opening bracket
///   (`snake_case`, `a*b`).
pub fn emphasised(text: &str, caret: usize) -> Option<Emphasised> {
    let chars: Vec<char> = text.chars().collect();
    if caret > chars.len() {
        return None;
    }
    for (markers, emphasis) in NOTATION {
        let width = markers.chars().count();
        let marker = markers.chars().next()?;
        // The closing marker must be what was just typed, right behind the caret.
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

/// Where the opening run of exactly `width` copies of `marker` begins, searching back from `end`:
/// `***` does not open a `**`.
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

/// A completed `[label](target)`, in character offsets within the block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Linked {
    /// Where the `[` is.
    pub open: usize,
    /// Where the label starts and ends — between `[` and `]`.
    pub start: usize,
    pub end: usize,
    /// Where the `)` ends — the caret's own position.
    pub close: usize,
    pub href: String,
}

/// What the `)` just typed at `caret` completed, if it ended a Markdown link. The label is not
/// blank and holds no bracket; the target is not empty and holds no whitespace or parenthesis, so
/// `(see [1](2) above)` closes at the first `)`. `![alt](src)`, an image, is left as typed.
pub fn linked(text: &str, caret: usize) -> Option<Linked> {
    let chars: Vec<char> = text.chars().collect();
    if caret > chars.len() || caret == 0 || chars[caret - 1] != ')' {
        return None;
    }
    // Back over the target to its `(`, which must sit right after a `]`.
    let mut paren = caret - 1;
    loop {
        paren = paren.checked_sub(1)?;
        match chars[paren] {
            '(' => break,
            ')' => return None,
            c if c.is_whitespace() => return None,
            _ => {}
        }
    }
    if paren + 1 == caret - 1 || paren == 0 || chars[paren - 1] != ']' {
        return None;
    }
    let end = paren - 1;
    // And back over the label to its `[`.
    let mut open = end;
    loop {
        open = open.checked_sub(1)?;
        match chars[open] {
            '[' => break,
            ']' => return None,
            _ => {}
        }
    }
    let start = open + 1;
    if chars[start..end].iter().all(|c| c.is_whitespace()) {
        return None;
    }
    if open > 0 && chars[open - 1] == '!' {
        return None;
    }
    Some(Linked {
        open,
        start,
        end,
        close: caret,
        href: chars[paren + 1..caret - 1].iter().collect(),
    })
}

/// Whether the three backticks just typed are a fence: a block whose whole text is ```. A fence
/// toggles whether its block is preformatted, and Enter in a preformatted block makes another, so
/// a code region is a run of preformatted paragraphs ended by the closing fence. ``` mid-sentence
/// is three backticks.
pub fn is_fence(text: &str, caret: usize) -> bool {
    let chars: Vec<char> = text.chars().collect();
    caret == chars.len() && chars.len() == 3 && chars.iter().all(|c| *c == '`')
}

/// The block kind a line-opening prefix asks for — `# ` up to six deep (`doc/text-core.md`), `- `
/// or `* ` for a list item — and how many characters to remove, recognised when its trailing space
/// lands.
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

    #[test]
    fn a_closing_parenthesis_completes_a_link() {
        let text = "see [the spec](https://x.org/a_b) now";
        let caret = text.find(" now").unwrap();
        let found = linked(text, caret).expect("a link");
        assert_eq!((found.open, found.start, found.end), (4, 5, 13));
        assert_eq!(found.href, "https://x.org/a_b");
        assert_eq!(found.close, caret);
    }

    #[test]
    fn what_is_not_a_link_is_left_as_typed() {
        for text in [
            "(an aside)",
            "[](https://x)",
            "[ ](https://x)",
            "[label]()",
            "[label] (https://x)",
            "[label](two words)",
            "![alt](pic.png)",
            "[a]b](c)",
        ] {
            assert_eq!(linked(text, text.chars().count()), None, "{text}");
        }
        // Not until the `)` lands.
        assert_eq!(linked("[a](b", 5), None);
    }

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

    /// `**` is tried before `*`, so a bold's third marker does not read as an italic.
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
    fn a_backtick_pair_is_monospace() {
        let found = typing("`code`").expect("a pair");
        assert_eq!(found.emphasis, Emphasis::Code);
        assert_eq!(
            found.emphasis.style().font_family.as_deref(),
            Some(MONOSPACE)
        );
        // And the rules that keep prose out of it hold for it too.
        assert_eq!(typing("a`b`"), None, "mid-word");
        assert_eq!(typing("` `"), None, "empty");
    }

    /// A fence is the whole block and nothing else, so ``` mid-sentence is three backticks.
    #[test]
    fn three_backticks_alone_are_a_fence() {
        assert!(is_fence("```", 3));
        assert!(!is_fence("``", 2), "two is an empty pair, not a fence");
        assert!(!is_fence("a```", 4), "not at the start of the block");
        assert!(!is_fence("```x", 4));
        assert!(!is_fence("```", 2), "and only as the third one lands");
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
