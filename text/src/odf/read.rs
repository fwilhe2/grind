// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ODT content model: one context per element read (`doc/odt-format.md` §§2–3). **\[ODT\]**
//! Everything else falls to `grind_core::odf::context::Ignore`, so the `text-content` alternatives
//! outside the scope line (`doc/text-core.md`) cost no code. Contexts never talk to each other;
//! all mutation goes through the shared [`Builder`].

use std::collections::HashMap;

use grind_core::odf::context::{Attrs, Context};
use grind_core::odf::names::{Name, Ns};
use grind_core::odf::xml::element_extent;

use crate::model::{Block, BlockId, BlockKind, Document, Run};
use crate::style::{self, CharStyle};

/// How deeply a list may nest before the count stops — so a crafted ten-thousand-deep `text:list`
/// cannot overflow it (§9); the context stack is bounded separately.
const MAX_LIST_DEPTH: u32 = 64;

/// How many `style:style` definitions a document may contribute before collecting stops, so a
/// million declared styles cannot grow a map without bound. Far above a real document's few
/// thousand.
const MAX_STYLES: usize = 100_000;

/// Everything the contexts share: the document under construction, and the style stack.
pub struct Builder {
    pub doc: Document,
    /// The named `text:style-name`s currently open, outermost first — composed into a run's name,
    /// since `text:span` nests and the model is flat (`doc/text-core.md`). An automatic style's
    /// span pushes its parent's name, or nothing, so the stack stays parallel with
    /// [`Builder::props`]; its generated name never survives ([`crate::style`]).
    spans: Vec<Option<String>>,
    /// The direct formatting of each open span, parallel to [`Builder::spans`] and already
    /// composed with everything outside it.
    props: Vec<CharStyle>,
    /// Every `style:style` of family `text`, by name — collected before the body, which ODF always
    /// puts after the style sections.
    styles: HashMap<String, TextFamily>,
    /// `office:font-face-decls`: a `style:name` to its family, resolved on reading so the model
    /// carries the family rather than the reference.
    fonts: HashMap<String, String>,
    /// The `xlink:href` of the open `text:a`, if any. One slot: `text:a` holds
    /// `paragraph-content`, so links cannot nest (rng:16453).
    href: Option<String>,
    /// How deep in `text:list` elements we are. 0 outside any list.
    list_depth: u32,
    /// The `text:list` elements open now, outermost first, with where each starts and its
    /// attributes (`Source::lists`).
    open_lists: Vec<(usize, String)>,
    /// The tables open now, outermost first, each with the row number its next `table:table-row`
    /// takes.
    tables: Vec<OpenTable>,
    /// How many tables have been opened, for naming one the document did not name.
    next_table: u32,
    /// The `table:table-cell` open now, if any; every block opened meanwhile is in that cell
    /// (`crate::model::Cell`). A cell of a nested table leaves it as it is, so the inner table's
    /// paragraphs belong to the outer cell ([`TableCell`]).
    cell: Option<crate::model::Cell>,
    /// The image being assembled from the open `draw:frame`(s) ([`PendingImage`]).
    image: Option<PendingImage>,
    /// How many `draw:frame` are open. LibreOffice wraps a resizable image in an inner frame
    /// (through a `draw:text-box`); only the outermost frame's end emits a [`Run::Image`].
    frame_depth: u32,
    /// The original bytes when they are a package, for a `draw:image`'s `xlink:href` into
    /// `Pictures/`.
    package: Option<Vec<u8>>,
    /// Every `style:page-layout` read so far, by name.
    page_layouts: HashMap<String, grind_core::page::PageGeometry>,
    /// Every paragraph style read so far, and whether it was automatic; `styles.xml`'s automatic
    /// ones are forgotten before `content.xml` is read, like [`Builder::styles`]'.
    paragraph_styles: HashMap<String, (bool, crate::paragraph::ParagraphStyle)>,
    /// Every `text:list-style`, by name, and whether it is an automatic one.
    list_styles: HashMap<String, (bool, crate::numbering::ListStyle)>,
    /// The levels of the `text:list-style` being read.
    list_levels: std::collections::BTreeMap<u32, crate::numbering::Level>,
    /// The lists open around the block being read, innermost last: the list style in effect and
    /// the numbering chain it counts in (`crate::numbering::ListMark`).
    list_frames: Vec<(Option<String>, usize)>,
    /// The numbering chains handed out, the last top-level chain of each style, and each
    /// `xml:id`'s — what `text:continue-numbering` and `text:continue-list` continue.
    chains: usize,
    chain_of_style: HashMap<Option<String>, usize>,
    chain_of_id: HashMap<String, usize>,
    /// The list item being read: its `text:start-value`, whether it is a `text:list-header`, and
    /// whether its first block is still to come.
    item: Option<(Option<u32>, bool, bool)>,
    /// The `style:tab-stops` of the paragraph style being read ([`TabStopsDef`]).
    tab_stops: Option<Vec<crate::paragraph::TabStop>>,
    /// Every `style:master-page` read so far — its name and page layout — in document order.
    master_pages: Vec<(String, String)>,
    /// Each master page's `style:next-style-name`.
    master_next: HashMap<String, String>,
    /// Each master page's header and footer paragraphs, by its name.
    master_marginals: HashMap<String, Marginals>,
    /// Each page layout's room for a header and a footer, in millimetres: `(min-height, spacing)`.
    layout_marginals: HashMap<String, (Option<Room>, Option<Room>)>,
    /// The header or footer paragraph being read, and those of it already read.
    marginal: Option<crate::marginal::Paragraph>,
    marginal_done: Vec<crate::marginal::Paragraph>,
    /// The lines found in the header or footer being read, taken by the master when it closes.
    marginal_lines: Vec<crate::marginal::Line>,
    /// Each graphic style's stroke — `(width in mm, #rrggbb)` — for the lines a master draws.
    graphic_strokes: HashMap<String, (f64, String)>,
    /// The citation of the note being read.
    citation: String,
    /// Whether the blocks being read are an index's generated entries ([`Block::generated`]).
    generating: bool,
    /// The innermost section open around the blocks being read ([`Block::section`]).
    section: Option<usize>,
    /// Every table-cell style read so far, and whether it was automatic.
    cell_styles: HashMap<String, (bool, crate::table_look::CellLook)>,
    /// Every table-column style's width in millimetres, and whether it was automatic.
    column_styles: HashMap<String, (bool, Option<f64>)>,
    /// Every table-row style that keeps its row together or gives it a height, and whether it was
    /// automatic.
    row_styles: HashMap<String, (bool, RowStyle)>,
    /// The level indents of the list or outline style being read.
    list_indents: std::collections::BTreeMap<u32, crate::numbering::LevelIndent>,
    /// Each table's look as read, by name — its column styles, heading rows and each cell's
    /// style — resolved once every style is known.
    table_pending: HashMap<String, PendingLook>,
}

/// What a table-row style says about its row.
#[derive(Clone, Copy, Default)]
struct RowStyle {
    /// `fo:keep-together="always"`.
    kept: bool,
    /// `style:min-row-height`, or `style:row-height`, in millimetres.
    height: Option<f64>,
}

/// A table's look before its styles are resolved.
#[derive(Default)]
struct PendingLook {
    columns: Vec<Option<String>>,
    header_rows: std::collections::BTreeSet<u32>,
    cells: Vec<((u32, u32), String)>,
    /// Each styled row's first number, how many it stands for, and its style.
    row_styles: Vec<(u32, u32, String)>,
    rows: u32,
}

/// The room a page layout gives a header or footer, in millimetres: `(min-height, spacing)`.
type Room = (f64, f64);

/// A master page's header and footer paragraphs, as read.
#[derive(Clone, Default)]
struct Marginals {
    header: Option<Vec<crate::marginal::Paragraph>>,
    footer: Option<Vec<crate::marginal::Paragraph>>,
    /// `style:header-first` and `style:footer-first`.
    header_first: Option<Vec<crate::marginal::Paragraph>>,
    footer_first: Option<Vec<crate::marginal::Paragraph>>,
    /// The `draw:line`s in either, which belong to the page rather than to a paragraph.
    lines: Vec<crate::marginal::Line>,
}

/// One `table:table` being read: its name, and the row number the next row in it takes.
struct OpenTable {
    name: String,
    row: u32,
    /// Whether this table is recorded in [`super::source::Source::tables`]: a top-level table read
    /// from a file.
    recorded: bool,
    /// Where its start tag ends, until the first row says where the column declarations do.
    columns_from: Option<usize>,
    /// How many blocks the document had when it opened.
    before: usize,
}

/// One `draw:frame` (rng:5089) being read, gathered across however many frames are nested around
/// the `draw:image`; LibreOffice always wraps one in a second for resizing.
#[derive(Default)]
struct PendingImage {
    /// `draw:mime-type` off the `draw:image` itself.
    mime: Option<String>,
    /// The bytes, once `office:binary-data` has been read out and decoded.
    data: Option<Vec<u8>>,
    /// `svg:width` / `svg:height` from the first frame that has them, outermost preferred.
    width: Option<String>,
    height: Option<String>,
    /// `text:anchor-type` from the outermost frame — whether the picture sits in the sentence or
    /// at the top of the paragraph.
    anchor: Option<String>,
    /// `svg:title` / `svg:desc` from the first frame that has them — the inner one when
    /// LibreOffice wrapped a caption.
    title: Option<String>,
    description: Option<String>,
    /// The plain text of the frame's own caption paragraph — everything [`TextBoxSearch`] sees
    /// except the nested frame. Emitted when the outermost frame closes, so it lands after the
    /// image whatever order its parts arrive in.
    caption: String,
    /// The outermost frame's start tag in the source, for [`super::source::Source::frames`].
    start: Option<std::ops::Range<usize>>,
}

