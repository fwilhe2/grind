// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A paragraph's runs as one string, and back (`doc/dsl.md` §3.6). **\[ODT\]**
//!
//! Beside `markdown.rs` rather than in it: that module answers what a character just typed
//! completed, for typing only; parsing and printing whole strings are different questions. The
//! marker table is shared, read from [`Emphasis::markers`] and [`Emphasis::style`].
//!
//! ## The grammar
//!
//! ```text
//! plain words                        one Run::Text with no formatting
//! **bold**  *italic*  __under__      one emphasis, from markdown.rs's table
//! ~~struck~~  `code`
//! [words]{bold=#true size=14pt}      anything the markers cannot say
//! [words](https://example.org)       text:a
//! {#intro}                           text:bookmark — an anchor, zero characters wide
//! \*  \[  \\                         a literal marker character
//! ```
//!
//! A `\t` in the KDL string is a `text:tab` and a `\n` a `text:line-break`; KDL escapes them, so
//! no notation is needed (§3.5).
//!
//! ## Three rules that make it total
//!
//! * One emphasis per marker pair, nothing nested: bold and italic together is `[x]{bold=#true
//!   italic=#true}`, never `***x***`, which needs CommonMark's delimiter-run heuristic to read.
//!   Marker content is literal and the parser never recurses, so every string has one reading.
//! * The writer escapes, so the reader may be simple: a marker the reader sees was meant. An
//!   unclosed `*` in a hand-written file is an asterisk.
//! * A raw KDL string turns the notation off (§3.5): `#"a literal ** stays"#`. The writer uses it
//!   for plain text that would otherwise need many backslashes, which is why [`read`] takes a
//!   `raw` flag.

use crate::markdown::Emphasis;
use crate::model::Run;
use crate::style::CharStyle;

/// Every character the notation can begin with, which decides whether a string is worth writing
/// raw. Not the escape set: [`escape`] looks ahead, since `_` alone and `{` without `#` open
/// nothing.
const SPECIAL: [char; 7] = ['\\', '*', '_', '~', '`', '[', '{'];

/// The emphases, longest marker first, so `**` is tried before `*`.
const BY_LENGTH: [Emphasis; 5] = [
    Emphasis::Bold,
    Emphasis::Underline,
    Emphasis::Strike,
    Emphasis::Italic,
    Emphasis::Code,
];

/// A block's runs as one string, plus whether it should be written as a **raw** KDL string.
pub struct Inline {
    pub text: String,
    /// True when the string carries no notation and holds a character that would otherwise need
    /// escaping.
    pub raw: bool,
}

/// Spell a block's runs as one string, or `None` when a run has no spelling — an image, the text
/// projection's named gap (`doc/projection-text.md`).
pub fn write(runs: &[Run]) -> Option<Inline> {
    // A kept element (a footnote, a field) is spelled as the text it shows; the element itself is
    // a named gap (`doc/projection-text.md`).
    let shown: Vec<Run>;
    let runs = if runs.iter().any(|r| matches!(r, Run::Kept { .. })) {
        shown = runs
            .iter()
            .map(|r| match r {
                Run::Kept { text, .. } => Run::plain(text.clone()),
                other => other.clone(),
            })
            .collect();
        &shown[..]
    } else {
        runs
    };
    let mut text = String::new();
    // A raw string is possible only while every run is unformatted.
    let mut plain = true;
    for run in runs {
        match run {
            Run::Tab => text.push('\t'),
            Run::Break => text.push('\n'),
            Run::Bookmark { name } => {
                plain = false;
                text.push_str("{#");
                text.push_str(&escape(name, Some('}')));
                text.push('}');
            }
            Run::Image { .. } => return None,
            Run::Kept { .. } => {}
            Run::Text {
                text: body,
                style,
                props,
                href,
            } => {
                match spelling(style.as_deref(), props, href.as_deref()) {
                    Spelling::Plain => text.push_str(&escape(body, None)),
                    Spelling::Marked(emphasis) => {
                        plain = false;
                        text.push_str(emphasis.markers());
                        text.push_str(&escape(body, None));
                        text.push_str(emphasis.markers());
                    }
                    Spelling::Attributes => {
                        plain = false;
                        text.push('[');
                        text.push_str(&escape(body, Some(']')));
                        text.push(']');
                        match (href, style.is_none() && props.is_plain()) {
                            // The `(url)` shorthand, for a run that is only a link.
                            (Some(url), true) => {
                                text.push('(');
                                text.push_str(&escape(url, Some(')')));
                                text.push(')');
                            }
                            _ => attributes(&mut text, style.as_deref(), props, href.as_deref()),
                        }
                    }
                }
            }
        }
    }
    // Not raw with a newline (a KDL multi-line string, with its own dedenting) or a tab (invisible
    // in the file). `plain` means every backslash in `text` is one `escape` added and `unescape`
    // removes, so the raw form is the same characters.
    let raw = plain
        && text.contains(SPECIAL)
        && !text.contains(['\n', '\t'])
        && !text.contains("\"#")
        && !text.ends_with('"');
    if raw {
        text = unescape(&text);
    }
    Some(Inline { text, raw })
}

