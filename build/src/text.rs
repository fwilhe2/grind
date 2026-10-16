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
pub struct Doc {
    blocks: Rc<RefCell<Vec<Item>>>,
    /// Whether `page_break()` was said and the block it is for has not come yet.
    pending_break: Rc<std::cell::Cell<bool>>,
    /// The flat ODF text document this one starts from — its page, its master pages with their
    /// headers and footers, and the paragraph styles blocks may name — if `text_from` said one.
    template: Option<Rc<String>>,
}

/// One thing said into the document: a block, or a table (which is `rows × columns` blocks).
enum Item {
    Block(Block),
    Table {
        rows: Vec<Vec<String>>,
        /// A paragraph style per column, `""` for none.
        styles: Vec<String>,
    },
}

impl Item {
    /// How many blocks it makes.
    fn blocks(&self) -> usize {
        match self {
            Item::Block(_) => 1,
            Item::Table { rows, .. } => rows.iter().map(Vec::len).sum(),
        }
    }
}

struct Block {
    /// Whether a page break was said before it: it starts a page.
    page_break: bool,
    kind: BlockKind,
    /// The block's text in the markdown-shaped notation, read on the way in.
    markdown: String,
    /// A bookmark anchored at the start of this block, if a script asked for one.
    bookmark: Option<String>,
    /// The named paragraph style of this block, which the template declares.
    style: Option<String>,
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
        "para",
        ["document: Text", "style: string", "text: string", "Text"],
        [
            "/// A paragraph in a named paragraph style the template declares.",
            "///",
            "/// Without a template (`text_from`) no style is declared, and `grind lint` reports",
            "/// the name as undeclared.",
        ],
        |doc: &mut Doc, style: &str, text: &str| {
            doc.push_styled(BlockKind::Paragraph, text, Some(style.to_owned()))
        },
    );
    // Like `bookmark`, on the block last said — which is what lets a list item, a heading or a
    // paragraph take a template's style without a second overload of each.
    hint(
        engine,
        "style",
        ["document: Text", "style: string", "Text"],
        [
            "/// Set the named paragraph style of the block last said, which the template",
            "/// declares. `para(style, text)` is the same thing for a paragraph, in one call.",
        ],
        |doc: &mut Doc, style: &str| -> Res<Doc> {
            doc.with_last_block("style()", |last| last.style = Some(style.to_owned()))?;
            Ok(doc.clone())
        },
    );
    // A table is a group of blocks, so it is not a `para` with a flag: it carries its own
    // rows, and `bookmark()` and `style()` — which are about one block — refuse it by name.
    hint(
        engine,
        "table",
        ["document: Text", "rows: array", "Text"],
        [
            "/// A table: an array of rows, each an array of cell texts. Every row needs the",
            "/// same number of cells. A cell's text is read in the suite's notation, so",
            "/// `**bold**` makes a heading row.",
        ],
        |doc: &mut Doc, rows: rhai::Array| doc.table(rows, rhai::Array::new()),
    );
    hint(
        engine,
        "table",
        ["document: Text", "rows: array", "styles: array", "Text"],
        [
            "/// A table whose columns are set in the paragraph styles the template declares —",
            "/// `[\"Name\", \"Amount\"]`, with `\"\"` for a column with none. An amount's column is",
            "/// where a right-aligned style goes.",
        ],
        |doc: &mut Doc, rows: rhai::Array, styles: rhai::Array| doc.table(rows, styles),
    );
    hint(
        engine,
        "page_break",
        ["document: Text", "Text"],
        [
            "/// The next paragraph, heading or item starts a new page.",
            "///",
            "/// Said as a paragraph style the document declares for it — `fo:break-before` —",
            "/// so it works whatever template the document starts from, and none.",
        ],
        |doc: &mut Doc| {
            doc.pending_break.set(true);
            doc.clone()
        },
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
            doc.with_last_block("bookmark()", |last| last.bookmark = Some(name.to_owned()))?;
            Ok(doc.clone())
        },
    );
    hint(
        engine,
        "blocks",
        ["document: Text", "int"],
        ["/// How many blocks have been said."],
        |doc: &mut Doc| doc.count() as i64,
    );
    hint_get(
        engine,
        "blocks",
        ["/// How many blocks have been said."],
        |doc: &mut Doc| doc.count() as i64,
    );
}

