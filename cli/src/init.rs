// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind init` — a directory set up for writing `.rhai` and `.grind` in an editor
//! (`doc/init.md`, I1).
//!
//! **What it writes is `doc/editor-setup.md` §3 and §6, executed**, plus the repository files
//! `doc/init.md` §3.7–§3.8 argue for, a starter script, and a README saying what to do first and
//! what the editor will and will not do. The scope line is §3.4's: a file is written only if a
//! person following this repository's own documentation would otherwise have written it by hand
//! — or, for the README, would otherwise have had to discover its contents by being surprised.
//!
//! Three rules, each a test in `cli/tests/init.rs`:
//!
//! * **Never clobber.** A file that exists with other content is *kept*, byte for byte, and the
//!   report says what it would have needed (`doc/init.md` §3.6); `--force` replaces it. A file
//!   that already holds exactly what would be written is *unchanged*, so a second run writes
//!   nothing at all.
//! * **Nothing outside the directory.** No editor is touched — extensions are recommended in
//!   `extensions.json`, never installed (§3.2) — and git is configured only when the directory
//!   is itself a repository's root, in that repository's own `.git/config`.
//! * **No project file.** Nothing written here is read back by `grind` (§3.1): delete any of it
//!   and every command behaves as before.
//!
//! The files are of two kinds (§3.5). **Generated** ones are this binary's answer to "what can
//! a script say" and carry a header naming the version that wrote them; **seeded** ones are a
//! starting point that belongs to the user from the moment they exist. In I1 both are treated
//! alike — written when missing, kept otherwise — and I2's `--update` is where they part.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// The starter: a script that builds a spreadsheet out of the data beside it. It must test,
/// build and lint clean — spelling included — which `cli/tests/init.rs` checks by running it.
const MODEL: &str = include_str!("../templates/generator/model.rhai");
const DATA: &str = include_str!("../templates/generator/data.json");

const EXTENSIONS: &str = include_str!("../templates/vscode/extensions.json");
const SETTINGS: &str = include_str!("../templates/vscode/settings.json");
const TASKS: &str = include_str!("../templates/vscode/tasks.json");

const EDITORCONFIG: &str = include_str!("../templates/repo/editorconfig");
const GITIGNORE: &str = include_str!("../templates/repo/gitignore");
const GITATTRIBUTES: &str = include_str!("../templates/repo/gitattributes");

/// What to do first and what to expect — the things measured about the editor that a person
/// would otherwise find out by being surprised (`doc/editor-setup.md` §8). Named in lower case
/// in the tree so it is not mistaken for this repository's own.
const README: &str = include_str!("../templates/repo/readme.md");

/// What git runs to turn an OpenDocument package into text for a diff (`doc/init.md` §3.8).
const TEXTCONV: &str = "grind project";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The binary's own answer; rewritten by a later `--update` (I2).
    Generated,
    /// A starting point, the user's once written.
    Seeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// It did not exist, and now does.
    Wrote,
    /// It already held exactly this.
    Unchanged,
    /// It exists with other content, and was left alone.
    Kept,
    /// It existed with other content, and `--force` replaced it.
    Replaced,
    /// A setting was made — git's, which is not a file of ours.
    Set,
    /// Not done, for the reason in the note.
    Skipped,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Wrote => "wrote",
            Action::Unchanged => "unchanged",
            Action::Kept => "kept",
            Action::Replaced => "replaced",
            Action::Set => "set",
            Action::Skipped => "skipped",
        }
    }
}