/// Read a string back into runs. With `raw` (the KDL string was `#"…"#`) the notation is off, and
/// only tab and line break become runs of their own.
pub fn read(text: &str, raw: bool) -> Vec<Run> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs = Vec::new();
    let mut buffer = String::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        // Tab and break first: they are elements, not notation, so a raw string has them too.
        if c == '\t' || c == '\n' {
            flush(&mut runs, &mut buffer);
            runs.push(if c == '\t' { Run::Tab } else { Run::Break });
            at += 1;
            continue;
        }
        if raw {
            buffer.push(c);
            at += 1;
            continue;
        }
        match c {
            '\\' if at + 1 < chars.len() => {
                buffer.push(chars[at + 1]);
                at += 2;
            }
            '{' => match bookmark(&chars, at) {
                Some((name, next)) => {
                    flush(&mut runs, &mut buffer);
                    runs.push(Run::Bookmark { name });
                    at = next;
                }
                None => {
                    buffer.push(c);
                    at += 1;
                }
            },
            '[' => match bracketed(&chars, at) {
                Some((run, next)) => {
                    flush(&mut runs, &mut buffer);
                    runs.push(run);
                    at = next;
                }
                None => {
                    buffer.push(c);
                    at += 1;
                }
            },
            '*' | '_' | '~' | '`' => match marked(&chars, at) {
                Some((run, next)) => {
                    flush(&mut runs, &mut buffer);
                    runs.push(run);
                    at = next;
                }
                None => {
                    buffer.push(c);
                    at += 1;
                }
            },
            _ => {
                buffer.push(c);
                at += 1;
            }
        }
    }
    flush(&mut runs, &mut buffer);
    runs
}

/// Which of the three forms a run is written in.
enum Spelling {
    Plain,
    Marked(Emphasis),
    Attributes,
}

/// A run with exactly one emphasis and nothing else gets markers; anything else not plain gets
/// attributes. Compared against `Emphasis::style()`, so "bold" means what typing `**bold**`
/// produces.
fn spelling(style: Option<&str>, props: &CharStyle, href: Option<&str>) -> Spelling {
    if style.is_some() || href.is_some() {
        return Spelling::Attributes;
    }
    if props.is_plain() {
        return Spelling::Plain;
    }
    match BY_LENGTH.iter().find(|e| e.style() == *props) {
        Some(emphasis) => Spelling::Marked(*emphasis),
        None => Spelling::Attributes,
    }
}

/// The `{…}` attribute list: every property, ODF values verbatim, the four switches as `#true`
/// when they hold the notation's value.
fn attributes(out: &mut String, style: Option<&str>, props: &CharStyle, href: Option<&str>) {
    out.push('{');
    let mut first = true;
    let mut put = |name: &str, value: &str, bare: bool| {
        if !first {
            out.push(' ');
        }
        first = false;
        out.push_str(name);
        out.push('=');
        match bare {
            true => out.push_str(value),
            false => out.push_str(&quote(value)),
        }
    };
    switch(
        &mut put,
        "bold",
        "weight",
        props.font_weight.as_deref(),
        "bold",
    );
    switch(
        &mut put,
        "italic",
        "slant",
        props.font_style.as_deref(),
        "italic",
    );
    switch(
        &mut put,
        "underline",
        "underline",
        props.underline.as_deref(),
        "solid",
    );
    switch(
        &mut put,
        "strike",
        "strike",
        props.line_through.as_deref(),
        "solid",
    );
    for (name, value) in [
        ("family", props.font_family.as_deref()),
        ("size", props.font_size.as_deref()),
        ("color", props.color.as_deref()),
        ("background", props.background.as_deref()),
        ("style", style),
        ("href", href),
    ] {
        if let Some(value) = value {
            put(name, value, false);
        }
    }
    out.push('}');
}

