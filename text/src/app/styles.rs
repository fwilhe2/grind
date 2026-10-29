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
        }
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
    /// indent give way to the paragraph's direct ones, never to a named style's
    /// (`doc/odt-format.md` §5c facts 20 and 22).
    pub fn level_indent(&self, index: usize) -> Option<numbering::LevelIndent> {
        let state = self.state.read().unwrap();
        let doc = &state.doc;
        let block = doc.blocks.get(index)?;
        let (style, level) = match block.kind {
            BlockKind::Heading { .. }
                if let Some(mark) = doc.list_marks.get(&block.id)
                    && let Some(depth) = mark.in_list =>
            {
                (doc.list_styles.get(mark.style.as_deref()?)?, depth)
            }
            BlockKind::Heading { level } => (doc.outline_style.as_ref()?, level),
            BlockKind::ListItem { depth } => {
                let name = doc.list_marks.get(&block.id)?.style.as_deref()?;
                (doc.list_styles.get(name)?, depth)
            }
            _ => return None,
        };
        let mut indent = style.indents.get(&level.max(1)).cloned()?;
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

    /// What kind of face each declared font family is ([`Document::font_generics`]).
    pub fn font_generics(&self) -> std::collections::HashMap<String, String> {
        self.state.read().unwrap().doc.font_generics.clone()
    }

    /// The block at `index`'s paragraph style, resolved down its chain to the default style
    /// ([`paragraph::resolve`]); `None` for no such block.
    pub fn paragraph(&self, index: usize) -> Option<paragraph::Resolved> {
        let state = self.state.read().unwrap();
        let block = state.doc.block(index)?;
        let style = block.style.as_deref();
        Some(paragraph::Resolved {
            props: paragraph::resolve(
                &state.doc.paragraph_styles,
                &state.doc.default_paragraph,
                style,
            ),
            declared: style.is_some_and(|name| state.doc.paragraph_styles.contains_key(name)),
        })
    }
}
