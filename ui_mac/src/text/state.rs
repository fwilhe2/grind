// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The page's caret, its selection, and an input method's composition — and every edit made at
//! them (M6). Portable, so each is tested here over a real `grind_text::App` and
//! [`grind_text::Fixed`].
//!
//! **The selection is presentation state**: an anchor and a caret, never told to the core. It
//! reaches `App` as two carets when something is done to it — erased, typed over, copied — the
//! arrangement every page in the suite has. The same holds for a **composition**: the marked
//! text an input method is still working on is held here and drawn by `text/paint.rs` through
//! `App::layout_composing`, and the document learns of it only when the input method commits.
//!
//! Every motion that is about lines is the core's (`App::caret_line`, `caret_line_bounds`,
//! through the page's `Faces`); every one about characters and words is `grind_text::caret`'s.
//! What is this shell's is the Mac's own reading of the keys, which `keys.rs` names: ⌥→ goes to
//! the end of a word, ← with a selection collapses it to its start, and ⌃K kills to the end of
//! the paragraph.

use std::ops::Range;

use grind_text::caret::{self, START};
use grind_text::style::CharStyle;
use grind_text::{App, Caret, Faces};

/// What a motion moves by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// One character, across a block boundary at either end.
    Char(i8),
    /// One word: to the end of one going right, the start of one going left.
    Word(i8),
    /// Lines down (positive) or up, by the laid-out line.
    Line(isize),
    /// Pages down or up — a screenful of lines, less one.
    Page(isize),
    /// The visual ends of the caret's own line — ⌘← and ⌘→.
    LineStart,
    LineEnd,
    /// The ends of the caret's block — ⌃A and ⌃E. Staying put at the end already reached.
    ParagraphStart,
    ParagraphEnd,
    /// The start of this block, or of the one before when already there — ⌥↑; and ⌥↓'s twin.
    ParagraphBack,
    ParagraphOn,
    /// ⌘↑ and ⌘↓.
    DocStart,
    DocEnd,
}

/// What an erase takes: a character, a word, to the line's end or the paragraph's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Char,
    Word,
    Line,
    Paragraph,
}

/// Text an input method has not committed yet: what it is, and which part of it the input
/// method has selected — the clause being converted, or where its own caret is — in characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composing {
    pub text: String,
    pub selected: Range<usize>,
}

/// The page's state between keystrokes.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub caret: Caret,
    /// Where a selection began; equal to `caret` when there is none.
    pub anchor: Caret,
    /// The x a run of vertical motions aims for — kept across them, dropped by anything else.
    pub goal_x: Option<f32>,
    /// What `App::type_markdown` said the next character must be set in — the style a closed
    /// `**span**` leaves behind, carried across keystrokes.
    pub resume: Option<CharStyle>,
    pub composing: Option<Composing>,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            caret: START,
            anchor: START,
            goal_x: None,
            resume: None,
            composing: None,
        }
    }
}

/// An edit the core refused, in its own words — the banner's sentence.
pub type Refused = String;

impl Page {
    /// The selection's two ends in document order, or `None` when nothing is selected.
    pub fn selection(&self) -> Option<(Caret, Caret)> {
        (self.anchor != self.caret)
            .then(|| (self.anchor.min(self.caret), self.anchor.max(self.caret)))
    }

    /// Put the caret at `at`, extending the selection or collapsing it, and forget the goal
    /// column and any style pending for the next character — which belongs to where it was
    /// pressed.
    pub fn place(&mut self, at: Caret, extend: bool) {
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
        self.goal_x = None;
        self.resume = None;
    }

    /// The caret put back somewhere real after the document changed under it — an undo, or
    /// another window's edit — clamped to the blocks and characters there now.
    pub fn clamp(&mut self, app: &App) {
        let clamp = |at: Caret| {
            let block = at.block.min(app.block_count().saturating_sub(1));
            Caret {
                block,
                offset: at.offset.min(caret::block_len(app, block)),
            }
        };
        self.caret = clamp(self.caret);
        self.anchor = clamp(self.anchor);
    }

