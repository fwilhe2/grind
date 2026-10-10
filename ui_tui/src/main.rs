// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-tui` — a vi-style terminal shell over the suite.
//!
//! **One binary, both document types** (`doc/suite.md`, R10 and S8). Which shell runs is
//! decided by [`grind_core::kind()`] reading the *bytes*, never the file name, because a
//! spreadsheet does not become a document by being called one. `.ods`/`.fods` opens
//! [`sheet`], `.odt`/`.fodt` opens [`text`], and an empty invocation opens whichever
//! `--text` or `--sheet` asks for.
//!
//! Pure Rust, so it depends on the two core crates directly: no FFI, no bindings. The loop is
//! render → block on a key → route it to the core, and every capability it offers also exists
//! in the CLI (doc/plan.md rule 4).
//!
//! The terminal is global state this process borrows. Raw mode and the alternate screen must
//! be handed back on *every* exit path — normal quit, error, or panic — or the user is left
//! with a shell that no longer echoes. [`restore_terminal`] and the panic hook are for that.

mod app;
mod chrome;
mod code;
mod help;
mod import;
mod ink;
mod pick;
mod problems;
mod sheet;
mod text;
mod welcome;

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::SetCursorStyle;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use app::RedrawFlag;
use grind_core::DocumentKind;

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// What `--help` prints: the invocation, then the same two help texts `:help` shows inside
/// each shell.
///
/// One source for both, because a key list that is written twice is a key list that is wrong
/// in one of the two places — and the one nobody reads is the one that rots.
fn usage() -> String {
    format!(
        "usage: grind-tui [--sheet|--text] [file]\n\n\
         The document type is read out of the file, not guessed from its name. With no file,\n\
         the welcome screen offers the choice; --sheet or --text skips it and starts that kind empty.\n\
         \n{}\n{}\n{}",
        crate::help::COMMON,
        crate::sheet::HELP,
        crate::text::HELP,
    )
}

