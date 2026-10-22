// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A sheet described by its **shape** rather than its cells: which rows are the header and the
//! data, and for each column its header, its type and a hint of what is in it.
//!
//! The glance of [`grind_sheet::assist_context`] shows a model every cell it can, which is what a
//! small local model needs and what nobody should paste into somebody else's service. This is the
//! other end: a few lines a person can read, check and copy into any model they choose, carrying
//! the headers, the types and — only when asked for — a few example values.
//!
//! Read entirely through `App`'s public reads; nothing here is stored.

use std::fmt;

use grind_sheet::numfmt::Kind as Format;
use grind_sheet::{App, CellValue, Pos, a1};

/// Rows looked at per column to decide its type and gather its examples. A column whose type
/// changes after row 2000 is described by its first 2000.
const SCAN: u32 = 2000;
/// Distinct text values listed before `…`.
const EXAMPLES: usize = 4;
/// A table this small is shown whole when values may be shown: every row, numbered, so a form of
/// labels and values (`Tiles per box | 12`) says which cell holds what — something a column's
/// type and range cannot.
const WHOLE_ROWS: u32 = 30;
const WHOLE_COLS: u32 = 8;

/// One sheet's table.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub sheet: String,
    /// The header row, 0-based, when the first row reads as one.
    pub header: Option<u32>,
    /// The data rows, 0-based, inclusive.
    pub first: u32,
    pub last: u32,
    pub columns: Vec<Column>,
    /// Every data row as the cells show it, when the table is small enough and values may be
    /// shown — `(row, cells)`, the row 0-based.
    pub rows: Option<Vec<(u32, Vec<String>)>>,
}

/// One column of a [`Table`].
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub col: u32,
    pub header: Option<String>,
    pub kind: Kind,
    /// Distinct values, or the lowest and highest, as the cells show them. Empty when the
    /// description was asked for without values.
    pub examples: Vec<String>,
    /// How many distinct texts there are, for a text column.
    pub distinct: Option<usize>,
    /// The formula in the first data row, in display syntax, when the column is computed.
    pub formula: Option<String>,
    /// Empty cells among the data rows.
    pub blanks: u32,
}

/// What a column holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Date,
    Time,
    /// A number shown as money, with the currency's symbol.
    Currency(String),
    Percent,
    Number,
    Text,
    Logical,
    /// Several of the above.
    Mixed,
    Empty,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Date => write!(f, "date"),
            Kind::Time => write!(f, "time"),
            Kind::Currency(symbol) if symbol.is_empty() => write!(f, "money"),
            Kind::Currency(symbol) => write!(f, "money ({symbol})"),
            Kind::Percent => write!(f, "percentage"),
            Kind::Number => write!(f, "number"),
            Kind::Text => write!(f, "text"),
            Kind::Logical => write!(f, "TRUE/FALSE"),
            Kind::Mixed => write!(f, "mixed"),
            Kind::Empty => write!(f, "empty"),
        }
    }
}

/// The table on `sheet`, or `None` for an empty sheet. `values` decides whether examples are
/// gathered at all.
pub fn describe(app: &App, sheet: usize, values: bool) -> Option<Table> {
    let (rows, cols) = app.used_extent(sheet).ok()?;
    if rows == 0 || cols == 0 {
        return None;
    }
    let get = |row, col| app.get(sheet, Pos { row, col }).unwrap_or_default();
    // A header is a first row of text over at least one column that is not text below it.
    let is_text = |v: &CellValue| matches!(v, CellValue::Text(_));
    let header = (rows > 1
        && (0..cols).all(|c| matches!(get(0, c), CellValue::Text(_) | CellValue::Empty))
        && (0..cols).any(|c| {
            let below = get(1, c);
            !below.is_empty() && !is_text(&below)
        }))
    .then_some(0);
    let first = header.map_or(0, |h| h + 1);
    // The used extent counts formatted blanks — a column formatted as dates to row 1000 ahead of
    // its data — so the table ends at the last row that holds something.
    let last = (first..rows)
        .rev()
        .find(|&row| (0..cols).any(|col| !get(row, col).is_empty()))
        .unwrap_or(first);
    let columns = (0..cols)
        .map(|col| column(app, sheet, col, header, first, last, values))
        .collect();
    let small = last - first < WHOLE_ROWS && cols <= WHOLE_COLS;
    let rows = (values && small).then(|| {
        (first..=last)
            .map(|row| {
                let cells = (0..cols)
                    .map(|col| cut(&app.value_text(sheet, Pos { row, col }).unwrap_or_default()))
                    .collect();
                (row, cells)
            })
            .collect()
    });
    Some(Table {
        sheet: app.sheet_name(sheet).ok()?,
        header,
        first,
        last,
        columns,
        rows,
    })
}

