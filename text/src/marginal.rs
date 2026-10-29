// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A page's header and footer — the master page's `style:header` and `style:footer` (rng:11962,
//! rng:10844) — read for printing and never written (`doc/pdf-export.md`); the envelope carries
//! `office:master-styles` out byte for byte. A paragraph is kept as text, with `text:page-number`
//! and `text:page-count` (rng:8696, rng:8751) as fields filled per page (`doc/odt-format.md` §5c,
//! fact 9).
//!
//! A table's or a list's paragraphs are kept as the header's own, in order. Not kept: character
//! formatting, pictures, a table's grid, and the first-page and left-page variants
//! (`style:header-first`, `style:header-left`).

/// One piece of a header or footer paragraph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// Text, a `text:tab` as `\t` and a `text:line-break` as `\n`, as in a body run.
    Text(String),
    /// `text:page-number`: the number of the page it is printed on, from 1.
    PageNumber,
    /// `text:page-count`: how many pages the document prints on.
    PageCount,
}

/// One paragraph of a header or footer, and the paragraph style it names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Paragraph {
    pub style: Option<String>,
    pub parts: Vec<Part>,
}

impl Paragraph {
    /// Its text as printed on page `page` (from 1) of `pages`.
    pub fn text(&self, page: usize, pages: usize) -> String {
        self.parts
            .iter()
            .map(|part| match part {
                Part::Text(text) => text.clone(),
                Part::PageNumber => page.to_string(),
                Part::PageCount => pages.to_string(),
            })
            .collect()
    }

    /// Append text, joining it to text already at the end.
    pub(crate) fn push_text(&mut self, text: &str) {
        match self.parts.last_mut() {
            Some(Part::Text(last)) => last.push_str(text),
            _ => self.parts.push(Part::Text(text.to_owned())),
        }
    }
}

/// A header or footer: its paragraphs and the room its page layout gives it, in millimetres —
/// `fo:min-height`, and the space to the body (`fo:margin-bottom` for a header, `fo:margin-top`
/// for a footer).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marginal {
    pub paragraphs: Vec<Paragraph>,
    pub min_height: f64,
    pub spacing: f64,
}

/// A footnote or endnote (`text:note`, rng:8465): where it is cited, the citation's text and its
/// paragraphs, read for a printed page to set at its foot. Never written; the paragraph holding
/// one keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// The block the citation stands in, and the character offset its text starts at.
    pub block: crate::model::BlockId,
    pub offset: usize,
    /// `text:note-citation`'s text — the number or mark shown in the paragraph.
    pub citation: String,
    /// `text:note-class="endnote"`: set at the end rather than at the foot of the page.
    pub endnote: bool,
    pub paragraphs: Vec<Paragraph>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fields_take_the_pages_own_values() {
        let paragraph = Paragraph {
            style: None,
            parts: vec![
                Part::Text("Page ".into()),
                Part::PageNumber,
                Part::Text(" of ".into()),
                Part::PageCount,
            ],
        };
        assert_eq!(paragraph.text(2, 11), "Page 2 of 11");
    }

    #[test]
    fn text_pushed_after_text_is_one_part() {
        let mut paragraph = Paragraph::default();
        paragraph.push_text("a");
        paragraph.push_text("b");
        paragraph.parts.push(Part::PageNumber);
        paragraph.push_text("c");
        assert_eq!(
            paragraph.parts,
            vec![
                Part::Text("ab".into()),
                Part::PageNumber,
                Part::Text("c".into())
            ]
        );
    }
}

/// A straight line drawn on every page set on a master page — a `draw:line` in its header or
/// footer: a DIN 5008 letter's fold and punch marks, a rule under a letterhead. Read for printing.
/// Coordinates are millimetres from the page's top-left corner (`svg:x1…y2`); the stroke is the
/// graphic style's, or half a point of black.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub from: (f64, f64),
    pub to: (f64, f64),
    /// In millimetres.
    pub width: f64,
    /// `#rrggbb`.
    pub color: String,
}
