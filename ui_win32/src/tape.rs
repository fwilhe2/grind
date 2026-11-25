// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tape (`doc/calc.md`) as this shell's list modal shows it: one row per line
//! with its value, then the verbs. Portable, so what a pick means tests on any host; the loop
//! that shows it is `win.rs`'s.

use grind_sheet::Tape;

/// What a row of the list does when it is picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Edit line `n` (0-based); emptied, it is deleted.
    Line(usize),
    Add,
    Undo,
    Redo,
    Open,
}

/// The list's rows and what each does, in the same order. A verb that cannot run is left out
/// rather than shown and refused.
pub fn rows(tape: &Tape) -> (Vec<String>, Vec<Pick>) {
    let mut rows = Vec::new();
    let mut picks = Vec::new();
    for (at, line) in tape.lines().into_iter().enumerate() {
        let mark = if line.error { "  \u{26a0}" } else { "" };
        rows.push(format!(
            "#{}   {}   = {}{mark}",
            line.handle, line.text, line.value
        ));
        picks.push(Pick::Line(at));
    }
    rows.push("Add a Line…".to_owned());
    picks.push(Pick::Add);
    if tape.can_undo() {
        rows.push("Undo".to_owned());
        picks.push(Pick::Undo);
    }
    if tape.can_redo() {
        rows.push("Redo".to_owned());
        picks.push(Pick::Redo);
    }
    if !tape.is_empty() {
        rows.push("Open as Spreadsheet".to_owned());
        picks.push(Pick::Open);
    }
    (rows, picks)
}

/// What the line prompt says: the tape's syntax, and a circle when there is one.
pub fn label(tape: &Tape) -> String {
    let hint = "A formula without its =. #2 reads line 2; vat = 0.19 names a line.";
    match tape.circles() {
        Some(circles) => format!("{circles}: #NUM! until the circle is broken. {hint}"),
        None => hint.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_tape_offers_only_a_new_line() {
        let tape = Tape::new(None);
        assert_eq!(
            rows(&tape),
            (vec!["Add a Line…".to_owned()], vec![Pick::Add])
        );
    }

    #[test]
    fn each_line_is_a_row_with_its_value_and_the_verbs_follow() {
        let mut tape = Tape::new(None);
        tape.push("vat = 0.19").unwrap();
        tape.push("1200 * (1 + vat)").unwrap();
        let (rows, picks) = rows(&tape);
        assert_eq!(rows[0], "#1   vat = 0.19   = 0.19");
        assert_eq!(rows[1], "#2   1200*(1+vat)   = 1428");
        assert_eq!(
            picks,
            [
                Pick::Line(0),
                Pick::Line(1),
                Pick::Add,
                Pick::Undo,
                Pick::Open
            ]
        );
        tape.undo();
        assert!(super::rows(&tape).1.contains(&Pick::Redo));
    }

    #[test]
    fn a_circle_is_marked_and_said() {
        let mut tape = Tape::new(None);
        tape.push("#1 + 1").unwrap();
        assert!(rows(&tape).0[0].ends_with('\u{26a0}'));
        assert!(label(&tape).starts_with("#1 reads itself"));
    }
}
