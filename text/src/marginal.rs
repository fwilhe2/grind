// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A page's header and footer — the master page's `style:header` and `style:footer` (rng:11962,
//! rng:10844) — read for printing and never written (`doc/pdf-export.md`); the envelope carries
//! `office:master-styles` out byte for byte. A paragraph is kept as text, with `text:page-number`
//! and `text:page-count` (rng:8696, rng:8751) as fields filled per page (`doc/odt-format.md` §5c,
//! fact 9).
//!
//! A table's or a list's paragraphs are kept as the header's own, in order, and a master's
//! first-page variants (`style:header-first`) beside its own, and each piece's character
//! formatting as a body run's, and the pictures anchored to a paragraph. Not kept: a picture set
//! as a character, a table's grid, and the left-page variants (`style:header-left`).

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

/// How a piece looks, as a body run does: its named character styles, space-separated, under its
/// direct formatting ([`crate::paragraph::shown`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub names: Option<String>,
    pub props: crate::style::CharStyle,
}

/// One paragraph of a header or footer, and the paragraph style it names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paragraph {
    pub style: Option<String>,
    pub parts: Vec<Part>,
    /// Each part's look, one per part; a part without one is unformatted.
    pub looks: Vec<Look>,
    /// The pictures anchored to it, which take no room.
    pub pictures: Vec<Picture>,
}

/// A picture anchored to a header or footer paragraph, placed by its graphic style
/// (`doc/odt-format.md` §5c fact 27). Lengths in millimetres.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub mime: String,
    pub data: std::sync::Arc<Vec<u8>>,
    pub width: f64,
    pub height: f64,
    pub across: Placed,
    pub down: Placed,
}

/// Where a picture goes along one axis: `style:horizontal-pos` or `style:vertical-pos`, the
/// `-rel` box it is measured in, both as spelled, and `svg:x` or `svg:y`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placed {
    pub pos: String,
    pub rel: String,
    pub offset: f64,
}

impl Placed {
    /// Where an extent `size` long starts, given the start and length of the box each `rel`
    /// names — `paper`, the text area or the body's (`content`) and the paragraph's — in a unit
    /// `scale` times a millimetre.
    pub fn start(
        &self,
        scale: f64,
        size: f64,
        paper: (f64, f64),
        content: (f64, f64),
        paragraph: (f64, f64),
    ) -> f64 {
        let (from, length) = match self.rel.as_str() {
            "page" => paper,
            "paragraph" => paragraph,
            _ => content,
        };
        match self.pos.as_str() {
            "left" | "top" => from,
            "right" | "bottom" => from + length - size,
            "center" | "middle" => from + (length - size) / 2.0,
            _ => from + self.offset * scale,
        }
    }
}

impl Paragraph {
    /// Its text as printed on page `page` (from 1) of `pages`.
    pub fn text(&self, page: usize, pages: usize) -> String {
        self.pieces(page, pages).map(|(text, _)| text).collect()
    }

    /// Each part's text on page `page` of `pages`, with its look.
    pub fn pieces(
        &self,
        page: usize,
        pages: usize,
    ) -> impl Iterator<Item = (String, Option<&Look>)> {
        let looks = self.looks.iter().map(Some).chain(std::iter::repeat(None));
        self.parts.iter().zip(looks).map(move |(part, look)| {
            let text = match part {
                Part::Text(text) => text.clone(),
                Part::PageNumber => page.to_string(),
                Part::PageCount => pages.to_string(),
            };
            (text, look)
        })
    }

    /// Append a part that looks like `look`.
    pub(crate) fn push(&mut self, part: Part, look: &Look) {
        self.parts.push(part);
        self.looks.push(look.clone());
    }

    /// Append text, joining it to text already at the end that looks the same.
    pub(crate) fn push_text(&mut self, text: &str, look: &Look) {
        if let (Some(Part::Text(last)), Some(last_look)) =
            (self.parts.last_mut(), self.looks.last())
            && last_look == look
        {
            last.push_str(text);
            return;
        }
        self.push(Part::Text(text.to_owned()), look);
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
            parts: vec![
                Part::Text("Page ".into()),
                Part::PageNumber,
                Part::Text(" of ".into()),
                Part::PageCount,
            ],
            ..Default::default()
        };
        assert_eq!(paragraph.text(2, 11), "Page 2 of 11");
    }

    #[test]
    fn text_pushed_after_text_is_one_part() {
        let mut paragraph = Paragraph::default();
        let plain = Look::default();
        let named = Look {
            names: Some("Emphasis".into()),
            ..Default::default()
        };
        paragraph.push_text("a", &plain);
        paragraph.push_text("b", &plain);
        paragraph.push(Part::PageNumber, &plain);
        paragraph.push_text("c", &plain);
        paragraph.push_text("d", &named);
        assert_eq!(
            paragraph.parts,
            vec![
                Part::Text("ab".into()),
                Part::PageNumber,
                Part::Text("c".into()),
                Part::Text("d".into())
            ]
        );
        assert_eq!(paragraph.looks[3], named);
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
