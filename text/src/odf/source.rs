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
//!   deleting one or moving one changes the **sequence**, and the sequence is what the file's
//!   structure is; splicing that means deciding where the new bytes go and with what
//!   indentation, for a diff that is no longer obviously smaller. The document regenerates,
//!   loudly and by one named rule. `grind_sheet` draws its line in the same place: a cell that
//!   did not exist regenerates too.
//! * **Only the flat form.** A `.odt` is a zip, and a zip has no diff to preserve. What that
//!   costs is measured rather than assumed, in `text/tests/libreoffice.rs`: a real Writer
//!   package comes back as three entries instead of nine, so `styles.xml`, `settings.xml`,
//!   `meta.xml` and the thumbnail are lost on a plain open-and-save. The same document loses
//!   nothing in the flat form, which makes this the *container's* limitation and not the
//!   model's — and the fix is this same trick one level up: keep the archive, replace one
//!   entry in it.
//!
//! Both fall back to regenerating the whole document, which is always correct — it is what the
//! writer did before any of this existed.

use std::collections::HashMap;
use std::ops::Range;

use grind_core::odf::Form;

use crate::model::BlockId;

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
}

/// The attributes of a block's start tag, minus the ones the writer produces from the model.
///
/// Verbatim, by slicing rather than by re-serialising: `Attrs` resolves prefixes to namespaces,
/// so rebuilding from it would spell a document's own attributes in *our* prefixes and turn a
/// one-element diff back into a whole-file one.
pub fn kept_attributes(start_tag: &[u8]) -> String {
    // What the writer always emits, and what would therefore appear twice.
    const DROP: [&str; 2] = ["text:style-name", "text:outline-level"];

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
        if !name.is_empty() && !DROP.contains(&name) {
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
