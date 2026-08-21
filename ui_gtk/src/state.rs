// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a keystroke means *while editing* — the modes, and the one function that decides.
//!
//! Pure, like [`crate::keymap`], and for the same reason: this is the part of a spreadsheet
//! that everyone has an opinion about, everyone notices when it is wrong, and no display is
//! needed to check. The widget turns the answer into GTK calls.
//!
//! The modes are Excel's, by name (doc/gtk-shell.md):
//!
//! * **Ready** — the grid has the keys. A printable character starts an edit seeded with it,
//!   F2 and a double-click start one seeded with the cell, everything else navigates.
//! * **Enter** — an edit that began by typing. An arrow key *commits* and moves, because
//!   the caret has nowhere useful to go in text the user has only just started.
//! * **Edit** — an edit that began from the cell's own content (F2, double-click). An arrow
//!   key moves the caret instead, because the text is worth navigating. F2 toggles.
//!
//! That difference between Enter and Edit is the whole reason two modes exist, and typing
//! `=` then pressing → is the case it exists for: in M6 that becomes Point mode and starts
//! building a reference, which is why the mode lives here rather than as a bool in the
//! widget.
//!
//! **Point** is not a fourth mode but a predicate. [`ref_eligible`] asks whether a reference
//! could go where the caret is — the text starts `=` and the last thing before the caret is
//! an operator, a `(`, a `;` or nothing — and while a pending reference exists the arrows
//! keep moving it. That is why the mode enum has three variants and the behaviour has four:
//! "am I pointing" is a question about the *text*, and a flag would go stale the moment
//! someone moved the caret.

use std::ops::Range;

use sheet_core::formula::display::{self, TokenKind};
use sheet_core::{Pos, a1};

use crate::keymap::{self, Dir, Key, Mods, Motion};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Ready,
    Enter,
    Edit,
}

impl Mode {
    pub fn is_editing(self) -> bool {
        !matches!(self, Mode::Ready)
    }
}

/// What an edit starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    /// Typing over a cell replaces it — the character is the whole new content.
    Char(char),
    /// F2 or a double-click: the cell's own text, ready to be amended.
    Cell,
}

/// What the widget should do about a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not ours. The event keeps travelling, which is what leaves the editor's own key
    /// handling — selection, caret, input method — untouched.
    Passthrough,
    /// Move the selection; Ready mode only.
    Navigate(keymap::Action),
    Begin(Seed),
    /// Store what the editor holds, then move.
    Commit(Option<Dir>),
    /// Throw the edit away. The document is not touched.
    Cancel,
    /// Empty the selection.
    Clear,
    /// F2 while editing: Enter ↔ Edit.
    ToggleMode,
    /// Start or move a **pending reference**: the shell moves the pointed selection and
    /// writes the reference it names into the buffer, replacing the last one it wrote.
    Point {
        motion: Motion,
        extend: bool,
    },
    /// F4: cycle the `$` markers of the reference being pointed at, or the one under the
    /// caret.
    CycleAbsolute,
}

/// Where the caret is and what is around it — what deciding a keystroke needs beyond the
/// key itself.
#[derive(Clone, Copy, Debug)]
pub struct Where<'a> {
    pub mode: Mode,
    pub text: &'a str,
    /// Byte offset, as `display::spans` counts.
    pub caret: usize,
    /// Whether a reference is currently being pointed at.
    pub pending: bool,
}

/// The whole state machine. One `match`, and no memory beyond what it is told.
pub fn on_key(at: Where, key: Key, mods: Mods) -> Outcome {
    match at.mode {
        Mode::Ready => ready(key, mods),
        Mode::Enter | Mode::Edit => editing(at, key, mods),
    }
}

fn ready(key: Key, mods: Mods) -> Outcome {
    match key {
        // Ctrl and Alt combinations belong to the window's actions and its menus; claiming
        // them here is how a shortcut ends up doing two things.
        _ if mods.ctrl || mods.alt => navigate(key, mods),
        Key::F2 => Outcome::Begin(Seed::Cell),
        Key::Delete | Key::Backspace => Outcome::Clear,
        // A printable character starts an edit and *is* the edit's first character. Control
        // characters reach here as themselves — Escape is `\u{1b}` — and are not printable.
        Key::Char(c) if !c.is_control() => Outcome::Begin(Seed::Char(c)),
        _ => navigate(key, mods),
    }
}

fn navigate(key: Key, mods: Mods) -> Outcome {
    keymap::action_for(key, mods).map_or(Outcome::Passthrough, Outcome::Navigate)
}