    /// Where `motion` from the caret lands, without moving anything. `page` is how many lines a
    /// page is, which the view knows and this does not.
    pub fn moved(&self, app: &App, faces: &dyn Faces, motion: Motion, page: isize) -> Caret {
        let at = self.caret;
        let len = |block| caret::block_len(app, block);
        let vertical = |lines: isize| {
            let goal = self
                .goal_x
                .or_else(|| app.caret_x(at, faces).ok())
                .unwrap_or(0.0);
            app.caret_line(at, lines, goal, faces).unwrap_or(at)
        };
        match motion {
            Motion::Char(step) => caret::step(app, at, i32::from(step)),
            Motion::Word(step) => caret::word(app, at, i32::from(step)),
            Motion::Line(lines) => vertical(lines),
            Motion::Page(pages) => vertical(pages * page.max(1)),
            Motion::LineStart | Motion::LineEnd => match app.caret_line_bounds(at, faces) {
                Ok((start, _)) if motion == Motion::LineStart => start,
                Ok((_, end)) => end,
                Err(_) => at,
            },
            Motion::ParagraphStart => Caret { offset: 0, ..at },
            Motion::ParagraphEnd => Caret {
                offset: len(at.block),
                ..at
            },
            Motion::ParagraphBack => match at.offset {
                0 if at.block > 0 => Caret {
                    block: at.block - 1,
                    offset: 0,
                },
                _ => Caret { offset: 0, ..at },
            },
            Motion::ParagraphOn => match at.offset == len(at.block) {
                true if at.block + 1 < app.block_count() => Caret {
                    block: at.block + 1,
                    offset: len(at.block + 1),
                },
                _ => Caret {
                    offset: len(at.block),
                    ..at
                },
            },
            Motion::DocStart => START,
            Motion::DocEnd => caret::end(app),
        }
    }

    /// Move the caret by `motion`, extending the selection or collapsing it.
    ///
    /// **With a selection up and no Shift, ← and → collapse it** to its start or its end rather
    /// than moving from the caret — every Mac text view's rule. The goal column survives a run
    /// of vertical motions and nothing else.
    pub fn navigate(
        &mut self,
        app: &App,
        faces: &dyn Faces,
        motion: Motion,
        extend: bool,
        page: isize,
    ) {
        if let (Motion::Char(step), false, Some((from, to))) = (motion, extend, self.selection()) {
            self.place(if step < 0 { from } else { to }, false);
            return;
        }
        let vertical = matches!(motion, Motion::Line(_) | Motion::Page(_));
        let goal = match vertical {
            true => self.goal_x.or_else(|| app.caret_x(self.caret, faces).ok()),
            false => None,
        };
        let to = self.moved(app, faces, motion, page);
        self.place(to, extend);
        self.goal_x = goal;
    }

    /// Everything selected, the caret at the end.
    pub fn select_all(&mut self, app: &App) {
        self.anchor = START;
        self.caret = caret::end(app);
        self.goal_x = None;
    }

    /// The word around `at` selected — a double-click. `grind_text::word` owns what a word is.
    pub fn select_word(&mut self, app: &App, at: Caret) {
        let text = app.input_text(at.block).unwrap_or_default();
        let (start, end) = grind_text::word::around(&text, at.offset);
        self.anchor = Caret {
            block: at.block,
            offset: start,
        };
        self.caret = Caret {
            block: at.block,
            offset: end,
        };
        self.goal_x = None;
    }

    /// The whole block `at` is in — a triple-click.
    pub fn select_block(&mut self, app: &App, at: Caret) {
        self.anchor = Caret {
            block: at.block,
            offset: 0,
        };
        self.caret = Caret {
            block: at.block,
            offset: caret::block_len(app, at.block),
        };
        self.goal_x = None;
    }

    /// What is selected, as plain text — a block break between two blocks is a newline, which
    /// is what [`Page::paste`] reads back as one.
    pub fn selected_text(&self, app: &App) -> Option<String> {
        let (from, to) = self.selection()?;
        let viewport = app.get_viewport(from.block..to.block + 1);
        let pieces: Vec<String> = viewport
            .iter()
            .map(|view| {
                let start = if view.index == from.block {
                    from.offset
                } else {
                    0
                };
                let end = if view.index == to.block {
                    to.offset
                } else {
                    usize::MAX
                };
                view.text
                    .chars()
                    .skip(start)
                    .take(end.saturating_sub(start))
                    .collect()
            })
            .collect();
        Some(pieces.join("\n"))
    }

