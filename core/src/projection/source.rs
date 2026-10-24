// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The projection a document was read from, kept so that saving can edit it. **\[GENERIC\]**
//!
//! R6 for the third physical form (`doc/dsl.md` §3.1): retain the bytes, remember where each
//! address sat in them, and splice the ones that changed, so comments, blank lines and alignment
//! survive. The reader already computes an address for every node it walks, and `kdl` hands over
//! the spans.
//!
//! Not `kdl`'s mutation API: in `kdl 6.7.1`, setting an entry's value leaves the printed output
//! unchanged, and clearing its format to force a reprint loses the whitespace around it
//! (`doc/dsl.md` §3.1). Values are spelled by [`super::emit::repr`], shared with the writer.
//!
//! Each of these regenerates the whole document rather than splicing:
//!
//! * an address the file did not spell (a cell that was empty, a block appended);
//! * an edit that changes a site's [`Shape`] (a plain number that becomes a formula);
//! * a save to another form.
//!
//! *ponytail:* one `String` key per address, so a hundred thousand cells carry a hundred thousand
//! small allocations. `odf/source.rs` keys per row instead; the same would work here with the
//! application's row concept, and waits for a document that hurts.

use std::collections::HashMap;
use std::ops::Range;

use kdl::{KdlEntry, KdlNode};

/// Where a node sits in the text it was parsed from — its name through its closing brace, without
/// the indentation before it and the newline after it. A range rather than `kdl`'s
/// `miette::SourceSpan`, so the applications need not depend on `miette`.
pub fn node_span(node: &KdlNode) -> Range<usize> {
    let span = node.span();
    span.offset()..span.offset() + span.len()
}

/// Where one value on a node sits — quotes included, whitespace around it excluded.
pub fn entry_span(entry: &KdlEntry) -> Range<usize> {
    let span = entry.span();
    span.offset()..span.offset() + span.len()
}

/// What a site can be replaced by, which is what it already is: a whole node, or one value on a
/// node (a grid row's cell, a paragraph's text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The span is a whole node, indentation and terminator excluded. A replacement is a node.
    Node,
    /// The span is one value on a node. A replacement is a value, spelled by
    /// [`super::emit::repr`].
    Value,
}

/// Where one address sits in the retained text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub span: Range<usize>,
    pub shape: Shape,
}

/// Where a stretch of a file is, as an editor's gutter numbers it: lines and columns from 1,
/// columns in characters rather than bytes, which is what `file:line:col` means to VS Code, Vim
/// and Emacs. A language server converts to UTF-16 from the span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Location {
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl Location {
    /// The location of `span` in `text`. Linear in the file per call: fine for a capped lint
    /// report, not for anything per keystroke.
    pub fn of(text: &str, span: Range<usize>) -> Self {
        let point = |offset: usize| {
            let before = &text[..offset];
            let line_start = before.rfind('\n').map_or(0, |at| at + 1);
            (
                before.matches('\n').count() + 1,
                before[line_start..].chars().count() + 1,
            )
        };
        let (line, column) = point(span.start);
        let (end_line, end_column) = point(span.end);
        Location {
            line,
            column,
            end_line,
            end_column,
        }
    }
}

/// The text a document was read from, and where its addresses are in it (addresses are strings for
/// [`super::Anchor`]'s reason).
#[derive(Clone, Debug, Default)]
pub struct Source {
    text: String,
    sites: HashMap<String, Site>,
}