fn editing(at: Where, key: Key, mods: Mods) -> Outcome {
    let reverse = |a, b| match mods.shift {
        true => a,
        false => b,
    };
    // These end an edit whatever else is going on, which is why they are decided first: Tab
    // and Return are motions in the keymap, and pointing must not swallow them.
    match key {
        Key::Escape => return Outcome::Cancel,
        Key::F2 => return Outcome::ToggleMode,
        Key::F4 => return Outcome::CycleAbsolute,
        Key::Return if !mods.ctrl => return Outcome::Commit(Some(reverse(Dir::Up, Dir::Down))),
        Key::Tab if !mods.ctrl => return Outcome::Commit(Some(reverse(Dir::Left, Dir::Right))),
        _ => {}
    }

    // Pointing borrows the keymap's motion vocabulary rather than growing a second one, so
    // Ctrl+arrow points at a data edge and PgDn points a screenful down, for free.
    let motion = match keymap::action_for(key, mods) {
        Some(keymap::Action::Move { motion, extend }) => Some((motion, extend)),
        _ => None,
    };
    match motion {
        // Already pointing: keep pointing, whatever the mode. The caret is inside the
        // reference being built, so there is nowhere else for an arrow to go.
        Some((motion, extend)) if at.pending => Outcome::Point { motion, extend },
        // Typing, and a reference could go here: this is the transition the mode exists
        // for — `=SUM(` then → starts a reference rather than committing.
        Some((motion, extend)) if at.mode == Mode::Enter && ref_eligible(at.text, at.caret) => {
            Outcome::Point { motion, extend }
        }
        // Typing, and it could not: an arrow commits and moves, as it does in every
        // spreadsheet.
        Some((Motion::By(dir), _)) if at.mode == Mode::Enter => Outcome::Commit(Some(dir)),
        // Amending: the caret moves through the text, and never points (F2 toggles).
        _ => Outcome::Passthrough,
    }
}

/// Whether a reference may be inserted at `caret` — the predicate Point mode is.
///
/// A formula, and the last thing before the caret is something a reference can follow: an
/// operator, a separator, an opening parenthesis, or the `=` itself. `SUM(B2` is *not*
/// eligible — an arrow there moves on rather than pointing at a second cell, which is what
/// stops a half-typed reference from being extended by accident.
pub fn ref_eligible(text: &str, caret: usize) -> bool {
    if !text.starts_with('=') {
        return false;
    }
    let before = &text[..caret.min(text.len())];
    match before.trim_end().chars().next_back() {
        Some(c) => "=(;+-*/^&<>:,".contains(c),
        // Nothing but the `=` and whitespace.
        None => false,
    }
}

/// F4: the reference to re-spell, and what to spell it as.
///
/// Excel's cycle, because everybody's fingers know it: `B2` → `$B$2` → `B$2` → `$B2` → `B2`.
/// The reference is the one the caret is in or at the end of, found with `display::spans` —
/// the same scanner that colours them, so what F4 acts on is what the user sees highlighted.
pub fn cycle_absolute(text: &str, caret: usize) -> Option<(Range<usize>, String)> {
    let span = display::spans(text).into_iter().find(|span| {
        span.kind == TokenKind::Ref && span.range.start <= caret && caret <= span.range.end
    })?;
    let mut reference = a1::parse(&text[span.range.clone()]).ok()?;
    // Both axes of both ends move together, which is what the four steps mean.
    let (col, row) = reference
        .start
        .col
        .zip(reference.start.row)
        .map(|(c, r)| (c.absolute, r.absolute))
        .unwrap_or((false, false));
    let (col, row) = match (col, row) {
        (false, false) => (true, true),
        (true, true) => (false, true),
        (false, true) => (true, false),
        _ => (false, false),
    };
    for end in [Some(&mut reference.start), reference.end.as_mut()]
        .into_iter()
        .flatten()
    {
        if let Some(axis) = end.col.as_mut() {
            axis.absolute = col;
        }
        if let Some(axis) = end.row.as_mut() {
            axis.absolute = row;
        }
    }
    Some((span.range, display::reference_text(&reference)))
}