/// One `style:style` of family `text`, as the document declared it.
struct TextFamily {
    /// Whether it came from `office:automatic-styles`: an automatic style may be resolved away, a
    /// named one may not ([`crate::style`]).
    automatic: bool,
    parent: Option<String>,
    props: CharStyle,
    /// Whether it says anything `props` does not carry ([`CharStyle::origin`]).
    extras: bool,
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            // Blocks come from the file; a document declaring none stays empty
            // (`Document::default` is for new documents).
            doc: Document::empty(),
            spans: Vec::new(),
            props: Vec::new(),
            styles: HashMap::new(),
            fonts: HashMap::new(),
            href: None,
            list_depth: 0,
            open_lists: Vec::new(),
            tables: Vec::new(),
            next_table: 0,
            cell: None,
            image: None,
            frame_depth: 0,
            package: None,
            page_layouts: HashMap::new(),
            paragraph_styles: HashMap::new(),
            tab_stops: None,
            list_styles: HashMap::new(),
            list_levels: Default::default(),
            list_frames: Vec::new(),
            chains: 0,
            chain_of_style: HashMap::new(),
            chain_of_id: HashMap::new(),
            item: None,
            master_pages: Vec::new(),
            master_next: HashMap::new(),
            master_marginals: HashMap::new(),
            layout_marginals: HashMap::new(),
            marginal: None,
            marginal_done: Vec::new(),
            marginal_lines: Vec::new(),
            graphic_strokes: HashMap::new(),
            citation: String::new(),
            generating: false,
            section: None,
            cell_styles: HashMap::new(),
            column_styles: HashMap::new(),
            row_styles: HashMap::new(),
            table_pending: HashMap::new(),
            list_indents: std::collections::BTreeMap::new(),
        }
    }

    /// Resolve every table's look against the cell and column styles, once every part is read.
    pub fn settle_table_looks(&mut self) {
        for (name, pending) in std::mem::take(&mut self.table_pending) {
            let columns = pending
                .columns
                .iter()
                .map(|style| {
                    let style = style.as_deref()?;
                    self.column_styles.get(style).and_then(|(_, width)| *width)
                })
                .collect();
            let cells = pending
                .cells
                .into_iter()
                .filter_map(|(at, style)| Some((at, self.cell_styles.get(&style)?.1.clone())))
                .collect();
            let styled_rows = || {
                pending
                    .row_styles
                    .iter()
                    .filter_map(|(row, repeated, style)| {
                        let (_, style) = self.row_styles.get(style)?;
                        Some((*row..row + repeated, *style))
                    })
            };
            let kept_rows = styled_rows()
                .filter(|(_, style)| style.kept)
                .flat_map(|(rows, _)| rows)
                .collect();
            let row_heights = styled_rows()
                .filter_map(|(rows, style)| {
                    let height = style.height?;
                    Some(rows.map(move |row| (row, height)))
                })
                .flatten()
                .collect();
            let look = crate::table_look::TableLook {
                columns,
                header_rows: pending.header_rows,
                cells,
                kept_rows,
                row_heights,
                rows: pending.rows,
            };
            self.doc.table_looks.insert(name, look);
        }
    }

    /// Hand the paragraph styles to the document once every part is read.
    pub fn settle_paragraph_styles(&mut self) {
        self.doc.list_styles = std::mem::take(&mut self.list_styles)
            .into_iter()
            .map(|(name, (_, style))| (name, style))
            .collect();
        self.doc.paragraph_styles = std::mem::take(&mut self.paragraph_styles)
            .into_iter()
            .map(|(name, (_, style))| (name, style))
            .collect();
    }

    /// Settle [`Document::page`] once every part is read: the master page called `Standard`, or
    /// the first there is, and its page layout; a layout nobody declared is no page. Called after
    /// the paragraph styles settle, since the first paragraph's style decides which master starts
    /// the document.
    pub fn settle_page(&mut self) {
        let named = |name: &str| self.master_pages.iter().find(|(m, _)| m == name);
        // The master the first paragraph's style names (Writer's First Page), and the one later
        // pages are set on: its `next-style-name`, else itself.
        let first = self
            .doc
            .blocks
            .first()
            .and_then(|block| {
                crate::paragraph::resolve(
                    &self.doc.paragraph_styles,
                    &self.doc.default_paragraph,
                    block.style.as_deref(),
                )
                .master_page
            })
            .and_then(|name| named(&name).cloned());
        let rest = match &first {
            Some((name, _)) => self
                .master_next
                .get(name)
                .and_then(|next| named(next).cloned())
                .or_else(|| first.clone()),
            None => named("Standard")
                .or_else(|| self.master_pages.first())
                .cloned(),
        };
        self.doc.page = rest
            .as_ref()
            .and_then(|(_, layout)| self.page_layouts.get(layout).copied());
        let marginals_of = |master: &(String, String)| {
            let marginals = self
                .master_marginals
                .get(&master.0)
                .cloned()
                .unwrap_or_default();
            let (header_room, footer_room) = self
                .layout_marginals
                .get(&master.1)
                .copied()
                .unwrap_or_default();
            let marginal = |paragraphs: Option<Vec<crate::marginal::Paragraph>>,
                            room: Option<(f64, f64)>| {
                paragraphs.map(|paragraphs| {
                    let (min_height, spacing) = room.unwrap_or_default();
                    crate::marginal::Marginal {
                        paragraphs,
                        min_height,
                        spacing,
                    }
                })
            };
            let first = (marginals.header_first.is_some() || marginals.footer_first.is_some())
                .then(|| {
                    (
                        marginal(marginals.header_first, header_room),
                        marginal(marginals.footer_first, footer_room),
                    )
                });
            (
                marginal(marginals.header, header_room),
                marginal(marginals.footer, footer_room),
                first,
            )
        };
        if let Some(rest) = &rest {
            (self.doc.header, self.doc.footer, _) = marginals_of(rest);
        }
        self.doc.first_page = first
            .filter(|first| rest.as_ref().is_some_and(|rest| rest.0 != first.0))
            .map(|first| {
                let (header, footer, _) = marginals_of(&first);
                (header, footer)
            });

        // Every master page by name, so a paragraph that starts a page on its own master anywhere
        // — a letter in a series — gets its page, header and footer (`crate::model::Master`).
        self.doc.rest_master = rest.as_ref().map(|(name, _)| name.clone());
        let masters: Vec<(String, crate::model::Master)> = self
            .master_pages
            .iter()
            .map(|master| {
                let (header, footer, first) = marginals_of(master);
                (
                    master.0.clone(),
                    crate::model::Master {
                        geometry: self.page_layouts.get(&master.1).copied(),
                        header,
                        footer,
                        first,
                        lines: self
                            .master_marginals
                            .get(&master.0)
                            .map(|m| m.lines.clone())
                            .unwrap_or_default(),
                        next: self.master_next.get(&master.0).cloned(),
                    },
                )
            })
            .collect();
        self.doc.masters = masters.into_iter().collect();
    }

    /// Record the package being read, for resolving `xlink:href`s. Called before parsing.
    pub fn set_package(&mut self, bytes: Vec<u8>) {
        self.package = Some(bytes);
    }

    /// A part of the package by path; `None` for the flat form and for a missing part.
    fn resolve_part(&self, path: &str) -> Option<Vec<u8>> {
        super::package::part(self.package.as_deref()?, path)
    }

    /// Open a `text:span`, resolving its style into a name the model keeps and formatting it
    /// applies — where `doc/text-core.md`'s line between generated and named styles is drawn. An
    /// undeclared name is kept as a name rather than given invented formatting.
    fn open_span(&mut self, name: Option<&str>) {
        let mut props = self.props.last().cloned().unwrap_or_default();
        let kept = match name.and_then(|name| self.styles.get(name).map(|s| (name, s))) {
            Some((name, style)) if style.automatic => {
                props.layer(&style.props);
                // Where the formatting came from, when that style says more than the fields, so a
                // restyle starts from it ([`CharStyle::origin`]).
                if style.extras {
                    props.origin = crate::style::Origin::new(name);
                }
                style.parent.clone()
            }
            // A declared named style stays behind its name, so `Emphasis` is a structural fact
            // rather than an italic that forgot why.
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

    /// Drop every automatic style read so far — after `styles.xml`, whose automatic styles only
    /// that part can name.
    pub fn forget_automatic_styles(&mut self) {
        // A header's or footer's paragraphs name `styles.xml`'s automatic styles (`MP1`), which
        // `content.xml` may reuse for something else, so each is kept under a name no file can
        // spell.
        for paragraphs in self
            .master_marginals
            .values_mut()
            .flat_map(|m| [m.header.as_mut(), m.footer.as_mut()])
            .flatten()
        {
            for paragraph in paragraphs {
                let Some(name) = paragraph.style.as_deref() else {
                    continue;
                };
                let Some((true, style)) = self.paragraph_styles.get(name) else {
                    continue;
                };
                let kept = format!("{MARGINAL_STYLE}{name}");
                let style = style.clone();
                self.paragraph_styles.insert(kept.clone(), (false, style));
                paragraph.style = Some(kept);
            }
        }
        self.cell_styles.retain(|_, (automatic, _)| !*automatic);
        self.column_styles.retain(|_, (automatic, _)| !*automatic);
        self.row_styles.retain(|_, (automatic, _)| !*automatic);
        self.paragraph_styles
            .retain(|_, (automatic, _)| !*automatic);
        self.list_styles.retain(|_, (automatic, _)| !*automatic);
        self.styles.retain(|_, style| !style.automatic);
    }

    /// Hand the automatic character styles to the source, so a formatting edit can splice by
    /// reusing a name the file spells (`super::source::TextStyle`). Sorted by name, so two saves
    /// of one document are the same bytes.
    pub fn publish_styles(&mut self) {
        let mut styles: Vec<super::source::TextStyle> = self
            .styles
            .iter()
            .filter(|(_, style)| style.automatic && (style.extras || !style.props.is_plain()))
            .map(|(name, style)| super::source::TextStyle {
                name: name.clone(),
                parent: style.parent.clone(),
                props: style.props.clone(),
                extras: style.extras,
            })
            .collect();
        styles.sort_by(|a, b| a.name.cmp(&b.name));
        self.doc.char_styles = self
            .styles
            .iter()
            .filter(|(_, style)| !style.automatic)
            .map(|(name, style)| {
                let named = crate::paragraph::NamedChar {
                    parent: style.parent.clone(),
                    props: style.props.clone(),
                };
                (name.clone(), named)
            })
            .collect();
        if let Some(source) = self.doc.source.as_deref_mut() {
            source.styles = styles;
        }
    }

    /// Start a block and make it current, returning its id for recording where it sat (R6).
    fn open(&mut self, kind: BlockKind, style: Option<String>) -> BlockId {
        let id = self.doc.next_id();
        let mut block = Block::new(id, kind);
        block.style = style;
        // Its table cell, if one is open around it.
        block.cell = self.cell.clone();
        block.generated = self.generating;
        block.section = self.section;
        // Which list a list item, or a heading inside one, is in, for its label
        // (`crate::numbering`).
        let in_list = match block.kind {
            BlockKind::ListItem { .. } => Some(None),
            BlockKind::Heading { .. } if self.list_depth > 0 => Some(Some(self.list_depth)),
            _ => None,
        };
        if let (Some(in_list), Some((style, chain))) = (in_list, self.list_frames.last()) {
            let (start, header, first) = match self.item.as_mut() {
                Some((start, header, fresh)) => {
                    let first = std::mem::replace(fresh, false);
                    (start.filter(|_| first), *header, first)
                }
                None => (None, false, true),
            };
            self.doc.list_marks.insert(
                id,
                crate::numbering::ListMark {
                    style: style.clone(),
                    chain: *chain,
                    start,
                    header,
                    first,
                    in_list,
                },
            );
        }
        // Every section the block is inside holds it, the outer ones too.
        let mut at = self.section;
        while let Some(index) = at {
            let Some(section) = self.doc.sections.get_mut(index) else {
                break;
            };
            section.blocks.push(id);
            at = section.parent;
        }
        self.doc.blocks.push(block);
        id
    }

    /// Remember where a picture's outermost frame is, so a regenerated paragraph writes the file's
    /// own frame back.
    fn record_frame(&mut self, image: &Run, start_tag: Option<std::ops::Range<usize>>) {
        let (Some(key), Some(start_tag)) = (super::source::image_key(image), start_tag) else {
            return;
        };
        let Some(source) = self.doc.source.as_deref_mut() else {
            return;
        };
        if let Some(range) = element_extent(&source.bytes, start_tag) {
            source.frames.push((key, range));
        }
    }

    /// R6: remember where this block's element is, so a save can replace it in place. A block
    /// whose extent does not resolve cannot be spliced, and the writer regenerates.
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
        let lists = self.open_lists.clone();
        let (depth, cell) = self
            .doc
            .blocks
            .last()
            .filter(|block| block.id == id)
            .map(|block| {
                let depth = match block.kind {
                    BlockKind::ListItem { depth } => depth,
                    _ => 0,
                };
                (depth, block.cell.clone())
            })
            .unwrap_or_default();
        if let Some(source) = self.doc.source.as_deref_mut() {
            source.blocks.insert(
                id,
                super::source::Block {
                    range,
                    keep,
                    depth,
                    cell,
                },
            );
            if !lists.is_empty() {
                source.lists.insert(id, lists);
            }
        }
    }

    /// The column declarations of the table being read end where `at` begins — called by its first
    /// row or header-row group.
    fn columns_end(&mut self, at: usize) {
        let Some(table) = self.tables.last_mut() else {
            return;
        };
        let (Some(from), true) = (table.columns_from.take(), table.recorded) else {
            return;
        };
        let name = table.name.clone();
        if let Some(source) = self.doc.source.as_deref_mut()
            && let Some(columns) = source
                .bytes
                .get(from..at)
                .and_then(|b| std::str::from_utf8(b).ok())
        {
            let columns = columns.trim().to_owned();
            if let Some(recorded) = source.tables.get_mut(&name) {
                recorded.columns = columns;
            }
        }
    }

    /// Hand the start tag at `span` to the table being read, when that table is recorded and the
    /// tag's prefixes are ones a writer can change.
    fn record_table(
        &mut self,
        mut with: impl FnMut(&mut super::source::TableSource, &str),
        span: std::ops::Range<usize>,
    ) {
        let Some(table) = self.tables.last().filter(|t| t.recorded) else {
            return;
        };
        let name = table.name.clone();
        let Some(source) = self.doc.source.as_deref_mut() else {
            return;
        };
        let Some(tag) = source
            .bytes
            .get(span)
            .and_then(|tag| std::str::from_utf8(tag).ok())
        else {
            return;
        };
        if !tag.starts_with("<table:") {
            return;
        }
        let open = match tag.trim_end().strip_suffix("/>") {
            Some(body) => format!("{}>", body.trim_end()),
            None => tag.to_owned(),
        };
        if let Some(recorded) = source.tables.get_mut(&name) {
            with(recorded, &open);
        }
    }

    /// Remember an element of `office:text` the model does not read, and the block it followed
    /// (`super::source::Sibling`).
    fn record_sibling(&mut self, start_tag: std::ops::Range<usize>) {
        // Inside a section an unread element travels with the section's own bytes; kept twice, it
        // would be written twice.
        if self.section.is_some() {
            return;
        }
        let after = self.doc.blocks.last().map(|block| block.id);
        let Some(source) = self.doc.source.as_deref_mut() else {
            return;
        };
        if let Some(range) = element_extent(&source.bytes, start_tag) {
            source
                .siblings
                .push(super::source::Sibling { after, range });
        }
    }

    /// Append text to the block being read with the open styling, merging into the previous run
    /// when the styling matches, so text split by an entity reference is one run.
    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let names: Vec<&str> = self.spans.iter().flatten().map(String::as_str).collect();
        let style = (!names.is_empty()).then(|| names.join(" "));
        let props = self.props.last().cloned().unwrap_or_default();
        let href = self.href.clone();
        let Some(block) = self.doc.blocks.last_mut() else {
            // Character data outside any block — indentation between elements, or structure not
            // followed — is dropped (§9).
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

    /// The element whose start tag is at `start`, as a [`Run::Kept`] showing `text`, or just the
    /// text when there are no bytes to keep. A kept element showing nothing is zero-width, a
    /// position like a bookmark.
    fn keep(&mut self, start: std::ops::Range<usize>, text: &str) {
        let xml = self
            .doc
            .source
            .as_deref()
            .filter(|_| !start.is_empty())
            .and_then(|source| {
                let range = grind_core::odf::xml::element_extent(&source.bytes, start)?;
                std::str::from_utf8(source.bytes.get(range)?)
                    .ok()
                    .map(str::to_owned)
            });
        match xml {
            Some(xml) => self.push_run(Run::Kept {
                xml,
                text: text.to_owned(),
            }),
            None => self.push_text(text),
        }
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
            // Automatic styles are generated and may be resolved into direct formatting; named
            // ones keep their names.
            (Ns::Office, "automatic-styles") => Some(Box::new(Styles { automatic: true })),
            (Ns::Office, "styles") => Some(Box::new(Styles { automatic: false })),
            (Ns::Office, "font-face-decls") => Some(Box::new(FontFaces)),
            (Ns::Office, "master-styles") => Some(Box::new(MasterStyles)),
            _ => None,
        }
    }
}

