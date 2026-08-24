// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind` — the suite CLI, and the parity ratchet.
//!
//! Every capability the core has is reachable from here, and `doc/cli-parity-sheet.md` plus
//! `tests/parity.rs` make that a build failure rather than a promise. A subcommand is one
//! `match` arm that drives [`App`]; anything longer than that belongs in the core.
//!
//! Diagnostics go to stderr, results to stdout, and an error never appears on stdout in
//! either format.

mod report;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use grind_sheet::a1;
use grind_sheet::formula::lex::column_name;
use grind_sheet::numfmt;
use grind_sheet::style::{self, CellStyle};
use grind_sheet::{App, CellValue, DocumentKind, Pos, RecalcMode, Session};
use grind_text::App as TextApp;

use report::{
    Cell, CellsReport, DocumentReport, Format, Name, Report, SheetInfo, TextDocumentReport,
    TextReport,
};

// ---------------------------------------------------------------------------
// grind text
// ---------------------------------------------------------------------------

/// Open a text document, refusing one that is not.
fn open_text(file: &Path) -> Result<TextApp, String> {
    let app = TextApp::new();
    app.open_file(file)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(app)
}

/// Resolve one address against the document.
fn at(app: &TextApp, address: &str) -> Result<usize, String> {
    let loc = grind_text::loc::parse(address).map_err(|e| e.to_string())?;
    app.resolve(&loc).map_err(|e| e.to_string())
}

/// Resolve a range of blocks. A **heading** address alone means its whole section, which is
/// what makes `grind text move report.fodt §3.2 §1` mean what a person expects — the extent is
/// computed from outline levels, because the document stores no such container.
fn span(app: &TextApp, address: &str) -> Result<std::ops::Range<usize>, String> {
    if !address.contains(':')
        && let Ok(index) = at(app, address)
        && let Some(section) = app.section(index)
    {
        return Ok(section);
    }
    let range = grind_text::loc::parse_range(address).map_err(|e| e.to_string())?;
    app.resolve_range(&range).map_err(|e| e.to_string())
}

/// Resolve one address to a caret — a block *and* an offset within it, which is what `p3+12`
/// names and what the caret-level verbs take.
fn caret_at(app: &TextApp, address: &str) -> Result<grind_text::Caret, String> {
    let loc = grind_text::loc::parse(address).map_err(|e| e.to_string())?;
    app.resolve_caret(&loc).map_err(|e| e.to_string())
}

/// Resolve a range of **characters**, as `erase` takes it.
///
/// Deliberately not [`span`]: there, a bare heading address means the heading's whole *section*,
/// because the verbs that take one relocate and delete structure. `erase` removes characters,
/// so a bare address there means that one block's text and a heading is not special. Two verbs
/// spelled the same and meaning different things would be worse than two functions.
fn caret_span(
    app: &TextApp,
    address: &str,
) -> Result<(grind_text::Caret, grind_text::Caret), String> {
    let range = grind_text::loc::parse_range(address).map_err(|e| e.to_string())?;
    app.resolve_caret_range(&range).map_err(|e| e.to_string())
}

/// One block's text, broken into lines at `width` — what `view --width` prints.
///
/// The CLI measures one unit per character (`grind_text::Fixed`), so a width of 72 is 72
/// characters. Good enough to be useful and honest about what it is: a terminal shell wants
/// `unicode-width` for CJK and combining marks, and implements `Metrics` itself.
fn wrapped(app: &TextApp, index: usize, width: u32) -> Result<Vec<String>, String> {
    let text: Vec<char> = app
        .input_text(index)
        .map_err(|e| e.to_string())?
        .chars()
        .collect();
    let layout = app
        .layout_block(index, width as f32, &grind_text::Fixed)
        .map_err(|e| e.to_string())?;
    Ok(layout
        .lines()
        .iter()
        // A trailing space or newline is *on* the line it ended, which is right for a caret and
        // wrong for printing, so the printer trims what the model deliberately keeps.
        .map(|line| text[line.start..line.end].iter().collect::<String>())
        .map(|line| line.trim_end().to_owned())
        .collect())
}

/// The block kind two flags describe. Neither means a paragraph.
fn kind_of(heading: Option<u32>, list: Option<u32>) -> Result<grind_text::BlockKind, String> {
    match (heading, list) {
        (Some(0), _) => Err("headings start at level 1".to_owned()),
        (Some(level), _) => Ok(grind_text::BlockKind::Heading { level }),
        (_, Some(0)) => Err("list items start at depth 1".to_owned()),
        (_, Some(depth)) => Ok(grind_text::BlockKind::ListItem { depth }),
        (None, None) => Ok(grind_text::BlockKind::Paragraph),
    }
}

/// A plain list of lines, which is most of what `grind text` prints.
fn text_lines(lines: Vec<String>) -> Result<Report, String> {
    Ok(Report::Text(TextReport { lines }))
}

