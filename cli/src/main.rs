// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind` — the suite CLI, and the parity ratchet.
//!
//! Every capability the core has is reachable from here, and `doc/cli-parity-sheet.md` plus
//! `tests/parity.rs` make that a build failure rather than a promise. A subcommand is one
//! `match` arm that drives [`App`]; anything longer than that belongs in the core.
//!
//! [`args`] is the command line as clap parses it, [`sheet`] and [`text`] are the two
//! applications' arms, and this file is the suite-level verbs and what the three share.
//! Diagnostics go to stderr, results to stdout, and an error never appears on stdout in either
//! format.

mod args;
mod fit;
mod init;
mod report;
mod sheet;
mod text;

use args::*;
use clap::{Parser, Subcommand, ValueEnum};
use grind_core::lint::{Options as LintOptions, Report as LintFindings, Rule, Severity};
use grind_sheet::a1;
use grind_sheet::formula::lex::column_name;
use grind_sheet::numfmt;
use grind_sheet::style::{self, CellStyle};
use grind_sheet::view::Overlays;
use grind_sheet::{App, CellValue, DocumentKind, Pos, RecalcMode, Session};
use grind_text::App as TextApp;
use report::{
    Cell, CellsReport, DocumentReport, Format, LintReport, Name, Report, SheetInfo, Shown,
    TextDocumentReport, TextReport,
};
use sheet::*;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use text::*;

/// Open a text document, refusing one that is not.
fn open_text(file: &Path) -> Result<TextApp, String> {
    let app = TextApp::new();
    app.open_file(file)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(app)
}

/// [`open_text`], with spelling checked the way the person has said text documents should be
/// (`grind text spelling`) — by default in the document's own language, when this build has the
/// dictionaries — and grammar checked by the built-in checker: what `lint` opens with, so its
/// `misspelt`, `grammar` and `style` rules have something to ask. A document in a language there
/// is no dictionary for is linted without spelling, silently: `grind text spell` is where that
/// is worth saying.
fn open_text_checked(file: &Path, hints: bool) -> Result<TextApp, String> {
    let app = open_text(file)?;
    #[cfg(feature = "spell")]
    let _ = grind_spell::preference::load(DocumentKind::Text).apply(&app);
    // The built-in checker, its opt-in rules with the hints: what `--hints` asks for is the
    // fuller look.
    #[cfg(feature = "grammar")]
    app.set_checker(Some(std::sync::Arc::new(grind_grammar::Builtin {
        picky: hints,
    })));
    #[cfg(not(feature = "grammar"))]
    let _ = hints;
    Ok(app)
}

/// `grind <app> spelling [setting]`: print, or set and then print, the remembered spelling setting
/// for one kind of document — the same file the windows read and write.
#[cfg(feature = "spell")]
fn spelling_setting(kind: DocumentKind, setting: &Option<String>) -> Result<Report, String> {
    use grind_spell::{Setting, preference};
    if let Some(tag) = setting {
        let chosen = Setting::from_tag(tag).ok_or_else(|| {
            format!("{tag:?} is none of auto, off, or a language: en, de, fr, es, it, pt, pl")
        })?;
        preference::save(kind, chosen).map_err(|e| e.to_string())?;
    }
    let now = preference::load(kind);
    let place = preference::path().map_or_else(String::new, |p| p.display().to_string());
    text_lines(vec![format!("{}\t{place}", now.tag())])
}

/// `--language`: a tag naming one of the dictionaries this build carries.
#[cfg(feature = "spell")]
fn spell_language(tag: &str) -> Result<grind_spell::Language, String> {
    grind_spell::Language::from_tag(tag).ok_or_else(|| {
        let have: Vec<&str> = grind_spell::Language::ALL.iter().map(|l| l.tag()).collect();
        format!(
            "no dictionary for {tag:?}; this build has {}",
            have.join(", ")
        )
    })
}

