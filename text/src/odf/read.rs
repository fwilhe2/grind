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

use std::collections::HashMap;

use grind_core::odf::context::{Attrs, Context};
use grind_core::odf::names::{Name, Ns};
use grind_core::odf::xml::element_extent;

use crate::model::{Block, BlockId, BlockKind, Document, Run};
use crate::style::{self, CharStyle};

/// How deeply a list may nest before we stop counting.
///
/// §9's rule applied to a different axis: never trust the file's structure to be sane. A
/// crafted document nesting `text:list` ten thousand deep is otherwise a depth counter that
/// overflows a `u32`. The context stack itself is already bounded by `MAX_DEPTH`, so this only
/// has to keep the *number* honest.
const MAX_LIST_DEPTH: u32 = 64;

/// How many `style:style` definitions a document may contribute before we stop collecting.
///
/// The same rule [`MAX_LIST_DEPTH`] follows on a different axis: never trust the file to be
/// sane. A crafted document declaring a million one-attribute styles is otherwise a map that
/// grows without bound while nothing in the body ever refers to any of them. Far above what a
/// real document reaches — a two-hundred-page Writer file lands in the low thousands.
const MAX_STYLES: usize = 100_000;

/// Everything the contexts share: the document under construction, and the style stack.
pub struct Builder {
    pub doc: Document,
    /// The **named** `text:style-name`s currently open, outermost first.
    ///
    /// `text:span` nests and the model is flat, so this is the composition
    /// `doc/text-core.md` describes: a run takes the whole stack, joined, rather than the
    /// innermost name. Lossy for the names, lossless for the rendering.
    ///
    /// A span whose name is an *automatic* style contributes an entry here too, so that the
    /// stack stays parallel with [`Builder::props`] and popping is symmetric — but its entry is
    /// the automatic style's `style:parent-style-name`, or nothing. The generated name itself
    /// never survives, because it means nothing outside the file that generated it
    /// ([`crate::style`]).
    spans: Vec<Option<String>>,
    /// The direct formatting of each open span, in the same order — one entry per entry of
    /// [`Builder::spans`], already composed with everything outside it, so the innermost is
    /// what a run takes.
    props: Vec<CharStyle>,
    /// Every `style:style` of family `text` this document declares, by name.
    ///
    /// Populated before the body, because ODF puts `office:font-face-decls`,
    /// `office:styles` and `office:automatic-styles` ahead of `office:body` in both physical
    /// forms — so a single pass has every definition in hand by the time a span refers to one.
    styles: HashMap<String, TextFamily>,
    /// `office:font-face-decls`: a `style:name` to the family it stands for. The indirection
    /// LibreOffice writes instead of `fo:font-family`, resolved on the way in so that the model
    /// carries the fact rather than the reference.
    fonts: HashMap<String, String>,
    /// The `xlink:href` of the `text:a` currently open, if any. Not a stack: the schema gives
    /// `text:a` `paragraph-content` rather than `paragraph-content-or-hyperlink`
    /// (rng:16453), so a hyperlink cannot nest inside a hyperlink.
    href: Option<String>,
    /// How deep in `text:list` elements we are. 0 outside any list.
    list_depth: u32,
    /// The image being assembled out of the `draw:frame`(s) currently open, if any — see
    /// [`PendingImage`].
    image: Option<PendingImage>,
    /// How many `draw:frame` are open right now. LibreOffice wraps a resizable frame's image
    /// in a second, inner frame (through a `draw:text-box`), so this is what tells the
    /// outermost one's `end` from the inner one's — only the outermost emits a [`Run::Image`].
    frame_depth: u32,
    /// The original bytes, kept only when they are a package — a zip has parts outside
    /// `content.xml` (`Pictures/foo.jpg`) that a `draw:image`'s `xlink:href` may point at, and
    /// resolving one means going back to the archive it came from.
    package: Option<Vec<u8>>,
}

