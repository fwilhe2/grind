// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! CommonMark in and out — the word processor's `csv.rs`.
//!
//! **Not [`crate::markdown`].** That module is a *typing* notation (`**bold**` while you write,
//! with `__` meaning underline); this one is a *file format*, and it follows CommonMark rather
//! than that notation, so `__x__` is bold here and underline has no spelling at all.
//!
//! **Reading** is `pulldown-cmark` (MIT), so nothing here parses markdown: this file maps its
//! events onto the block model. **Writing** is ours, since the model is the source and a
//! serializer is a fold over it.
//!
//! Like CSV it is one way each, with a named loss in each direction rather than an
//! approximation:
//!
//! * **In:** a block quote is its paragraphs, unquoted; an ordered list is a bulleted one (the
//!   model has no numbering); a rule and raw HTML are dropped; a soft line break is a space. A
//!   fenced block is a run of paragraphs in the [`PREFORMATTED`] style, one per line — the shape
//!   `` ``` `` already makes when typed. A picture comes in when its bytes can be had (a
//!   `data:` URI, or whatever the caller's [`Resolve`] finds) and is its alt text otherwise.
//! * **Out:** underline, colour, highlight, size, a bookmark, a picture's size and a table's
//!   spans have no CommonMark spelling and are dropped; a list item is always `- `, and a
//!   table's first row is its header, since a pipe table has to have one.
//!
//! **Pictures are the one thing markdown cannot hold**, so an export is a text *and* some files
//! ([`Exported`]): each picture beside the markdown, named for its bytes, or — [`Pictures::Inline`]
//! — a `data:` URI inside it. Files are the default because the commonest renderer of all,
//! GitHub's, does not show a `data:` URI, and base64 makes a diff unreadable. This is not the
//! projection's sidecar, which `doc/projection-text.md` turned down: a projection is a
//! [`crate::Form`], bytes in and bytes out with no path, where a markdown export is lossy and one
//! way, like CSV, and its bytes twin is [`Exported`] itself. `![alt "long"]` carries the
//! picture's `svg:title` and `svg:desc`, so the alternative text comes back.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::markdown::{MONOSPACE, PREFORMATTED};
use crate::model::{Block, BlockKind, Cell, Document, Run};
use crate::style::CharStyle;

/// Whether a file *name* is a markdown file — `.md`, `.markdown` or `.mdown`, in any case.
///
/// The one question a name answers, for `csv::is_delimited_name`'s reason: plain text has no
/// signature. Asked only after the bytes have said they are not an ODF document.
pub fn is_markdown_name(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    matches!(
        file.rsplit_once('.').map(|(stem, ext)| (stem.is_empty(), ext.to_ascii_lowercase())),
        Some((false, ext)) if matches!(ext.as_str(), "md" | "markdown" | "mdown")
    )
}

/// A markdown file opened as a document of its own: what [`open`] hands a shell. The twin of
/// `grind_sheet::csv::Opened`.
#[derive(Debug)]
pub struct Opened {
    /// `notes.md` becomes `notes.fodt`. Opened with **no path**, so Save asks where the ODF
    /// document goes and can never write it over the markdown it came from.
    pub name: String,
    /// Flat ODF, for [`crate::App::open_bytes`].
    pub odf: Vec<u8>,
    /// One sentence for a notice bar.
    pub summary: String,
}

/// Open a markdown file as a new text document, its pictures found through `resolve`
/// ([`beside`] for a file on disk, [`nowhere`] for one with no directory). `None` when `name`
/// is not one ([`is_markdown_name`]), so a caller tries this after ODF and passes anything else
/// on.
pub fn open(name: &str, bytes: &[u8], resolve: &Resolve<'_>) -> Option<Result<Opened, String>> {
    is_markdown_name(name).then(|| open_markdown(name, bytes, resolve))
}

fn open_markdown(name: &str, bytes: &[u8], resolve: &Resolve<'_>) -> Result<Opened, String> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
        .map_err(|_| format!("{name}: not UTF-8"))?;
    let doc = document(text, resolve);
    let blocks = doc.blocks.len();
    let headings = doc.outline().count();
    let odf = crate::write_bytes(&doc, crate::Form::Flat).map_err(|e| format!("{name}: {e}"))?;
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    Ok(Opened {
        name: format!("{}{stem}.fodt", &name[..name.len() - file.len()]),
        odf,
        summary: format!(
            "Imported from Markdown: {blocks} block{}, {headings} heading{}.",
            if blocks == 1 { "" } else { "s" },
            if headings == 1 { "" } else { "s" },
        ),
    })
}

