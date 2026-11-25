// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A tape shown as a notebook: one editable row per line and a blank row after the last, where
//! Enter opens a fresh row under the one it commits. Which row the caret goes to after a key is
//! decided here, so every client's tape moves alike (`doc/calc.md` §7).

use crate::formula::assist::{self, Assist, Piece};
use crate::{Result, Tape, TapeSpan};

/// A row of the notebook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// Line `n`, 0-based.
    Line(usize),
    /// A blank row that becomes line `n` when something is typed into it: the one after the last
    /// line, or one Enter opened.
    Blank(usize),
}

/// What a key asks of the row the caret is on, with what the row holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Commit, and open a blank row under it.
    Enter,
    Up,
    Down,
    /// Backspace in an empty row: the line goes, and the caret to the end of the one above.
    Erase,
}

#[derive(Clone, Debug, Default)]
pub struct Notebook {
    /// The blank row Enter opened among the lines, which exists while the caret is on it.
    opened: Option<usize>,
}

impl Notebook {
    /// The rows in order: each line, the row Enter opened, and the blank row at the end.
    #[must_use]
    pub fn rows(&self, tape: &Tape) -> Vec<Row> {
        let len = tape.len();
        let mut rows = Vec::with_capacity(len + 2);
        for at in 0..len {
            if self.opened == Some(at) {
                rows.push(Row::Blank(at));
            }
            rows.push(Row::Line(at));
        }
        rows.push(Row::Blank(len));
        rows
    }

    /// The handle a row shows: `#n` for a line and for the row at the end, `+` for an opened one.
    #[must_use]
    pub fn label(row: Row, tape: &Tape) -> String {
        match row {
            Row::Line(at) => format!("#{}", at + 1),
            Row::Blank(at) if at >= tape.len() => format!("#{}", at + 1),
            Row::Blank(_) => "+".to_owned(),
        }
    }

    /// What `row` holds before anything is typed into it.
    #[must_use]
    pub fn text(row: Row, tape: &Tape) -> String {
        match row {
            Row::Line(at) => tape
                .lines()
                .get(at)
                .map_or_else(String::new, |l| l.text.clone()),
            Row::Blank(_) => String::new(),
        }
    }

    /// Write what `row` holds, as the caret leaves it: a changed line is edited, an emptied one
    /// deleted, and a blank row with something in it becomes a line. Answers what happened.
    /// Leaving closes an opened row either way.
    pub fn commit(&mut self, tape: &mut Tape, row: Row, text: &str) -> Result<Done> {
        let done = match row {
            Row::Line(at) if text.trim().is_empty() => {
                tape.delete(at)?;
                Done::Deleted
            }
            Row::Line(at) if tape.reads_as(at, text) => Done::Nothing,
            Row::Line(at) => {
                tape.edit(at, text)?;
                Done::Written
            }
            Row::Blank(_) if text.trim().is_empty() => Done::Nothing,
            Row::Blank(at) => {
                tape.insert(at.min(tape.len()), text)?;
                Done::Written
            }
        };
        self.opened = None;
        Ok(done)
    }

    /// Commit `row` and say where the caret goes for `step`. Refused, nothing changed and the
    /// caret stays.
    pub fn step(&mut self, tape: &mut Tape, row: Row, text: &str, step: Step) -> Result<Row> {
        let opened = self.opened;
        let done = if step == Step::Erase {
            match row {
                Row::Line(at) => {
                    tape.delete(at)?;
                    Done::Deleted
                }
                Row::Blank(_) => Done::Nothing,
            }
        } else {
            self.commit(tape, row, text)?
        };
        self.opened = None;
        let len = tape.len();
        let end = Row::Blank(len);
        // The line the row is, or has become; and the first line below it.
        let (this, below) = match (row, done) {
            (Row::Line(at), Done::Deleted) => (None, at),
            (Row::Line(at), _) | (Row::Blank(at), Done::Written) => (Some(at), at + 1),
            (Row::Blank(at), _) => (None, at),
        };
        let line_or_end = |at: usize| if at < len { Row::Line(at) } else { end };
        let above = match (row, this) {
            (_, Some(at)) | (Row::Line(at) | Row::Blank(at), None) => at.checked_sub(1),
        };
        Ok(match step {
            // Enter in an empty blank row has nothing to commit, and stays.
            Step::Enter if matches!((row, done), (Row::Blank(_), Done::Nothing)) => {
                self.opened = opened;
                row
            }
            Step::Enter => {
                self.opened = (below < len).then_some(below);
                Row::Blank(below)
            }
            Step::Down => line_or_end(below),
            Step::Up | Step::Erase => match above {
                Some(at) => Row::Line(at),
                None => this.map_or_else(|| line_or_end(0), Row::Line),
            },
        })
    }
}

