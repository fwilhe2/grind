// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a formula assistant is shown of a document (`doc/formula-assistant.md` §4.1, A0).
//!
//! The glance a person takes before writing a formula — which sheets there are, what the top of
//! each one holds, the rows just above the cell the formula goes in, the defined names and the
//! document's locale — as one bounded value. Its [`fmt::Display`] is the text a model is given,
//! **exactly**: decision 8 of that document is that nothing about a document reaches a model that
//! `grind sheet assist` does not print, so there is one rendering and the CLI prints it.
//!
//! Bounded by construction, not by the document: a sheet shows at most [`HEAD_ROWS`] rows from
//! its top, [`NEAR_ROWS`] more above the target cell when that is further down, [`MAX_COLS`]
//! columns and [`MAX_TEXT`] characters of any one cell, and a document at most [`MAX_SHEETS`]
//! sheets — the target's always among them. What is left out is *counted* and said, so a model is
//! never handed half a table as though it were all of it.
//!
//! The target comes **last**, after the document, so that everything before it is the same for
//! every cell of one document — which is what lets a model server keep it evaluated.
//!
//! Every cell is shown as [`App::input_text`] would put it in front of a person — a formula in
//! display syntax, a date in ISO, a number in the document's own spelling — since that is the
//! spelling a formula written back is read in.
//!
//! [`App::input_text`]: crate::App::input_text

use std::fmt;

use serde::Serialize;

use crate::model::{CellValue, Document, Pos};
use crate::{Error, Result, a1, formula, input_of};

/// Rows shown from the top of every sheet.
pub const HEAD_ROWS: u32 = 40;
/// Rows shown above the target cell, on its own sheet, when it lies below [`HEAD_ROWS`].
pub const NEAR_ROWS: u32 = 10;
/// Columns shown of any sheet.
pub const MAX_COLS: u32 = 26;
/// Sheets shown of a document. The target's own sheet is always one of them.
pub const MAX_SHEETS: usize = 16;
/// Characters shown of any one cell; a longer one ends in `…`.
pub const MAX_TEXT: usize = 80;

/// The glance. Built by [`crate::App::assist_context`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AssistContext {
    /// Where the formula goes, sheet-qualified: `Samples.C5`.
    pub at: String,
    /// The document's own locale (`de-DE`), when it states one.
    pub locale: Option<String>,
    /// The sheets shown, in document order.
    pub sheets: Vec<SheetGlance>,
    /// The names of the sheets left out, in document order.
    pub sheets_left_out: Vec<String>,
    /// Every named expression: its name and its definition in display syntax.
    pub names: Vec<(String, String)>,
}

/// One sheet, as much of it as the bounds allow.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetGlance {
    pub name: String,
    /// The sheet's whole used extent, shown or not: rows and columns.
    pub rows: u32,
    pub cols: u32,
    /// The rows shown, top to bottom, each with its non-empty cells left to right. Rows with
    /// nothing in them are left out of the list but not out of the count.
    pub shown: Vec<Vec<Glimpse>>,
    /// Rows of the used extent that are not shown.
    pub rows_left_out: u32,
    /// Columns of the used extent that are not shown.
    pub cols_left_out: u32,
}

/// One cell.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Glimpse {
    /// `B7`, without the sheet.
    pub address: String,
    pub kind: Kind,
    /// As a person would see it in the formula bar, cut at [`MAX_TEXT`].
    pub input: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Number,
    Text,
    Logical,
    Formula,
}

pub(crate) fn build(doc: &Document, sheet: usize, at: Pos) -> Result<AssistContext> {
    let target = doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
    let count = doc.sheets.len();
    // The first MAX_SHEETS in document order, with the target's own sheet swapped in for the
    // last of them when it is further along.
    let mut chosen: Vec<usize> = (0..count.min(MAX_SHEETS)).collect();
    if !chosen.contains(&sheet) {
        chosen.pop();
        chosen.push(sheet);
    }
    let mut sheets = Vec::new();
    for &index in &chosen {
        sheets.push(glance(doc, index, (index == sheet).then_some(at))?);
    }
    let names = doc
        .names
        .iter()
        .map(|(name, expr)| {
            let shown = formula::display::to_display(expr)
                .map(|s| s.trim_start_matches('=').to_owned())
                .unwrap_or_else(|_| expr.clone());
            (name.clone(), shown)
        })
        .collect();
    Ok(AssistContext {
        at: a1::format(Some(&target.name), at),
        locale: doc.locale.as_ref().map(|l| l.tag()),
        sheets,
        sheets_left_out: (0..count)
            .filter(|i| !chosen.contains(i))
            .map(|i| doc.sheets[i].name.clone())
            .collect(),
        names,
    })
}

