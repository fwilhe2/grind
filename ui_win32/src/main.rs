// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-win32` — the Windows shell over the suite.
//!
//! Rust-direct like the CLI, the terminal and the two GTK windows: it depends on `grind-core`,
//! `grind-sheet` and `grind-text` as ordinary Cargo dependencies, with no FFI, no bindings and
//! no generated code. **One binary, both document types**, chosen by [`grind_core::kind()`]
//! reading the file's *bytes* — never its name, because a spreadsheet does not become a
//! document by being called one.
//!
//! What it buys is one line and is meant literally: **the built `.exe` needs nothing that
//! Windows does not already ship.** No .NET runtime, no Windows App SDK, no Visual C++
//! redistributable — the MSVC C runtime is linked statically by `.cargo/config.toml`, and
//! `win32.yml` reads the import table back rather than trusting it.
//!
//! What it costs is the Fluent control set. This window is drawn with GDI, so it follows
//! Windows' *conventions* — the shell font, the user's wheel and caret settings, Ctrl+Y for
//! redo, a Save/Don't Save/Cancel dialog on close, a dark title bar when the theme is dark —
//! without using Windows' *controls*. `doc/windows-shell.md` is normative for all of it and is
//! where the argument, the milestones and the named gaps live.
//!
//! This file owns the process and nothing else. The window is `win.rs`'s, the grid's arithmetic
//! is `sheet/geom.rs`'s and its drawing `sheet/draw.rs`'s — **W1 is the window and the read-only
//! grid**; the text pane is W5 and is refused here rather than opened as an empty spreadsheet.

// A GUI application, not a console one. Without this, launching from Explorer flashes up a
// console window behind the shell and leaves it there for the life of the process. The cost is
// that there is no stderr to print to, which is why the paths below end in a message box —
// the same bargain every Windows GUI application makes.
#![cfg_attr(windows, windows_subsystem = "windows")]

// Off Windows nothing calls into these, so every function reads as dead code and every import
// the Windows half needs reads as unused — but their tests are the reason this crate builds
// here at all, and silencing the two lints is what keeps `cargo clippy` clean on the
// development machine. Both are scoped to `not(windows)`, so a genuinely dead function is still
// reported where it matters.
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod args;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod menu;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod notice;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod sheet;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod theme;

#[cfg(windows)]
mod dialog;
#[cfg(windows)]
mod gdi;
#[cfg(windows)]
mod win;

use std::path::Path;

use args::Command;
use grind_core::DocumentKind;

fn version() -> String {
    format!(
        "grind-win32 {}",
        grind_core::build_info::describe_version(env!("CARGO_PKG_VERSION"))
    )
}

/// What kind of document a file holds, read from its bytes.
///
/// Decided *before* parsing, because §8's reader is tolerant by construction and would hand
/// back an empty document rather than an error if it were handed the wrong type.
fn sniff(path: &Path) -> Result<DocumentKind, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    grind_core::kind(&bytes).ok_or_else(|| {
        format!(
            "{}: not an ODF spreadsheet or text document",
            path.display()
        )
    })
}

/// Resolve the command line into the document type a window would open, or a message saying
/// why it cannot.
///
/// Split out from [`main`] because it is the whole of the decision and none of the platform:
/// it is exercised by the tests at the bottom of this file on any host, where `run` cannot be.
type Opening = (
    DocumentKind,
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
);

fn resolve(command: Command) -> Result<Opening, String> {
    let Command::Open {
        kind,
        path,
        render_to,
    } = command
    else {
        unreachable!("help, version and errors are handled before this")
    };
    match &path {
        // A file decides for itself; the flag only gets to disagree, and disagreeing is an
        // error rather than an override.
        Some(file) => {
            let found = sniff(file)?;
            let kind = args::reconcile(kind, found, &file.display().to_string())?;
            Ok((kind, path, render_to))
        }
        // Nothing to read, so the flag is the only opinion there is. A spreadsheet by default,
        // matching `grind-tui`.
        None => Ok((kind.unwrap_or(DocumentKind::Spreadsheet), None, render_to)),
    }
}

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;

    let command = args::parse(std::env::args().skip(1));
    match command {
        Command::Help => {
            message_box("grind-win32", args::USAGE, false);
            ExitCode::SUCCESS
        }
        Command::Version => {
            message_box("grind-win32", &version(), false);
            ExitCode::SUCCESS
        }
        Command::Error(message) => {
            message_box(
                "grind-win32",
                &format!("{message}\n\n{}", args::USAGE),
                true,
            );
            ExitCode::from(2)
        }
        open => match resolve(open) {
            Err(message) => {
                message_box("grind-win32", &message, true);
                ExitCode::FAILURE
            }
            // A text document is a document this shell can *identify* and cannot yet open —
            // the pane is W5. Saying so is the honest answer; opening an empty spreadsheet
            // instead would look like the file had failed to load.
            Ok((DocumentKind::Text, ..)) => {
                message_box(
                    "grind-win32",
                    "This is a word processor document, and this build has no text pane yet \
                     (W5 — see doc/windows-shell.md).\n\nIt opens today in grind-tui, in \
                     grind-text-gtk, and in the browser shell.",
                    true,
                );
                ExitCode::FAILURE
            }
            // `--render-to` draws one frame with no window at all and exits, so it comes
            // before the window is opened rather than after (`doc/windows-shell.md`,
            // decision 5). Its errors still go to a message box: this is a GUI-subsystem
            // binary and there is no stderr, even when it is doing something headless.
            Ok((_, path, Some(target))) => match win::render(path, &target) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    message_box("grind-win32", &message, true);
                    ExitCode::FAILURE
                }
            },
            Ok((_, path, None)) => match win::run(path) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    message_box("grind-win32", &message, true);
                    ExitCode::FAILURE
                }
            },
        },
    }
}

