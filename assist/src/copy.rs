// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A prompt for **any** model, to copy: the document's shape ([`crate::shape`]), what the person
//! wants, where the formula goes, and how to answer. Grind sends it nowhere — the person copies
//! it into the model they choose and pastes the answer back, where [`crate::read`] takes either
//! syntax and the engine evaluates it before anything is written.
//!
//! The benchmark found that a small local model is what fails; the parts grind owns — describing
//! the document, reading a formula, checking it — all work. This keeps those and lets the person
//! pick the model.

use grind_sheet::formula::lex::{Op, Reference};
use grind_sheet::formula::parse::Expr;
use grind_sheet::formula::{display, funcs, parse, shift};
use grind_sheet::numfmt::Format;
use grind_sheet::{App, Form, Pos, RecalcMode, a1};

use crate::{Spelling, read, shape};

/// The prompt for a formula in `start` (filled to `end` when they differ) on `sheet`, doing what
/// `request` says. `values` puts example values in the description; without it only headers and
/// types leave the document.
pub fn copy_prompt(
    app: &App,
    sheet: usize,
    start: Pos,
    end: Pos,
    request: &str,
    values: bool,
) -> Result<String, String> {
    let name = app.sheet_name(sheet).map_err(|e| e.to_string())?;
    let cell = |pos| a1::format(None, pos);
    let mut out = String::from(
        "I am working in a spreadsheet that uses OpenFormula, the formula language of \
         LibreOffice Calc. Please write one formula for me.\n\n## The spreadsheet\n\n",
    );
    // The target's own sheet first, then the others in document order.
    let order = std::iter::once(sheet).chain((0..app.sheet_count()).filter(|&s| s != sheet));
    let mut empty = Vec::new();
    for s in order {
        match shape::describe(app, s, values) {
            Some(table) => out += &format!("{table}\n"),
            None => empty.push(app.sheet_name(s).map_err(|e| e.to_string())?),
        }
    }
    if !empty.is_empty() {
        out += &format!("Empty sheets: {}.\n\n", empty.join(", "));
    }
    let names = app.names();
    if !names.is_empty() {
        out += "Named ranges: ";
        out += &names
            .iter()
            .map(|(n, d)| format!("{n} = {d}"))
            .collect::<Vec<_>>()
            .join(", ");
        out += "\n\n";
    }
    if let Some(locale) = app.locale() {
        out += &format!("The document's locale is {}.\n\n", locale.tag());
    }
    out += &format!(
        "## What I want\n\n{}\n\n## Where it goes\n\n",
        request.trim()
    );
    if start == end {
        out += &format!(
            "The formula goes in cell {} of sheet \"{name}\".\n\n",
            cell(start)
        );
    } else {
        out += &format!(
            "The formula goes in {} of sheet \"{name}\" and will be filled to {}:{}. Relative \
             references move with each cell ({} refers to its own row), so use $ for anything \
             that must stay put.\n\n",
            cell(start),
            cell(start),
            cell(end),
            cell(Pos {
                row: start.row + 1,
                ..start
            }),
        );
    }
    let mut functions: Vec<&str> = funcs::implemented().to_vec();
    functions.sort_unstable();
    out += &format!(
        "## How to answer\n\n\
         - Reply with the formula for {} only, on one line, starting with =. No explanation.\n\
         - Excel syntax is fine (commas, Sheet!A1); so is LibreOffice's (semicolons, Sheet.A1).\n\
         - Use only these functions: {}.\n\
         - To leave a cell looking empty, return \"\".\n",
        cell(start),
        functions.join(", ")
    );
    Ok(out)
}

/// What an answer would do, worked out on a scratch copy of the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Tried {
    /// Which syntax the answer was read in.
    pub spelling: Spelling,
    /// The formula as the formula bar shows it, for the first target cell.
    pub formula: String,
    /// The stored spelling, which [`apply_answer`] writes.
    pub canonical: String,
    /// The number format the results take ([`inherited_format`]), if any.
    pub format: Option<Format>,
    /// Every target cell and what it shows once the formula is filled and the copy recalculated.
    pub values: Vec<(Pos, String)>,
}