fn glance(doc: &Document, index: usize, at: Option<Pos>) -> Result<SheetGlance> {
    let sheet = &doc.sheets[index];
    let (rows, cols) = (sheet.used_rows(), sheet.used_cols());
    let mut wanted: Vec<u32> = (0..rows.min(HEAD_ROWS)).collect();
    if let Some(at) = at {
        // The rows just above the target and the target's own, in case it already holds
        // something; never past the used extent unless the target itself is.
        let from = at.row.saturating_sub(NEAR_ROWS).max(HEAD_ROWS);
        wanted.extend((from..=at.row).filter(|r| *r < rows));
    }
    let shown_cols = cols.min(MAX_COLS);
    let mut shown = Vec::new();
    for &row in &wanted {
        let mut cells = Vec::new();
        for col in 0..shown_cols {
            let pos = Pos { row, col };
            let kind = match (sheet.formula(pos), sheet.get(pos)) {
                (Some(_), _) => Kind::Formula,
                (None, CellValue::Empty) => continue,
                (None, CellValue::Number(_)) => Kind::Number,
                (None, CellValue::Text(_)) => Kind::Text,
                (None, CellValue::Bool(_)) => Kind::Logical,
            };
            cells.push(Glimpse {
                address: a1::format(None, pos),
                kind,
                input: cut(input_of(doc, index, pos)?),
            });
        }
        if !cells.is_empty() {
            shown.push(cells);
        }
    }
    Ok(SheetGlance {
        name: sheet.name.clone(),
        rows,
        cols,
        shown,
        rows_left_out: rows - wanted.len() as u32,
        cols_left_out: cols - shown_cols,
    })
}

fn cut(text: String) -> String {
    match text.char_indices().nth(MAX_TEXT) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    }
}

impl fmt::Display for AssistContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.locale {
            Some(locale) => writeln!(f, "The document's locale is {locale}.")?,
            None => writeln!(f, "The document states no locale.")?,
        }
        for sheet in &self.sheets {
            writeln!(f)?;
            write!(f, "{sheet}")?;
        }
        if !self.sheets_left_out.is_empty() {
            writeln!(f)?;
            writeln!(f, "Sheets not shown: {}.", self.sheets_left_out.join(", "))?;
        }
        if !self.names.is_empty() {
            writeln!(f)?;
            writeln!(f, "Named expressions:")?;
            for (name, definition) in &self.names {
                writeln!(f, "  {name} = {definition}")?;
            }
        }
        // Last, after everything that is the same wherever in the document the formula goes: a
        // server keeps the longest prefix it has already read, so a second question about the
        // same document costs the words that differ and not the document again.
        writeln!(f)?;
        writeln!(f, "The formula goes in {}.", self.at)
    }
}

impl fmt::Display for SheetGlance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Sheet {}: {} rows, {} columns",
            self.name, self.rows, self.cols
        )?;
        if self.shown.is_empty() {
            writeln!(f, "  (empty)")?;
        }
        for row in &self.shown {
            let cells: Vec<String> = row.iter().map(Glimpse::to_string).collect();
            writeln!(f, "  {}", cells.join(" | "))?;
        }
        if self.rows_left_out > 0 {
            writeln!(f, "  … {} more rows not shown", self.rows_left_out)?;
        }
        if self.cols_left_out > 0 {
            writeln!(f, "  … {} more columns not shown", self.cols_left_out)?;
        }
        Ok(())
    }
}