/// A switch-shaped property: `bold=#true` for its canonical value, `weight="600"` otherwise.
fn switch(
    put: &mut impl FnMut(&str, &str, bool),
    switch: &str,
    named: &str,
    value: Option<&str>,
    canonical: &str,
) {
    match value {
        Some(value) if value == canonical => put(switch, "#true", true),
        Some(value) => put(named, value, false),
        None => {}
    }
}

/// `{#name}` — a bookmark. Returns the name and where the notation ends.
fn bookmark(chars: &[char], at: usize) -> Option<(String, usize)> {
    if chars.get(at + 1) != Some(&'#') {
        return None;
    }
    let (name, close) = until(chars, at + 2, '}')?;
    (!name.is_empty()).then_some((unescape(&name), close + 1))
}

/// `[text](url)` or `[text]{attrs}`.
fn bracketed(chars: &[char], at: usize) -> Option<(Run, usize)> {
    let (body, close) = until(chars, at + 1, ']')?;
    let text = unescape(&body);
    match chars.get(close + 1) {
        Some('(') => {
            let (url, end) = until(chars, close + 2, ')')?;
            Some((
                Run::Text {
                    text,
                    style: None,
                    props: CharStyle::default(),
                    href: Some(unescape(&url)),
                },
                end + 1,
            ))
        }
        Some('{') => {
            let (list, end) = until(chars, close + 2, '}')?;
            let (style, props, href) = parse_attributes(&list);
            Some((
                Run::Text {
                    text,
                    style,
                    props,
                    href,
                },
                end + 1,
            ))
        }
        // `[words]` with nothing after it is not a construct — it is a bracket somebody typed.
        _ => None,
    }
}

/// A marker pair. The content is literal by construction, so this never recurses.
fn marked(chars: &[char], at: usize) -> Option<(Run, usize)> {
    for emphasis in BY_LENGTH {
        let markers: Vec<char> = emphasis.markers().chars().collect();
        let width = markers.len();
        if chars.get(at..at + width) != Some(&markers[..]) {
            continue;
        }
        // The closing run of the same markers, skipping escapes exactly as the scanner does.
        let mut scan = at + width;
        let mut content = String::new();
        while scan < chars.len() {
            if chars[scan] == '\\' && scan + 1 < chars.len() {
                content.push(chars[scan + 1]);
                scan += 2;
                continue;
            }
            if chars.get(scan..scan + width) == Some(&markers[..]) {
                // An empty pair is two markers somebody typed, not an empty emphasis.
                return (!content.is_empty()).then(|| {
                    (
                        Run::Text {
                            text: content,
                            style: None,
                            props: emphasis.style(),
                            href: None,
                        },
                        scan + width,
                    )
                });
            }
            content.push(chars[scan]);
            scan += 1;
        }
    }
    None
}

/// Everything up to the next unescaped `end`, still escaped, and where that `end` is. An attribute
/// list has escapes of its own (`\"`), so this only skips escapes and each caller decodes:
/// [`unescape`] a body, [`pairs`] a list.
fn until(chars: &[char], from: usize, end: char) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut at = from;
    while at < chars.len() {
        match chars[at] {
            '\\' if at + 1 < chars.len() => {
                out.push('\\');
                out.push(chars[at + 1]);
                at += 2;
            }
            c if c == end => return Some((out, at)),
            c => {
                out.push(c);
                at += 1;
            }
        }
    }
    None
}

/// `\x` becomes `x`, everywhere — [`escape`]'s inverse.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    out
}

/// The inside of a `{…}`: `name=value` pairs, values bare or `"quoted"`.
fn parse_attributes(list: &str) -> (Option<String>, CharStyle, Option<String>) {
    let mut style = None;
    let mut props = CharStyle::default();
    let mut href = None;
    for (name, value) in pairs(list) {
        let set = |slot: &mut Option<String>, canonical: &str| {
            *slot = Some(match value == "#true" {
                true => canonical.to_owned(),
                false => value.clone(),
            });
        };
        match name.as_str() {
            "bold" | "weight" => set(&mut props.font_weight, "bold"),
            "italic" | "slant" => set(&mut props.font_style, "italic"),
            "underline" => set(&mut props.underline, "solid"),
            "strike" => set(&mut props.line_through, "solid"),
            "family" => props.font_family = Some(value),
            "size" => props.font_size = Some(value),
            "color" => props.color = Some(value),
            "background" => props.background = Some(value),
            "style" => style = Some(value),
            "href" => href = Some(value),
            // An attribute with no property is dropped, as the XML reader drops unknown ones.
            _ => {}
        }
    }
    (style, props, href)
}

