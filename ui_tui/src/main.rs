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
mod code;
mod help;
mod problems;
mod sheet;
mod text;

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
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
         --sheet (the default) or --text says which to start empty.\n\
         \n{}\n{}\n{}",
        crate::help::COMMON,
        crate::sheet::HELP,
        crate::text::HELP,
    )
}

fn main() -> ExitCode {
    let mut kind: Option<DocumentKind> = None;
    let mut path: Option<PathBuf> = None;
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
            "--sheet" => kind = Some(DocumentKind::Spreadsheet),
            "--text" => kind = Some(DocumentKind::Text),
            other if other.starts_with('-') => {
                eprintln!("grind-tui: unknown option {other}");
                return ExitCode::FAILURE;
            }
            other => path = Some(PathBuf::from(other)),
        }
    }

    // A file decides for itself. `--sheet`/`--text` only answer the empty case, and disagreeing
    // with the file is an error rather than a silent override — opening a spreadsheet as a
    // document would show an empty one, which is exactly the confusion `kind` exists to stop.
    let kind = match &path {
        Some(path) => match sniff(path) {
            Ok(found) => {
                if let Some(asked) = kind
                    && asked != found
                {
                    eprintln!(
                        "grind-tui: {} is a {}, not a {}",
                        path.display(),
                        describe(found),
                        describe(asked)
                    );
                    return ExitCode::FAILURE;
                }
                found
            }
            Err(error) => {
                eprintln!("grind-tui: {}: {error}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => kind.unwrap_or(DocumentKind::Spreadsheet),
    };

    let result = match kind {
        DocumentKind::Text => run_text(path),
        _ => run_sheet(path),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("grind-tui: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What kind of document a file holds, read from its bytes.
fn sniff(path: &Path) -> io::Result<DocumentKind> {
    let bytes = std::fs::read(path)?;
    grind_core::kind(&bytes).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "not an ODF spreadsheet or text document",
        )
    })
}

fn describe(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Spreadsheet => "spreadsheet",
        DocumentKind::Text => "text document",
        _ => "document",
    }
}

fn run_sheet(path: Option<PathBuf>) -> io::Result<()> {
    let core = Arc::new(grind_sheet::App::new());
    if let Some(path) = &path {
        core.open_file(path)
            .map_err(|e| io::Error::other(format!("{}: {e}", path.display())))?;
    }
    let redraw = Arc::new(RedrawFlag::default());
    core.set_observer(redraw.clone());
    redraw.raise(); // paint the first frame before waiting for input

    let mut terminal = setup_terminal()?;
    let result = event_loop(
        &mut terminal,
        &redraw,
        &mut sheet::app::App::new(core, redraw.clone(), path),
    );
    restore_terminal();
    result
}

fn run_text(path: Option<PathBuf>) -> io::Result<()> {
    let core = Arc::new(grind_text::App::new());
    if let Some(path) = &path {
        core.open_file(path)
            .map_err(|e| io::Error::other(format!("{}: {e}", path.display())))?;
    }
    let redraw = Arc::new(RedrawFlag::default());
    core.set_observer(redraw.clone());
    redraw.raise();

    let mut terminal = setup_terminal()?;
    let result = event_loop(
        &mut terminal,
        &redraw,
        &mut text::app::App::new(core, redraw.clone(), path),
    );
    restore_terminal();
    result
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
        }
    };
}

shell!(sheet::app::App);
shell!(text::app::App);

fn event_loop<S: Shell>(terminal: &mut Tui, redraw: &RedrawFlag, shell: &mut S) -> io::Result<()> {
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
    }
    Ok(())
}

fn setup_terminal() -> io::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // The alternate screen keeps the user's scrollback intact.
    execute!(stdout, EnterAlternateScreen)?;

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
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}