/// A whole document read from markdown. Never empty: a document keeps one paragraph.
pub fn document(markdown: &str, resolve: &Resolve<'_>) -> Document {
    let mut doc = Document::empty();
    let blocks = parse(markdown, &mut doc, resolve);
    if blocks.is_empty() {
        return Document::default();
    }
    doc.blocks = blocks;
    doc
}

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH
}

/// Markdown as blocks. `doc` mints their ids and names their tables, and is otherwise left
/// alone — so this serves both a new document and an import into one that has blocks already.
///
/// A picture comes in when its bytes can be had: a `data:` URI always, anything else when
/// `resolve` hands back the bytes of the (percent-decoded) path it names — [`beside`] is the one
/// that reads files. One that cannot be had, or whose bytes are no picture, is its alt text.
pub fn parse(markdown: &str, doc: &mut Document, resolve: &Resolve<'_>) -> Vec<Block> {
    let mut reader = Reader {
        doc,
        resolve,
        image: None,
        out: Vec::new(),
        open: None,
        pending: None,
        lists: 0,
        bold: 0,
        italic: 0,
        strike: 0,
        href: Vec::new(),
        code: false,
        table: None,
        taken: BTreeSet::new(),
    };
    reader.taken = reader
        .doc
        .blocks
        .iter()
        .filter_map(|b| b.cell.as_ref().map(|c| c.table.clone()))
        .collect();
    for event in Parser::new_ext(markdown, options()) {
        reader.event(event);
    }
    reader.flush();
    reader.out
}

struct TableState {
    name: String,
    next_row: u32,
    row: u32,
    column: u32,
    head: bool,
}

/// Where an import's pictures come from: a path as the markdown spelled it, decoded, to bytes.
pub type Resolve<'r> = dyn Fn(&str) -> Option<Vec<u8>> + 'r;

/// The resolver that has no files: only a `data:` URI comes in.
pub fn nowhere(_: &str) -> Option<Vec<u8>> {
    None
}

/// The resolver for a markdown file at `markdown`: a **relative** path is read from beside it,
/// and anything else — an absolute path, a URL — is not read at all, so opening a markdown file
/// cannot reach for the network or for a file somewhere it did not put one.
pub fn beside(markdown: &std::path::Path) -> impl Fn(&str) -> Option<Vec<u8>> + use<> {
    let dir = markdown
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    move |target: &str| {
        let relative = std::path::Path::new(target);
        if target.contains(':') || relative.is_absolute() || target.starts_with(['/', '\\']) {
            return None;
        }
        std::fs::read(dir.join(relative)).ok()
    }
}

/// A picture being read: where it points, the long text, and the alt text as it arrives.
struct PendingImage {
    url: String,
    title: String,
    alt: String,
}

struct Reader<'a> {
    doc: &'a mut Document,
    resolve: &'a Resolve<'a>,
    image: Option<PendingImage>,
    out: Vec<Block>,
    /// The block text is being added to.
    open: Option<Block>,
    /// The kind the next paragraph takes — a list item's first block is the item itself.
    pending: Option<BlockKind>,
    lists: u32,
    bold: u32,
    italic: u32,
    strike: u32,
    href: Vec<String>,
    code: bool,
    table: Option<TableState>,
    taken: BTreeSet<String>,
}