/// Split an attribute list into pairs, by hand: this is inside a KDL string already unescaped
/// once.
fn pairs(list: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = list.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at].is_whitespace() {
            at += 1;
            continue;
        }
        let Some(eq) = (at..chars.len()).find(|i| chars[*i] == '=' || chars[*i].is_whitespace())
        else {
            break;
        };
        if chars[eq] != '=' {
            at = eq;
            continue;
        }
        let name: String = chars[at..eq].iter().collect();
        let mut value = String::new();
        at = eq + 1;
        if chars.get(at) == Some(&'"') {
            at += 1;
            while at < chars.len() && chars[at] != '"' {
                if chars[at] == '\\' && at + 1 < chars.len() {
                    at += 1;
                }
                value.push(chars[at]);
                at += 1;
            }
            at += 1;
        } else {
            while at < chars.len() && !chars[at].is_whitespace() {
                if chars[at] == '\\' && at + 1 < chars.len() {
                    at += 1;
                }
                value.push(chars[at]);
                at += 1;
            }
        }
        out.push((name, value));
    }
    out
}

/// An attribute value, bare when it can be and quoted when it must. `}` is escaped inside quotes
/// too, since [`until`] finds the list's end before [`pairs`] looks at quotes.
fn quote(value: &str) -> String {
    if !value.is_empty() && !value.contains([' ', '"', '\\', '}', '=']) {
        return value.to_owned();
    }
    let mut out = String::from("\"");
    for c in value.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        if c == '}' {
            // Consumed by `until`, so `pairs` sees a plain `}` inside the quotes.
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Backslash every character that would open a construct here, with the reader's lookahead: `_`,
/// `~` and `{` only where they would mean something (not in `snake_case`), `*` always. `also` is
/// the closer of the bracket this text goes inside — `]` in a `[…]` body, `)` in a URL — or `None`
/// in prose.
fn escape(text: &str, also: Option<char>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        let opens = match c {
            '\\' | '*' | '`' | '[' => true,
            '_' | '~' => next == Some(*c),
            '{' => next == Some('#'),
            c => Some(*c) == also,
        };
        if opens {
            out.push('\\');
        }
        out.push(*c);
    }
    out
}

fn flush(runs: &mut Vec<Run>, buffer: &mut String) {
    if !buffer.is_empty() {
        runs.push(Run::plain(std::mem::take(buffer)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled(text: &str, props: CharStyle) -> Run {
        Run::Text {
            text: text.to_owned(),
            style: None,
            props,
            href: None,
        }
    }

    /// Spelling runs and reading them back is the identity.
    fn round_trip(runs: Vec<Run>) {
        let inline = write(&runs).expect("spellable");
        assert_eq!(
            read(&inline.text, inline.raw),
            runs,
            "via {:?} (raw {})",
            inline.text,
            inline.raw
        );
    }

    #[test]
    fn plain_prose_is_itself() {
        let inline = write(&[Run::plain("hello there")]).unwrap();
        assert_eq!(inline.text, "hello there");
        assert!(!inline.raw, "nothing in it needed escaping");
        round_trip(vec![Run::plain("hello there")]);
    }

    #[test]
    fn each_emphasis_is_its_own_markers() {
        for emphasis in BY_LENGTH {
            let runs = vec![styled("x", emphasis.style())];
            let inline = write(&runs).unwrap();
            assert_eq!(
                inline.text,
                format!("{m}x{m}", m = emphasis.markers()),
                "{emphasis:?}"
            );
            round_trip(runs);
        }
    }

    /// Two emphases at once are attributes, never `***x***`.
    #[test]
    fn two_emphases_are_written_out_rather_than_nested() {
        let mut props = Emphasis::Bold.style();
        props.font_style = Some("italic".to_owned());
        let inline = write(&[styled("x", props.clone())]).unwrap();
        assert_eq!(inline.text, "[x]{bold=#true italic=#true}");
        round_trip(vec![styled("x", props)]);
    }

    #[test]
    fn a_property_the_markers_cannot_say_is_an_attribute() {
        let props = CharStyle {
            font_size: Some("14pt".to_owned()),
            color: Some("#ff4136".to_owned()),
            font_family: Some("Liberation Serif".to_owned()),
            ..Default::default()
        };
        let inline = write(&[styled("x", props.clone())]).unwrap();
        assert_eq!(
            inline.text,
            "[x]{family=\"Liberation Serif\" size=14pt color=#ff4136}"
        );
        round_trip(vec![styled("x", props)]);
    }

    /// A value outside the notation's vocabulary is written as its property.
    #[test]
    fn an_unusual_weight_keeps_its_own_value() {
        let props = CharStyle {
            font_weight: Some("600".to_owned()),
            underline: Some("dotted".to_owned()),
            ..Default::default()
        };
        let inline = write(&[styled("x", props.clone())]).unwrap();
        assert_eq!(inline.text, "[x]{weight=600 underline=dotted}");
        round_trip(vec![styled("x", props)]);
    }

    #[test]
    fn a_link_is_the_short_form_and_a_formatted_link_is_not() {
        let link = Run::Text {
            text: "the site".to_owned(),
            style: None,
            props: CharStyle::default(),
            href: Some("https://example.org".to_owned()),
        };
        assert_eq!(
            write(std::slice::from_ref(&link)).unwrap().text,
            "[the site](https://example.org)"
        );
        round_trip(vec![link]);

        let bold_link = Run::Text {
            text: "the site".to_owned(),
            style: None,
            props: Emphasis::Bold.style(),
            href: Some("https://example.org".to_owned()),
        };
        assert_eq!(
            write(std::slice::from_ref(&bold_link)).unwrap().text,
            "[the site]{bold=#true href=https://example.org}"
        );
        round_trip(vec![bold_link]);
    }

    #[test]
    fn a_named_style_survives_as_a_name() {
        let run = Run::Text {
            text: "x".to_owned(),
            style: Some("Emphasis".to_owned()),
            props: CharStyle::default(),
            href: None,
        };
        assert_eq!(
            write(std::slice::from_ref(&run)).unwrap().text,
            "[x]{style=Emphasis}"
        );
        round_trip(vec![run]);
    }

    #[test]
    fn a_bookmark_is_an_anchor_and_takes_no_space() {
        let runs = vec![
            Run::Bookmark {
                name: "intro".to_owned(),
            },
            Run::plain("after it"),
        ];
        assert_eq!(write(&runs).unwrap().text, "{#intro}after it");
        round_trip(runs);
    }

    #[test]
    fn a_tab_and_a_break_are_elements_rather_than_notation() {
        let runs = vec![Run::plain("a"), Run::Tab, Run::plain("b"), Run::Break];
        let inline = write(&runs).unwrap();
        assert_eq!(inline.text, "a\tb\n");
        assert!(!inline.raw, "a raw string would hide them");
        round_trip(runs);
    }

    /// The escaping rule, in both directions and at its worst.
    #[test]
    fn a_literal_marker_survives_beside_a_real_one() {
        let runs = vec![
            Run::plain("2*3*4 and "),
            styled("bold", Emphasis::Bold.style()),
            Run::plain(" [not a link] {not a mark}"),
        ];
        let inline = write(&runs).unwrap();
        assert_eq!(
            inline.text,
            "2\\*3\\*4 and **bold** \\[not a link] {not a mark}"
        );
        assert!(!inline.raw, "there is notation in it");
        round_trip(runs);
    }

    /// §3.5: a paragraph *about* markdown is a raw string, and the notation is off inside one.
    #[test]
    fn plain_text_full_of_markers_is_written_raw() {
        let runs = vec![Run::plain("a literal ** stays literal, and *this* too")];
        let inline = write(&runs).unwrap();
        assert!(inline.raw, "{:?}", inline.text);
        assert_eq!(
            inline.text, "a literal ** stays literal, and *this* too",
            "raw means the characters themselves — no backslashes"
        );
        round_trip(runs);
    }

    #[test]
    fn a_marker_with_no_partner_is_a_character() {
        assert_eq!(
            read("2*3*4", false),
            vec![
                Run::plain("2"),
                styled("3", Emphasis::Italic.style()),
                Run::plain("4")
            ]
        );
        assert_eq!(
            read("a * b", false),
            vec![Run::plain("a * b")],
            "no closing marker"
        );
        assert_eq!(
            read("**", false),
            vec![Run::plain("**")],
            "an empty pair is two characters"
        );
        assert_eq!(
            read("[words]", false),
            vec![Run::plain("[words]")],
            "a bracket with nothing after it"
        );
    }

    /// An image has no spelling, and the writer says so.
    #[test]
    fn an_image_has_no_spelling_yet() {
        let runs = vec![
            Run::plain("before"),
            Run::Image {
                mime: "image/png".to_owned(),
                data: vec![1, 2, 3],
                width: None,
                height: None,
                anchor: None,
                title: None,
                description: None,
            },
        ];
        assert!(write(&runs).is_none());
    }
}
