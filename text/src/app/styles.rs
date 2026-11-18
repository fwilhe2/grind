// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a renderer reads besides the text: the page, its marginals and notes, and the styles a
//! block resolves to.

use crate::*;

impl App {
    /// The page this document says it is set on, or `None` — in which case printing uses A4
    /// (`doc/pdf-export.md`, decision 7). Read from the master page, never written.
    pub fn page(&self) -> Option<grind_core::page::PageGeometry> {
        self.state.read().unwrap().doc.page
    }

    /// Set the page the document is set on: paper, orientation and margins, written over its
    /// master page's layout and nothing else (`doc/page-model.md` PM8). One undo step.
    pub fn set_page(&self, page: grind_core::page::PageGeometry) -> Result<()> {
        self.mutate(|state| {
            Self::commit(state, crate::action::Action::SetPage { page: Some(page) })
        })
    }

    /// The default paragraph style's widows and orphans ([`Document::default_paragraph`]).
    pub fn paragraph_defaults(&self) -> ParagraphDefaults {
        let state = self.state.read().unwrap();
        let count = |value: &Option<String>| value.as_deref().and_then(|v| v.trim().parse().ok());
        ParagraphDefaults {
            widows: count(&state.doc.default_paragraph.widows),
            orphans: count(&state.doc.default_paragraph.orphans),
        }
    }

    /// The page's header and footer ([`Document::header`], `crate::marginal`), read for printing.
    pub fn marginals(&self) -> (Option<marginal::Marginal>, Option<marginal::Marginal>) {
        let state = self.state.read().unwrap();
        (state.doc.header.clone(), state.doc.footer.clone())
    }

    /// The first page's own header and footer, when it is set on a master page of its own
    /// ([`model::Document::first_page`]); `None` when it wears [`App::marginals`] like the rest.
    #[allow(clippy::type_complexity)]
    pub fn first_page_marginals(
        &self,
    ) -> Option<(Option<marginal::Marginal>, Option<marginal::Marginal>)> {
        self.state.read().unwrap().doc.first_page.clone()
    }

    /// One master page by name — its page, header, footer and next master ([`model::Master`]) —
    /// and the name of the body's master, for a paragraph that starts a page on a master of its
    /// own.
    pub fn master(&self, name: &str) -> Option<model::Master> {
        self.state.read().unwrap().doc.masters.get(name).cloned()
    }

    /// Whether two paragraphs' spaces below and above add, or the larger is kept
    /// ([`model::Document::paragraph_spaces_add`]).
    pub fn paragraph_spaces_add(&self) -> bool {
        self.state.read().unwrap().doc.paragraph_spaces_add
    }

    /// The name of the master the document's ordinary pages are set on, if it has any.
    pub fn rest_master(&self) -> Option<String> {
        self.state.read().unwrap().doc.rest_master.clone()
    }

    /// A paragraph style resolved by name, as [`App::paragraph`] resolves a block's — for a header
    /// paragraph, which is no block.
    pub fn resolve_style(&self, name: Option<&str>) -> paragraph::Resolved {
        let state = self.state.read().unwrap();
        paragraph::Resolved {
            props: paragraph::resolve(
                &state.doc.paragraph_styles,
                &state.doc.default_paragraph,
                name,
            ),
            declared: name.is_some_and(|n| state.doc.paragraph_styles.contains_key(n)),
            named: paragraph::named(&state.doc.paragraph_styles, name),
        }
    }

    /// How a header or footer piece looking like `look` is shown: its named character styles
    /// resolved under its direct formatting.
    pub fn shown(&self, look: &marginal::Look) -> CharStyle {
        let state = self.state.read().unwrap();
        paragraph::shown(&state.doc.char_styles, look.names.as_deref(), &look.props)
    }

    /// Every footnote and endnote with the index of the block it is cited in
    /// ([`Document::notes`]), read for printing.
    pub fn notes(&self) -> Vec<(usize, marginal::Note)> {
        let state = self.state.read().unwrap();
        state
            .doc
            .notes
            .iter()
            .filter_map(|note| Some((state.doc.index_of(note.block)?, note.clone())))
            .collect()
    }

    /// How the notes look: their citations' styles and the rule over the footnotes
    /// ([`Document::note_look`]).
    pub fn note_look(&self) -> marginal::NoteLook {
        self.state.read().unwrap().doc.note_look.clone()
    }

    /// The look of the table named `name` — widths, cell styles, heading rows
    /// ([`Document::table_looks`]) — read for showing and printing.
    pub fn table_look(&self, name: &str) -> Option<table_look::TableLook> {
        self.state
            .read()
            .unwrap()
            .doc
            .table_looks
            .get(name)
            .cloned()
    }

    /// Where the block at `index`'s label and text go, as its list level states
    /// ([`numbering::LevelIndent`]): a heading's outline level, a list item's level of its list's
    /// style. `None` otherwise, or where nothing is placed. The level's margin and first-line
    /// indent give way to the paragraph's direct ones, and to a named style's only where it
    /// names the list style itself (`doc/odt-format.md` §5c facts 20, 22 and 42).
    pub fn level_indent(&self, index: usize) -> Option<numbering::LevelIndent> {
        let state = self.state.read().unwrap();
        let doc = &state.doc;
        let block = doc.blocks.get(index)?;
        let (style, level) = list_level(doc, block)?;
        let mut indent = style.indents.get(&level.max(1)).cloned()?;
        // A style naming the list style itself places its items by its own indents (fact 42).
        let resolved = crate::paragraph::resolve(
            &doc.paragraph_styles,
            &doc.default_paragraph,
            block.style.as_deref(),
        );
        if resolved
            .list_style
            .as_deref()
            .is_some_and(|name| !name.is_empty())
        {
            if let Some(margin) = &resolved.margin_left {
                indent.margin_left = Some(margin.clone());
            }
            if let Some(first) = &resolved.text_indent {
                indent.text_indent = Some(first.clone());
            }
        }
        if let Some(direct) = block
            .style
            .as_deref()
            .and_then(|name| doc.paragraph_styles.get(name))
            .filter(|style| style.automatic)
        {
            if let Some(margin) = &direct.props.margin_left {
                indent.margin_left = Some(margin.clone());
            }
            if let Some(first) = &direct.props.text_indent {
                indent.text_indent = Some(first.clone());
            }
        }
        Some(indent)
    }