/// A key a [`TapeEditor`] answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Up,
    Down,
    Tab,
    Escape,
    Backspace,
}

/// A tape with a notebook's caret: the row being typed in, what it holds and where its caret is,
/// and the offers for the word at the caret. A client with one edit control moves that control to
/// [`TapeEditor::row`] and copies [`TapeEditor::text`] into it after every call.
#[derive(Clone, Debug)]
pub struct TapeEditor {
    tape: Tape,
    book: Notebook,
    assist: Assist,
    row: Row,
    text: String,
    /// A byte offset into `text`.
    caret: usize,
    note: String,
}

impl TapeEditor {
    /// The caret in the blank row at the end.
    #[must_use]
    pub fn new(tape: Tape) -> Self {
        let row = Row::Blank(tape.len());
        let mut editor = Self {
            tape,
            book: Notebook::default(),
            assist: Assist::default(),
            row,
            text: String::new(),
            caret: 0,
            note: String::new(),
        };
        editor.say_circles();
        editor
    }

    #[must_use]
    pub fn tape(&self) -> &Tape {
        &self.tape
    }

    #[must_use]
    pub fn into_tape(self) -> Tape {
        self.tape
    }

    #[must_use]
    pub fn rows(&self) -> Vec<Row> {
        self.book.rows(&self.tape)
    }

    #[must_use]
    pub fn row(&self) -> Row {
        self.row
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// A byte offset into [`TapeEditor::text`].
    #[must_use]
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// A row that does not read, or the circle the tape is in; empty when there is nothing to say.
    #[must_use]
    pub fn note(&self) -> &str {
        &self.note
    }

    /// What the band under the rows shows: the offers, or the call the caret is in.
    #[must_use]
    pub fn band(&self) -> Vec<Piece> {
        assist::band(&self.assist, true)
    }

    #[must_use]
    pub fn is_offering(&self) -> bool {
        self.assist.is_offering()
    }

    /// `row`'s text as it is coloured: what is being typed for the caret's row, the line's own
    /// text for any other.
    #[must_use]
    pub fn shown(&self, row: Row) -> (String, Vec<TapeSpan>) {
        if row == self.row {
            let spans = self.tape.spans(&self.text, Some(self.caret));
            return (self.text.clone(), spans);
        }
        let text = Notebook::text(row, &self.tape);
        let spans = self.tape.spans(&text, None);
        (text, spans)
    }

    /// The edit control's text or caret changed.
    pub fn typed(&mut self, text: &str, caret: usize) {
        text.clone_into(&mut self.text);
        self.caret = text.floor_char_boundary(caret);
        self.tape.assist(&mut self.assist, &self.text, self.caret);
    }

    /// Answer `key`; `false` leaves it to the control (Backspace in a row with text, Tab with
    /// nothing offered, Escape with nothing to take back).
    pub fn key(&mut self, key: Key) -> bool {
        let offering = self.assist.is_offering();
        match key {
            Key::Tab if offering => {
                if let Some((span, insert)) = self.assist.accept() {
                    self.text.replace_range(span.clone(), &insert);
                    self.caret = span.start + insert.len();
                    self.tape.assist(&mut self.assist, &self.text, self.caret);
                }
            }
            Key::Down if offering => self.assist.step(1),
            Key::Up if offering => self.assist.step(-1),
            Key::Escape if offering => self.assist.dismiss(),
            Key::Escape if self.text != Notebook::text(self.row, &self.tape) => {
                self.go(self.row);
            }
            Key::Enter => self.step(Step::Enter),
            Key::Down => self.step(Step::Down),
            Key::Up => self.step(Step::Up),
            Key::Backspace if self.text.is_empty() => self.step(Step::Erase),
            Key::Tab | Key::Escape | Key::Backspace => return false,
        }
        true
    }

    /// A click on `row`: the row the caret leaves is written first, and stays when it does not
    /// read.
    pub fn click(&mut self, row: Row) {
        if row == self.row {
            return;
        }
        let done = match self.book.commit(&mut self.tape, self.row, &self.text) {
            Ok(done) => done,
            Err(error) => {
                self.note = error.to_string();
                return;
            }
        };
        // The rows under the click moved by the line written or deleted.
        let row = match (self.row, done, row) {
            (Row::Blank(at), Done::Written, Row::Line(n)) if n >= at => Row::Line(n + 1),
            (Row::Blank(_), Done::Written, Row::Blank(n)) => Row::Blank(n + 1),
            (Row::Line(at), Done::Deleted, Row::Line(n)) if n > at => Row::Line(n - 1),
            (Row::Line(_), Done::Deleted, Row::Blank(n)) => Row::Blank(n.saturating_sub(1)),
            _ => row,
        };
        let end = Row::Blank(self.tape.len());
        self.landed(self.rows().into_iter().find(|r| *r == row).unwrap_or(end));
    }

    pub fn undo(&mut self) {
        if self.tape.undo() {
            self.reset();
        }
    }

    pub fn redo(&mut self) {
        if self.tape.redo() {
            self.reset();
        }
    }

    /// The line the caret was on, or the end, after the tape changed under it.
    fn reset(&mut self) {
        self.book = Notebook::default();
        let row = match self.row {
            Row::Line(at) if at < self.tape.len() => Row::Line(at),
            _ => Row::Blank(self.tape.len()),
        };
        self.landed(row);
    }

    fn step(&mut self, step: Step) {
        match self.book.step(&mut self.tape, self.row, &self.text, step) {
            Ok(row) => self.landed(row),
            Err(error) => self.note = error.to_string(),
        }
    }

    /// Put the caret at the end of `row`, holding its own text.
    fn go(&mut self, row: Row) {
        self.row = row;
        self.text = Notebook::text(row, &self.tape);
        self.caret = self.text.len();
        self.assist.clear();
    }

    fn landed(&mut self, row: Row) {
        self.go(row);
        self.say_circles();
    }

    fn say_circles(&mut self) {
        self.note = self.tape.circles().map_or_else(String::new, |circles| {
            format!("{circles}: #NUM! until the circle is broken")
        });
    }
}

/// What [`Notebook::commit`] did to the tape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    Nothing,
    Written,
    Deleted,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape(lines: &[&str]) -> Tape {
        let mut t = Tape::default();
        for l in lines {
            t.push(l).unwrap();
        }
        t
    }

