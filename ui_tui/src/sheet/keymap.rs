// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Vi-style Normal-mode key handling, as a pure function — the same shape as
//! `ui_sheet_gtk/src/keymap.rs`: key plus modifiers in, an [`Action`] out, no ratatui `Frame` or
//! terminal handle anywhere near it, so the whole map unit-tests with no terminal attached.
//!
//! `KeyCode`/`KeyModifiers` are used directly rather than through a shell-local `Key` enum —
//! unlike GTK's `gdk::Key`, crossterm's is already a plain data type with nothing to
//! translate away.
//!
//! ponytail: `g` alone stands in for vi's `gg` (a real two-key chord would need the caller
//! to track a pending first key). One key does the same job here; upgrade if a document ever
//! needs a literal `g` bound to something else.

use std::collections::HashSet;

use super::{MAX_COLS, MAX_ROWS};
use grind_sheet::Pos;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// The tracks the document does not show, on both axes.
///
/// Handed in rather than looked up, because this module is pure and has never seen a document —
/// which is exactly the upgrade `doc/sheet-shell.md` names for the GNOME window's own version of
/// this gap: *`keymap.rs` is pure and knows nothing about the document, so skipping means handing
/// it the hidden set.*
#[derive(Clone, Copy, Debug)]
pub struct Folded<'a> {
    pub rows: &'a HashSet<u32>,
    pub cols: &'a HashSet<u32>,
}

impl Folded<'static> {
    /// A sheet that folds nothing away.
    ///
    /// Test-only, like [`crate::code::Code::line`] and for the same reason: the shell always has
    /// the two real sets to hand (`App::folded_rows`, `App::folded_cols`), so a constructor that
    /// said *nothing is hidden* would only ever be a way of not asking.
    #[cfg(test)]
    pub fn none() -> Self {
        static EMPTY: std::sync::OnceLock<HashSet<u32>> = std::sync::OnceLock::new();
        let empty = EMPTY.get_or_init(HashSet::new);
        Folded {
            rows: empty,
            cols: empty,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    By(Dir),
    Page(Dir),
    RowStart,
    RowEnd,
    SheetStart,
    SheetEnd,
}

/// What a Normal- or Visual-mode key asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    /// `i` / `a` — start editing, keeping the cell's current text.
    Insert,
    /// `c` — start editing from empty.
    Change,
    /// `x` — empty the cell, or the whole selection in Visual mode: the typing rule's own
    /// "empty input clears it" (`App::enter`), over a rectangle.
    Clear,
    Undo,
    Redo,
    /// `:` — open the command line.
    Command,
    /// `v` — start selecting a rectangle, or stop. The terminal's answer to dragging one out,
    /// and what every verb over a *range* needs before it can exist.
    Visual,
    /// `y` — copy the selection into the register as tab-separated text; `p` puts it back.
    Yank,
    Put,
    /// A marker key over the selection: `*` bold, `/` italic — **the same two keys the word
    /// processor's Visual mode uses** (`crate::text::keymap`), because one suite should not
    /// have two vocabularies for emphasis.
    Bold,
    Italic,
    /// `-` — back to no styling at all.
    Plain,
    /// `n` / `N` — the next or previous match of the last `:find`, vi's own two keys for it.
    Next(bool),
    /// `Esc` — leave Visual mode, changing nothing.
    Escape,
}

