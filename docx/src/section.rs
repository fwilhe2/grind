// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `w:sectPr` — a section's page: its size, its margins, its header and footer, and how it
//! starts (§17.6).
//!
//! A Word document is a sequence of sections, each closed by a `w:sectPr` — inside the last
//! paragraph's `w:pPr` for every section but the last, whose `w:sectPr` is the body's own last
//! child. The text model has **one** page ([`grind_text::Document::page`], `doc/pdf-export.md`
//! P2), so the first section's page becomes the document's, and every later section is two
//! questions: does it *start* on a new page (a break the model can carry, as a paragraph's
//! `fo:break-before`), and does its page *differ* (which it cannot, and which is counted).

use crate::xml::{Handled, Reader, Word as _, WordAttrs as _};

/// Which pages a header or footer reference applies to (§17.18.36, `ST_HdrFtr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Default,
    First,
    Even,
}

/// One section's page, in twips, as the file states it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub landscape: bool,
    pub top: Option<i64>,
    pub bottom: Option<i64>,
    pub left: Option<i64>,
    pub right: Option<i64>,
    /// The distance from the page's edge to the header's top and to the footer's bottom.
    pub header: Option<i64>,
    pub footer: Option<i64>,
    pub gutter: Option<i64>,
    /// `w:headerReference`/`w:footerReference`: which pages, and the relationship id.
    pub headers: Vec<(Which, String)>,
    pub footers: Vec<(Which, String)>,
    /// `w:titlePg` — the first page has its own header and footer.
    pub title_page: bool,
    /// `w:type` — `nextPage` (the default), `continuous`, `evenPage`, `oddPage`, `nextColumn`.
    pub start: Option<String>,
    /// `w:cols/@w:num` above one.
    pub columns: Option<i64>,
}

impl Section {
    /// Whether this section starts on a new page — every kind but `continuous` and
    /// `nextColumn` does, and `nextPage` is what an absent `w:type` means (§17.6.22).
    pub fn starts_a_page(&self) -> bool {
        !matches!(self.start.as_deref(), Some("continuous" | "nextColumn"))
    }

    /// Whether two sections are set on the same page — the question a later section is
    /// asked, since the model has one page for the whole document.
    pub fn same_page(&self, other: &Section) -> bool {
        (
            self.width,
            self.height,
            self.top,
            self.bottom,
            self.left,
            self.right,
        ) == (
            other.width,
            other.height,
            other.top,
            other.bottom,
            other.left,
            other.right,
        )
    }
}

/// Read a `w:sectPr` the reader has just opened.
pub fn read(r: &mut Reader) -> grind_ooxml::Result<Section> {
    let mut s = Section::default();
    r.children(|_, name, attrs| {
        if name.w("pgSz") {
            s.width = attrs.int("w");
            s.height = attrs.int("h");
            s.landscape = attrs.w("orient") == Some("landscape");
        } else if name.w("pgMar") {
            s.top = attrs.int("top");
            s.bottom = attrs.int("bottom");
            s.left = attrs.int("left").or_else(|| attrs.int("start"));
            s.right = attrs.int("right").or_else(|| attrs.int("end"));
            s.header = attrs.int("header");
            s.footer = attrs.int("footer");
            s.gutter = attrs.int("gutter");
        } else if name.w("headerReference") || name.w("footerReference") {
            let which = match attrs.w("type") {
                Some("first") => Which::First,
                Some("even") => Which::Even,
                _ => Which::Default,
            };
            if let Some(id) = attrs.rel("id") {
                let list = if name.w("headerReference") {
                    &mut s.headers
                } else {
                    &mut s.footers
                };
                list.push((which, id.to_owned()));
            }
        } else if name.w("titlePg") {
            s.title_page = attrs.on();
        } else if name.w("type") {
            s.start = attrs.val().map(str::to_owned);
        } else if name.w("cols") {
            s.columns = attrs.int("num").filter(|n| *n > 1);
        } else {
            return Ok(Handled::No);
        }
        Ok(Handled::Yes)
    })?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = grind_ooxml::names::WORD_T;
    const R: &str = grind_ooxml::names::REL_T;

    #[test]
    fn a_section_reads_its_page_and_its_marginals() {
        let xml = format!(
            r#"<w:sectPr xmlns:w="{W}" xmlns:r="{R}">
                 <w:headerReference w:type="default" r:id="rId8"/>
                 <w:footerReference w:type="first" r:id="rId9"/>
                 <w:pgSz w:w="11906" w:h="16838"/>
                 <w:pgMar w:top="1417" w:right="1134" w:bottom="1134" w:left="1417" w:header="708" w:footer="708" w:gutter="0"/>
                 <w:cols w:space="708"/>
                 <w:titlePg/>
               </w:sectPr>"#
        );
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap();
        let s = read(&mut reader).unwrap();
        assert_eq!((s.width, s.height), (Some(11906), Some(16838)));
        assert_eq!(
            (s.top, s.left, s.header),
            (Some(1417), Some(1417), Some(708))
        );
        assert_eq!(s.headers, vec![(Which::Default, "rId8".to_owned())]);
        assert_eq!(s.footers, vec![(Which::First, "rId9".to_owned())]);
        assert!(s.title_page);
        assert_eq!(s.columns, None, "one column is no columns");
        assert!(s.starts_a_page(), "an absent type is nextPage");
    }
}