/// One `draw:frame` (rng:5089) being read, gathered from however many of them turn out to be
/// nested around the actual `draw:image` — LibreOffice always wraps one in a second frame for
/// resizing, and this build does not distinguish that from a document that only had one.
#[derive(Default)]
struct PendingImage {
    /// `draw:mime-type` off the `draw:image` itself.
    mime: Option<String>,
    /// The bytes, once `office:binary-data` has been read out and decoded.
    data: Option<Vec<u8>>,
    /// `svg:width` / `svg:height` off whichever frame had them first — outermost preferred,
    /// since that is the size a person actually sees.
    width: Option<String>,
    height: Option<String>,
    /// The plain text of the frame's own caption paragraph (`text:p text:style-name="Figure"`
    /// or whatever a document called it) — everything [`TextBoxSearch`] sees that is not the
    /// nested resizing frame itself. Not a separate run until the outermost frame closes, so
    /// it lands *after* the image rather than before it however the caption's text nodes and
    /// its sequence field arrive.
    caption: String,
}

/// One `style:style` of family `text`, as the document declared it.
struct TextFamily {
    /// Whether it came from `office:automatic-styles`. Automatic means generated, which is why
    /// it may be resolved away and a named one may not — [`crate::style`] makes the argument.
    automatic: bool,
    parent: Option<String>,
    props: CharStyle,
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            doc: Document::new(),
            spans: Vec::new(),
            props: Vec::new(),
            styles: HashMap::new(),
            fonts: HashMap::new(),
            href: None,
            list_depth: 0,
            image: None,
            frame_depth: 0,
            package: None,
        }
    }

    /// Record the package this document is being read from, so a `draw:image`'s `xlink:href`
    /// can later be resolved against it. Called from `odf::read` before parsing starts.
    pub fn set_package(&mut self, bytes: Vec<u8>) {
        self.package = Some(bytes);
    }

    /// A part of the package this document was read from, by path — `None` for the flat form
    /// (nothing to resolve against) and for anything the archive does not actually hold.
    fn resolve_part(&self, path: &str) -> Option<Vec<u8>> {
        super::package::part(self.package.as_deref()?, path)
    }

    /// Open a `text:span`, resolving its style name into a name the model keeps and the
    /// formatting the model applies.
    ///
    /// The one place `doc/text-core.md`'s line between a *generated* style and a *named* one is
    /// drawn. A name nothing declares is kept as a name: an unknown style is a fact about the
    /// document, and inventing formatting for it would be worse than carrying it inert.
    fn open_span(&mut self, name: Option<&str>) {
        let mut props = self.props.last().cloned().unwrap_or_default();
        let kept = match name.and_then(|name| self.styles.get(name).map(|s| (name, s))) {
            Some((_, style)) if style.automatic => {
                props.layer(&style.props);
                style.parent.clone()
            }
            // A declared *named* style: its properties are the document's own vocabulary and
            // stay behind the name. Nothing is layered, which is what keeps `Emphasis` a
            // structural fact rather than an italic that has forgotten why.
            Some((name, _)) => Some(name.to_owned()),
            None => name.map(str::to_owned),
        };
        self.spans.push(kept.filter(|name| !name.is_empty()));
        self.props.push(props);
    }

    fn close_span(&mut self) {
        self.spans.pop();
        self.props.pop();
    }

    /// Record a style definition, if it is one this model has somewhere to put.
    fn declare(&mut self, name: String, style: TextFamily) {
        if self.styles.len() < MAX_STYLES {
            self.styles.insert(name, style);
        }
    }

    /// Hand the automatic character styles to the source, so that a formatting edit can splice
    /// by reusing a name the file already spells (`super::source::TextStyle`).
    ///
    /// Called once, after parsing: a `HashMap` has no order and a splice needs a stable one, so
    /// the list is sorted by name rather than left to iteration order — two saves of the same
    /// document must produce the same bytes.
    pub fn publish_styles(&mut self) {
        let mut styles: Vec<super::source::TextStyle> = self
            .styles
            .iter()
            .filter(|(_, style)| style.automatic && !style.props.is_plain())
            .map(|(name, style)| super::source::TextStyle {
                name: name.clone(),
                parent: style.parent.clone(),
                props: style.props.clone(),
            })
            .collect();
        styles.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(source) = self.doc.source.as_deref_mut() {
            source.styles = styles;
        }
    }

    /// Start a block and make it current, returning its id so the caller can record where it
    /// sat in the file (R6).
    fn open(&mut self, kind: BlockKind, style: Option<String>) -> BlockId {
        let id = self.doc.next_id();
        let mut block = Block::new(id, kind);
        block.style = style;
        self.doc.blocks.push(block);
        id
    }

    /// R6: remember where this block's element is, so a later save can replace it in place.
    ///
    /// Recorded only for the flat form — a package is a zip and has no diff to preserve — and
    /// only when the extent actually resolves. A block with no span simply cannot be spliced,
    /// which the writer treats as "regenerate" rather than as an error.
    fn record(&mut self, id: BlockId, start_tag: std::ops::Range<usize>) {
        let Some(source) = self.doc.source.as_deref() else {
            return;
        };
        let Some(tag) = source.bytes.get(start_tag.clone()) else {
            return;
        };
        let keep = super::source::kept_attributes(tag);
        let Some(range) = element_extent(&source.bytes, start_tag) else {
            return;
        };
        if let Some(source) = self.doc.source.as_deref_mut() {
            source
                .blocks
                .insert(id, super::source::Block { range, keep });
        }
    }

    /// Append text to the block being read, with whatever styling is open.
    ///
    /// Merges into the previous run when the styling matches, so a paragraph split across
    /// several `Event::Text`s by an entity reference is one run rather than three.
    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let names: Vec<&str> = self.spans.iter().flatten().map(String::as_str).collect();
        let style = (!names.is_empty()).then(|| names.join(" "));
        let props = self.props.last().cloned().unwrap_or_default();
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
            props: last_props,
            href: last_href,
        }) = block.runs.last_mut()
            && *last_style == style
            && *last_props == props
            && *last_href == href
        {
            last.push_str(text);
            return;
        }
        block.runs.push(Run::Text {
            text: text.to_owned(),
            style,
            props,
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
            // Both style containers, and the flag is the whole difference between them:
            // automatic styles are generated and may be resolved into direct formatting,
            // named ones are the document's vocabulary and keep their names.
            (Ns::Office, "automatic-styles") => Some(Box::new(Styles { automatic: true })),
            (Ns::Office, "styles") => Some(Box::new(Styles { automatic: false })),
            (Ns::Office, "font-face-decls") => Some(Box::new(FontFaces)),
            _ => None,
        }
    }
}