fn run_text(command: &TextCommand, cli: &Cli) -> Result<Report, String> {
    match command {
        TextCommand::New { file, force } => {
            if file.exists() && !force {
                return Err(format!("{} exists; pass --force", file.display()));
            }
            finish_text(&TextApp::new(), cli, file, true)
        }

        TextCommand::View {
            file,
            range,
            marks,
            width,
        } => {
            let app = open_text(file)?;
            let blocks = match range {
                Some(range) => span(&app, range)?,
                None => 0..app.block_count(),
            };
            let view = app.get_viewport(blocks);
            let mut lines = Vec::new();
            for block in view.iter() {
                // Without --width a block is one line, which is what `view` has always printed.
                // With it, the core breaks the block and the CLI prints what any shell would
                // draw at that width — the same engine, measured one unit per character.
                let pieces = match width {
                    None => vec![block.text.clone()],
                    Some(width) => wrapped(&app, block.index, *width)?,
                };
                for (n, piece) in pieces.into_iter().enumerate() {
                    lines.push(match marks {
                        false => piece,
                        // Only the first line of a block carries its address, so the marks stay
                        // one-per-block and a wrapped block reads as a block.
                        true if n == 0 => format!(
                            "{}\t{}\t{}",
                            grind_text::loc::format(block.index),
                            describe_kind(&block.kind),
                            piece
                        ),
                        true => format!("\t\t{piece}"),
                    });
                }
            }
            text_lines(lines)
        }

        TextCommand::Caret {
            file,
            at: address,
            down,
            home,
            end,
            width,
        } => {
            let app = open_text(file)?;
            let caret = caret_at(&app, address)?;
            let width = *width as f32;
            let moved = if *home || *end {
                let (start, finish) = app
                    .caret_line_bounds(caret, width, &grind_text::Fixed)
                    .map_err(|e| e.to_string())?;
                match home {
                    true => start,
                    false => finish,
                }
            } else {
                // The goal column is the caret's own x, because a single move from a script has
                // no run of keystrokes to remember one from.
                let goal = app
                    .caret_x(caret, width, &grind_text::Fixed)
                    .map_err(|e| e.to_string())?;
                app.caret_line(
                    caret,
                    down.unwrap_or(0) as isize,
                    goal,
                    width,
                    &grind_text::Fixed,
                )
                .map_err(|e| e.to_string())?
            };
            text_lines(vec![grind_text::loc::format_offset(
                moved.block,
                moved.offset,
            )])
        }

        TextCommand::Get { file, at: address } => {
            let app = open_text(file)?;
            let index = at(&app, address)?;
            text_lines(vec![app.input_text(index).map_err(|e| e.to_string())?])
        }

        TextCommand::Set {
            file,
            at: address,
            text,
        } => {
            let app = open_text(file)?;
            let index = at(&app, address)?;
            app.set_text(index, &read_stdin_if_dash(text)?)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Type {
            file,
            at: address,
            text,
        } => {
            let app = open_text(file)?;
            let caret = caret_at(&app, address)?;
            let text = read_stdin_if_dash(text)?;
            app.insert_text(caret, &text).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, !text.is_empty())
        }

        TextCommand::Erase { file, range } => {
            let app = open_text(file)?;
            let (from, to) = caret_span(&app, range)?;
            let removed = app.erase(from, to).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, removed > 0)
        }

        TextCommand::Split { file, at: address } => {
            let app = open_text(file)?;
            let caret = caret_at(&app, address)?;
            app.split_block(caret).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Join { file, at: address } => {
            let app = open_text(file)?;
            let index = at(&app, address)?;
            app.join_block(index).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Insert {
            file,
            at: address,
            text,
            after,
            heading,
            list,
        } => {
            let app = open_text(file)?;
            let index = match address {
                Some(address) => at(&app, address)? + usize::from(*after),
                // No address appends, which is what building a document from a script does
                // most of the time.
                None => app.block_count(),
            };
            app.insert(index, kind_of(*heading, *list)?, &read_stdin_if_dash(text)?)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Delete { file, range } => {
            let app = open_text(file)?;
            let blocks = span(&app, range)?;
            let removed = app.delete(blocks).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, removed > 0)
        }

        TextCommand::Move { file, range, to } => {
            let app = open_text(file)?;
            let blocks = span(&app, range)?;
            let landing = at(&app, to)?;
            let moved = app
                .move_blocks(blocks, landing)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, moved > 0)
        }

        TextCommand::Style { file, range, style } => {
            let app = open_text(file)?;
            let blocks = span(&app, range)?;
            let changed = app
                .set_style(blocks, style.clone())
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, changed > 0)
        }

        TextCommand::Kind {
            file,
            at: address,
            heading,
            list,
        } => {
            let app = open_text(file)?;
            let index = at(&app, address)?;
            app.set_kind(index, kind_of(*heading, *list)?)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Outline { file, filter } => {
            let app = open_text(file)?;
            let needle = filter.as_deref().unwrap_or("").to_lowercase();
            text_lines(
                app.outline()
                    .into_iter()
                    .filter(|h| {
                        needle.is_empty()
                            || h.text.to_lowercase().contains(&needle)
                            || h.address().contains(&needle)
                    })
                    .map(|h| format!("{}\t{}\t{}", h.address(), h.level, h.text))
                    .collect(),
            )
        }

        TextCommand::Format {
            file,
            range,
            show,
            bold,
            italic,
            underline,
            strike,
            font,
            size,
            color,
            background,
        } => {
            let app = open_text(file)?;
            let (from, to) = caret_span(&app, range)?;
            if *show {
                let style = app.char_style(from, to).map_err(|e| e.to_string())?;
                return text_lines(describe_char_style(&style));
            }
            let mut style = grind_text::CharStyle {
                font_family: font.clone(),
                font_size: size.clone(),
                color: color.clone(),
                background: background.clone(),
                ..grind_text::CharStyle::default()
            };
            style.set_bold(*bold);
            style.set_italic(*italic);
            style.set_underlined(*underline);
            style.set_struck(*strike);
            let changed = app
                .set_char_style(from, to, &style)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, changed > 0)
        }

        TextCommand::Formatting { file } => {
            let app = open_text(file)?;
            text_lines(
                app.formatting()
                    .into_iter()
                    .map(|b| {
                        format!(
                            "{}\t{}\t{}",
                            grind_text::loc::format(b.index),
                            b.style.as_deref().unwrap_or("(direct)"),
                            b.text
                        )
                    })
                    .collect(),
            )
        }

        TextCommand::Find { file, needle } => {
            let app = open_text(file)?;
            text_lines(
                app.find(needle)
                    .into_iter()
                    .map(|m| format!("{}\t{}", m.address(), m.text))
                    .collect(),
            )
        }

        TextCommand::Replace {
            file,
            needle,
            replacement,
        } => {
            let app = open_text(file)?;
            let changed = app
                .replace(needle, replacement)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, changed > 0)
        }

        TextCommand::Name {
            file,
            name,
            at: address,
            delete,
        } => {
            let app = open_text(file)?;
            let Some(name) = name else {
                return text_lines(
                    app.bookmarks()
                        .into_iter()
                        .map(|(name, index)| format!("#{name}\t{}", grind_text::loc::format(index)))
                        .collect(),
                );
            };
            let target = match (delete, address) {
                (true, _) => None,
                (false, Some(address)) => Some(at(&app, address)?),
                (false, None) => {
                    // Reading one back, rather than setting it — the same shape `grind sheet
                    // name <name>` has.
                    let index = app
                        .bookmarks()
                        .into_iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, index)| index)
                        .ok_or_else(|| format!("no bookmark named {name}"))?;
                    return text_lines(vec![format!(
                        "#{name}\t{}",
                        grind_text::loc::format(index)
                    )]);
                }
            };
            let changed = app.set_bookmark(name, target).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, changed)
        }

        TextCommand::Words { file } => {
            let app = open_text(file)?;
            Ok(Report::TextDocument(text_document(
                &app, file, false, false,
            )))
        }
    }
}

fn describe_kind(kind: &grind_text::BlockKind) -> String {
    match kind {
        grind_text::BlockKind::Paragraph => "p".to_owned(),
        grind_text::BlockKind::Heading { level } => format!("h{level}"),
        grind_text::BlockKind::ListItem { depth } => format!("li{depth}"),
    }
}

/// Save if anything changed, then report — `finish`'s twin for text documents.
fn finish_text(app: &TextApp, cli: &Cli, file: &Path, changed: bool) -> Result<Report, String> {
    let written = changed && !cli.dry_run;
    if written {
        app.save_file(file).map_err(|e| e.to_string())?;
    }
    Ok(Report::TextDocument(text_document(
        app, file, changed, written,
    )))
}

fn text_document(app: &TextApp, file: &Path, changed: bool, written: bool) -> TextDocumentReport {
    let counts = app.counts();
    TextDocumentReport {
        path: show_path(file),
        kind: None,
        changed,
        written,
        blocks: counts.blocks,
        words: counts.words,
        characters: counts.characters,
        headings: counts.headings,
        bookmarks: app.bookmarks().into_iter().map(|(name, _)| name).collect(),
        can_undo: app.can_undo(),
        can_redo: app.can_redo(),
    }
}

fn long_version() -> &'static str {
    use std::sync::OnceLock;
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| grind_sheet::build_info::describe_version(env!("CARGO_PKG_VERSION")))
}

#[derive(Parser)]
#[command(
    name = "grind",
    version,
    long_version = long_version(),
    about = "Drive the ODF suite from the shell",
    long_about = "Drive the ODF suite from the shell.\n\n\
        Commands are grouped by the kind of document they act on — `grind sheet …` for \
        spreadsheets — with a few verbs at the top level that work on any ODF document and \
        decide what it is by reading it.\n\n\
        Each invocation loads the file, applies one command and writes it back. Pass \
        --session to carry undo history across invocations; without it every command starts \
        with empty history."
)]
struct Cli {
    #[command(subcommand)]
    command: Top,

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

/// The first level: which application, or a verb that needs no application.
///
/// A suite-level verb is one whose answer does not depend on knowing what is *in* the
/// document — what kind it is, and moving it between the two physical forms. Everything else
/// belongs to an app, because "set a cell" and "insert a paragraph" are not the same verb with
/// a different noun (doc/suite.md, "The CLI").
#[derive(Subcommand)]
enum Top {
    /// Spreadsheets — cells, formulas, sheets, number formats
    Sheet {
        #[command(subcommand)]
        command: Command,
    },

    /// Text documents — paragraphs, headings, lists, outlines
    ///
    /// Blocks are addressed by position (p12), by an offset within one (p12+40), by a range
    /// (p12:p20), by a bookmark (#intro) or by outline path (§2.1.3, or s2.1.3). The last two
    /// survive edits elsewhere in the document, which p12 does not.
    Text {
        #[command(subcommand)]
        command: TextCommand,
    },

    /// What a document is, and what is in it
    ///
    /// Works on any ODF document: the kind is read out of the file (the package `mimetype`
    /// entry, or the flat root's `office:mimetype`) rather than guessed from its name.
    Info { file: PathBuf },

    /// Convert between the package and flat forms — `.ods` to `.fods` and back
    ///
    /// The form comes from the output extension. Never between document *kinds*: a
    /// spreadsheet does not become a text document by being written differently.
    Convert { file: PathBuf, out: PathBuf },
}

/// The formats `sheet format` can ask the core for — [`numfmt::preset`]'s vocabulary, with
/// `general` for "no format at all" and `datetime` for §4.3.4's date-carrying-a-time.
///
/// Dates and times are the ISO spellings. A locale-specific one is a `Format` the core can
/// hold and nothing can yet ask for; see `numfmt`.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Style {
    General,
    Number,
    Percent,
    Currency,
    Date,
    Datetime,
    Time,
    Boolean,
    Text,
}

/// `fo:text-align`, in the spelling a person uses. ODF's own values are writing-direction
/// relative — `start` and `end`, not left and right — and this is the translation.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Align {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum VAlign {
    Top,
    Middle,
    Bottom,
}

