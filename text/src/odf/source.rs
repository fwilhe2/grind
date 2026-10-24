// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The file a document came from, kept so that saving can edit it instead of replacing it (R6):
//! editing one paragraph of a long `.fodt` changes one line of `git diff`, and a writer that
//! touches only what changed cannot lose what it never understood — change tracking, indexes,
//! sections, frames, fields, other vendors' extensions.
//!
//! * Content edits splice — retyping, restyling or changing a block's kind replaces an element the
//!   file spells. Inserting, deleting or moving a block changes the sequence, and the body
//!   regenerates for the file it came from (`odf::write`'s `Origin`): untouched blocks are their
//!   own bytes, lists open with their [`Source::lists`] attributes, and each [`Sibling`] goes back
//!   after the block it followed.
//! * A package splices its `content.xml` ([`Source::bytes`]) and rebuilds the archive from its own
//!   entries ([`Source::package`], `grind_core::odf::envelope::repackage`).
//!
//! A regenerated body is merged into the original (`grind_core::odf::envelope::merge`), so
//! everything outside `office:body` is the file's own, and what an edited block cannot carry is an
//! `Error::WouldLose`.

use std::collections::HashMap;
use std::ops::Range;

use grind_core::odf::Form;

use crate::model::BlockId;
use crate::style::CharStyle;

/// An automatic text style the source file already declares: its name, parent and what it sets. A
/// spliced paragraph refers to style names that must exist in bytes the splice does not touch, so
/// an edit whose formatting the file already names reuses that name and splices, and one that
/// needs a new declaration regenerates.
#[derive(Clone, Debug)]
pub struct TextStyle {
    pub name: String,
    pub parent: Option<String>,
    pub props: CharStyle,
    /// Whether it says anything `props` does not carry — superscript, a language — so it is never
    /// reused for formatting that only looks the same ([`Source::style_named`]), only as a
    /// restyled run's base.
    pub extras: bool,
}

/// The bytes a document was read from, plus where its blocks are in them.
#[derive(Clone, Debug)]
pub struct Source {
    /// Which physical form those bytes are; splicing is refused for any other.
    pub form: Form,
    /// The file exactly as it was read. For the flat form this is also `content.xml`, which is
    /// why the ranges below index straight into it.
    pub bytes: Vec<u8>,
    /// Where each block's element sits, keyed by [`BlockId`] since an index changes with every
    /// insertion above it.
    pub blocks: HashMap<BlockId, Block>,
    /// The automatic character styles the file declares, in order ([`TextStyle`]).
    pub styles: Vec<TextStyle>,
    /// The whole archive, when the document came from a package — [`Source::bytes`] is then its
    /// `content.xml` — so every other entry is written back on save.
    pub package: Option<Vec<u8>>,
    /// The elements of `office:text` the model has no block for, in file order, so a regenerated
    /// body puts each back after the block it followed.
    pub siblings: Vec<Sibling>,
    /// The `text:list` elements around each list item, outermost first: where each starts in
    /// [`Source::bytes`] (its identity) and its attributes verbatim — the list style, its
    /// `xml:id`. A regenerated body opens its lists with these.
    pub lists: HashMap<BlockId, Vec<(usize, String)>>,
    /// Every picture's outermost `draw:frame` extent in [`Source::bytes`], keyed by what it read
    /// as ([`image_key`]). The model keeps a picture's bytes, size and anchor; a regenerated
    /// paragraph writes the file's own frame for a picture that is unchanged — its name, style,
    /// nested resizing frame and caption, `Pictures/` reference — wherever the edit moved it.
    pub frames: Vec<(u64, Range<usize>)>,
    /// Every top-level `table:table` by name, as the file spelled it, so a regenerated body writes
    /// an untouched table as its own bytes and an edited one with its own styles.
    pub tables: HashMap<String, TableSource>,
}

/// One table as the file spelled it ([`Source::tables`]). What makes it look like itself — column
/// widths, borders, a heading row, a fill — is style names on the table, its columns, rows and
/// cells, which the model does not carry.
#[derive(Clone, Debug, Default)]
pub struct TableSource {
    /// The whole element, for writing an untouched table back as it was.
    pub range: Option<Range<usize>>,
    /// The blocks it held when read, in order — what "untouched" is measured against.
    pub blocks: Vec<BlockId>,
    /// Its start tag, verbatim.
    pub start: String,
    /// Everything between the start tag and the first row: the column declarations and
    /// whatever else the file put there, verbatim.
    pub columns: String,
    /// The rows inside `table:table-header-rows`.
    pub header_rows: std::collections::BTreeSet<u32>,
    /// Each row's start tag, by row index (the first row of a repeated one).
    pub rows: HashMap<u32, String>,
    /// Each cell's start tag — covered ones too — by position, written in its open form.
    pub cells: HashMap<(u32, u32), String>,
}