/// Read `answer` in display or Excel syntax ([`read`]), put it in `start`, fill it to `end` and
/// recalculate — all on a **copy** of the document, so a formula reading its neighbours (a running
/// balance) sees their new values, and the document itself is untouched: no undo entry, no
/// observer, nothing to take back. Refused when the answer is not a formula in either syntax.
pub fn try_answer(
    app: &App,
    sheet: usize,
    start: Pos,
    end: Pos,
    answer: &str,
) -> Result<Tried, String> {
    let (canonical, spelling) =
        read(answer, true).ok_or_else(|| format!("not a formula in either syntax: {answer}"))?;
    let copy = App::new();
    let bytes = app.save_bytes(Form::Flat).map_err(|e| e.to_string())?;
    copy.open_bytes("copy.fods", &bytes)
        .map_err(|e| e.to_string())?;
    let format = inherited_format(app, sheet, start, end, &canonical);
    put(&copy, sheet, start, end, &canonical, format.as_ref())?;
    let mut values = Vec::new();
    for row in start.row..=end.row {
        for col in start.col..=end.col {
            let pos = Pos { row, col };
            values.push((pos, copy.value_text(sheet, pos).map_err(|e| e.to_string())?));
        }
    }
    Ok(Tried {
        spelling,
        formula: display::to_display(&canonical).unwrap_or_else(|_| canonical.clone()),
        canonical,
        format,
        values,
    })
}

/// Write what [`try_answer`] tried into the document itself: the formula in `start`, filled to
/// `end`, recalculated. Ordinary edits, each one an undo step.
pub fn apply_answer(
    app: &App,
    sheet: usize,
    start: Pos,
    end: Pos,
    tried: &Tried,
) -> Result<(), String> {
    put(
        app,
        sheet,
        start,
        end,
        &tried.canonical,
        tried.format.as_ref(),
    )
}

/// The number format a result takes: the one format shared by the cells its **value comes from**
/// ([`carried`]) — the amounts a `SUMIFS` adds rather than the dates it compares, both branches of
/// an `IF`, the operands of `+` — when the target cells have none of their own. A result that is
/// a count, a month, a comparison or anything else whose kind is not its inputs' takes nothing,
/// and so does one drawn from two formats; a format somebody already put on the target is never
/// replaced.
pub fn inherited_format(
    app: &App,
    sheet: usize,
    start: Pos,
    end: Pos,
    canonical: &str,
) -> Option<Format> {
    let own = (start.row..=end.row)
        .flat_map(|row| (start.col..=end.col).map(move |col| Pos { row, col }))
        .any(|pos| app.format_at(sheet, pos).ok().flatten().is_some());
    if own {
        return None;
    }
    let expr = parse::parse(canonical).ok()?;
    let mut found: Option<Format> = None;
    for reference in carried(&expr)? {
        let Ok((at, from, to)) = a1::resolve_in(app, sheet, reference) else {
            return None;
        };
        // The first few numeric cells of the range speak for it; a header above them does not.
        let numeric = (from.row..=to.row.min(from.row + 50))
            .flat_map(|row| (from.col..=to.col).map(move |col| Pos { row, col }))
            .filter(|pos| matches!(app.get(at, *pos), Ok(grind_sheet::CellValue::Number(_))))
            .take(5);
        for pos in numeric {
            let format = app.format_at(at, pos).ok().flatten()?;
            match &found {
                Some(seen) if *seen != format => return None,
                _ => found = Some(format),
            }
        }
    }
    found
}

/// The references a formula's *value* is carried from, or `None` when the value is of another
/// kind than any input (a count, a month, a comparison, a lookup's column, a call this table does
/// not know). A literal carries nothing and does not spoil the rest: `IF(…;SUM(D2:D9);"")` is
/// carried from `D2:D9`.
fn union<'a>(items: &[&'a Expr]) -> Option<Vec<&'a Reference>> {
    let mut all = Vec::new();
    for item in items {
        all.extend(carried(item)?);
    }
    Some(all)
}