/// `office:font-face-decls` — `style:font-name` to a real family (§5.2 of the spreadsheet's
/// notes, and the reason `grind_sheet::style::CellStyle` deliberately carries no font at all).
///
/// Resolved here so that the model holds `"Georgia"` rather than `"F1"`, which is what makes a
/// font reachable from a shell without teaching every shell about a second vocabulary.
struct FontFaces;

impl Context<Builder> for FontFaces {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "font-face")
            && let Some(declared) = attrs.get(Ns::Style, "name")
        {
            // `svg:font-family` is where ODF puts it; `fo:font-family` appears in files from
            // producers that treated the two as interchangeable, and reading both costs a line.
            if let Some(family) = attrs
                .get(Ns::Svg, "font-family")
                .or_else(|| attrs.get(Ns::Fo, "font-family"))
                && b.fonts.len() < MAX_STYLES
            {
                b.fonts
                    .insert(declared.to_owned(), style::unquote_family(family));
            }
        }
        None
    }
}

/// `office:automatic-styles` or `office:styles` — the `style:style` declarations.
///
/// Only family `text` is *collected*. A paragraph or table style is a `None` here and therefore
/// an `Ignore` subtree, exactly as any other unmodelled element is: the block's own
/// `text:style-name` is kept verbatim and never resolved (`doc/text-core.md` gates that).
///
/// Its **name** is recorded whatever the family, into [`Document::styles`]. That is a different
/// question from what the style contains — *does this name refer to anything the document
/// declares?* — and a paragraph style is exactly the case that makes it worth asking, since a
/// block's `text:style-name` is one (`doc/dsl.md` §4.3, `undeclared-style`).
struct Styles {
    automatic: bool,
}

