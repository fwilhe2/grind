// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-win32` — the Windows shell over the suite, depending on `grind-core`, `grind-sheet` and
//! `grind-text` directly. **One binary, both document types**, chosen by [`grind_core::kind()`]
//! from the file's bytes.
//!
//! **The `.exe` needs nothing Windows does not ship**: no .NET, no Windows App SDK, no Visual C++
//! redistributable — the C runtime is linked statically (`.cargo/config.toml`), and
//! `artifacts.yml` reads the import table back. The cost is the Fluent control set: drawn with
//! GDI, the window follows Windows' conventions — the shell font, the user's wheel and caret
//! settings, Ctrl+Y, Save/Don't Save/Cancel on close, a dark title bar — without its controls.
//! `doc/windows-shell.md` is normative.
//!
//! This file owns the process. The window is `win.rs`'s; the grid's arithmetic `sheet/geom.rs`'s
//! and drawing `sheet/draw.rs`'s; the text pane's `text/`'s and `metrics.rs`'s.

// A GUI application, not a console one: otherwise launching from Explorer leaves a console window
// behind the shell. So there is no stderr, and the paths below end in a message box.
#![cfg_attr(windows, windows_subsystem = "windows")]

// Off Windows nothing calls the Windows half, so its functions read as dead and its imports as
// unused — but the tests are why the crate builds here, so the two lints are silenced for
// `not(windows)` only, and a dead function is still reported where it matters.
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod args;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod assoc;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod code;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod import;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod menu;
// The drawn menu bar's layout, hit-testing and keyboard walk.
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod menubar;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod metrics;
// The modals' geometry — ContentDialog's anatomy, and fitting one to the screen.
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod modal;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod notice;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod problems;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod sheet;
// The format strip both panes draw — its layout portable, its painting `cfg(windows)`.
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod strip;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod surrogate;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod text;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod theme;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod welcome;

#[cfg(windows)]
mod clipboard;
#[cfg(windows)]
mod dialog;
#[cfg(windows)]
mod gdi;
#[cfg(windows)]
mod image;
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

/// What kind of document a file holds, read from its bytes before parsing, since a tolerant reader
/// would return an empty document of the wrong type rather than an error.
fn sniff(path: &Path) -> Result<DocumentKind, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    // A workbook or a CSV is a spreadsheet this shell imports (`import.rs`); `grind_core::kind`
    // answers only which *ODF* type some bytes are, and should.
    if import::is_workbook(&bytes) || import::is_delimited(path, &bytes) {
        return Ok(DocumentKind::Spreadsheet);
    }
    if import::is_markdown(path, &bytes) || import::is_word(&bytes) {
        return Ok(DocumentKind::Text);
    }
    grind_core::kind(&bytes).ok_or_else(|| {
        format!(
            "{}: not an ODF spreadsheet or text document",
            path.display()
        )
    })
}

/// The command line resolved: the document kind a window would open, the file, and — for a render
/// — where the frame goes and in which palette; or a message saying why not. Split from [`main`]
/// as the whole decision and none of the platform, so it is tested on any host. The kind is an
/// `Option` because **"no document" is an answer**: the welcome screen.
type Opening = (
    Option<DocumentKind>,
    Option<std::path::PathBuf>,
    Option<(std::path::PathBuf, bool)>,
);

