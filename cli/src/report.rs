// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the CLI prints. `main.rs` drives the core; this decides how the answer looks.
//!
//! JSON is **untagged**: one bare object, no envelope, because the caller already knows
//! which subcommand it ran. `changed` and `written` are the load-bearing pair — an agent
//! asserts on those rather than parsing prose, which is the whole reason `--format json`
//! exists. `changed` means the command did something; `written` means the disk was touched,
//! and it is false under `--dry-run` and false for a no-op. **A no-op is a success.**

use clap::ValueEnum;
use serde::Serialize;
use sheet_core::CellValue;
use sheet_core::formula::value::format_number;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Format {
    Text,
    Json,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Report {
    Cells(CellsReport),
    Document(DocumentReport),
    Text(TextReport),
}

/// `get` and `view`.
#[derive(Debug, Serialize)]
pub struct CellsReport {
    pub path: String,
    pub sheet: String,
    /// Whether the text output prints stored values rather than display text (`--raw`).
    /// Not serialised: JSON carries both spellings of every cell and lets the caller pick.
    #[serde(skip)]
    pub raw: bool,
    /// Row-major, one entry per cell in the requested rectangle.
    pub cells: Vec<Cell>,
    pub rows: u32,
    pub cols: u32,
}

#[derive(Debug, Serialize)]
pub struct Cell {
    #[serde(rename = "ref")]
    pub address: String,
    /// The stored value, spelled the way the file stores it. A script computing with a
    /// cell wants this one, whatever the document's number format says.
    pub value: String,
    /// What the cell *displays*, its number format applied — the same text a spreadsheet
    /// would show in that cell.
    pub text: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
}

/// Every mutating command, plus `new` and `info`.
#[derive(Debug, Serialize)]
pub struct DocumentReport {
    pub path: String,
    pub changed: bool,
    pub written: bool,
    pub sheets: Vec<SheetInfo>,
    pub names: Vec<Name>,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Serialize)]
pub struct SheetInfo {
    pub name: String,
    pub rows: u32,
    pub cols: u32,
    pub formulas: usize,
}

#[derive(Debug, Serialize)]
pub struct Name {
    pub name: String,
    pub expression: String,
}

/// `fmt` and `functions` — output that is not about a document.
#[derive(Debug, Serialize)]
pub struct TextReport {
    pub lines: Vec<String>,
}

impl Report {
    pub fn print(&self, format: Format) {
        match format {
            Format::Json => println!(
                "{}",
                serde_json::to_string(self).expect("report is serializable")
            ),
            Format::Text => self.print_text(),
        }
    }

    fn print_text(&self) {
        match self {
            // Tab-separated and nothing else, so `view` composes with cut, awk and friends.
            // One cell prints as one bare field, which is what `get` wants.
            Report::Cells(cells) => {
                for row in cells.cells.chunks(cells.cols.max(1) as usize) {
                    let line: Vec<&str> = row
                        .iter()
                        .map(|c| match cells.raw {
                            true => c.value.as_str(),
                            false => c.text.as_str(),
                        })
                        .collect();
                    println!("{}", line.join("\t"));
                }
            }
            Report::Document(doc) => {
                for sheet in &doc.sheets {
                    println!(
                        "{}\t{} rows\t{} cols\t{} formulas",
                        sheet.name, sheet.rows, sheet.cols, sheet.formulas
                    );
                }
                for name in &doc.names {
                    println!("{}\t{}", name.name, name.expression);
                }
                println!(
                    "{}{}{}{}{}",
                    doc.path,
                    if doc.changed { "" } else { "  (no change)" },
                    if doc.written { "" } else { "  (not written)" },
                    if doc.can_undo { "  undo" } else { "" },
                    if doc.can_redo { "  redo" } else { "" },
                );
            }
            Report::Text(text) => {
                for line in &text.lines {
                    println!("{line}");
                }
            }
        }
    }
}

/// A cell as text. Numbers go through the formula engine's own formatter rather than `{}`,
/// so the CLI shows the same 15 significant digits the writer will store.
pub fn show(value: &CellValue) -> String {
    match value {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => format_number(*n),
        CellValue::Text(s) => s.clone(),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
    }
}

/// The value type, named as ODF names it — `office:value-type`, not a Rust variant.
pub fn kind(value: &CellValue) -> &'static str {
    match value {
        CellValue::Empty => "empty",
        CellValue::Number(_) => "float",
        CellValue::Text(_) => "string",
        CellValue::Bool(_) => "boolean",
    }
}
