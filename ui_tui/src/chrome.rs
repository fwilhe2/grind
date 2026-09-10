// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The chrome both halves wear: a **title bar** at the top, a **status bar** at the bottom, and
//! the mode chip that says which of vi's modes is on.
//!
//! [`crate::app`] says the grid and the flow have no rendering in common and does not invent a
//! widget for them. This is the exception that proves it rather than a hole in it: a bar is not a
//! document, it is the frame round one, and both halves frame the same four facts — which
//! document is open, whether it has unsaved changes, which mode the keyboard is in, and where the
//! cursor is. Two spellings of that would drift the way two key lists drift
//! (`crate::help`), so there is one.
//!
//! **Named colours, never hex** — `crate::code`'s rule, and for the same reason: the reader chose
//! a sixteen-colour palette and this is not the window that ignores it. A bar *does* paint its own
//! ground, which is not a contradiction: the ground it paints is the reader's own `Blue`, and the
//! document area between the two bars is left exactly as the terminal had it.
//!
//! **Two rows is what the chrome costs**, out of a window that may only have twenty-four. That is
//! paid for by what the rows carry that nothing else could: the sheet strip is this shell's only
//! answer to "which sheets are there" (`:sheet` needs the name before it can go to it), and the
//! heading path is the only thing on screen that says where in a long document the caret is.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// Which of vi's modes a shell is in — the one thing a modal editor must never leave a reader
/// guessing about.
///
/// Neither half's own `Mode` enum: those carry a mode's *state* (the edit buffer, the command
/// line), and this carries only what a bar says about it. Each half maps its own onto this in one
/// line, which is cheaper than teaching this module about a `Vec<char>` it would never read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    Command,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::Command => "COMMAND",
        }
    }

    /// The colour that mode is worn in. Four hues a reader can tell apart at a glance without
    /// reading the word — which is the point of a chip over a bare label.
    pub fn color(self) -> Color {
        match self {
            Mode::Normal => Color::Blue,
            Mode::Insert => Color::Green,
            Mode::Visual => Color::Magenta,
            Mode::Command => Color::Yellow,
        }
    }

    /// The ink drawn *on* that colour. Black on the two bright grounds, white on the two dark
    /// ones — the only contrast decision in this file, and it is made once here rather than at
    /// each of the four call sites.
    fn ink(self) -> Color {
        match self {
            Mode::Insert | Mode::Command => Color::Black,
            Mode::Normal | Mode::Visual => Color::White,
        }
    }

    /// The chip itself.
    pub fn chip(self) -> Span<'static> {
        Span::styled(
            format!(" {} ", self.label()),
            Style::default()
                .fg(self.ink())
                .bg(self.color())
                .add_modifier(Modifier::BOLD),
        )
    }
}

/// The title bar's ground — the reader's own blue, which is the one colour every terminal theme
/// has and every terminal theme makes readable under white.
pub fn title_style() -> Style {
    Style::default().bg(Color::Blue).fg(Color::White)
}

/// The status bar's ground, unchanged from what this shell has always drawn: black on grey, so a
/// message stays the most legible thing on screen.
pub fn status_style() -> Style {
    Style::default().bg(Color::Gray).fg(Color::Black)
}

/// Quieter text on the status bar — a hint, a position, a count. Grey rather than
/// [`Modifier::DIM`], because SGR 2 is optional and a bar that vanishes on half the terminals in
/// the world is not a bar (`doc/tui-shell.md` makes the same call for a `` `code` `` run and
/// records what it costs).
pub fn muted() -> Style {
    Style::default().bg(Color::Gray).fg(Color::DarkGray)
}

/// A badge at the very left of the title bar: which document type this window is holding.
///
/// One binary opens both (R10), and the two halves look nothing alike, so this is belt and braces
/// rather than information — but it is also where the eye lands first, and a bar that starts with
/// a coloured word reads as a bar rather than as a line of text that happens to be inverted.
pub fn badge(what: &str) -> Span<'static> {
    Span::styled(
        format!(" {what} "),
        Style::default()
            .fg(Color::Blue)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
}

