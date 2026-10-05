// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The file a document came from, kept so that saving can edit it instead of replacing it.
//!
//! This is **R6** (doc/plan.md) for text documents, and `doc/suite.md` argues it is worth more
//! here than it was for the spreadsheet: *a word processor whose files live in git.* Editing
//! one paragraph of a two-hundred-page `.fodt` changes one line of `git diff`, and opening a
//! document to read it is not a commit.
//!
//! It is also the fidelity requirement wearing different clothes, and a text document makes
//! that sharper. A real `.odt` carries change tracking, six kinds of index, sections, frames,
//! fields and another vendor's extensions — far more content this build has no model for than
//! any spreadsheet has. **A writer that touches only what changed cannot lose what it never
//! understood**, which is why `doc/text-core.md` can put ten of sixteen block types out of
//! scope and still be a good custodian of them.
//!
//! **Retain and splice, not a fuller model** — `grind_sheet::odf::source` makes the argument
//! and it is unchanged: carrying every unknown element as a shadow tree grows the model to the
//! size of ODF, which is the trade this project exists not to make.
//!
//! Two boundaries, each a documented property of the trick rather than a corner cut:
//!
//! * **Only content edits splice.** Retyping a paragraph, restyling one, changing what kind of
//!   block it is — those replace an element the file already spells. *Inserting* a block,
//!   deleting one or moving one changes the **sequence**, and the body regenerates — but for
//!   the file it came from (`odf::write`'s `Origin`): every block nobody edited is written as
//!   its own bytes, each list opens with its [`Source::lists`] attributes, and every
//!   [`Sibling`] goes back after the block it followed. An Enter in a LibreOffice document is
//!   one line out and two lines in.
//! * **A package splices its `content.xml`.** A `.odt` is a zip, so [`Source::bytes`] is the
//!   `content.xml` inside it and [`Source::package`] the archive around it; a save splices the
//!   one and rebuilds the other from its own entries (`grind_core::odf::envelope::repackage`),
//!   so `styles.xml`, `meta.xml`, `settings.xml` and the pictures come back byte for byte.
//!
//! Both fall back to regenerating the **body**, which is merged back into the original
//! (`grind_core::odf::envelope::merge`): every part outside `office:body` — styles, master
//! pages, metadata, settings — is the file's own. **Saving never makes an existing file
//! worse**: what an edited block cannot carry (a footnote inside it, say) makes the save an
//! `Error::WouldLose` rather than a smaller file.

use std::collections::HashMap;
use std::ops::Range;

use grind_core::odf::Form;

use crate::model::BlockId;
use crate::style::CharStyle;

/// An automatic text style the source file already declares: its name, what it inherits from,
/// and what it sets.
///
/// Kept so that a **formatting** edit can splice like any other. The writer names the character
/// styles it emits (`T1`, `T2`, …), and a spliced paragraph refers to names that have to exist
/// in bytes the splice does not touch — so an edit whose formatting the file already has a name
/// for reuses that name and splices, and one that needs a *new* declaration regenerates. That
/// is the same line `grind_sheet` draws for a cell style the file has no entry for, and it is
/// why `<style:style>` never has to be spliced into `office:automatic-styles`.
#[derive(Clone, Debug)]
pub struct TextStyle {
    pub name: String,
    pub parent: Option<String>,
    pub props: CharStyle,
    /// Whether it says anything `props` does not carry — superscript, a language. Such a style
    /// is never reused for formatting that merely *looks* like it to the model
    /// ([`Source::style_named`]); it is only ever a restyled run's base.
    pub extras: bool,
}

/// The bytes a document was read from, plus where its blocks are in them.
#[derive(Clone, Debug)]
pub struct Source {
    /// Which physical form those bytes are. Splicing is refused for any other, so a `.fodt`
    /// opened and saved as `.odt` regenerates rather than producing a zip full of flat XML.
    pub form: Form,
    /// The file exactly as it was read. For the flat form this is also `content.xml`, which is
    /// why the ranges below index straight into it.
    pub bytes: Vec<u8>,
    /// Where each block's element sits.
    ///
    /// Keyed by [`BlockId`] rather than by position, and that is what block ids were built for
    /// (`crate::model::BlockId`): an index is invalidated by every insertion above it, and this
    /// map has to outlive exactly those edits.
    pub blocks: HashMap<BlockId, Block>,
    /// The automatic character styles the file declares, in the order it declares them. See
    /// [`TextStyle`] for why a splice needs them.
    pub styles: Vec<TextStyle>,
    /// The whole archive, when the document came from a package — [`Source::bytes`] is then
    /// its `content.xml`. Every other entry is written back from here on save
    /// (`grind_core::odf::envelope::repackage`), so `styles.xml`, `meta.xml`, `settings.xml`
    /// and the pictures survive.
    pub package: Option<Vec<u8>>,
    /// The elements of `office:text` the model has no block for, in file order — kept so a
    /// regenerated body can put each one back after the block it followed.
    pub siblings: Vec<Sibling>,
    /// The `text:list` elements around each list item, outermost first: where each one starts
    /// in [`Source::bytes`] (its identity) and its attributes, verbatim — the list style that
    /// decides numbered or bulleted, its `xml:id`. A regenerated body opens its lists with
    /// these rather than bare.
    pub lists: HashMap<BlockId, Vec<(usize, String)>>,
    /// Every picture's outermost `draw:frame`, by what it read as ([`image_key`]): its extent
    /// in [`Source::bytes`]. The model carries a picture's bytes, size and anchor and nothing
    /// else of the frame — its `draw:name`, its `draw:style-name` (wrap, position, border), the
    /// resizing frame and caption LibreOffice nests inside it, a package's `Pictures/` reference.
    /// A regenerated paragraph writes the file's own frame for a picture that is still the one
    /// it read, wherever the edit moved it, so none of that goes.
    pub frames: Vec<(u64, Range<usize>)>,
    /// Every top-level `table:table`, by its name: how the file spelled it, so a regenerated
    /// body writes an untouched table back as its own bytes and an edited one with its own
    /// styles — the table's, its columns', its rows' and its cells' — rather than bare.
    pub tables: HashMap<String, TableSource>,
}