impl Context<Builder> for Styles {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Style, "style") {
            return None;
        }
        if let Some(declared) = attrs.get(Ns::Style, "name")
            && b.doc.styles.len() < MAX_STYLES
        {
            b.doc.styles.insert(declared.to_owned());
        }
        if attrs.get(Ns::Style, "family") != Some("text") {
            return None;
        }
        Some(Box::new(TextStyleDef {
            automatic: self.automatic,
            name: attrs.get(Ns::Style, "name").map(str::to_owned),
            parent: attrs.get(Ns::Style, "parent-style-name").map(str::to_owned),
            props: CharStyle::default(),
        }))
    }
}

/// One `style:style style:family="text"`, gathering its `style:text-properties`.
struct TextStyleDef {
    automatic: bool,
    name: Option<String>,
    parent: Option<String>,
    props: CharStyle,
}

impl Context<Builder> for TextStyleDef {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Style, "text-properties") {
            return None;
        }
        // `style:font-name` is LibreOffice's spelling and `fo:font-family` the schema's plain
        // one; a document may use either and this build stores the family in both cases.
        let family = attrs
            .get(Ns::Fo, "font-family")
            .map(style::unquote_family)
            .or_else(|| {
                attrs
                    .get(Ns::Style, "font-name")
                    .and_then(|n| b.fonts.get(n).cloned())
            });
        self.props = CharStyle {
            font_family: family,
            font_size: attrs.get(Ns::Fo, "font-size").map(str::to_owned),
            font_weight: attrs.get(Ns::Fo, "font-weight").map(str::to_owned),
            font_style: attrs.get(Ns::Fo, "font-style").map(str::to_owned),
            underline: attrs
                .get(Ns::Style, "text-underline-style")
                .map(str::to_owned),
            line_through: attrs
                .get(Ns::Style, "text-line-through-style")
                .map(str::to_owned),
            color: attrs.get(Ns::Fo, "color").map(str::to_owned),
            background: attrs.get(Ns::Fo, "background-color").map(str::to_owned),
        };
        None
    }

    fn end(&mut self, b: &mut Builder) {
        let Some(name) = self.name.take() else { return };
        b.declare(
            name,
            TextFamily {
                automatic: self.automatic,
                parent: self.parent.take(),
                props: std::mem::take(&mut self.props),
            },
        );
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
            let id = b.open(kind, style);
            b.record(id, attrs.span());
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
            let id = b.open(BlockKind::Heading { level }, style);
            b.record(id, attrs.span());
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
            b.open_span(attrs.get(Ns::Text, "style-name"));
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
        (Ns::Draw, "frame") => Some(open_frame(attrs, b)),
        _ => None,
    }
}

/// Open a `draw:frame`, whether it is the outermost one or one LibreOffice nested inside it
/// (through a `draw:text-box`) purely for resizing. Only the first frame's size is kept unless
/// it did not say, and only the outermost frame's [`Frame::end`] turns any of this into a run.
fn open_frame(attrs: &Attrs, b: &mut Builder) -> Ctx {
    let pending = b.image.get_or_insert_with(PendingImage::default);
    if pending.width.is_none() {
        pending.width = attrs.get(Ns::Svg, "width").map(str::to_owned);
    }
    if pending.height.is_none() {
        pending.height = attrs.get(Ns::Svg, "height").map(str::to_owned);
    }
    b.frame_depth += 1;
    Box::new(Frame)
}

/// `draw:frame` (rng:5089) — a picture, and the only content of one this build reads.
/// Everything else a frame can hold (`draw:object`, `draw:applet`, a plain shape) is out of
/// scope and inert, exactly like any other unmodelled element.
struct Frame;

