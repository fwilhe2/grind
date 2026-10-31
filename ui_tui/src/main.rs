// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-tui` — a vi-style terminal shell over the suite. **One binary, both document types**:
//! which half runs is decided by [`grind_core::kind()`] reading the bytes, never the name —
//! [`sheet`] for a spreadsheet, [`text`] for a text document, and for an empty invocation
//! whichever `--sheet` or `--text` asks for, else the welcome pane.
//!
//! The terminal is global state this process borrows: raw mode and the alternate screen must be
//! handed back on every exit path, panic included, or the user is left with a shell that no longer
//! echoes ([`restore_terminal`] and the panic hook).

mod app;
mod chrome;
mod code;
mod dict;
mod help;
mod ink;
mod pick;
mod problems;
mod recent;
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
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use app::RedrawFlag;
use grind_core::DocumentKind;

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// What `--help` prints: the invocation, then the same two help texts `:help` shows.
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

    // A file decides for itself (`grind_core::kind::reconcile`); `--sheet`/`--text` only answer
    // the empty case, and disagreeing with the file is an error — a spreadsheet opened as a
    // document would show an empty one.
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
    let (mut file, mut imported) = (None, None);
    if let Some(given) = &path {
        let bytes = std::fs::read(given)?;
        recent::note(given);
        file = Some(
            match grind_open::import_file(given, &bytes)
                .transpose()
                .map_err(io::Error::other)?
            {
                Some(mut import) => {
                    let odf = std::mem::take(&mut import.odf);
                    let name = import.name.clone();
                    imported = Some(import);
                    (name, odf)
                }
                None => (given.display().to_string(), bytes),
            },
        );
    }
    // An import has no path: `:w` must be told where the ODF document goes, and cannot write
    // over the file it came from.
    let path = path.filter(|_| imported.is_none());
    let fail = |name: &str, e: &dyn std::fmt::Display| io::Error::other(format!("{name}: {e}"));
    match kind {
        DocumentKind::Text => {
            let core = Arc::new(grind_text::App::new());
            if let Some((name, bytes)) = &file {
                core.open_bytes(name, bytes).map_err(|e| fail(name, &e))?;
            }
            Ok(Pane::Text(Box::new(
                text::app::App::new(core, redraw, path)
                    .spelling()
                    .grammar()
                    .noticed(imported),
            )))
        }
        _ => {
            let core = Arc::new(grind_sheet::App::new());
            if let Some((name, bytes)) = &file {
                core.open_bytes(name, bytes).map_err(|e| fail(name, &e))?;
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
    grind_open::kind(&path.display().to_string(), &bytes).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "not an ODF spreadsheet or text document",
        )
    })
}

/// What the event loop needs of a shell — the loop's shape, not an abstraction over documents.
trait Shell {
    fn draw(&mut self, frame: &mut ratatui::Frame<'_>);
    fn on_key(&mut self, key: ratatui::crossterm::event::KeyEvent);
    /// A paste, whole — bracketed paste is on, so it does not arrive as keys.
    fn on_paste(&mut self, text: &str);
    fn should_quit(&self) -> bool;
    /// A request to be replaced by another pane, taken once.
    fn take_switch(&mut self) -> Option<app::Switch>;
    /// The redraw flag the loop waits on.
    fn redraw(&self) -> Arc<RedrawFlag>;
    /// Whether work in the background wants the loop to wake without a key.
    fn waiting(&self) -> bool {
        false
    }
    /// Collect or start that work.
    fn tick(&mut self) {}
}

macro_rules! shell {
    ($t:ty) => {
        shell!(@impl $t, {});
    };
    // A shell with work in the background forwards `waiting` and `tick` too.
    ($t:ty, background) => {
        shell!(@impl $t, {
            fn waiting(&self) -> bool {
                <$t>::waiting(self)
            }
            fn tick(&mut self) {
                <$t>::tick(self);
            }
        });
    };
    (@impl $t:ty, { $($background:tt)* }) => {
        impl Shell for $t {
            $($background)*
            fn draw(&mut self, frame: &mut ratatui::Frame<'_>) {
                <$t>::draw(self, frame);
            }
            fn on_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
                <$t>::on_key(self, key);
            }
            fn on_paste(&mut self, text: &str) {
                <$t>::on_paste(self, text);
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
shell!(text::app::App, background);
shell!(welcome::Welcome);

fn event_loop<S: Shell>(terminal: &mut Tui, shell: &mut S) -> io::Result<Option<app::Switch>> {
    let redraw = shell.redraw();
    // The terminal is shared by successive panes, and the last one's picture is still on it.
    terminal.clear()?;
    while !shell.should_quit() {
        shell.tick();
        if redraw.take() {
            terminal.draw(|frame| shell.draw(frame))?;
        }
        // Block until something happens; a TUI has no reason to spin — unless an answer from the
        // background is wanted, which it is woken for five times a second.
        if shell.waiting() && !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) => shell.on_key(key),
            Event::Paste(text) => shell.on_paste(&text),
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
    // The alternate screen keeps the user's scrollback. The cursor is a **bar**: it is shown only
    // where text is typed (the formula line, the `:` line), and an insertion point is a bar
    // between characters.
    execute!(
        stdout,
        EnterAlternateScreen,
        SetCursorStyle::BlinkingBar,
        EnableBracketedPaste
    )?;

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
        DisableBracketedPaste,
        SetCursorStyle::DefaultUserShape,
        LeaveAlternateScreen
    );
}
