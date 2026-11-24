// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Edit ▸ Scratch Calculator…: the tape (`doc/calc.md`, an experiment) in `prompt.rs`'s alerts —
//! a pop-up of its lines with their values, then the verbs. The rows are portable, so what a
//! pick means tests on any host; the alerts that show them are AppKit's.

use grind_sheet::Tape;

/// What a row of the pop-up does when it is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Change or delete line `n` (0-based).
    Line(usize),
    Add,
    Undo,
    Redo,
    Open,
}

/// The pop-up's rows and what each does, in the same order: a verb that cannot run is left out.
/// A new line comes first, since it is what the tape is opened for.
pub fn rows(tape: &Tape) -> (Vec<String>, Vec<Pick>) {
    let mut rows = vec!["Add a Line…".to_owned()];
    let mut picks = vec![Pick::Add];
    for (at, line) in tape.lines().into_iter().enumerate() {
        let mark = if line.error { "  \u{26a0}" } else { "" };
        rows.push(format!(
            "#{}   {}   = {}{mark}",
            line.handle, line.text, line.value
        ));
        picks.push(Pick::Line(at));
    }
    for (can, row, pick) in [
        (tape.can_undo(), "Undo", Pick::Undo),
        (tape.can_redo(), "Redo", Pick::Redo),
        (!tape.is_empty(), "Open as Spreadsheet", Pick::Open),
    ] {
        if can {
            rows.push(row.to_owned());
            picks.push(pick);
        }
    }
    (rows, picks)
}

/// The sentence over the pop-up: the tape's syntax, and a circle when there is one.
pub fn detail(tape: &Tape) -> String {
    let hint = "A formula without its =. #2 reads line 2; vat = 0.19 names a line.";
    match tape.circles() {
        Some(circles) => format!("{circles}: #NUM! until the circle is broken. {hint}"),
        None => hint.to_owned(),
    }
}

/// The tape, step by step, until the pop-up is cancelled: the promoted spreadsheet when it ends
/// on *Open as Spreadsheet*.
#[cfg(target_os = "macos")]
pub fn run(mtm: objc2::MainThreadMarker, tape: &mut Tape) -> Option<grind_sheet::Document> {
    use crate::prompt;
    let mut chosen = 0;
    loop {
        let (rows, picks) = rows(tape);
        let detail = detail(tape);
        chosen = prompt::pick(
            mtm,
            "Scratch Calculator",
            &detail,
            "Choose",
            &rows,
            chosen.min(rows.len() - 1),
        )?;
        let done = match picks.get(chosen).copied()? {
            Pick::Add => match prompt::ask(mtm, "Add a Line", &detail, "Add", "") {
                Some(text) => tape.push(&text).map(|_| ()),
                None => continue,
            },
            Pick::Line(line) => {
                let text = tape.lines().get(line).map(|l| l.text.clone())?;
                match prompt::change(mtm, &format!("Line #{}", line + 1), &detail, &text) {
                    Some(Some(text)) => tape.edit(line, &text),
                    Some(None) => tape.delete(line),
                    None => continue,
                }
            }
            Pick::Undo => {
                tape.undo();
                Ok(())
            }
            Pick::Redo => {
                tape.redo();
                Ok(())
            }
            Pick::Open => match tape.promote() {
                Ok(doc) => return Some(doc),
                Err(error) => Err(error),
            },
        };
        if let Err(error) = done {
            prompt::tell(mtm, "The tape did not take that.", &error.to_string());
        }
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
        assert_eq!(rows[1], "#1   vat = 0.19   = 0.19");
        assert_eq!(rows[2], "#2   1200*(1+vat)   = 1428");
        assert_eq!(
            picks,
            [
                Pick::Add,
                Pick::Line(0),
                Pick::Line(1),
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
        assert!(rows(&tape).0[1].ends_with('\u{26a0}'));
        assert!(detail(&tape).starts_with("#1 reads itself"));
    }
}