impl Context<Builder> for Frame {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Draw, "frame") => Some(open_frame(attrs, b)),
            (Ns::Draw, "image") => {
                let mime = attrs.get(Ns::Draw, "mime-type").map(str::to_owned);
                // `common-draw-data-attlist` (rng:1621) vs. `office-binary-data` (rng:5383):
                // the schema's own choice between a reference and inline bytes. A package
                // form's picture is usually the first — `Pictures/foo.jpg`, resolved against
                // the archive this document was opened from — so it is fetched eagerly, before
                // `b.image` is borrowed, rather than waiting on a child element that never
                // comes.
                let href = attrs.get(Ns::Xlink, "href").map(str::to_owned);
                let resolved = href.as_deref().and_then(|href| b.resolve_part(href));
                if let Some(pending) = &mut b.image {
                    pending.mime = mime;
                    if let Some(data) = resolved {
                        pending.data = Some(data);
                    }
                }
                match href {
                    Some(_) => None,
                    None => Some(Box::new(ImageData)),
                }
            }
            // The frame's own caption text, read for its plain text alone — its own styling
            // and its sequence field's structure are out of scope, exactly like everywhere
            // else a run only keeps text (`doc/text-core.md`).
            (Ns::Draw, "text-box") => Some(Box::new(TextBoxSearch)),
            _ => None,
        }
    }

    fn end(&mut self, b: &mut Builder) {
        b.frame_depth = b.frame_depth.saturating_sub(1);
        // Only the outermost frame's close turns what was gathered into a run — an inner one
        // closing first would otherwise emit a second, empty image from what is left.
        if b.frame_depth == 0
            && let Some(pending) = b.image.take()
            && let (Some(mime), Some(data)) = (pending.mime, pending.data)
        {
            b.push_run(Run::Image {
                mime,
                data,
                width: pending.width,
                height: pending.height,
            });
            // After the image, never before it — whatever order its text nodes and its
            // sequence field arrived in while the text-box was still open.
            if !pending.caption.is_empty() {
                b.push_run(Run::plain(pending.caption));
            }
        }
    }
}

/// `draw:text-box` — a frame's caption. Modelled enough to find an image LibreOffice nests
/// inside one *and* to keep the caption's own plain text (`text:p`/`text:h`'s character data,
/// plus a `text:sequence` field's computed value, rng:8655's `<rng:text/>`) — everything else
/// the caption paragraph could carry (its own styling, a hyperlink) is out of scope the same
/// way it would be anywhere else a run keeps only text.
struct TextBoxSearch;

impl Context<Builder> for TextBoxSearch {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Text, "p" | "h" | "sequence") => Some(Box::new(TextBoxSearch)),
            (Ns::Draw, "frame") => Some(open_frame(attrs, b)),
            _ => None,
        }
    }

    fn text(&mut self, text: &str, b: &mut Builder) {
        if let Some(pending) = &mut b.image {
            pending.caption.push_str(text);
        }
    }
}

/// `draw:image` (rng:5380) — its own attributes are read by [`Frame::start_child`], which also
/// resolves an `xlink:href` reference eagerly. This context is only reached when there was
/// none, so it has to find the `office:binary-data` (rng:7681) inline instead.
struct ImageData;

impl Context<Builder> for ImageData {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        name.is(Ns::Office, "binary-data")
            .then(|| Box::new(BinaryData::default()) as Ctx)
    }
}

/// `office:binary-data` — base64, wrapped across many lines by every writer that produces it,
/// this one included. Whitespace is not part of the alphabet, so it is stripped before
/// decoding rather than tripping the decoder over a document being pretty-printed.
#[derive(Default)]
struct BinaryData {
    base64: String,
}

impl Context<Builder> for BinaryData {
    fn text(&mut self, text: &str, _b: &mut Builder) {
        self.base64.push_str(text);
    }

    fn end(&mut self, b: &mut Builder) {
        use base64::Engine as _;
        let cleaned: String = self.base64.chars().filter(|c| !c.is_whitespace()).collect();
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(cleaned)
            && let Some(pending) = &mut b.image
        {
            pending.data = Some(bytes);
        }
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
        b.close_span();
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
