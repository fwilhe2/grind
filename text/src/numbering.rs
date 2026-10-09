// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! List numbering, **read for showing and never authored** — `crate::paragraph`'s rule, for
//! `text:list-style` (rng:17558).
//!
//! The model flattens a list into blocks with a depth (`doc/text-core.md`), and that stays the
//! model: nothing here is written back, and a save keeps a list's style because the writer keeps
//! its element. What this adds is the label a reader of the page sees in front of an item — `1.`,
//! `2.a)`, `(iv)`, the style's own bullet — derived from three things read with the document:
//! the list styles, which list each item belongs to, and the order of the items. A label is
//! never stored, so it cannot go stale: insert an item and every number after it moves.
//!
//! **What LibreOffice does, measured** (`doc/odt-format.md` §5c, fact 15): an item's label is
//! its level's prefix, then the numbers of the levels `text:display-levels` asks for — each in
//! its own level's format, joined by `.` — then the level's suffix; a `text:list-header` neither
//! counts nor shows one; every top-level `text:list` starts again unless it says
//! `text:continue-numbering` or `text:continue-list`; `text:start-value` on an item sets its
//! number and on a level its first; and a list naming no style shows no mark at all.
//!
//! **Headings are numbered the same way**, by the document's one `text:outline-style`
//! (rng:17530, [`crate::Document::outline_style`]) — `1`, `1.1`, `2.1.1` — counted across the
//! whole document level by level, a `text:is-list-header` heading neither counting nor showing
//! one, and a level whose format is empty showing nothing (§5c fact 20).

use std::collections::{BTreeMap, HashMap};

use crate::model::{BlockId, BlockKind, Document};

/// One `text:list-style`: what each level's items wear, by level from 1, and where its label
/// and text go.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListStyle {
    pub levels: BTreeMap<u32, Level>,
    pub indents: BTreeMap<u32, LevelIndent>,
}

/// One level's `style:list-level-label-alignment` (rng:13281), ODF's values verbatim: the
/// paragraph's left margin and first-line indent while it is at this level, the tab stop its
/// label is followed by, and what follows the label — `listtab`, `space` or `nothing`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelIndent {
    pub margin_left: Option<String>,
    pub text_indent: Option<String>,
    pub tab: Option<String>,
    pub followed_by: Option<String>,
}

/// One `text:list-level-style-*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Level {
    /// `text:list-level-style-bullet` (and `-image`, drawn as a bullet): the character.
    Bullet(String),
    /// `text:list-level-style-number`.
    Number {
        /// `style:num-format` — `1`, `a`, `A`, `i`, `I`, or empty for no number at all.
        format: String,
        prefix: String,
        suffix: String,
        /// `text:display-levels`: how many levels' numbers the label shows, this one last.
        display: u32,
        /// `text:start-value`: the level's first number.
        start: u32,
        /// `loext:num-list-format`, LibreOffice's template for the whole label — `%1%.%2%.` —
        /// which, where a level has one, Writer shows instead of the prefix, the numbers
        /// `display` asks for and the suffix (`doc/odt-format.md` §5c fact 21).
        template: Option<String>,
    },
}

/// Which list a list item's block belongs to, as the reader found it — what a label is derived
/// from, and never written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListMark {
    /// The list style in effect: the `text:list`'s own, or the nearest enclosing one's.
    pub style: Option<String>,
    /// Which numbering this item counts in: one per top-level `text:list`, shared by a list that
    /// continues another.
    pub chain: usize,
    /// The item's own `text:start-value`.
    pub start: Option<u32>,
    /// A `text:list-header`, which is not numbered.
    pub header: bool,
    /// Whether this is the item's first block; a second paragraph in one item has no label.
    pub first: bool,
}

/// `n` in a `style:num-format`: arabic, lower or upper letters (`z` then `aa`), lower or upper
/// roman. Anything else is arabic, and an empty format shows no number.
pub fn number(n: u32, format: &str) -> String {
    match format {
        "" => String::new(),
        "a" => letters(n, b'a'),
        "A" => letters(n, b'A'),
        "i" => roman(n).to_lowercase(),
        "I" => roman(n),
        _ => n.to_string(),
    }
}

