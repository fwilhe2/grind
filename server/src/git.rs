// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! History as git commits, by running the `git` the person has installed (`doc/server.md` §3,
//! Tier 2). Present only when the store's directory is inside a work tree.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

pub struct Git {
    dir: PathBuf,
    /// `-c user.name=…` and `-c user.email=…` for what the repository does not set itself.
    identity: Vec<String>,
}

/// One commit that touched a document.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Version {
    pub rev: String,
    pub author: String,
    /// ISO 8601, as git writes it.
    pub date: String,
    pub subject: String,
    /// The document's path at that commit, relative to the top of the work tree.
    #[serde(skip)]
    pub path: String,
}

impl Git {
    /// `dir`'s work tree, or `None` when it has none or git is not installed.
    pub fn find(dir: &Path) -> Option<Git> {
        let mut git = Git {
            dir: dir.to_owned(),
            identity: Vec::new(),
        };
        let out = git.run(&["rev-parse", "--is-inside-work-tree"]).ok()?;
        if String::from_utf8_lossy(&out).trim() != "true" {
            return None;
        }
        // A repository without an identity still gets its history, under this name.
        for (key, fallback) in [
            ("user.name", "grind serve"),
            ("user.email", "grind@localhost"),
        ] {
            if git.run(&["config", "--get", key]).is_err() {
                git.identity.push("-c".to_owned());
                git.identity.push(format!("{key}={fallback}"));
            }
        }
        Some(git)
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(&self.identity)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("git: {e}"))?;
        match out.status.success() {
            true => Ok(out.stdout),
            false => Err(format!(
                "git {}: {}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
        }
    }

    /// Commit `paths` (relative to the store) as they are on disk now, and nothing else that
    /// happens to be staged. Nothing changed is no commit.
    pub fn commit(&self, paths: &[&str], message: &str) -> Result<(), String> {
        let mut add = vec!["add", "-A", "--"];
        add.extend(paths);
        self.run(&add)?;
        let mut staged = vec!["diff", "--cached", "--name-only", "--"];
        staged.extend(paths);
        if self.run(&staged)?.is_empty() {
            return Ok(());
        }
        let mut commit = vec!["commit", "-q", "-m", message, "--"];
        commit.extend(paths);
        self.run(&commit).map(|_| ())
    }

    /// Every commit that touched `path`, newest first, following renames.
    pub fn log(&self, path: &str) -> Result<Vec<Version>, String> {
        let out = self.run(&[
            "log",
            "--follow",
            "--name-only",
            "--format=%x1e%H%x1f%an%x1f%aI%x1f%s",
            "--",
            path,
        ])?;
        let text = String::from_utf8_lossy(&out);
        Ok(text
            .split('\u{1e}')
            .filter_map(|record| {
                let (head, names) = record.split_once('\n')?;
                let mut fields = head.split('\u{1f}');
                Some(Version {
                    rev: fields.next()?.to_owned(),
                    author: fields.next()?.to_owned(),
                    date: fields.next()?.to_owned(),
                    subject: fields.next()?.to_owned(),
                    path: names.lines().find(|l| !l.is_empty())?.to_owned(),
                })
            })
            .collect())
    }

    /// The bytes of `path` (relative to the top of the work tree) at `rev`.
    pub fn show(&self, rev: &str, path: &str) -> Result<Vec<u8>, String> {
        self.run(&["show", &format!("{rev}:{path}")])
    }
}

/// A full commit id: forty hex digits, so nothing that reaches `git` could be an option.
pub fn is_rev(rev: &str) -> bool {
    rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit())
}