/// `grind text` — the word processor.
///
/// Deliberately narrower than `grind sheet`, and `doc/text-core.md` is why: the scope line for
/// a text document was invented rather than extracted from a normative tier, so every verb
/// here has to earn its place rather than mirror one that exists for cells.
#[derive(Subcommand)]
enum TextCommand {
    /// Create an empty document
    ///
    /// The physical form comes from the extension, and flat XML is the default: .ods and .odt
    /// write a zip, anything else — including a name with no extension at all — writes one XML
    /// file that git diff can read (doc/flat-first.md).
    New {
        file: PathBuf,
        /// Overwrite the file if it already exists
        #[arg(long)]
        force: bool,
    },

    /// Print the document as plain text, one block per line
    View {
        file: PathBuf,
        /// Blocks to print, e.g. p2 or p2:p9; defaults to the whole document
        range: Option<String>,
        /// Prefix each line with its address and kind
        #[arg(long)]
        marks: bool,
        /// Wrap at this many characters, the way a shell would at its own width
        #[arg(long, value_name = "COLUMNS")]
        width: Option<u32>,
    },

    /// Move a caret by lines, or to the ends of its line, and print where it lands
    ///
    /// Down-arrow, Up-arrow, Home and End — from a script. Every one of them is defined in
    /// terms of a *line*, so every one needs a width; the CLI measures one unit per character,
    /// which is why `--width 40` means forty characters. That layout lives in the core is what
    /// makes this answerable here at all rather than only inside a GUI
    /// (`doc/text-layout.md`).
    Caret {
        file: PathBuf,
        /// Where the caret is now, e.g. p3+12, #intro+5 or §2.1
        at: String,
        /// Move down this many lines (negative moves up)
        #[arg(long, allow_negative_numbers = true, value_name = "LINES")]
        down: Option<i32>,
        /// Move to the start of the caret's own line
        #[arg(long, conflicts_with_all = ["down", "end"])]
        home: bool,
        /// Move to the end of the caret's own line
        #[arg(long, conflicts_with_all = ["down", "home"])]
        end: bool,
        /// Wrap at this many characters; without it the block is one line
        #[arg(long, value_name = "COLUMNS", default_value = "0")]
        width: u32,
    },

    /// Print one block
    Get {
        file: PathBuf,
        /// Block address, e.g. p3, #intro or §2.1
        at: String,
    },

    /// Replace a block's text, keeping its kind and its style
    ///
    /// Bookmarks on the block are kept: an anchor is a position, not content.
    Set {
        file: PathBuf,
        /// Block address, e.g. p3, #intro or §2.1
        at: String,
        /// The new text; "-" reads it from stdin
        #[arg(allow_hyphen_values = true)]
        text: String,
    },

    /// Insert text at a caret, without disturbing the rest of the block
    ///
    /// What typing does. `set` replaces a whole block, which is what a script wants; this is
    /// what a cursor wants, and the CLI has it because rule 4 says a shell may not have a verb
    /// the CLI does not. The text takes the formatting of the run at the caret.
    Type {
        file: PathBuf,
        /// Where it goes, e.g. p3+12 — an address with no offset means the front of its block
        at: String,
        /// The text; "-" reads it from stdin
        #[arg(allow_hyphen_values = true)]
        text: String,
    },

    /// Erase a range of characters, leaving the blocks that hold them
    ///
    /// `delete` removes blocks; this removes text. A range that crosses a block boundary
    /// closes the boundary up, leaving one block that keeps the first one's kind and style —
    /// one undo step, however many blocks it spanned.
    Erase {
        file: PathBuf,
        /// Characters, e.g. p3+12:p3+20 — a bare address is that block's whole text
        range: String,
    },

    /// Split a block in two at a caret
    ///
    /// The Return key. The second half keeps the first's kind and style, except that a heading
    /// split at its very end leaves a body paragraph behind it.
    Split {
        file: PathBuf,
        /// Where to cut, e.g. p3+12
        at: String,
    },

    /// Join a block with the one after it
    ///
    /// The Backspace key, seen from the block above it. The first block's kind and style are
    /// the ones that survive.
    Join {
        file: PathBuf,
        /// The first of the two, e.g. p3
        at: String,
    },

    /// Insert a block before an address, or at the end
    Insert {
        file: PathBuf,
        /// Where it goes; omit to append to the end of the document
        at: Option<String>,
        /// The text; "-" reads it from stdin
        #[arg(long, allow_hyphen_values = true, default_value = "")]
        text: String,
        /// Insert after the address rather than before it
        #[arg(long)]
        after: bool,
        /// Make it a heading at this outline level
        #[arg(long, value_name = "LEVEL", conflicts_with = "list")]
        heading: Option<u32>,
        /// Make it a list item at this nesting depth
        #[arg(long, value_name = "DEPTH", num_args = 0..=1, default_missing_value = "1")]
        list: Option<u32>,
    },

    /// Delete a block or a range of them
    Delete {
        file: PathBuf,
        /// Blocks to delete, e.g. p3 or p3:p7
        range: String,
    },

    /// Move a range of blocks so that it starts at an address
    ///
    /// The whole point of §2.1.3 addressing: `grind text move report.fodt §3.2 §1` relocates a
    /// section, and the section's extent is computed from outline levels.
    Move {
        file: PathBuf,
        /// Blocks to move, e.g. p3:p7 — or a heading address, which moves its whole section
        range: String,
        /// Where they land, e.g. p1
        to: String,
    },

    /// Set or clear the named paragraph style of a range of blocks
    Style {
        file: PathBuf,
        /// Blocks, e.g. p3 or p3:p7
        range: String,
        /// The style name, e.g. Heading_20_1; omit to clear
        #[arg(long)]
        style: Option<String>,
    },

    /// Change what kind of block this is — paragraph, heading or list item
    Kind {
        file: PathBuf,
        /// Block address
        at: String,
        /// Outline level for a heading
        #[arg(long, value_name = "LEVEL", conflicts_with = "list")]
        heading: Option<u32>,
        /// Nesting depth for a list item
        #[arg(long, value_name = "DEPTH", num_args = 0..=1, default_missing_value = "1")]
        list: Option<u32>,
    },

    /// Print every heading, its level and the address that finds it again
    ///
    /// The spreadsheet's `calculations` for prose: a long document hides its shape behind its
    /// text, and the only way to see it is a list.
    Outline {
        file: PathBuf,
        /// Only headings whose text or address contains this
        #[arg(long)]
        filter: Option<String>,
    },

    /// Set how a span of characters looks — bold, italic, a font, a size
    ///
    /// The *direct* formatting, which is what a toolbar's B and I buttons write. Replaces
    /// rather than adds, so `grind text format p3` with no options makes that paragraph plain
    /// again — and `--show` is how a shell reads the current formatting first, which is what
    /// makes "bold as well" one command after another. Named character styles are untouched:
    /// they are the document's own vocabulary and `grind text style` is where they live.
    Format {
        file: PathBuf,
        /// Characters, e.g. p3+12:p3+20 — or a bare address for a whole block
        range: String,
        /// Print the formatting of the range instead of setting any
        #[arg(long, conflicts_with_all = [
            "bold", "italic", "underline", "strike", "font", "size", "color", "background",
        ])]
        show: bool,
        #[arg(long)]
        bold: bool,
        #[arg(long)]
        italic: bool,
        #[arg(long)]
        underline: bool,
        /// Strike the text through
        #[arg(long)]
        strike: bool,
        /// Font family, e.g. Georgia
        #[arg(long, value_name = "FAMILY")]
        font: Option<String>,
        /// Font size, e.g. 14pt
        #[arg(long)]
        size: Option<String>,
        /// Text colour: a palette name (navy, red, silver, …) or #rrggbb
        #[arg(long, value_parser = color)]
        color: Option<String>,
        /// Highlight behind the text: a palette name, #rrggbb, or "transparent"
        #[arg(long, value_parser = color)]
        background: Option<String>,
    },

    /// Print every block carrying a style of its own
    ///
    /// "Why is this paragraph different?" — answered in one place, which is the thing no
    /// mainstream word processor does.
    Formatting { file: PathBuf },

    /// Print every occurrence of some text, with the address of each
    Find {
        file: PathBuf,
        #[arg(allow_hyphen_values = true)]
        needle: String,
    },

    /// Replace every occurrence of some text
    Replace {
        file: PathBuf,
        #[arg(allow_hyphen_values = true)]
        needle: String,
        #[arg(allow_hyphen_values = true)]
        replacement: String,
    },

    /// Put a bookmark on a block, list them, or delete one
    ///
    /// A bookmark is the named-range analogue: an anchor that moves with the text, so #intro
    /// keeps meaning the same place after an edit above it.
    Name {
        file: PathBuf,
        /// The bookmark name; omit to list them all
        name: Option<String>,
        /// Where it goes
        at: Option<String>,
        /// Remove it instead
        #[arg(long)]
        delete: bool,
    },

    /// Count blocks, headings, words and characters
    Words { file: PathBuf },
}

