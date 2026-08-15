// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `sheet` — the CLI, and the parity ratchet.
//!
//! Every capability the core has is reachable from here, and `doc/cli-parity.md` plus
//! `tests/parity.rs` make that a build failure rather than a promise. A subcommand is one
//! `match` arm that drives [`App`]; anything longer than that belongs in the core.
//!
//! Diagnostics go to stderr, results to stdout, and an error never appears on stdout in
//! either format.

mod a1;
mod report;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use sheet_core::{App, CellValue, Pos, Session};

use report::{Cell, CellsReport, DocumentReport, Format, Name, Report, SheetInfo, TextReport};

#[derive(Parser)]
#[command(
    name = "sheet",
    version,
    about = "Drive the ODF spreadsheet core from the shell",
    long_about = "Drive the ODF spreadsheet core from the shell.\n\n\
        Cells are addressed in ODF reference syntax without the brackets: A1, $B$7, \
        A1:D20, Data.B2, 'Q3 Actuals'.A1:.C9. Without a sheet name the first sheet is \
        meant.\n\n\
        Formulas are stored verbatim in OpenFormula syntax, where a reference is bracketed \
        and arguments are separated by ';' — sheet set book.ods A3 '=SUM([.A1:.A2])'. \
        There is deliberately no translation from other spreadsheets' syntax.\n\n\
        Each invocation loads the file, applies one command and writes it back. Pass \
        --session to carry undo history across invocations; without it every command starts \
        with empty history."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// File holding undo history between invocations
    #[arg(long, global = true, value_name = "PATH")]
    session: Option<PathBuf>,

    /// Output format
    #[arg(long, global = true, value_enum, default_value = "text")]
    format: Format,

    /// Apply the command and report the result, but write nothing to disk
    #[arg(long, global = true)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Create an empty document
    New {
        file: PathBuf,
        /// Overwrite the file if it already exists
        #[arg(long)]
        force: bool,
    },

    /// Print a cell or a range
    Get {
        file: PathBuf,
        /// Cell address, e.g. A1 or Data.B2
        address: String,
        /// Print the formula source instead of the value
        #[arg(long)]
        formula: bool,
    },

    /// Print a rectangle of values, tab-separated
    View {
        file: PathBuf,
        /// Range to print; defaults to everything the sheet uses
        address: Option<String>,
        /// Stop after this many rows
        #[arg(long, default_value_t = 40)]
        max_rows: u32,
    },

    /// Set a cell's value or formula
    ///
    /// A leading '=' makes it a formula; otherwise the value is taken as a number, TRUE or
    /// FALSE, or text. "-" reads the value from stdin.
    Set {
        file: PathBuf,
        /// Cell address, e.g. A1 or Data.B2
        address: String,
        /// The value; hyphen-leading values are taken literally, so -1 is a number
        #[arg(allow_hyphen_values = true)]
        value: String,
        /// Store the value as text, whatever it looks like
        #[arg(long)]
        text: bool,
    },

    /// Clear a cell, leaving it empty
    Clear {
        file: PathBuf,
        /// Cell address, e.g. A1 or Data.B2
        address: String,
        /// Remove only the formula, keeping the value it last computed
        #[arg(long)]
        formula_only: bool,
    },

    /// Recalculate every formula in the document
    Recalc { file: PathBuf },

    /// Rewrite a document in the form the output extension names (.fods flat, else package)
    Convert { file: PathBuf, out: PathBuf },

    /// Report sheets, extents, formula counts and named expressions
    Info { file: PathBuf },

    /// Undo the last change recorded in the session
    Undo { file: PathBuf },

    /// Redo the last undone change recorded in the session
    Redo { file: PathBuf },

    /// Parse a formula and print it back in normalised form
    Fmt {
        /// The formula, e.g. '=SUM([.A1:.A2])'
        #[arg(allow_hyphen_values = true)]
        formula: String,
    },

    /// List the OpenFormula functions this build implements
    Functions,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(report) => {
            report.print(cli.format);
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("sheet: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<Report, String> {
    match &cli.command {
        Command::New { file, force } => {
            if file.exists() && !force {
                return Err(format!("{} exists; pass --force", file.display()));
            }
            let app = App::new();
            finish(&app, cli, file, true)
        }

        Command::Get {
            file,
            address,
            formula,
        } => {
            let app = load(file, cli)?;
            let reference = a1::parse(address)?;
            if !a1::is_single(&reference) {
                return Err(format!("{address}: get takes one cell, not a range"));
            }
            let (sheet, pos, _) = a1::resolve(&app, &reference)?;
            if *formula {
                let source = app
                    .formula(sheet, pos)
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                return Ok(Report::Text(TextReport {
                    lines: vec![source],
                }));
            }
            Ok(Report::Cells(cells(&app, file, sheet, pos, pos, u32::MAX)?))
        }

        Command::View {
            file,
            address,
            max_rows,
        } => {
            let app = load(file, cli)?;
            let (sheet, start, end) = match address {
                Some(address) => a1::resolve(&app, &a1::parse(address)?)?,
                // No range given: everything the sheet uses.
                None => {
                    let (rows, cols) = app.used_extent(0).map_err(|e| e.to_string())?;
                    let last = Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
                    (0, Pos::new(0, 0), last)
                }
            };
            Ok(Report::Cells(cells(
                &app, file, sheet, start, end, *max_rows,
            )?))
        }

        Command::Set {
            file,
            address,
            value,
            text,
        } => {
            let app = load(file, cli)?;
            let (sheet, pos, _) = single(&app, address)?;
            let value = read_stdin_if_dash(value)?;
            if !text && value.starts_with('=') {
                // Stored verbatim; `App::set_formula` computes and stores the cached value
                // alongside, because a formula without one renders blank in LibreOffice.
                app.set_formula(sheet, pos, &value)
                    .map_err(|e| e.to_string())?;
            } else {
                app.set_cell(sheet, pos, literal(&value, *text))
                    .map_err(|e| e.to_string())?;
            }
            finish(&app, cli, file, true)
        }

        Command::Clear {
            file,
            address,
            formula_only,
        } => {
            let app = load(file, cli)?;
            let (sheet, pos, _) = single(&app, address)?;
            if *formula_only {
                app.clear_formula(sheet, pos).map_err(|e| e.to_string())?;
            } else {
                app.clear_formula(sheet, pos).map_err(|e| e.to_string())?;
                app.set_cell(sheet, pos, CellValue::Empty)
                    .map_err(|e| e.to_string())?;
            }
            finish(&app, cli, file, true)
        }

        Command::Recalc { file } => {
            let app = load(file, cli)?;
            let recalc = app.recalc().map_err(|e| e.to_string())?;
            // Diagnostics to stderr, so the warning cannot corrupt parseable output. This
            // build implements a subset of OpenFormula; a document using anything else
            // loses a good cached value to #NAME? here, and saying so is the difference
            // between a recalculation and silent data loss. `sheet undo` is the way back.
            if recalc.spoiled > 0 {
                eprintln!(
                    "sheet: {} cell(s) became errors — a function this build does not \
                     implement; undo to restore, or see `sheet functions`",
                    recalc.spoiled
                );
            }
            finish(&app, cli, file, recalc.changed > 0)
        }

        Command::Convert { file, out } => {
            let app = load(file, cli)?;
            finish(&app, cli, out, true)
        }

        Command::Info { file } => {
            let app = load(file, cli)?;
            Ok(Report::Document(document(&app, file, false, false)))
        }

        Command::Undo { file } => {
            let app = load(file, cli)?;
            require_session(cli, "undo")?;
            let changed = app.undo();
            finish(&app, cli, file, changed)
        }

        Command::Redo { file } => {
            let app = load(file, cli)?;
            require_session(cli, "redo")?;
            let changed = app.redo();
            finish(&app, cli, file, changed)
        }

        Command::Fmt { formula } => {
            let expr = sheet_core::formula::parse::parse(formula).map_err(|e| e.to_string())?;
            Ok(Report::Text(TextReport {
                lines: vec![format!("={expr}")],
            }))
        }

        Command::Functions => {
            let names = sheet_core::formula::funcs::implemented();
            let mut lines: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
            lines.sort();
            lines.push(format!("{} of 110 in the Small Group", names.len()));
            Ok(Report::Text(TextReport { lines }))
        }
    }
}

// --- helpers ---

/// Open the document and, if a session file was named, resume its history.
fn load(file: &Path, cli: &Cli) -> Result<App, String> {
    let app = App::new();
    app.open_file(file)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    if let Some(path) = &cli.session
        && path.exists()
    {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read session {}: {e}", path.display()))?;
        let session: Session = serde_json::from_str(&raw)
            .map_err(|e| format!("cannot parse session {}: {e}", path.display()))?;
        app.restore_session(session);
    }
    Ok(app)
}

/// Persist the document when it changed, then the session, then report.
fn finish(app: &App, cli: &Cli, file: &Path, changed: bool) -> Result<Report, String> {
    let written = changed && !cli.dry_run;
    if written {
        app.save_file(file).map_err(|e| e.to_string())?;
    }
    save_session(app, cli)?;
    Ok(Report::Document(document(app, file, changed, written)))
}

fn save_session(app: &App, cli: &Cli) -> Result<(), String> {
    let (Some(path), false) = (&cli.session, cli.dry_run) else {
        return Ok(());
    };
    let session = serde_json::to_string(&app.session()).expect("session is serializable");
    std::fs::write(path, session)
        .map_err(|e| format!("cannot write session {}: {e}", path.display()))
}

/// Undo history lives in the session file, never in the document.
fn require_session(cli: &Cli, what: &str) -> Result<(), String> {
    if cli.session.is_none() {
        return Err(format!(
            "{what} needs --session: history is not stored in the document"
        ));
    }
    Ok(())
}

fn document(app: &App, file: &Path, changed: bool, written: bool) -> DocumentReport {
    let sheets = (0..app.sheet_count())
        .map(|i| {
            let (rows, cols) = app.used_extent(i).unwrap_or((0, 0));
            SheetInfo {
                name: app.sheet_name(i).unwrap_or_default(),
                rows,
                cols,
                formulas: app.formula_count(i).unwrap_or(0),
            }
        })
        .collect();
    DocumentReport {
        path: show_path(file),
        changed,
        written,
        sheets,
        names: app
            .names()
            .into_iter()
            .map(|(name, expression)| Name { name, expression })
            .collect(),
        can_undo: app.can_undo(),
        can_redo: app.can_redo(),
    }
}

/// A rectangle of cells, read the only way a shell may read them.
fn cells(
    app: &App,
    file: &Path,
    sheet: usize,
    start: Pos,
    end: Pos,
    max_rows: u32,
) -> Result<CellsReport, String> {
    let last_row = end
        .row
        .min(start.row.saturating_add(max_rows.saturating_sub(1)));
    let rows = start.row..last_row.saturating_add(1);
    let cols = start.col..end.col.saturating_add(1);
    let name = app.sheet_name(sheet).map_err(|e| e.to_string())?;
    let viewport = app
        .get_viewport(sheet, rows.clone(), cols.clone())
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows.clone() {
        for col in cols.clone() {
            let pos = Pos::new(row, col);
            let value = viewport.get(row, col).cloned().unwrap_or(CellValue::Empty);
            out.push(Cell {
                address: a1::format(None, pos),
                value: report::show(&value),
                kind: report::kind(&value),
                formula: app.formula(sheet, pos).map_err(|e| e.to_string())?,
            });
        }
    }
    Ok(CellsReport {
        path: show_path(file),
        sheet: name,
        cells: out,
        rows: rows.end - rows.start,
        cols: cols.end - cols.start,
    })
}

fn single(app: &App, address: &str) -> Result<(usize, Pos, Pos), String> {
    let reference = a1::parse(address)?;
    if !a1::is_single(&reference) {
        return Err(format!("{address}: expected one cell, not a range"));
    }
    a1::resolve(app, &reference)
}

/// What the user typed, as a value: a number, a logical, or text (§6.3 has nothing to say
/// here — this is a shell's convenience, not a conversion the engine performs).
fn literal(value: &str, force_text: bool) -> CellValue {
    if force_text {
        return CellValue::Text(value.to_owned());
    }
    if let Ok(n) = value.parse::<f64>() {
        return CellValue::Number(n);
    }
    match value {
        "TRUE" | "true" => CellValue::Bool(true),
        "FALSE" | "false" => CellValue::Bool(false),
        "" => CellValue::Empty,
        _ => CellValue::Text(value.to_owned()),
    }
}

fn read_stdin_if_dash(value: &str) -> Result<String, String> {
    if value != "-" {
        return Ok(value.to_owned());
    }
    let mut buffer = String::new();
    std::io::stdin()
        .read_to_string(&mut buffer)
        .map_err(|e| format!("cannot read stdin: {e}"))?;
    Ok(buffer.trim_end_matches('\n').to_owned())
}

fn show_path(path: &Path) -> String {
    path.display().to_string()
}
