// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor's half of the host API.
//!
//! Smaller than the spreadsheet's by the same margin the models differ: a text document is a
//! **flat sequence of blocks** (`text/src/model.rs`), so a script that can say *heading*,
//! *paragraph* and *list item* can say the whole shape of one.
//!
//! ```rhai
//! let d = text();
//! d.heading(1, "Quarterly report");
//! d.bookmark("intro");
//! d.para("Revenue rose **12%** on the year.");
//! d.item(1, "North led on volume");
//! d
//! ```
//!
//! **The inline notation is `grind_text::markdown`'s, not a fourth reading of `**`.** A
//! paragraph's text goes in through `App::type_markdown`, which is the same function the TUI's
//! formatting toolbar uses and the same one `doc/dsl.md` §3.6 built the projection's inline
//! notation beside. A generator that had its own emphasis rule would be the `**` problem one
//! layer up, and `CLAUDE.md` names that as the mistake the core exists to prevent.
//!
//! What a script cannot say here, each because the model reaches it through a caret rather
//! than through a block: character formatting that does not come from the notation (a font, a
//! size, a colour), images (`doc/dsl.md` §3.8's open question, and it has no projection
//! either), and a named paragraph style — a generator cannot *declare* one, and `grind lint`'s
//! `undeclared-style` would rightly complain about every block that used it.

use std::cell::RefCell;
use std::rc::Rc;

use grind_text::{App, BlockKind, Caret};
use rhai::{Engine, EvalAltResult};

use crate::hint::{hint, hint_get};

type Res<T> = Result<T, Box<EvalAltResult>>;

/// A text document, as a script builds it: blocks in the order they were said.
///
/// Shared like [`crate::sheet::Sheet`] and for the same reason — a document handed to a
/// function has to be the same document when it comes back.
#[derive(Clone, Default)]
pub struct Doc(Rc<RefCell<Vec<Block>>>);

struct Block {
    kind: BlockKind,
    /// The block's text in the markdown-shaped notation, read on the way in.
    markdown: String,
    /// A bookmark anchored at the start of this block, if a script asked for one.
    bookmark: Option<String>,
}

/// Every method hands the document back, so a script may chain — and because it is a shared
/// handle, the one that comes back is the one that went in.
///
/// Documented at the registration for the reason `crate::hint` gives: what an editor shows
/// while somebody types is written here or nowhere.
pub fn register(engine: &mut Engine) {
    engine.register_type_with_name::<Doc>("Text");
    hint(
        engine,
        "text",
        ["Text"],
        [
            "/// An empty text document. Say headings, paragraphs and list items into it, and",
            "/// return it at the end of the script.",
        ],
        Doc::default,
    );
    hint(
        engine,
        "para",
        ["document: Text", "text: string", "Text"],
        [
            "/// A paragraph.",
            "///",
            "/// The text is read in the notation the whole suite types in: `**bold**`,",
            "/// `_italic_`, `` `code` ``.",
        ],
        |doc: &mut Doc, text: &str| doc.push(BlockKind::Paragraph, text),
    );
    hint(
        engine,
        "heading",
        ["document: Text", "level: int", "text: string", "Text"],
        [
            "/// A heading, level 1 to 6.",
            "///",
            "/// The outline is implied by the levels and by nothing else — a heading does not",
            "/// contain what follows it.",
        ],
        |doc: &mut Doc, level: i64, text: &str| -> Res<Doc> {
            let level = u32::try_from(level)
                .ok()
                .filter(|level| (1..=6).contains(level))
                .ok_or_else(|| bad(format!("{level}: a heading is level 1 to 6")))?;
            Ok(doc.push(BlockKind::Heading { level }, text))
        },
    );
    hint(
        engine,
        "item",
        ["document: Text", "depth: int", "text: string", "Text"],
        ["/// A list item, nested 1 to 9 deep."],
        |doc: &mut Doc, depth: i64, text: &str| -> Res<Doc> {
            let depth = u32::try_from(depth)
                .ok()
                .filter(|depth| (1..=9).contains(depth))
                .ok_or_else(|| bad(format!("{depth}: a list item nests 1 to 9 deep")))?;
            Ok(doc.push(BlockKind::ListItem { depth }, text))
        },
    );
    hint(
        engine,
        "item",
        ["document: Text", "text: string", "Text"],
        ["/// A list item at depth 1."],
        |doc: &mut Doc, text: &str| doc.push(BlockKind::ListItem { depth: 1 }, text),
    );
    // An anchor on the block last said, which is what `#intro` addresses and what makes a
    // generated document navigable — `loc.rs`'s point that a named target survives edits
    // elsewhere and `p12` does not.
    hint(
        engine,
        "bookmark",
        ["document: Text", "name: string", "Text"],
        [
            "/// Anchor a bookmark at the start of the block last said.",
            "///",
            "/// `#name` then addresses it — an address that survives editing above it, which",
            "/// `p12` does not.",
        ],
        |doc: &mut Doc, name: &str| -> Res<Doc> {
            let mut blocks = doc.0.borrow_mut();
            let last = blocks
                .last_mut()
                .ok_or_else(|| bad("bookmark() needs a block to anchor to"))?;
            last.bookmark = Some(name.to_owned());
            drop(blocks);
            Ok(doc.clone())
        },
    );
    hint(
        engine,
        "blocks",
        ["document: Text", "int"],
        ["/// How many blocks have been said."],
        |doc: &mut Doc| doc.0.borrow().len() as i64,
    );
    hint_get(
        engine,
        "blocks",
        ["/// How many blocks have been said."],
        |doc: &mut Doc| doc.0.borrow().len() as i64,
    );
}

fn bad(message: impl Into<String>) -> Box<EvalAltResult> {
    message.into().into()
}

impl Doc {
    fn push(&mut self, kind: BlockKind, markdown: &str) -> Doc {
        self.0.borrow_mut().push(Block {
            kind,
            markdown: markdown.to_owned(),
            bookmark: None,
        });
        self.clone()
    }
}

/// Build the document a script described.
pub fn materialise(doc: &Doc) -> Result<App, String> {
    let app = App::new();
    let blocks = doc.0.borrow();
    if blocks.is_empty() {
        return Err("a text document needs at least one block".to_owned());
    }
    // A new document already has one empty paragraph in it. Filling that one first and
    // appending the rest keeps a generated document from starting with a blank line, and
    // means `d.para("…")` alone produces a one-paragraph document rather than a two.
    let existing = app.block_count();
    for (nth, block) in blocks.iter().enumerate() {
        let index = match nth < existing {
            true => nth,
            false => {
                app.insert(app.block_count(), block.kind.clone(), "")
                    .map_err(say)?;
                app.block_count() - 1
            }
        };
        app.set_kind(index, block.kind.clone()).map_err(say)?;
        if !block.markdown.is_empty() {
            let at = Caret {
                block: index,
                offset: 0,
            };
            app.type_markdown(at, &block.markdown, None).map_err(say)?;
        }
        if let Some(name) = &block.bookmark {
            app.set_bookmark(name, Some(index)).map_err(say)?;
        }
    }
    // A document with fewer blocks than the empty one started with cannot happen — there is
    // exactly one — but the day `App::new` changes, this is the line that keeps the promise
    // above rather than the comment.
    if blocks.len() < existing {
        app.delete(blocks.len()..existing).map_err(say)?;
    }
    Ok(app)
}

fn say(error: grind_text::Error) -> String {
    error.to_string()
}