impl Reader<'_> {
    fn event(&mut self, event: Event) {
        if let Some(image) = &mut self.image {
            match event {
                Event::Text(text) | Event::Code(text) => image.alt.push_str(&text),
                Event::SoftBreak | Event::HardBreak => image.alt.push(' '),
                Event::End(TagEnd::Image) => self.picture(),
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) if self.code => self.code_text(&text),
            Event::Text(text) => self.run(&text, false),
            Event::Code(text) => self.run(&text, true),
            Event::SoftBreak => self.run(" ", false),
            Event::HardBreak => {
                self.ensure();
                if let Some(block) = &mut self.open {
                    block.runs.push(Run::Break);
                }
            }
            // Raw HTML, rules, footnotes, math and task markers: nothing in the model.
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                self.begin(BlockKind::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.pending = None;
                self.begin(BlockKind::Heading {
                    level: match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    },
                });
            }
            Tag::List(_) => {
                self.flush();
                self.lists += 1;
            }
            Tag::Item => {
                self.flush();
                self.pending = Some(BlockKind::ListItem { depth: self.lists });
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.pending = None;
                self.code = true;
                let _ = matches!(kind, CodeBlockKind::Fenced(_));
            }
            Tag::Table(_) => {
                self.flush();
                let mut n = 1;
                let name = loop {
                    let name = format!("Table{n}");
                    if !self.taken.contains(&name) {
                        break name;
                    }
                    n += 1;
                };
                self.taken.insert(name.clone());
                self.table = Some(TableState {
                    name,
                    next_row: 0,
                    row: 0,
                    column: 0,
                    head: false,
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = &mut self.table {
                    t.row = t.next_row;
                    t.next_row += 1;
                    t.column = 0;
                    t.head = matches!(tag, Tag::TableHead);
                }
            }
            Tag::TableCell => {
                self.flush();
                self.begin(BlockKind::Paragraph);
                if let (Some(t), Some(block)) = (&self.table, &mut self.open) {
                    block.cell = Some(Cell::new(t.name.clone(), t.row, t.column));
                }
                if self.table.as_ref().is_some_and(|t| t.head) {
                    self.bold += 1;
                }
            }
            Tag::Strong => self.bold += 1,
            Tag::Emphasis => self.italic += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.href.push(dest_url.into_string()),
            Tag::Image {
                dest_url, title, ..
            } => {
                self.image = Some(PendingImage {
                    url: dest_url.into_string(),
                    title: title.into_string(),
                    alt: String::new(),
                });
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item => self.flush(),
            TagEnd::List(_) => {
                self.flush();
                self.lists = self.lists.saturating_sub(1);
            }
            TagEnd::CodeBlock => self.code = false,
            TagEnd::Table => {
                self.flush();
                self.table = None;
            }
            TagEnd::TableCell => {
                if self.table.as_ref().is_some_and(|t| t.head) {
                    self.bold = self.bold.saturating_sub(1);
                }
                self.flush();
                if let Some(t) = &mut self.table {
                    t.column += 1;
                }
            }
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.href.pop();
            }
            _ => {}
        }
    }

    /// Open a block of `kind`, unless a list item has claimed the first one.
    fn begin(&mut self, kind: BlockKind) {
        let kind = match (&kind, self.pending.take()) {
            (BlockKind::Paragraph, Some(item)) => item,
            _ => kind,
        };
        let id = self.doc.next_id();
        self.open = Some(Block::new(id, kind));
    }

    /// Text outside any paragraph — a tight list item's own.
    fn ensure(&mut self) {
        if self.open.is_none() {
            self.begin(BlockKind::Paragraph);
        }
    }

    fn flush(&mut self) {
        if let Some(mut block) = self.open.take() {
            crate::model::coalesce(&mut block.runs);
            self.out.push(block);
        }
    }

    fn run(&mut self, text: &str, code: bool) {
        self.ensure();
        let mut props = CharStyle::default();
        if self.bold > 0 {
            props.set_bold(true);
        }
        if self.italic > 0 {
            props.set_italic(true);
        }
        if self.strike > 0 {
            props.set_struck(true);
        }
        if code {
            props.font_family = Some(MONOSPACE.to_owned());
        }
        let href = self.href.last().cloned();
        if let Some(block) = &mut self.open {
            block.runs.push(Run::Text {
                text: text.to_owned(),
                style: None,
                props,
                href,
            });
        }
    }

    /// The picture just closed: a run when its bytes can be had, its alt text when they cannot.
    fn picture(&mut self) {
        let Some(image) = self.image.take() else {
            return;
        };
        let data = match image.url.strip_prefix("data:") {
            Some(uri) => data_uri(uri),
            None => (self.resolve)(&percent_decode(&image.url)),
        };
        let found = data.and_then(|data| Some((crate::picture::mime(&data)?, data)));
        let Some((mime, data)) = found else {
            if !image.alt.is_empty() {
                self.run(&image.alt, false);
            }
            return;
        };
        self.ensure();
        let some = |s: String| (!s.is_empty()).then_some(s);
        if let Some(block) = &mut self.open {
            block.runs.push(Run::Image {
                mime: mime.to_owned(),
                data,
                width: None,
                height: None,
                anchor: None,
                title: some(image.alt),
                description: some(image.title),
            });
        }
    }

    fn code_text(&mut self, text: &str) {
        for line in text.strip_suffix('\n').unwrap_or(text).split('\n') {
            let id = self.doc.next_id();
            let mut block = Block::new(id, BlockKind::Paragraph);
            block.style = Some(PREFORMATTED.to_owned());
            if !line.is_empty() {
                block.runs.push(Run::plain(line));
            }
            self.out.push(block);
        }
    }
}