    fn texts(t: &Tape) -> Vec<String> {
        t.lines().into_iter().map(|l| l.text).collect()
    }

    #[test]
    fn enter_on_the_last_row_adds_a_line_and_moves_to_a_new_blank_row() {
        let mut t = Tape::default();
        let mut book = Notebook::default();
        assert_eq!(book.rows(&t), [Row::Blank(0)]);
        let at = book
            .step(&mut t, Row::Blank(0), "1 + 1", Step::Enter)
            .unwrap();
        assert_eq!(at, Row::Blank(1));
        let at = book.step(&mut t, at, "#1 * 3", Step::Enter).unwrap();
        assert_eq!(at, Row::Blank(2));
        assert_eq!(texts(&t), ["1+1", "#1*3"]);
        assert_eq!(book.rows(&t), [Row::Line(0), Row::Line(1), Row::Blank(2)]);
        // Enter in the empty row has nothing to do.
        assert_eq!(
            book.step(&mut t, at, " ", Step::Enter).unwrap(),
            Row::Blank(2)
        );
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn enter_on_a_line_opens_a_row_under_it_that_becomes_a_line() {
        let mut t = tape(&["1", "2"]);
        let mut book = Notebook::default();
        let at = book.step(&mut t, Row::Line(0), "1", Step::Enter).unwrap();
        assert_eq!(at, Row::Blank(1));
        assert_eq!(
            book.rows(&t),
            [Row::Line(0), Row::Blank(1), Row::Line(1), Row::Blank(2)]
        );
        assert_eq!(Notebook::label(Row::Blank(1), &t), "+");
        assert_eq!(Notebook::label(Row::Blank(2), &t), "#3");
        let at = book.step(&mut t, at, "1.5", Step::Enter).unwrap();
        assert_eq!(texts(&t), ["1", "1.5", "2"]);
        assert_eq!(at, Row::Blank(2), "and opens the next one under it");
        // Leaving an opened row empty closes it.
        let at = book.step(&mut t, at, "", Step::Down).unwrap();
        assert_eq!(at, Row::Line(2));
        assert_eq!(book.rows(&t).len(), 4);
    }

    #[test]
    fn the_arrows_commit_the_row_and_move_to_the_next() {
        let mut t = tape(&["a = 1", "a + 1"]);
        let mut book = Notebook::default();
        let at = book
            .step(&mut t, Row::Line(0), "a = 5", Step::Down)
            .unwrap();
        assert_eq!(at, Row::Line(1));
        assert_eq!(t.lines()[1].value, "6");
        assert_eq!(
            book.step(&mut t, at, "a + 1", Step::Down).unwrap(),
            Row::Blank(2)
        );
        assert_eq!(
            book.step(&mut t, Row::Blank(2), "", Step::Up).unwrap(),
            Row::Line(1)
        );
        assert_eq!(
            book.step(&mut t, Row::Line(0), "a = 5", Step::Up).unwrap(),
            Row::Line(0)
        );
        // An unchanged line is not written again, so there is nothing more to undo.
        t.undo();
        assert_eq!(t.lines()[0].text, "a = 1");
    }

    #[test]
    fn a_refused_row_stays_with_the_caret_in_it() {
        let mut t = tape(&["1"]);
        let mut book = Notebook::default();
        assert!(book.step(&mut t, Row::Line(0), "1 +", Step::Down).is_err());
        assert!(
            book.step(&mut t, Row::Blank(1), "2 *", Step::Enter)
                .is_err()
        );
        assert_eq!(texts(&t), ["1"]);
    }

    #[test]
    fn erasing_an_empty_row_deletes_its_line_and_goes_up() {
        let mut t = tape(&["1", "2", "3"]);
        let mut book = Notebook::default();
        assert_eq!(
            book.step(&mut t, Row::Line(1), "", Step::Erase).unwrap(),
            Row::Line(0)
        );
        assert_eq!(texts(&t), ["1", "3"]);
        assert_eq!(
            book.step(&mut t, Row::Line(0), "", Step::Erase).unwrap(),
            Row::Line(0)
        );
        assert_eq!(texts(&t), ["3"]);
        assert_eq!(
            book.step(&mut t, Row::Blank(1), "", Step::Erase).unwrap(),
            Row::Line(0)
        );
        assert_eq!(
            book.step(&mut t, Row::Line(0), "", Step::Erase).unwrap(),
            Row::Blank(0)
        );
        assert!(t.is_empty());
    }

    #[test]
    fn the_editor_types_lines_completes_names_and_moves_with_the_keys() {
        let mut ed = TapeEditor::new(Tape::default());
        ed.typed("foo = 3", 7);
        assert!(ed.key(Key::Enter));
        assert_eq!((ed.row(), ed.text()), (Row::Blank(1), ""));
        ed.typed("2 * fo", 6);
        assert!(ed.is_offering());
        let band: String = ed.band().iter().map(|p| p.text.as_str()).collect();
        assert!(band.starts_with("foo"), "{band}");
        assert!(ed.key(Key::Tab));
        assert_eq!((ed.text(), ed.caret()), ("2 * foo", 7));
        let (_, spans) = ed.shown(ed.row());
        assert!(spans.iter().any(|s| s.ink == crate::TapeInk::Name));
        assert!(ed.key(Key::Enter));
        assert_eq!(ed.tape().lines()[1].value, "6");
        // A line that does not read keeps the caret, and the editor says why.
        ed.typed("1 +", 3);
        assert!(ed.key(Key::Enter));
        assert_eq!((ed.row(), ed.text()), (Row::Blank(2), "1 +"));
        assert_ne!(ed.note(), "");
        // Escape takes the row back to what it was; Backspace in it, emptied, goes up.
        assert!(ed.key(Key::Escape));
        assert_eq!(ed.text(), "");
        assert!(
            !ed.key(Key::Escape),
            "nothing left to take back: the dialog's"
        );
        assert!(ed.key(Key::Backspace));
        assert_eq!((ed.row(), ed.text()), (Row::Line(1), "2*foo"));
        ed.typed("2*fo", 4);
        assert!(!ed.key(Key::Backspace), "a row with text is the control's");
        assert!(ed.key(Key::Tab), "the offer for the word at the caret");
        assert_eq!(ed.text(), "2*foo");
    }

    #[test]
    fn a_click_writes_the_row_left_and_undo_keeps_the_caret_near() {
        let mut ed = TapeEditor::new(tape(&["1", "2"]));
        ed.typed("3", 1);
        ed.click(Row::Line(0));
        assert_eq!(texts(ed.tape()), ["1", "2", "3"]);
        assert_eq!((ed.row(), ed.text()), (Row::Line(0), "1"));
        ed.typed("1 +", 3);
        ed.click(Row::Line(1));
        assert_eq!(
            ed.row(),
            Row::Line(0),
            "a row that does not read is not left"
        );
        ed.typed("10", 2);
        ed.click(Row::Line(1));
        ed.undo();
        assert_eq!(texts(ed.tape()), ["1", "2", "3"]);
        assert_eq!((ed.row(), ed.text()), (Row::Line(1), "2"));
        ed.redo();
        assert_eq!(ed.tape().lines()[0].value, "10");
        // A line emptied and left goes, and the click lands on the line it meant.
        ed.typed("", 0);
        ed.click(Row::Line(2));
        assert_eq!((ed.row(), ed.text()), (Row::Line(1), "3"));
    }

    #[test]
    fn a_line_emptied_and_left_is_deleted() {
        let mut t = tape(&["1", "2"]);
        let mut book = Notebook::default();
        assert_eq!(
            book.commit(&mut t, Row::Line(0), " ").unwrap(),
            Done::Deleted
        );
        assert_eq!(texts(&t), ["2"]);
        assert_eq!(
            book.commit(&mut t, Row::Blank(1), "").unwrap(),
            Done::Nothing
        );
    }
}