fn column(
    app: &App,
    sheet: usize,
    col: u32,
    header: Option<u32>,
    first: u32,
    last: u32,
    values: bool,
) -> Column {
    let at = |row| Pos { row, col };
    let mut kinds: Vec<Kind> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let mut low: Option<(f64, Pos)> = None;
    let mut high: Option<(f64, Pos)> = None;
    let mut blanks = 0;
    for row in first..=last.min(first + SCAN - 1) {
        let value = app.get(sheet, at(row)).unwrap_or_default();
        let format = app.format_at(sheet, at(row)).ok().flatten();
        let kind = match (&value, format.as_ref().map(|f| f.preset_params())) {
            (CellValue::Empty, _) => {
                blanks += 1;
                continue;
            }
            (CellValue::Text(t), _) => {
                if !texts.contains(t) {
                    texts.push(t.clone());
                }
                Kind::Text
            }
            (CellValue::Bool(_), _) => Kind::Logical,
            (CellValue::Number(n), format) => {
                if low.is_none_or(|(l, _)| *n < l) {
                    low = Some((*n, at(row)));
                }
                if high.is_none_or(|(h, _)| *n > h) {
                    high = Some((*n, at(row)));
                }
                match format.map(|(kind, _, _, symbol)| (kind, symbol)) {
                    Some((Format::Date, _)) => Kind::Date,
                    Some((Format::Time, _)) => Kind::Time,
                    Some((Format::Currency, symbol)) => Kind::Currency(symbol),
                    Some((Format::Percentage, _)) => Kind::Percent,
                    // A date with no format of its own still types back as ISO.
                    _ if is_iso_date(&app.input_text(sheet, at(row)).unwrap_or_default()) => {
                        Kind::Date
                    }
                    _ => Kind::Number,
                }
            }
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    let kind = match kinds.len() {
        0 => Kind::Empty,
        1 => kinds.remove(0),
        _ => Kind::Mixed,
    };
    let shown = |pos: Pos| app.value_text(sheet, pos).unwrap_or_default();
    let examples = match (values, &kind) {
        (false, _) => Vec::new(),
        (true, Kind::Text | Kind::Mixed) => {
            let mut some: Vec<String> = texts.iter().take(EXAMPLES).map(|t| cut(t)).collect();
            if texts.len() > EXAMPLES {
                some.push("…".into());
            }
            some
        }
        (true, Kind::Logical | Kind::Empty) => Vec::new(),
        (true, _) => match (low, high) {
            (Some((_, l)), Some((_, h))) if l != h => vec![shown(l), shown(h)],
            (Some((_, l)), _) => vec![shown(l)],
            _ => Vec::new(),
        },
    };
    let formula = app
        .formula(sheet, at(first))
        .ok()
        .flatten()
        .and_then(|_| app.input_text(sheet, at(first)).ok());
    Column {
        col,
        header: header.and_then(|h| match app.get(sheet, at(h)).unwrap_or_default() {
            CellValue::Text(t) => Some(t),
            _ => None,
        }),
        distinct: matches!(kind, Kind::Text).then_some(texts.len()),
        kind,
        examples,
        formula,
        blanks,
    }
}

fn is_iso_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() >= 10 && b[4] == b'-' && b[7] == b'-' && b[..4].iter().all(u8::is_ascii_digit)
}

fn cut(text: &str) -> String {
    match text.char_indices().nth(30) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

impl fmt::Display for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let corner = |row, col| a1::format(None, Pos { row, col });
        let right = self.columns.last().map_or(0, |c| c.col);
        let top = self.header.unwrap_or(self.first);
        write!(
            f,
            "Sheet \"{}\": a table in {}:{}",
            self.sheet,
            corner(top, 0),
            corner(self.last, right)
        )?;
        match self.header {
            Some(h) => writeln!(
                f,
                " — row {} is the header, rows {}–{} are data ({} rows).",
                h + 1,
                self.first + 1,
                self.last + 1,
                self.last - self.first + 1
            )?,
            None => writeln!(
                f,
                " — no header row, rows {}–{}.",
                self.first + 1,
                self.last + 1
            )?,
        }
        for c in &self.columns {
            let letter = a1::format(None, Pos { row: 0, col: c.col });
            let letter = letter.trim_end_matches('1');
            let header = c.header.as_deref().unwrap_or("(no header)");
            write!(f, "  {letter:<3}{header:<20} {}", c.kind)?;
            if let Some(formula) = &c.formula {
                write!(f, ", computed: {formula}")?;
            } else if !c.examples.is_empty() {
                let joined = match c.kind {
                    Kind::Text | Kind::Mixed => c.examples.join(", "),
                    _ => c.examples.join(" … "),
                };
                write!(f, ": {joined}")?;
                if let Some(n) = c.distinct.filter(|n| *n > EXAMPLES) {
                    write!(f, " ({n} different)")?;
                }
            }
            if c.blanks > 0 && c.kind != Kind::Empty {
                write!(f, "; {} empty", c.blanks)?;
            }
            writeln!(f)?;
        }
        if let Some(rows) = &self.rows {
            writeln!(f, "  Every row (row number, then each column):")?;
            for (row, cells) in rows {
                writeln!(f, "  {:>4}  {}", row + 1, cells.join(" | "))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::RecalcMode;

    fn ledger() -> App {
        let app = App::new();
        let rows: Vec<Vec<String>> = [
            "Date\tCategory\tAmount\tCleared",
            "2026-09-01\tIncome\t3250\tTRUE",
            "2026-09-03\tGroceries\t-86.4\tTRUE",
            "2026-10-02\tGroceries\t-40\tFALSE",
        ]
        .iter()
        .map(|r| r.split('\t').map(str::to_owned).collect())
        .collect();
        // An ISO date is read as a date only into a cell already formatted as one.
        let date = grind_sheet::numfmt::preset(Format::Date, 0, false, "");
        let euro = grind_sheet::numfmt::preset(Format::Currency, 2, true, "€");
        let (a2, a4) = (Pos { row: 1, col: 0 }, Pos { row: 3, col: 0 });
        app.set_format(0, a2, a4, Some(date)).unwrap();
        app.set_format(
            0,
            Pos { row: 1, col: 2 },
            Pos { row: 3, col: 2 },
            Some(euro),
        )
        .unwrap();
        app.enter_range(0, Pos { row: 0, col: 0 }, &rows, RecalcMode::No)
            .unwrap();
        app
    }

    #[test]
    fn a_ledger_reads_as_a_header_and_typed_columns() {
        let table = describe(&ledger(), 0, true).unwrap();
        assert_eq!((table.header, table.first, table.last), (Some(0), 1, 3));
        let kinds: Vec<&Kind> = table.columns.iter().map(|c| &c.kind).collect();
        assert_eq!(
            kinds,
            [
                &Kind::Date,
                &Kind::Text,
                &Kind::Currency("€".into()),
                &Kind::Logical
            ]
        );
        let text = table.to_string();
        assert!(text.starts_with("Sheet \"Sheet1\": a table in A1:D4 — row 1 is the header"));
        assert!(text.contains("A  Date"), "{text}");
        assert!(text.contains("text: Income, Groceries"), "{text}");
        assert!(text.contains("money (€): "), "{text}");
        // Small, so shown whole, row by row.
        assert!(text.contains("     3  "), "{text}");
        assert!(text.contains("| Groceries |"), "{text}");
        assert!(text.contains("date: "), "{text}");
    }

    #[test]
    fn without_values_nothing_but_the_shape_is_described() {
        let text = describe(&ledger(), 0, false).unwrap().to_string();
        assert!(
            !text.contains("Groceries") && !text.contains("3250"),
            "{text}"
        );
        assert!(text.contains("Category"), "{text}");
        assert!(!text.contains("Every row"), "{text}");
    }
}