fn carried<'a>(expr: &'a Expr) -> Option<Vec<&'a Reference>> {
    match expr {
        Expr::Ref(reference) => Some(vec![reference]),
        Expr::Number(_) | Expr::Text(_) | Expr::Empty => Some(Vec::new()),
        Expr::Paren(inner) | Expr::Prefix(_, inner) => carried(inner),
        Expr::Binary(op, left, right) => match op {
            Op::Add | Op::Sub | Op::Mul | Op::Div => union(&[left.as_ref(), right.as_ref()]),
            _ => None,
        },
        Expr::Call { name, args } => {
            let pick = |indices: &[usize]| -> Vec<&'a Expr> {
                indices.iter().filter_map(|&i| args.get(i)).collect()
            };
            let all: Vec<&Expr> = args.iter().collect();
            match name.as_str() {
                "SUM"
                | "MIN"
                | "MAX"
                | "AVERAGE"
                | "MEDIAN"
                | "ROUND"
                | "ROUNDUP"
                | "ROUNDDOWN"
                | "TRUNC"
                | "INT"
                | "ABS"
                | "MROUND"
                | "CEILING"
                | "FLOOR"
                | "COM.MICROSOFT.CEILING"
                | "COM.MICROSOFT.FLOOR"
                | "EOMONTH"
                | "EDATE"
                | "WORKDAY" => union(&all),
                "LARGE" | "SMALL" | "INDEX" | "IFERROR" | "IFNA" => union(&pick(&[0, 1])),
                "SUBTOTAL" => union(&pick(&[1])),
                "IF" => union(&pick(&[1, 2])),
                "CHOOSE" => union(&all[1.min(all.len())..]),
                "SUMIF" | "AVERAGEIF" => match args.len() {
                    3 => union(&pick(&[2])),
                    _ => union(&pick(&[0])),
                },
                "SUMIFS" | "AVERAGEIFS" => union(&pick(&[0])),
                _ => None,
            }
        }
        Expr::Postfix(..) | Expr::Error(_) | Expr::Name(_) => None,
    }
}

