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
use grind_sheet::formula::{display, friendly, funcs, value::FormulaError};
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
    /// The `n` functions whose names and summaries share the most words with the request — a
    /// short list a small model can read, at the price of a per-request part the server cannot
    /// keep between requests.
    Relevant(usize),
}

/// What a prompt teaches beyond the bare instructions — the variants tried after A1 found
/// SmolLM3 writing Excel, choosing `VLOOKUP` for everything and referring to the empty sheet its
/// formula sits on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coaching {
    /// [`RULES`]: name the sheet in every reference, and grind's spelling against Excel's.
    pub rules: bool,
    /// Worked examples, `(request, formula)`, shown before the document. The benchmark takes them
    /// from *other* documents than the task's, so a task never sees its own answer.
    pub examples: Vec<(String, String)>,
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

/// The rules [`Coaching::rules`] adds, each aimed at a failure A1 counted.
const RULES: &str = "\
- The sheet the formula goes in is usually empty. The data is on the other sheets shown, so name
  the sheet in every reference: Ledger.D2:D14, never D2:D14.
- This is not Excel. Write SUM(A1;A2), never SUM(A1,A2). Write Ledger.A1, never Ledger!A1. Write
  TRUE() and FALSE().
- Choose the simplest function that does what is asked. To add, count or average the rows that
  meet a condition use SUMIF, COUNTIF or AVERAGEIF. Use VLOOKUP, INDEX or MATCH only to find one
  value by a key. Use a function on a single cell when the request is about one cell.
- Write one formula and stop.";

/// The prompt for one request. The system part is everything that does not depend on the request
/// — the instructions, the rules, the whole catalog, the examples — so a server keeps it; the
/// user part is the document as `context` renders it, a relevant catalog if one was asked for,
/// and the request.
pub fn prompt(
    context: &AssistContext,
    request: &str,
    catalog: Catalog,
    coaching: &Coaching,
) -> Prompt {
    let mut system = INSTRUCTIONS.to_owned();
    if coaching.rules {
        system = format!("{system}\n{RULES}");
    }
    if catalog == Catalog::Whole {
        let mut functions: Vec<String> = funcs::catalog().iter().map(line).collect();
        functions.sort();
        system = format!(
            "{system}\n- Call only the functions in this list, with the parameters it gives.\n\n\
             Functions:\n{}",
            functions.join("\n")
        );
    }
    if !coaching.examples.is_empty() {
        let shown: Vec<String> = coaching
            .examples
            .iter()
            .map(|(request, formula)| format!("Request: {request}\nFormula: {formula}"))
            .collect();
        system = format!(
            "{system}\n\nExamples, from other documents:\n\n{}",
            shown.join("\n\n")
        );
    }
    let mut user = format!("The document:\n{context}\n");
    if let Catalog::Relevant(n) = catalog {
        let functions: Vec<String> = relevant(request, n).into_iter().map(line).collect();
        user += &format!(
            "Functions that may help (call only functions this spreadsheet has):\n{}\n\n",
            functions.join("\n")
        );
    }
    user += &format!("The request: {request}\n");
    Prompt { system, user }
}

/// The functions spreadsheets call most, in order — `doc/function-coverage.md`'s corpus counts,
/// abridged — for breaking ties in [`relevant`].
const COMMON: &[&str] = &[
    "SUM",
    "IF",
    "SUMIF",
    "COUNTIF",
    "AVERAGE",
    "ROUND",
    "COUNT",
    "MAX",
    "MIN",
    "VLOOKUP",
    "INDEX",
    "MATCH",
    "AND",
    "OR",
    "COUNTA",
    "SUMPRODUCT",
    "IFERROR",
    "ABS",
    "INT",
    "LEFT",
];

fn line(info: &funcs::FuncInfo) -> String {
    format!("{} — {}", info.signature, info.brief)
}