    /// Erase what is selected, leaving the caret where it began. `Ok(true)` when there was a
    /// selection — every edit starts with this, since typing over a selection replaces it.
    pub fn drop_selection(&mut self, app: &App) -> Result<bool, Refused> {
        let Some((from, to)) = self.selection() else {
            return Ok(false);
        };
        app.erase(from, to).map_err(|error| error.to_string())?;
        self.place(from, false);
        Ok(true)
    }

    /// Type `text` at the caret through `App::type_markdown`, so `**bold**` is read as it is
    /// typed — the notation every page in the suite reads, never a fifth idea of it. Replaces a
    /// selection, and a composition: this is what an input method's commit is.
    pub fn type_text(&mut self, app: &App, text: &str) -> Result<(), Refused> {
        self.composing = None;
        self.drop_selection(app)?;
        let resume = self.resume.take();
        let typed = app
            .type_markdown(self.caret, text, resume.as_ref())
            .map_err(|error| error.to_string())?;
        self.place(typed.caret, false);
        self.resume = typed.resume;
        Ok(())
    }

    /// Insert `text` as it is — no notation read — which is what a line break and a literal tab
    /// are, since `**` in a pasted sentence was typed by somebody else.
    pub fn insert_plain(&mut self, app: &App, text: &str) -> Result<(), Refused> {
        self.drop_selection(app)?;
        let at = self.caret;
        app.insert_text(at, text)
            .map_err(|error| error.to_string())?;
        self.place(
            Caret {
                block: at.block,
                offset: at.offset + text.chars().count(),
            },
            false,
        );
        Ok(())
    }

    /// ⌫ and its relatives: what `unit` takes going `forward` or back. A selection is erased
    /// instead, whatever the unit. At a block's edge the character taken is the **break**
    /// between two blocks, which is `erase` across the boundary — one verb and one undo step.
    pub fn erase(
        &mut self,
        app: &App,
        faces: &dyn Faces,
        unit: Unit,
        forward: bool,
    ) -> Result<(), Refused> {
        if self.drop_selection(app)? {
            return Ok(());
        }
        let at = self.caret;
        let step = if forward { 1 } else { -1 };
        let other = match unit {
            Unit::Char => caret::step(app, at, step),
            Unit::Word => caret::word(app, at, step),
            Unit::Line => match app.caret_line_bounds(at, faces) {
                Ok((start, end)) => {
                    let edge = if forward { end } else { start };
                    // At the line's own edge already: the break before or after it, which is
                    // what a second ⌘⌫ at the front of a line does.
                    if edge == at {
                        caret::step(app, at, step)
                    } else {
                        edge
                    }
                }
                Err(_) => at,
            },
            Unit::Paragraph => {
                let edge = Caret {
                    offset: if forward {
                        caret::block_len(app, at.block)
                    } else {
                        0
                    },
                    ..at
                };
                // ⌃K at the end of a paragraph joins the next one, as Emacs's kill does.
                if edge == at {
                    caret::step(app, at, step)
                } else {
                    edge
                }
            }
        };
        if other == at {
            // The very start or end of the document: nothing to erase.
            return Ok(());
        }
        let (from, to) = (at.min(other), at.max(other));
        app.erase(from, to).map_err(|error| error.to_string())?;
        self.place(from, false);
        Ok(())
    }

    /// Return: one block becomes two, and the caret goes to the front of the second.
    pub fn split(&mut self, app: &App) -> Result<(), Refused> {
        self.drop_selection(app)?;
        let at = self.caret;
        app.split_block(at).map_err(|error| error.to_string())?;
        self.place(
            Caret {
                block: at.block + 1,
                offset: 0,
            },
            false,
        );
        Ok(())
    }