/// The key map. One table, no state — `None` means the shell does not claim the key.
///
/// `visual` says whether a rectangle is being dragged out, and changes only what the keys that
/// need a range mean.
pub fn normal_action(code: KeyCode, mods: KeyModifiers, visual: bool) -> Option<Action> {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let by = |dir| Some(Action::Move(Motion::By(dir)));
    match code {
        KeyCode::Char('v') => Some(Action::Visual),
        KeyCode::Esc => Some(Action::Escape),
        KeyCode::Char('y') => Some(Action::Yank),
        KeyCode::Char('p') => Some(Action::Put),
        // Formatting only binds once there is something to format, so `*` and `-` stay
        // available to whatever wants them next.
        KeyCode::Char('*') if visual => Some(Action::Bold),
        KeyCode::Char('/') if visual => Some(Action::Italic),
        KeyCode::Char('-') if visual => Some(Action::Plain),
        KeyCode::Char('h') | KeyCode::Left => by(Dir::Left),
        KeyCode::Char('l') | KeyCode::Right => by(Dir::Right),
        KeyCode::Char('k') | KeyCode::Up => by(Dir::Up),
        KeyCode::Char('j') | KeyCode::Down => by(Dir::Down),
        KeyCode::Char('f') if ctrl => Some(Action::Move(Motion::Page(Dir::Down))),
        KeyCode::Char('b') if ctrl => Some(Action::Move(Motion::Page(Dir::Up))),
        KeyCode::PageDown => Some(Action::Move(Motion::Page(Dir::Down))),
        KeyCode::PageUp => Some(Action::Move(Motion::Page(Dir::Up))),
        KeyCode::Char('0') | KeyCode::Home => Some(Action::Move(Motion::RowStart)),
        KeyCode::Char('$') | KeyCode::End => Some(Action::Move(Motion::RowEnd)),
        KeyCode::Char('g') => Some(Action::Move(Motion::SheetStart)),
        KeyCode::Char('G') => Some(Action::Move(Motion::SheetEnd)),
        KeyCode::Char('x') | KeyCode::Char('d') => Some(Action::Clear),
        KeyCode::Char('i') | KeyCode::Char('a') => Some(Action::Insert),
        KeyCode::Char('c') => Some(Action::Change),
        KeyCode::Char('n') => Some(Action::Next(true)),
        KeyCode::Char('N') => Some(Action::Next(false)),
        KeyCode::Char('u') => Some(Action::Undo),
        KeyCode::Char('r') if ctrl => Some(Action::Redo),
        KeyCode::Char(':') => Some(Action::Command),
        _ => None,
    }
}

/// Apply a motion, clamped to the sheet's own limits — [`super::MAX_ROWS`]/[`super::MAX_COLS`]
/// for a plain move, `extent` (the used region) for the row/sheet edges, matching
/// `ui_sheet_gtk/src/keymap.rs`'s `moved` — and **over the tracks the document actually shows**.
///
/// Every distance here is counted in tracks that are *drawn*: `j` is the next row on screen, not
/// the next row in the model, and a page is a screenful of rows somebody can see. Anything else
/// puts the cursor where nothing is drawn, which reads as a shell dropping keystrokes rather than
/// as a cursor sitting on a folded row — see [`walk`].
pub fn moved(from: Pos, motion: Motion, extent: (u32, u32), page: u32, folded: Folded<'_>) -> Pos {
    let page = page.max(1);
    let (last_row, last_col) = (MAX_ROWS - 1, MAX_COLS - 1);
    match motion {
        Motion::By(dir) => step(from, dir, 1, folded),
        Motion::Page(dir) => step(from, dir, page, folded),
        Motion::RowStart => Pos::new(from.row, shown(0, folded.cols, last_col)),
        Motion::RowEnd => Pos::new(
            from.row,
            shown_back(extent.1.saturating_sub(1), folded.cols),
        ),
        Motion::SheetStart => Pos::new(
            shown(0, folded.rows, last_row),
            shown(0, folded.cols, last_col),
        ),
        Motion::SheetEnd => Pos::new(
            shown_back(extent.0.saturating_sub(1), folded.rows),
            shown_back(extent.1.saturating_sub(1), folded.cols),
        ),
    }
}

fn step(from: Pos, dir: Dir, by: u32, folded: Folded<'_>) -> Pos {
    let (last_row, last_col) = (MAX_ROWS - 1, MAX_COLS - 1);
    match dir {
        Dir::Left => Pos::new(from.row, walk(from.col, by, false, folded.cols, last_col)),
        Dir::Right => Pos::new(from.row, walk(from.col, by, true, folded.cols, last_col)),
        Dir::Up => Pos::new(walk(from.row, by, false, folded.rows, last_row), from.col),
        Dir::Down => Pos::new(walk(from.row, by, true, folded.rows, last_row), from.col),
    }
}

