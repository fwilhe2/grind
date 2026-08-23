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

use std::fmt;

use clap::ValueEnum;
use grind_sheet::CellValue;
use grind_sheet::formula::value::format_number;
use grind_sheet::numfmt;
use grind_sheet::style::{CellStyle, EDGES};
use serde::Serialize;

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
    /// Boxed: a `CellStyle` and a `Format` together are five times the next variant, and
    /// every command would carry the difference.
    CellStyle(Box<CellStyleReport>),
    Text(TextReport),
    /// A text document's shape — `grind info` over a `.odt`, and every `grind text` command
    /// that writes.
    TextDocument(TextDocumentReport),
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
    /// What kind of document this is — `"spreadsheet"`, `"text document"`.
    ///
    /// Only `grind info` fills it in. Every other report comes from a command that already
    /// named the kind by being under `grind sheet`, and printing it there would put a word
    /// nobody asked for at the top of every `set`. Skipped in JSON when absent for the same
    /// reason `stale` is: a field every consumer has to ignore is worse than no field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    pub changed: bool,
    pub written: bool,
    /// Formula cells whose cached value a recalculation would change — a document that
    /// disagrees with itself. Editing a cell a formula reads does this without touching the
    /// formula's own cell, and ODF has no dirty bit to write, so it has to be *reported*.
    /// Zero for the overwhelmingly common case, and `#[serde(skip_serializing_if)]` keeps it
    /// out of the JSON then rather than adding a field every consumer has to ignore.
    #[serde(skip_serializing_if = "is_zero")]
    pub stale: usize,
    pub sheets: Vec<SheetInfo>,
    pub names: Vec<Name>,
    pub can_undo: bool,
    pub can_redo: bool,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
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

/// `style --show` and `format --show` — how one cell looks, and how its value is spelled.
///
/// One report for both because a shell reading either wants the same shape, and because the
/// two travel on one `style:style` in the file. JSON carries the structures themselves — a
/// picker restoring its state wants the fields, not prose — and the text form is one
/// `key<TAB>value` line per thing that is set, which is `sheet format`'s own flags for a
/// format and ODF's own attribute names for a style.
#[derive(Debug, Serialize)]
pub struct CellStyleReport {
    pub path: String,
    pub sheet: String,
    #[serde(rename = "ref")]
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<CellStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<numfmt::Format>,
}

/// `fmt` and `functions` — output that is not about a document.
#[derive(Debug, Serialize)]
pub struct TextReport {
    pub lines: Vec<String>,
}

/// What a text document is and what is in it.
///
/// A separate variant rather than fields bolted onto [`DocumentReport`]: a spreadsheet has
/// sheets and a text document has an outline, and a struct carrying both with one half always
/// empty is a shape that lies to whichever consumer reads it second.
#[derive(Debug, Serialize)]
pub struct TextDocumentReport {
    pub path: String,
    /// Present only for `grind info`, for the reason [`DocumentReport::kind`] gives.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    pub changed: bool,
    pub written: bool,
    pub blocks: usize,
    pub words: usize,
    pub characters: usize,
    pub headings: usize,
    pub bookmarks: Vec<String>,
    pub can_undo: bool,
    pub can_redo: bool,
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
                if let Some(kind) = doc.kind {
                    println!("{kind}");
                }
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
            Report::CellStyle(cell) => {
                for (key, value) in describe(cell) {
                    println!("{key}\t{value}");
                }
            }
            Report::Text(text) => {
                for line in &text.lines {
                    println!("{line}");
                }
            }
            Report::TextDocument(doc) => {
                if let Some(kind) = doc.kind {
                    println!("{kind}");
                }
                println!(
                    "{} blocks\t{} headings\t{} words\t{} characters",
                    doc.blocks, doc.headings, doc.words, doc.characters
                );
                for name in &doc.bookmarks {
                    println!("#{name}");
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
        }
    }
}

/// A `--show` report as `key<TAB>value` lines — only what is actually set, so a plain cell
/// prints nothing at all and a script can test for that.
///
/// A style prints under ODF's own attribute names, because those are what the fields *are*
/// (`core/src/style.rs`). A format prints under `sheet format`'s flag names, because those
/// are what recreates it — and `preset` says whether they can: a document may hold a format
/// this vocabulary cannot build (`DD.MM.YYYY`, a two-branch currency), and reporting its
/// decimals as though `sheet format` would reproduce it would be a lie.
fn describe(cell: &CellStyleReport) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut set =
        |key: &str, value: &dyn fmt::Display| out.push((key.to_owned(), value.to_string()));

    if let Some(format) = &cell.format {
        let (kind, decimals, grouping, symbol) = format.preset_params();
        set("kind", &format!("{kind:?}").to_lowercase());
        // The three numeric families are the ones whose digits and separators mean anything.
        if matches!(
            kind,
            numfmt::Kind::Number | numfmt::Kind::Percentage | numfmt::Kind::Currency
        ) {
            set("decimals", &decimals);
            set("grouping", &grouping);
        }
        if !symbol.is_empty() {
            set("symbol", &symbol);
        }
        if let Some(locale) = &format.locale {
            set("locale", &locale.tag());
        }
        if !format.maps.is_empty() {
            set("branches", &format.maps.len());
        }
        set("preset", &format.is_preset());
    }

    if let Some(style) = &cell.style {
        for (key, value) in [
            ("fo:font-weight", &style.font_weight),
            ("fo:font-style", &style.font_style),
            ("fo:font-size", &style.font_size),
            ("fo:color", &style.color),
            ("fo:background-color", &style.background),
            ("fo:text-align", &style.align),
            ("style:vertical-align", &style.vertical_align),
            ("fo:wrap-option", &style.wrap),
        ] {
            if let Some(value) = value {
                set(key, value);
            }
        }
        // The shorthand when the edges agree, and the four attributes when they do not —
        // which is how the file spells it either way.
        match style.uniform_border() {
            Some(border) => set("fo:border", &border),
            None => {
                for (edge, value) in EDGES.iter().zip(&style.borders) {
                    if let Some(value) = value {
                        set(&format!("fo:border-{edge}"), value);
                    }
                }
            }
        }
    }
    out
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