/// The bytes of a `data:` URI past its scheme — base64 or percent-encoded, as RFC 2397 allows.
fn data_uri(uri: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let (head, body) = uri.split_once(',')?;
    match head.ends_with(";base64") {
        true => {
            let cleaned: String = body.chars().filter(|c| !c.is_whitespace()).collect();
            base64::engine::general_purpose::STANDARD
                .decode(cleaned)
                .ok()
        }
        false => Some(percent_decode(body).into_bytes()),
    }
}

/// `%20` back to a space: what a path in a link spells and a file name does not.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(&a), Some(&b)) if hex(a).is_some() && hex(b).is_some() => {
                out.push((hex(a).unwrap_or(0) * 16 + hex(b).unwrap_or(0)) as u8);
                i += 3;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Where an export's pictures go. Markdown holds text, so a picture is either a file of its own
/// beside it or a `data:` URI inside it — and which is a choice about where the markdown will
/// be read, not about the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pictures {
    /// Each picture a file in this directory, relative to the markdown — `![alt](dir/img-….png)`.
    /// The default, because a renderer that shows a `data:` URI is the exception (GitHub's does
    /// not) and a diff of base64 is unreadable.
    Files { dir: String },
    /// Each picture inside the markdown as a `data:` URI: one file, nothing beside it.
    Inline,
}

impl Pictures {
    /// The directory a markdown file's pictures go in when nobody names one: `README.md`'s are
    /// `README.images`.
    pub fn beside(markdown_name: &str) -> Self {
        let file = markdown_name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(markdown_name);
        let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
        Self::Files {
            dir: format!("{stem}.images"),
        }
    }
}

/// A picture an export wrote a reference to and the caller has to write: its bytes at `path`,
/// relative to the markdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub path: String,
    pub data: Vec<u8>,
}

/// An export: the markdown, and the pictures it names ([`Pictures::Files`]). Empty for
/// [`Pictures::Inline`], whose pictures are in the text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub markdown: String,
    pub pictures: Vec<Picture>,
}

impl Exported {
    /// Write the markdown to `path` and each picture beside it — the half of an export a shell
    /// with a filesystem does not want to own, and the same in each of them. Answers the
    /// sentence to show.
    pub fn save(&self, path: &std::path::Path) -> Result<String, String> {
        let base = path.parent().unwrap_or(std::path::Path::new(""));
        if !base.as_os_str().is_empty() {
            std::fs::create_dir_all(base).map_err(|e| format!("{}: {e}", base.display()))?;
        }
        for picture in &self.pictures {
            let target = base.join(&picture.path);
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            grind_core::atomic::write(&target, &picture.data)
                .map_err(|e| format!("{}: {e}", target.display()))?;
        }
        grind_core::atomic::write(path, &self.markdown)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(self.summary(&path.display().to_string()))
    }

