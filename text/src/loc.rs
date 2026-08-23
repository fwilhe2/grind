// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Addressing a place in a text document — the `grind_sheet::a1` of this crate.
//!
//! **The only 0↔1 conversion in `grind-text`.** A shell never does its own index arithmetic,
//! for the same reason no shell does its own cell arithmetic: one conversion in one file is
//! testable, and two are eventually inconsistent.
//!
//! The spreadsheet got a gift here that this crate does not: A1 notation is normative,
//! universally understood, and already specified. Text has no such thing, so the vocabulary
//! below is **invented**, and is written down rather than left implicit:
//!
//! | Spelling | Means |
//! |---|---|
//! | `p12` | the 12th block, 1-based |
//! | `p12:p20` | a range of blocks |
//! | `p12+40` | character offset 40 within block 12 |
//! | `#intro` | wherever the `text:bookmark` named `intro` is |
//! | `§2.1.3` | outline path — chapter 2, section 1, subsection 3 |
//!
//! The last two are the interesting ones. `p12` is what a machine uses and is invalidated by
//! every insertion above it; `#intro` and `§2.1.3` are what a *person* or a *script* uses and
//! survive editing elsewhere in the document. `grind text set report.fodt §3.2 "…"` still
//! works next week, which is a capability the CLI has and no word processor's UI does.
//!
//! `§` is accepted as `s` too, because it is not on most keyboards.

use crate::model::{BlockKind, Document};

/// Where something is, before it has been resolved against a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loc {
    /// `p12` — a block by position, **0-based here**, 1-based in the spelling.
    Block { index: usize, offset: Option<usize> },
    /// `#intro` — a bookmark by name.
    Bookmark { name: String },
    /// `§2.1.3` — an outline path, each component 1-based.
    Outline { path: Vec<u32> },
}

/// A span of blocks — `p12:p20`, or a single `Loc` standing for one block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: Loc,
    pub end: Loc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

fn bad(what: &str) -> ParseError {
    ParseError(format!(
        "{what} is not a location — try p12, p12+40, p12:p20, #bookmark or \u{a7}2.1.3"
    ))
}

/// Parse one location.
pub fn parse(text: &str) -> Result<Loc, ParseError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(bad("an empty address"));
    }

    if let Some(name) = text.strip_prefix('#') {
        if name.is_empty() {
            return Err(bad(text));
        }
        return Ok(Loc::Bookmark {
            name: name.to_owned(),
        });
    }

    // `§2.1.3`, and `s2.1.3` for keyboards without the character.
    let outline = text
        .strip_prefix('\u{a7}')
        .or_else(|| text.strip_prefix('s').or_else(|| text.strip_prefix('S')));
    if let Some(rest) = outline
        && rest.starts_with(|c: char| c.is_ascii_digit())
    {
        let mut path = Vec::new();
        for part in rest.split('.') {
            let n: u32 = part.parse().map_err(|_| bad(text))?;
            if n == 0 {
                return Err(bad(text));
            }
            path.push(n);
        }
        return Ok(Loc::Outline { path });
    }

    // `p12`, `p12+40`.
    let rest = text
        .strip_prefix('p')
        .or_else(|| text.strip_prefix('P'))
        .ok_or_else(|| bad(text))?;
    let (number, offset) = match rest.split_once('+') {
        Some((number, offset)) => {
            let offset: usize = offset.parse().map_err(|_| bad(text))?;
            (number, Some(offset))
        }
        None => (rest, None),
    };
    let number: usize = number.parse().map_err(|_| bad(text))?;
    if number == 0 {
        return Err(ParseError(
            "blocks are numbered from 1, so there is no p0".to_owned(),
        ));
    }
    Ok(Loc::Block {
        index: number - 1,
        offset,
    })
}

/// Parse `p12:p20`, or a single location standing for one block.
pub fn parse_range(text: &str) -> Result<Range, ParseError> {
    match text.split_once(':') {
        Some((start, end)) => Ok(Range {
            start: parse(start)?,
            end: parse(end)?,
        }),
        None => {
            let one = parse(text)?;
            Ok(Range {
                start: one.clone(),
                end: one,
            })
        }
    }
}

/// A block index back as the `p12` a person types. **The 0→1 conversion.**
pub fn format(index: usize) -> String {
    format!("p{}", index + 1)
}

/// A block index and a character offset, as `p12+40`.
pub fn format_offset(index: usize, offset: usize) -> String {
    format!("p{}+{}", index + 1, offset)
}

