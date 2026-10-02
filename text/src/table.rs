// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a shell puts a table — the half of *Insert Table* that is not a dialog.
//!
//! [`App::insert_table`] does exactly what `grind text table` says: it inserts a table at an
//! index and nothing else. A window needs two decisions on top, and the GNOME window had them
//! to itself until the browser and the Windows pane wanted the same verb: **below** the caret's
//! block (and below the whole table when the caret is already in one — inserting at `block + 1`
//! would cut that table in two), and **never last**, since a document must not *end* with a
//! table: there would be nowhere to type after it, and LibreOffice appends a paragraph to any
//! document that does (`doc/odt-format.md` §5b).

use crate::{App, BlockKind, Result};

/// `3x4`, `3 4`, `3×4` or `3, 4` — rows, then columns, each at least one and not absurd.
pub fn parse_size(answer: &str) -> Option<(u32, u32)> {
    let mut numbers = answer
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().ok());
    let (rows, columns) = (numbers.next()??, numbers.next()??);
    (numbers.next().is_none() && (1..=64).contains(&rows) && (1..=16).contains(&columns))
        .then_some((rows, columns))
}

/// Insert a `rows` × `columns` table below the block at `block`, and return the index of its
/// first cell — where the caret belongs afterwards. When the table would end the document an
/// empty paragraph follows it, as a second action and so a second undo step, which is the honest
/// cost of the core's own verb doing exactly what it says.
pub fn insert_below(app: &App, block: usize, rows: u32, columns: u32) -> Result<usize> {
    let count = app.block_count();
    let at = match app.table(block) {
        Some(table) => table.blocks.end,
        None => block + 1,
    }
    .min(count);
    app.insert_table(at, rows, columns, None)?;
    if at + (rows * columns) as usize == app.block_count() {
        app.insert(app.block_count(), BlockKind::Paragraph, "")?;
    }
    Ok(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(app: &App) -> Vec<String> {
        app.get_viewport(0..app.block_count())
            .iter()
            .map(|b| b.text.clone())
            .collect()
    }

    #[test]
    fn a_table_goes_below_the_block_and_the_document_still_ends_in_a_paragraph() {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "one").unwrap();
        let at = insert_below(&app, 0, 1, 2).unwrap();
        assert_eq!(at, 1);
        assert_eq!(app.table(1).unwrap().blocks, 1..3);
        assert_eq!(app.block_count(), 4, "a paragraph to type in after it");
        assert_eq!(text(&app)[3], "");
    }

    #[test]
    fn from_inside_a_table_the_new_one_goes_after_it_rather_than_through_it() {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "one").unwrap();
        insert_below(&app, 0, 2, 2).unwrap();
        app.insert(app.block_count(), BlockKind::Paragraph, "end")
            .unwrap();
        let first = app.table(1).unwrap();
        let at = insert_below(&app, 2, 1, 1).unwrap();
        assert_eq!(at, first.blocks.end);
        assert_eq!(app.table(1).unwrap().blocks, first.blocks, "intact");
    }

    #[test]
    fn a_table_size_is_two_numbers_however_they_are_separated() {
        for answer in ["3x4", "3 4", "3×4", " 3, 4 "] {
            assert_eq!(parse_size(answer), Some((3, 4)), "{answer:?}");
        }
        for answer in ["", "3", "0x2", "2x0", "3x4x5", "x", "99x2", "2x99"] {
            assert_eq!(parse_size(answer), None, "{answer:?}");
        }
    }
}