    /// Tab and ⇧Tab: structural where `grind_text::indent_kind` says there is structure to
    /// change — nesting a list item, un-nesting one, starting a list at the front of a block —
    /// and a literal `text:tab` otherwise.
    pub fn tab(&mut self, app: &App, back: bool) -> Result<(), Refused> {
        let index = self.caret.block;
        let kind = app
            .get_viewport(index..index + 1)
            .get(index)
            .map(|view| view.kind.clone());
        let by = if back { -1 } else { 1 };
        match kind.and_then(|kind| grind_text::indent_kind(&kind, self.caret.offset, by)) {
            Some(kind) => app.set_kind(index, kind).map_err(|error| error.to_string()),
            None if !back => self.insert_plain(app, "\t"),
            None => Ok(()),
        }
    }

    /// Paste `text` at the caret, replacing the selection. A newline in it — `\r\n` or `\n` —
    /// becomes a block break, so a paragraph of prose pastes as paragraphs.
    ///
    /// ponytail: one `split_block` and one `insert_text` per line, so a pasted paragraph is as
    /// many ⌘Z as it has pieces. `App` has no multi-block insert, and adding one is a core
    /// change; the Windows pane pastes the same way and names the same cost. The trigger is a
    /// third page that pastes.
    pub fn paste(&mut self, app: &App, text: &str) -> Result<(), Refused> {
        self.composing = None;
        self.drop_selection(app)?;
        for (index, line) in text
            .split('\n')
            .map(|line| line.trim_end_matches('\r'))
            .enumerate()
        {
            if index > 0 {
                self.split(app)?;
            }
            if !line.is_empty() {
                self.insert_plain(app, line)?;
            }
        }
        Ok(())
    }

    /// An input method's marked text: `text`, with `selected` of it selected, in characters.
    /// Empty text ends the composition with nothing committed — what an input method does when
    /// its composition is backspaced away.
    ///
    /// A selection is erased when a composition *starts*, since what is composed replaces it —
    /// the same rule as typing, taken one keystroke early because the input method has already
    /// begun.
    pub fn mark(&mut self, app: &App, text: &str, selected: Range<usize>) -> Result<(), Refused> {
        if text.is_empty() {
            self.composing = None;
            return Ok(());
        }
        if self.composing.is_none() {
            self.drop_selection(app)?;
        }
        let len = text.chars().count();
        self.composing = Some(Composing {
            text: text.to_owned(),
            selected: selected.start.min(len)..selected.end.min(len),
        });
        Ok(())
    }

    /// `unmarkText`: the input method is done and the marked text stands as it is.
    pub fn unmark(&mut self, app: &App) -> Result<(), Refused> {
        match self.composing.take() {
            Some(composing) => self.type_text(app, &composing.text),
            None => Ok(()),
        }
    }

    /// Where the caret is drawn: inside the composition at the input method's own caret when
    /// there is one, and at [`Page::caret`] otherwise.
    pub fn shown_caret(&self) -> Caret {
        match &self.composing {
            Some(composing) => Caret {
                offset: self.caret.offset + composing.selected.end,
                ..self.caret
            },
            None => self.caret,
        }
    }

    /// Undo or redo, and the caret put back somewhere real afterwards.
    pub fn history(&mut self, app: &App, undo: bool) -> bool {
        self.composing = None;
        let done = if undo { app.undo() } else { app.redo() };
        self.clamp(app);
        self.anchor = self.caret;
        self.resume = None;
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::{BlockKind, Fixed, Uniform};

    fn app(blocks: &[&str]) -> App {
        let app = App::new();
        for (index, text) in blocks.iter().enumerate() {
            match index {
                0 => app.set_text(0, text).unwrap(),
                _ => app.insert(index, BlockKind::Paragraph, text).unwrap(),
            }
        }
        app
    }

    fn at(block: usize, offset: usize) -> Caret {
        Caret { block, offset }
    }

    fn texts(app: &App) -> Vec<String> {
        app.get_viewport(0..app.block_count())
            .iter()
            .map(|view| view.text.clone())
            .collect()
    }

    /// Ten characters a line, one unit a line.
    fn faces() -> Uniform<'static> {
        Uniform::new(10.0, &Fixed)
    }

    #[test]
    fn a_selection_is_the_two_ends_in_order_or_nothing() {
        let mut page = Page::default();
        assert_eq!(page.selection(), None);
        page.caret = at(0, 5);
        page.anchor = at(1, 2);
        assert_eq!(page.selection(), Some((at(0, 5), at(1, 2))));
    }