/// What a picture read as — type, bytes, size, anchor and alternative text — as one number, so the
/// writer can tell whether an image run is still a frame the file spelled ([`Source::frames`]).
/// `None` for any other run.
pub fn image_key(run: &crate::model::Run) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let crate::model::Run::Image {
        mime,
        data,
        width,
        height,
        anchor,
        title,
        description,
    } = run
    else {
        return None;
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (mime, data, width, height, anchor, title, description).hash(&mut hasher);
    Some(hasher.finish())
}

/// One element of the body the model does not read — the prelude (`text:sequence-decls`,
/// `text:variable-decls`), a `text:section`, a `text:table-of-content`, a vendor's own — carried
/// through verbatim at the same place in the sequence.
#[derive(Clone, Debug)]
pub struct Sibling {
    /// The block it came after, or `None` for one ahead of every block.
    pub after: Option<BlockId>,
    /// Its extent in [`Source::bytes`].
    pub range: Range<usize>,
}

impl Source {
    /// The automatic style a run's formatting was read from, when it says more than the model
    /// reads ([`CharStyle::origin`]) — what a restyle of that run is built from.
    pub fn base_of(&self, props: &CharStyle) -> Option<&TextStyle> {
        let name = props.origin.name()?;
        self.styles.iter().find(|style| style.name == name)
    }

    /// The name this file already uses for exactly this formatting, if any. Parentless only: the
    /// writer emits character styles that inherit from nothing and puts a named style in a span
    /// around them, so reusing one with a parent would apply the parent twice.
    pub fn style_named(&self, props: &CharStyle) -> Option<&str> {
        self.styles
            .iter()
            .find(|style| style.parent.is_none() && !style.extras && style.props.same(props))
            .map(|style| style.name.as_str())
    }
}

/// One block element of the source file.
#[derive(Clone, Debug)]
pub struct Block {
    /// The element's extent in [`Source::bytes`], start tag through end tag.
    pub range: Range<usize>,
    /// Every attribute of the original start tag the writer does not produce itself, spelled as
    /// the file spelled it — `text:class-names`, `text:cond-style-name`, `xml:id`, a vendor's own
    /// — which re-deriving the tag from the model would drop.
    pub keep: String,
    /// How many lists it sat in, and which table cell: context a splice cannot change, so a block
    /// whose context moved regenerates the body.
    pub depth: u32,
    pub cell: Option<crate::model::Cell>,
}

/// The attributes of a block's start tag minus the ones the writer produces from the model, sliced
/// verbatim so they keep the document's prefixes.
pub fn kept_attributes(start_tag: &[u8]) -> String {
    // What the writer always emits, and what would therefore appear twice.
    attributes(start_tag, &["text:style-name", "text:outline-level"])
}

/// Every attribute of a start tag except those in `drop`, as the file spelled them — a list's
/// `text:style-name` and `xml:id`, for the `text:list` a regenerated body opens.
pub fn attributes(start_tag: &[u8], drop: &[&str]) -> String {
    let Ok(tag) = std::str::from_utf8(start_tag) else {
        return String::new();
    };
    // Past `<text:p`, and stopping before the `/>` or `>` that closes it.
    let Some(body) = tag.find(char::is_whitespace).map(|i| &tag[i..]) else {
        return String::new();
    };
    let body = body.trim_end_matches('>').trim_end_matches('/');

    let mut out = String::new();
    let mut rest = body;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim();
        let after = &rest[eq + 1..];
        // An attribute value is quoted and cannot contain its own quote character, so the next
        // one of the same kind ends it — `>` and `<` inside notwithstanding.
        let Some(quote) = after.chars().find(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let Some(open) = after.find(quote) else { break };
        let Some(len) = after[open + 1..].find(quote) else {
            break;
        };
        let end = open + 1 + len + 1;
        if !name.is_empty() && !drop.contains(&name) {
            out.push(' ');
            out.push_str(name);
            out.push('=');
            out.push_str(&after[open..end]);
        }
        rest = &after[end..];
    }
    out
}

impl Source {
    pub fn new(form: Form, bytes: Vec<u8>) -> Self {
        Self {
            form,
            bytes,
            blocks: HashMap::new(),
            styles: Vec::new(),
            package: None,
            siblings: Vec::new(),
            lists: HashMap::new(),
            frames: Vec::new(),
            tables: HashMap::new(),
        }
    }
}

/// What has changed since the document was read: whether the block sequence moved.
#[derive(Clone, Debug, Default)]
pub struct Edits {
    /// Blocks whose content changed but which are still in the document.
    pub blocks: std::collections::BTreeSet<BlockId>,
    /// Whether the block sequence changed — an insertion, a deletion, a move. Sticky: once the
    /// sequence has moved, a patch list over the original bytes no longer describes the document.
    pub structural: bool,
}