/// `office:font-face-decls` — `style:font-name` to a real family, so the model holds `"Georgia"`
/// rather than `"F1"`.
struct FontFaces;

impl Context<Builder> for FontFaces {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "font-face")
            && let Some(declared) = attrs.get(Ns::Style, "name")
        {
            // `svg:font-family` is ODF's place; some producers write `fo:font-family`.
            if let Some(family) = attrs
                .get(Ns::Svg, "font-family")
                .or_else(|| attrs.get(Ns::Fo, "font-family"))
                && b.fonts.len() < MAX_STYLES
            {
                let family = style::unquote_family(family);
                if let Some(generic) = attrs.get(Ns::Style, "font-family-generic") {
                    b.doc
                        .font_generics
                        .insert(family.clone(), generic.to_owned());
                }
                b.fonts.insert(declared.to_owned(), family);
            }
        }
        None
    }
}

/// The prefix a header's or footer's own automatic paragraph style is kept under once `styles.xml`
/// is read ([`Builder::forget_automatic_styles`]); no `NCName` starts with a space.
const MARGINAL_STYLE: &str = " styles.xml ";

/// `office:master-styles`: each `style:master-page`'s name and page layout, and its headers and
/// footers into [`crate::marginal`], for printing.
struct MasterStyles;

impl Context<Builder> for MasterStyles {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "master-page")
            && let (Some(master), Some(layout)) = (
                attrs.get(Ns::Style, "name"),
                attrs.get(Ns::Style, "page-layout-name"),
            )
            && b.master_pages.len() < MAX_STYLES
        {
            b.master_pages.push((master.to_owned(), layout.to_owned()));
            if let Some(next) = attrs.get(Ns::Style, "next-style-name") {
                b.master_next.insert(master.to_owned(), next.to_owned());
            }
            return Some(Box::new(MasterPage {
                name: master.to_owned(),
            }));
        }
        None
    }
}

/// One `style:master-page`: its header and footer (`crate::marginal`), and its first page's.
/// The left-page variants are ignored.
struct MasterPage {
    name: String,
}

/// Which of a master page's headers and footers is being read.
#[derive(Clone, Copy)]
enum Place {
    Header,
    Footer,
    HeaderFirst,
    FooterFirst,
}

impl Context<Builder> for MasterPage {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        let place = match (name.ns, name.local.as_str()) {
            (Ns::Style, "header") => Place::Header,
            (Ns::Style, "footer") => Place::Footer,
            (Ns::Style, "header-first") => Place::HeaderFirst,
            (Ns::Style, "footer-first") => Place::FooterFirst,
            _ => return None,
        };
        Some(Box::new(MarginalText {
            master: self.name.clone(),
            place,
            top: true,
        }))
    }
}