    /// The picture the block at `index`'s list level draws for its label, if it is a picture
    /// bullet (`doc/odt-format.md` §5c fact 41).
    pub fn level_picture(&self, index: usize) -> Option<numbering::LevelPicture> {
        let state = self.state.read().unwrap();
        let doc = &state.doc;
        let (style, level) = list_level(doc, doc.blocks.get(index)?)?;
        style.pictures.get(&level.max(1)).cloned()
    }

    /// What kind of face each declared font family is ([`Document::font_generics`]).
    pub fn font_generics(&self) -> std::collections::HashMap<String, String> {
        self.state.read().unwrap().doc.font_generics.clone()
    }

    /// Hand `visit` every picture in the body, its MIME type and bytes, without copying them.
    pub fn visit_pictures(&self, visit: &mut dyn FnMut(&str, &[u8])) {
        let state = self.state.read().unwrap();
        for run in state.doc.blocks.iter().flat_map(|block| &block.runs) {
            if let crate::model::Run::Image { mime, data, .. } = run {
                visit(mime, data);
            }
        }
    }

    /// Every font family the document names, each distinct name once: the runs' own first, in
    /// document order, then the default paragraph style's and each paragraph style's in use.
    /// Read in place, since a window asks after every change.
    pub fn font_families(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |family: &str| {
            if !out.iter().any(|seen| seen == family) {
                out.push(family.to_owned());
            }
        };
        let styles: Vec<Option<String>> = {
            let state = self.state.read().unwrap();
            let runs = state.doc.blocks.iter().flat_map(|block| &block.runs);
            for family in runs.filter_map(|run| run.props()?.font_family.as_deref()) {
                add(family);
            }
            let mut styles = vec![None];
            for block in &state.doc.blocks {
                if block.style.is_some() && !styles.contains(&block.style) {
                    styles.push(block.style.clone());
                }
            }
            styles
        };
        for style in &styles {
            if let Some(family) = self.resolve_style(style.as_deref()).props.font_family {
                add(&family);
            }
        }
        out
    }

    /// The block at `index`'s paragraph style, resolved down its chain to the default style
    /// ([`paragraph::resolve`]), with a page break made here over it; `None` for no such block.
    pub fn paragraph(&self, index: usize) -> Option<paragraph::Resolved> {
        self.paragraph_shared(index).map(Arc::unwrap_or_clone)
    }

    /// [`App::paragraph`], shared rather than copied: what a caller asking for every block's
    /// style many times over — a page being set — reads without cloning a style each time.
    pub fn paragraph_shared(&self, index: usize) -> Option<Arc<paragraph::Resolved>> {
        let state = self.state.read().unwrap();
        let block = state.doc.block(index)?;
        let broken = |resolved: Arc<paragraph::Resolved>| {
            if !block.page_break {
                return resolved;
            }
            let mut own = (*resolved).clone();
            own.props.break_before = Some("page".to_owned());
            Arc::new(own)
        };
        let mut kept = self.paragraphs.lock().unwrap();
        if let Some(resolved) = kept.get(&block.style) {
            return Some(broken(resolved.clone()));
        }
        let style = block.style.as_deref();
        let resolved = Arc::new(paragraph::Resolved {
            props: paragraph::resolve(
                &state.doc.paragraph_styles,
                &state.doc.default_paragraph,
                style,
            ),
            declared: style.is_some_and(|name| state.doc.paragraph_styles.contains_key(name)),
            named: paragraph::named(&state.doc.paragraph_styles, style),
        });
        kept.insert(block.style.clone(), resolved.clone());
        Some(broken(resolved))
    }

    /// Which set of styles the document has: the same until another document is opened, since
    /// no edit changes a style — what a page set before an edit may be recognised by.
    pub fn styles_generation(&self) -> u64 {
        self.state.read().unwrap().doc.generation
    }

    /// Whether a page starts at block `index`: its style says so, or a break was made there.
    pub fn starts_page(&self, index: usize) -> bool {
        self.paragraph_shared(index).is_some_and(|resolved| {
            matches!(
                resolved.props.break_before.as_deref(),
                Some("page" | "even-page" | "odd-page")
            )
        })
    }
}

/// The list style and level a block's label comes from: a heading's outline level, or the level
/// of the list it is read in.
fn list_level<'a>(
    doc: &'a crate::Document,
    block: &crate::Block,
) -> Option<(&'a numbering::ListStyle, u32)> {
    match block.kind {
        BlockKind::Heading { .. }
            if let Some(mark) = doc.list_marks.get(&block.id)
                && let Some(depth) = mark.in_list =>
        {
            Some((doc.list_styles.get(mark.style.as_deref()?)?, depth))
        }
        BlockKind::Heading { level } => Some((doc.outline_style.as_ref()?, level)),
        BlockKind::ListItem { depth } => {
            let name = doc.list_marks.get(&block.id)?.style.as_deref()?;
            Some((doc.list_styles.get(name)?, depth))
        }
        BlockKind::Paragraph => None,
    }
}
