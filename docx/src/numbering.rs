// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `word/numbering.xml` — what makes a paragraph a list item, and what its label looks like
//! (§17.9).
//!
//! Two levels of indirection, both of which a reader has to follow: a paragraph names a
//! **`w:num`** by `w:numId`, the `w:num` names a **`w:abstractNum`** that holds up to nine
//! `w:lvl` definitions, and the `w:num` may override any level (`w:lvlOverride`) or restart its
//! count (`w:startOverride`). The result here is one [`List`] per `numId`, levels resolved, and
//! `emit.rs` writes each as one ODF `text:list-style` — so a numbered list arrives *numbered* in
//! the file a save writes and in what LibreOffice shows, even though the text model itself draws
//! every list item with a bullet (`grind_text::paint::bullet`).

use std::collections::HashMap;

use crate::props::{self, Fonts, Props};
use crate::xml::{Handled, Reader, Word as _, WordAttrs as _};

/// One level of a list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Level {
    /// `w:numFmt` — `decimal`, `bullet`, `lowerLetter`, … (§17.18.59).
    pub format: String,
    /// `w:lvlText` — `%1.`, `%1.%2.`, or the bullet character itself.
    pub text: String,
    pub start: i64,
    /// `w:ind` of the level, in twips: the text's left edge and how far the label hangs.
    pub left: Option<i64>,
    pub hanging: Option<i64>,
    /// The label's own formatting — a bullet's font, most often.
    pub props: Props,
    /// `w:pStyle` — the paragraph style this level belongs to, which is how a heading style
    /// that names a list but no level finds its level (§17.9.24).
    pub style: Option<String>,
}

/// One `numId`'s list: its nine levels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct List {
    pub levels: Vec<Level>,
    /// The abstract definition it came from — two `numId`s sharing one are the same list to
    /// Word's eye, a fact recorded in `doc/docx-format.md` §4.2 and not yet acted on.
    pub abstract_id: i64,
}

impl List {
    pub fn level(&self, ilvl: usize) -> Option<&Level> {
        self.levels.get(ilvl)
    }

    /// Whether every level is a bullet — the question "is this a bulleted list?" asks.
    pub fn is_bulleted(&self) -> bool {
        self.levels.first().is_some_and(|l| l.format == "bullet")
    }
}

#[derive(Clone, Debug, Default)]
pub struct Numbering {
    pub lists: HashMap<i64, List>,
}

/// A `w:num`'s `w:lvlOverride`s, by level: a replacement level, a restarted count, or both.
type Overrides = HashMap<usize, (Option<Level>, Option<i64>)>;

/// Read `word/numbering.xml`.
pub fn read(bytes: &[u8], fonts: &Fonts) -> Numbering {
    let mut abstracts: HashMap<i64, Vec<Level>> = HashMap::new();
    // numId → (abstractNumId, overrides by level)
    let mut nums: Vec<(i64, i64, Overrides)> = Vec::new();
    let mut reader = Reader::new(bytes);
    if !matches!(reader.root(), Ok(Some(_))) {
        return Numbering::default();
    }
    let _ = reader.children(|r, name, attrs| {
        if name.w("abstractNum") {
            let Some(id) = attrs.int("abstractNumId") else {
                return Ok(Handled::No);
            };
            let mut levels = vec![Level::default(); 9];
            r.children(|r, name, attrs| {
                if !name.w("lvl") {
                    return Ok(Handled::No);
                }
                let ilvl = attrs.int("ilvl").unwrap_or(0);
                let level = read_level(r, fonts)?;
                if let Some(slot) = usize::try_from(ilvl).ok().and_then(|i| levels.get_mut(i)) {
                    *slot = level;
                }
                Ok(Handled::Yes)
            })?;
            abstracts.insert(id, levels);
            Ok(Handled::Yes)
        } else if name.w("num") {
            let Some(id) = attrs.int("numId") else {
                return Ok(Handled::No);
            };
            let mut abstract_id = None;
            let mut overrides = HashMap::new();
            r.children(|r, name, attrs| {
                if name.w("abstractNumId") {
                    abstract_id = attrs.int("val");
                } else if name.w("lvlOverride") {
                    let ilvl = attrs.int("ilvl").unwrap_or(0) as usize;
                    let mut level = None;
                    let mut start = None;
                    r.children(|r, name, attrs| {
                        if name.w("startOverride") {
                            start = attrs.int("val");
                        } else if name.w("lvl") {
                            level = Some(read_level(r, fonts)?);
                        } else {
                            return Ok(Handled::No);
                        }
                        Ok(Handled::Yes)
                    })?;
                    overrides.insert(ilvl, (level, start));
                } else {
                    return Ok(Handled::No);
                }
                Ok(Handled::Yes)
            })?;
            if let Some(abstract_id) = abstract_id {
                nums.push((id, abstract_id, overrides));
            }
            Ok(Handled::Yes)
        } else {
            Ok(Handled::No)
        }
    });
    let mut lists = HashMap::new();
    for (id, abstract_id, overrides) in nums {
        let mut levels = abstracts.get(&abstract_id).cloned().unwrap_or_default();
        for (ilvl, (level, start)) in overrides {
            if let Some(slot) = levels.get_mut(ilvl) {
                if let Some(level) = level {
                    *slot = level;
                }
                if let Some(start) = start {
                    slot.start = start;
                }
            }
        }
        lists.insert(
            id,
            List {
                levels,
                abstract_id,
            },
        );
    }
    Numbering { lists }
}

