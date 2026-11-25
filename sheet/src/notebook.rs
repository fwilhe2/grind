// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A tape shown as a notebook: one editable row per line and a blank row after the last, where
//! Enter opens a fresh row under the one it commits. Which row the caret goes to after a key is
//! decided here, so every client's tape moves alike (`doc/calc.md` §7).

use crate::{Result, Tape};

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
