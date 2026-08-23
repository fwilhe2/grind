// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ODT content model: one context per element we care about. **\[ODT\]**
//!
//! Structure per `doc/odt-format.md` §§2–3. Everything not named here is handled by
//! `grind_core::odf::context::Ignore` without a line of code — which is why the ten of
//! `text-content`'s sixteen alternatives outside the scope line (`doc/text-core.md`) cost
//! nothing at all: not a match arm, not a skip list. That is §8's whole design, arriving
//! unchanged in a second document type, and it is the return on S1.
//!
//! Contexts never talk to each other. A child is told where it lives when it is created, and
//! all mutation flows through the shared [`Builder`] — the same rule `grind_sheet::odf::read`
//! follows, so there is no child-to-parent channel to get wrong and no downcasting.

use grind_core::odf::context::{Attrs, Context};
use grind_core::odf::names::{Name, Ns};

use crate::model::{Block, BlockKind, Document, Run};

/// How deeply a list may nest before we stop counting.
///
/// §9's rule applied to a different axis: never trust the file's structure to be sane. A
/// crafted document nesting `text:list` ten thousand deep is otherwise a depth counter that
/// overflows a `u32`. The context stack itself is already bounded by `MAX_DEPTH`, so this only
/// has to keep the *number* honest.
const MAX_LIST_DEPTH: u32 = 64;

/// Everything the contexts share: the document under construction, and the style stack.
pub struct Builder {
    pub doc: Document,
    /// The `text:style-name`s currently open, outermost first.
    ///
    /// `text:span` nests and the model is flat, so this is the composition
    /// `doc/text-core.md` describes: a run takes the whole stack, joined, rather than the
    /// innermost name. Lossy for the names, lossless for the rendering.
    spans: Vec<String>,
    /// The `xlink:href` of the `text:a` currently open, if any. Not a stack: the schema gives
    /// `text:a` `paragraph-content` rather than `paragraph-content-or-hyperlink`
    /// (rng:16453), so a hyperlink cannot nest inside a hyperlink.
    href: Option<String>,
    /// How deep in `text:list` elements we are. 0 outside any list.
    list_depth: u32,
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            doc: Document::new(),
            spans: Vec::new(),
            href: None,
            list_depth: 0,
        }
    }

    /// Start a block and make it current.
    fn open(&mut self, kind: BlockKind, style: Option<String>) {
        let id = self.doc.next_id();
        let mut block = Block::new(id, kind);
        block.style = style;
        self.doc.blocks.push(block);
    }

    /// Append text to the block being read, with whatever styling is open.
    ///
    /// Merges into the previous run when the styling matches, so a paragraph split across
    /// several `Event::Text`s by an entity reference is one run rather than three.
    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let style = (!self.spans.is_empty()).then(|| self.spans.join(" "));
        let href = self.href.clone();
        let Some(block) = self.doc.blocks.last_mut() else {
            // Character data outside any block. Real files have it — it is the indentation
            // between elements — and `Context::text` already defaults to a no-op, so this only
            // fires for a document whose structure we did not follow. Dropping it is §9.
            return;
        };
        if let Some(Run::Text {
            text: last,
            style: last_style,
            href: last_href,
        }) = block.runs.last_mut()
            && *last_style == style
            && *last_href == href
        {
            last.push_str(text);
            return;
        }
        block.runs.push(Run::Text {
            text: text.to_owned(),
            style,
            href,
        });
    }

    fn push_run(&mut self, run: Run) {
        if let Some(block) = self.doc.blocks.last_mut() {
            block.runs.push(run);
        }
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

type Ctx = Box<dyn Context<Builder>>;

pub struct Root;

impl Context<Builder> for Root {
    fn start_child(&mut self, name: &Name, _a: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Office, "document" | "document-content" | "document-styles") => {
                Some(Box::new(Root))
            }
            (Ns::Office, "body") => Some(Box::new(Body)),
            _ => None,
        }
    }
}

struct Body;

impl Context<Builder> for Body {
    fn start_child(&mut self, name: &Name, _a: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        // `office:text`, not `text:text` — the body element is in the *office* namespace
        // (rng:7693), and it is the one place in this reader where the prefix a person reads
        // and the namespace the element is in do not spell the same word.
        name.is(Ns::Office, "text").then(|| Box::new(Text) as Ctx)
    }
}

/// `office:text` — the body. Its content is a **flat** sequence (`doc/odt-format.md` §2), so
/// this context and [`ListItem`] both dispatch the same block set.
struct Text;

impl Context<Builder> for Text {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        block_child(name, attrs, b)
    }
}