fn read_level(r: &mut Reader, fonts: &Fonts) -> grind_ooxml::Result<Level> {
    let mut level = Level {
        format: "decimal".into(),
        start: 1,
        ..Level::default()
    };
    r.children(|r, name, attrs| {
        match name.local.as_str() {
            _ if name.ns != grind_ooxml::names::Ns::Word => return Ok(Handled::No),
            "start" => level.start = attrs.int("val").unwrap_or(1),
            "numFmt" => level.format = attrs.val().unwrap_or("decimal").to_owned(),
            "lvlText" => level.text = attrs.val().unwrap_or_default().to_owned(),
            "pPr" => {
                r.children(|_, name, attrs| {
                    if name.w("ind") {
                        level.left = attrs.int("left").or_else(|| attrs.int("start"));
                        level.hanging = attrs
                            .int("hanging")
                            .or_else(|| attrs.int("firstLine").map(|f| -f));
                    }
                    Ok(Handled::Yes)
                })?;
            }
            "rPr" => level.props = props::read_rpr(r, fonts)?.props,
            "pStyle" => level.style = attrs.val().map(str::to_owned),
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    Ok(level)
}

/// A bullet as a character a reader without Word's symbol fonts can draw.
///
/// Word's bullets are very often a code point in the private-use area of the *Symbol* or
/// *Wingdings* font — `U+F0B7` is Symbol's round bullet — which in any other font is nothing at
/// all. The handful every document uses are mapped to their Unicode meaning
/// (`doc/docx-format.md` §4.3); anything else is the round bullet, which is what it most likely
/// was, rather than a missing glyph.
pub fn bullet(text: &str) -> char {
    match text.chars().next() {
        Some('\u{F0B7}' | '\u{F06C}' | '\u{2022}') | None => '•',
        Some('o') => '◦',
        Some('\u{F0A7}' | '\u{F06E}' | '\u{F0A8}') => '▪',
        Some('\u{F0D8}') => '➢',
        Some('\u{F0FC}') => '✓',
        Some('\u{F076}') => '❖',
        Some('\u{F0A1}') => '○',
        Some(c) if ('\u{F000}'..='\u{F0FF}').contains(&c) => '•',
        Some(c) => c,
    }
}

/// `w:numFmt` as ODF's `style:num-format` (rng:2163). Every format with no ODF spelling is
/// decimal, which is what it counts in even when it does not look like it.
pub fn num_format(format: &str) -> &'static str {
    match format {
        "lowerLetter" => "a",
        "upperLetter" => "A",
        "lowerRoman" => "i",
        "upperRoman" => "I",
        "none" => "",
        _ => "1",
    }
}

/// `w:lvlText` (`%1.%2.`) as ODF's prefix, suffix and how many levels the label shows: what
/// comes before the first `%n`, what comes after the last, and how many `%n` there are.
pub fn label_parts(text: &str) -> (String, String, usize) {
    let bytes: Vec<char> = text.chars().collect();
    let mut first = None;
    let mut last = None;
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '%' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            first.get_or_insert(i);
            last = Some(i + 2);
            count += 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    match (first, last) {
        (Some(first), Some(last)) => (
            bytes[..first].iter().collect(),
            bytes[last..].iter().collect(),
            count,
        ),
        _ => (text.to_owned(), String::new(), 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = grind_ooxml::names::WORD_T;

    #[test]
    fn a_num_resolves_through_its_abstract_and_its_overrides() {
        let xml = format!(
            r#"<w:numbering xmlns:w="{W}">
                 <w:abstractNum w:abstractNumId="0">
                   <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/>
                     <w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl>
                   <w:lvl w:ilvl="1"><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2)"/></w:lvl>
                 </w:abstractNum>
                 <w:abstractNum w:abstractNumId="1">
                   <w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val=""/>
                     <w:rPr><w:rFonts w:ascii="Symbol" w:hAnsi="Symbol"/></w:rPr></w:lvl>
                 </w:abstractNum>
                 <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
                 <w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
                 <w:num w:numId="3"><w:abstractNumId w:val="0"/>
                   <w:lvlOverride w:ilvl="0"><w:startOverride w:val="5"/></w:lvlOverride></w:num>
               </w:numbering>"#
        );
        let n = read(xml.as_bytes(), &Fonts::default());
        let one = &n.lists[&1];
        assert_eq!(one.level(0).unwrap().text, "%1.");
        assert_eq!(one.level(0).unwrap().left, Some(720));
        assert_eq!(one.level(0).unwrap().hanging, Some(360));
        assert_eq!(one.level(1).unwrap().format, "lowerLetter");
        assert!(!one.is_bulleted());
        assert!(n.lists[&2].is_bulleted());
        assert_eq!(bullet(&n.lists[&2].level(0).unwrap().text), '•');
        assert_eq!(
            n.lists[&3].level(0).unwrap().start,
            5,
            "the override restarts"
        );
        assert_eq!(n.lists[&1].level(0).unwrap().start, 1, "and only there");
    }

    #[test]
    fn a_label_splits_around_its_numbers() {
        assert_eq!(label_parts("%1."), ("".into(), ".".into(), 1));
        assert_eq!(label_parts("(%1)"), ("(".into(), ")".into(), 1));
        assert_eq!(label_parts("%1.%2.%3"), ("".into(), "".into(), 3));
        assert_eq!(label_parts("Step %1:"), ("Step ".into(), ":".into(), 1));
        assert_eq!(label_parts("•"), ("•".into(), "".into(), 0));
    }
}