/// `a`…`z`, then `aa`…`zz`, as LibreOffice and Writer count letters.
fn letters(n: u32, base: u8) -> String {
    if n == 0 {
        return "0".to_owned();
    }
    let letter = char::from(base + ((n - 1) % 26) as u8);
    std::iter::repeat_n(letter, ((n - 1) / 26 + 1) as usize).collect()
}

fn roman(mut n: u32) -> String {
    if n == 0 || n >= 4000 {
        return n.to_string();
    }
    const TABLE: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, spelling) in TABLE {
        while n >= value {
            out.push_str(spelling);
            n -= value;
        }
    }
    out
}

/// A bullet character as it should be drawn: a symbol font's private-use code point — Word's
/// `` from Symbol, which a document carries over from Windows — is no glyph in any other
/// face, so it is drawn as the plain bullet it stands for.
fn drawable(bullet: &str) -> String {
    match bullet.chars().next() {
        None => "\u{2022}".to_owned(),
        Some(c) if ('\u{e000}'..='\u{f8ff}').contains(&c) => "\u{2022}".to_owned(),
        Some(_) => bullet.to_owned(),
    }
}

/// Every list item's label among the first `upto` blocks, by index. An **empty** label is a
/// header or an item's second paragraph, which wear nothing; `None` is a block that is not a
/// list item, or one whose list states no style to say what it wears — a shell then draws its
/// own bullet (`crate::paint::bullet`), which is what a list made before this build wrote one
/// with a style of its own (`Lgrind1`, `odf::write`) looks like.
///
/// An item added by editing has no [`ListMark`] of its own; it takes the one of the item before
/// it at its depth, so pressing Enter in a numbered list numbers the new item.
pub fn labels(doc: &Document, upto: usize) -> Vec<Option<String>> {
    let mut out = Vec::with_capacity(upto.min(doc.blocks.len()));
    // Per chain, the current number of each level from 1 (`None` where none has been seen).
    let mut counters: HashMap<usize, Vec<Option<u32>>> = HashMap::new();
    let mut previous: Vec<Option<ListMark>> = Vec::new();
    // The outline's own counters, one per heading level from 1.
    let mut outline: Vec<Option<u32>> = Vec::new();
    for block in doc.blocks.iter().take(upto) {
        if let BlockKind::Heading { level } = block.kind {
            previous.clear();
            let mark = doc.list_marks.get(&block.id);
            out.push(match &doc.outline_style {
                Some(_) if mark.is_some_and(|mark| mark.header) => Some(String::new()),
                Some(style) => {
                    let label = count(style, &mut outline, level.max(1) as usize, mark);
                    Some(label).filter(|label| !label.is_empty())
                }
                None => None,
            });
            continue;
        }
        let BlockKind::ListItem { depth } = block.kind else {
            previous.clear();
            out.push(None);
            continue;
        };
        let level = depth.max(1) as usize;
        let mark = match doc.list_marks.get(&block.id) {
            Some(mark) => mark.clone(),
            None => match previous.get(level - 1).cloned().flatten() {
                Some(before) => ListMark {
                    start: None,
                    header: false,
                    first: true,
                    ..before
                },
                None => {
                    out.push(None);
                    continue;
                }
            },
        };
        previous.resize(level, None);
        previous[level - 1] = Some(mark.clone());
        if mark.header || !mark.first {
            out.push(Some(String::new()));
            continue;
        }
        let Some(style) = mark.style.as_deref().and_then(|s| doc.list_styles.get(s)) else {
            out.push(None);
            continue;
        };
        let chain = counters.entry(mark.chain).or_default();
        out.push(Some(count(style, chain, level, Some(&mark))));
    }
    out
}