#[derive(Subcommand)]
enum Command {
    /// Create an empty document
    ///
    /// The physical form comes from the extension, and flat XML is the default: .ods and .odt
    /// write a zip, anything else — including a name with no extension at all — writes one XML
    /// file that git diff can read (doc/flat-first.md).
    New {
        file: PathBuf,
        /// Overwrite the file if it already exists
        #[arg(long)]
        force: bool,
    },

    /// Print a cell or a range
    ///
    /// Prints what the cell displays — its number format applied, so a date prints as a
    /// date. `--raw` prints the stored value instead.
    Get {
        file: PathBuf,
        /// Cell address, e.g. A1 or Data.B2
        address: String,
        /// Print the formula source instead of the value
        #[arg(long)]
        formula: bool,
        /// Print what an editor would show: the text that, set again, changes nothing
        #[arg(long, conflicts_with = "formula")]
        input: bool,
        /// Print a formula's calculated, formatted result rather than its source
        #[arg(long, conflicts_with_all = ["formula", "input"])]
        value: bool,
        /// Print the stored value rather than the formatted display text
        #[arg(long)]
        raw: bool,
    },

    /// Print a rectangle of values, tab-separated
    View {
        file: PathBuf,
        /// Range to print; defaults to everything the sheet uses
        address: Option<String>,
        /// Stop after this many rows
        #[arg(long, default_value_t = 40)]
        max_rows: u32,
        /// Print stored values rather than formatted display text
        #[arg(long)]
        raw: bool,
    },

