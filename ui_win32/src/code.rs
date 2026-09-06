// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The "Show Source" dialog — D9, `doc/dsl.md` §6. **Portable, and tested on any host**: turning
//! a [`Projection`] into the lines a listbox can show is arithmetic over
//! [`Projection::line_count`]/[`Projection::line_pieces`], the same four line-shaped questions
//! `ui_tui`'s `code.rs` and both GTK windows' own code views already answer with — this crate
//! answers them the same way rather than inventing a fifth reading of "what is a line".
//!
//! Unlike those three shells there is no embedded pane here: this shell's dialog-for-a-list
//! idiom (`dialog::choose`, already `text_outline` and `text_block_kind_dialog`'s) is a closer
//! fit than drawing a fourth text-view widget, and it is read-only for the same reason every
//! other shell's is — §6.4 has no error-tolerant parser or model-diff to make it editable.
//! `win.rs`'s `show_source` builds this list, opens it already marked on the line the pane's own
//! selection or caret projects to (`Projection::line_of`), and turns the line picked back into
//! an address (`Projection::address_on_line`) the same go-to machinery every other jump uses.

use grind_core::projection::Projection;

/// Every line of the projection's text form, in order — row `n` is exactly what
/// [`Projection::line_pieces`] says line `n` is made of, concatenated back into one string. No
/// syntax colour: a `LISTBOX` draws one ink for the whole control, so highlighting is a gap this
/// shell's dialog idiom has and a drawn pane would not.
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