/// Count one item at `level` in `chain` — its own `start`, else one past the last at its level,
/// else its level's first — and answer its label. Every level below it starts again.
fn count(
    style: &ListStyle,
    chain: &mut Vec<Option<u32>>,
    level: usize,
    mark: Option<&ListMark>,
) -> String {
    let start_of = |level: usize| match style.levels.get(&(level as u32)) {
        Some(Level::Number { start, .. }) => *start,
        _ => 1,
    };
    chain.resize(level, None);
    chain[level - 1] = Some(match (mark.and_then(|mark| mark.start), chain[level - 1]) {
        (Some(start), _) => start,
        (None, Some(n)) => n + 1,
        (None, None) => start_of(level),
    });
    // A level shown above this one that no item opened counts as its first number.
    for (i, slot) in chain.iter_mut().enumerate().take(level - 1) {
        slot.get_or_insert(start_of(i + 1));
    }
    let label = compose(style, chain, level);
    chain.truncate(level);
    label
}

/// The label of an item at `level`, its own and the levels above it counted in `chain`.
fn compose(style: &ListStyle, chain: &[Option<u32>], level: usize) -> String {
    match style.levels.get(&(level as u32)) {
        Some(Level::Bullet(bullet)) => drawable(bullet),
        Some(Level::Number {
            template: Some(template),
            ..
        }) => fill(template, |l| {
            let format = match style.levels.get(&(l as u32)) {
                Some(Level::Number { format, .. }) => format.as_str(),
                _ => "1",
            };
            match chain.get(l - 1).copied().flatten() {
                Some(n) => number(n, format),
                None => String::new(),
            }
        }),
        Some(Level::Number {
            prefix,
            suffix,
            display,
            ..
        }) => {
            let first = level.saturating_sub((*display).max(1) as usize - 1).max(1);
            let numbers: Vec<String> = (first..=level)
                .map(|l| {
                    let format = match style.levels.get(&(l as u32)) {
                        Some(Level::Number { format, .. }) => format.as_str(),
                        _ => "1",
                    };
                    number(chain[l - 1].unwrap_or(1), format)
                })
                .filter(|n| !n.is_empty())
                .collect();
            format!("{prefix}{}{suffix}", numbers.join("."))
        }
        None => drawable(""),
    }
}

/// `loext:num-list-format`'s template with every `%N%` replaced by `level(N)`, and everything
/// else — a `%` that opens no level number included — kept as it is.
fn fill(template: &str, level: impl Fn(usize) -> String) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        match (
            after[..digits].parse::<usize>(),
            after[digits..].starts_with('%'),
        ) {
            (Ok(n), true) if (1..=10).contains(&n) => {
                out.push_str(&level(n));
                rest = &after[digits + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The marks of a document's list items, for the reader — a block id to the list it is in.
pub type Marks = HashMap<BlockId, ListMark>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_in_every_format_odf_names() {
        assert_eq!(number(4, "1"), "4");
        assert_eq!(number(2, "a"), "b");
        assert_eq!(number(27, "a"), "aa");
        assert_eq!(number(28, "A"), "BB");
        assert_eq!(number(4, "I"), "IV");
        assert_eq!(number(1994, "i"), "mcmxciv");
        assert_eq!(number(3, ""), "");
        assert_eq!(number(3, "1st"), "3", "an unknown format is arabic");
    }

    /// `doc/odt-format.md` §5c fact 21, each case as Writer showed it.
    #[test]
    fn a_label_template_fills_every_level_it_names() {
        let level = |n: usize| ["I", "A", "1", "a", "1", "", "i"][n - 1].to_owned();
        assert_eq!(fill("%1%.%2%.", level), "I.A.");
        assert_eq!(fill("[%1%-%3%] x", level), "[I-1] x");
        assert_eq!(fill("%4%%4%", level), "aa");
        assert_eq!(fill("no number", level), "no number");
        assert_eq!(fill("%6%.", level), ".");
        assert_eq!(fill("%5%/%7%", level), "1/i");
        assert_eq!(fill("100% %x% %1", level), "100% %x% %1");
    }

    #[test]
    fn a_symbol_fonts_bullet_is_drawn_as_a_bullet() {
        assert_eq!(drawable("\u{f0b7}"), "\u{2022}");
        assert_eq!(drawable("–"), "–");
    }
}