/// The formula in `start`, and in every other cell of the range with its relative references
/// moved by that cell's offset ([`shift::shift`], what a fill does), entered as **one** batch by
/// the typing rule — so taking back an applied answer is one Ctrl+Z, not a formula and a fill.
fn put(
    app: &App,
    sheet: usize,
    start: Pos,
    end: Pos,
    canonical: &str,
    format: Option<&Format>,
) -> Result<(), String> {
    // The format first, so that one Ctrl+Z takes the formulas — everything that shows — away.
    if let Some(format) = format {
        app.set_format(sheet, start, end, Some(format.clone()))
            .map_err(|e| e.to_string())?;
    }
    let expr = parse::parse(canonical).map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    for row in start.row..=end.row {
        let mut cells = Vec::new();
        for col in start.col..=end.col {
            let moved = shift::shift(
                &expr,
                i64::from(row - start.row),
                i64::from(col - start.col),
            );
            // The typing rule takes the stored spelling, as every shell hands it after
            // `from_display`.
            cells.push(format!("={moved}"));
        }
        rows.push(cells);
    }
    app.enter_range(sheet, start, &rows, RecalcMode::Document)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::RecalcMode;

    #[test]
    fn the_prompt_says_where_it_goes_and_how_to_answer() {
        let app = App::new();
        let rows: Vec<Vec<String>> = vec![
            vec!["Item".into(), "Price".into()],
            vec!["Tea".into(), "3.5".into()],
            vec!["Cake".into(), "4".into()],
        ];
        app.enter_range(0, Pos { row: 0, col: 0 }, &rows, RecalcMode::No)
            .unwrap();
        let prompt = copy_prompt(
            &app,
            0,
            Pos { row: 1, col: 2 },
            Pos { row: 2, col: 2 },
            "Price with 19% VAT",
            false,
        )
        .unwrap();
        assert!(
            prompt.contains("Sheet \"Sheet1\": a table in A1:B3"),
            "{prompt}"
        );
        assert!(prompt.contains("## What I want\n\nPrice with 19% VAT\n"));
        assert!(prompt.contains("goes in C2 of sheet \"Sheet1\" and will be filled to C2:C3"));
        assert!(prompt.contains("SUMIF, SUMIFS, SUMPRODUCT"));
        assert!(
            !prompt.contains("Tea"),
            "values were asked to stay home: {prompt}"
        );
    }

    #[test]
    fn an_answer_is_tried_on_a_copy_and_a_running_total_sees_its_neighbours() {
        let app = App::new();
        let rows: Vec<Vec<String>> = ["Amount", "10", "5", "-3"]
            .iter()
            .map(|r| vec![(*r).to_owned()])
            .collect();
        app.enter_range(0, Pos { row: 0, col: 0 }, &rows, RecalcMode::No)
            .unwrap();
        let (b2, b4) = (Pos { row: 1, col: 1 }, Pos { row: 3, col: 1 });
        // Excel's spelling of a running balance: each row adds its amount to the row above.
        let tried = try_answer(&app, 0, b2, b4, "=SUM(B1,A2)").unwrap();
        assert_eq!(tried.spelling, Spelling::Excel);
        assert_eq!(tried.formula, "=SUM(B1;A2)");
        let shown: Vec<&str> = tried.values.iter().map(|(_, v)| v.as_str()).collect();
        assert_eq!(shown, ["10", "15", "12"]);
        // The document was not touched.
        assert_eq!(app.formula(0, b2).unwrap(), None);
        apply_answer(&app, 0, b2, b4, &tried).unwrap();
        assert_eq!(app.value_text(0, b4).unwrap(), "12");
        assert_eq!(app.input_text(0, b4).unwrap(), "=SUM(B3;A4)");
        // One step back takes the whole answer away.
        assert!(app.undo());
        assert_eq!(app.formula(0, b2).unwrap(), None);
        assert_eq!(app.formula(0, b4).unwrap(), None);
        assert_eq!(app.value_text(0, Pos { row: 3, col: 0 }).unwrap(), "-3");
        assert!(try_answer(&app, 0, b2, b4, "=SUM(").is_err());
    }

    #[test]
    fn a_result_takes_the_format_of_what_it_reads_when_there_is_one() {
        let app = App::new();
        let euro = grind_sheet::numfmt::preset(grind_sheet::numfmt::Kind::Currency, 2, true, "€");
        let a = |row| Pos { row, col: 0 };
        app.set_format(0, a(1), a(3), Some(euro.clone())).unwrap();
        let rows: Vec<Vec<String>> = ["Amount", "10", "5", "-3"]
            .iter()
            .map(|r| vec![(*r).to_owned()])
            .collect();
        app.enter_range(0, a(0), &rows, RecalcMode::No).unwrap();
        let b2 = Pos { row: 1, col: 1 };
        let tried = try_answer(&app, 0, b2, b2, "=SUM(A2:A4)").unwrap();
        assert_eq!(tried.format, Some(euro));
        assert_eq!(
            tried.values[0].1,
            app.value_text(0, a(1)).unwrap().replace("10", "12")
        );
        // Over a plain number as well: two formats, so none.
        app.enter(0, Pos { row: 5, col: 0 }, "7", RecalcMode::No)
            .unwrap();
        assert_eq!(try_answer(&app, 0, b2, b2, "=A2+A6").unwrap().format, None);
        // Applied: one step back takes the formula; the format left behind is on an empty cell.
        apply_answer(&app, 0, b2, b2, &tried).unwrap();
        assert!(app.format_at(0, b2).unwrap().is_some());
        assert!(app.undo());
        assert_eq!(app.formula(0, b2).unwrap(), None);
    }
}