/// An outline path back as `§2.1.3`.
pub fn format_outline(path: &[u32]) -> String {
    let parts: Vec<String> = path.iter().map(u32::to_string).collect();
    format!("\u{a7}{}", parts.join("."))
}

/// Resolve a location to a block index, against the document it names.
///
/// The whole reason `#intro` and `§2.1.3` are worth having: they are resolved *now*, against
/// the document as it currently is, so an edit somewhere else does not invalidate them.
pub fn resolve(doc: &Document, loc: &Loc) -> Result<usize, ParseError> {
    match loc {
        Loc::Block { index, .. } => {
            if *index >= doc.blocks.len() {
                return Err(ParseError(format!(
                    "{} is past the end; the document has {} block(s)",
                    format(*index),
                    doc.blocks.len()
                )));
            }
            Ok(*index)
        }
        Loc::Bookmark { name } => {
            let id = doc
                .bookmarks
                .get(name)
                .ok_or_else(|| ParseError(format!("no bookmark named {name}")))?;
            doc.index_of(*id)
                .ok_or_else(|| ParseError(format!("bookmark {name} points nowhere")))
        }
        Loc::Outline { path } => outline_index(doc, path)
            .ok_or_else(|| ParseError(format!("no section {}", format_outline(path)))),
    }
}

/// Walk the outline counting headings at each level, and return the block the path names.
///
/// Written as a walk rather than a lookup because the document stores no outline — it is
/// implied by `text:outline-level` on a flat sequence (`doc/odt-format.md` §2), so the
/// numbering has to be *computed* the same way a reader's eye computes it.
///
/// A level that is skipped — a level-3 heading directly under a level-1 — does not shift the
/// count of the level it skipped, which is what LibreOffice's own outline pane shows and the
/// only reading under which `§2.1` keeps meaning the same thing when an unrelated section is
/// edited.
fn outline_index(doc: &Document, path: &[u32]) -> Option<usize> {
    if path.is_empty() {
        return None;
    }
    // `counters[d]` is how many headings of level d+1 have been seen inside the current parent.
    let mut counters: Vec<u32> = Vec::new();
    for (index, level) in doc.outline() {
        let depth = level as usize;
        // Entering a deeper level starts its count; returning to a shallower one discards
        // everything below.
        if depth > counters.len() {
            counters.resize(depth, 0);
        } else {
            counters.truncate(depth);
        }
        counters[depth - 1] += 1;

        if counters.len() >= path.len() && counters[..path.len()] == *path {
            return Some(index);
        }
    }
    None
}

/// The outline path of the heading at `index`, or `None` if it is not a heading.
///
/// [`outline_index`]'s inverse, and the reason `grind text outline` can print an address a
/// user can type straight back in.
pub fn outline_path(doc: &Document, index: usize) -> Option<Vec<u32>> {
    let mut counters: Vec<u32> = Vec::new();
    for (i, level) in doc.outline() {
        let depth = level as usize;
        if depth > counters.len() {
            counters.resize(depth, 0);
        } else {
            counters.truncate(depth);
        }
        counters[depth - 1] += 1;
        if i == index {
            return Some(counters.clone());
        }
    }
    None
}

/// Resolve a range to the block indices it covers, inclusive of both ends.
pub fn resolve_range(doc: &Document, range: &Range) -> Result<std::ops::Range<usize>, ParseError> {
    let start = resolve(doc, &range.start)?;
    let end = resolve(doc, &range.end)?;
    if end < start {
        return Err(ParseError(format!(
            "{} comes after {}",
            format(start),
            format(end)
        )));
    }
    Ok(start..end + 1)
}