/// What the title bar calls a file: its name, not its path.
///
/// A path can be longer than the window, and a bar filled with `/home/…/scratch/2026/` has pushed
/// out the two things beside it that a reader cannot get anywhere else — the sheet strip and the
/// heading path. The whole path is still what `:w` reports, what the code and problems panes are
/// titled with, and what the shell was invoked with; this is the tab title.
pub fn file_name(path: Option<&std::path::Path>) -> String {
    match path {
        Some(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        None => "untitled".to_owned(),
    }
}

/// The document's name, and a marker when it has changes that are not on disk.
///
/// `can_undo` is what "modified" means here — the same question `:q` asks before refusing to quit,
/// so the marker and the refusal can never disagree about whether there is work to lose.
pub fn document(name: &str, modified: bool) -> Vec<Span<'static>> {
    let mut out = vec![Span::styled(
        format!(" {name}"),
        title_style().add_modifier(Modifier::BOLD),
    )];
    if modified {
        out.push(Span::styled(
            " \u{25cf}",
            title_style().fg(Color::LightYellow),
        ));
    }
    out
}

/// A strip of tabs, with `active` picked out — the sheet tab bar every other shell has and this
/// one had no room for until there was a title bar to put it in.
///
/// Windowed round the active tab when they do not all fit, with `\u{2039}` / `\u{203a}` saying
/// which way the rest are. A strip that simply ran off the edge would hide exactly the tab a
/// reader of a wide workbook is looking for.
pub fn tabs(names: &[String], active: usize, room: usize) -> Vec<Span<'static>> {
    if names.is_empty() {
        return Vec::new();
    }
    let active = active.min(names.len() - 1);
    let label = |index: usize| format!(" {} ", names[index]);
    // Grow outwards from the active tab for as long as the next one on either side fits, taking
    // the left first so a strip that fits entirely reads in document order from its own start.
    let mut first = active;
    let mut last = active;
    let mut used = label(active).width();
    loop {
        let mut grew = false;
        if first > 0 {
            let want = label(first - 1).width();
            if used + want + 2 <= room {
                used += want;
                first -= 1;
                grew = true;
            }
        }
        if last + 1 < names.len() {
            let want = label(last + 1).width();
            if used + want + 2 <= room {
                used += want;
                last += 1;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    let mut out = Vec::new();
    if first > 0 {
        out.push(Span::styled("\u{2039}", title_style()));
    }
    for index in first..=last {
        out.push(Span::styled(
            label(index),
            match index == active {
                true => Style::default()
                    .fg(Color::Blue)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
                false => title_style().fg(Color::Gray),
            },
        ));
    }
    if last + 1 < names.len() {
        out.push(Span::styled("\u{203a}", title_style()));
    }
    out
}

/// A bar: `left` at one end, `right` at the other, `ground` in between, exactly `width` cells
/// wide.
///
/// Measured in **terminal cells** rather than in `char`s — `Span::width` is `unicode-width`'s
/// answer, the same one [`crate::text::Cells`] gives the layout engine, so a bar holding a CJK
/// file name lines up with the one holding an ASCII one.
///
/// When the two ends will not both fit, the **right one yields**: it carries a position or a
/// count, and the left carries what the reader asked for — a file name, a message, the mode. A
/// left that is still too long is cut with an ellipsis rather than allowed to wrap, since a bar
/// that became two rows would move the document under it.
pub fn bar(
    width: u16,
    ground: Style,
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
) -> Line<'static> {
    let width = usize::from(width);
    let sum = |spans: &[Span<'static>]| spans.iter().map(|span| span.width()).sum::<usize>();
    let mut left = left;
    let mut right = right;
    if sum(&left) + sum(&right) > width {
        right.clear();
    }
    let mut used = sum(&left);
    if used > width {
        // Truncate span by span from the right, leaving room for the ellipsis.
        let room = width.saturating_sub(1);
        let mut kept = Vec::new();
        let mut so_far = 0;
        for span in left {
            if so_far + span.width() <= room {
                so_far += span.width();
                kept.push(span);
                continue;
            }
            let style = span.style;
            let text: String = span
                .content
                .chars()
                .take_while(|c| {
                    let fits = so_far + c.to_string().width() <= room;
                    if fits {
                        so_far += c.to_string().width();
                    }
                    fits
                })
                .collect();
            if !text.is_empty() {
                kept.push(Span::styled(text, style));
            }
            break;
        }
        kept.push(Span::styled("\u{2026}", ground));
        left = kept;
        used = so_far + 1;
    }
    let filler = width.saturating_sub(used + sum(&right));
    let mut spans = left;
    spans.push(Span::styled(" ".repeat(filler), ground));
    spans.extend(right);
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn a_bar_is_exactly_as_wide_as_it_was_asked_for() {
        for width in [1u16, 8, 20, 80] {
            let line = bar(
                width,
                status_style(),
                vec![Span::raw("left side")],
                vec![Span::raw("right")],
            );
            assert_eq!(text(&line).width(), usize::from(width), "width {width}");
        }
    }

    /// The end that yields is the right one, and a left that still will not fit is cut rather
    /// than allowed to wrap the bar onto a second row.
    #[test]
    fn the_right_end_yields_first_and_the_left_is_cut_last() {
        let line = bar(
            12,
            status_style(),
            vec![Span::raw("a message")],
            vec![Span::raw("R1C1")],
        );
        assert_eq!(text(&line), "a message   ", "the right end went");

        let line = bar(6, status_style(), vec![Span::raw("a long message")], vec![]);
        assert_eq!(text(&line), "a lon\u{2026}");
    }

    /// A CJK name is measured in cells, not in `char`s — the same answer `Cells` gives the
    /// layout engine, so the two halves of this shell agree about how wide text is.
    #[test]
    fn a_wide_character_takes_two_cells_of_the_bar() {
        let line = bar(
            10,
            title_style(),
            vec![Span::raw("\u{4e16}\u{754c}")],
            vec![],
        );
        assert_eq!(text(&line).width(), 10);
        assert_eq!(text(&line), "\u{4e16}\u{754c}      ");
    }

    #[test]
    fn every_mode_has_its_own_colour_and_says_its_own_name() {
        let modes = [Mode::Normal, Mode::Insert, Mode::Visual, Mode::Command];
        for (i, mode) in modes.iter().enumerate() {
            for other in &modes[i + 1..] {
                assert_ne!(mode.color(), other.color(), "{mode:?} vs {other:?}");
                assert_ne!(mode.label(), other.label());
            }
            assert!(mode.chip().content.contains(mode.label()));
        }
    }

    /// The strip windows round the active tab rather than running off the edge — otherwise the
    /// one tab a reader is looking for is the one that is never on screen.
    #[test]
    fn a_tab_strip_too_long_to_fit_keeps_the_active_tab_in_it() {
        let names: Vec<String> = (1..=9).map(|n| format!("Sheet{n}")).collect();
        let shown = |active: usize| -> String {
            tabs(&names, active, 24)
                .iter()
                .map(|s| s.content.as_ref())
                .collect()
        };
        assert!(shown(0).contains("Sheet1"));
        assert!(shown(8).contains("Sheet9"), "{}", shown(8));
        assert!(shown(8).starts_with('\u{2039}'), "and says there is more");
        assert!(shown(0).ends_with('\u{203a}'));
        for active in 0..names.len() {
            assert!(
                shown(active).width() <= 24,
                "active {active}: {}",
                shown(active)
            );
        }
        assert!(tabs(&[], 0, 20).is_empty());
    }

    /// One sheet is still a strip, and it fits whatever it is called.
    #[test]
    fn a_single_tab_is_drawn_even_when_it_barely_fits() {
        let names = vec!["Quarterly Results 2026".to_owned()];
        let spans = tabs(&names, 0, 8);
        assert_eq!(spans.len(), 1, "no arrows: there is nothing either side");
        assert!(spans[0].content.contains("Quarterly"));
    }

    /// A bar filled with a path has pushed out the two things beside it a reader cannot get
    /// anywhere else.
    #[test]
    fn the_title_bar_calls_a_file_by_its_name_and_not_its_path() {
        assert_eq!(
            file_name(Some(std::path::Path::new("/home/me/work/2026/book.fods"))),
            "book.fods"
        );
        assert_eq!(
            file_name(Some(std::path::Path::new("book.fods"))),
            "book.fods"
        );
        assert_eq!(file_name(None), "untitled");
    }

    #[test]
    fn an_unsaved_document_wears_a_marker_and_a_saved_one_does_not() {
        let with: String = document("book.fods", true)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        let without: String = document("book.fods", false)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(with.contains('\u{25cf}'));
        assert!(!without.contains('\u{25cf}'));
        assert!(without.contains("book.fods"));
    }
}
