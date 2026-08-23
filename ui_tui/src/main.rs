// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `sheet-tui` — a vi-style terminal shell over `grind-sheet`.
//!
//! Pure Rust, so it depends on `grind-sheet` directly: no FFI, no bindings. The loop is
//! render → block on a key → route it to the core, and every capability it offers also
//! exists in the CLI (doc/plan.md rule 4).
//!
//! The terminal is global state this process borrows. Raw mode and the alternate screen must
//! be handed back on *every* exit path — normal quit, error, or panic — or the user is left
//! with a shell that no longer echoes. [`restore_terminal`] and the panic hook are for that,
//! the same shape as `editor`'s `ui_tui/src/main.rs`.

mod app;
mod keymap;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use app::{App, RedrawFlag};
use grind_sheet::App as CoreApp;

/// The ODF sheet limits, and the only bound `keymap::moved` clamps a plain move to.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

type Tui = Terminal<CrosstermBackend<Stdout>>;

const USAGE: &str = "usage: sheet-tui [file]

No file opens an empty document, like `sheet-gtk`.

Normal mode (vi-style):
  h j k l / arrows   move
  Ctrl+f / Ctrl+b     page down / up
  0 / $               start / end of row
  g / G               A1 / the last used cell
  i, a                edit the cell, keeping its text
  c                   edit the cell, starting empty
  x                   clear the cell
  u / Ctrl+r          undo / redo
  :                   command line

Insert mode:
  Enter    commit and move down
  Esc      cancel

Command line:
  :w [file]   :q   :q!   :wq / :x   :recalc   :sheet <name>   :<address>
";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let arg = args.next();
    if arg.as_deref().is_some_and(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if arg
        .as_deref()
        .is_some_and(|a| a == "-V" || a == "--version")
    {
        println!(
            "sheet-tui {}",
            grind_sheet::build_info::describe_version(env!("CARGO_PKG_VERSION"))
        );
        return ExitCode::SUCCESS;
    }
    let path = arg.map(PathBuf::from);

    let core = Arc::new(CoreApp::new());
    if let Some(path) = &path
        && let Err(error) = core.open_file(path)
    {
        eprintln!("sheet-tui: {}: {error}", path.display());
        return ExitCode::FAILURE;
    }

    let redraw = Arc::new(RedrawFlag::default());
    core.set_observer(redraw.clone());
    redraw.raise(); // paint the first frame before waiting for input

    match run(core, redraw, path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sheet-tui: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(core: Arc<CoreApp>, redraw: Arc<RedrawFlag>, path: Option<PathBuf>) -> io::Result<()> {
    let mut terminal = setup_terminal()?;
    // Run the loop, then restore unconditionally — including when it returned an error,
    // which must not reach the user through a raw-mode terminal.
    let result = event_loop(&mut terminal, core, redraw, path);
    restore_terminal();
    result
}

fn event_loop(
    terminal: &mut Tui,
    core: Arc<CoreApp>,
    redraw: Arc<RedrawFlag>,
    path: Option<PathBuf>,
) -> io::Result<()> {
    let mut app = App::new(core, redraw.clone(), path);

    while !app.should_quit() {
        if redraw.take() {
            terminal.draw(|frame| app.draw(frame))?;
        }
        // Block until something happens; a TUI has no reason to spin.
        match event::read()? {
            Event::Key(key) => app.on_key(key),
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