/// Whether a block is a heading — for a caller deciding whether `§` addressing applies.
pub fn is_heading(doc: &Document, index: usize) -> bool {
    matches!(
        doc.block(index).map(|b| &b.kind),
        Some(BlockKind::Heading { .. })
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Block, Run};

    fn doc(spec: &[(u32, &str)]) -> Document {
        // (0, text) is a paragraph; (n, text) is a heading at level n.
        let mut d = Document::new();
        for (level, text) in spec {
            let id = d.next_id();
            let kind = match level {
                0 => BlockKind::Paragraph,
                n => BlockKind::Heading { level: *n },
            };
            let mut block = Block::new(id, kind);
            block.runs.push(Run::Text {
                text: (*text).to_owned(),
                style: None,
                href: None,
            });
            d.blocks.push(block);
        }
        d
    }

    #[test]
    fn a_block_address_is_one_based_outside_and_zero_based_inside() {
        assert_eq!(
            parse("p1"),
            Ok(Loc::Block {
                index: 0,
                offset: None
            })
        );
        assert_eq!(
            parse("p12+40"),
            Ok(Loc::Block {
                index: 11,
                offset: Some(40)
            })
        );
        assert_eq!(format(0), "p1");
        assert_eq!(format_offset(11, 40), "p12+40");
        // The 1-based spelling has no zero, and saying so beats an off-by-one.
        assert!(parse("p0").is_err());
    }

    #[test]
    fn the_spellings_that_are_not_addresses_are_refused() {
        for bad in [
            "", "12", "px", "p1+", "p1+x", "#", "\u{a7}", "\u{a7}0", "q1",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn a_bookmark_resolves_to_wherever_it_now_is() {
        let mut d = doc(&[(0, "one"), (0, "two")]);
        d.blocks[1].runs.push(Run::Bookmark {
            name: "here".to_owned(),
        });
        d.reindex_bookmarks();
        assert_eq!(resolve(&d, &parse("#here").unwrap()), Ok(1));

        // Insert above it: the *index* of the bookmark moved, and the address did not have to.
        let id = d.next_id();
        d.blocks.insert(0, Block::new(id, BlockKind::Paragraph));
        assert_eq!(
            resolve(&d, &parse("#here").unwrap()),
            Ok(2),
            "an anchor survives an edit above it; p2 would not have"
        );
        assert!(resolve(&d, &parse("#gone").unwrap()).is_err());
    }

    #[test]
    fn an_outline_path_counts_headings_within_their_parent() {
        let d = doc(&[
            (1, "One"),         // §1     idx 0
            (0, "text"),        //        idx 1
            (2, "One.One"),     // §1.1   idx 2
            (2, "One.Two"),     // §1.2   idx 3
            (3, "One.Two.One"), // §1.2.1 idx 4
            (1, "Two"),         // §2     idx 5
            (2, "Two.One"),     // §2.1   idx 6
        ]);
        for (spelling, index) in [
            ("\u{a7}1", 0),
            ("\u{a7}1.1", 2),
            ("\u{a7}1.2", 3),
            ("\u{a7}1.2.1", 4),
            ("\u{a7}2", 5),
            ("\u{a7}2.1", 6),
        ] {
            assert_eq!(
                resolve(&d, &parse(spelling).unwrap()),
                Ok(index),
                "{spelling}"
            );
        }
        // The count restarts inside each parent, which is the whole point.
        assert!(resolve(&d, &parse("\u{a7}2.2").unwrap()).is_err());
        assert!(resolve(&d, &parse("\u{a7}3").unwrap()).is_err());
    }

    #[test]
    fn an_outline_path_round_trips_through_its_own_index() {
        let d = doc(&[(1, "One"), (2, "A"), (2, "B"), (1, "Two")]);
        for index in [0usize, 1, 2, 3] {
            let path = outline_path(&d, index).expect("every block here is a heading");
            assert_eq!(
                resolve(&d, &parse(&format_outline(&path)).unwrap()),
                Ok(index),
                "{path:?}"
            );
        }
        // A paragraph has no outline path.
        let d = doc(&[(0, "just text")]);
        assert_eq!(outline_path(&d, 0), None);
    }

    #[test]
    fn s_is_accepted_where_the_section_sign_is_not_on_the_keyboard() {
        assert_eq!(parse("s2.1"), parse("\u{a7}2.1"));
        assert_eq!(parse("S2"), parse("\u{a7}2"));
        // But `s` alone is not an outline path, and must not swallow a bare word.
        assert!(parse("section").is_err());
    }

    #[test]
    fn a_range_covers_both_ends_and_refuses_to_run_backwards() {
        let d = doc(&[(0, "a"), (0, "b"), (0, "c"), (0, "d")]);
        assert_eq!(resolve_range(&d, &parse_range("p2:p3").unwrap()), Ok(1..3));
        // A single address is a range of one.
        assert_eq!(resolve_range(&d, &parse_range("p2").unwrap()), Ok(1..2));
        assert!(resolve_range(&d, &parse_range("p3:p2").unwrap()).is_err());
        assert!(resolve_range(&d, &parse_range("p1:p99").unwrap()).is_err());
    }
}