impl Source {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            sites: HashMap::new(),
        }
    }

    /// Note that `address` is projected by this stretch of the text. An address recorded twice
    /// keeps its first span; a hand-written file that spells one twice regenerates rather than
    /// splicing the wrong site.
    pub fn record(&mut self, address: impl Into<String>, span: Range<usize>, shape: Shape) {
        self.sites
            .entry(address.into())
            .or_insert(Site { span, shape });
    }

    /// The bytes as they were read.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where an address is, if the file spelled it at all.
    pub fn site(&self, address: &str) -> Option<&Site> {
        self.sites.get(address)
    }

    /// Where an address is, as a line and column in the text as read — after an unsaved edit,
    /// still the place in the file on disk. `None` for an address the file did not spell.
    pub fn locate(&self, address: &str) -> Option<Location> {
        self.site(address)
            .map(|site| Location::of(&self.text, site.span.clone()))
    }

    /// Every address the file spelled and where, in file order — [`super::Projection::anchors`]
    /// for the file somebody wrote rather than the one this crate writes. What a language server's
    /// hover and inline values use.
    pub fn anchors(&self) -> Vec<super::Anchor> {
        let mut anchors: Vec<super::Anchor> = self
            .sites
            .iter()
            .map(|(address, site)| super::Anchor {
                address: address.clone(),
                span: site.span.clone(),
            })
            .collect();
        anchors.sort_by(|a, b| {
            (a.span.start, a.span.end, &a.address).cmp(&(b.span.start, b.span.end, &b.address))
        });
        anchors
    }

    /// How many addresses were recorded.
    pub fn len(&self) -> usize {
        self.sites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    /// The text with each range replaced, or `None` — refuse and regenerate — if the ranges
    /// overlap or run past the end, which would mean the map and the text have come apart.
    pub fn splice(&self, mut patches: Vec<(Range<usize>, String)>) -> Option<String> {
        patches.sort_by_key(|(range, _)| range.start);
        if patches.windows(2).any(|w| w[0].0.end > w[1].0.start) {
            return None;
        }
        let mut out = String::with_capacity(self.text.len());
        let mut at = 0usize;
        for (range, replacement) in patches {
            out.push_str(self.text.get(at..range.start)?);
            out.push_str(&replacement);
            at = range.end;
        }
        out.push_str(self.text.get(at..)?);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        let mut source = Source::new("one two three\n");
        source.record("a", 0..3, Shape::Value);
        source.record("b", 4..7, Shape::Value);
        source.record("c", 8..13, Shape::Node);
        source
    }

    #[test]
    fn splicing_replaces_the_ranges_and_copies_everything_else() {
        let text = source()
            .splice(vec![(8..13, "THREE".to_owned()), (0..3, "ONE".to_owned())])
            .expect("two disjoint ranges");
        assert_eq!(text, "ONE two THREE\n", "out of order in, in order out");
    }

    #[test]
    fn an_address_is_located_by_line_and_character_column() {
        let mut source = Source::new("// é\ncell B5 \"café\"\n  row 1 2\n");
        source.record("B5", 6..21, Shape::Node);
        source.record("C7", 28..29, Shape::Value);
        let b5 = source.locate("B5").expect("recorded");
        assert_eq!(
            (b5.line, b5.column, b5.end_line, b5.end_column),
            (2, 1, 2, 15)
        );
        let c7 = source.locate("C7").expect("recorded");
        assert_eq!((c7.line, c7.column), (3, 7), "after the indentation");
        assert_eq!(source.locate("Z9"), None, "a file that never spelled it");
        // `é` is two bytes and one column, so a byte count would put the mark one too far right.
        let mut accent = Source::new("é x");
        accent.record("x", 3..4, Shape::Value);
        assert_eq!(accent.locate("x").expect("recorded").column, 3);
    }

    #[test]
    fn the_anchors_are_the_sites_in_file_order() {
        let anchors: Vec<(String, Range<usize>)> = source()
            .anchors()
            .into_iter()
            .map(|a| (a.address, a.span))
            .collect();
        assert_eq!(
            anchors,
            [
                ("a".to_owned(), 0..3),
                ("b".to_owned(), 4..7),
                ("c".to_owned(), 8..13)
            ]
        );
    }

    #[test]
    fn nothing_spliced_is_the_file_that_came_in() {
        assert_eq!(
            source().splice(Vec::new()).expect("nothing"),
            "one two three\n"
        );
    }

    #[test]
    fn overlapping_patches_are_refused_rather_than_tangled() {
        assert_eq!(
            source().splice(vec![(0..5, "x".to_owned()), (4..7, "y".to_owned())]),
            None
        );
        assert_eq!(
            source().splice(vec![(0..99, "x".to_owned())]),
            None,
            "past the end"
        );
    }

    #[test]
    fn the_first_span_for_an_address_is_the_one_kept() {
        let mut source = Source::new("a a\n");
        source.record("dup", 0..1, Shape::Value);
        source.record("dup", 2..3, Shape::Node);
        assert_eq!(
            source.site("dup"),
            Some(&Site {
                span: 0..1,
                shape: Shape::Value
            })
        );
        assert_eq!(source.site("nothing"), None);
        assert_eq!(source.len(), 1);
    }
}