    /// Set a cell's value or formula
    ///
    /// A leading '=' makes it a formula and a leading apostrophe forces text; otherwise the
    /// value is taken as a number, TRUE or FALSE, or text. "-" reads the value from stdin.
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
        /// Recalculate the document in the same undo step, unless that would spoil a cell
        #[arg(long)]
        recalc: bool,
    },

    /// Fill a rectangle from tab-separated rows
    ///
    /// Each cell is read the way `set` reads its value. "-" reads the rows from stdin.
    Paste {
        file: PathBuf,
        /// Where the first cell goes, e.g. B2
        anchor: String,
        /// Tab-separated rows, one per line; "-" reads them from stdin
        #[arg(allow_hyphen_values = true)]
        rows: String,
        /// Recalculate the document in the same undo step, unless that would spoil a cell
        #[arg(long)]
        recalc: bool,
    },

    /// Replicate one cell across a rectangle — extend a calculation the way a drag handle
    /// or Ctrl+D/Ctrl+R does
    ///
    /// A formula's relative references shift by each target's offset from `source`; its
    /// absolute ones (`$`) do not move. A plain value is copied as is.
    Fill {
        file: PathBuf,
        /// The cell whose content is replicated, e.g. A1
        source: String,
        /// Where it lands, e.g. A2:A10 or B1:D1
        address: String,
        /// Recalculate the document in the same undo step, unless that would spoil a cell
        #[arg(long)]
        recalc: bool,
    },

    /// Evaluate a formula against the document without storing anything
    ///
    /// The address is where the formula would sit, which is what its relative references
    /// are relative to.
    Eval {
        file: PathBuf,
        /// Cell address the formula would live at, e.g. B5
        address: String,
        /// The formula, e.g. '=SUM([.B2:.B4])'
        #[arg(allow_hyphen_values = true)]
        formula: String,
    },

    /// Clear a cell or a range, leaving it empty
    Clear {
        file: PathBuf,
        /// Cell address or range, e.g. A1 or B2:D40
        address: String,
        /// Remove only the formula, keeping the value it last computed (one cell)
        #[arg(long)]
        formula_only: bool,
    },

    /// Set how a cell or range is displayed
    ///
    /// The value never changes — a number format is display only. `general` removes the
    /// format, leaving the plain spelling of the value.
    Format {
        file: PathBuf,
        /// Cell address or range, e.g. B2:B40 or Data.C:C
        address: String,
        /// How to display it
        #[arg(value_enum, required_unless_present = "show", conflicts_with = "show")]
        style: Option<Style>,
        /// Digits after the decimal point, for number, percent and currency
        #[arg(long, default_value_t = 2)]
        decimals: u8,
        /// Group thousands, e.g. 1,234,567
        #[arg(long)]
        grouping: bool,
        /// Currency symbol
        #[arg(long, default_value = "$")]
        symbol: String,
        /// Locale for the decimal and grouping characters, e.g. de-DE. Defaults to
        /// $GRIND_LOCALE, then $XDG_CONFIG_HOME/sheet/locale, then none.
        #[arg(long, value_parser = locale)]
        locale: Option<grind_sheet::locale::Locale>,
        /// Print the format of one cell instead of setting one
        #[arg(long, conflicts_with_all = ["decimals", "grouping", "symbol", "locale"])]
        show: bool,
    },

    /// Set how a cell or range looks
    ///
    /// Replaces the cell's styling rather than adding to it, so `sheet style A1` with no
    /// options makes A1 plain again — and `--show` is how a shell reads the current styling
    /// first, which is what makes "bold as well" one command after another. Fonts are
    /// deliberately absent: LibreOffice rewrites a font family into a reference nothing here
    /// can follow yet.
    Style {
        file: PathBuf,
        /// Cell address or range, e.g. A1:D1 or Data.B:B
        address: String,
        /// Print the styling of one cell instead of setting any
        #[arg(long, conflicts_with_all = [
            "bold", "italic", "color", "background", "align", "valign", "wrap", "size", "border",
        ])]
        show: bool,
        #[arg(long)]
        bold: bool,
        #[arg(long)]
        italic: bool,
        /// Text colour: a palette name (navy, red, silver, …) or #rrggbb
        #[arg(long, value_parser = color)]
        color: Option<String>,
        /// Cell background: a palette name, #rrggbb, or "transparent"
        #[arg(long, value_parser = color)]
        background: Option<String>,
        /// Horizontal alignment
        #[arg(long, value_enum)]
        align: Option<Align>,
        /// Vertical alignment
        #[arg(long, value_enum)]
        valign: Option<VAlign>,
        /// Wrap text at the cell edge
        #[arg(long)]
        wrap: bool,
        /// Font size, e.g. 14pt
        #[arg(long)]
        size: Option<String>,
        /// Border on every edge, as width, line and colour: "0.5pt solid navy"
        #[arg(long, value_parser = border)]
        border: Option<String>,
    },

    /// Set the width of a column or a run of columns (§5.4)
    ///
    /// With no length, prints the width of every sized column in the range — nothing at all
    /// when they are all at the shell's default, which is what a script tests for.
    Width {
        file: PathBuf,
        /// A column or a run of them: B, B:D, or Data.B:D
        columns: String,
        /// An ODF length: 2.5cm, 64pt, 0.9in. Omit to print.
        length: Option<String>,
        /// Back to the default width
        #[arg(long, conflicts_with = "length")]
        clear: bool,
    },

    /// Set the height of a row or a run of rows (§5.4)
    ///
    /// The twin of `width`, addressed by row number: 3, 3:7, or Data.3:7.
    Height {
        file: PathBuf,
        /// A row or a run of them: 3, 3:7, or Data.3:7
        rows: String,
        /// An ODF length: 0.5cm, 14pt. Omit to print.
        length: Option<String>,
        /// Back to the default height
        #[arg(long, conflicts_with = "length")]
        clear: bool,
    },

    /// Hide — or with `--unhide`, show — a column or a row, or a run of them (§5.4)
    ///
    /// `sheet hide book.ods B:D` hides columns B through D; `sheet hide book.ods 3:7` hides
    /// rows 3 through 7 — the same spec `width`/`height` take, disambiguated the same way.
    /// With no track, prints every column and row hidden by hand, across every sheet.
    Hide {
        file: PathBuf,
        /// A column or a run of them (B, B:D, Data.B:D), or a row or a run (3, 3:7, Data.3:7)
        tracks: Option<String>,
        /// Show it again instead of hiding it
        #[arg(long)]
        unhide: bool,
    },

    /// Define, redefine or delete a named range or expression (§5.11)
    ///
    /// With no target, prints what the name stands for. `sheet info` lists them all.
    Name {
        file: PathBuf,
        /// The name, as a formula would spell it: a letter or _, then letters, digits or _
        name: String,
        /// An address (`Data.B2:C9`), or an expression when it starts with `=`
        /// (`=SUM([$Data.$B$2:.$B$9])`). Omit to print the current definition.
        target: Option<String>,
        /// Delete the name instead
        #[arg(long, conflicts_with = "target")]
        delete: bool,
    },

    /// Filter rows by a set of values per column (§9.4)
    ///
    /// `sheet filter book.ods A1:F12 C=Desk C=Lamp` keeps the rows whose column C holds one
    /// of those values and hides the rest. A value is matched against the cell's display
    /// text, exactly; `C=` keeps the empty ones. With no range, prints each sheet's filter
    /// and the rows it hides.
    Filter {
        file: PathBuf,
        /// The filtered range, header row included: A1:F12, or Data.A1:F12. With --clear,
        /// any address on the sheet whose filter is to go.
        range: Option<String>,
        /// A column and one value it keeps: C=Desk. Repeat for more values or more columns.
        keep: Vec<String>,
        /// Remove the filter instead
        #[arg(long)]
        clear: bool,
        /// The range's first row is data rather than a heading
        #[arg(long)]
        no_header: bool,
    },

    /// Append an empty sheet
    Add {
        file: PathBuf,
        /// The new sheet's name, unique in the document
        name: String,
    },

    /// Rename a sheet
    ///
    /// Formulas naming the old sheet are not rewritten — they go stale, and recalculating
    /// turns them into errors. `sheet info` lists the sheets.
    Rename {
        file: PathBuf,
        /// The sheet to rename
        sheet: String,
        /// Its new name
        name: String,
    },

    /// Delete a sheet and everything on it
    ///
    /// The document's last sheet cannot be deleted. Undo brings the cells back.
    Remove {
        file: PathBuf,
        /// The sheet to delete
        sheet: String,
    },

    /// Recalculate every formula in the document
    Recalc { file: PathBuf },

    /// Undo the last change recorded in the session
    Undo { file: PathBuf },

    /// Redo the last undone change recorded in the session
    Redo { file: PathBuf },

    /// Parse a formula and print it back in normalised form
    ///
    /// Stored form brackets its references — =SUM([.B2:.B4]) — and display form, what a
    /// formula bar shows, does not: =SUM(B2:B4). The two convert losslessly.
    Fmt {
        /// The formula, e.g. '=SUM([.A1:.A2])'
        #[arg(allow_hyphen_values = true)]
        formula: String,
        /// Print it in display form, without the brackets
        #[arg(long, conflicts_with_all = ["from_display", "friendly"])]
        display: bool,
        /// Read display form and print the stored form
        #[arg(long, conflicts_with = "friendly")]
        from_display: bool,
        /// Print a read-only, IDE-flavoured rendering: full function names, one argument
        /// per line past a width, each argument labelled with its parameter. Never parses
        /// back — this is not a fourth spelling of the formula, only an explanation of one.
        #[arg(long)]
        friendly: bool,
        /// With --friendly, keep it on one line however wide it gets — what a formula bar
        /// shows, as against the multi-line explanation
        #[arg(long, requires = "friendly")]
        inline: bool,
    },

    /// List every calculated cell in a document — its formula, its result, what it calls
    ///
    /// Plain arithmetic counts: =A1/2 calls no function and is still a calculation.
    Calculations {
        file: PathBuf,
        /// Only cells whose sheet, address, formula or function names contain this
        #[arg(long, value_name = "TEXT")]
        filter: Option<String>,
    },

    /// List the OpenFormula functions this build implements
    Functions {
        /// Print each one's signature, summary and specification section
        #[arg(long)]
        long: bool,
        /// Only functions whose name contains this
        #[arg(long, value_name = "TEXT")]
        filter: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(report) => {
            report.print(cli.format);
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("grind: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<Report, String> {
    match &cli.command {
        Top::Sheet { command } => run_sheet(command, cli),
        Top::Text { command } => run_text(command, cli),

        // --- suite-level: whatever the document is ---
        //
        // These two are the reason `grind_core::kind` exists. Everything else in this file
        // knows which application it is under; these have to work it out, and they do it by
        // reading the file rather than by trusting its name.
        Top::Info { file } => match document_kind(file)? {
            DocumentKind::Spreadsheet => {
                let app = open_as(file, DocumentKind::Spreadsheet, cli)?;
                let mut report = document(&app, file, false, false, 0);
                report.kind = Some(DocumentKind::Spreadsheet.label());
                Ok(Report::Document(report))
            }
            DocumentKind::Text => {
                let app = open_text(file)?;
                let mut report = text_document(&app, file, false, false);
                report.kind = Some(DocumentKind::Text.label());
                Ok(Report::TextDocument(report))
            }
            kind => Err(unsupported(file, Some(kind))),
        },

        Top::Convert { file, out } => match document_kind(file)? {
            DocumentKind::Spreadsheet => {
                let app = open_as(file, DocumentKind::Spreadsheet, cli)?;
                finish(&app, cli, out, true)
            }
            DocumentKind::Text => {
                let app = open_text(file)?;
                finish_text(&app, cli, out, true)
            }
            kind => Err(unsupported(file, Some(kind))),
        },
    }
}

/// What kind of document a file holds, read from its bytes.
fn document_kind(file: &Path) -> Result<DocumentKind, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    grind_sheet::kind(&bytes).ok_or_else(|| unsupported(file, None))
}

/// The diagnostic for a document this build has no application for — and, when there is one,
/// the command that would have worked. Telling a user what to type instead is most of the
/// value of knowing the kind at all.
fn unsupported(file: &Path, kind: Option<DocumentKind>) -> String {
    match kind {
        Some(kind) => format!(
            "{} is a {}{}",
            file.display(),
            kind.label(),
            match kind.command() {
                Some(command) => format!("; try `grind {command}`"),
                None => ", which this build does not open".to_owned(),
            }
        ),
        None => format!("{}: not an OpenDocument file", file.display()),
    }
}

/// What kind of document a file holds, or a diagnostic naming what it is instead.
///
/// The whole point of doing this *before* opening: §8's reader is tolerant by construction, so
/// handing it a document of the wrong kind produces an empty one rather than an error. A user
/// who typed the wrong subcommand deserves to be told which one is right, not handed a
/// spreadsheet with no cells in it.
fn open_as(file: &Path, wanted: DocumentKind, cli: &Cli) -> Result<App, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    match grind_sheet::kind(&bytes) {
        Some(kind) if kind == wanted => load(file, cli),
        Some(kind) => Err(format!(
            "{} is a {}{}",
            file.display(),
            kind.label(),
            match kind.command() {
                Some(command) => format!("; try `grind {command}`"),
                None => String::new(),
            }
        )),
        None => Err(format!("{}: not an OpenDocument file", file.display())),
    }
}

