// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The formula assistant: a local model **proposes** a formula, and the spreadsheet engine
//! **decides** whether it is one (`doc/formula-assistant.md`).
//!
//! This is A0, and it has no model in it on purpose. What is here is everything a model will be
//! measured *by*, built and proved before any model is involved:
//!
//! - [`prompt`], what a model is given: fixed instructions, the function catalog generated from
//!   the engine (decision 2 — the knowledge is grind's, never trained in), the document as
//!   [`AssistContext`] shows it (decision 8 — `grind sheet assist` prints that part exactly), and
//!   the request.
//! - [`Backend`], the one trait a model sits behind, so A1 can compare runtimes before one is
//!   chosen.
//! - [`check`], the verdict on one candidate: does it parse, does it evaluate without an error,
//!   and is the answer the expected one — decision 1, the part that makes a guess trustworthy.
//! - [`mod@bench`], the benchmark over the function samples, scored by value and never by text.

use grind_sheet::assist_context::AssistContext;
use grind_sheet::formula::{display, funcs, value::FormulaError};
use grind_sheet::{App, CellValue, Pos};

pub mod bench;
pub mod fence;
pub mod server;

/// What a model is asked: the prompt, and how many candidates are wanted.
#[derive(Clone, Debug)]
pub struct Request {
    /// An opaque label for the request — a sample's address in the benchmark. A model never sees
    /// it; the benchmark's stand-in backends answer by it.
    pub id: String,
    pub prompt: Prompt,
    pub candidates: usize,
    /// The grammar every candidate must be generated in, as GBNF ([`fence::Fence`]), or `None`
    /// for an unfenced request. A stand-in backend ignores it.
    pub fence: Option<String>,
}

/// A prompt in the two parts a chat model takes: what is the same for every request, and what
/// is not. The split is also what makes a request cheap — a server keeps the evaluated `system`
/// part between requests and only reads `user` afresh.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub system: String,
    pub user: String,
}

impl std::fmt::Display for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n\n{}", self.system, self.user)
    }
}

/// Whether the prompt carries the function catalog — decision 2, and the first thing A1 measures
/// is what it is worth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Catalog {
    /// Every function the engine implements, with its signature and summary.
    Whole,
    /// None: what the model knows on its own.
    None,
}

/// Something that proposes formulas: a model behind a runtime (A1 on), or a stand-in.
///
/// Each candidate is one formula in display syntax, with or without its `=`. A backend may
/// return fewer than asked for, or none; it never has to be right, which is the point of
/// [`check`].
pub trait Backend {
    fn propose(&mut self, request: &Request) -> Result<Vec<String>, String>;
}

/// The instructions every prompt starts with. Fixed text, so two runs of one model on one
/// request see one prompt.
const INSTRUCTIONS: &str = "\
You write one spreadsheet formula in OpenFormula display syntax, the way a spreadsheet's formula
bar shows it. Rules:
- Start with =.
- Separate arguments with ; and never with a comma.
- Refer to a cell as B7, to a range as B2:B14, and to another sheet's cell as Sheet.B7.
- Write text in double quotes, and a date as DATE(year;month;day).
- Answer with the formula alone: no explanation, no code fence.";

/// The prompt for one request: the fixed instructions and, with [`Catalog::Whole`], every
/// function the engine implements with its signature and summary, as the system part; the
/// document as `context` renders it and what the person asked for as the user part.
pub fn prompt(context: &AssistContext, request: &str, catalog: Catalog) -> Prompt {
    let system = match catalog {
        Catalog::Whole => {
            let mut functions: Vec<String> = funcs::catalog()
                .iter()
                .map(|info| format!("{} — {}", info.signature, info.brief))
                .collect();
            functions.sort();
            format!(
                "{INSTRUCTIONS}\n- Call only the functions in this list, with the parameters it \
                 gives.\n\nFunctions:\n{}",
                functions.join("\n")
            )
        }
        Catalog::None => INSTRUCTIONS.to_owned(),
    };
    Prompt {
        system,
        user: format!("The document:\n{context}\nThe request: {request}\n"),
    }
}

/// What became of one candidate, in the order the checks run.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Display syntax the parser refuses.
    Unparseable,
    /// Evaluates to an error — `#NAME?` for a function this build does not have.
    Error(FormulaError),
    /// Evaluates, to something other than what was expected.
    Wrong(CellValue),
    /// Evaluates to what was expected.
    Right,
}

impl Verdict {
    /// Whether a candidate survives decision 1's checks, right or not: a person would be shown
    /// it.
    pub fn survives(&self) -> bool {
        matches!(self, Verdict::Wrong(_) | Verdict::Right)
    }
}

