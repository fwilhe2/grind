// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The **code view** — the document as its projection (`doc/dsl.md` §6): one `<pre>` of `<span>`s,
//! a class per [`TokenKind`], since the writer already says what every byte is. Shared by both
//! panes: a projection is plain text either way, coloured by the core's vocabulary.
//!
//! **Read-only** (§6.4). Clicking a line asks the pane to select what that line projects.
//!
//! The HTML is built as a string, like `chart::svg` and `runs::html`: one `set_inner_html` per
//! render, and one escaping function for every path.

use grind_core::projection::{Projection, TokenKind};

/// The whole projection as markup: one `<div class="code-line">` per line, with its gutter and
/// coloured pieces. `cursor` is the line the selection is on; `None` when the projection does not
/// spell the selection (an empty cell, mostly), and then no line is current.
pub fn html(projection: &Projection, cursor: Option<usize>) -> String {
    let mut out = String::with_capacity(projection.text().len() * 2);
    for index in 0..projection.line_count() {
        let current = match Some(index) == cursor {
            true => " code-line-current",
            false => "",
        };
        out.push_str(&format!(
            "<div class=\"code-line{current}\" data-line=\"{index}\">\
             <span class=\"code-gutter\">{}</span>",
            index + 1
        ));
        for piece in projection.line_pieces(index) {
            match piece.kind {
                Some(kind) => out.push_str(&format!(
                    "<span class=\"code-{}\">{}</span>",
                    class(kind),
                    escape(piece.text)
                )),
                // The stretches the writer never named — indentation, braces, the spaces
                // between values — are emitted too, and unwrapped. A page that dropped them
                // would show a projection that does not parse.
                None => out.push_str(&escape(piece.text)),
            }
        }
        out.push_str("</div>");
    }
    out
}

/// The class name a kind is painted with. `style.css` holds the colours, which is where a
/// shell's colours belong — [`TokenKind::name`] is the core's word for the *thing*, and this is
/// the page's word for the same thing, deliberately equal so the stylesheet reads as the enum.
pub fn class(kind: TokenKind) -> &'static str {
    kind.name()
}

/// HTML-escaping, the same five characters `runs::html` escapes and for the same reason: a
/// document's own text is not markup, and a cell holding `<script>` is a cell.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spreadsheet() -> Projection {
        let doc = grind_sheet::projection::read(
            "grind spreadsheet\n\nsheet Sales {\n    at A1 {\n        row North 4200\n    }\n}\n",
        )
        .expect("parses");
        grind_sheet::projection::project(&doc)
    }

    #[test]
    fn every_line_is_a_row_with_a_gutter_and_its_own_number() {
        let projection = spreadsheet();
        let html = html(&projection, Some(2));
        assert_eq!(
            html.matches("class=\"code-line").count(),
            projection.line_count(),
            "one row per line, blanks included"
        );
        assert!(
            html.contains("data-line=\"2\""),
            "and each says which: {html}"
        );
        assert_eq!(
            html.matches("code-line-current").count(),
            1,
            "exactly one current line"
        );
        assert!(
            !super::html(&projection, None).contains("code-line-current"),
            "and none when the selection projects nothing"
        );
    }

    /// The whole claim of §6.1: the colours come from the writer. A `<span>` per token, classed
    /// by what the writer said it was — and the *text* still reads back as the projection.
    #[test]
    fn the_classes_are_the_writers_own_and_the_text_survives() {
        let projection = spreadsheet();
        let html = html(&projection, None);
        for class in ["code-node", "code-text", "code-number", "code-keyword"] {
            assert!(html.contains(class), "no {class} in {html}");
        }
        // Strip the markup back off and the projection is still in there, line by line.
        let stripped = strip(&html);
        for line in projection.text().lines() {
            assert!(
                stripped.contains(line.trim_start()),
                "line {line:?} did not survive being marked up: {stripped}"
            );
        }
    }

    /// A document's own text is not markup, whatever is in it.
    #[test]
    fn a_cell_holding_a_tag_is_a_cell() {
        let app = grind_sheet::App::new();
        app.enter(
            0,
            grind_sheet::Pos::new(0, 0),
            "'<script>alert(1)</script>",
            grind_sheet::RecalcMode::No,
        )
        .expect("types");
        let html = html(&app.project(), None);
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    /// Remove every tag, and turn the entities back — enough to check what a reader sees.
    fn strip(html: &str) -> String {
        let mut out = String::new();
        let mut in_tag = false;
        for c in html.chars() {
            match (c, in_tag) {
                ('<', _) => in_tag = true,
                ('>', _) => in_tag = false,
                (c, false) => out.push(c),
                _ => {}
            }
        }
        out.replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
    }
}
