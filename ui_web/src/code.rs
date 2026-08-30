// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The **code view** — the document as its projection, shown (`doc/dsl.md` §6, D9).
//!
//! One `<pre>` of `<span>`s, which is what §6.1 predicted this shell's half would cost: the
//! projection writer already says what every byte it emits *is*, so the page paints a class per
//! [`TokenKind`] and takes no highlighter. `ui_web/src/text/runs.rs` already cuts a laid-out line
//! into spans for the document pane, and this is the same shape over a different cutter.
//!
//! **Shared by both panes**, like [`crate::palette`] and unlike everything else here — the grid
//! and the flow have no markup in common, and their projections have all of it, because a
//! projection is plain text either way and its colours come from a vocabulary the core owns.
//!
//! **Read-only** (§6.4). Clicking a line asks the pane to select what that line projects, which
//! is §6.2's map in the direction that has to be built rather than assumed; typing into it is a
//! feature with three prerequisites this build has none of, and the gate is in `doc/dsl.md`.
//!
//! The HTML is built as a string rather than through `Document::create_element`, exactly as
//! `chart::svg` and `runs::html` already are: one `set_inner_html` per render beats a hundred
//! DOM calls, and the escaping is one function that every path goes through.

use grind_core::projection::{Projection, TokenKind};

/// The whole projection as markup: one `<div class="code-line">` per line, holding the gutter
/// and the coloured pieces.
///
/// `cursor` is the line the pane's selection is on, drawn as the current line. `None` means the
/// selection is somewhere the projection does not spell — an empty cell, most often — and then
/// no line is current, which is honest rather than defaulting to the first.
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
