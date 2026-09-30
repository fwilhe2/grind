// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The command line, decided without a window.
//!
//! Portable on purpose, like every file in this crate that can be (`doc/macos-shell.md`, "The
//! crate"). A flag is a string and a document kind is an enum, so nothing here needs AppKit,
//! and the whole of it is tested on the Linux machine this repository is developed on.
//!
//! The semantics are `ui_win32/src/args.rs`'s, which are `ui_tui/src/main.rs`'s. **A file
//! decides its own type**, `--sheet`/`--text` answer only the empty case, disagreeing with the
//! file is an error rather than an override, and no file and no flag means the welcome window
//! (decision 6).
//!
//! **What is different here is who else reads `argv`.** A Mac application's command line is
//! shared with AppKit:
//!
//! * Any `-Key value` pair lands in `NSUserDefaults`' argument domain. `-AppleLanguages (de)`,
//!   `-NSDocumentRevisionsDebugMode YES` and their like are how a developer, a scheme or a CI
//!   step sets a default for one launch.
//! * Old versions of LaunchServices passed a `-psn_0_…` process serial number.
//!
//! Neither is a file, and treating either as one would make `grind-mac -AppleLanguages (de)`
//! try to open a document called `(de)`. So a single dash followed by a letter is AppKit's, and
//! its value goes with it. `-h` and `-V`, the two short flags this binary has, are matched first.
//!
//! **Files opened from the Finder do not arrive here at all.** A double-click, a drop on the Dock
//! icon and `open book.fods` all reach the application as an Apple event, which
//! `NSDocumentController` turns into a document (decision 5). `argv` is the developer's and the
//! CI runner's way in, and `open -a Grind --args book.fods` is how to reach it through
//! LaunchServices.

use std::path::PathBuf;

use grind_core::DocumentKind;

/// What the process was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Open a window. `path` is `None` for an empty document of `kind`'s type, and `kind` is
    /// `None` when nobody named one, which is the welcome window.
    Open {
        kind: Option<DocumentKind>,
        path: Option<PathBuf>,
        /// `--render-to <file.png>`: draw one frame with no window, write it, exit. Not a user
        /// feature. It is how custom drawing gets an assertable output (decision 9).
        render_to: Option<PathBuf>,
        /// `--dark`: which appearance that frame is drawn in.
        ///
        /// **Only meaningful with `--render-to`**, for the reason `ui_win32` gives. A window
        /// follows the system's appearance, and a render must depend on its command line alone.
        dark: bool,
        /// `--drive <script> --out <dir>`: open a real window, replay the script's synthesized
        /// events, write its snapshots to the directory, print a transcript and exit (decision
        /// 9). Not a user feature either.
        drive: Option<Drive>,
    },
    Help,
    Version,
    /// Something was wrong with the arguments. The string is the whole message.
    Error(String),
}

/// A drive: the script to replay, and where its snapshots go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    pub script: PathBuf,
    pub out: PathBuf,
}

/// What `--help` prints.
pub const USAGE: &str = "usage: grind-mac [--sheet|--text] [file] [--render-to <png> [--dark]]
                 [--drive <script> --out <dir>]

One application, both document types. Which one opens is read out of the file, not
guessed from its name; with no file and no flag the welcome window is shown, where a
spreadsheet, a text document or an existing file can be chosen.

  --sheet          start an empty spreadsheet, skipping the welcome window
  --text           start an empty text document, skipping the welcome window
  --render-to <f>  draw one frame to a PNG and exit, with no window
  --dark           draw that frame in the dark appearance
  --drive <s>      open a window, replay the script's events, and exit
  --out <dir>      where a drive's snapshots are written
  -h, --help       this text
  -V, --version    version and build stamp

AppKit's own launch arguments (-Key value, such as -AppleLanguages '(de)') are
passed through to it and are not files.
";

