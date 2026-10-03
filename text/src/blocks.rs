// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Moving and deleting whole paragraphs from a caret — the half of *Move Up*, *Move Down* and
//! *Delete Paragraph* that is not a menu.
//!
//! [`App::move_blocks`] and [`App::delete`] take addresses; a window has a caret and a
//! selection, and needs three decisions on top of them: which blocks the verb means (every one
//! the selection touches), where a move lands (past the neighbour, and nowhere at either end),
//! and that a document keeps at least one paragraph. The macOS shell wrote them; the browser,
//! the Windows pane and the GNOME window wanted the same, and a block in a table moves like any
//! other (the model is flat).

use std::ops::RangeInclusive;

use crate::App;

/// Move blocks `first..=last` one place up or down, as one `App::move_blocks` and so one undo
/// step. The caller moves its caret and anchor by one block the same way. `Err` is a sentence,
/// at the top or the bottom where there is nowhere to go.
pub fn shift(app: &App, blocks: RangeInclusive<usize>, up: bool) -> Result<(), String> {
    let (first, last) = (*blocks.start(), *blocks.end());
    let to = match up {
        true if first > 0 => first - 1,
        false if last + 1 < app.block_count() => last + 2,
        _ => return Err("there is nowhere to move it".to_owned()),
    };
    app.move_blocks(first..last + 1, to)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Delete blocks `first..=last` in one `App::delete`, and answer the block the caret belongs at
/// afterwards — the one that followed them, or the last there is. `Err` when they are every block
/// there is, since a document always has somewhere for a caret to be.
pub fn remove(app: &App, blocks: RangeInclusive<usize>) -> Result<usize, String> {
    let (first, last) = (*blocks.start(), *blocks.end());
    if first == 0 && last + 1 >= app.block_count() {
        return Err("a document keeps at least one paragraph".to_owned());
    }
    app.delete(first..last + 1)
        .map_err(|error| error.to_string())?;
    Ok(first.min(app.block_count().saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockKind;

    fn three() -> App {
        let app = App::new();
        app.set_text(0, "a").unwrap();
        for (i, t) in ["b", "c"].iter().enumerate() {
            app.insert(i + 1, BlockKind::Paragraph, t).unwrap();
        }
        app
    }

    fn texts(app: &App) -> Vec<String> {
        app.get_viewport(0..app.block_count())
            .iter()
            .map(|b| b.text.clone())
            .collect()
    }

    #[test]
    fn a_block_moves_past_its_neighbour_and_not_off_either_end() {
        let app = three();
        shift(&app, 1..=1, true).unwrap();
        assert_eq!(texts(&app), ["b", "a", "c"]);
        assert!(shift(&app, 0..=0, true).is_err());
        shift(&app, 0..=1, false).unwrap();
        assert_eq!(texts(&app), ["c", "b", "a"]);
        assert!(shift(&app, 2..=2, false).is_err());
    }

    #[test]
    fn deleting_lands_on_what_followed_and_keeps_one_block() {
        let app = three();
        assert_eq!(remove(&app, 1..=1), Ok(1));
        assert_eq!(texts(&app), ["a", "c"]);
        assert_eq!(remove(&app, 1..=1), Ok(0), "the last block: the one before");
        assert!(remove(&app, 0..=0).is_err());
    }
}
