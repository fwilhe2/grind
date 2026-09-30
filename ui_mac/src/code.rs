// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The source pane's arithmetic (D9, M8) — portable.
//!
//! The pane shows the document as its projection, read-only, in an `NSTextView`, and three
//! questions connect it to the document: which line the selection or the caret projects to
//! (`Projection::line_of`), what a line projects back to (`Projection::address_on_line`), and
//! where a line is — in UTF-16 units, since that is what an `NSRange` counts. The first two are
//! the core's, shared by every shell's code view; this file is the third, and the colouring,
//! which comes from the projection's own token map rather than a highlighter (`doc/dsl.md` §6).

use std::ops::Range;

use grind_core::projection::{Projection, TokenKind};
use grind_core::utf16;

/// How a token is drawn — a role in the pane's palette, resolved against the system's colours
/// by `source_pane.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    /// Node names — the structure.
    Node,
    Property,
    Text,
    Number,
    Keyword,
    Comment,
    /// Anything the token map does not claim — spaces, braces.
    Plain,
}

pub fn tint(kind: Option<TokenKind>) -> Tint {
    match kind {
        Some(TokenKind::Node) => Tint::Node,
        Some(TokenKind::Property) => Tint::Property,
        Some(TokenKind::Text) => Tint::Text,
        Some(TokenKind::Number) => Tint::Number,
        Some(TokenKind::Keyword) => Tint::Keyword,
        Some(TokenKind::Comment) => Tint::Comment,
        _ => Tint::Plain,
    }
}

/// Every coloured run of the text, as a UTF-16 range and its tint — what the pane's text storage
/// is painted with. Plain runs are left out: they take the pane's own ink.
pub fn runs(projection: &Projection) -> Vec<(Range<usize>, Tint)> {
    let text = projection.text();
    let mut out = Vec::new();
    let mut units = 0;
    for line in 0..projection.line_count() {
        let Some(span) = projection.line_span(line) else {
            continue;
        };
        units = utf16::units_before(text, span.start).max(units);
        let mut at = units;
        for piece in projection.line_pieces(line) {
            let len = piece.text.encode_utf16().count();
            let tint = tint(piece.kind);
            if tint != Tint::Plain && len > 0 {
                out.push((at..at + len, tint));
            }
            at += len;
        }
    }
    out
}

/// Where line `line` is, in UTF-16 units, without its newline.
pub fn line_range(projection: &Projection, line: usize) -> Option<Range<usize>> {
    let span = projection.line_span(line)?;
    let text = projection.text();
    Some(utf16::units_before(text, span.start)..utf16::units_before(text, span.end))
}

/// Which line UTF-16 unit `units` of the projection's text is on.
pub fn line_at(projection: &Projection, units: usize) -> usize {
    let text = projection.text();
    let byte = utf16::byte_of(text, units);
    text[..byte].matches('\n').count()
}

/// The line the document's place — `Sheet1.B2`, `p12` — projects to.
pub fn line_of(projection: &Projection, address: &str) -> Option<usize> {
    projection.line_of(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::{Pos, RecalcMode};

    fn projection() -> Projection {
        let app = grind_sheet::App::new();
        app.enter(0, Pos::new(1, 1), "héllo", RecalcMode::No)
            .unwrap();
        app.enter(0, Pos::new(4, 0), "12", RecalcMode::No).unwrap();
        app.project()
    }

    /// A line's range is the line's own text, in the units an `NSRange` counts.
    #[test]
    fn a_line_is_where_its_text_is() {
        let projection = projection();
        let text: Vec<u16> = projection.text().encode_utf16().collect();
        for line in 0..projection.line_count() {
            let range = line_range(&projection, line).unwrap();
            let shown = String::from_utf16(&text[range.clone()]).unwrap();
            let pieces: String = projection
                .line_pieces(line)
                .into_iter()
                .map(|piece| piece.text)
                .collect();
            assert_eq!(shown, pieces, "line {line}");
            assert_eq!(line_at(&projection, range.start), line);
        }
    }

    /// The round trip the pane makes: a cell to its line, and the line back to the cell.
    #[test]
    fn a_cell_projects_to_a_line_and_back() {
        let projection = projection();
        let line = line_of(&projection, "Sheet1.B2").expect("B2 is spelled");
        let back = projection.address_on_line(line).expect("the line is B2's");
        assert_eq!(line_of(&projection, back), Some(line), "{back}");
    }

    #[test]
    fn the_runs_are_the_token_maps_and_lie_inside_the_text() {
        let projection = projection();
        let total = projection.text().encode_utf16().count();
        let runs = runs(&projection);
        assert!(runs.iter().any(|(_, tint)| *tint == Tint::Node));
        assert!(
            runs.iter().any(|(_, tint)| *tint == Tint::Text),
            "the string"
        );
        assert!(runs.iter().all(|(range, _)| range.end <= total));
        assert!(runs.windows(2).all(|pair| pair[0].0.end <= pair[1].0.start));
    }
}