fn bad(message: impl Into<String>) -> Box<EvalAltResult> {
    message.into().into()
}

impl Doc {
    fn push(&mut self, kind: BlockKind, markdown: &str) -> Doc {
        self.push_styled(kind, markdown, None)
    }

    fn push_styled(&mut self, kind: BlockKind, markdown: &str, style: Option<String>) -> Doc {
        self.blocks.borrow_mut().push(Item::Block(Block {
            page_break: self.pending_break.replace(false),
            kind,
            markdown: markdown.to_owned(),
            bookmark: None,
            style,
        }));
        self.clone()
    }

    /// How many blocks have been said, a table's cells each counting as one.
    fn count(&self) -> usize {
        self.blocks.borrow().iter().map(Item::blocks).sum()
    }

    /// Change the block last said, or say why a table or an empty document cannot be.
    fn with_last_block(&self, what: &str, change: impl FnOnce(&mut Block)) -> Res<()> {
        match self.blocks.borrow_mut().last_mut() {
            Some(Item::Block(block)) => {
                change(block);
                Ok(())
            }
            Some(Item::Table { .. }) => Err(bad(format!(
                "{what} is about one block, and the last thing said is a table"
            ))),
            None => Err(bad(format!("{what} needs a block to work on"))),
        }
    }

    fn table(&self, rows: rhai::Array, styles: rhai::Array) -> Res<Doc> {
        let text = |value: &rhai::Dynamic| -> Res<String> {
            value
                .clone()
                .into_string()
                .map_err(|got| bad(format!("a table cell is text, not {got}")))
        };
        let rows: Vec<Vec<String>> = rows
            .iter()
            .map(|row| {
                row.clone()
                    .into_array()
                    .map_err(|got| bad(format!("a table row is an array of texts, not {got}")))?
                    .iter()
                    .map(text)
                    .collect()
            })
            .collect::<Res<_>>()?;
        let columns = rows.first().map_or(0, Vec::len);
        if rows.is_empty() || columns == 0 {
            return Err(bad("a table needs at least one row and one cell"));
        }
        if let Some(ragged) = rows.iter().position(|row| row.len() != columns) {
            return Err(bad(format!(
                "row {} has {} cells where the first has {columns}: a table is as wide in every row",
                ragged + 1,
                rows[ragged].len()
            )));
        }
        let styles: Vec<String> = styles.iter().map(text).collect::<Res<_>>()?;
        if styles.len() > columns {
            return Err(bad(format!(
                "{} column styles for a table {columns} columns wide",
                styles.len()
            )));
        }
        if self.pending_break.get() {
            return Err(bad(
                "page_break() must be followed by a paragraph, heading or item, not a table",
            ));
        }
        self.blocks.borrow_mut().push(Item::Table { rows, styles });
        Ok(self.clone())
    }

    /// A document that starts from `template`, flat ODF text, read by the data wall's rules.
    pub(crate) fn from_template(template: String) -> Doc {
        Doc {
            blocks: Rc::default(),
            pending_break: Rc::default(),
            template: Some(Rc::new(template)),
        }
    }
}

/// The derived paragraph style that starts a page: `parent`'s, plus `fo:break-before`.
fn break_style(parent: Option<&str>) -> String {
    format!("grind-break-{}", parent.unwrap_or("none"))
}

