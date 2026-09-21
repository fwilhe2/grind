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
use grind_core::lint::Diagnostic;
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
    /// What `grind lint` found (`doc/dsl.md` §4.3, D6).
    Lint(LintReport),
    /// What `grind sheet import` carried, and what it did not (`doc/xlsx-import.md`).
    /// Boxed for the reason `CellStyle` is: the fidelity report outgrew every other variant.
    #[cfg(feature = "xlsx")]
    Import(Box<ImportReport>),
}

/// The fidelity report, which is **part of the output rather than an afterthought**: what a
/// conversion dropped is as important as what it carried, and a converter that lies about it
/// is worse than one that refuses.
#[cfg(feature = "xlsx")]
#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub input: String,
    pub output: String,
    pub written: bool,
    /// Whether the conversion lost nothing at all — `grind_xlsx::Report::lossless`.
    pub lossless: bool,
    /// `--strict`: a conversion that is not lossless fails, and is not written.
    pub strict: bool,
    /// `transitional`, `strict` or `mixed` — a fact about the file the reader stated rather
    /// than branched on.
    pub flavour: &'static str,
    pub sheets: usize,
    pub cells: usize,
    /// Cells read and not carried, because the import's materialisation bound was reached
    /// (`grind_xlsx::sheet::MAX_CELLS`). Zero for every document that is not enormous; not a
    /// dropped *construct*, since the model could hold them, but a loss all the same.
    pub over_budget: usize,
    pub formulas: usize,
    /// Cells carrying a number format of their own.
    pub formatted: usize,
    /// Number formats that could not be spelled in full, by class and by the number of cells
    /// each class cost. A class that would have misstated the number took the cell's whole
    /// format with it and the cell shows its plain value; the rest took a piece.
    pub formats_lost: Vec<DroppedCount>,
    /// Cells carrying a cell style of their own — a font, fill, border or alignment that
    /// differs from the workbook's default.
    pub styled: usize,
    /// Pieces of the workbook's look the model has no slot for, by class: per cell for a
    /// piece of a cell's style, per track for a zero-size row or column, once per sheet for
    /// panes, an outline or a sheet-wide column width. The label says which.
    pub appearance_lost: Vec<DroppedCount>,
    /// Formulas read and not carried, by the class that stopped each one. The cell kept the
    /// value Excel cached for it, so the document is not wrong — it no longer recalculates
    /// there, which is what this says.
    pub untranslated: Vec<DroppedCount>,
    /// Functions a carried formula names that this build cannot evaluate. Not a loss: the
    /// formula came through intact, and this is a fact about the evaluator.
    pub unknown_functions: Vec<String>,
    /// Every construct the model has no home for, by name and count.
    pub dropped: Vec<DroppedCount>,
    /// Defined names carried into the document.
    pub names: usize,
    /// Defined names that were not, each with the class that stopped it.
    pub names_lost: Vec<NameLost>,
    /// Sheets whose name changed, and to what. Every reference followed.
    pub renamed: Vec<Renamed>,
    /// Namespaces the workbook said a consumer must understand and this one does not. Never
    /// a refusal — in a spreadsheet such a namespace guards a feature rather than the cell
    /// values, and cell values are what an import is for.
    pub must_understand: Vec<String>,
}

#[cfg(feature = "xlsx")]
#[derive(Debug, Serialize)]
pub struct NameLost {
    pub name: String,
    pub why: String,
}

#[cfg(feature = "xlsx")]
#[derive(Debug, Serialize)]
pub struct Renamed {
    pub from: String,
    pub to: String,
}

#[cfg(feature = "xlsx")]
#[derive(Debug, Serialize)]
pub struct DroppedCount {
    pub what: String,
    pub count: usize,
}