/// A `style:header` or `style:footer`: its paragraphs, as text and page fields. Those in a
/// table or a list are read in order as its own, the grid and the labels dropped — a Word
/// header is often a one-row table holding a reference and a logo.
struct MarginalText {
    master: String,
    place: Place,
    /// Whether this is the header or footer itself rather than a `text:section`, table or list
    /// inside one; LibreOffice wraps a header in a section when one was put there
    /// (`tdf116848.odt`).
    top: bool,
}

impl Context<Builder> for MarginalText {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if matches!(
            (name.ns, name.local.as_str()),
            (Ns::Text, "section" | "list" | "list-item" | "list-header")
                | (
                    Ns::Table,
                    "table"
                        | "table-header-rows"
                        | "table-rows"
                        | "table-row-group"
                        | "table-row"
                        | "table-cell"
                )
        ) {
            return Some(Box::new(MarginalText {
                master: self.master.clone(),
                place: self.place,
                top: false,
            }));
        }
        if !matches!((name.ns, name.local.as_str()), (Ns::Text, "p" | "h")) {
            return None;
        }
        b.marginal = Some(crate::marginal::Paragraph {
            style: attrs.get(Ns::Text, "style-name").map(str::to_owned),
            parts: Vec::new(),
        });
        Some(Box::new(MarginalInline { closes: true }))
    }

    fn end(&mut self, b: &mut Builder) {
        if !self.top {
            return;
        }
        let paragraphs = std::mem::take(&mut b.marginal_done);
        let lines = std::mem::take(&mut b.marginal_lines);
        let marginals = b.master_marginals.entry(self.master.clone()).or_default();
        marginals.lines.extend(lines);
        let slot = match self.place {
            Place::Header => &mut marginals.header,
            Place::Footer => &mut marginals.footer,
            Place::HeaderFirst => &mut marginals.header_first,
            Place::FooterFirst => &mut marginals.footer_first,
        };
        *slot = Some(paragraphs);
    }
}

/// The inside of a header or footer paragraph, and of every span in it.
struct MarginalInline {
    /// Whether this is the paragraph itself rather than a span inside it.
    closes: bool,
}

impl Context<Builder> for MarginalInline {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        use crate::marginal::Part;
        let paragraph = b.marginal.as_mut()?;
        match (name.ns, name.local.as_str()) {
            (Ns::Text, "span" | "a") => return Some(Box::new(MarginalInline { closes: false })),
            // A line on the page — a fold mark, a rule — is the master's, its ends `svg:` lengths
            // from the corner.
            (Ns::Draw, "line") => {
                let at = |local: &str| {
                    attrs
                        .get(Ns::Svg, local)
                        .and_then(grind_core::style::length_mm)
                };
                if let (Some(x1), Some(y1), Some(x2), Some(y2)) =
                    (at("x1"), at("y1"), at("x2"), at("y2"))
                    && b.marginal_lines.len() < MAX_STYLES
                {
                    let (width, color) = attrs
                        .get(Ns::Draw, "style-name")
                        .and_then(|name| b.graphic_strokes.get(name).cloned())
                        .unwrap_or((0.18, "#000000".to_owned()));
                    b.marginal_lines.push(crate::marginal::Line {
                        from: (x1, y1),
                        to: (x2, y2),
                        width,
                        color,
                    });
                }
            }
            (Ns::Text, "page-number") => paragraph.parts.push(Part::PageNumber),
            (Ns::Text, "page-count") => paragraph.parts.push(Part::PageCount),
            (Ns::Text, "tab") => paragraph.push_text("\t"),
            (Ns::Text, "line-break") => paragraph.push_text("\n"),
            (Ns::Text, "s") => {
                let count = attrs.count(Ns::Text, "c", 1000) as usize;
                paragraph.push_text(&" ".repeat(count));
            }
            // Any other field — file name, date, chapter — shows its cached text. A note's body is
            // not the header's text, and nothing outside `text:` is text.
            (Ns::Text, local) if !matches!(local, "note" | "ruby-text") => {
                return Some(Box::new(MarginalInline { closes: false }));
            }
            _ => {}
        }
        // The page's number and count are the page's own, so their cached text is dropped.
        None
    }

    fn text(&mut self, text: &str, b: &mut Builder) {
        if let Some(paragraph) = b.marginal.as_mut() {
            paragraph.push_text(text);
        }
    }
    fn end(&mut self, b: &mut Builder) {
        if self.closes
            && let Some(paragraph) = b.marginal.take()
        {
            b.marginal_done.push(paragraph);
        }
    }
}

/// A `graphic` style: only its stroke, which is what a `draw:line` in a header is drawn with.
struct GraphicStyleDef {
    name: String,
}

impl Context<Builder> for GraphicStyleDef {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "graphic-properties") && b.graphic_strokes.len() < MAX_STYLES {
            let width = attrs
                .get(Ns::Svg, "stroke-width")
                .and_then(grind_core::style::length_mm)
                .unwrap_or(0.18);
            let color = attrs
                .get(Ns::Svg, "stroke-color")
                .unwrap_or("#000000")
                .to_owned();
            b.graphic_strokes.insert(self.name.clone(), (width, color));
        }
        None
    }
}

/// A `table-cell`, `table-column` or `table-row` style: a cell's fill, borders, padding and
/// vertical alignment, a column's width, or whether a row keeps together (`crate::table_look`).
struct TableStyleDef {
    name: String,
    family: TableFamily,
    automatic: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TableFamily {
    Cell,
    Column,
    Row,
}

impl Context<Builder> for TableStyleDef {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let mm = |local: &str| {
            attrs
                .get(Ns::Fo, local)
                .and_then(grind_core::style::length_mm)
        };
        if self.family == TableFamily::Row && name.is(Ns::Style, "table-row-properties") {
            let style = RowStyle {
                kept: attrs.get(Ns::Fo, "keep-together") == Some("always"),
                height: attrs
                    .get(Ns::Style, "min-row-height")
                    .or_else(|| attrs.get(Ns::Style, "row-height"))
                    .and_then(grind_core::style::length_mm),
            };
            if (style.kept || style.height.is_some()) && b.row_styles.len() < MAX_STYLES {
                b.row_styles
                    .insert(self.name.clone(), (self.automatic, style));
            }
        } else if self.family == TableFamily::Column
            && name.is(Ns::Style, "table-column-properties")
        {
            let width = attrs
                .get(Ns::Style, "column-width")
                .and_then(grind_core::style::length_mm);
            if b.column_styles.len() < MAX_STYLES {
                b.column_styles
                    .insert(self.name.clone(), (self.automatic, width));
            }
        } else if self.family == TableFamily::Cell && name.is(Ns::Style, "table-cell-properties") {
            let all = attrs.get(Ns::Fo, "border").map(str::to_owned);
            let side = |local: &str| {
                attrs
                    .get(Ns::Fo, local)
                    .map(str::to_owned)
                    .or_else(|| all.clone())
            };
            let pad = mm("padding").unwrap_or(0.0);
            let padding = |local: &str| mm(local).unwrap_or(pad);
            // `"0.002cm 0.088cm 0.002cm"`: the inner line, the gap, the outer line.
            let widths = |local: &str| {
                let spelled = attrs
                    .get(Ns::Style, local)
                    .or_else(|| attrs.get(Ns::Style, "border-line-width"))?;
                let mut parts = spelled.split_whitespace().map(grind_core::style::length_mm);
                let three = [parts.next()??, parts.next()??, parts.next()??];
                parts.next().is_none().then_some(three)
            };
            let look = crate::table_look::CellLook {
                background: attrs
                    .get(Ns::Fo, "background-color")
                    .filter(|c| *c != "transparent")
                    .map(str::to_owned),
                border: [
                    side("border-top"),
                    side("border-right"),
                    side("border-bottom"),
                    side("border-left"),
                ],
                line_widths: [
                    widths("border-line-width-top"),
                    widths("border-line-width-right"),
                    widths("border-line-width-bottom"),
                    widths("border-line-width-left"),
                ],
                padding: [
                    padding("padding-top"),
                    padding("padding-right"),
                    padding("padding-bottom"),
                    padding("padding-left"),
                ],
                vertical_align: attrs.get(Ns::Style, "vertical-align").map(str::to_owned),
            };
            if b.cell_styles.len() < MAX_STYLES {
                b.cell_styles
                    .insert(self.name.clone(), (self.automatic, look));
            }
        }
        None
    }
}

/// Where a paragraph style's properties go when it closes.
enum ParagraphTarget {
    /// `style:default-style style:family="paragraph"`, the root of every chain.
    Default,
    Named {
        name: String,
        parent: Option<String>,
        automatic: bool,
    },
}

/// A paragraph style — named, automatic or the default — read for the properties a page needs
/// (`crate::paragraph`).
struct ParagraphStyleDef {
    target: ParagraphTarget,
    props: crate::paragraph::ParagraphProps,
}

