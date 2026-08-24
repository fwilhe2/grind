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
//! | `#intro` | wherever the `text:bookmark` named `intro` is |
//! | `§2.1.3` | outline path — chapter 2, section 1, subsection 3 |
//! | `…+40` | character offset 40 into whichever of those it follows |
//!
//! The middle two are the interesting ones. `p12` is what a machine uses and is invalidated by
//! every insertion above it; `#intro` and `§2.1.3` are what a *person* or a *script* uses and
//! survive editing elsewhere in the document. `grind text set report.fodt §3.2 "…"` still
//! works next week, which is a capability the CLI has and no word processor's UI does.
//!
//! **The offset is its own axis**, which is why it is a field of [`Loc`] rather than a variant:
//! `§3.2+0` and `#intro+5` are as good addresses as `p12+40`, and they are the ones worth
//! having, because a caret pinned to a block number is invalidated by the same insertion that
//! `#intro` was invented to survive. `grind text type report.fodt '#intro+0' 'NOTE: '` means
//! the same thing next week; `p12+0` does not.
//!
//! An offset makes an address a **caret** — a place *between* characters, not a character —
//! which is why `+0` is the front of the block and `+<len>` is the back of it, and why an
//! offset past the end clamps instead of failing. [`resolve_caret`] is where one becomes a
//! [`Caret`].
//!
//! `§` is accepted as `s` too, because it is not on most keyboards.

use crate::model::{Block, BlockKind, Document};

/// A place in the document: a block, and a character offset within it. **A caret.**
///
/// What an address with an offset names once it has been resolved, and what a shell holds as
/// its cursor. Ordered by document position, so `from <= to` is the whole check a range needs.
///
/// Both numbers are 0-based, like everything inside this crate; [`format_offset`] is where they
/// become the `p12+40` a person types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Caret {
    pub block: usize,
    pub offset: usize,
}

/// What an address points *at*, before any offset into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `p12` — a block by position, **0-based here**, 1-based in the spelling.
    Block(usize),
    /// `#intro` — a bookmark by name.
    Bookmark(String),
    /// `§2.1.3` — an outline path, each component 1-based.
    Outline(Vec<u32>),
}

/// Where something is, before it has been resolved against a document.
///
/// Two independent parts on purpose. *Which block* has three spellings with very different
/// lifetimes; *how far into it* is one number that means the same thing after all three. Making
/// the offset a field rather than folding it into the block spelling is what lets `#intro+5`
/// exist, and `#intro+5` is the address a script actually wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loc {
    pub target: Target,
    /// A character offset into the block, when the address carried one.
    ///
    /// `None` is not "0": it is *unspecified*, and the two resolvers read it differently —
    /// [`resolve_caret`] takes it as the front of the block, [`resolve_caret_range`] takes it
    /// as the front at the start of a range and the **back** at the end of one, so that a bare
    /// `p3` covers the whole of p3.
    pub offset: Option<usize>,
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
    let (text, offset) = split_offset(text);
    Ok(Loc {
        target: parse_target(text)?,
        offset,
    })
}

/// Peel a trailing `+<digits>` off an address.
///
/// **Only digits count.** A bookmark name is arbitrary text out of somebody's document, so
/// `#a+b` has to stay the bookmark named `a+b` rather than becoming a parse error — the rule is
/// that a `+` followed by a number is an offset and every other `+` is part of the name. The
/// one thing this costs is a bookmark whose name genuinely ends in `+12`, which is not
/// addressable and is written down here rather than discovered.
fn split_offset(text: &str) -> (&str, Option<usize>) {
    match text.rsplit_once('+') {
        Some((head, tail)) if !head.is_empty() => match tail.parse::<usize>() {
            Ok(offset) => (head, Some(offset)),
            Err(_) => (text, None),
        },
        _ => (text, None),
    }
}