    /// ← and → with a selection collapse it rather than moving; with Shift they extend.
    #[test]
    fn an_arrow_collapses_a_selection_to_its_end_on_that_side() {
        let app = app(&["hello world"]);
        let mut page = Page::default();
        page.place(at(0, 2), false);
        page.navigate(&app, &faces(), Motion::Char(1), true, 5);
        page.navigate(&app, &faces(), Motion::Char(1), true, 5);
        assert_eq!(page.selection(), Some((at(0, 2), at(0, 4))));
        let mut left = page.clone();
        left.navigate(&app, &faces(), Motion::Char(-1), false, 5);
        assert_eq!((left.caret, left.selection()), (at(0, 2), None));
        page.navigate(&app, &faces(), Motion::Char(1), false, 5);
        assert_eq!((page.caret, page.selection()), (at(0, 4), None));
    }

    /// Down by a laid-out line, keeping the column across a short line.
    #[test]
    fn a_vertical_run_keeps_its_column() {
        // Ten characters a line: "the cat " / "sat" / then a short block / then a long one.
        let app = app(&["the cat sat", "ab", "0123456789"]);
        let mut page = Page::default();
        page.place(at(0, 6), false);
        page.navigate(&app, &faces(), Motion::Line(1), false, 5);
        assert_eq!(page.caret, at(0, 11), "the short second line's end");
        page.navigate(&app, &faces(), Motion::Line(1), false, 5);
        assert_eq!(page.caret, at(1, 2));
        page.navigate(&app, &faces(), Motion::Line(1), false, 5);
        assert_eq!(page.caret, at(2, 6), "back in the column it started in");
    }

    #[test]
    fn the_paragraph_motions_stay_or_cross_as_the_mac_does() {
        let app = app(&["one", "two"]);
        let mut page = Page::default();
        page.place(at(1, 2), false);
        assert_eq!(
            page.moved(&app, &faces(), Motion::ParagraphStart, 5),
            at(1, 0)
        );
        assert_eq!(
            page.moved(&app, &faces(), Motion::ParagraphEnd, 5),
            at(1, 3)
        );
        assert_eq!(
            page.moved(&app, &faces(), Motion::ParagraphBack, 5),
            at(1, 0)
        );
        page.place(at(1, 0), false);
        assert_eq!(
            page.moved(&app, &faces(), Motion::ParagraphStart, 5),
            at(1, 0),
            "stays"
        );
        assert_eq!(
            page.moved(&app, &faces(), Motion::ParagraphBack, 5),
            at(0, 0),
            "crosses"
        );
        page.place(at(0, 3), false);
        assert_eq!(page.moved(&app, &faces(), Motion::ParagraphOn, 5), at(1, 3));
        assert_eq!(page.moved(&app, &faces(), Motion::DocEnd, 5), at(1, 3));
        assert_eq!(page.moved(&app, &faces(), Motion::DocStart, 5), START);
    }

    /// `**bold**` typed a character at a time is bold, its markers gone, and what follows is
    /// not — the notation read as it is typed, across keystrokes.
    #[test]
    fn markdown_typed_a_key_at_a_time_formats_and_then_stops() {
        let app = app(&[""]);
        let mut page = Page::default();
        for c in "say **this** and".chars() {
            page.type_text(&app, &c.to_string()).unwrap();
        }
        assert_eq!(texts(&app), ["say this and"]);
        let bold = |from, to| app.char_style(at(0, from), at(0, to)).unwrap().is_bold();
        assert!(bold(4, 8), "this");
        assert!(!bold(8, 12), " and");
        assert_eq!(page.caret, at(0, 12));
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let app = app(&["hello world"]);
        let mut page = Page::default();
        page.place(at(0, 6), false);
        page.place(at(0, 11), true);
        page.type_text(&app, "there").unwrap();
        assert_eq!(texts(&app), ["hello there"]);
    }