fn resolve(command: Command) -> Result<Opening, String> {
    let Command::Open {
        kind,
        path,
        render_to,
        dark,
    } = command
    else {
        unreachable!("help, version and errors are handled before this")
    };
    let render = render_to.map(|target| (target, dark));
    match &path {
        // A file decides for itself; the flag only gets to disagree, and disagreeing is an
        // error rather than an override.
        Some(file) => {
            let found = sniff(file)?;
            let kind = grind_core::kind::reconcile(kind, found, &file.display().to_string())?;
            Ok((Some(kind), path, render))
        }
        // Nothing to read, so the flag is the only opinion; with no flag either, nobody has said,
        // which is the welcome screen.
        None => Ok((kind, None, render)),
    }
}

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;

    let argv: Vec<String> = std::env::args().skip(1).collect();
    // Whether this invocation has anybody in front of it. `--render-to` runs on machines with no
    // display and, on a CI runner, no interactive session, where a `MessageBoxW` waits for an OK
    // that can never be clicked — one mis-quoted argument timed a job out after twenty minutes of
    // silence. So the headless path reports through [`say`], including the argument errors decided
    // before the flag is parsed, which is why this asks the raw command line.
    let headless = argv.iter().any(|arg| arg == "--render-to");
    let say = |text: &str, error: bool| {
        if headless {
            console_line(text);
        } else {
            message_box("grind-win32", text, error);
        }
    };

    match args::parse(argv) {
        Command::Help => {
            say(args::USAGE, false);
            ExitCode::SUCCESS
        }
        Command::Version => {
            say(&version(), false);
            ExitCode::SUCCESS
        }
        Command::Register => match assoc::register_self() {
            Ok(()) => {
                say(&assoc::registered_message(), false);
                ExitCode::SUCCESS
            }
            Err(message) => {
                say(&message, true);
                ExitCode::FAILURE
            }
        },
        Command::Unregister => match assoc::unregister_self() {
            Ok(()) => {
                say(assoc::UNREGISTERED_MESSAGE, false);
                ExitCode::SUCCESS
            }
            Err(message) => {
                say(&message, true);
                ExitCode::FAILURE
            }
        },
        Command::Error(message) => {
            say(&format!("{message}\n\n{}", args::USAGE), true);
            ExitCode::from(2)
        }
        open => match resolve(open) {
            Err(message) => {
                say(&message, true);
                ExitCode::FAILURE
            }
            // `--render-to` draws one frame with no window and exits, so it comes before a window
            // is opened (decision 5).
            Ok((kind, path, Some((target, dark)))) => {
                match win::render(kind, path, &target, dark) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(message) => {
                        say(&message, true);
                        ExitCode::FAILURE
                    }
                }
            }
            // A window, so somebody is in front of it: offer this executable for the types it
            // opens when this copy is not already offered (`assoc.rs`). A render is a CI step and
            // leaves the registry alone.
            Ok((kind, path, None)) => {
                assoc::offer();
                match win::run(kind, path) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(message) => {
                        say(&message, true);
                        ExitCode::FAILURE
                    }
                }
            }
        },
    }
}

/// Say something on the headless path, where a dialog would deadlock. A GUI-subsystem process
/// inherits its parent's standard handles, so `eprintln!` reaches a redirected pipe — a script or
/// CI step — and goes nowhere otherwise, which is fine: the exit code carries the failure.
#[cfg(windows)]
fn console_line(text: &str) {
    eprintln!("grind-win32: {text}");
}

/// Say something before there is a window: with no console, `eprintln!` goes nowhere when launched
/// from Explorer, so this asks the system for a dialog.
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

/// Off Windows the crate still builds, for its portable half's tests. The binary has nothing to do
/// there, but reports what it *would* have done, so the argument handling runs rather than only
/// tests.
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
        Command::Register | Command::Unregister => {
            eprintln!("grind-win32: file associations are a Windows registry matter");
            ExitCode::FAILURE
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
                eprintln!(
                    "grind-win32 runs on Windows only. Here it would {}.",
                    match (kind, path.as_ref()) {
                        (Some(kind), Some(path)) =>
                            format!("open {} as a {}", path.display(), kind.label()),
                        (Some(kind), None) => {
                            format!("start an empty {}", kind.label())
                        }
                        // No file and no flag: the welcome screen, which is the whole of this
                        // change visible from the one platform that cannot draw it.
                        (None, _) => "show the welcome screen".to_owned(),
                    }
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
            dark: false,
        }
    }

    /// The version block is named with the literal `1`, never `VS_VERSION_INFO`.
    ///
    /// A text test for a binary fact observable only on `windows-latest`: `VS_VERSION_INFO` is a
    /// `#define` in `winver.h`, `grind.rc` includes no headers, and an undefined identifier in a
    /// resource's name position is a **string name** to `rc.exe`, not an error. So the resource
    /// linked as `RT_VERSION`/"VS_VERSION_INFO", while `GetFileVersionInfo`, Explorer and
    /// `win32.yml` ask for `RT_VERSION`/1 and found nothing. Measured by compiling both spellings
    /// and reading the linked PE's resource directory.
    #[test]
    fn the_version_block_is_named_with_the_literal_one() {
        let rc = include_str!("../data/grind.rc");
        let declaration = rc
            .lines()
            .map(str::trim)
            .find(|line| line.ends_with("VERSIONINFO"))
            .expect("grind.rc declares a version block");
        assert_eq!(
            declaration, "1 VERSIONINFO",
            "the version block must be named `1` — `VS_VERSION_INFO` is undefined without \
             winver.h and becomes a string name, which GetFileVersionInfo never looks for"
        );
    }

    /// No file and no flag is **no document**: the welcome screen.
    #[test]
    fn an_empty_invocation_names_no_document() {
        assert_eq!(resolve(open(None, None)).unwrap(), (None, None, None));
    }

    #[test]
    fn the_flag_decides_when_there_is_no_file() {
        assert_eq!(
            resolve(open(None, Some(DocumentKind::Text))).unwrap(),
            (Some(DocumentKind::Text), None, None)
        );
        assert_eq!(
            resolve(open(None, Some(DocumentKind::Spreadsheet))).unwrap(),
            (Some(DocumentKind::Spreadsheet), None, None)
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
            Some(DocumentKind::Text),
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