impl Context<Builder> for ParagraphStyleDef {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let get = |local: &str| attrs.get(Ns::Fo, local).map(str::to_owned);
        let props = &mut self.props;
        if name.is(Ns::Style, "paragraph-properties") {
            props.margin_top = get("margin-top").or_else(|| get("margin"));
            props.margin_bottom = get("margin-bottom").or_else(|| get("margin"));
            props.margin_left = get("margin-left").or_else(|| get("margin"));
            props.margin_right = get("margin-right").or_else(|| get("margin"));
            props.text_indent = get("text-indent");
            props.line_height = get("line-height");
            props.text_align = get("text-align");
            props.break_before = get("break-before");
            props.break_after = get("break-after");
            props.keep_with_next = get("keep-with-next");
            props.widows = get("widows");
            props.orphans = get("orphans");
            props.tab_stop_distance = attrs.get(Ns::Style, "tab-stop-distance").map(str::to_owned);
            props.page_number = attrs.get(Ns::Style, "page-number").map(str::to_owned);
            return Some(Box::new(TabStopsDef));
        } else if name.is(Ns::Style, "text-properties") {
            // The family as a run's is read: the font-face reference resolved, the XSL-FO quoting
            // removed.
            props.font_family = attrs
                .get(Ns::Style, "font-name")
                .and_then(|face| b.fonts.get(face).cloned())
                .or_else(|| get("font-family").map(|family| style::unquote_family(&family)));
            props.font_size = get("font-size");
            props.font_weight = get("font-weight");
            props.font_style = get("font-style");
            props.color = get("color");
            props.language = get("language");
            props.country = get("country");
        }
        None
    }

    fn end(&mut self, b: &mut Builder) {
        let mut props = std::mem::take(&mut self.props);
        props.tab_stops = b.tab_stops.take();
        match &self.target {
            ParagraphTarget::Default => *b.doc.default_paragraph = props,
            ParagraphTarget::Named {
                name,
                parent,
                automatic,
            } => {
                if b.paragraph_styles.len() < MAX_STYLES {
                    let style = crate::paragraph::ParagraphStyle {
                        parent: parent.clone(),
                        props,
                        automatic: *automatic,
                    };
                    b.paragraph_styles.insert(name.clone(), (*automatic, style));
                }
            }
        }
    }
}

/// A paragraph style's `style:paragraph-properties`, for `style:tab-stops`, handed over through
/// [`Builder::tab_stops`].
struct TabStopsDef;

impl Context<Builder> for TabStopsDef {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "tab-stops") {
            b.tab_stops = Some(Vec::new());
            return Some(Box::new(TabStopsDef));
        }
        if name.is(Ns::Style, "tab-stop")
            && let (Some(stops), Some(position)) =
                (b.tab_stops.as_mut(), attrs.get(Ns::Style, "position"))
            && stops.len() < MAX_STYLES
        {
            // The leader character its text states, else its style's; a space or `none` is no
            // leader.
            let leader = attrs
                .get(Ns::Style, "leader-text")
                .filter(|t| !t.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| match attrs.get(Ns::Style, "leader-style") {
                    Some("dotted") => Some(".".to_owned()),
                    Some("dash" | "long-dash") => Some("-".to_owned()),
                    Some("solid") => Some("_".to_owned()),
                    _ => None,
                });
            stops.push(crate::paragraph::TabStop {
                position: position.to_owned(),
                kind: attrs.get(Ns::Style, "type").map(str::to_owned),
                char: attrs.get(Ns::Style, "char").map(str::to_owned),
                leader,
            });
        }
        None
    }
}

/// `style:header-style` or `style:footer-style`: the room a page layout gives a header or footer.
struct MarginalRoom {
    layout: String,
    header: bool,
}

impl Context<Builder> for MarginalRoom {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "header-footer-properties") {
            let length = |local: &str| {
                attrs
                    .get(Ns::Fo, local)
                    .and_then(grind_core::style::length_mm)
                    .filter(|mm| *mm >= 0.0)
                    .unwrap_or(0.0)
            };
            let spacing = length(if self.header {
                "margin-bottom"
            } else {
                "margin-top"
            });
            let room = (length("min-height"), spacing);
            let entry = b.layout_marginals.entry(self.layout.clone()).or_default();
            match self.header {
                true => entry.0 = Some(room),
                false => entry.1 = Some(room),
            }
        }
        None
    }
}

/// `style:page-layout` (rng:12213): its `style:page-layout-properties` (rng:12248) are the
/// page's size and margins, read by [`grind_core::page::PageGeometry::from_properties`].
struct PageLayout {
    name: String,
}

impl Context<Builder> for PageLayout {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let header = match (name.ns, name.local.as_str()) {
            (Ns::Style, "header-style") => Some(true),
            (Ns::Style, "footer-style") => Some(false),
            _ => None,
        };
        if let Some(header) = header {
            return Some(Box::new(MarginalRoom {
                layout: self.name.clone(),
                header,
            }));
        }
        if name.is(Ns::Style, "page-layout-properties") && b.page_layouts.len() < MAX_STYLES {
            let page =
                grind_core::page::PageGeometry::from_properties(|local| attrs.get(Ns::Fo, local));
            b.page_layouts.insert(self.name.clone(), page);
        }
        None
    }
}

/// `office:automatic-styles` or `office:styles` — the `style:style` declarations: text styles for
/// runs, paragraph styles ([`ParagraphStyleDef`]), and table cell and column styles
/// ([`TableStyleDef`]). Every name, whatever the family, goes into [`Document::styles`] for
/// `undeclared-style` (`doc/dsl.md` §4.3).
struct Styles {
    automatic: bool,
}

impl Context<Builder> for Styles {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "page-layout") {
            let name = attrs.get(Ns::Style, "name")?.to_owned();
            return Some(Box::new(PageLayout { name }));
        }
        if name.is(Ns::Style, "default-style")
            && attrs.get(Ns::Style, "family") == Some("paragraph")
        {
            return Some(Box::new(ParagraphStyleDef {
                target: ParagraphTarget::Default,
                props: crate::paragraph::ParagraphProps::default(),
            }));
        }
        if name.is(Ns::Text, "list-style") {
            let declared = attrs.get(Ns::Style, "name")?.to_owned();
            b.list_levels.clear();
            b.list_indents.clear();
            return Some(Box::new(ListStyleDef {
                name: declared,
                automatic: self.automatic,
                outline: false,
            }));
        }
        if name.is(Ns::Text, "outline-style") {
            b.list_levels.clear();
            b.list_indents.clear();
            return Some(Box::new(ListStyleDef {
                name: String::new(),
                automatic: false,
                outline: true,
            }));
        }
        if !name.is(Ns::Style, "style") {
            return None;
        }
        if let Some(declared) = attrs.get(Ns::Style, "name")
            && b.doc.styles.len() < MAX_STYLES
        {
            b.doc.styles.insert(declared.to_owned());
        }
        if attrs.get(Ns::Style, "family") == Some("graphic") {
            let declared = attrs.get(Ns::Style, "name")?.to_owned();
            return Some(Box::new(GraphicStyleDef { name: declared }));
        }
        let family = match attrs.get(Ns::Style, "family") {
            Some("table-cell") => Some(TableFamily::Cell),
            Some("table-column") => Some(TableFamily::Column),
            Some("table-row") => Some(TableFamily::Row),
            _ => None,
        };
        if let Some(family) = family {
            let declared = attrs.get(Ns::Style, "name")?.to_owned();
            return Some(Box::new(TableStyleDef {
                name: declared,
                family,
                automatic: self.automatic,
            }));
        }
        if attrs.get(Ns::Style, "family") == Some("paragraph") {
            let declared = attrs.get(Ns::Style, "name")?.to_owned();
            return Some(Box::new(ParagraphStyleDef {
                target: ParagraphTarget::Named {
                    name: declared,
                    parent: attrs.get(Ns::Style, "parent-style-name").map(str::to_owned),
                    automatic: self.automatic,
                },
                props: crate::paragraph::ParagraphProps {
                    master_page: attrs
                        .get(Ns::Style, "master-page-name")
                        .filter(|m| !m.is_empty())
                        .map(str::to_owned),
                    ..Default::default()
                },
            }));
        }
        if attrs.get(Ns::Style, "family") != Some("text") {
            return None;
        }
        Some(Box::new(TextStyleDef {
            automatic: self.automatic,
            name: attrs.get(Ns::Style, "name").map(str::to_owned),
            parent: attrs.get(Ns::Style, "parent-style-name").map(str::to_owned),
            props: CharStyle::default(),
            extras: false,
        }))
    }
}

/// One `text:list-style` (rng:17558): what each level's items wear (`crate::numbering`) — or, with
/// `outline`, the `text:outline-style` (rng:17530), whose levels are numbers.
struct ListStyleDef {
    name: String,
    automatic: bool,
    outline: bool,
}

impl Context<Builder> for ListStyleDef {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        use crate::numbering::Level;
        let level = attrs
            .get(Ns::Text, "level")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|l| (1..=10).contains(l))?;
        let style = match (name.ns, name.local.as_str()) {
            (Ns::Text, "list-level-style-bullet") => Level::Bullet(
                attrs
                    .get(Ns::Text, "bullet-char")
                    .unwrap_or_default()
                    .to_owned(),
            ),
            (Ns::Text, "list-level-style-image") => Level::Bullet(String::new()),
            (Ns::Text, "list-level-style-number" | "outline-level-style") => {
                let get = |ns, local| attrs.get(ns, local).unwrap_or_default().to_owned();
                Level::Number {
                    format: get(Ns::Style, "num-format"),
                    prefix: get(Ns::Style, "num-prefix"),
                    suffix: get(Ns::Style, "num-suffix"),
                    display: attrs
                        .get(Ns::Text, "display-levels")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1),
                    start: attrs
                        .get(Ns::Text, "start-value")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1),
                    template: attrs.get(Ns::Loext, "num-list-format").map(str::to_owned),
                }
            }
            _ => return None,
        };
        b.list_levels.insert(level, style);
        Some(Box::new(LevelProps { level }))
    }

    fn end(&mut self, b: &mut Builder) {
        let style = crate::numbering::ListStyle {
            levels: std::mem::take(&mut b.list_levels),
            indents: std::mem::take(&mut b.list_indents),
        };
        if self.outline {
            b.doc.outline_style = Some(style);
        } else if b.list_styles.len() < MAX_STYLES {
            b.list_styles
                .insert(self.name.clone(), (self.automatic, style));
        }
    }
}

/// Inside one list level: `style:list-level-properties` and its
/// `style:list-level-label-alignment`, which places the label and the text (rng:13281).
struct LevelProps {
    level: u32,
}