    #[test]
    fn an_erase_takes_its_unit_and_at_an_edge_the_break() {
        let app = app(&["one two", "three"]);
        let mut page = Page::default();
        page.place(at(1, 0), false);
        page.erase(&app, &faces(), Unit::Char, false).unwrap();
        assert_eq!(texts(&app), ["one twothree"], "the break between them");
        assert_eq!(page.caret, at(0, 7));
        page.erase(&app, &faces(), Unit::Word, false).unwrap();
        assert_eq!(texts(&app), ["one three"]);
        page.erase(&app, &faces(), Unit::Paragraph, true).unwrap();
        assert_eq!(texts(&app), ["one "], "⌃K to the end");
        page.place(START, false);
        page.erase(&app, &faces(), Unit::Char, false).unwrap();
        assert_eq!(texts(&app), ["one "], "nothing before the start");
    }

    #[test]
    fn return_splits_and_tab_nests_a_list_or_types_a_tab() {
        let app = app(&["ab"]);
        let mut page = Page::default();
        page.place(at(0, 1), false);
        page.split(&app).unwrap();
        assert_eq!(texts(&app), ["a", "b"]);
        assert_eq!(page.caret, at(1, 0));
        page.place(at(0, 1), false);
        page.tab(&app, false).unwrap();
        assert_eq!(texts(&app), ["a\t", "b"], "mid-paragraph, a tab");
        app.set_kind(1, BlockKind::ListItem { depth: 1 }).unwrap();
        page.place(at(1, 0), false);
        page.tab(&app, false).unwrap();
        assert_eq!(
            app.get_viewport(1..2).get(1).unwrap().kind,
            BlockKind::ListItem { depth: 2 }
        );
    }

    #[test]
    fn a_copy_and_a_paste_carry_block_breaks_as_newlines() {
        let app = app(&["first line", "second"]);
        let mut page = Page::default();
        page.place(at(0, 6), false);
        page.place(at(1, 3), true);
        assert_eq!(page.selected_text(&app).as_deref(), Some("line\nsec"));
        page.paste(&app, "A\r\nB").unwrap();
        assert_eq!(texts(&app), ["first A", "Bond"]);
        assert_eq!(page.caret, at(1, 1));
    }

    /// Marked text is held here and not written; committing it types it, once, and an empty
    /// one ends the composition with nothing written.
    #[test]
    fn a_composition_is_nothing_until_it_is_committed() {
        let app = app(&["caf"]);
        let mut page = Page::default();
        page.place(at(0, 3), false);
        page.mark(&app, "\u{b4}", 1..1).unwrap();
        assert_eq!(texts(&app), ["caf"], "nothing written while composing");
        assert_eq!(
            page.shown_caret(),
            at(0, 4),
            "the caret after the marked text"
        );
        page.type_text(&app, "é").unwrap();
        assert_eq!(texts(&app), ["café"]);
        assert_eq!(page.composing, None);
        page.mark(&app, "x", 0..1).unwrap();
        page.mark(&app, "", 0..0).unwrap();
        assert_eq!(
            (texts(&app), page.composing.clone()),
            (vec!["café".to_owned()], None)
        );
        page.mark(&app, "ok", 2..2).unwrap();
        page.unmark(&app).unwrap();
        assert_eq!(texts(&app), ["caféok"], "unmarking commits what is there");
    }

    #[test]
    fn an_undo_puts_the_caret_somewhere_that_still_exists() {
        let app = app(&["x"]);
        let mut page = Page::default();
        page.place(at(0, 1), false);
        page.split(&app).unwrap();
        page.type_text(&app, "long line").unwrap();
        assert!(page.history(&app, true));
        assert_eq!(
            page.caret,
            at(1, 0),
            "the typing gone, its block still there"
        );
        assert!(page.history(&app, true));
        assert_eq!(page.caret, at(0, 0), "clamped into the one block left");
    }

    #[test]
    fn a_double_click_takes_the_word_and_a_triple_the_block() {
        let app = app(&["don't stop"]);
        let mut page = Page::default();
        page.select_word(&app, at(0, 2));
        assert_eq!(page.selection(), Some((at(0, 0), at(0, 5))));
        page.select_block(&app, at(0, 7));
        assert_eq!(page.selection(), Some((at(0, 0), at(0, 10))));
        page.select_all(&app);
        assert_eq!(page.selection(), Some((START, at(0, 10))));
    }
}
