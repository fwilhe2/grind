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

use grind_sheet::formula::funcs;
use grind_sheet::{App, Pos, a1};

use crate::shape;

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
}