/// Which call the caret is inside, and which argument of it — what a signature hint shows.
///
/// Scanned backwards over the display text, counting parentheses and skipping string
/// literals, because the caret is where the user is and the call that matters is the
/// innermost one containing it.
pub fn call_at(text: &str, caret: usize) -> Option<(String, usize)> {
    let head: Vec<char> = text[..caret.min(text.len())].chars().collect();
    let mut depth = 0i32;
    let mut argument = 0usize;
    let mut i = head.len();
    let mut in_string = false;
    while i > 0 {
        i -= 1;
        match head[i] {
            // Quotes are counted from the left, so a backwards scan flips at every one.
            '"' => in_string = !in_string,
            _ if in_string => {}
            ')' => depth += 1,
            ';' if depth == 0 => argument += 1,
            '(' if depth > 0 => depth -= 1,
            '(' => {
                // The name in front of it, if there is one — otherwise this was a grouping
                // parenthesis and the call, if any, is further out.
                let end = i;
                while i > 0 && is_name_char(head[i - 1]) {
                    i -= 1;
                }
                if i == end {
                    argument = 0;
                    continue;
                }
                return Some((head[i..end].iter().collect(), argument));
            }
            _ => {}
        }
    }
    None
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// Where the cursor goes after a commit, and what to remember for the next one.
///
/// **Tab-column memory**: Enter after a run of Tabs returns to the column the run started
/// in, which is how a person fills a table row by row. One remembered integer, and
/// disproportionately loved by anyone who has typed a table.
pub fn after_commit(
    from: Pos,
    direction: Option<Dir>,
    tab_origin: Option<u32>,
) -> (Pos, Option<u32>) {
    match direction {
        Some(Dir::Right) => (
            Pos::new(from.row, from.col.saturating_add(1)),
            Some(tab_origin.unwrap_or(from.col)),
        ),
        Some(Dir::Left) => (
            Pos::new(from.row, from.col.saturating_sub(1)),
            Some(tab_origin.unwrap_or(from.col)),
        ),
        Some(Dir::Down) => (
            Pos::new(from.row.saturating_add(1), tab_origin.unwrap_or(from.col)),
            None,
        ),
        Some(Dir::Up) => (
            Pos::new(from.row.saturating_sub(1), tab_origin.unwrap_or(from.col)),
            None,
        ),
        None => (from, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Motion;

    fn plain() -> Mods {
        Mods::default()
    }

    fn at(mode: Mode, text: &str) -> Where<'_> {
        Where {
            mode,
            text,
            caret: text.len(),
            pending: false,
        }
    }

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Default::default()
        }
    }

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Default::default()
        }
    }

    #[test]
    fn typing_starts_an_edit_that_replaces_the_cell() {
        assert_eq!(
            on_key(at(Mode::Ready, ""), Key::Char('7'), plain()),
            Outcome::Begin(Seed::Char('7'))
        );
        assert_eq!(
            on_key(at(Mode::Ready, ""), Key::F2, plain()),
            Outcome::Begin(Seed::Cell)
        );
        // A shortcut is not typing, and must not start an edit with a stray character.
        assert_eq!(
            on_key(at(Mode::Ready, ""), Key::Char('s'), ctrl()),
            Outcome::Passthrough
        );
        assert_eq!(
            on_key(at(Mode::Ready, ""), Key::Char('a'), ctrl()),
            Outcome::Navigate(keymap::Action::SelectAll)
        );
    }

    #[test]
    fn ready_mode_still_navigates() {
        assert_eq!(
            on_key(at(Mode::Ready, ""), Key::Down, plain()),
            Outcome::Navigate(keymap::Action::Move {
                motion: Motion::By(Dir::Down),
                extend: false
            })
        );
        assert_eq!(
            on_key(at(Mode::Ready, ""), Key::Delete, plain()),
            Outcome::Clear
        );
    }

    /// The reason there are two editing modes at all.
    #[test]
    fn an_arrow_commits_while_typing_and_moves_the_caret_while_amending() {
        assert_eq!(
            on_key(at(Mode::Enter, ""), Key::Right, plain()),
            Outcome::Commit(Some(Dir::Right))
        );
        assert_eq!(
            on_key(at(Mode::Edit, ""), Key::Right, plain()),
            Outcome::Passthrough
        );
        assert_eq!(
            on_key(at(Mode::Edit, ""), Key::F2, plain()),
            Outcome::ToggleMode
        );
    }

    #[test]
    fn enter_and_tab_commit_and_shift_reverses_them() {
        for mode in [Mode::Enter, Mode::Edit] {
            assert_eq!(
                on_key(at(mode, ""), Key::Return, plain()),
                Outcome::Commit(Some(Dir::Down))
            );
            assert_eq!(
                on_key(at(mode, ""), Key::Return, shift()),
                Outcome::Commit(Some(Dir::Up))
            );
            assert_eq!(
                on_key(at(mode, ""), Key::Tab, plain()),
                Outcome::Commit(Some(Dir::Right))
            );
            assert_eq!(
                on_key(at(mode, ""), Key::Tab, shift()),
                Outcome::Commit(Some(Dir::Left))
            );
            assert_eq!(on_key(at(mode, ""), Key::Escape, plain()), Outcome::Cancel);
        }
    }

    /// Everything the machine does not claim has to reach the editor, or typing stops
    /// working in the subtlest possible way.
    #[test]
    fn the_editor_keeps_every_key_this_does_not_claim() {
        for key in [Key::Char('x'), Key::Home, Key::End, Key::PageUp, Key::Other] {
            assert_eq!(
                on_key(at(Mode::Edit, ""), key, plain()),
                Outcome::Passthrough,
                "{key:?}"
            );
        }
    }

    /// The predicate Point mode is, in the four cases that decide whether an arrow key
    /// points or commits.
    #[test]
    fn a_reference_may_follow_an_operator_and_nothing_else() {
        for text in ["=", "=SUM(", "=1+", "=SUM(A1;", "=A1:"] {
            assert!(ref_eligible(text, text.len()), "{text}");
        }
        for text in ["", "7", "=SUM(B2", "=SUM(B2)", "text"] {
            assert!(!ref_eligible(text, text.len()), "{text}");
        }
        // Where the caret is, not where the text ends.
        assert!(ref_eligible("=SUM(B2)", 5));
    }

    /// The transition the whole milestone exists for: `=SUM(` then an arrow starts a
    /// reference, where the same key one character later commits.
    #[test]
    fn an_arrow_points_where_a_reference_could_go_and_commits_where_it_could_not() {
        let pointing = Where {
            mode: Mode::Enter,
            text: "=SUM(",
            caret: 5,
            pending: false,
        };
        assert_eq!(
            on_key(pointing, Key::Down, plain()),
            Outcome::Point {
                motion: Motion::By(Dir::Down),
                extend: false
            }
        );
        let typing = Where {
            text: "=SUM(B2",
            caret: 7,
            ..pointing
        };
        assert_eq!(
            on_key(typing, Key::Down, plain()),
            Outcome::Commit(Some(Dir::Down))
        );

        // Once a reference is pending, the arrows keep moving it whatever the text says.
        let moving = Where {
            pending: true,
            ..typing
        };
        assert_eq!(
            on_key(moving, Key::Right, shift()),
            Outcome::Point {
                motion: Motion::By(Dir::Right),
                extend: true
            }
        );
        // And Tab still commits: pointing must not swallow the keys that end an edit.
        assert_eq!(
            on_key(moving, Key::Tab, plain()),
            Outcome::Commit(Some(Dir::Right))
        );
    }

    /// Excel's cycle, because everybody's fingers know it.
    #[test]
    fn f4_walks_the_four_spellings_and_comes_back() {
        let mut text = "=SUM(B2:B4)".to_owned();
        let mut seen = Vec::new();
        for _ in 0..5 {
            let (span, replacement) = cycle_absolute(&text, 6).expect(&text);
            text.replace_range(span, &replacement);
            seen.push(replacement);
        }
        assert_eq!(
            seen,
            ["$B$2:$B$4", "B$2:B$4", "$B2:$B4", "B2:B4", "$B$2:$B$4"]
        );
    }

    #[test]
    fn f4_does_nothing_where_there_is_no_reference() {
        assert!(cycle_absolute("=SUM(1;2)", 6).is_none());
        assert!(cycle_absolute("hello", 2).is_none());
    }

    /// What a signature hint needs: which call the caret is in, and which argument.
    #[test]
    fn the_caret_knows_which_argument_it_is_in() {
        assert_eq!(call_at("=SUM(", 5), Some(("SUM".to_owned(), 0)));
        assert_eq!(call_at("=SUM(1;2", 8), Some(("SUM".to_owned(), 1)));
        assert_eq!(call_at("=SUM(1;2;3", 10), Some(("SUM".to_owned(), 2)));
        // The innermost call wins, and a closed one is not it.
        assert_eq!(call_at("=IF(A1;SUM(1;2", 14), Some(("SUM".to_owned(), 1)));
        assert_eq!(call_at("=IF(A1;SUM(1;2);", 16), Some(("IF".to_owned(), 2)));
        // A `;` inside a string is not an argument separator, and a bare group is no call.
        assert_eq!(call_at("=SUM(\"a;b\";2", 12), Some(("SUM".to_owned(), 1)));
        assert_eq!(call_at("=(1+2", 5), None);
        assert_eq!(call_at("=A1", 3), None);
        // Dollars are part of a reference, not of anything the scan cares about.
        assert_eq!(call_at("=SUM($A$13:$A$15", 16), Some(("SUM".to_owned(), 0)));
    }

    /// Tab-column memory: Enter after a run of Tabs returns to where the run began.
    #[test]
    fn enter_after_a_run_of_tabs_goes_back_to_the_first_column() {
        let mut at = Pos::new(0, 0);
        let mut origin = None;
        for _ in 0..3 {
            (at, origin) = after_commit(at, Some(Dir::Right), origin);
        }
        assert_eq!(at, Pos::new(0, 3));
        (at, origin) = after_commit(at, Some(Dir::Down), origin);
        assert_eq!(at, Pos::new(1, 0), "back to the column the run started in");
        assert_eq!(origin, None, "and the run is over");

        // A plain Enter with no run behind it just goes down.
        let (to, _) = after_commit(Pos::new(4, 2), Some(Dir::Down), None);
        assert_eq!(to, Pos::new(5, 2));
    }
}