impl Context<Builder> for LevelProps {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "list-level-properties") {
            return Some(Box::new(LevelProps { level: self.level }));
        }
        if name.is(Ns::Style, "list-level-label-alignment") {
            let get = |ns, local| attrs.get(ns, local).map(str::to_owned);
            b.list_indents.insert(
                self.level,
                crate::numbering::LevelIndent {
                    margin_left: get(Ns::Fo, "margin-left"),
                    text_indent: get(Ns::Fo, "text-indent"),
                    tab: get(Ns::Text, "list-tab-stop-position"),
                    followed_by: get(Ns::Text, "label-followed-by"),
                },
            );
        }
        None
    }
}

/// One `style:style style:family="text"`, gathering its `style:text-properties`.
struct TextStyleDef {
    automatic: bool,
    name: Option<String>,
    parent: Option<String>,
    props: CharStyle,
    /// Whether the style says anything [`CharStyle`] does not read.
    extras: bool,
}

impl Context<Builder> for TextStyleDef {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Style, "text-properties") {
            // A paragraph's or a graphic's properties on a character style: nothing a run reads.
            self.extras = true;
            return None;
        }
        // `style:font-name` is LibreOffice's spelling and `fo:font-family` the schema's; either
        // gives the family.
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
            origin: Default::default(),
        };
        // Anything else on the element (superscript, a language, letter spacing) is unread, and
        // makes the style one a restyle must build from.
        const READ: [(Ns, &str); 9] = [
            (Ns::Fo, "font-family"),
            (Ns::Style, "font-name"),
            (Ns::Fo, "font-size"),
            (Ns::Fo, "font-weight"),
            (Ns::Fo, "font-style"),
            (Ns::Style, "text-underline-style"),
            (Ns::Style, "text-line-through-style"),
            (Ns::Fo, "color"),
            (Ns::Fo, "background-color"),
        ];
        if attrs
            .names()
            .any(|n| !READ.iter().any(|(ns, local)| n.is(*ns, local)))
        {
            self.extras = true;
        }
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
                extras: self.extras,
            },
        );
    }
}

struct Body;

impl Context<Builder> for Body {
    fn start_child(&mut self, name: &Name, _a: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        // `office:text`, not `text:text`: the body is in the office namespace (rng:7693).
        name.is(Ns::Office, "text").then(|| Box::new(Text) as Ctx)
    }
}

/// `office:text` — the body, a flat sequence (`doc/odt-format.md` §2); it and [`ListItem`]
/// dispatch the same blocks.
struct Text;

impl Context<Builder> for Text {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let child = block_child(name, attrs, b);
        // Something the model has no block for — `text:sequence-decls`, a section, a table of
        // contents — kept with the block it followed, so a regenerated body puts it back.
        if child.is_none() {
            b.record_sibling(attrs.span());
        }
        child
    }
}

/// The blocks that may appear in a body or a list item (`text-list-item-content` admits what
/// `office-text-content-main` does). Anything outside `doc/text-core.md`'s scope returns `None`
/// and is swallowed.
fn block_child(name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
    let style = attrs.get(Ns::Text, "style-name").map(str::to_owned);
    match (name.ns, name.local.as_str()) {
        (Ns::Text, "p") => {
            let kind = match b.list_depth {
                0 => BlockKind::Paragraph,
                depth => BlockKind::ListItem { depth },
            };
            let id = b.open(kind, style);
            // An index's entry has no place a splice could write it back to; the index element is
            // kept whole (`Builder::generating`).
            if !b.generating {
                b.record(id, attrs.span());
            }
            Some(Box::new(Paragraph))
        }
        (Ns::Text, "h") => {
            // `text:outline-level` is required and unbounded (rng:6867); missing or unparseable
            // means 1.
            let level = attrs
                .get(Ns::Text, "outline-level")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .filter(|n| *n > 0)
                .unwrap_or(1);
            let id = b.open(BlockKind::Heading { level }, style);
            // What the outline's numbering needs beyond the level: whether it is left out, and a
            // number of its own (`crate::numbering`).
            let header = attrs.get(Ns::Text, "is-list-header") == Some("true");
            let start = attrs
                .get(Ns::Text, "start-value")
                .and_then(|v| v.trim().parse().ok());
            if let Some(mark) = b.doc.list_marks.get_mut(&id) {
                mark.header |= header;
                mark.start = start.or(mark.start);
            } else if header || start.is_some() {
                b.doc.list_marks.insert(
                    id,
                    crate::numbering::ListMark {
                        style: None,
                        chain: usize::MAX,
                        start,
                        header,
                        first: true,
                        in_list: None,
                    },
                );
            }
            // An index's entry has no place a splice could write it back to; the index element is
            // kept whole (`Builder::generating`).
            if !b.generating {
                b.record(id, attrs.span());
            }
            Some(Box::new(Paragraph))
        }
        (Ns::Text, "list") => {
            let before = b.open_lists.len();
            // The style in effect is the list's own or its enclosing list's; a top-level list
            // starts its own numbering unless it says it continues one.
            let frames = b.list_frames.len();
            let style = attrs
                .get(Ns::Text, "style-name")
                .map(str::to_owned)
                .or_else(|| b.list_frames.last().and_then(|(style, _)| style.clone()));
            let chain = match b.list_frames.last() {
                Some((_, chain)) => *chain,
                None => {
                    let continued = match (
                        attrs.get(Ns::Text, "continue-list"),
                        attrs.get(Ns::Text, "continue-numbering"),
                    ) {
                        (Some(id), _) => b.chain_of_id.get(id).copied(),
                        (None, Some("true")) => b.chain_of_style.get(&style).copied(),
                        _ => None,
                    };
                    let chain = continued.unwrap_or_else(|| {
                        b.chains += 1;
                        b.chains
                    });
                    b.chain_of_style.insert(style.clone(), chain);
                    chain
                }
            };
            if let Some(id) = attrs.get(Ns::Xml, "id") {
                b.chain_of_id.insert(id.to_owned(), chain);
            }
            b.list_frames.push((style, chain));
            if let Some(tag) = b
                .doc
                .source
                .as_deref()
                .and_then(|source| source.bytes.get(attrs.span()))
            {
                let kept = super::source::attributes(tag, &[]);
                b.open_lists.push((attrs.span().start, kept));
            }
            Some(Box::new(List::new(before, frames)))
        }
        // `table:table` is one of `text-content`'s alternatives (rng:16938): in a text document a
        // table is a block, and its cells hold blocks.
        (Ns::Table, "table") => Some(Box::new(Table::open(
            attrs.get(Ns::Table, "name"),
            attrs.span(),
            b,
        ))),
        // A section (rng:16697): its blocks are read in place, shown and spliced inside it. A
        // regenerate that would drop the wrapper is refused by the save's loss check.
        (Ns::Text, "section") => {
            let start = b
                .doc
                .source
                .as_deref()
                .and_then(|source| source.bytes.get(attrs.span()))
                .and_then(|tag| std::str::from_utf8(tag).ok())
                .filter(|tag| !tag.trim_end().ends_with("/>"))
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    let name = attrs.get(Ns::Text, "name").unwrap_or("Section");
                    format!(
                        "<text:section text:name=\"{}\">",
                        name.replace('"', "&quot;")
                    )
                });
            let parent = b.section;
            let range = b
                .doc
                .source
                .as_deref()
                .and_then(|source| element_extent(&source.bytes, attrs.span()));
            b.section = Some(b.doc.sections.len());
            b.doc.sections.push(crate::model::Section {
                start,
                parent,
                range,
                blocks: Vec::new(),
            });
            Some(Box::new(SectionText { parent }))
        }
        // An index (rng:17154 on): the element is kept whole, and the entries its
        // `text:index-body` last generated are read as generated blocks — shown, never written as
        // blocks.
        (Ns::Text, local) if INDEXES.contains(&local) => {
            b.record_sibling(attrs.span());
            Some(Box::new(Index))
        }
        _ => None,
    }
}

/// A section's content: the body's blocks, with the section restored as the innermost when it
/// closes.
struct SectionText {
    parent: Option<usize>,
}

impl Context<Builder> for SectionText {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        Text.start_child(name, attrs, b)
    }

    fn end(&mut self, b: &mut Builder) {
        b.section = self.parent;
    }
}

/// The index elements whose generated entries are shown.
const INDEXES: &[&str] = &[
    "table-of-content",
    "illustration-index",
    "alphabetical-index",
    "table-index",
    "object-index",
    "user-index",
    "bibliography",
];

/// An index: its source (how it is generated) is skipped; its body is what was generated.
struct Index;

impl Context<Builder> for Index {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Text, "index-body" | "index-title") => Some(Box::new(Index)),
            (Ns::Text, "p" | "h") => {
                b.generating = true;
                let child = block_child(name, attrs, b);
                Some(Box::new(Generated(child)))
            }
            _ => None,
        }
    }
}

/// One generated entry, read as any paragraph is, the flag lowered when it closes.
struct Generated(Option<Ctx>);

impl Context<Builder> for Generated {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        self.0.as_mut()?.start_child(name, attrs, b)
    }

    fn text(&mut self, text: &str, b: &mut Builder) {
        if let Some(inner) = self.0.as_mut() {
            inner.text(text, b);
        }
    }

    fn end(&mut self, b: &mut Builder) {
        if let Some(inner) = self.0.as_mut() {
            inner.end(b);
        }
        b.generating = false;
    }
}

/// `table:table` (rng:15939) in a text document: its rows and cells give the blocks their
/// coordinates, and the extent is derived from them. Its column declarations and styles are
/// recorded for its look ([`crate::table_look`]) and for writing it back
/// ([`super::source::TableSource`]). The name and row counter live on [`Builder::tables`], since
/// rows arrive through three containers (`table:table-header-rows` and the two groups).
struct Table {
    /// Whether this context is the `table:table` itself rather than a row container inside it —
    /// whether its `end` closes the table.
    owns: bool,
    /// Whether its rows are the table's heading (`table:table-header-rows`).
    header: bool,
}