/// `template` — or a bare document when there is none — with the paragraph styles that carry a
/// page break declared in it, so a script's `page_break()` is a *named style the document has*
/// like any other, whatever it started from.
fn with_break_styles(template: Option<&str>, parents: &[Option<String>]) -> String {
    const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
    let declared: String = parents
        .iter()
        .map(|parent| {
            let parent_attr = parent.as_deref().map_or(String::new(), |p| {
                format!(r#" style:parent-style-name="{}""#, xml_attr(p))
            });
            format!(
                r#"<style:style style:name="{}" style:family="paragraph"{parent_attr}><style:paragraph-properties xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" fo:break-before="page"/></style:style>"#,
                xml_attr(&break_style(parent.as_deref())),
            )
        })
        .collect();
    let bare = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document xmlns:office="{OFFICE}" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text"><office:body><office:text/></office:body></office:document>"#
    );
    let template = template.unwrap_or(&bare);
    if let Some(at) = template.find("</office:styles>") {
        return format!("{}{declared}{}", &template[..at], &template[at..]);
    }
    if let Some(at) = template.find("<office:styles/>") {
        return format!(
            "{}<office:styles>{declared}</office:styles>{}",
            &template[..at],
            &template[at + "<office:styles/>".len()..]
        );
    }
    for anchor in [
        "<office:automatic-styles",
        "<office:master-styles",
        "<office:body",
    ] {
        if let Some(at) = template.find(anchor) {
            return format!(
                "{}<office:styles>{declared}</office:styles>{}",
                &template[..at],
                &template[at..]
            );
        }
    }
    template.to_owned()
}

fn xml_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

/// Build the document a script described.
pub fn materialise(doc: &Doc) -> Result<App, String> {
    let app = App::new();
    let items = doc.blocks.borrow();
    if items.is_empty() {
        return Err("a text document needs at least one block".to_owned());
    }
    // Every style a page break needs, declared up front — in the template, or in a bare
    // document when there is none.
    let mut parents: Vec<Option<String>> = Vec::new();
    for item in items.iter() {
        if let Item::Block(block) = item
            && block.page_break
            && !parents.contains(&block.style)
        {
            parents.push(block.style.clone());
        }
    }
    let source = match (&doc.template, parents.is_empty()) {
        (Some(template), true) => Some(template.as_str().to_owned()),
        (template, false) => Some(with_break_styles(
            template.as_deref().map(String::as_str),
            &parents,
        )),
        (None, true) => None,
    };
    if let Some(source) = &source {
        // The template's body is read as the empty document it should be; what survives is the
        // part a save carries through untouched — page, master pages, styles.
        app.open_bytes("template.fodt", source.as_bytes())
            .map_err(say)?;
        let kept = app.block_count();
        if kept > 0 {
            app.delete(0..kept).map_err(say)?;
        }
    }
    // A new document already has one empty paragraph in it, and a template's body is dropped
    // but may leave one. Filling the first block into that one keeps a generated document from
    // starting with a blank line — and one that opens with a table has no block to fill, so the
    // placeholder goes.
    let mut spare = app.block_count();
    if matches!(items.first(), Some(Item::Table { .. })) && spare > 0 {
        app.delete(0..spare).map_err(say)?;
        spare = 0;
    }
    let mut spare = spare.min(1);
    let mut leftover = app.block_count();
    for item in items.iter() {
        match item {
            Item::Block(block) => {
                let index = if spare > 0 {
                    spare = 0;
                    leftover = 0;
                    0
                } else {
                    app.insert(app.block_count(), block.kind.clone(), "")
                        .map_err(say)?;
                    app.block_count() - 1
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
                let style = match block.page_break {
                    true => Some(break_style(block.style.as_deref())),
                    false => block.style.clone(),
                };
                if let Some(style) = style {
                    app.set_style(index..index + 1, Some(style)).map_err(say)?;
                }
            }
            Item::Table { rows, styles } => {
                let start = app.block_count();
                let columns = rows[0].len();
                app.insert_table(start, rows.len() as u32, columns as u32, None)
                    .map_err(say)?;
                for (r, row) in rows.iter().enumerate() {
                    for (c, text) in row.iter().enumerate() {
                        let index = start + r * columns + c;
                        if !text.is_empty() {
                            let at = Caret {
                                block: index,
                                offset: 0,
                            };
                            app.type_markdown(at, text, None).map_err(say)?;
                        }
                        if let Some(style) = styles.get(c).filter(|style| !style.is_empty()) {
                            app.set_style(index..index + 1, Some(style.clone()))
                                .map_err(say)?;
                        }
                    }
                }
            }
        }
    }
    // The placeholder a table-first document could not delete is still in front of it.
    if leftover > 0 && matches!(items.first(), Some(Item::Table { .. })) {
        app.delete(0..leftover).map_err(say)?;
    }
    Ok(app)
}

fn say(error: grind_text::Error) -> String {
    error.to_string()
}