/// Move `by` tracks **the document shows**, stopping at the sheet's own edge and never landing on
/// a folded one.
///
/// The bug this exists for: a filter that folds rows 3 to 7 away used to mean five presses of `j`
/// to move one row on screen, with the cursor invisible for four of them — which does not look
/// like a cursor on a hidden row, it looks like a terminal dropping keys. Every spreadsheet steps
/// over what it is not drawing, and now so does this one.
///
/// The scan past a run of folded tracks is bounded by how many of them there are, and the outer
/// loop by `by`; a sheet whose every remaining track is folded stops where it started.
fn walk(at: u32, by: u32, forward: bool, folded: &HashSet<u32>, last: u32) -> u32 {
    let mut here = at;
    for _ in 0..by {
        let mut next = here;
        loop {
            next = match next {
                n if forward && n < last => n + 1,
                n if !forward && n > 0 => n - 1,
                // The edge, or nothing but folded tracks between here and it: stopping on the
                // last one that shows beats stopping on one that does not.
                _ => return here,
            };
            if !folded.contains(&next) {
                break;
            }
        }
        here = next;
    }
    here
}

/// The first track at or after `at` that the document shows — where an edge motion lands when the
/// edge itself is folded away, and where the cursor goes after `:hide` takes its column out.
pub fn shown(at: u32, folded: &HashSet<u32>, last: u32) -> u32 {
    let mut here = at;
    while here < last && folded.contains(&here) {
        here += 1;
    }
    here
}