impl Table {
    fn open(name: Option<&str>, span: std::ops::Range<usize>, b: &mut Builder) -> Self {
        // `table:name` is optional (rng:15970), but the model tells tables apart by name, so a
        // unique one is generated.
        let name = match name.map(str::trim).filter(|name| !name.is_empty()) {
            Some(name) => name.to_owned(),
            None => {
                b.next_table += 1;
                format!("Table{}", b.next_table)
            }
        };
        let recorded = b.cell.is_none() && b.doc.source.is_some() && !span.is_empty();
        let mut columns_from = None;
        if recorded
            && let Some(source) = b.doc.source.as_deref_mut()
            && let Some(tag) = source
                .bytes
                .get(span.clone())
                .and_then(|tag| std::str::from_utf8(tag).ok())
                .filter(|tag| !tag.trim_end().ends_with("/>"))
        {
            let range = element_extent(&source.bytes, span.clone());
            columns_from = Some(span.end);
            source.tables.insert(
                name.clone(),
                super::source::TableSource {
                    range,
                    start: tag.to_owned(),
                    ..Default::default()
                },
            );
        }
        b.tables.push(OpenTable {
            name,
            row: 0,
            recorded: columns_from.is_some(),
            columns_from,
            before: b.doc.blocks.len(),
        });
        Table {
            owns: true,
            header: false,
        }
    }

    /// A container of rows that is not the table — a header-row group or a grouping element —
    /// sharing the table's name and row counter.
    fn group(header: bool) -> Self {
        Table {
            owns: false,
            header,
        }
    }
}

impl Context<Builder> for Table {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            // The three row containers that are not the table, read as one: a header row's cells
            // are ordinary cells.
            (Ns::Table, "table-header-rows") => {
                b.columns_end(attrs.span().start);
                Some(Box::new(Table::group(true)))
            }
            (
                Ns::Table,
                "table-row-group"
                | "table-rows"
                | "table-columns"
                | "table-header-columns"
                | "table-column-group",
            ) => Some(Box::new(Table::group(self.header))),
            (Ns::Table, "table-column") => {
                let repeated = attrs.count(Ns::Table, "number-columns-repeated", MAX_TABLE_SPAN);
                let style = attrs.get(Ns::Table, "style-name").map(str::to_owned);
                if let Some(table) = b.tables.last() {
                    let pending = b.table_pending.entry(table.name.clone()).or_default();
                    for _ in 0..repeated.max(1) {
                        pending.columns.push(style.clone());
                    }
                }
                None
            }
            (Ns::Table, "table-row") => {
                b.columns_end(attrs.span().start);
                let repeated = attrs.count(Ns::Table, "number-rows-repeated", MAX_TABLE_SPAN);
                let row = Row::open(repeated, b);
                let header = self.header;
                b.record_table(
                    |table, tag| {
                        table.rows.insert(row.row, tag.to_owned());
                        if header {
                            table.header_rows.extend(row.row..row.row + row.repeated);
                        }
                    },
                    attrs.span(),
                );
                let pending = b.table_pending.entry(row.table.clone()).or_default();
                pending.rows = pending.rows.max(row.row + row.repeated);
                if let Some(style) = attrs.get(Ns::Table, "style-name") {
                    pending
                        .row_styles
                        .push((row.row, row.repeated, style.to_owned()));
                }
                if self.header {
                    pending.header_rows.extend(row.row..row.row + row.repeated);
                }
                Some(Box::new(row))
            }
            _ => None,
        }
    }

    fn end(&mut self, b: &mut Builder) {
        // One pop per push, and only `Table::open` pushes: a row container's `end` runs while its
        // table is still open.
        if self.owns
            && let Some(table) = b.tables.pop()
            && table.recorded
        {
            let held: Vec<BlockId> = b.doc.blocks[table.before.min(b.doc.blocks.len())..]
                .iter()
                .filter(|block| block.cell.as_ref().is_some_and(|c| c.table == table.name))
                .map(|block| block.id)
                .collect();
            if let Some(source) = b.doc.source.as_deref_mut()
                && let Some(recorded) = source.tables.get_mut(&table.name)
            {
                recorded.blocks = held;
            }
        }
    }
}

/// The largest repeat or span honoured: `table:number-columns-repeated` is an unbounded
/// `positiveInteger`, and a million columns is a memory-exhaustion vector.
const MAX_TABLE_SPAN: u32 = 1024;

/// `table:table-row` (rng:16223) — the cells of one row, numbered as they arrive.
struct Row {
    table: String,
    row: u32,
    column: u32,
    /// `table:number-rows-repeated`: how many identical rows this element stands for.
    repeated: u32,
    /// How many blocks the document had when this row opened, so the repeats can be copies.
    before: usize,
}

impl Row {
    /// Take the next row number from the table this row is in.
    fn open(repeated: u32, b: &mut Builder) -> Self {
        let (table, row) = match b.tables.last_mut() {
            Some(table) => {
                let row = table.row;
                table.row += repeated.max(1);
                (table.name.clone(), row)
            }
            None => (String::new(), 0),
        };
        Row {
            table,
            row,
            column: 0,
            repeated: repeated.max(1),
            before: b.doc.blocks.len(),
        }
    }
}

impl Context<Builder> for Row {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let covered = match (name.ns, name.local.as_str()) {
            (Ns::Table, "table-cell") => false,
            // A position covered by a span above or to the left (rng:14298): no content, it only
            // moves the column on.
            (Ns::Table, "covered-table-cell") => true,
            _ => return None,
        };
        let repeated = attrs
            .count(Ns::Table, "number-columns-repeated", MAX_TABLE_SPAN)
            .max(1);
        let column = self.column;
        self.column += repeated;
        let row = self.row;
        b.record_table(
            |table, tag| {
                for offset in 0..repeated {
                    table.cells.insert((row, column + offset), tag.to_owned());
                }
            },
            attrs.span(),
        );
        if covered {
            return None;
        }
        if let Some(style) = attrs.get(Ns::Table, "style-name") {
            let pending = b.table_pending.entry(self.table.clone()).or_default();
            for offset in 0..repeated {
                pending
                    .cells
                    .push(((self.row, column + offset), style.to_owned()));
            }
        }
        let cell = crate::model::Cell {
            table: self.table.clone(),
            row: self.row,
            column,
            columns_spanned: attrs
                .count(Ns::Table, "number-columns-spanned", MAX_TABLE_SPAN)
                .max(1),
            rows_spanned: attrs
                .count(Ns::Table, "number-rows-spanned", MAX_TABLE_SPAN)
                .max(1),
        };
        Some(Box::new(TableCell::open(cell, repeated, b)))
    }

    fn end(&mut self, b: &mut Builder) {
        // `table:number-rows-repeated` expanded, as `text:s` is (`doc/odt-format.md` §3.3),
        // bounded by `MAX_TABLE_SPAN`.
        repeat_blocks(b, self.before, self.repeated, |cell, copy| cell.row += copy);
    }
}

/// Copy the blocks added since `before` once per extra repeat, each copy's cell moved by `shift` —
/// for row and column repeats alike.
fn repeat_blocks(
    b: &mut Builder,
    before: usize,
    repeated: u32,
    shift: impl Fn(&mut crate::model::Cell, u32),
) {
    if repeated <= 1 || b.doc.blocks.len() <= before {
        return;
    }
    let original: Vec<Block> = b.doc.blocks[before..].to_vec();
    for copy in 1..repeated {
        for block in &original {
            let id = b.doc.next_id();
            let mut clone = block.clone();
            clone.id = id;
            if let Some(cell) = clone.cell.as_mut() {
                shift(cell, copy);
            }
            b.doc.blocks.push(clone);
        }
    }
}

/// `table:table-cell` (rng:16052), whose content is `zeroOrMore text-content` (rng:16126), the
/// body's own production, so a cell may hold a heading or a list.
struct TableCell {
    /// How many blocks the document had before this cell opened, to tell an empty cell.
    before: usize,
    /// The cell this one is nested inside, restored on the way out.
    outer: Option<crate::model::Cell>,
    /// `table:number-columns-repeated`, expanded on the way out the way a row's is.
    repeated: u32,
    /// Whether this cell is the one blocks are attributed to. False in a nested table: the model
    /// has one coordinate per block, so the inner table's paragraphs belong to the outer cell —
    /// its text kept, its structure not, since a nested table would split the outer one's block
    /// run.
    owns: bool,
}

impl TableCell {
    fn open(cell: crate::model::Cell, repeated: u32, b: &mut Builder) -> Self {
        let owns = b.cell.is_none();
        let outer = match owns {
            true => b.cell.replace(cell),
            false => b.cell.clone(),
        };
        TableCell {
            before: b.doc.blocks.len(),
            outer,
            repeated: repeated.max(1),
            owns,
        }
    }
}

impl Context<Builder> for TableCell {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        block_child(name, attrs, b)
    }

    fn end(&mut self, b: &mut Builder) {
        // An empty cell gets a paragraph: LibreOffice writes one into `<table:table-cell/>` on
        // every save (`doc/odt-format.md` §5b), so reading stays idempotent across loop C, and a
        // caret has somewhere to go.
        if self.owns && b.doc.blocks.len() == self.before {
            b.open(BlockKind::Paragraph, None);
        }
        // The columns this element stands for, expanded, as `Row::end` does rows.
        if self.owns {
            repeat_blocks(b, self.before, self.repeated, |cell, copy| {
                cell.column += copy
            });
        }
        b.cell = self.outer.take();
    }
}

/// `text:list` — nests through its items, never through itself (rng:17494).
struct List {
    /// Whether this element incremented the depth; past [`MAX_LIST_DEPTH`] its items still read,
    /// and it must not decrement on the way out.
    counted: bool,
    /// How many entries [`Builder::open_lists`] had before this one, restored at its `end`.
    open: usize,
    /// And how many [`Builder::list_frames`].
    frames: usize,
}

impl List {
    fn new(open: usize, frames: usize) -> Self {
        List {
            counted: false,
            open,
            frames,
        }
    }
}