    /// "Wrote README.md, and 2 pictures in README.images." — what every shell says after one.
    pub fn summary(&self, name: &str) -> String {
        let dirs: BTreeSet<&str> = self
            .pictures
            .iter()
            .map(|p| p.path.rsplit_once('/').map_or(".", |(dir, _)| dir))
            .collect();
        match self.pictures.len() {
            0 => format!("Wrote {name}."),
            n => format!(
                "Wrote {name}, and {n} picture{} in {}.",
                if n == 1 { "" } else { "s" },
                dirs.into_iter().collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

/// The pictures an export has met so far.
struct Out<'a> {
    mode: &'a Pictures,
    pictures: Vec<Picture>,
}

impl Out<'_> {
    /// Where the markdown points for this picture. A file is named for its **content**, so the
    /// same picture exported twice is the same file — one picture used twice is one file, and
    /// re-exporting an unchanged document changes nothing a diff would show.
    fn target(&mut self, mime: &str, data: &[u8]) -> String {
        use base64::Engine as _;
        match self.mode {
            Pictures::Inline => format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(data)
            ),
            Pictures::Files { dir } => {
                let name = format!("img-{:016x}.{}", fnv(data), extension(mime));
                let path = match dir.trim_end_matches(['/', '\\']) {
                    "" => name,
                    dir => format!("{dir}/{name}"),
                };
                if !self.pictures.iter().any(|p| p.path == path) {
                    self.pictures.push(Picture {
                        path: path.clone(),
                        data: data.to_vec(),
                    });
                }
                path
            }
        }
    }
}

/// FNV-1a, 64 bits: a name that is the same on every machine and every build, which
/// `DefaultHasher` does not promise.
fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A file extension for a picture's MIME type.
fn extension(mime: &str) -> &str {
    match mime.to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tif",
        "image/heic" => "heic",
        _ => "bin",
    }
}

/// The document as CommonMark in one string, its pictures inside it as `data:` URIs —
/// [`export`] with [`Pictures::Inline`].
pub fn write(blocks: &[Block]) -> String {
    export(blocks, &Pictures::Inline).markdown
}

/// The document as CommonMark, its pictures where `pictures` says.
pub fn export(blocks: &[Block], pictures: &Pictures) -> Exported {
    let mut sink = Out {
        mode: pictures,
        pictures: Vec::new(),
    };
    let markdown = fold(blocks, &mut sink);
    Exported {
        markdown,
        pictures: sink.pictures,
    }
}