/// One table as the file spelled it ([`Source::tables`]).
///
/// The model's table is its blocks' cell coordinates and nothing else (`crate::model::Cell`);
/// what makes it *look* like itself — column widths, borders, a heading row, a fill — is in
/// style names on four kinds of element. Without these, a body regenerated for an edit
/// anywhere in the document wrote every table bare, and the save's loss check refused it.
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

/// What a picture read as — its type, bytes, size and anchor — as one number, so the writer can
/// tell whether an image run is still a frame the file spelled ([`Source::frames`]). `None` for
/// any other run.
pub fn image_key(run: &crate::model::Run) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let crate::model::Run::Image {
        mime,
        data,
        width,
        height,
        anchor,
    } = run
    else {
        return None;
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (mime, data, width, height, anchor).hash(&mut hasher);
    Some(hasher.finish())
}

/// One element of the body the model does not read: the prelude (`text:sequence-decls`,
/// `text:variable-decls`), a `text:section`, a `text:table-of-content`, a vendor's own. Never
/// interpreted — its bytes are the file's, and a regenerated body carries them through
/// verbatim at the same place in the sequence.
#[derive(Clone, Debug)]
pub struct Sibling {
    /// The block it came after, or `None` for one ahead of every block.
    pub after: Option<BlockId>,
    /// Its extent in [`Source::bytes`].
    pub range: Range<usize>,
}

impl Source {
    /// The name this file already uses for exactly this formatting, if it has one.
    ///
    /// **Parentless only.** The writer emits a character style that inherits from nothing and
    /// puts any named style in a span *around* it, so reusing one that inherits would apply its
    /// parent twice — once as the outer span and once through the inheritance — and the second
    /// read would compose the name into itself. A style with a parent is left alone and the
    /// edit regenerates, which is the safe half of a choice that only costs bytes.
    /// The automatic style a run's formatting was read from, when it says more than the model
    /// reads ([`CharStyle::origin`]) — what a restyle of that run is built from.
    pub fn base_of(&self, props: &CharStyle) -> Option<&TextStyle> {
        let name = props.origin.name()?;
        self.styles.iter().find(|style| style.name == name)
    }

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
    /// Every attribute of the original start tag that the writer does **not** produce itself,
    /// spelled exactly as the file spelled it, ready to drop into a new one.
    ///
    /// The load-bearing detail, and the same one `grind_sheet::odf::source` records. A
    /// paragraph carries `text:class-names`, `text:cond-style-name`, `xml:id` and whatever a
    /// vendor added; re-deriving the start tag from the model would silently drop all of it.
    /// Keeping the *attributes* rather than one of them is what makes that safe.
    pub keep: String,
    /// How many lists it sat in, and which table cell — the context a splice cannot change,
    /// since both are elements *around* the block's own. A block whose context moved
    /// regenerates the body instead.
    pub depth: u32,
    pub cell: Option<crate::model::Cell>,
}

/// The attributes of a block's start tag, minus the ones the writer produces from the model.
///
/// Verbatim, by slicing rather than by re-serialising: `Attrs` resolves prefixes to namespaces,
/// so rebuilding from it would spell a document's own attributes in *our* prefixes and turn a
/// one-element diff back into a whole-file one.
pub fn kept_attributes(start_tag: &[u8]) -> String {
    // What the writer always emits, and what would therefore appear twice.
    attributes(start_tag, &["text:style-name", "text:outline-level"])
}

/// Every attribute of a start tag except those named in `drop`, spelled as the file spelled
/// them — a list's own `text:style-name` and `xml:id`, kept for the `text:list` a regenerated
/// body opens around the same items.
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

/// What has changed since the document was read.
///
/// The spreadsheet's `Edits` in a different shape: it tracks *which cells* were written and
/// whether any edit needed a style the file does not contain. A text document's structure is
/// its block sequence, so what matters here is whether that sequence moved.
#[derive(Clone, Debug, Default)]
pub struct Edits {
    /// Blocks whose content changed but which are still in the document.
    pub blocks: std::collections::BTreeSet<BlockId>,
    /// Whether the block **sequence** changed — an insertion, a deletion, a move.
    ///
    /// Sticky, and it makes splicing impossible for the rest of the session: once the sequence
    /// has moved, the file's structure and the model's no longer correspond, and a patch list
    /// over the original bytes would be describing a document that no longer exists.
    pub structural: bool,
}