/// The blocks that may appear in a body or inside a list item.
///
/// One function rather than two identical match arms, because `text-list-item-content` admits
/// the same things `office-text-content-main` does. Everything outside `doc/text-core.md`'s
/// scope line simply returns `None` and is swallowed whole.
fn block_child(name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
    let style = attrs.get(Ns::Text, "style-name").map(str::to_owned);
    match (name.ns, name.local.as_str()) {
        (Ns::Text, "p") => {
            let kind = match b.list_depth {
                0 => BlockKind::Paragraph,
                depth => BlockKind::ListItem { depth },
            };
            b.open(kind, style);
            Some(Box::new(Paragraph))
        }
        (Ns::Text, "h") => {
            // `text:outline-level` is required and unbounded (rng:6867), so a level of 9 is a
            // document to load rather than an error. Missing or unparseable means 1: a heading
            // that says it is a heading is one, whatever else it fails to say.
            let level = attrs
                .get(Ns::Text, "outline-level")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .filter(|n| *n > 0)
                .unwrap_or(1);
            b.open(BlockKind::Heading { level }, style);
            Some(Box::new(Paragraph))
        }
        (Ns::Text, "list") => Some(Box::new(List::new())),
        _ => None,
    }
}

/// `text:list` — nests through its items, never through itself (rng:17494).
struct List {
    /// Whether this element actually incremented the depth. A list past [`MAX_LIST_DEPTH`]
    /// still reads its items; it just stops making the number bigger, and has to remember not
    /// to decrement on the way out.
    counted: bool,
}

impl List {
    fn new() -> Self {
        List { counted: false }
    }
}

impl Context<Builder> for List {
    fn start_child(&mut self, name: &Name, _a: &Attrs, b: &mut Builder) -> Option<Ctx> {
        // Depth is incremented when the first item opens rather than when the list does, so
        // that an empty `text:list` costs nothing.
        match (name.ns, name.local.as_str()) {
            // `text:list-header` holds the same content as an item and is not a numbered one;
            // read as an item, because the distinction is numbering, which is out of scope.
            (Ns::Text, "list-item" | "list-header") => {
                if !self.counted && b.list_depth < MAX_LIST_DEPTH {
                    b.list_depth += 1;
                    self.counted = true;
                }
                Some(Box::new(ListItem))
            }
            _ => None,
        }
    }

    fn end(&mut self, b: &mut Builder) {
        if self.counted {
            b.list_depth = b.list_depth.saturating_sub(1);
        }
    }
}

struct ListItem;

impl Context<Builder> for ListItem {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        block_child(name, attrs, b)
    }
}

/// `text:p` and `text:h` — the same content model (rng:17950, rng:17095), so one context.
struct Paragraph;

impl Context<Builder> for Paragraph {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        inline_child(name, attrs, b)
    }

    fn text(&mut self, text: &str, b: &mut Builder) {
        b.push_text(text);
    }
}

/// `paragraph-content` (rng:8405) — the inline model, shared by paragraphs, spans and
/// hyperlinks.
fn inline_child(name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
    match (name.ns, name.local.as_str()) {
        (Ns::Text, "span") => {
            // The style stack, not the innermost name: spans nest and the model is flat.
            b.spans
                .push(attrs.get(Ns::Text, "style-name").unwrap_or("").to_owned());
            Some(Box::new(Span))
        }
        (Ns::Text, "a") => {
            b.href = attrs.get(Ns::Xlink, "href").map(str::to_owned);
            Some(Box::new(Hyperlink))
        }
        (Ns::Text, "s") => {
            // ODF's run-length encoding of spaces (rng:8408). Expanded here and re-encoded on
            // write, exactly as `table:number-columns-repeated` is — a correctness trap rather
            // than an optimisation, and `Attrs::count` already has this shape. Clamped for the
            // same reason it is there: a document claiming four billion spaces is a
            // memory-exhaustion vector, not an intent.
            let count = attrs.count(Ns::Text, "c", 4096);
            b.push_text(&" ".repeat(count as usize));
            None
        }
        (Ns::Text, "tab") => {
            b.push_run(Run::Tab);
            None
        }
        (Ns::Text, "line-break") => {
            b.push_run(Run::Break);
            None
        }
        (Ns::Text, "bookmark" | "bookmark-start") => {
            if let Some(name) = attrs.get(Ns::Text, "name") {
                b.push_run(Run::Bookmark {
                    name: name.to_owned(),
                });
            }
            None
        }
        _ => None,
    }
}

/// `text:span` — nests, so its children are the same inline set.
struct Span;

impl Context<Builder> for Span {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        inline_child(name, attrs, b)
    }

    fn text(&mut self, text: &str, b: &mut Builder) {
        b.push_text(text);
    }

    fn end(&mut self, b: &mut Builder) {
        b.spans.pop();
    }
}

/// `text:a` — a hyperlink. Cannot nest inside itself per the schema, so the href is one slot
/// rather than a stack.
struct Hyperlink;

impl Context<Builder> for Hyperlink {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        inline_child(name, attrs, b)
    }

    fn text(&mut self, text: &str, b: &mut Builder) {
        b.push_text(text);
    }

    fn end(&mut self, b: &mut Builder) {
        b.href = None;
    }
}