fn fold(blocks: &[Block], pictures: &mut Out) -> String {
    let mut out = String::new();
    let mut i = 0;
    let mut previous_item = false;
    while i < blocks.len() {
        let block = &blocks[i];
        let item = matches!(block.kind, BlockKind::ListItem { .. }) && block.cell.is_none();
        if !out.is_empty() {
            out.push_str(if item && previous_item { "\n" } else { "\n\n" });
        }
        previous_item = item;
        if let Some(cell) = &block.cell {
            let end = blocks[i..]
                .iter()
                .position(|b| b.cell.as_ref().is_none_or(|c| c.table != cell.table))
                .map_or(blocks.len(), |n| i + n);
            out.push_str(&table(&blocks[i..end], pictures));
            i = end;
        } else if block.style.as_deref() == Some(PREFORMATTED) {
            let end = blocks[i..]
                .iter()
                .position(|b| b.style.as_deref() != Some(PREFORMATTED) || b.cell.is_some())
                .map_or(blocks.len(), |n| i + n);
            out.push_str(&fence(&blocks[i..end]));
            i = end;
        } else {
            out.push_str(&one(block, pictures));
            i += 1;
        }
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn one(block: &Block, pictures: &mut Out) -> String {
    match block.kind {
        BlockKind::Heading { level } => {
            // A heading is one line; a break inside it would end it.
            let text = inline(&block.runs, pictures).replace('\n', " ");
            format!("{} {}", "#".repeat(level.clamp(1, 6) as usize), text)
        }
        BlockKind::ListItem { depth } => {
            let indent = "  ".repeat(depth.saturating_sub(1) as usize);
            let text = inline(&block.runs, pictures).replace('\n', &format!("\n{indent}  "));
            format!("{indent}- {text}")
        }
        BlockKind::Paragraph => escape_start(&inline(&block.runs, pictures)),
    }
}

fn fence(blocks: &[Block]) -> String {
    let lines: Vec<String> = blocks.iter().map(|b| b.text()).collect();
    // The fence must be longer than any run of backticks inside.
    let longest = lines.iter().map(|l| run_of(l, '`')).max().unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}\n{}\n{fence}", lines.join("\n"))
}

fn table(blocks: &[Block], pictures: &mut Out) -> String {
    let columns = blocks
        .iter()
        .filter_map(|b| b.cell.as_ref())
        .map(|c| c.column as usize + 1)
        .max()
        .unwrap_or(1);
    let mut rows: Vec<Vec<Vec<String>>> = Vec::new();
    let mut last: Option<&Cell> = None;
    for block in blocks {
        let Some(cell) = &block.cell else { continue };
        // Rows are in the order the cells are, and a cell may hold several blocks.
        if last.is_none_or(|l| l.row != cell.row) {
            rows.push(vec![Vec::new(); columns]);
        }
        let row = rows.last_mut().expect("a row was just pushed");
        row[cell.column as usize].push(inline(&block.runs, pictures).replace('\n', " "));
        last = Some(cell);
    }
    let line = |cells: &[Vec<String>]| {
        let cells: Vec<String> = cells
            .iter()
            .map(|c| c.join("<br>").replace('|', "\\|"))
            .collect();
        format!("| {} |", cells.join(" | "))
    };
    let mut lines = Vec::new();
    for (n, row) in rows.iter().enumerate() {
        lines.push(line(row));
        if n == 0 {
            lines.push(format!("|{}", " --- |".repeat(columns)));
        }
    }
    lines.join("\n")
}

fn run_of(text: &str, c: char) -> usize {
    let (mut best, mut now) = (0, 0);
    for ch in text.chars() {
        now = if ch == c { now + 1 } else { 0 };
        best = best.max(now);
    }
    best
}

/// The runs of a block as inline markdown.
fn inline(runs: &[Run], pictures: &mut Out) -> String {
    let mut out = String::new();
    for run in runs {
        match run {
            Run::Text {
                text, props, href, ..
            } => {
                let piece = span(text, props, href.as_deref());
                // A `!` just before a link would make the link a picture.
                if piece.starts_with('[') && out.ends_with('!') {
                    out.insert(out.len() - 1, '\\');
                }
                out.push_str(&piece);
            }
            Run::Tab => out.push('\t'),
            Run::Break => out.push_str("\\\n"),
            Run::Bookmark { .. } => {}
            Run::Image {
                mime,
                data,
                title,
                description,
                ..
            } => {
                let alt = title.as_deref().or(description.as_deref()).unwrap_or("");
                let alt = alt.replace('\n', " ");
                let target = pictures.target(mime, data);
                let target = match target.contains([' ', '(', ')', '<', '>']) {
                    true => format!("<{target}>"),
                    false => target,
                };
                let _ = write!(out, "![{}]({target}", escape(&alt));
                // The long text is the link's title, so it comes back as the long text.
                if let Some(description) = description.as_deref().filter(|_| title.is_some()) {
                    let quoted = description
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\n', " ");
                    let _ = write!(out, " \"{quoted}\"");
                }
                out.push(')');
            }
            // A footnote's mark or a field's value is what the reader sees, so it is what
            // markdown says.
            Run::Kept { text, .. } => out.push_str(&span(text, &CharStyle::default(), None)),
        }
    }
    out
}

fn span(text: &str, props: &CharStyle, href: Option<&str>) -> String {
    if text.is_empty() {
        return String::new();
    }
    // Emphasis cannot open before or close after a space, so the space goes outside the markers.
    let body = text.trim();
    let lead = &text[..text.len() - text.trim_start().len()];
    let trail = &text[text.trim_end().len()..];
    if body.is_empty() {
        return text.to_owned();
    }
    let code = props
        .font_family
        .as_deref()
        .is_some_and(|f| f.eq_ignore_ascii_case(MONOSPACE));
    let mut inner = if code {
        let ticks = "`".repeat(run_of(body, '`') + 1);
        let pad = if body.starts_with('`') || body.ends_with('`') {
            " "
        } else {
            ""
        };
        format!("{ticks}{pad}{body}{pad}{ticks}")
    } else {
        escape(body)
    };
    if props.is_struck() {
        inner = format!("~~{inner}~~");
    }
    if props.is_italic() {
        inner = format!("*{inner}*");
    }
    if props.is_bold() {
        inner = format!("**{inner}**");
    }
    if let Some(href) = href {
        let href = href
            .replace(' ', "%20")
            .replace(')', "%29")
            .replace('(', "%28");
        inner = format!("[{inner}]({href})");
    }
    format!("{lead}{inner}{trail}")
}

/// Backslash-escape what would otherwise be read as markup anywhere in a line.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '`' | '|' | '&'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// What only matters at the start of a line: a heading, a bullet, a quote, a numbered item.
fn escape_start(line: &str) -> String {
    let trimmed = line.trim_start();
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    let numbered = digits > 0 && matches!(trimmed[digits..].chars().next(), Some('.' | ')'));
    if trimmed.starts_with(['#', '-', '+', '=']) || numbered {
        let at = line.len() - trimmed.len() + if numbered { digits } else { 0 };
        return format!("{}\\{}", &line[..at], &line[at..]);
    }
    line.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(doc: &Document) -> Vec<String> {
        doc.blocks
            .iter()
            .map(|b| format!("{:?}:{}", b.kind, b.text()))
            .collect()
    }

    #[test]
    fn headings_paragraphs_and_lists() {
        let doc = document(
            "# Title\n\nSome *words*.\n\n- a\n  - b\n- c\n\n1. one\n",
            &nowhere,
        );
        assert_eq!(
            kinds(&doc),
            [
                "Heading { level: 1 }:Title",
                "Paragraph:Some words.",
                "ListItem { depth: 1 }:a",
                "ListItem { depth: 2 }:b",
                "ListItem { depth: 1 }:c",
                "ListItem { depth: 1 }:one",
            ]
        );
    }

    #[test]
    fn inline_formatting_and_links() {
        let doc = document("**b** *i* ~~s~~ `c` [l](http://x.y)", &nowhere);
        let runs = &doc.blocks[0].runs;
        let find = |t: &str| {
            runs.iter()
                .find_map(|r| match r {
                    Run::Text {
                        text, props, href, ..
                    } if text == t => Some((props.clone(), href.clone())),
                    _ => None,
                })
                .unwrap()
        };
        assert!(find("b").0.is_bold());
        assert!(find("i").0.is_italic());
        assert!(find("s").0.is_struck());
        assert_eq!(find("c").0.font_family.as_deref(), Some(MONOSPACE));
        assert_eq!(find("l").1.as_deref(), Some("http://x.y"));
    }

    #[test]
    fn a_fence_is_preformatted_paragraphs_and_comes_back_a_fence() {
        let doc = document("```rust\nfn a() {}\n\nfn b() {}\n```\n", &nowhere);
        assert_eq!(doc.blocks.len(), 3);
        assert!(
            doc.blocks
                .iter()
                .all(|b| b.style.as_deref() == Some(PREFORMATTED))
        );
        assert_eq!(write(&doc.blocks), "```\nfn a() {}\n\nfn b() {}\n```\n");
    }

    #[test]
    fn a_table_is_cells_and_the_first_row_is_bold() {
        let doc = document("| a | b |\n|---|---|\n| 1 | 2 |\n", &nowhere);
        assert_eq!(doc.blocks.len(), 4);
        let cell = doc.blocks[3].cell.as_ref().unwrap();
        assert_eq!((cell.row, cell.column), (1, 1));
        assert!(matches!(&doc.blocks[0].runs[0], Run::Text { props, .. } if props.is_bold()));
        assert_eq!(
            write(&doc.blocks),
            "| **a** | **b** |\n| --- | --- |\n| 1 | 2 |\n"
        );
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let source = "# T\n\nA **bold** and *it* with `code` and [a link](http://a.b/c).\n\n- one\n  - two\n- three\n\nLine\\\nbreak\n";
        let doc = document(source, &nowhere);
        let written = write(&doc.blocks);
        assert_eq!(written, source);
        let again = document(&written, &nowhere);
        assert_eq!(kinds(&doc), kinds(&again));
    }

    #[test]
    fn markup_characters_in_text_are_escaped() {
        let mut doc = Document::empty();
        let id = doc.next_id();
        let mut block = Block::new(id, BlockKind::Paragraph);
        block.runs.push(Run::plain("# not a heading *or* [link]"));
        let written = write(&[block]);
        let back = document(&written, &nowhere);
        assert_eq!(back.blocks[0].text(), "# not a heading *or* [link]");
        assert_eq!(back.blocks[0].kind, BlockKind::Paragraph);
    }

    #[test]
    fn emphasis_keeps_its_spaces_outside_the_markers() {
        let mut props = CharStyle::default();
        props.set_bold(true);
        assert_eq!(span(" b ", &props, None), " **b** ");
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-pixels-";

    fn picture(title: Option<&str>, description: Option<&str>) -> Block {
        let mut block = Block::new(crate::model::BlockId(1), BlockKind::Paragraph);
        block.runs = vec![
            Run::plain("See "),
            Run::Image {
                mime: "image/png".to_owned(),
                data: PNG.to_vec(),
                width: Some("4cm".to_owned()),
                height: None,
                anchor: None,
                title: title.map(str::to_owned),
                description: description.map(str::to_owned),
            },
        ];
        block
    }

    #[test]
    fn a_picture_is_a_file_named_for_its_bytes() {
        let blocks = [picture(Some("A chart"), None), picture(None, None)];
        let out = export(&blocks, &Pictures::beside("docs/README.md"));
        let path = format!("README.images/img-{:016x}.png", fnv(PNG));
        assert_eq!(
            out.markdown,
            format!("See ![A chart]({path})\n\nSee ![]({path})\n")
        );
        // The same picture twice is one file.
        assert_eq!(
            out.pictures,
            [Picture {
                path,
                data: PNG.to_vec()
            }]
        );
        // And the name is the same on every export.
        assert_eq!(export(&blocks, &Pictures::beside("README.md")), out);
    }

    #[test]
    fn a_picture_inline_is_a_data_uri_and_reads_back() {
        let written = write(&[picture(Some("A [chart]"), Some("Sales \"by\" month"))]);
        assert!(
            written.starts_with("See ![A \\[chart\\]](data:image/png;base64,"),
            "{written}"
        );
        let back = document(&written, &nowhere);
        let Run::Image {
            mime,
            data,
            title,
            description,
            ..
        } = &back.blocks[0].runs[1]
        else {
            panic!("{:?}", back.blocks[0].runs)
        };
        assert_eq!((mime.as_str(), data.as_slice()), ("image/png", PNG));
        assert_eq!(title.as_deref(), Some("A [chart]"));
        assert_eq!(description.as_deref(), Some("Sales \"by\" month"));
    }

    #[test]
    fn a_picture_is_found_through_the_resolver_or_is_its_alt_text() {
        let resolve = |path: &str| (path == "my pics/a.png").then(|| PNG.to_vec());
        let doc = document("![one](my%20pics/a.png) ![two](missing.png)", &resolve);
        let runs = &doc.blocks[0].runs;
        assert!(matches!(&runs[0], Run::Image { title, .. } if title.as_deref() == Some("one")));
        assert_eq!(doc.blocks[0].text(), "\u{fffc} two");
        // Bytes that are no picture stay the alt text too.
        let doc = document("![x](a.png)", &|_: &str| Some(b"text".to_vec()));
        assert_eq!(doc.blocks[0].text(), "x");
    }

    #[test]
    fn the_resolver_beside_a_file_reads_only_relative_paths() {
        let dir = std::env::temp_dir().join(format!("grind-md-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("img")).unwrap();
        std::fs::write(dir.join("img/a.png"), PNG).unwrap();
        let resolve = beside(&dir.join("notes.md"));
        assert_eq!(resolve("img/a.png").as_deref(), Some(PNG));
        let absolute = dir.join("img/a.png").display().to_string();
        assert_eq!(resolve(&absolute), None);
        assert_eq!(resolve("https://example.org/a.png"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_bang_before_a_link_is_not_a_picture() {
        let mut block = Block::new(crate::model::BlockId(1), BlockKind::Paragraph);
        block.runs = vec![
            Run::plain("Wow!"),
            Run::Text {
                text: "here".to_owned(),
                style: None,
                props: CharStyle::default(),
                href: Some("http://a.b".to_owned()),
            },
        ];
        let back = document(&write(&[block]), &nowhere);
        assert_eq!(back.blocks[0].text(), "Wow!here");
    }

    #[test]
    fn names_and_empty_input() {
        for name in ["a.md", "A.MD", "x/y.markdown"] {
            assert!(is_markdown_name(name), "{name}");
        }
        for name in ["a.fodt", ".md", "md", "a.md.fodt"] {
            assert!(!is_markdown_name(name), "{name}");
        }
        assert_eq!(document("", &nowhere).blocks.len(), 1);
        assert!(open("a.txt", b"x", &nowhere).is_none());
        let opened = open("dir/notes.md", b"# Hi\n", &nowhere).unwrap().unwrap();
        assert_eq!(opened.name, "dir/notes.fodt");
        assert!(crate::open_bytes(&opened.name, &opened.odf).is_ok());
    }
}