fn run_sheet(command: &Command, cli: &Cli) -> Result<Report, String> {
    match command {
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
            input,
            value,
            raw,
        } => {
            let app = load(file, cli)?;
            let reference = a1::parse(address).say()?;
            if !a1::is_single(&reference) {
                return Err(format!("{address}: get takes one cell, not a range"));
            }
            let (sheet, pos, _) = a1::resolve(&app, &reference).say()?;
            if *formula {
                let source = app.formula(sheet, pos).say()?.unwrap_or_default();
                return Ok(Report::Text(TextReport {
                    lines: vec![source],
                }));
            }
            if *input {
                // What a GUI puts in its formula bar, and what `sheet set` takes back
                // unchanged — the same text, because it is the same rule.
                return Ok(Report::Text(TextReport {
                    lines: vec![app.input_text(sheet, pos).say()?],
                }));
            }
            if *value {
                return Ok(Report::Text(TextReport {
                    lines: vec![app.value_text(sheet, pos).say()?],
                }));
            }
            Ok(Report::Cells(cells(
                &app,
                file,
                sheet,
                pos,
                pos,
                u32::MAX,
                *raw,
            )?))
        }

        Command::View {
            file,
            address,
            max_rows,
            raw,
        } => {
            let app = load(file, cli)?;
            let (sheet, start, end) = match address {
                Some(address) => a1::resolve(&app, &a1::parse(address).say()?).say()?,
                // No range given: everything the sheet uses.
                None => {
                    let (rows, cols) = app.used_extent(0).say()?;
                    let last = Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
                    (0, Pos::new(0, 0), last)
                }
            };
            Ok(Report::Cells(cells(
                &app, file, sheet, start, end, *max_rows, *raw,
            )?))
        }

        Command::Set {
            file,
            address,
            value,
            text,
            recalc,
        } => {
            let app = load(file, cli)?;
            let (sheet, pos, _) = single(&app, address)?;
            let value = read_stdin_if_dash(value)?;
            // `App::enter` is the typing rule, shared with every GUI: a leading `=` is a
            // formula stored verbatim with its cached value, a leading `'` is text, and
            // `--text` is that same rule spelled as a flag.
            app.enter(sheet, pos, &forced_text(&value, *text), mode(*recalc))
                .say()?;
            finish(&app, cli, file, true)
        }

        Command::Paste {
            file,
            anchor,
            rows,
            recalc,
        } => {
            let app = load(file, cli)?;
            let (sheet, pos, _) = single(&app, anchor)?;
            let text = read_stdin_if_dash(rows)?;
            let rows: Vec<Vec<String>> = text
                .lines()
                .map(|line| line.split('\t').map(str::to_owned).collect())
                .collect();
            let outcome = app.enter_range(sheet, pos, &rows, mode(*recalc)).say()?;
            finish(&app, cli, file, outcome.cells > 0)
        }

        Command::Fill {
            file,
            source,
            address,
            recalc,
        } => {
            let app = load(file, cli)?;
            let (sheet, from, _) = single(&app, source)?;
            let (_, start, end) = a1::resolve(&app, &a1::parse(address).say()?).say()?;
            let outcome = app.fill(sheet, from, start, end, mode(*recalc)).say()?;
            finish(&app, cli, file, outcome.cells > 0)
        }

        Command::Eval {
            file,
            address,
            formula,
        } => {
            let app = load(file, cli)?;
            let (sheet, pos, _) = single(&app, address)?;
            // A read, not an edit: nothing is stored and nothing is written.
            let value = app.preview(sheet, pos, formula).say()?;
            Ok(Report::Text(TextReport {
                lines: vec![report::show(&value)],
            }))
        }

        Command::Clear {
            file,
            address,
            formula_only,
        } => {
            let app = load(file, cli)?;
            if *formula_only {
                // The one form that is a cell rather than a rectangle: it keeps the value.
                let (sheet, pos, _) = single(&app, address)?;
                app.clear_formula(sheet, pos).say()?;
                return finish(&app, cli, file, true);
            }
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(address).say()?).say()?;
            let changed = app.clear_range(sheet, start, end).say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Format {
            file,
            address,
            style,
            decimals,
            grouping,
            symbol,
            locale,
            show,
        } => {
            let app = load(file, cli)?;
            if *show {
                return shown(&app, file, address, false, true);
            }
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(address).say()?).say()?;
            let format = match style.expect("clap requires a style unless --show") {
                Style::General => None,
                Style::Datetime => Some(numfmt::datetime_preset()),
                style => Some(numfmt::preset(kind(style), *decimals, *grouping, symbol)),
            }
            .map(|format| {
                format.in_locale(
                    locale
                        .clone()
                        .or_else(grind_sheet::locale::from_environment),
                )
            });
            let changed = app.set_format(sheet, start, end, format).say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Style {
            file,
            address,
            show,
            bold,
            italic,
            color,
            background,
            align,
            valign,
            wrap,
            size,
            border,
        } => {
            let app = load(file, cli)?;
            if *show {
                return shown(&app, file, address, true, false);
            }
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(address).say()?).say()?;
            let mut want = CellStyle {
                font_weight: bold.then(|| "bold".to_owned()),
                font_style: italic.then(|| "italic".to_owned()),
                font_size: size.clone(),
                color: color.clone(),
                background: background.clone(),
                align: align.map(|a| {
                    match a {
                        // §16.5: the values are relative to the writing direction.
                        Align::Left => "start",
                        Align::Center => "center",
                        Align::Right => "end",
                        Align::Justify => "justify",
                    }
                    .to_owned()
                }),
                vertical_align: valign.map(|v| {
                    match v {
                        VAlign::Top => "top",
                        VAlign::Middle => "middle",
                        VAlign::Bottom => "bottom",
                    }
                    .to_owned()
                }),
                wrap: wrap.then(|| "wrap".to_owned()),
                borders: Default::default(),
            };
            want.set_border(border.clone());
            let changed = app.set_style(sheet, start, end, Some(want)).say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Width {
            file,
            columns,
            length,
            clear,
        } => {
            let app = load(file, cli)?;
            let (sheet, start, end) = tracks(&app, columns)?;
            let range = start.col..end.col + 1;
            let Some(length) = length.clone().or(clear.then(String::new)) else {
                let widths = app.col_widths(sheet).say()?;
                return Ok(lines(widths.into_iter().filter_map(|(col, w)| {
                    range.contains(&col).then(|| (column_name(col), w))
                })));
            };
            let changed = app
                .set_col_width(sheet, range, (!length.is_empty()).then_some(length))
                .say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Height {
            file,
            rows,
            length,
            clear,
        } => {
            let app = load(file, cli)?;
            let (sheet, start, end) = tracks(&app, rows)?;
            let range = start.row..end.row + 1;
            let Some(length) = length.clone().or(clear.then(String::new)) else {
                let heights = app.row_heights(sheet).say()?;
                return Ok(lines(heights.into_iter().filter_map(|(row, h)| {
                    range.contains(&row).then(|| ((row + 1).to_string(), h))
                })));
            };
            let changed = app
                .set_row_height(sheet, range, (!length.is_empty()).then_some(length))
                .say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Hide {
            file,
            tracks,
            unhide,
        } => {
            let app = load(file, cli)?;
            let Some(spec) = tracks else {
                let mut hidden = Vec::new();
                for i in 0..app.sheet_count() {
                    let name = app.sheet_name(i).unwrap_or_default();
                    for col in app.hidden_cols(i).unwrap_or_default() {
                        hidden.push((format!("{name}.{}", column_name(col)), "column".to_owned()));
                    }
                    for row in app.manually_hidden_rows(i).unwrap_or_default() {
                        hidden.push((format!("{name}.{}", row + 1), "row".to_owned()));
                    }
                }
                return Ok(lines(hidden.into_iter()));
            };
            let (sheet, range, is_cols) = hide_tracks(&app, spec)?;
            let changed = match is_cols {
                true => app.set_col_hidden(sheet, range, !unhide).say()?,
                false => app.set_row_hidden(sheet, range, !unhide).say()?,
            };
            finish(&app, cli, file, changed > 0)
        }

        Command::Name {
            file,
            name,
            target,
            delete,
        } => {
            let app = load(file, cli)?;
            if *delete {
                let removed = app.clear_name(name);
                if !removed {
                    return Err(format!("no such name: {name}"));
                }
                return finish(&app, cli, file, true);
            }
            let Some(target) = target else {
                // Reading one, which is a `get` for names and writes nothing.
                let (_, expression) = app
                    .names()
                    .into_iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(name))
                    .ok_or_else(|| format!("no such name: {name}"))?;
                return Ok(Report::Text(TextReport {
                    lines: vec![expression],
                }));
            };

            // Same rule as `set`: a leading `=` means a formula, anything else is an
            // address — `a1::definition`, shared with the shells so the two cannot differ.
            let expression = a1::definition(&app, target).say()?;
            app.set_name(name, &expression).say()?;
            finish(&app, cli, file, true)
        }

        Command::Filter {
            file,
            range,
            keep,
            clear,
            no_header,
        } => {
            let app = load(file, cli)?;
            let Some(range) = range else {
                if *clear {
                    app.set_filter(0, None).say()?;
                    return finish(&app, cli, file, true);
                }
                // Reading: what each sheet filters, and what that hides. 1-based rows,
                // since this is a person reading.
                return Ok(lines((0..app.sheet_count()).filter_map(|i| {
                    let filter = app.filter(i).ok()??;
                    let hidden = app.hidden_rows(i).unwrap_or_default();
                    let name = app.sheet_name(i).unwrap_or_default();
                    let rows: Vec<String> = hidden.iter().map(|r| (r + 1).to_string()).collect();
                    Some((
                        format!(
                            "{}:{}",
                            a1::format(Some(&name), filter.start),
                            a1::format(None, filter.end)
                        ),
                        format!("hides {}", rows.join(",")),
                    ))
                })));
            };
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(range).say()?).say()?;
            if *clear {
                app.set_filter(sheet, None).say()?;
                return finish(&app, cli, file, true);
            }
            let mut filter = grind_sheet::Filter::new(
                // The name LibreOffice gives an autofilter nobody named.
                "__Anonymous_Sheet_DB__0",
                start,
                end,
            );
            filter.contains_header = !no_header;
            for item in keep {
                let (column, value) = item
                    .split_once('=')
                    .ok_or_else(|| format!("{item}: expected COLUMN=VALUE"))?;
                let (_, at, _) = single(&app, &format!("{column}1"))?;
                if at.col < start.col || at.col > end.col {
                    return Err(format!("{column} is outside {range}"));
                }
                filter
                    .keep
                    .entry(at.col - start.col)
                    .or_default()
                    .insert(value.to_owned());
            }
            app.set_filter(sheet, Some(filter)).say()?;
            finish(&app, cli, file, true)
        }

        Command::Add { file, name } => {
            let app = load(file, cli)?;
            app.add_sheet(name).say()?;
            finish(&app, cli, file, true)
        }

        Command::Rename { file, sheet, name } => {
            let app = load(file, cli)?;
            let index = a1::sheet(&app, sheet).say()?;
            app.rename_sheet(index, name).say()?;
            finish(&app, cli, file, true)
        }

        Command::Remove { file, sheet } => {
            let app = load(file, cli)?;
            let index = a1::sheet(&app, sheet).say()?;
            app.remove_sheet(index).say()?;
            finish(&app, cli, file, true)
        }

        Command::Recalc { file } => {
            let app = load(file, cli)?;
            let recalc = app.recalc().say()?;
            // Diagnostics to stderr, so the warning cannot corrupt parseable output. This
            // build implements a subset of OpenFormula; a document using anything else
            // loses a good cached value to #NAME? here, and saying so is the difference
            // between a recalculation and silent data loss. `sheet undo` is the way back.
            if recalc.spoiled > 0 {
                eprintln!(
                    "sheet: {} cell(s) became errors — a function this build does not \
                     implement; undo to restore, or see `grind sheet functions`",
                    recalc.spoiled
                );
            }
            finish(&app, cli, file, recalc.changed > 0)
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

        Command::Fmt {
            formula,
            display,
            from_display,
            friendly,
            inline,
        } => {
            use grind_sheet::formula::display;
            let line = match (display, from_display, friendly) {
                (true, _, _) => display::to_display(formula).say()?,
                (_, true, _) => display::from_display(formula).say()?,
                (_, _, true) if *inline => {
                    grind_sheet::formula::friendly::explain_inline(formula).say()?
                }
                (_, _, true) => grind_sheet::formula::friendly::explain(formula).say()?,
                _ => format!("={}", grind_sheet::formula::parse::parse(formula).say()?),
            };
            Ok(Report::Text(TextReport { lines: vec![line] }))
        }

        Command::Calculations { file, filter } => {
            let app = load(file, cli)?;
            let needle = filter.as_deref().unwrap_or_default();
            let found: Vec<_> = app
                .calculations()
                .into_iter()
                .filter(|calc| calc.matches(needle))
                .collect();
            let mut lines: Vec<String> = found
                .iter()
                .map(|calc| {
                    format!(
                        "{}\t{}\t{}\t{}",
                        calc.address(),
                        calc.formula,
                        calc.value,
                        calc.functions.join(" ")
                    )
                })
                .collect();
            let tally = grind_sheet::function_tally(&found);
            let counted = match found.len() {
                1 => "1 calculation".to_owned(),
                n => format!("{n} calculations"),
            };
            let summary = match tally.is_empty() {
                true => counted,
                false => format!(
                    "{counted} — {}",
                    tally
                        .iter()
                        .map(|(name, count)| format!("{name} ×{count}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            lines.push(summary);
            Ok(Report::Text(TextReport { lines }))
        }

        Command::Functions { long, filter } => {
            let names = grind_sheet::formula::funcs::implemented();
            let matches = |name: &str| {
                filter
                    .as_ref()
                    .is_none_or(|f| name.to_uppercase().contains(&f.to_uppercase()))
            };
            // The catalog is the same list with the spec's own summary and syntax beside
            // each name — what a GUI's autocomplete offers, so the two cannot disagree.
            let mut lines: Vec<String> = match long {
                true => grind_sheet::formula::funcs::catalog()
                    .iter()
                    .filter(|info| matches(info.name))
                    .map(|info| {
                        // The friendly signature carries the alias in its head, so it is one
                        // column rather than two: `Present Value(Rate; Number Of Periods; …)`.
                        let friendly = grind_sheet::formula::friendly::signature(info.name)
                            .map(|(head, params)| format!("{head}({})", params.join("; ")))
                            .unwrap_or_else(|| info.name.to_owned());
                        let category = grind_sheet::formula::funcs::category(info);
                        format!(
                            "{}\t{friendly}\t{category}\t{}\t§{}",
                            info.signature, info.brief, info.section
                        )
                    })
                    .collect(),
                false => names
                    .iter()
                    .filter(|n| matches(n))
                    .map(|n| (*n).to_owned())
                    .collect(),
            };
            lines.sort();
            let beyond = grind_sheet::formula::funcs::beyond_small_group();
            // Not "112 of 110": the Small Group is the conformance claim and the functions
            // moved in beside it are counted apart, or the line reads as broken arithmetic.
            let mut summary = format!(
                "{} of the Small Group's {}",
                names.len() - beyond.len(),
                grind_sheet::formula::funcs::SMALL_GROUP
            );
            if !beyond.is_empty() {
                summary.push_str(&format!(
                    " — plus {} beyond it: {}",
                    beyond.len(),
                    beyond.join(", ")
                ));
            }
            lines.push(summary);
            Ok(Report::Text(TextReport { lines }))
        }
    }
}

// --- helpers ---

/// A core error reaching the user as text.
///
/// The CLI's own failures are already strings — a bad flag, a file that exists — so this is
/// the one adapter between the two, rather than a closure at every call site.
trait Say<T> {
    fn say(self) -> Result<T, String>;
}

impl<T, E: std::fmt::Display> Say<T> for std::result::Result<T, E> {
    fn say(self) -> Result<T, String> {
        self.map_err(|e| e.to_string())
    }
}

/// A locale tag as a person types one — `de-DE`, or a bare `de`.
fn locale(value: &str) -> Result<grind_sheet::locale::Locale, String> {
    grind_sheet::locale::Locale::parse(value)
        .ok_or_else(|| format!("{value}: expected a language tag like de-DE"))
}

/// A colour as ODF spells one. Checked here rather than in the core: this is where a user's
/// typing enters, and a *document's* value is whatever the document said.
fn color(value: &str) -> Result<String, String> {
    // A palette name first, so `--color navy` and a GUI's navy swatch write the same
    // attribute (`style::PALETTE`). Anything else has to be a colour a document can hold.
    if let Some(hex) = style::palette(value) {
        return Ok(hex.to_owned());
    }
    let hex = value.strip_prefix('#').unwrap_or_default();
    if value == "transparent" || (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())) {
        return Ok(value.to_owned());
    }
    Err(format!(
        "{value}: expected #rrggbb, transparent, or a palette name ({})",
        style::PALETTE
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// A border as its three parts, so a typo becomes an error here rather than an attribute
/// LibreOffice silently drops. Its colour takes a palette name too, which means resolving
/// one — a name reaching the file would be an attribute LibreOffice drops silently.
fn border(value: &str) -> Result<String, String> {
    let malformed = || {
        format!(
            "{value}: expected a width, a line and a colour, \
                 e.g. \"0.5pt solid #000000\""
        )
    };
    let fields: Vec<&str> = value.split_whitespace().collect();
    let [width, line, name] = fields[..] else {
        return Err(malformed());
    };
    // The width is kept as it was typed; only the colour is rewritten.
    let resolved = format!("{width} {line} {}", color(name)?);
    match style::border_parts(&resolved) {
        Some(_) => Ok(resolved),
        None => Err(malformed()),
    }
}

/// The core's [`numfmt::Kind`] for a command-line style. `General` and `Datetime` never get
/// here: neither is a family.
fn kind(style: Style) -> numfmt::Kind {
    match style {
        Style::Number => numfmt::Kind::Number,
        Style::Percent => numfmt::Kind::Percentage,
        Style::Currency => numfmt::Kind::Currency,
        Style::Date => numfmt::Kind::Date,
        Style::Time => numfmt::Kind::Time,
        Style::Boolean => numfmt::Kind::Boolean,
        Style::Text => numfmt::Kind::Text,
        Style::General | Style::Datetime => unreachable!("handled by the caller"),
    }
}

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
///
/// Also the one place staleness is reported, because it is the one place every mutating
/// command passes through. A cached value and a formula are two claims about the same cell,
/// and editing a cell a formula *reads* makes them disagree without touching the formula's
/// own cell — so `set B2 4321` leaves `B5 = SUM([.B2:.B4])` on disk next to the total it
/// used to have. ODF has no dirty bit, and every reader including LibreOffice shows the
/// cached value, so a document written that way is quietly wrong until someone recalculates
/// it.
///
/// Warned about rather than fixed. Recalculating on every edit would be the wrong default
/// for the same reason `sheet recalc` prints `spoiled`: this build implements the Small
/// Group, a document is free to use any of Part 4's other ~370 functions, and recalculating
/// one that does turns good cached values into `#NAME?`. Choosing that is the user's, which
/// is what a separate command means.
fn finish(app: &App, cli: &Cli, file: &Path, changed: bool) -> Result<Report, String> {
    let written = changed && !cli.dry_run;
    if written {
        app.save_file(file).say()?;
    }
    save_session(app, cli)?;

    // Only after a change, and only over a document that has formulas — `stale` costs a
    // recalculation, and asking it of `sheet info` would make reading a document as
    // expensive as recalculating one.
    let stale =
        match changed && (0..app.sheet_count()).any(|i| app.formula_count(i).unwrap_or(0) > 0) {
            true => app.stale(),
            false => Default::default(),
        };
    if stale.changed > 0 {
        eprintln!(
            "grind: {} formula cell(s) now disagree with their cached value — \
             run `grind sheet recalc`{}",
            stale.changed,
            match stale.spoiled {
                0 => String::new(),
                n => format!(
                    ", though {n} of them would become errors — a name that is no longer \
                     defined, or a function this build does not implement; see \
                     `grind sheet functions`"
                ),
            }
        );
    }
    Ok(Report::Document(document(
        app,
        file,
        changed,
        written,
        stale.changed,
    )))
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

fn document(app: &App, file: &Path, changed: bool, written: bool, stale: usize) -> DocumentReport {
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
        kind: None,
        changed,
        written,
        stale,
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
#[allow(clippy::too_many_arguments)]
fn cells(
    app: &App,
    file: &Path,
    sheet: usize,
    start: Pos,
    end: Pos,
    max_rows: u32,
    raw: bool,
) -> Result<CellsReport, String> {
    let last_row = end
        .row
        .min(start.row.saturating_add(max_rows.saturating_sub(1)));
    let rows = start.row..last_row.saturating_add(1);
    let cols = start.col..end.col.saturating_add(1);
    let name = app.sheet_name(sheet).say()?;
    let viewport = app.get_viewport(sheet, rows.clone(), cols.clone()).say()?;

    let mut out = Vec::new();
    for row in rows.clone() {
        for col in cols.clone() {
            let pos = Pos::new(row, col);
            let value = viewport.get(row, col).cloned().unwrap_or(CellValue::Empty);
            out.push(Cell {
                address: a1::format(None, pos),
                value: report::show(&value),
                text: viewport.text(row, col).unwrap_or_default().to_owned(),
                kind: report::kind(&value),
                formula: app.formula(sheet, pos).say()?,
            });
        }
    }
    Ok(CellsReport {
        path: show_path(file),
        sheet: name,
        raw,
        cells: out,
        rows: rows.end - rows.start,
        cols: cols.end - cols.start,
    })
}

/// A run of columns (`B`, `B:D`, `Data.B:D`) or of rows (`3`, `3:7`) as the sheet and the
/// corners `core::a1` resolves it to.
///
/// A single track is spelled as a range of one and goes down the same path, so this stays
/// two lines and does no index arithmetic — §5.8's whole-column and whole-row forms are
/// already what a lone `B` or `3` means, and the only `+ 1` in the workspace stays in `a1`.
fn tracks(app: &App, spec: &str) -> Result<(usize, Pos, Pos), String> {
    let spec = match spec.contains(':') {
        true => spec.to_owned(),
        false => format!("{spec}:{spec}"),
    };
    a1::resolve(app, &a1::parse(&spec).say()?).say()
}

/// [`tracks`], plus which axis the spec named — a whole-column reference (`B`, `B:D`) has
/// no row of its own to be a whole one, and a whole-row reference is the same the other way
/// round, which is what `hide` needs to know and `width`/`height` do not, since each of
/// those already commits to one axis by its own name.
fn hide_tracks(app: &App, spec: &str) -> Result<(usize, std::ops::Range<u32>, bool), String> {
    let full = match spec.contains(':') {
        true => spec.to_owned(),
        false => format!("{spec}:{spec}"),
    };
    let reference = a1::parse(&full).say()?;
    let is_cols = reference.start.col.is_some() && reference.start.row.is_none();
    let (sheet, start, end) = a1::resolve(app, &reference).say()?;
    let range = match is_cols {
        true => start.col..end.col + 1,
        false => start.row..end.row + 1,
    };
    Ok((sheet, range, is_cols))
}

/// `key<TAB>value` lines, the shape every `--show` prints in.
fn lines(rows: impl Iterator<Item = (String, String)>) -> Report {
    Report::Text(TextReport {
        lines: rows.map(|(key, value)| format!("{key}\t{value}")).collect(),
    })
}

fn single(app: &App, address: &str) -> Result<(usize, Pos, Pos), String> {
    let reference = a1::parse(address).say()?;
    if !a1::is_single(&reference) {
        return Err(format!("{address}: expected one cell, not a range"));
    }
    a1::resolve(app, &reference).say()
}

/// `--show` for both `style` and `format`: one cell's styling, its format, or both.
///
/// One cell rather than a rectangle, because the answer for a rectangle is either "they
/// agree" or a list, and a caller that wants the list already has `sheet view`. It reads and
/// writes nothing, so no `finish` and no `stale` warning.
/// `grind text format --show`, as lines: one `property<TAB>value` per property that is set,
/// and nothing at all for a plain span.
///
/// The values are ODF's own, unchanged — `bold`, `12pt`, `#001f3f` — because that is what the
/// model carries and re-spelling them here would invent a second vocabulary for a shell to
/// parse. Properties the span does not agree about are absent, which is the same answer a
/// toolbar shows over a mixed selection (`grind_text::App::char_style`).
fn describe_char_style(style: &grind_text::CharStyle) -> Vec<String> {
    [
        ("font", &style.font_family),
        ("size", &style.font_size),
        ("weight", &style.font_weight),
        ("style", &style.font_style),
        ("underline", &style.underline),
        ("strike", &style.line_through),
        ("color", &style.color),
        ("background", &style.background),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.as_ref().map(|value| format!("{name}\t{value}")))
    .collect()
}

fn shown(
    app: &App,
    file: &Path,
    address: &str,
    style: bool,
    format: bool,
) -> Result<Report, String> {
    let (sheet, pos, _) = single(app, address)?;
    Ok(Report::CellStyle(Box::new(report::CellStyleReport {
        path: file.display().to_string(),
        sheet: app.sheet_name(sheet).say()?,
        address: a1::format(None, pos),
        style: match style {
            true => app.style_at(sheet, pos).say()?,
            false => None,
        },
        format: match format {
            true => app.format_at(sheet, pos).say()?,
            false => None,
        },
    })))
}

/// `--text`, as the core's typing rule spells it. The rule lives in `App::enter`, so a
/// document edited from a shell and one edited from the CLI cannot disagree about what
/// `123` in a cell means.
fn forced_text(value: &str, force: bool) -> String {
    match force {
        true => format!("'{value}"),
        false => value.to_owned(),
    }
}

fn mode(recalc: bool) -> RecalcMode {
    match recalc {
        true => RecalcMode::Document,
        false => RecalcMode::No,
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