impl fmt::Display for Glimpse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            // Quoted the way a formula quotes a string, so `"3"` the text and `3` the number
            // cannot be read as one another.
            Kind::Text => write!(
                f,
                "{} \"{}\"",
                self.address,
                self.input.trim_start_matches('\'').replace('"', "\"\"")
            ),
            _ => write!(f, "{} {}", self.address, self.input),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, RecalcMode};

    fn pos(row: u32, col: u32) -> Pos {
        Pos { row, col }
    }

    fn app() -> App {
        let app = App::new();
        app.enter(0, pos(0, 0), "Item", RecalcMode::No).unwrap();
        app.enter(0, pos(0, 1), "Price", RecalcMode::No).unwrap();
        app.enter(0, pos(1, 0), "Tea", RecalcMode::No).unwrap();
        app.enter(0, pos(1, 1), "3.5", RecalcMode::No).unwrap();
        app.enter(0, pos(2, 0), "'007", RecalcMode::No).unwrap();
        app.enter(0, pos(2, 1), "=B2*2", RecalcMode::No).unwrap();
        app
    }

    #[test]
    fn every_kind_of_cell_is_shown_as_it_is_typed() {
        let context = app().assist_context(0, pos(3, 1)).unwrap();
        assert_eq!(context.at, "Sheet1.B4");
        let text = context.to_string();
        assert!(text.contains("A1 \"Item\" | B1 \"Price\""), "{text}");
        assert!(text.contains("A2 \"Tea\" | B2 3.5"), "{text}");
        // Text that would read as a number is still quoted text, without the `'` that forces it.
        assert!(text.contains("A3 \"007\" | B3 =B2*2"), "{text}");
        assert!(
            text.ends_with("\nThe formula goes in Sheet1.B4.\n"),
            "{text}"
        );
    }

    #[test]
    fn names_are_shown_in_display_syntax() {
        let app = app();
        app.set_name("price", "[$Sheet1.$B$2]").unwrap();
        let context = app.assist_context(0, pos(3, 1)).unwrap();
        assert_eq!(context.names, vec![("price".into(), "$Sheet1.$B$2".into())]);
    }

    #[test]
    fn a_long_sheet_is_bounded_and_says_so() {
        let app = App::new();
        let rows: Vec<Vec<String>> = (0..5000).map(|r| vec![r.to_string()]).collect();
        app.enter_range(0, pos(0, 0), &rows, RecalcMode::No)
            .unwrap();
        let context = app.assist_context(0, pos(4000, 1)).unwrap();
        let sheet = &context.sheets[0];
        assert_eq!(sheet.rows, 5000);
        // The head, and the ten rows above the target with the target's own row.
        assert_eq!(sheet.shown.len() as u32, HEAD_ROWS + NEAR_ROWS + 1);
        assert_eq!(sheet.rows_left_out, 5000 - HEAD_ROWS - NEAR_ROWS - 1);
        assert_eq!(sheet.shown.last().unwrap()[0].address, "A4001");
        assert!(context.to_string().contains("more rows not shown"));
    }

    #[test]
    fn the_target_sheet_is_shown_however_far_along_it_is() {
        let app = App::new();
        for i in 1..=MAX_SHEETS + 3 {
            app.add_sheet(&format!("S{i}")).unwrap();
        }
        let last = app.sheet_count() - 1;
        let context = app.assist_context(last, pos(0, 0)).unwrap();
        assert_eq!(context.sheets.len(), MAX_SHEETS);
        assert_eq!(
            context.sheets.last().unwrap().name,
            format!("S{}", MAX_SHEETS + 3)
        );
        assert_eq!(context.sheets_left_out.len(), 4);
        assert!(
            !context
                .sheets_left_out
                .contains(&context.sheets.last().unwrap().name)
        );
    }

    #[test]
    fn a_long_cell_is_cut() {
        let app = App::new();
        app.enter(0, pos(0, 0), &"x".repeat(200), RecalcMode::No)
            .unwrap();
        let context = app.assist_context(0, pos(1, 0)).unwrap();
        let input = &context.sheets[0].shown[0][0].input;
        assert_eq!(input.chars().count(), MAX_TEXT + 1);
        assert!(input.ends_with('…'));
    }
}