fn main() -> ExitCode {
    let mut kind: Option<DocumentKind> = None;
    let mut path: Option<PathBuf> = None;
    let mut asked = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{}", usage());
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!(
                    "grind-tui {}",
                    grind_core::build_info::describe_version(env!("CARGO_PKG_VERSION"))
                );
                return ExitCode::SUCCESS;
            }
            "--sheet" => {
                kind = Some(DocumentKind::Spreadsheet);
                asked = true;
            }
            "--text" => {
                kind = Some(DocumentKind::Text);
                asked = true;
            }
            other if other.starts_with('-') => {
                eprintln!("grind-tui: unknown option {other}");
                return ExitCode::FAILURE;
            }
            other => path = Some(PathBuf::from(other)),
        }
    }

    // A file decides for itself (`grind_core::kind::reconcile`). `--sheet`/`--text` only answer
    // the empty case, and disagreeing with the file is an error rather than a silent override —
    // opening a spreadsheet as a document would show an empty one, which is exactly the
    // confusion `kind` exists to stop.
    let kind = match &path {
        Some(path) => match sniff(path) {
            Ok(found) => {
                match grind_core::kind::reconcile(kind, found, &path.display().to_string()) {
                    Ok(found) => found,
                    Err(error) => {
                        eprintln!("grind-tui: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            Err(error) => {
                eprintln!("grind-tui: {}: {error}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => kind.unwrap_or(DocumentKind::Spreadsheet),
    };

    // Nothing named at all is the welcome screen, not a guess.
    let welcome = path.is_none() && !asked;
    let result = session(kind, path, welcome);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("grind-tui: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The terminal, and the panes that come and go in it: the first is what the command line named,
/// and `:new` and `:open` (`app::Switch`) replace it without leaving the terminal.
fn session(kind: DocumentKind, path: Option<PathBuf>, welcome: bool) -> io::Result<()> {
    // Read before the terminal is taken, so an unreadable file is an error on a normal screen.
    let mut next = Some(match welcome {
        true => welcome_pane(),
        false => prepare(kind, path)?,
    });
    let mut terminal = setup_terminal()?;
    let mut result = Ok(());
    while let Some(pane) = next.take() {
        match run_pane(&mut terminal, pane) {
            Ok(Some(switch)) => {
                let (kind, path) = match switch {
                    app::Switch::New(kind) => (kind, None),
                    app::Switch::Open(path, kind) => (kind, Some(path)),
                    app::Switch::Welcome => {
                        next = Some(welcome_pane());
                        continue;
                    }
                };
                match prepare(kind, path) {
                    Ok(pane) => next = Some(pane),
                    Err(error) => result = Err(error),
                }
            }
            Ok(None) => {}
            Err(error) => result = Err(error),
        }
    }
    restore_terminal();
    result
}

/// A pane, built and loaded but not yet running.
enum Pane {
    Sheet(Box<sheet::app::App>),
    Text(Box<text::app::App>),
    Welcome(Box<welcome::Welcome>),
}

fn welcome_pane() -> Pane {
    let redraw = Arc::new(RedrawFlag::default());
    redraw.raise();
    Pane::Welcome(Box::new(welcome::Welcome::new(redraw)))
}

fn prepare(kind: DocumentKind, path: Option<PathBuf>) -> io::Result<Pane> {
    let redraw = Arc::new(RedrawFlag::default());
    redraw.raise(); // paint the first frame before waiting for input
    match kind {
        DocumentKind::Text => {
            let core = Arc::new(grind_text::App::new());
            let (mut path, mut notice) = (path, None);
            if let Some(given) = path.clone() {
                let bytes = std::fs::read(&given)?;
                let fail = |e: &dyn std::fmt::Display| {
                    io::Error::other(format!("{}: {e}", given.display()))
                };
                if import::is_markdown(&given, &bytes) {
                    // Markdown is opened the way a CSV is: a new document, and no path.
                    notice =
                        Some(import::open_markdown(&core, &given, &bytes).map_err(|e| fail(&e))?);
                    path = None;
                } else if import::is_word(&bytes) {
                    // A Word document is opened the way a workbook is: imported, a new ODF
                    // document, and no path, so `:w` cannot write ODF over the `.docx`.
                    notice = Some(import::open_word(&core, &given, &bytes).map_err(|e| fail(&e))?);
                    path = None;
                } else {
                    core.open_bytes(&given.display().to_string(), &bytes)
                        .map_err(|e| fail(&e))?;
                }
            }
            Ok(Pane::Text(Box::new(
                text::app::App::new(core, redraw, path)
                    .spelling()
                    .noticed(notice),
            )))
        }
        _ => {
            let core = Arc::new(grind_sheet::App::new());
            let (mut path, mut imported) = (path, None);
            if let Some(given) = path.clone() {
                let bytes = std::fs::read(&given)?;
                if import::is_workbook(&bytes) {
                    // No path: `:w` must be told where the ODF document goes.
                    imported = Some(
                        import::open(&core, &given, &bytes)
                            .map_err(|e| io::Error::other(format!("{}: {e}", given.display())))?,
                    );
                    path = None;
                } else if import::is_delimited(&given, &bytes) {
                    // A CSV is opened the way a workbook is: a new document, and no path.
                    imported = Some(
                        import::open_delimited(&core, &given, &bytes)
                            .map_err(|e| io::Error::other(format!("{}: {e}", given.display())))?,
                    );
                    path = None;
                } else {
                    core.open_bytes(&given.display().to_string(), &bytes)
                        .map_err(|e| io::Error::other(format!("{}: {e}", given.display())))?;
                }
            }
            Ok(Pane::Sheet(Box::new(
                sheet::app::App::new(core, redraw, path)
                    .imported(imported)
                    .spelling(),
            )))
        }
    }
}

/// Run one pane until it quits or asks to be replaced.
fn run_pane(terminal: &mut Tui, pane: Pane) -> io::Result<Option<app::Switch>> {
    match pane {
        Pane::Sheet(mut pane) => event_loop(terminal, &mut *pane),
        Pane::Text(mut pane) => event_loop(terminal, &mut *pane),
        Pane::Welcome(mut pane) => event_loop(terminal, &mut *pane),
    }
}

/// What kind of document a file holds, read from its bytes.
fn sniff(path: &Path) -> io::Result<DocumentKind> {
    let bytes = std::fs::read(path)?;
    // An Excel workbook is a spreadsheet this shell *imports* (`import.rs`). `grind_core::kind`
    // does not know it, and should not: it answers which ODF document type some bytes are.
    if import::is_workbook(&bytes) || import::is_delimited(path, &bytes) {
        return Ok(DocumentKind::Spreadsheet);
    }
    if import::is_markdown(path, &bytes) || import::is_word(&bytes) {
        return Ok(DocumentKind::Text);
    }
    grind_core::kind(&bytes).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "not an ODF spreadsheet or text document",
        )
    })
}

/// What the loop needs of a shell — and all either of them has in common.
///
/// Three methods rather than a shared widget: a grid and a flow have no rendering in common,
/// and `doc/suite.md` rejects a generic `App<D: Document>` for the same reason. This is the
/// event loop's shape, not an abstraction over documents.
trait Shell {
    fn draw(&mut self, frame: &mut ratatui::Frame<'_>);
    fn on_key(&mut self, key: ratatui::crossterm::event::KeyEvent);
    fn should_quit(&self) -> bool;
    /// A request to be replaced by another pane, taken once.
    fn take_switch(&mut self) -> Option<app::Switch>;
    /// The redraw flag the loop waits on.
    fn redraw(&self) -> Arc<RedrawFlag>;
}

macro_rules! shell {
    ($t:ty) => {
        impl Shell for $t {
            fn draw(&mut self, frame: &mut ratatui::Frame<'_>) {
                <$t>::draw(self, frame);
            }
            fn on_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
                <$t>::on_key(self, key);
            }
            fn should_quit(&self) -> bool {
                <$t>::should_quit(self)
            }
            fn take_switch(&mut self) -> Option<app::Switch> {
                <$t>::take_switch(self)
            }
            fn redraw(&self) -> Arc<RedrawFlag> {
                <$t>::redraw_flag(self)
            }
        }
    };
}

shell!(sheet::app::App);
shell!(text::app::App);
shell!(welcome::Welcome);

fn event_loop<S: Shell>(terminal: &mut Tui, shell: &mut S) -> io::Result<Option<app::Switch>> {
    let redraw = shell.redraw();
    // The terminal is shared by successive panes, and the last one's picture is still on it.
    terminal.clear()?;
    while !shell.should_quit() {
        if redraw.take() {
            terminal.draw(|frame| shell.draw(frame))?;
        }
        // Block until something happens; a TUI has no reason to spin.
        match event::read()? {
            Event::Key(key) => shell.on_key(key),
            Event::Resize(_, _) => redraw.raise(),
            _ => {}
        }
        if let Some(switch) = shell.take_switch() {
            return Ok(Some(switch));
        }
    }
    Ok(None)
}

fn setup_terminal() -> io::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // The alternate screen keeps the user's scrollback intact. The cursor is a **bar**: it is only
    // ever shown where text is being typed — the formula line and the `:` line — and a bar
    // between two characters is what an insertion point looks like in every editor that
    // distinguishes one from a block cursor over a character.
    execute!(stdout, EnterAlternateScreen, SetCursorStyle::BlinkingBar)?;

    // From here on a panic would leave the terminal unusable, so teardown runs first.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous_hook(info);
    }));

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.clear()?;
    Ok(terminal)
}

/// Undo [`setup_terminal`]. Errors are swallowed on purpose: this runs while unwinding or on
/// the way out, and there is nothing useful left to do about them.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        SetCursorStyle::DefaultUserShape,
        LeaveAlternateScreen
    );
}
