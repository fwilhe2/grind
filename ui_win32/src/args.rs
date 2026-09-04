// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The command line, decided without a window.
//!
//! Windows-free on purpose, like every other file in this crate that can be — see
//! `doc/windows-shell.md`'s crate table. A flag is a string and a document kind is an enum, so
//! nothing here needs the `windows` crate, which is what lets the whole of it be tested on the
//! Linux machine this repository is developed on.
//!
//! The semantics are `ui_tui/src/main.rs`'s, deliberately and to the letter: **a file decides
//! its own type**, `--sheet` / `--text` answer only the empty case, and asking for one when the
//! file is the other is an error rather than a silent override. Opening a spreadsheet as a
//! document would show an empty one, which is exactly the confusion `grind_core::kind` exists to
//! prevent.

use std::path::PathBuf;

use grind_core::DocumentKind;

/// What the process was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Open a window. `path` is `None` for an empty document of `kind`'s type.
    Open {
        kind: Option<DocumentKind>,
        path: Option<PathBuf>,
        /// `--render-to <file>`: draw one frame, write it, exit. Not a user feature — it is how
        /// custom drawing gets an assertable output (`doc/windows-shell.md`, decision 5).
        render_to: Option<PathBuf>,
    },
    Help,
    Version,
    /// Something was wrong with the arguments. The string is the whole message.
    Error(String),
}

/// What `--help` prints, and what the message box shows when there is no console.
pub const USAGE: &str = "usage: grind-win32 [--sheet|--text] [file] [--render-to <bmp>]

One window, both document types. Which one opens is read out of the file, not guessed
from its name; with no file, --sheet (the default) or --text says which to start empty.

  --sheet          start an empty spreadsheet
  --text           start an empty text document
  --render-to <f>  draw one frame to a BMP and exit
  -h, --help       this text
  -V, --version    version and build stamp
";

/// Parse the arguments, after the executable's own name.
pub fn parse(args: impl IntoIterator<Item = String>) -> Command {
    let mut kind: Option<DocumentKind> = None;
    let mut path: Option<PathBuf> = None;
    let mut render_to: Option<PathBuf> = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Command::Help,
            "-V" | "--version" => return Command::Version,
            "--sheet" => kind = Some(DocumentKind::Spreadsheet),
            "--text" => kind = Some(DocumentKind::Text),
            "--render-to" => match args.next() {
                Some(target) => render_to = Some(PathBuf::from(target)),
                None => return Command::Error("--render-to needs a file to write".into()),
            },
            other if other.starts_with("--") => {
                return Command::Error(format!("unknown option {other}"));
            }
            // A single `-` is not a flag and not a file: this shell has no stdin to read a
            // document from, unlike the CLI, because a window cannot be handed a pipe.
            "-" => return Command::Error("this shell cannot read a document from stdin".into()),
            other => {
                if path.is_some() {
                    return Command::Error(format!("one file at a time, and {other} is a second"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    Command::Open {
        kind,
        path,
        render_to,
    }
}

/// Reconcile what the user asked for with what the file turned out to be.
///
/// Separated from the read so that the *rule* is testable without a filesystem — the read
/// itself is three lines in `main.rs` and has nothing to decide.
pub fn reconcile(
    asked: Option<DocumentKind>,
    found: DocumentKind,
    shown: &str,
) -> Result<DocumentKind, String> {
    match asked {
        Some(asked) if asked != found => Err(format!(
            "{shown} is a {}, not a {}",
            describe(found),
            describe(asked)
        )),
        _ => Ok(found),
    }
}

/// What a document type is called in a sentence.
pub fn describe(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Spreadsheet => "spreadsheet",
        DocumentKind::Text => "text document",
        _ => "document",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(args: &[&str]) -> Command {
        parse(args.iter().map(|a| a.to_string()))
    }

    fn open(args: &[&str]) -> (Option<DocumentKind>, Option<PathBuf>, Option<PathBuf>) {
        match parse_str(args) {
            Command::Open {
                kind,
                path,
                render_to,
            } => (kind, path, render_to),
            other => panic!("expected an open, got {other:?}"),
        }
    }

    #[test]
    fn no_arguments_opens_an_empty_spreadsheet() {
        // `None` rather than `Spreadsheet`: the default is applied after the file is
        // consulted, so that a file can still decide for itself.
        assert_eq!(open(&[]), (None, None, None));
    }

    #[test]
    fn a_file_is_taken_as_a_path() {
        assert_eq!(
            open(&["book.fods"]),
            (None, Some(PathBuf::from("book.fods")), None)
        );
    }

    #[test]
    fn the_type_flags_are_recorded_but_not_resolved_here() {
        assert_eq!(open(&["--text"]), (Some(DocumentKind::Text), None, None));
        assert_eq!(
            open(&["--sheet"]),
            (Some(DocumentKind::Spreadsheet), None, None)
        );
    }

    #[test]
    fn render_to_takes_the_next_argument() {
        let (_, path, render) = open(&["book.fods", "--render-to", "shot.bmp"]);
        assert_eq!(path, Some(PathBuf::from("book.fods")));
        assert_eq!(render, Some(PathBuf::from("shot.bmp")));
    }

    #[test]
    fn render_to_without_a_target_is_an_error() {
        assert!(matches!(
            parse_str(&["--render-to"]),
            Command::Error(message) if message.contains("needs a file")
        ));
    }

    #[test]
    fn help_and_version_win_wherever_they_appear() {
        assert_eq!(parse_str(&["book.fods", "--help"]), Command::Help);
        assert_eq!(parse_str(&["-h"]), Command::Help);
        assert_eq!(parse_str(&["--text", "-V"]), Command::Version);
    }

    #[test]
    fn an_unknown_option_is_refused_rather_than_ignored() {
        assert!(matches!(
            parse_str(&["--fluent"]),
            Command::Error(message) if message.contains("--fluent")
        ));
    }

    /// A window opens one document. Silently dropping the second would be the kind of
    /// almost-worked that costs somebody an afternoon.
    #[test]
    fn two_files_is_an_error() {
        assert!(matches!(
            parse_str(&["one.fods", "two.fodt"]),
            Command::Error(message) if message.contains("second")
        ));
    }

    /// Windows passes `"C:\Users\...\My Book.fods"` as one argument, already unquoted, and a
    /// path beginning with a drive letter must not look like a flag.
    #[test]
    fn a_windows_path_is_a_path() {
        let (_, path, _) = open(&["C:\\Users\\florian\\My Book.fods"]);
        assert_eq!(
            path,
            Some(PathBuf::from("C:\\Users\\florian\\My Book.fods"))
        );
    }

    #[test]
    fn a_file_that_agrees_with_the_flag_is_fine() {
        let found = DocumentKind::Text;
        assert_eq!(
            reconcile(Some(DocumentKind::Text), found, "a.fodt"),
            Ok(found)
        );
        assert_eq!(reconcile(None, found, "a.fodt"), Ok(found));
    }

    /// The rule `ui_tui` already follows: a file decides, and disagreeing is an error rather
    /// than an override, because opening a spreadsheet as a document shows an empty one.
    #[test]
    fn a_file_that_contradicts_the_flag_is_an_error() {
        let error = reconcile(
            Some(DocumentKind::Text),
            DocumentKind::Spreadsheet,
            "book.fods",
        )
        .unwrap_err();
        assert!(error.contains("book.fods"), "{error}");
        assert!(error.contains("spreadsheet"), "{error}");
        assert!(error.contains("text document"), "{error}");
    }
}