/// `grind build` over a script that ended with a map of documents: each into `dir` under its
/// name, in the form `--as` names — flat ODF by default, because it is the one that diffs.
fn write_series(
    all: &[(String, std::rc::Rc<grind_build::Artifact>)],
    dir: &Path,
    series_as: Option<&str>,
    no_recalc: bool,
    cli: &Cli,
) -> Result<Report, String> {
    use grind_build::Artifact;
    let kind = all[0].1.kind();
    let ext = match (series_as, kind) {
        (Some(ext), _) => ext.trim_start_matches('.'),
        (None, DocumentKind::Spreadsheet) => "fods",
        (None, _) => "fodt",
    };
    if !matches!(ext, "fodt" | "odt" | "fods" | "ods" | "grind" | "pdf") {
        return Err(format!(
            "{ext:?} is not something a series is written as: fodt, odt, fods, ods, grind or pdf"
        ));
    }
    if dir.extension().is_some() {
        return Err(format!(
            "{} has an extension, and a series is written into a directory: one file per document",
            dir.display()
        ));
    }
    if !cli.dry_run {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut written = Vec::new();
    for (name, one) in all {
        let path = dir.join(format!("{name}.{ext}"));
        match &**one {
            Artifact::Spreadsheet(app) => {
                if !no_recalc {
                    app.recalc().say()?;
                }
                if !cli.dry_run {
                    app.save_file(&path).say()?;
                }
            }
            Artifact::Text(app) => match ext {
                "pdf" => {
                    #[cfg(feature = "pdf")]
                    if !cli.dry_run {
                        let options = grind_print::Options {
                            paper: None,
                            title: Some(name.clone()),
                        };
                        let (bytes, _) = grind_print::export(app, fonts(app, true), &options)?;
                        grind_core::atomic::write(&path, &bytes)
                            .map_err(|e| format!("{}: {e}", path.display()))?;
                    }
                    #[cfg(not(feature = "pdf"))]
                    return Err("this build has no PDF export".to_owned());
                }
                _ => {
                    if !cli.dry_run {
                        app.save_file(&path).map_err(|e| e.to_string())?;
                    }
                }
            },
            Artifact::Series(_) => unreachable!("a series is never nested"),
        }
        written.push(show_path(&path));
    }
    text_lines(written)
}

/// The fonts a PDF command sets the document in: the machine's as well as the bundled ones,
/// unless `--bundled-fonts` asks for output that is the same everywhere.
#[cfg(feature = "pdf")]
fn fonts(app: &TextApp, bundled_only: bool) -> grind_print::Fonts {
    match bundled_only {
        true => grind_print::Fonts::bundled(),
        false => grind_print::fonts_for(app),
    }
}

/// Resolve one address against the document.
fn at(app: &TextApp, address: &str) -> Result<usize, String> {
    let loc = grind_text::loc::parse(address).map_err(|e| e.to_string())?;
    app.resolve(&loc).map_err(|e| e.to_string())
}

/// A plain list of lines, which is most of what `grind text` prints.
fn text_lines(lines: Vec<String>) -> Result<Report, String> {
    Ok(Report::Text(TextReport { lines }))
}

/// Every node of the projection as a VS Code snippet — `.vscode/grind-projection.code-snippets`,
/// the one copy, read at compile time so `grind definitions --projection` needs no repository.
const PROJECTION_SNIPPETS: &str = include_str!("../../.vscode/grind-projection.code-snippets");

/// `project --anchors --as-read`: the span map of the file as written, in `--anchors`' own
/// shape — `address<TAB>line<TAB>start<TAB>end` — so one reader takes either.
///
/// The line is counted in the file itself, which is the text the spans are into; a document
/// that was not read from a projection has no such map, and saying so beats printing nothing.
fn as_read_report(
    file: &Path,
    anchors: Vec<grind_core::projection::Anchor>,
) -> Result<Report, String> {
    if anchors.is_empty() {
        return Err(format!(
            "{} was not read from a projection; --as-read describes a .grind as written",
            file.display()
        ));
    }
    let text = std::fs::read_to_string(file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let line = |offset: usize| text[..offset.min(text.len())].matches('\n').count() + 1;
    text_lines(
        anchors
            .iter()
            .map(|a| {
                format!(
                    "{}\t{}\t{}\t{}",
                    a.address,
                    line(a.span.start),
                    a.span.start,
                    a.span.end
                )
            })
            .collect(),
    )
}

/// `<app> project`, for either application — the projection, or one of the two maps beside it.
///
/// One function because there is one answer: `Projection` is the container's type, the same
/// whatever document it came from, so printing it is container work and not an application's.
/// The verb is per app (R9, and `App::project` is per app), and what it prints is not.
fn project_report(
    projection: &grind_sheet::projection::Projection,
    tokens: bool,
    anchors: bool,
) -> Result<Report, String> {
    let text = projection.text();
    // The line a byte offset is on, 1-based — the number an editor puts in its gutter, so that
    // a span map printed here can be fed straight to one.
    let line = |offset: usize| text[..offset].matches('\n').count() + 1;
    text_lines(match (tokens, anchors) {
        (true, _) => projection
            .tokens()
            .iter()
            .map(|token| {
                format!(
                    "{}\t{}\t{}",
                    token.kind.name(),
                    token.span.start,
                    token.span.end
                )
            })
            .collect(),
        (_, true) => projection
            .anchors()
            .iter()
            .map(|anchor| {
                format!(
                    "{}\t{}\t{}\t{}",
                    anchor.address,
                    line(anchor.span.start),
                    anchor.span.start,
                    anchor.span.end
                )
            })
            .collect(),
        // `lines()` drops the trailing newline the projection ends with; `text_lines` puts one
        // back, so this is the file byte for byte.
        _ => text.lines().map(str::to_owned).collect(),
    })
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
        reading_minutes: counts.reading_minutes(),
        bookmarks: app.bookmarks().into_iter().map(|(name, _)| name).collect(),
        page: None,
        can_undo: app.can_undo(),
        can_redo: app.can_redo(),
    }
}

/// The one directory a script may read data from: `--data`, or the script's own. The walls around
/// it are `grind_build::Directory`'s.
fn data_root(script: &Path, data: Option<&Path>) -> Result<grind_build::Directory, String> {
    let root = match data {
        Some(dir) => dir.to_path_buf(),
        None => match script.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        },
    };
    grind_build::Directory::new(&root).map_err(|e| format!("--data {e}"))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // The language server owns stdout for as long as it runs: nothing may be printed around
    // it, so it never reaches the report below.
    if let Top::Lsp = cli.command {
        return lsp();
    }
    match run(&cli) {
        Ok(report) => {
            report.print(cli.format);
            // Only `lint` can fail this way, and only on an *error*-severity finding — so CI
            // can gate on a document contradicting itself without every warning breaking a
            // build (`report::LintReport::failed`).
            match report.failed() {
                true => ExitCode::FAILURE,
                false => ExitCode::SUCCESS,
            }
        }
        Err(message) => {
            eprintln!("grind: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `grind lsp`: the server, with what it may not link itself handed in (`grind_lsp::Host`).
#[cfg(feature = "lsp")]
fn lsp() -> ExitCode {
    struct Host;
    impl grind_lsp::Host for Host {
        fn definitions(&self) -> String {
            grind_build::definitions()
        }
        fn check_script(&self, source: &str) -> Result<(), grind_lsp::ScriptError> {
            grind_build::check(source, "<script>").map_err(|e| grind_lsp::ScriptError {
                line: e.line,
                column: e.column,
                message: e.message,
            })
        }
        fn attach_spelling_sheet(&self, app: &App) {
            check_spelling(app);
        }
        fn attach_spelling_text(&self, app: &TextApp) {
            #[cfg(feature = "spell")]
            let _ = grind_spell::preference::load(DocumentKind::Text).apply(app);
            #[cfg(not(feature = "spell"))]
            let _ = app;
        }
    }
    match grind_lsp::serve(&Host) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("grind lsp: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "lsp"))]
fn lsp() -> ExitCode {
    eprintln!("grind: this build has no language server (built without the `lsp` feature)");
    ExitCode::FAILURE
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
                report.page = Some(report::PageReport::new(app.page()));
                Ok(Report::TextDocument(report))
            }
            kind => Err(unsupported(file, Some(kind))),
        },

        // The suite-level third: the kind decides which application's rules run, and the
        // answer has one shape either way because a diagnostic is document-type-neutral.
        Top::Lint { file, hints, off } => {
            let options = lint_options(*hints, off);
            let findings = match document_kind(file)? {
                DocumentKind::Spreadsheet => {
                    let app = open_as(file, DocumentKind::Spreadsheet, cli)?;
                    check_spelling(&app);
                    app.lint(&options)
                }
                DocumentKind::Text => open_text_checked(file, *hints)?.lint(&options),
                kind => return Err(unsupported(file, Some(kind))),
            };
            Ok(lint_report(file, findings))
        }

        // The same function the two application verbs call, so the three cannot print three
        // different projections of one file.
        Top::Project {
            file,
            tokens,
            anchors,
            as_read,
        } => match document_kind(file)? {
            DocumentKind::Spreadsheet => {
                let app = open_as(file, DocumentKind::Spreadsheet, cli)?;
                match as_read {
                    true => as_read_report(file, app.source_anchors()),
                    false => project_report(&app.project(), *tokens, *anchors),
                }
            }
            DocumentKind::Text => {
                let app = open_text(file)?;
                match as_read {
                    true => as_read_report(file, app.source_anchors()),
                    false => project_report(&app.project(), *tokens, *anchors),
                }
            }
            kind => Err(unsupported(file, Some(kind))),
        },

        // The one verb that runs a script, and the only place in the suite that links an
        // evaluator (R11). Note what it does *not* do: it never reads the output, so building
        // over an existing document replaces it rather than editing it — a build product is
        // the script's answer, not a document with a history.
        Top::Build {
            script,
            out,
            series_as,
            no_recalc,
            data,
        } => {
            let source = std::fs::read_to_string(script)
                .map_err(|e| format!("cannot read {}: {e}", script.display()))?;
            // Data comes from one directory a *person* named — `--data`, or the one the script
            // is in, which is what makes `json("prices.json")` mean the file beside it. The
            // walls around it are `grind_build::Directory`'s, not this file's (doc/dsl.md §2).
            let root = data_root(script, data.as_deref())?;
            let built = grind_build::build_with(
                &source,
                &script.display().to_string(),
                std::rc::Rc::new(root),
            )
            .map_err(|e| e.to_string())?;
            match built {
                grind_build::Artifact::Series(all) => {
                    write_series(&all, out, series_as.as_deref(), *no_recalc, cli)
                }
                grind_build::Artifact::Spreadsheet(app) => {
                    if !no_recalc {
                        // A generated document has no cached values until something computes
                        // them, and a formula saved without one renders blank in LibreOffice.
                        let recalc = app.recalc().say()?;
                        if recalc.spoiled > 0 {
                            eprintln!(
                                "grind: {} cell(s) use a function this build does not \
                                 implement and were left as the script wrote them; see \
                                 `grind sheet functions`",
                                recalc.spoiled
                            );
                        }
                    }
                    finish(&app, cli, out, true)
                }
                grind_build::Artifact::Text(app) => finish_text(&app, cli, out, true),
            }
        }

        // The same script, built the same way, and its `test_` functions called with what it
        // built. Nothing is written; a failed test is the command's failure (`Report::failed`).
        Top::Test { script, data } => {
            let source = std::fs::read_to_string(script)
                .map_err(|e| format!("cannot read {}: {e}", script.display()))?;
            let root = data_root(script, data.as_deref())?;
            let tests = grind_build::test::run(
                &source,
                &script.display().to_string(),
                std::rc::Rc::new(root),
            )
            .map_err(|e| e.to_string())?;
            Ok(Report::Tests(report::TestsReport::new(
                &show_path(script),
                &tests,
            )))
        }

        // No document, no script — the vocabulary itself, for an editor. The engine's own answer:
        // what `grind build` registered, so it cannot describe a function that is not there.
        //
        // The projection's snippets are the exception: written by hand, held to the grammar by
        // `cli/tests/editor.rs`, and compiled in so an installed `grind` can hand them to a
        // project with no copy of this repository.
        Top::Definitions {
            snippets,
            projection,
        } => Ok(Report::Text(TextReport {
            lines: match (*projection, *snippets) {
                (true, _) => PROJECTION_SNIPPETS.to_owned(),
                (false, true) => grind_build::snippets(),
                (false, false) => grind_build::definitions(),
            }
            .lines()
            .map(str::to_owned)
            .collect(),
        })),

        // Handled in `main`, before anything that prints: the server owns stdout.
        Top::Lsp => unreachable!("`main` runs the language server itself"),

        // No document and no script: a directory around them (`doc/init.md`).
        Top::Init {
            dir,
            template,
            update,
            force,
        } => Ok(Report::Init(init::init(
            dir,
            init::Options {
                template: *template,
                force: *force,
                update: *update,
            },
            PROJECTION_SNIPPETS,
        )?)),

        // No document: the list is the core's, so every window's About and this agree.
        Top::Licences { list: true } => Ok(Report::Components(
            grind_core::third_party::components()
                .iter()
                .map(Into::into)
                .collect(),
        )),
        Top::Licences { list: false } => Ok(Report::Text(TextReport {
            lines: grind_core::third_party::notices()
                .lines()
                .map(str::to_owned)
                .collect(),
        })),

        #[cfg(feature = "spell")]
        Top::Words { add, remove } => {
            use grind_spell::personal;
            if let Some(typed) = add {
                personal::add(&personal::word(typed)?).map_err(|e| e.to_string())?;
            }
            if let Some(word) = remove
                && !personal::remove(word).map_err(|e| e.to_string())?
            {
                return Err(format!("{word:?} is not in your dictionary"));
            }
            text_lines(personal::words())
        }

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

fn lint_options(hints: bool, off: &[String]) -> LintOptions {
    LintOptions {
        hints,
        off: off.to_vec(),
    }
}

/// The core's findings as the CLI's report: the same list, with the counts a script reads
/// instead of re-deriving and the path that says what was linted.
fn lint_report(file: &Path, findings: LintFindings) -> Report {
    let count = |severity| {
        findings
            .diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count()
    };
    Report::Lint(LintReport {
        path: file.display().to_string(),
        errors: count(Severity::Error),
        warnings: count(Severity::Warning),
        hints: count(Severity::Hint),
        truncated: findings.truncated,
        diagnostics: findings.diagnostics,
    })
}

/// `--rules`: the rule table, `id<TAB>severity<TAB>what`.
///
/// Printed from the same `RULES` array the linter runs, so the list cannot describe rules that
/// do not exist — the `funcs::implemented()` argument, one crate over.
fn rule_list(rules: &[Rule]) -> Report {
    Report::Text(TextReport {
        lines: rules
            .iter()
            .map(|rule| format!("{}\t{}\t{}", rule.id, rule.severity, rule.what))
            .collect(),
    })
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

/// Attach the remembered spreadsheet spelling setting to `app` — nothing at all when it is `off`,
/// or when this build has no dictionaries. A language with no dictionary is linted without
/// spelling, silently: `grind sheet spell` is where that is worth saying.
fn check_spelling(app: &App) {
    #[cfg(feature = "spell")]
    let _ = grind_spell::preference::load(DocumentKind::Spreadsheet).apply(app);
    #[cfg(not(feature = "spell"))]
    let _ = app;
}

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

trait Say<T> {
    fn say(self) -> Result<T, String>;
}

impl<T, E: std::fmt::Display> Say<T> for std::result::Result<T, E> {
    fn say(self) -> Result<T, String> {
        self.map_err(|e| e.to_string())
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
    grind_core::atomic::write(path, session)
        .map_err(|e| format!("cannot write session {}: {e}", path.display()))
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
        locale: app.locale().map(|locale| locale.tag()),
        can_undo: app.can_undo(),
        can_redo: app.can_redo(),
    }
}

/// `key<TAB>value` lines, the shape every `--show` prints in.
fn lines(rows: impl Iterator<Item = (String, String)>) -> Report {
    Report::Text(TextReport {
        lines: rows.map(|(key, value)| format!("{key}\t{value}")).collect(),
    })
}

/// A text file, or standard input for `-`.
///
/// Read as **bytes** and decoded by `csv::decode`, so a file in a legacy encoding — which is
/// what an older Excel writes when it is not asked for UTF-8 — gets a sentence naming the fix
/// rather than mojibake in a document. Guessing the encoding is the alternative, and a wrong
/// guess is silent: `doc/not-doing.md`'s rule about semantics applies to text as much as to
/// numbers. The sentence is the core's (`csv::NOT_UTF8`) now that three windows say it too;
/// what belongs here is the subject in front of it.
fn read_text_file_or_stdin(source: &str) -> Result<String, String> {
    let bytes = match source {
        "-" => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .read_to_end(&mut bytes)
                .map_err(|e| format!("cannot read stdin: {e}"))?;
            bytes
        }
        path => std::fs::read(path).map_err(|e| format!("{path}: {e}"))?,
    };
    grind_sheet::csv::decode(bytes).map_err(|why| format!("{source}: {why}"))
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