/// Say something before there is a window to say it in.
///
/// A GUI-subsystem binary has no console: `eprintln!` here goes nowhere at all when the program
/// is launched from Explorer. Every other shell in this suite can use stderr; this one has to
/// ask the system for a dialog.
#[cfg(windows)]
fn message_box(title: &str, text: &str, error: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW,
    };
    use windows::core::PCWSTR;

    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let text = wide(text);
    let title = wide(title);
    let icon = if error {
        MB_ICONERROR
    } else {
        MB_ICONINFORMATION
    };
    // SAFETY: both pointers are to NUL-terminated buffers that outlive the call, and the
    // dialog is modal, so nothing else in this process runs while it is up.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | icon,
        );
    }
}

/// On anything but Windows this crate still builds, so that the portable half can be compiled
/// and tested on the machine this repository is developed on. The binary itself has nothing to
/// do there — but it reports what it *would* have done, which makes the argument handling
/// runnable rather than only testable.
#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;

    match args::parse(std::env::args().skip(1)) {
        Command::Help => {
            print!("{}", args::USAGE);
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("{}", version());
            ExitCode::SUCCESS
        }
        Command::Error(message) => {
            eprintln!("grind-win32: {message}\n\n{}", args::USAGE);
            ExitCode::from(2)
        }
        open => match resolve(open) {
            Err(message) => {
                eprintln!("grind-win32: {message}");
                ExitCode::FAILURE
            }
            Ok((kind, path, _)) => {
                let what = path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "a new document".to_string());
                eprintln!(
                    "grind-win32 runs on Windows only. Here it would open {what} as a {}.",
                    args::describe(kind)
                );
                ExitCode::FAILURE
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(path: Option<&str>, kind: Option<DocumentKind>) -> Command {
        Command::Open {
            kind,
            path: path.map(std::path::PathBuf::from),
            render_to: None,
        }
    }

    #[test]
    fn an_empty_invocation_opens_a_spreadsheet() {
        assert_eq!(
            resolve(open(None, None)).unwrap(),
            (DocumentKind::Spreadsheet, None, None)
        );
    }

    #[test]
    fn the_flag_decides_when_there_is_no_file() {
        assert_eq!(
            resolve(open(None, Some(DocumentKind::Text))).unwrap(),
            (DocumentKind::Text, None, None)
        );
    }

    /// The type comes out of the bytes, not the name — so a text document called `.fods`
    /// still opens as a text document. `grind_core::kind` is the whole reason that works, and
    /// this test is here so a future refactor cannot quietly start sniffing extensions.
    #[test]
    fn the_bytes_decide_and_not_the_extension() {
        let dir = std::env::temp_dir().join(format!("grind-win32-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lying = dir.join("actually-a-document.fods");
        let bytes =
            grind_text::write_bytes(&grind_text::Document::default(), grind_core::Form::Flat)
                .expect("a default document writes");
        std::fs::write(&lying, bytes).unwrap();

        let (kind, ..) = resolve(open(Some(lying.to_str().unwrap()), None)).unwrap();
        assert_eq!(
            kind,
            DocumentKind::Text,
            "the name says sheet, the bytes say text"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_is_reported_rather_than_invented() {
        // Opening fails on a missing file rather than starting an empty document, so a typo
        // cannot silently create one.
        let error = resolve(open(Some("no-such-file-here.fods"), None)).unwrap_err();
        assert!(error.contains("no-such-file-here.fods"), "{error}");
    }

    #[test]
    fn a_file_that_is_not_a_document_is_refused() {
        let dir = std::env::temp_dir().join(format!("grind-win32-junk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let junk = dir.join("notes.fods");
        std::fs::write(&junk, b"this is not a document at all").unwrap();

        let error = resolve(open(Some(junk.to_str().unwrap()), None)).unwrap_err();
        assert!(error.contains("not an ODF"), "{error}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
