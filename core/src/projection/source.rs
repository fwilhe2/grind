// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The projection a document was read from, kept so that saving can edit it. **\[GENERIC\]**
//!
//! This is R6 (`doc/plan.md`) applied to the third physical form, and `doc/dsl.md` §3.1 promised
//! it before either projection existed: *"a `.grind` file will have comments, blank lines and
//! hand-chosen alignment in it, none of which exist in the ODF model. They survive an edit for
//! exactly the reason `office:settings` survives an edit to a `.fods`: the writer never
//! regenerates what nobody touched."*
//!
//! **The same trick as `odf/source.rs`, and deliberately so.** Retain the bytes, remember where
//! each address sat in them, and splice the ones that changed. What is different is how cheap
//! the map is to build: the projection *reader* already computes an address for every node it
//! walks — that is what makes the format bijective — so recording one more span per address is
//! bookkeeping rather than a second pass. `kdl` hands over the spans, which is the same property
//! that makes it formatting-preserving.
//!
//! **Not `kdl`'s own mutation API, though it was the obvious route.** Probed against `kdl 6.7.1`:
//! setting an entry's value leaves the printed output *unchanged*, because a parsed entry retains
//! the text it was spelled as and that text wins; clearing the format to force a reprint also
//! clears the whitespace around the value, so `row "North"  4200` loses its hand alignment. A
//! byte splice at the span `kdl` reports keeps both, needs no second document model in memory,
//! and is the machinery this project has already built twice. What `kdl` is still the authority
//! on is where the spans *are* and how a value is spelled — [`super::emit::repr`] is the one
//! function that answers the second, shared with the writer so the two cannot disagree.
//!
//! **Three honest boundaries**, each of which regenerates rather than failing:
//!
//! * **An address the file did not spell has no site.** A cell that was empty and now is not,
//!   a block appended at the end — there is no span to splice into, so the document is written
//!   out whole. A half-spliced file would be worse than a large diff.
//! * **A site has a shape.** A value in a grid row can only be replaced by a value; a node can
//!   only be replaced by a node. An edit that changes which of the two an address needs — a
//!   plain number that becomes a formula — is a structural change and regenerates.
//! * **Only the projection.** A `.grind` read and saved as a `.fods` is a conversion, and the
//!   form the bytes came from is not the form they are going to.
//!
//! *ponytail:* one `String` key per address, so a projection of a hundred thousand cells carries
//! a hundred thousand small allocations beside the text. `odf/source.rs` avoids that by keying
//! per *row* and scanning a short list; the same shape would work here and needs a row concept,
//! which is the application's. The ceiling is a document nobody hand-writes, which is the only
//! kind this form is for — and it is a size question rather than a correctness one, so it waits
//! for a document that actually hurts.

use std::collections::HashMap;
use std::ops::Range;

use kdl::{KdlEntry, KdlNode};

/// Where a node sits in the text it was parsed from — its name through its closing brace, with
/// the indentation before it and the newline after it left out.
///
/// A `Range<usize>` rather than the `miette::SourceSpan` `kdl` hands back, because every other
/// span in this workspace is a range and one spelling is enough. Here rather than in an
/// application for a smaller reason that matters as much: converting it there would make
/// `miette` a direct dependency of both application crates to name one type.
pub fn node_span(node: &KdlNode) -> Range<usize> {
    let span = node.span();
    span.offset()..span.offset() + span.len()
}

/// Where one value on a node sits — quotes included, whitespace around it excluded.
pub fn entry_span(entry: &KdlEntry) -> Range<usize> {
    let span = entry.span();
    span.offset()..span.offset() + span.len()
}

/// What a site can be replaced by, which is what it already is.
///
/// KDL's vocabulary rather than any document type's (R8): whether an address is projected as a
/// whole node or as one value on somebody else's node is a question about the *syntax*, and both
/// applications answer it — a spreadsheet's grid row holds one cell per value, and a paragraph's
/// text is one value on the node that is the paragraph.
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

/// The text a document was read from, and where its addresses are in it.
///
/// The address is a `String` for [`super::Anchor`]'s reason and no other: `Sheet1.B5` and `p12`
/// are two applications' spellings of a place, this crate is not allowed to know either, and
/// carrying one is all a splice needs — the app hands back the same string it recorded.
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

    /// Note that `address` is projected by this stretch of the text.
    ///
    /// Recorded once per address: an address that appears twice keeps the **first** span, because
    /// a reader that has already built the model from the earlier one is describing the state the
    /// later one would overwrite. In practice neither projection spells an address twice, and a
    /// hand-written file that does is exactly the case where regenerating is the safer answer —
    /// which is what a splice of the wrong site would quietly avoid.
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

    /// How many addresses were recorded — what a test asserts a reader did not quietly stop
    /// doing.
    pub fn len(&self) -> usize {
        self.sites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    /// The text with each range replaced, or `None` if the patches do not describe one file.
    ///
    /// `None` is *refuse and regenerate*, never an error: overlapping ranges would produce
    /// tangled bytes rather than a diagnostic, and a range past the end means the map and the
    /// text have come apart. Both are impossible by construction — sites are siblings in one
    /// parse — and both are cheap to check, which is the trade `odf::write::splice` already made.
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
