// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The file a document came from, kept so that saving can edit it instead of replacing it.
//!
//! This is R6 (doc/plan.md): *writing must change as little of the XML as possible*.
//! Regenerating from the model is right for a document this program authored and wrong for
//! one it opened — everything the model does not carry (`office:meta`, `office:settings`,
//! styles nothing references, a chart, another vendor's namespace) goes, and the diff is the
//! whole file. Editing one number should be one line of `git diff`, the way it is in the
//! `.fods` repositories this format is good for.
//!
//! **Retain and splice, not a fuller model.** Carrying every unknown element as a shadow
//! tree would grow the model to the size of ODF, which is the trade this project exists not
//! to make. Instead the reader keeps the bytes it read and remembers where each cell's
//! element sat in them; the writer serialises the cells that changed and drops them into
//! those exact ranges. Every other byte — indentation included — is the file that came in.
//!
//! **A repeated cell is split rather than skipped.** `table:number-columns-repeated` is not
//! an edge case in files LibreOffice writes — a row of five empty cells is one element, and
//! so is the trailing run of sixteen thousand — so treating one as unspliceable would leave
//! R6 true only for cells that already had a value. Writing into one re-emits *that element*
//! as the run before, the changed cell, and the run after, which is still a one-element diff.
//! [`Cell::cols`] is what makes that possible: the element knows which addresses it stands
//! for.
//!
//! Three things are deliberately *not* here, each an honest boundary of the trick rather
//! than a corner cut:
//!
//! * **A repeated *row* is not split.** One element there stands for many rows rather than
//!   many columns, so splitting means emitting whole rows and the diff stops being small —
//!   which is the entire point. Rows are recorded only where `table:number-rows-repeated` is
//!   absent or 1.
//! * **Only values and formulas splice.** A changed number format or cell style needs a new
//!   `style:style` in `office:automatic-styles`, which is a second splice site and a pool to
//!   merge with the document's own; [`crate::model::Edits::only_values`] goes false and the
//!   writer regenerates.
//! * **Only the flat form.** A `.ods` is a zip, and a zip has no diff to preserve.
//!
//! Every one of those falls back to regenerating the whole document, which is always
//! correct — it is what the writer did before any of this existed.

use std::collections::HashMap;
use std::ops::Range;

use super::write::Form;

/// The bytes a document was read from, plus where its cells are in them.
#[derive(Clone, Debug)]
pub struct Source {
    /// Which physical form those bytes are. Splicing is refused for any other, so a
    /// `.fods` opened and saved as `.ods` regenerates rather than producing a zip full of
    /// flat XML.
    pub form: Form,
    /// The file exactly as it was read. For the flat form this is also `content.xml`, which
    /// is why the ranges below index straight into it.
    pub bytes: Vec<u8>,
    /// The cell elements of each `(sheet, row)`, in column order.
    ///
    /// Per row rather than per cell because of the repeated element: keying by address would
    /// mean sixteen thousand identical entries for one trailing `<table:table-cell
    /// table:number-columns-repeated="16384"/>`, which is in most real files and in every
    /// row of some. A row's elements are few, so finding the one covering a column is a scan
    /// over a short list.
    pub rows: HashMap<(usize, u32), Vec<Cell>>,
}

/// One cell element of the source file.
#[derive(Clone, Debug)]
pub struct Cell {
    /// The element's extent in [`Source::bytes`], start tag through end tag.
    pub range: Range<usize>,
    /// The columns it stands for — one column, or the run a
    /// `table:number-columns-repeated` covers.
    pub cols: Range<u32>,
    /// Every attribute of the original element that the writer does **not** produce itself,
    /// spelled exactly as the file spelled it, ready to drop into a start tag.
    ///
    /// The load-bearing detail of the whole splice, and the reason it is the *attributes*
    /// rather than just the style name. `table:style-name="ce7"` has to survive because that
    /// style lives in a part of the document nothing here models — our pool would emit `ce0`
    /// and point the cell at a style the file does not contain. But so does
    /// `table:number-columns-spanned`: writing a number into a merged cell and silently
    /// un-merging it is a worse bug than a large diff, and it is what re-deriving the whole
    /// start tag does. See [`kept_attributes`] for what is dropped and why.
    pub keep: String,
}

/// The attributes of a cell's start tag, minus the ones the writer produces from the model.
///
/// Verbatim, by slicing rather than by re-serialising: `Attrs` resolves prefixes to
/// namespaces, so rebuilding from it would spell a document's own attributes in *our*
/// prefixes and turn a one-element diff back into a whole-file one.
///
/// Two groups are dropped. The first is what the writer always emits — the value, its type,
/// the formula, the repeat count — which would otherwise appear twice and make the element
/// ill-formed. The second is what *describes* that value and is no longer true once it
/// changes: `office:currency`, and the `calcext:` mirror of the value type that LibreOffice
/// writes (R4 — allowed, but not something to carry forward onto a value it no longer
/// matches).
pub fn kept_attributes(start_tag: &[u8]) -> String {
    const DROP: [&str; 10] = [
        "office:value-type",
        "office:value",
        "office:date-value",
        "office:time-value",
        "office:boolean-value",
        "office:string-value",
        "office:currency",
        "table:formula",
        "table:number-columns-repeated",
        "calcext:value-type",
    ];

    let Ok(tag) = std::str::from_utf8(start_tag) else {
        return String::new();
    };
    // Past `<table:table-cell`, and stopping before the `/>` or `>` that closes it.
    let Some(body) = tag.find(char::is_whitespace).map(|i| &tag[i..]) else {
        return String::new();
    };
    let body = body.trim_end_matches('>').trim_end_matches('/');

    let mut out = String::new();
    let mut rest = body;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim();
        let after = &rest[eq + 1..];
        // An attribute value is quoted, and cannot contain its own quote character — so the
        // next one of the same kind ends it, `>` and `<` inside notwithstanding.
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
            rows: HashMap::new(),
        }
    }

    /// The element covering `col` of this row, if the file spelled one.
    pub fn covering(&self, sheet: usize, row: u32, col: u32) -> Option<&Cell> {
        self.rows
            .get(&(sheet, row))?
            .iter()
            .find(|c| c.cols.contains(&col))
    }
}