/// Parse the arguments, after the executable's own name.
pub fn parse(args: impl IntoIterator<Item = String>) -> Command {
    let mut kind: Option<DocumentKind> = None;
    let mut path: Option<PathBuf> = None;
    let mut render_to: Option<PathBuf> = None;
    let mut dark = false;
    let mut script: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
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
            "--dark" => dark = true,
            "--drive" => match args.next() {
                Some(file) => script = Some(PathBuf::from(file)),
                None => return Command::Error("--drive needs a script to replay".into()),
            },
            "--out" => match args.next() {
                Some(dir) => out = Some(PathBuf::from(dir)),
                None => return Command::Error("--out needs a directory".into()),
            },
            other if other.starts_with("--") => {
                return Command::Error(format!("unknown option {other}"));
            }
            // A single `-` is not a flag and not a file: a window cannot be handed a pipe.
            "-" => return Command::Error("this shell cannot read a document from stdin".into()),
            // LaunchServices' process serial number, from before macOS 10.9. Nothing to read.
            other if other.starts_with("-psn_") => {}
            // AppKit's argument domain: `-Key value`, both of them AppKit's. The value is
            // consumed even when it is missing, which AppKit tolerates too.
            other if is_user_default(other) => {
                args.next();
            }
            other => {
                if path.is_some() {
                    return Command::Error(format!("one file at a time, and {other} is a second"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    if dark && render_to.is_none() {
        // Refused rather than ignored: a window follows the system appearance, and a flag that
        // silently did nothing there would read as this shell failing to honour it.
        return Command::Error("--dark only means something with --render-to".into());
    }
    let drive = match (script, out) {
        (Some(script), Some(out)) => Some(Drive { script, out }),
        (Some(_), None) => return Command::Error("--drive needs --out <dir>".into()),
        (None, Some(_)) => return Command::Error("--out only means something with --drive".into()),
        (None, None) => None,
    };
    if drive.is_some() && render_to.is_some() {
        return Command::Error(
            "a render draws without a window and a drive needs one; ask for one".into(),
        );
    }
    Command::Open {
        kind,
        path,
        render_to,
        dark,
        drive,
    }
}

/// Whether `arg` is a key in `NSUserDefaults`' argument domain.
///
/// A single dash followed by a letter. The flags this binary itself has with one dash (`-h`,
/// `-V`) are matched before this is asked, and a path beginning with `-` would be as unusual on
/// a Mac as anywhere else. `./-odd.fods` still reaches it.
fn is_user_default(arg: &str) -> bool {
    let mut chars = arg.chars();
    chars.next() == Some('-') && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
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
                ..
            } => (kind, path, render_to),
            other => panic!("expected an open, got {other:?}"),
        }
    }

    #[test]
    fn no_arguments_names_no_document_at_all() {
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
        let (_, path, render) = open(&["book.fods", "--render-to", "shot.png"]);
        assert_eq!(path, Some(PathBuf::from("book.fods")));
        assert_eq!(render, Some(PathBuf::from("shot.png")));
    }

    #[test]
    fn render_to_without_a_target_is_an_error() {
        assert!(matches!(
            parse_str(&["--render-to"]),
            Command::Error(message) if message.contains("needs a file")
        ));
    }

    #[test]
    fn dark_belongs_to_a_render_and_nowhere_else() {
        let dark = |args: &[&str]| match parse_str(args) {
            Command::Open { dark, .. } => dark,
            other => panic!("expected an open, got {other:?}"),
        };
        assert!(!dark(&["book.fods", "--render-to", "shot.png"]));
        assert!(dark(&["--dark", "--render-to", "shot.png"]), "either order");
        assert!(matches!(
            parse_str(&["book.fods", "--dark"]),
            Command::Error(message) if message.contains("--render-to")
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

    #[test]
    fn two_files_is_an_error() {
        assert!(matches!(
            parse_str(&["one.fods", "two.fodt"]),
            Command::Error(message) if message.contains("second")
        ));
    }

    /// The one rule this file has that the Windows one does not. A default for one launch is
    /// AppKit's, value and all, so neither half of the pair is a document.
    #[test]
    fn appkit_launch_arguments_are_not_files() {
        assert_eq!(
            open(&["-AppleLanguages", "(de)", "book.fods"]),
            (None, Some(PathBuf::from("book.fods")), None)
        );
        assert_eq!(
            open(&["book.fods", "-NSDocumentRevisionsDebugMode", "YES"]),
            (None, Some(PathBuf::from("book.fods")), None)
        );
        assert_eq!(
            open(&["-AppleLanguages"]),
            (None, None, None),
            "a key with no value is AppKit's to shrug at, not a file"
        );
    }

    #[test]
    fn a_process_serial_number_is_ignored() {
        assert_eq!(
            open(&["-psn_0_1234567", "book.fods"]),
            (None, Some(PathBuf::from("book.fods")), None)
        );
    }

    /// The two short flags this binary does have are not mistaken for AppKit's.
    #[test]
    fn our_own_short_flags_are_not_user_defaults() {
        assert_eq!(
            parse_str(&["-V", "-AppleLanguages", "(de)"]),
            Command::Version
        );
    }

    #[test]
    fn a_path_with_spaces_or_a_leading_dot_is_a_path() {
        assert_eq!(
            open(&["/Users/florian/My Book.fods"]).1,
            Some(PathBuf::from("/Users/florian/My Book.fods"))
        );
        assert_eq!(open(&["./-odd.fods"]).1, Some(PathBuf::from("./-odd.fods")));
    }

    #[test]
    fn a_drive_names_its_script_and_where_its_snapshots_go() {
        let command = parse_str(&["book.fods", "--drive", "bold.drive", "--out", "shots"]);
        let Command::Open { drive, .. } = command else {
            panic!("an open")
        };
        assert_eq!(
            drive,
            Some(Drive {
                script: PathBuf::from("bold.drive"),
                out: PathBuf::from("shots"),
            })
        );
        assert!(matches!(
            parse_str(&["--drive", "bold.drive"]),
            Command::Error(message) if message.contains("--out")
        ));
        assert!(matches!(
            parse_str(&["--out", "shots"]),
            Command::Error(message) if message.contains("--drive")
        ));
        assert!(matches!(
            parse_str(&["--drive", "a", "--out", "b", "--render-to", "c.png"]),
            Command::Error(_)
        ));
    }
}
