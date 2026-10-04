// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A page's header and footer — the master page's `style:header` and `style:footer` (rng:11962,
//! rng:10844) — **read for printing and never written** (`doc/pdf-export.md` P8).
//!
//! The envelope already carries `office:master-styles` out of a file byte for byte, so nothing
//! here is a model to edit: it is what a printed page needs to draw the same header Writer does
//! (`doc/odt-format.md` §5c, fact 9). A paragraph is kept as text and the two fields that need a
//! page to have a value — `text:page-number` and `text:page-count` (rng:8696, rng:8751) — whose
//! cached text in the file is dropped, since on paper it is the page's own.
//!
//! What is not kept, named: character formatting inside a header, pictures and tables in one,
//! and the first-page and left-page variants (`style:header-first`, `style:header-left`).

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

/// A header or a footer: its paragraphs, and the room its page layout gives it in millimetres —
/// the least height it takes (`fo:min-height`) and the space between it and the body
/// (`fo:margin-bottom` for a header, `fo:margin-top` for a footer).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marginal {
    pub paragraphs: Vec<Paragraph>,
    pub min_height: f64,
    pub spacing: f64,
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
