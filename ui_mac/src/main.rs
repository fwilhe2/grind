// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind-mac` — the macOS shell over the suite, shipped as `Grind.app`.
//!
//! **M0: wiring, and nothing that opens a window yet.** `doc/macos-shell.md` is normative for
//! this crate and is where the decisions, the milestones and the named gaps live. It is written
//! so that as much as possible of this shell can be built and tested on Linux, because the
//! machine this repository is developed on is not a Mac:
//!
//! * Everything that can be a pure function is one, and compiles and tests on any host. That is
//!   the argument parsing here, and from M2 the menus, the selector tables, the edit modes and
//!   the Info.plist.
//! * Everything that needs AppKit is behind `cfg(target_os = "macos")`. The Linux machine
//!   type-checks and lints it with `--target aarch64-apple-darwin` (nothing links, so no SDK is
//!   needed), and `artifacts.yml`'s `macos` job is the one place it links and runs.
//!
//! This file owns the process. Today it resolves the command line into what a window *would*
//! open and says so, which makes the argument handling runnable rather than only testable. The
//! same thing `ui_win32`'s W0 did.

mod args;
mod import;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::Command;
use grind_core::DocumentKind;

fn version() -> String {
    format!(
        "grind-mac {}",
        grind_core::build_info::describe_version(env!("CARGO_PKG_VERSION"))
    )
}

/// What kind of document a file holds, read from its bytes. Decided *before* parsing, because
/// the reader is tolerant by construction and would hand back an empty document rather than an
/// error if it were handed the wrong type.
fn sniff(path: &Path) -> Result<DocumentKind, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    // A workbook or a CSV is a spreadsheet this shell imports; `grind_core::kind` answers only
    // which *ODF* type some bytes are, and should.
    if import::is_workbook(&bytes) || import::is_delimited(path, &bytes) {
        return Ok(DocumentKind::Spreadsheet);
    }
    grind_core::kind(&bytes).ok_or_else(|| {
        format!(
            "{}: not an ODF spreadsheet or text document",
            path.display()
        )
    })
}

/// Which document, from where, and — when this invocation is a render — where the frame goes
/// and in which appearance. The kind is an `Option` because **"no document" is an answer**: the
/// welcome window (decision 6).
type Opening = (
    Option<DocumentKind>,
    Option<PathBuf>,
    Option<(PathBuf, bool)>,
);

/// Resolve the command line into what a window would open, or a message saying why it cannot.
/// The whole of the decision and none of the platform, so it is tested on any host.
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
        Some(file) => {
            let found = sniff(file)?;
            let kind = grind_core::kind::reconcile(kind, found, &file.display().to_string())?;
            Ok((Some(kind), path, render))
        }
        None => Ok((kind, None, render)),
    }
}

/// What a window would do with `opening`, in a sentence.
fn intent(opening: &Opening) -> String {
    let (kind, path, render) = opening;
    let what = match (kind, path) {
        (Some(kind), Some(path)) => {
            format!("open {} as a {}", path.display(), kind.label())
        }
        (Some(kind), None) => format!("start an empty {}", kind.label()),
        (None, _) => "show the welcome window".to_owned(),
    };
    match render {
        Some((target, dark)) => format!(
            "{what}, and draw it to {} in the {} appearance",
            target.display(),
            if *dark { "dark" } else { "light" }
        ),
        None => what,
    }
}

fn main() -> ExitCode {
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
            eprintln!("grind-mac: {message}\n\n{}", args::USAGE);
            ExitCode::from(2)
        }
        open => match resolve(open) {
            Err(message) => {
                eprintln!("grind-mac: {message}");
                ExitCode::FAILURE
            }
            Ok(opening) => {
                // Not an error in the arguments, but not a window either — exit non-zero so no
                // script mistakes M0 for a shell that did what it was asked.
                eprintln!(
                    "grind-mac has no window yet (doc/macos-shell.md, M0). It would {}.",
                    intent(&opening)
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
            path: path.map(PathBuf::from),
            render_to: None,
            dark: false,
        }
    }

    #[test]
    fn an_empty_invocation_names_no_document() {
        assert_eq!(resolve(open(None, None)).unwrap(), (None, None, None));
        assert_eq!(intent(&(None, None, None)), "show the welcome window");
    }

    #[test]
    fn the_flag_decides_when_there_is_no_file() {
        assert_eq!(
            resolve(open(None, Some(DocumentKind::Text))).unwrap(),
            (Some(DocumentKind::Text), None, None)
        );
    }

    /// The type comes out of the bytes, not the name — so a text document called `.fods` still
    /// opens as a text document.
    #[test]
    fn the_bytes_decide_and_not_the_extension() {
        let dir = std::env::temp_dir().join(format!("grind-mac-args-{}", std::process::id()));
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
        let error = resolve(open(Some("no-such-file-here.fods"), None)).unwrap_err();
        assert!(error.contains("no-such-file-here.fods"), "{error}");
    }

    #[test]
    fn a_render_says_where_and_in_which_appearance() {
        let opening = (
            Some(DocumentKind::Spreadsheet),
            Some(PathBuf::from("book.fods")),
            Some((PathBuf::from("frame.png"), true)),
        );
        assert_eq!(
            intent(&opening),
            "open book.fods as a spreadsheet, and draw it to frame.png in the dark appearance"
        );
    }
}
