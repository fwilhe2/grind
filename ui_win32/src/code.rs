// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Show Source dialog (`doc/dsl.md` §6) — portable: turning a [`Projection`] into the lines a
//! listbox shows is arithmetic over [`Projection::line_count`]/[`Projection::line_pieces`], the
//! questions every shell's code view asks. A list in `dialog::choose` rather than a drawn text
//! pane, read-only (§6.4). `win.rs`'s `show_source` opens it on the line the selection or caret
//! projects to (`Projection::line_of`) and turns the line picked back into an address
//! (`Projection::address_on_line`) for the go-to every jump uses.

use grind_core::projection::Projection;

/// Every line of the projection's text form, in order — row `n` is [`Projection::line_pieces`]'
/// line `n` concatenated. No syntax colour: a `LISTBOX` draws one ink.
pub fn rows(projection: &Projection) -> Vec<String> {
    (0..projection.line_count())
        .map(|line| {
            projection
                .line_pieces(line)
                .into_iter()
                .map(|piece| piece.text)
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection() -> Projection {
        let app = grind_sheet::App::new();
        app.enter(
            0,
            grind_sheet::model::Pos::new(0, 0),
            "1",
            grind_sheet::RecalcMode::No,
        )
        .unwrap();
        app.project()
    }

    #[test]
    fn every_row_is_the_lines_own_text() {
        let projection = projection();
        let rows = rows(&projection);
        assert_eq!(rows.len(), projection.line_count());
        for (index, row) in rows.iter().enumerate() {
            let expected: String = projection
                .line_pieces(index)
                .into_iter()
                .map(|piece| piece.text)
                .collect();
            assert_eq!(*row, expected);
        }
    }

    #[test]
    fn an_empty_document_still_has_a_source() {
        let app = grind_sheet::App::new();
        let projection = app.project();
        assert_eq!(rows(&projection).len(), projection.line_count());
        assert!(!rows(&projection).is_empty(), "the kind header is a line");
    }
}