/// Check one candidate formula at `pos` against the answer it should give.
///
/// Nothing is stored: [`App::preview`] takes a read lock and makes no undo entry. Numbers agree
/// to nine significant digits — the question is whether the model chose the right formula, not
/// the engine's last digits, which `sheet/tests/function_samples.rs` holds to the oracle on its
/// own.
pub fn check(app: &App, sheet: usize, pos: Pos, candidate: &str, expected: &CellValue) -> Verdict {
    let typed = candidate.trim();
    let typed = match typed.starts_with('=') {
        true => typed.to_owned(),
        false => format!("={typed}"),
    };
    let Ok(canonical) = display::from_display(&typed) else {
        return Verdict::Unparseable;
    };
    let value = match app.preview(sheet, pos, &canonical) {
        Ok(value) => value,
        Err(_) => return Verdict::Unparseable,
    };
    if let CellValue::Text(text) = &value
        && let Some(error) = FormulaError::from_name(text)
        && FormulaError::from_name(&shown(expected)) != Some(error)
    {
        return Verdict::Error(error);
    }
    match agrees(expected, &value) {
        true => Verdict::Right,
        false => Verdict::Wrong(value),
    }
}

fn shown(value: &CellValue) -> String {
    match value {
        CellValue::Text(text) => text.clone(),
        _ => String::new(),
    }
}

fn agrees(expected: &CellValue, got: &CellValue) -> bool {
    match (expected, got) {
        (CellValue::Number(x), CellValue::Number(y)) => {
            x == y || (x - y).abs() <= 1e-9 * x.abs().max(y.abs())
        }
        (CellValue::Empty, CellValue::Text(t)) | (CellValue::Text(t), CellValue::Empty) => {
            t.is_empty()
        }
        (a, b) => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::RecalcMode;

    fn pos(row: u32, col: u32) -> Pos {
        Pos { row, col }
    }

    fn app() -> App {
        let app = App::new();
        for (row, input) in ["2", "3", "4"].iter().enumerate() {
            app.enter(0, pos(row as u32, 0), input, RecalcMode::No)
                .unwrap();
        }
        app
    }

    #[test]
    fn a_candidate_is_judged_by_its_value_not_its_spelling() {
        let app = app();
        let nine = CellValue::Number(9.0);
        for candidate in ["=SUM(A1:A3)", "SUM(A1:A3)", "=A1+A2+A3", " =a1+a2+a3 "] {
            assert_eq!(
                check(&app, 0, pos(3, 0), candidate, &nine),
                Verdict::Right,
                "{candidate}"
            );
        }
        assert_eq!(
            check(&app, 0, pos(3, 0), "=SUM(A1:A2)", &nine),
            Verdict::Wrong(CellValue::Number(5.0))
        );
    }

    #[test]
    fn what_cannot_be_shown_is_named() {
        let app = app();
        let nine = CellValue::Number(9.0);
        assert_eq!(
            check(&app, 0, pos(3, 0), "=SUM(A1:A3", &nine),
            Verdict::Unparseable
        );
        // Excel's separator: the display parser does not take a comma between arguments.
        assert_eq!(
            check(&app, 0, pos(3, 0), "=SUM(A1,A3)", &nine),
            Verdict::Unparseable
        );
        assert_eq!(
            check(&app, 0, pos(3, 0), "=SUMMATION(A1:A3)", &nine),
            Verdict::Error(FormulaError::Name)
        );
        assert!(!Verdict::Unparseable.survives());
        assert!(Verdict::Wrong(CellValue::Empty).survives());
    }

    #[test]
    fn an_expected_error_is_an_answer_like_any_other() {
        let app = app();
        let expected = CellValue::Text("#DIV/0!".into());
        assert_eq!(
            check(&app, 0, pos(3, 0), "=A1/0", &expected),
            Verdict::Right
        );
    }

    #[test]
    fn the_prompt_carries_the_context_exactly_and_every_function() {
        let app = app();
        let context = app.assist_context(0, pos(3, 0)).unwrap();
        let prompt = prompt(&context, "the total of column A", Catalog::Whole);
        assert!(prompt.user.contains(&context.to_string()));
        for name in funcs::implemented() {
            assert!(
                prompt.system.contains(&format!("\n{name}(")),
                "{name} is not offered"
            );
        }
        assert!(
            prompt
                .user
                .ends_with("The request: the total of column A\n")
        );
        let bare = super::prompt(&context, "the total of column A", Catalog::None);
        assert_eq!(bare.user, prompt.user);
        assert!(!bare.system.contains("SUM("));
    }
}