fn parse_target(text: &str) -> Result<Target, ParseError> {
    if let Some(name) = text.strip_prefix('#') {
        if name.is_empty() {
            return Err(bad(text));
        }
        return Ok(Target::Bookmark(name.to_owned()));
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
        return Ok(Target::Outline(path));
    }

    // `p12`.
    let number = text
        .strip_prefix('p')
        .or_else(|| text.strip_prefix('P'))
        .ok_or_else(|| bad(text))?;
    let number: usize = number.parse().map_err(|_| bad(text))?;
    if number == 0 {
        return Err(ParseError(
            "blocks are numbered from 1, so there is no p0".to_owned(),
        ));
    }
    Ok(Target::Block(number - 1))
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
    match &loc.target {
        Target::Block(index) => {
            if *index >= doc.blocks.len() {
                return Err(ParseError(format!(
                    "{} is past the end; the document has {} block(s)",
                    format(*index),
                    doc.blocks.len()
                )));
            }
            Ok(*index)
        }
        Target::Bookmark(name) => {
            let id = doc
                .bookmarks
                .get(name)
                .ok_or_else(|| ParseError(format!("no bookmark named {name}")))?;
            doc.index_of(*id)
                .ok_or_else(|| ParseError(format!("bookmark {name} points nowhere")))
        }
        Target::Outline(path) => outline_index(doc, path)
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
/// The inverse of the walk above, and the reason `grind text outline` can print an address a
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

/// Resolve a location to a [`Caret`] — a block **and an offset within it**.
///
/// An address with no offset of its own — `p12`, `#intro`, `§2.1.3` — names the *start* of its
/// block, which is what "insert here" means for an address that does not say otherwise.
///
/// **An offset past the end of the block is clamped to it** rather than refused. A caret is a
/// place in a document that can shrink underneath it — a script computes `p3+40` from `grind
/// text find`, something deletes half of p3, and the address is now past the end without being
/// wrong about anything. Clamping puts the caret at the end of the text, which is where a
/// person watching would expect it.
pub fn resolve_caret(doc: &Document, loc: &Loc) -> Result<Caret, ParseError> {
    let block = resolve(doc, loc)?;
    let len = doc.block(block).map_or(0, Block::len);
    Ok(Caret {
        block,
        offset: loc.offset.unwrap_or(0).min(len),
    })
}

/// Resolve a range of *characters* — what `grind text erase` takes.
///
/// The one place this differs from [`resolve_range`], and it is deliberate: an end that carries
/// no offset means **the end of its block** rather than the start. So `p3` is all of p3's text,
/// `p3+12:p3+20` is eight characters of it, and `p3:p5` is everything from the front of p3 to
/// the back of p5. Every spelling then means the span a person would point at.
///
/// A block range is a different question and keeps its own resolver — `p3:p5` there is three
/// whole blocks, and a heading address there is its whole section.
pub fn resolve_caret_range(doc: &Document, range: &Range) -> Result<(Caret, Caret), ParseError> {
    let start = resolve_caret(doc, &range.start)?;
    let block = resolve(doc, &range.end)?;
    let len = doc.block(block).map_or(0, Block::len);
    let end = Caret {
        block,
        offset: range.end.offset.unwrap_or(len).min(len),
    };
    if end < start {
        return Err(ParseError(format!(
            "{} comes after {}",
            format_offset(start.block, start.offset),
            format_offset(end.block, end.offset)
        )));
    }
    Ok((start, end))
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
    use crate::model::Run;

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

    fn loc(target: Target, offset: Option<usize>) -> Result<Loc, ParseError> {
        Ok(Loc { target, offset })
    }

    #[test]
    fn a_block_address_is_one_based_outside_and_zero_based_inside() {
        assert_eq!(parse("p1"), loc(Target::Block(0), None));
        assert_eq!(parse("p12+40"), loc(Target::Block(11), Some(40)));
        assert_eq!(format(0), "p1");
        assert_eq!(format_offset(11, 40), "p12+40");
        // The 1-based spelling has no zero, and saying so beats an off-by-one.
        assert!(parse("p0").is_err());
    }

    /// The offset is its own axis, so it composes with **every** spelling — which is the point
    /// of the restructure: a caret address that only works on `p12` is invalidated by the same
    /// insertion `#intro` exists to survive.
    #[test]
    fn an_offset_composes_with_every_way_of_naming_a_block() {
        assert_eq!(
            parse("#intro+5"),
            loc(Target::Bookmark("intro".to_owned()), Some(5))
        );
        assert_eq!(
            parse("\u{a7}2.1+0"),
            loc(Target::Outline(vec![2, 1]), Some(0))
        );
        // A `+` that is not followed by a number is part of the name, because a bookmark name
        // is arbitrary text out of somebody's document.
        assert_eq!(parse("#a+b"), loc(Target::Bookmark("a+b".to_owned()), None));
        // And an offset is still refused where the rest of the address is nonsense.
        assert!(parse("+3").is_err());
        assert!(parse("q1+3").is_err());
    }

    #[test]
    fn an_offset_resolves_against_any_spelling_of_the_block() {
        let mut d = doc(&[(1, "Chapter"), (0, "body text")]);
        d.blocks[1].runs.insert(
            0,
            Run::Bookmark {
                name: "here".to_owned(),
            },
        );
        d.reindex_bookmarks();

        let caret = |s: &str| resolve_caret(&d, &parse(s).unwrap()).unwrap();
        let want = Caret {
            block: 1,
            offset: 4,
        };
        assert_eq!(caret("p2+4"), want);
        assert_eq!(
            caret("#here+4"),
            want,
            "the address that survives an edit above"
        );
        // §1 is the heading; its section's body is not addressable this way, which is correct —
        // an outline path names a heading, and +4 is four characters into that heading.
        assert_eq!(
            caret("\u{a7}1+4"),
            Caret {
                block: 0,
                offset: 4
            }
        );
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
    fn an_address_without_an_offset_means_the_front_of_its_block() {
        let d = doc(&[(0, "hello"), (0, "world")]);
        let caret = |s: &str| resolve_caret(&d, &parse(s).unwrap()).unwrap();
        assert_eq!(
            caret("p2"),
            Caret {
                block: 1,
                offset: 0
            }
        );
        assert_eq!(
            caret("p2+3"),
            Caret {
                block: 1,
                offset: 3
            }
        );
        // Clamped, not refused: the document can shrink under an address that was right when
        // it was written.
        assert_eq!(
            caret("p2+99"),
            Caret {
                block: 1,
                offset: 5
            }
        );
    }

    #[test]
    fn a_character_range_with_no_offsets_is_the_whole_block() {
        let d = doc(&[(0, "hello"), (0, "world")]);
        let span = |s: &str| resolve_caret_range(&d, &parse_range(s).unwrap()).unwrap();
        // The difference from `resolve_range`, and the reason it is a second function: the end
        // means the *end* of its block, so a bare address covers the text a person points at.
        assert_eq!(
            span("p1"),
            (
                Caret {
                    block: 0,
                    offset: 0
                },
                Caret {
                    block: 0,
                    offset: 5
                }
            )
        );
        assert_eq!(
            span("p1+1:p1+3"),
            (
                Caret {
                    block: 0,
                    offset: 1
                },
                Caret {
                    block: 0,
                    offset: 3
                }
            )
        );
        assert_eq!(
            span("p1+2:p2"),
            (
                Caret {
                    block: 0,
                    offset: 2
                },
                Caret {
                    block: 1,
                    offset: 5
                }
            )
        );
        // Backwards within one block is caught too, which a block-level range cannot see.
        assert!(resolve_caret_range(&d, &parse_range("p1+4:p1+2").unwrap()).is_err());
        assert!(resolve_caret_range(&d, &parse_range("p2:p1").unwrap()).is_err());
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