impl Context<Builder> for List {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        // Depth goes up when the first item opens, so an empty `text:list` costs nothing.
        match (name.ns, name.local.as_str()) {
            // `text:list-header` holds an item's content without a number; read as an item, with
            // the difference kept for numbering.
            (Ns::Text, local @ ("list-item" | "list-header")) => {
                b.item = Some((
                    attrs
                        .get(Ns::Text, "start-value")
                        .and_then(|v| v.trim().parse().ok()),
                    local == "list-header",
                    true,
                ));
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
        b.open_lists.truncate(self.open);
        b.list_frames.truncate(self.frames);
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

/// `paragraph-content` (rng:8405) — the inline model shared by paragraphs, spans and hyperlinks.
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
            // ODF's run-length encoding of spaces (rng:8408), expanded here and re-encoded on
            // writing; clamped, since four billion spaces is a memory-exhaustion vector.
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
        // A tracked change's marks (rng:9170) and a comment with the end of its range
        // (`office:annotation`, rng:7787): kept whole, showing nothing. The change itself is in
        // `text:tracked-changes`, carried with the body.
        (Ns::Text, "change" | "change-start" | "change-end")
        | (Ns::Office, "annotation" | "annotation-end") => Some(Box::new(FieldText {
            start: attrs.span(),
            text: String::new(),
            silent: true,
        })),
        // A field's text is its cached value (rng:8549 on), so "Figure 1." shows its 1; the element
        // is kept, and an edit that would flatten it is refused at save.
        (Ns::Text, local) if FIELDS.contains(&local) => Some(Box::new(FieldText {
            start: attrs.span(),
            text: String::new(),
            silent: false,
        })),
        // A footnote or endnote: its citation shows where it stands and its paragraphs are read
        // beside the blocks (`crate::marginal::Note`); the element is kept, so an edit that would
        // lose it is refused at save.
        (Ns::Text, "note") => {
            let block = b.doc.blocks.last()?;
            let note = crate::marginal::Note {
                block: block.id,
                offset: block.runs.iter().map(Run::len).sum(),
                citation: String::new(),
                endnote: attrs.get(Ns::Text, "note-class") == Some("endnote"),
                paragraphs: Vec::new(),
            };
            b.marginal_done.clear();
            Some(Box::new(NoteText {
                note: Some(note),
                start: attrs.span(),
            }))
        }
        _ => None,
    }
}

/// The text fields whose cached value is shown (rng:8549–9116): every field carrying one as
/// character content, `text:page-number` and `text:page-count` included.
pub(crate) const FIELDS: &[&str] = &[
    "sequence",
    "sequence-ref",
    "date",
    "time",
    "page-number",
    "page-count",
    "page-continuation",
    "chapter",
    "file-name",
    "template-name",
    "sheet-name",
    "title",
    "subject",
    "keywords",
    "description",
    "initial-creator",
    "creator",
    "creation-date",
    "creation-time",
    "modification-date",
    "modification-time",
    "print-date",
    "print-time",
    "printed-by",
    "editing-cycles",
    "editing-duration",
    "author-name",
    "author-initials",
    "sender-firstname",
    "sender-lastname",
    "sender-initials",
    "sender-title",
    "sender-position",
    "sender-email",
    "sender-phone-private",
    "sender-fax",
    "sender-company",
    "sender-phone-work",
    "sender-street",
    "sender-city",
    "sender-postal-code",
    "sender-country",
    "sender-state-or-province",
    "variable-get",
    "variable-set",
    "variable-input",
    "user-field-get",
    "user-field-input",
    "user-defined",
    "expression",
    "text-input",
    "placeholder",
    "conditional-text",
    "hidden-text",
    "bookmark-ref",
    "reference-ref",
    "note-ref",
    "word-count",
    "character-count",
    "paragraph-count",
    "page-variable-get",
    "database-display",
    "database-name",
    "database-row-number",
    "dde-connection",
    "execute-macro",
    "measure",
    "table-formula",
    "bibliography-mark",
];

/// A `text:note`: its citation into the run, and its body's paragraphs into the note.
struct NoteText {
    note: Option<crate::marginal::Note>,
    /// Where its start tag is, so the whole element can be kept ([`Run::Kept`]).
    start: std::ops::Range<usize>,
}

impl Context<Builder> for NoteText {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Text, "note-citation") => Some(Box::new(Citation)),
            (Ns::Text, "note-body") => Some(Box::new(NoteBody)),
            _ => None,
        }
    }

    fn end(&mut self, b: &mut Builder) {
        if let Some(mut note) = self.note.take() {
            note.citation = std::mem::take(&mut b.citation);
            note.paragraphs = std::mem::take(&mut b.marginal_done);
            b.keep(self.start.clone(), &note.citation);
            b.doc.notes.push(note);
        }
    }
}

/// A note's citation: shown in the paragraph, and kept as the note's mark.
struct Citation;

impl Context<Builder> for Citation {
    fn text(&mut self, text: &str, b: &mut Builder) {
        b.citation.push_str(text);
    }
}

/// A note's body: its paragraphs as text, the way a header's are read.
struct NoteBody;

impl Context<Builder> for NoteBody {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !matches!((name.ns, name.local.as_str()), (Ns::Text, "p" | "h")) {
            return None;
        }
        b.marginal = Some(crate::marginal::Paragraph {
            style: attrs.get(Ns::Text, "style-name").map(str::to_owned),
            parts: Vec::new(),
        });
        Some(Box::new(MarginalInline { closes: true }))
    }
}

/// A field: its cached value shown where it stands, and the element kept whole ([`Run::Kept`]), so
/// an edit beside it writes the field back.
struct FieldText {
    start: std::ops::Range<usize>,
    text: String,
    /// Shows nothing — a comment, whose paragraphs are its own and not the field's value.
    silent: bool,
}

impl Context<Builder> for FieldText {
    fn text(&mut self, text: &str, _b: &mut Builder) {
        if !self.silent {
            self.text.push_str(text);
        }
    }

    fn end(&mut self, b: &mut Builder) {
        b.keep(self.start.clone(), &self.text);
    }
}

/// Open a `draw:frame`, outermost or one LibreOffice nested inside for resizing. The first frame's
/// size is kept unless it gave none, and only the outermost [`Frame::end`] makes a run.
fn open_frame(attrs: &Attrs, b: &mut Builder) -> Ctx {
    let outermost = b.frame_depth == 0;
    let pending = b.image.get_or_insert_with(PendingImage::default);
    if outermost {
        pending.start = Some(attrs.span());
    }
    if pending.width.is_none() {
        pending.width = attrs.get(Ns::Svg, "width").map(str::to_owned);
    }
    if pending.height.is_none() {
        pending.height = attrs.get(Ns::Svg, "height").map(str::to_owned);
    }
    if pending.anchor.is_none() {
        pending.anchor = attrs.get(Ns::Text, "anchor-type").map(str::to_owned);
    }
    b.frame_depth += 1;
    Box::new(Frame)
}

/// `draw:frame` (rng:5089) — a picture; anything else a frame holds (`draw:object`, a shape) is
/// inert.
struct Frame;

impl Context<Builder> for Frame {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Draw, "frame") => Some(open_frame(attrs, b)),
            (Ns::Draw, "image") => {
                let mime = attrs.get(Ns::Draw, "mime-type").map(str::to_owned);
                // `xlink:href` (rng:1621) or inline `office:binary-data` (rng:5383). A package's
                // picture is usually the first, resolved against the archive here, before
                // `b.image` is borrowed.
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
            // The caption's plain text only (`doc/text-core.md`).
            (Ns::Draw, "text-box") => Some(Box::new(TextBoxSearch)),
            (Ns::Svg, "title") => Some(Box::new(AltText::default())),
            (Ns::Svg, "desc") => Some(Box::new(AltText {
                long: true,
                ..AltText::default()
            })),
            _ => None,
        }
    }

    fn end(&mut self, b: &mut Builder) {
        b.frame_depth = b.frame_depth.saturating_sub(1);
        // Only the outermost frame's close makes a run, or an inner one closing first would emit
        // an empty image.
        if b.frame_depth == 0
            && let Some(pending) = b.image.take()
            && let Some(data) = pending.data
            // A frame that states no type (odfpy writes none) is typed by its own bytes.
            && let Some(mime) = pending
                .mime
                .or_else(|| crate::picture::mime(&data).map(str::to_owned))
        {
            let image = Run::Image {
                mime,
                data,
                width: pending.width,
                height: pending.height,
                anchor: pending.anchor,
                title: pending.title,
                description: pending.description,
            };
            b.record_frame(&image, pending.start);
            b.push_run(image);
            // The caption after the image, whatever order its parts arrived in.
            if !pending.caption.is_empty() {
                b.push_run(Run::plain(pending.caption));
            }
        }
    }
}

/// `svg:title` or `svg:desc` on a frame (rng:1756), kept from the first frame that has it.
#[derive(Default)]
struct AltText {
    long: bool,
    text: String,
}

impl Context<Builder> for AltText {
    fn text(&mut self, text: &str, _: &mut Builder) {
        self.text.push_str(text);
    }

    fn end(&mut self, b: &mut Builder) {
        if let Some(pending) = &mut b.image {
            let slot = match self.long {
                true => &mut pending.description,
                false => &mut pending.title,
            };
            if slot.is_none() && !self.text.is_empty() {
                *slot = Some(std::mem::take(&mut self.text));
            }
        }
    }
}

/// `draw:text-box` — a frame's caption: read to find an image LibreOffice nests inside it and to
/// keep the caption's plain text (character data, and a `text:sequence`'s computed value,
/// rng:8655).
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

/// `draw:image` (rng:5380), reached only when [`Frame::start_child`] found no `xlink:href`, so it
/// looks for inline `office:binary-data` (rng:7681).
struct ImageData;

impl Context<Builder> for ImageData {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        name.is(Ns::Office, "binary-data")
            .then(|| Box::new(BinaryData::default()) as Ctx)
    }
}

/// `office:binary-data`: base64 wrapped across lines; whitespace is stripped before decoding.
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

/// `text:a` — a hyperlink, which cannot nest.
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