/// One file, or one setting, and what happened to it.
#[derive(Debug, Serialize)]
pub struct Entry {
    /// Relative to the directory, with `/` whatever the platform.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    pub action: Action,
    /// Why, or what a kept file still needs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What `grind init` did, and what is left for the person to do.
#[derive(Debug, Serialize)]
pub struct InitReport {
    pub dir: String,
    pub entries: Vec<Entry>,
    /// Commands to run next, each with what it is for.
    pub next: Vec<(String, String)>,
}

/// One file `init` would write: where, of which kind, what, and — for a seeded file the user
/// already has — what it says a kept copy is missing, if it can tell.
struct Plan {
    path: &'static str,
    kind: Kind,
    content: String,
    missing: Option<fn(&str) -> Option<&'static str>>,
}

/// The whole set, in the order the report lists it.
fn plan(projection_snippets: &str) -> Vec<Plan> {
    // The commit as well as the version: the suite's version number does not move yet, and
    // which build wrote a vocabulary is what a stale one has to be told by (I2).
    let version = format!(
        "{} (commit {})",
        env!("CARGO_PKG_VERSION"),
        grind_core::build_info::COMMIT
    );
    let generated = |what: &str, command: &str, body: String| {
        format!(
            "// Generated by grind {version}: {what}.\n\
             // Do not edit it; after upgrading grind, write it again:\n\
             //   {command}\n{body}"
        )
    };
    let seeded = |path, content: &str, missing| Plan {
        path,
        kind: Kind::Seeded,
        content: content.to_owned(),
        missing,
    };
    vec![
        seeded("model.rhai", MODEL, None),
        seeded("data.json", DATA, None),
        seeded("README.md", README, None),
        seeded(".vscode/extensions.json", EXTENSIONS, None),
        seeded(
            ".vscode/settings.json",
            SETTINGS,
            Some(|text| {
                (!text.contains("*.grind")).then_some(
                    "add \"files.associations\": { \"*.grind\": \"kdl\" } — without it a .grind is plain text",
                )
            }),
        ),
        seeded(".vscode/tasks.json", TASKS, None),
        Plan {
            path: ".vscode/grind.code-snippets",
            kind: Kind::Generated,
            content: generated(
                "every function a script can call, as VS Code snippets",
                "grind definitions --snippets > .vscode/grind.code-snippets",
                ensure_newline(grind_build::snippets()),
            ),
            missing: None,
        },
        Plan {
            path: ".vscode/grind-projection.code-snippets",
            kind: Kind::Generated,
            content: generated(
                "every node of a .grind, as VS Code snippets",
                "grind definitions --projection > .vscode/grind-projection.code-snippets",
                ensure_newline(projection_snippets.to_owned()),
            ),
            missing: None,
        },
        Plan {
            path: "grind.d.rhai",
            kind: Kind::Generated,
            content: generated(
                "every function a script can call, as a Rhai definition file",
                "grind definitions > grind.d.rhai",
                ensure_newline(grind_build::definitions()),
            ),
            missing: None,
        },
        seeded(".editorconfig", EDITORCONFIG, None),
        seeded(".gitignore", GITIGNORE, None),
        seeded(
            ".gitattributes",
            GITATTRIBUTES,
            Some(|text| {
                (!text.contains("diff=grind"))
                    .then_some("add `*.ods diff=grind` and `*.odt diff=grind` for diffs that read")
            }),
        ),
    ]
}

fn ensure_newline(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// Set up `dir` — created if missing — and report what was done to each file.
pub fn init(dir: &Path, force: bool, projection_snippets: &str) -> Result<InitReport, String> {
    if dir.exists() && !dir.is_dir() {
        return Err(format!("{} is a file, not a directory", dir.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let mut entries = Vec::new();
    for file in plan(projection_snippets) {
        entries.push(place(dir, file, force)?);
    }
    let repository = dir.join(".git").is_dir();
    if repository {
        entries.push(textconv(dir));
    }

    let mut next = vec![
        (
            format!("code {}", shown(dir)),
            "open it, install the two extensions it offers, and trust the folder — in \
             Restricted Mode a .grind stays plain text and no task runs (see README.md)"
                .to_owned(),
        ),
        (
            format!("grind test {}", shown(&dir.join("model.rhai"))),
            "run the starter's tests — or Ctrl+Shift+B in VS Code to build it".to_owned(),
        ),
    ];
    if !repository {
        next.push((
            format!("git config diff.grind.textconv \"{TEXTCONV}\""),
            "once per clone, after `git init`: diffs of .ods and .odt that read".to_owned(),
        ));
    }
    Ok(InitReport {
        dir: dir.display().to_string(),
        entries,
        next,
    })
}

/// Write one file unless that would overwrite something different.
fn place(dir: &Path, file: Plan, force: bool) -> Result<Entry, String> {
    let target = dir.join(file.path);
    let existing = match std::fs::read(&target) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", target.display())),
    };
    let entry = |action, note: Option<String>| Entry {
        path: file.path.to_owned(),
        kind: Some(file.kind),
        action,
        note,
    };
    let action = match &existing {
        Some(bytes) if bytes == file.content.as_bytes() => {
            return Ok(entry(Action::Unchanged, None));
        }
        Some(bytes) if !force => {
            let text = String::from_utf8_lossy(bytes);
            let note = match file.missing.and_then(|missing| missing(&text)) {
                Some(advice) => format!("exists and is yours; {advice}"),
                None => "exists and is yours; --force replaces it".to_owned(),
            };
            return Ok(entry(Action::Kept, Some(note)));
        }
        Some(_) => Action::Replaced,
        None => Action::Wrote,
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(&target, &file.content)
        .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
    Ok(entry(action, None))
}

/// Point this repository's `diff=grind` at `grind project` — in `.git/config`, which is not
/// committed, so it is the one thing a clone does not get (`doc/init.md` §3.8). A value someone
/// already set is theirs, `--force` or not.
fn textconv(dir: &Path) -> Entry {
    let entry = |action, note: &str| Entry {
        path: ".git/config".to_owned(),
        kind: None,
        action,
        note: Some(note.to_owned()),
    };
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(dir).args(args).output();
    match git(&["config", "--local", "--get", "diff.grind.textconv"]) {
        Err(_) => entry(Action::Skipped, "git is not on PATH"),
        Ok(out) if out.status.success() => {
            let current = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            match current == TEXTCONV {
                true => entry(Action::Unchanged, "diff.grind.textconv"),
                false => entry(
                    Action::Kept,
                    &format!("diff.grind.textconv is already \"{current}\", and is yours"),
                ),
            }
        }
        Ok(_) => match git(&["config", "--local", "diff.grind.textconv", TEXTCONV]) {
            Ok(out) if out.status.success() => entry(
                Action::Set,
                "diff.grind.textconv = \"grind project\" — .ods and .odt diff as text",
            ),
            _ => entry(Action::Skipped, "git config failed"),
        },
    }
}

/// A path as a person would type it back: `./model.rhai` is `model.rhai`, and one with a
/// space in it is quoted.
fn shown(path: &Path) -> String {
    let path = path.strip_prefix(".").unwrap_or(path);
    let text = match path.as_os_str().is_empty() {
        true => ".".to_owned(),
        false => path.display().to_string(),
    };
    match text.contains(' ') {
        true => format!("\"{text}\""),
        false => text,
    }
}

impl InitReport {
    pub fn print_text(&self) {
        let width = self.entries.iter().map(|e| e.path.len()).max().unwrap_or(0);
        for entry in &self.entries {
            match &entry.note {
                Some(note) => {
                    println!("{:<9} {:<width$}  {note}", entry.action.label(), entry.path)
                }
                None => println!("{:<9} {}", entry.action.label(), entry.path),
            }
        }
        println!("\nnext:");
        for (command, why) in &self.next {
            println!("  {command}\n      {why}");
        }
    }
}
