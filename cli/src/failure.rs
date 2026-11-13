// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Why a command did not do what it was asked, in a form a script can branch on: a kind, which
//! picks the exit status, and the sentence a person reads.

use serde::Serialize;

use crate::report::Format;

/// The exit statuses `grind --help` documents, besides 0.
pub(crate) const REFUSED: u8 = 1;
/// clap's own status for a command line it cannot parse.
pub(crate) const USAGE: u8 = 2;
pub(crate) const UNREADABLE: u8 = 3;
/// `lint` found an error, a `grind test` failed, or a `--strict` import would lose something.
pub(crate) const CHECK_FAILED: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    /// The input was read, and what it asks for cannot be done: a formula that does not parse,
    /// an address with no such sheet, a value out of range.
    Refused,
    /// A file named on the command line could not be read, or is not a document this command
    /// opens.
    Unreadable,
    /// A script stopped with an error, at [`Failure::at`] when Rhai knew where.
    Script,
    /// The command line itself: an unknown command or flag, a missing argument.
    Usage,
}

#[derive(Debug, Serialize)]
pub(crate) struct Failure {
    pub kind: Kind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<At>,
}

/// Where in a script it stopped; 1-based.
#[derive(Debug, Serialize)]
pub(crate) struct At {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

impl Failure {
    pub(crate) fn unreadable(message: impl Into<String>) -> Self {
        Failure {
            kind: Kind::Unreadable,
            message: message.into(),
            at: None,
        }
    }

    pub(crate) fn exit_code(&self) -> u8 {
        match self.kind {
            Kind::Refused | Kind::Script => REFUSED,
            Kind::Unreadable => UNREADABLE,
            Kind::Usage => USAGE,
        }
    }

    /// Standard error either way: as `grind: …` for a person, or as one JSON object, which is
    /// what `--format json` promises a script.
    pub(crate) fn print(&self, format: Format) {
        match format {
            Format::Text => eprintln!("grind: {self}"),
            Format::Json => {
                #[derive(Serialize)]
                struct Envelope<'a> {
                    error: &'a Failure,
                    exit: u8,
                }
                let envelope = Envelope {
                    error: self,
                    exit: self.exit_code(),
                };
                eprintln!(
                    "{}",
                    serde_json::to_string(&envelope).expect("a failure is serializable")
                );
            }
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.at {
            Some(At {
                file,
                line: Some(line),
                column: Some(column),
            }) => write!(f, "{file}:{line}:{column}: {}", self.message),
            Some(At {
                file,
                line: Some(line),
                column: None,
            }) => write!(f, "{file}:{line}: {}", self.message),
            Some(At { file, .. }) => write!(f, "{file}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Most failures are refusals, and most of the code says why in a sentence.
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure {
            kind: Kind::Refused,
            message,
            at: None,
        }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Failure::from(message.to_owned())
    }
}

/// For a helper that reports in sentences and calls one that knows its kind.
impl From<Failure> for String {
    fn from(failure: Failure) -> Self {
        failure.to_string()
    }
}

impl From<grind_build::Error> for Failure {
    fn from(e: grind_build::Error) -> Self {
        Failure {
            kind: Kind::Script,
            message: e.message,
            at: Some(At {
                file: e.script,
                line: e.line,
                column: e.column,
            }),
        }
    }
}

/// A command line clap refused, as a [`Failure`] — for `--format json`, which clap's own
/// message would break. Help and `--version` are not failures and are not passed here.
pub(crate) fn usage(error: &clap::Error) -> Failure {
    // The message is the paragraph before the usage line: one line, or a heading and what it
    // lists (`the following required arguments were not provided: <ADDRESS>`).
    let rendered = error.render().to_string();
    let message: Vec<&str> = rendered
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .map(str::trim)
        .collect();
    let message = message.join(" ");
    Failure {
        kind: Kind::Usage,
        message: message.trim_prefix("error: ").to_owned(),
        at: None,
    }
}

/// Whether the command line asked for JSON, read before clap has parsed it.
pub(crate) fn wants_json(args: &[std::ffi::OsString]) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == "--format" && pair[1] == "json")
        || args.iter().any(|a| a == "--format=json")
}