/// The last track at or before `at` that the document shows — [`shown`] the other way, for the
/// motions that go to an end rather than to a beginning.
fn shown_back(at: u32, folded: &HashSet<u32>) -> u32 {
    let mut here = at;
    while here > 0 && folded.contains(&here) {
        here -= 1;
    }
    here
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hjkl_and_the_arrows_agree() {
        for (ch, arrow, dir) in [
            ('h', KeyCode::Left, Dir::Left),
            ('l', KeyCode::Right, Dir::Right),
            ('k', KeyCode::Up, Dir::Up),
            ('j', KeyCode::Down, Dir::Down),
        ] {
            let want = Some(Action::Move(Motion::By(dir)));
            assert_eq!(
                normal_action(KeyCode::Char(ch), KeyModifiers::NONE, false),
                want
            );
            assert_eq!(normal_action(arrow, KeyModifiers::NONE, false), want);
        }
    }

    #[test]
    fn ctrl_f_and_b_page_like_vi() {
        assert_eq!(
            normal_action(KeyCode::Char('f'), KeyModifiers::CONTROL, false),
            Some(Action::Move(Motion::Page(Dir::Down)))
        );
        assert_eq!(
            normal_action(KeyCode::Char('b'), KeyModifiers::CONTROL, false),
            Some(Action::Move(Motion::Page(Dir::Up)))
        );
        // Unmodified 'f'/'b' are not bound to anything.
        assert_eq!(
            normal_action(KeyCode::Char('f'), KeyModifiers::NONE, false),
            None
        );
    }

    #[test]
    fn insert_change_and_clear() {
        assert_eq!(
            normal_action(KeyCode::Char('i'), KeyModifiers::NONE, false),
            Some(Action::Insert)
        );
        assert_eq!(
            normal_action(KeyCode::Char('a'), KeyModifiers::NONE, false),
            Some(Action::Insert)
        );
        assert_eq!(
            normal_action(KeyCode::Char('c'), KeyModifiers::NONE, false),
            Some(Action::Change)
        );
        assert_eq!(
            normal_action(KeyCode::Char('x'), KeyModifiers::NONE, false),
            Some(Action::Clear)
        );
    }

    /// vi's own two keys for stepping a search, and they mean the same thing in both halves of
    /// this shell — one suite should not have two ways to say "the next one".
    #[test]
    fn n_and_shift_n_step_a_search() {
        assert_eq!(
            normal_action(KeyCode::Char('n'), KeyModifiers::NONE, false),
            Some(Action::Next(true))
        );
        assert_eq!(
            normal_action(KeyCode::Char('N'), KeyModifiers::NONE, false),
            Some(Action::Next(false))
        );
    }

    #[test]
    fn undo_redo_and_command_line() {
        assert_eq!(
            normal_action(KeyCode::Char('u'), KeyModifiers::NONE, false),
            Some(Action::Undo)
        );
        assert_eq!(
            normal_action(KeyCode::Char('r'), KeyModifiers::CONTROL, false),
            Some(Action::Redo)
        );
        assert_eq!(
            normal_action(KeyCode::Char(':'), KeyModifiers::NONE, false),
            Some(Action::Command)
        );
    }

    #[test]
    fn a_plain_move_stops_at_the_sheet_edge() {
        assert_eq!(
            step(Pos::new(0, 0), Dir::Left, 1, Folded::none()),
            Pos::new(0, 0)
        );
        assert_eq!(
            step(Pos::new(0, 0), Dir::Up, 1, Folded::none()),
            Pos::new(0, 0)
        );
        assert_eq!(
            step(Pos::new(MAX_ROWS - 1, 0), Dir::Down, 1, Folded::none()),
            Pos::new(MAX_ROWS - 1, 0)
        );
    }

    #[test]
    fn g_and_shift_g_are_the_sheet_ends() {
        let extent = (10, 5);
        assert_eq!(
            moved(
                Pos::new(3, 3),
                Motion::SheetStart,
                extent,
                1,
                Folded::none()
            ),
            Pos::new(0, 0)
        );
        assert_eq!(
            moved(Pos::new(3, 3), Motion::SheetEnd, extent, 1, Folded::none()),
            Pos::new(9, 4)
        );
    }

    #[test]
    fn row_start_and_end_use_the_used_extent() {
        let extent = (10, 5);
        assert_eq!(
            moved(Pos::new(2, 3), Motion::RowStart, extent, 1, Folded::none()),
            Pos::new(2, 0)
        );
        assert_eq!(
            moved(Pos::new(2, 3), Motion::RowEnd, extent, 1, Folded::none()),
            Pos::new(2, 4)
        );
    }

    #[test]
    fn paging_moves_a_screenful_and_stops_at_the_top() {
        assert_eq!(
            moved(
                Pos::new(1, 0),
                Motion::Page(Dir::Down),
                (100, 10),
                4,
                Folded::none()
            )
            .row,
            5
        );
        assert_eq!(
            moved(
                Pos::new(1, 0),
                Motion::Page(Dir::Up),
                (100, 10),
                4,
                Folded::none()
            )
            .row,
            0
        );
    }

    fn folded(rows: &[u32], cols: &[u32]) -> (HashSet<u32>, HashSet<u32>) {
        (
            rows.iter().copied().collect(),
            cols.iter().copied().collect(),
        )
    }

    /// **Every distance here is counted in tracks that are drawn.** A motion that stepped onto a
    /// folded row would put the cursor where nothing is on screen, and pressing `j` five times to
    /// move one row reads as a terminal dropping keystrokes rather than as a cursor on a hidden
    /// row — which is exactly what it was doing.
    #[test]
    fn a_motion_counts_only_the_tracks_the_document_shows() {
        let (rows, cols) = folded(&[2, 3, 4, 5, 6], &[1, 2, 3]);
        let folded = Folded {
            rows: &rows,
            cols: &cols,
        };
        let extent = (20, 10);
        let down =
            |from: u32| moved(Pos::new(from, 0), Motion::By(Dir::Down), extent, 1, folded).row;
        assert_eq!(down(0), 1);
        assert_eq!(down(1), 7, "over the whole folded run in one step");
        assert_eq!(down(7), 8);

        let up = |from: u32| moved(Pos::new(from, 0), Motion::By(Dir::Up), extent, 1, folded).row;
        assert_eq!(up(7), 1, "and back over it");
        assert_eq!(up(1), 0);

        // The same on the other axis, which `:hide` made reachable.
        let right = moved(Pos::new(0, 0), Motion::By(Dir::Right), extent, 1, folded);
        assert_eq!(right.col, 4);
        assert_eq!(
            moved(right, Motion::By(Dir::Left), extent, 1, folded).col,
            0
        );

        // A page is a screenful of rows somebody can *see*, so four of them from row 0 lands
        // past the run rather than inside it.
        assert_eq!(
            moved(Pos::new(0, 0), Motion::Page(Dir::Down), extent, 4, folded).row,
            9,
            "0 \u{2192} 1 \u{2192} 7 \u{2192} 8 \u{2192} 9, four steps over rows that show"
        );
    }

    /// An edge motion cannot land on a folded track either: `0` goes to the first column that
    /// shows, `$` and `G` back up to the last one.
    #[test]
    fn the_edge_motions_land_on_a_track_that_shows() {
        let (rows, cols) = folded(&[9], &[0, 1, 9]);
        let folded = Folded {
            rows: &rows,
            cols: &cols,
        };
        let extent = (10, 10);
        assert_eq!(
            moved(Pos::new(3, 5), Motion::RowStart, extent, 1, folded).col,
            2,
            "A and B are folded, so `0` is C"
        );
        assert_eq!(
            moved(Pos::new(3, 5), Motion::RowEnd, extent, 1, folded).col,
            8,
            "the last column is folded, so `$` is the one before it"
        );
        assert_eq!(
            moved(Pos::new(3, 5), Motion::SheetEnd, extent, 1, folded),
            Pos::new(8, 8)
        );
        assert_eq!(
            moved(Pos::new(3, 5), Motion::SheetStart, extent, 1, folded),
            Pos::new(0, 2)
        );
    }

    /// A sheet with nothing left to step onto stops where it was, rather than walking to the
    /// far edge or spinning.
    #[test]
    fn a_run_of_folded_tracks_with_no_end_stops_where_it_started() {
        let all: HashSet<u32> = (1..MAX_ROWS).collect();
        let none = HashSet::new();
        let folded = Folded {
            rows: &all,
            cols: &none,
        };
        assert_eq!(
            moved(Pos::new(0, 0), Motion::By(Dir::Down), (10, 10), 1, folded).row,
            0
        );
    }

    #[test]
    fn an_empty_sheet_navigates_without_underflowing() {
        let extent = (0, 0);
        for motion in [Motion::SheetEnd, Motion::RowEnd] {
            assert_eq!(
                moved(Pos::new(0, 0), motion, extent, 1, Folded::none()),
                Pos::new(0, 0)
            );
        }
    }

    /// The keys a *range* needs, and the rule that keeps them out of the way until there is
    /// one — the same rule the word processor's map follows, on the same two markers.
    #[test]
    fn the_formatting_keys_only_bind_once_something_is_selected() {
        for (ch, action) in [
            ('*', Action::Bold),
            ('/', Action::Italic),
            ('-', Action::Plain),
        ] {
            assert_eq!(
                normal_action(KeyCode::Char(ch), KeyModifiers::NONE, true),
                Some(action),
                "{ch}"
            );
            assert_eq!(
                normal_action(KeyCode::Char(ch), KeyModifiers::NONE, false),
                None,
                "{ch} means nothing without a selection"
            );
        }
        for visual in [false, true] {
            assert_eq!(
                normal_action(KeyCode::Char('v'), KeyModifiers::NONE, visual),
                Some(Action::Visual)
            );
            assert_eq!(
                normal_action(KeyCode::Char('y'), KeyModifiers::NONE, visual),
                Some(Action::Yank)
            );
        }
    }
}