/// The `n` functions sharing the most words with `request`: its name, plain-English name and
/// summary against the request's words, each cut to five letters so `averages` meets `average`.
/// Ties go to [`COMMON`] first and catalog order after, so a request sharing no word with any
/// summary still gets the functions people use most rather than the alphabet's first dozen.
fn relevant(request: &str, n: usize) -> Vec<&'static funcs::FuncInfo> {
    let stems = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 3)
            .map(|w| w.to_lowercase().chars().take(5).collect())
            .collect()
    };
    let asked = stems(request);
    let mut scored: Vec<(usize, usize, &funcs::FuncInfo)> = funcs::catalog()
        .iter()
        .enumerate()
        .map(|(at, info)| {
            let alias = friendly::alias(info.name).unwrap_or(info.name);
            let words = stems(&format!("{} {alias} {}", info.name, info.brief));
            let shared = asked.iter().filter(|w| words.contains(w)).count();
            let rank = COMMON
                .iter()
                .position(|c| *c == info.name)
                .unwrap_or(COMMON.len() + at);
            (shared, rank, info)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(n)
        .map(|(_, _, info)| info)
        .collect()
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

/// Check one candidate formula at `pos` against the answer it should give, reading it as display
/// syntax only. [`check_with`] with `excel: false`.
pub fn check(app: &App, sheet: usize, pos: Pos, candidate: &str, expected: &CellValue) -> Verdict {
    check_with(app, sheet, pos, candidate, expected, false)
}

/// How a candidate was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spelling {
    /// Display syntax, the formula bar's: `SUM(Ledger.D2:D14;1)`.
    Display,
    /// Excel's, translated by the import filter: `SUM(Ledger!D2:D14,1)`.
    Excel,
}

/// A candidate as the stored formula it means, and which reading produced it: display syntax
/// first, and with `excel`, Excel's syntax through `grind_xlsx::formula::translate` when display
/// syntax refuses it. Either way the result is a tree printed by the core's own serialiser — a
/// formula the parser could have produced — never a string rewritten into one.
///
/// Display first, because a formula that reads both ways must mean what the formula bar means.
/// The translator renames nothing whose semantics differ between the two (Excel's `CEILING`
/// arrives as `COM.MICROSOFT.CEILING`), so a translated candidate cannot quietly change meaning.
pub fn read(candidate: &str, excel: bool) -> Option<(String, Spelling)> {
    let typed = candidate.trim();
    let typed = match typed.starts_with('=') {
        true => typed.to_owned(),
        false => format!("={typed}"),
    };
    let as_excel = || {
        grind_xlsx::formula::translate(&typed)
            .ok()
            .map(|expr| (format!("={expr}"), Spelling::Excel))
    };
    // `!` outside a string is Excel's sheet separator to anybody writing a formula from a
    // request; in display syntax it is the intersection operator, which §2.3.2 leaves out of the
    // Small Group, and `Ledger!D2` would read as the name `Ledger` intersected with `D2`, which is
    // #NULL!. So a `!` sends the candidate to Excel's reading first.
    if excel
        && bang_outside_strings(&typed)
        && let Some(read) = as_excel()
    {
        return Some(read);
    }
    if let Ok(canonical) = display::from_display(&typed) {
        return Some((canonical, Spelling::Display));
    }
    match excel {
        true => as_excel(),
        false => None,
    }
}

fn bang_outside_strings(text: &str) -> bool {
    let mut quoted = false;
    text.chars().any(|c| {
        if c == '"' {
            quoted = !quoted;
        }
        c == '!' && !quoted
    })
}

/// [`check`], reading Excel's syntax too when `excel` is set ([`read`]).
///
/// Nothing is stored: [`App::preview`] takes a read lock and makes no undo entry. Numbers agree
/// to nine significant digits — the question is whether the model chose the right formula, not
/// the engine's last digits, which `sheet/tests/function_samples.rs` holds to the oracle on its
/// own.
pub fn check_with(
    app: &App,
    sheet: usize,
    pos: Pos,
    candidate: &str,
    expected: &CellValue,
    excel: bool,
) -> Verdict {
    let Some((canonical, _)) = read(candidate, excel) else {
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
        let prompt = prompt(
            &context,
            "the total of column A",
            Catalog::Whole,
            &Coaching::default(),
        );
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
        let bare = super::prompt(
            &context,
            "the total of column A",
            Catalog::None,
            &Coaching::default(),
        );
        assert_eq!(bare.user, prompt.user);
        assert!(!bare.system.contains("SUM("));
    }

    #[test]
    fn coaching_goes_where_a_server_can_keep_it() {
        let app = app();
        let context = app.assist_context(0, pos(3, 0)).unwrap();
        let coaching = Coaching {
            rules: true,
            examples: vec![(
                "Spent on tea".into(),
                "=SUMIF(Ledger.B2:B9;\"Tea\";Ledger.D2:D9)".into(),
            )],
        };
        let prompt = prompt(
            &context,
            "the average mark",
            Catalog::Relevant(5),
            &coaching,
        );
        assert!(prompt.system.contains("never SUM(A1,A2)"));
        assert!(
            prompt
                .system
                .contains("Request: Spent on tea\nFormula: =SUMIF(")
        );
        // The relevant catalog depends on the request, so it is the user part's.
        assert!(!prompt.system.contains("Functions"));
        assert!(prompt.user.contains("\nAVERAGE("), "{}", prompt.user);
        assert_eq!(prompt.user.matches(" — ").count(), 5);
    }

    #[test]
    fn excel_is_read_only_when_asked_and_only_after_display_syntax() {
        let app = app();
        let nine = CellValue::Number(9.0);
        assert_eq!(
            check(&app, 0, pos(3, 0), "=SUM(A1,A3)+A2", &nine),
            Verdict::Unparseable
        );
        assert_eq!(
            check_with(&app, 0, pos(3, 0), "=SUM(A1,A3)+A2", &nine, true),
            Verdict::Right
        );
        assert_eq!(
            check_with(&app, 0, pos(3, 0), "=SUM(Sheet1!A1:A3)", &nine, true),
            Verdict::Right
        );
        assert_eq!(read("=SUM(A1:A3)", true).unwrap().1, Spelling::Display);
        assert_eq!(read("=SUM(A1,A3)", true).unwrap().1, Spelling::Excel);
        // Excel's CEILING is not ODF's: it arrives under the name LibreOffice gives it.
        assert!(
            read("=CEILING(A1,2)", true)
                .unwrap()
                .0
                .contains("COM.MICROSOFT.CEILING")
        );
        assert_eq!(read("=SUM(A1,", true), None);
        // A `!` is Excel's sheet separator here, never ODF's intersection.
        assert_eq!(read("=SUM(Sheet1!A1:A3)", true).unwrap().1, Spelling::Excel);
        assert_eq!(read("=LEN(\"Hi!\")", true).unwrap().1, Spelling::Display);
    }
}