#[cfg(feature = "xlsx")]
impl ImportReport {
    pub fn new(
        input: &str,
        output: &str,
        document: &grind_sheet::model::Document,
        report: &grind_xlsx::Report,
        written: bool,
        strict: bool,
    ) -> Self {
        Self {
            input: input.to_owned(),
            output: output.to_owned(),
            written,
            lossless: report.lossless(),
            strict,
            flavour: match report.flavour {
                grind_xlsx::Flavour::Transitional => "transitional",
                grind_xlsx::Flavour::Strict => "strict",
                grind_xlsx::Flavour::Mixed => "mixed",
            },
            sheets: document.sheets.len(),
            cells: report.cells,
            over_budget: report.over_budget,
            formulas: report.formulas,
            formatted: report.formatted,
            formats_lost: report
                .formats_lost
                .iter()
                .map(|(class, count)| DroppedCount {
                    what: class.label().to_owned(),
                    count: *count,
                })
                .collect(),
            styled: report.styled,
            appearance_lost: report
                .appearance_lost
                .iter()
                .map(|(class, count)| DroppedCount {
                    what: class.label().to_owned(),
                    count: *count,
                })
                .collect(),
            untranslated: report
                .refused
                .iter()
                .map(|(class, count)| DroppedCount {
                    what: class.label().to_owned(),
                    count: *count,
                })
                .collect(),
            unknown_functions: report.unknown_functions.iter().cloned().collect(),
            dropped: report
                .dropped
                .iter()
                .map(|(what, count)| DroppedCount {
                    what: what.label().to_owned(),
                    count: *count,
                })
                .collect(),
            names: report.names,
            names_lost: report
                .names_lost
                .iter()
                .map(|(name, why)| NameLost {
                    name: name.clone(),
                    why: why.label().to_owned(),
                })
                .collect(),
            renamed: report
                .renamed
                .iter()
                .map(|(from, to)| Renamed {
                    from: from.clone(),
                    to: to.clone(),
                })
                .collect(),
            must_understand: report.must_understand.iter().cloned().collect(),
        }
    }
}

/// `get` and `view`.
#[derive(Debug, Serialize)]
pub struct CellsReport {
    pub path: String,
    pub sheet: String,
    /// Which of a cell's spellings the *text* output prints. Not serialised: JSON carries
    /// every one of them and lets the caller pick.
    #[serde(skip)]
    pub shown: Shown,
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
    /// The same formula read through the document's names — `=tax_rate*subtotal` where
    /// `formula` says `=[.B2]*[.B7]` (`doc/view-modes.md` §3.3). Present only when `--names`
    /// asked for it, because it costs the document-wide analysis that resolves them.
    ///
    /// A separate field rather than a different spelling of `formula`: one is what the file
    /// stores and the other is a reading of it, and a consumer that wants the source must
    /// not have to know which flags produced its input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub named_formula: Option<String>,
    /// What the cell *is* — `doc/view-modes.md`'s role, present only when it was asked for.
    /// Deriving one costs a document-wide analysis, so a plain `view` does not pay for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<&'static str>,
    /// The named expression bound to this cell, when one is (`--names`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Which column of a cell the text output prints — one per line of `view`, tab-separated.
///
/// JSON is unaffected: it carries every spelling the read produced, and this is only which
/// one a terminal gets. Being an enum rather than a pile of booleans is what makes the
/// flags mutually exclusive by construction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shown {
    /// What the cell displays, its number format applied.
    #[default]
    Text,
    /// The stored value, as the file spells it (`--raw`).
    Value,
    /// `doc/view-modes.md`'s role (`--roles`).
    Role,
    /// The name bound to the cell, or nothing (`--names`).
    Name,
    /// The formula source, or nothing (`--formulas`).
    Formula,
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

/// `grind lint`, for either application.
///
/// One shape whichever kind of document was linted: `main.rs` picks the rules by reading the
/// file's kind, and a diagnostic is already document-type-neutral (`grind_core::lint`). So a
/// consumer of `--format json` parses one thing and a shell renders one thing.
#[derive(Debug, Serialize)]
pub struct LintReport {
    pub path: String,
    pub diagnostics: Vec<Diagnostic>,
    /// Whether looking stopped at `grind_core::lint::MAX_DIAGNOSTICS`. Serialised always,
    /// because a consumer that reports "clean" has to know the list is complete.
    pub truncated: bool,
    pub errors: usize,
    pub warnings: usize,
    pub hints: usize,
}

impl LintReport {
    /// Whether this is a failure — what `grind lint`'s exit status is made of.
    ///
    /// Errors only. A warning is a document saying something it probably did not mean and a
    /// hint is house style; gating CI on either would mean nobody could turn the rule on for
    /// an existing document, which is how a linter ends up permanently disabled.
    pub fn failed(&self) -> bool {
        self.errors > 0
    }
}

impl Report {
    /// Whether the command found something that should fail a script. `lint` does on an
    /// error-severity finding, and `sheet import --strict` on any loss; every other report here
    /// is the result of an operation that either worked or returned an `Err`, and a no-op is a
    /// success.
    pub fn failed(&self) -> bool {
        matches!(self, Report::Lint(lint) if lint.failed()) || self.import_refused()
    }

    #[cfg(feature = "xlsx")]
    fn import_refused(&self) -> bool {
        matches!(self, Report::Import(import) if import.strict && !import.lossless)
    }

    #[cfg(not(feature = "xlsx"))]
    fn import_refused(&self) -> bool {
        false
    }

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
                        .map(|c| match cells.shown {
                            Shown::Text => c.text.as_str(),
                            Shown::Value => c.value.as_str(),
                            Shown::Role => c.role.unwrap_or_default(),
                            Shown::Name => c.name.as_deref().unwrap_or_default(),
                            // `--formulas --names` prints the reading; `--formulas` alone
                            // prints the source.
                            Shown::Formula => c
                                .named_formula
                                .as_deref()
                                .or(c.formula.as_deref())
                                .unwrap_or_default(),
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
            // The losses first and the counts last, because what a conversion dropped is the
            // half a person has to decide about.
            #[cfg(feature = "xlsx")]
            Report::Import(import) => {
                for dropped in &import.dropped {
                    println!("dropped\t{}\t{}", dropped.count, dropped.what);
                }
                for class in &import.formats_lost {
                    println!("format\t{}\t{}", class.count, class.what);
                }
                for class in &import.appearance_lost {
                    println!("look\t{}\t{}", class.count, class.what);
                }
                for class in &import.untranslated {
                    println!("untranslated\t{}\t{}", class.count, class.what);
                }
                for lost in &import.names_lost {
                    println!("name lost\t{}\t{}", lost.name, lost.why);
                }
                for renamed in &import.renamed {
                    println!("renamed\t{}\t{}", renamed.from, renamed.to);
                }
                if import.over_budget > 0 {
                    println!("over budget\t{}\tcells", import.over_budget);
                }
                for name in &import.unknown_functions {
                    println!("unknown function\t{name}");
                }
                for namespace in &import.must_understand {
                    println!("not understood\t{namespace}");
                }
                println!(
                    "{} -> {}{}",
                    import.input,
                    import.output,
                    match (import.written, import.strict && !import.lossless) {
                        (true, _) => "",
                        (false, true) => " (--strict: something was lost, nothing written)",
                        (false, false) => " (dry run, nothing written)",
                    }
                );
                println!(
                    "{} sheets\t{} cells\t{} formulas\t{} formatted\t{} styled\t{}",
                    import.sheets,
                    import.cells,
                    import.formulas,
                    import.formatted,
                    import.styled,
                    import.flavour
                );
            }
            // One diagnostic per line, in the shape every compiler prints — so an editor's
            // error parser and a person reading a terminal both already understand it.
            Report::Lint(lint) => {
                for diagnostic in &lint.diagnostics {
                    println!("{diagnostic}");
                }
                if lint.truncated {
                    println!("(stopped after {} findings)", lint.diagnostics.len());
                }
                println!(
                    "{}: {}",
                    lint.path,
                    match lint.diagnostics.is_empty() {
                        true => "no problems found".to_owned(),
                        false => format!(
                            "{} error(s), {} warning(s), {} hint(s)",
                            lint.errors, lint.warnings, lint.hints
                        ),
                    }
                );
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
