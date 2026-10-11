// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **`grind init`, held to `doc/init.md` I1's exit criterion**: the starter builds, tests and
//! lints clean; a second run writes nothing; a file the user already had survives byte for
//! byte; and nothing is written outside the directory named.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A temporary directory that removes itself — `cli.rs`'s, for the same reason.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "grind-init-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("sandbox");
        Sandbox(dir)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn grind_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grind"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("grind runs")
}

fn ok_in(dir: &Path, args: &[&str]) -> String {
    let out = grind_in(dir, args);
    assert!(
        out.status.success(),
        "grind {args:?} failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

/// `grind --format json init <dir>`, as path → action.
fn init(parent: &Path, dir: &str, extra: &[&str]) -> BTreeMap<String, String> {
    let mut args = vec!["--format", "json", "init", dir];
    args.extend(extra);
    let report: serde_json::Value =
        serde_json::from_str(&ok_in(parent, &args)).expect("the report is JSON");
    report["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| {
            (
                e["path"].as_str().expect("path").to_owned(),
                e["action"].as_str().expect("action").to_owned(),
            )
        })
        .collect()
}

/// Every file under `dir`, relative, with its bytes — `.git` left out, since git writes its
/// own bookkeeping there whenever it is asked anything.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("readable") {
            let path = entry.expect("entry").path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let bytes = std::fs::read(&path).expect("readable");
                into.insert(path.strip_prefix(root).expect("inside").to_owned(), bytes);
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(dir, dir, &mut files);
    files
}

/// JSON with `//` comment lines at the top — what a generated `.code-snippets` file is, and
/// what VS Code's JSONC reader takes.
fn jsonc(text: &str) -> serde_json::Value {
    let body: String = text
        .lines()
        .skip_while(|line| line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&body).expect("JSON once the header comments are skipped")
}

#[test]
fn a_new_directory_gets_every_file_and_nothing_outside_it() {
    let sandbox = Sandbox::new("fresh");
    let report = init(&sandbox.0, "pricing", &[]);
    let dir = sandbox.0.join("pricing");

    for path in [
        "model.rhai",
        "data.json",
        "README.md",
        ".vscode/extensions.json",
        ".vscode/settings.json",
        ".vscode/tasks.json",
        ".vscode/grind.code-snippets",
        ".vscode/grind-projection.code-snippets",
        "grind.d.rhai",
        ".editorconfig",
        ".gitignore",
        ".gitattributes",
    ] {
        assert_eq!(
            report.get(path).map(String::as_str),
            Some("wrote"),
            "{path}"
        );
        assert!(dir.join(path).is_file(), "{path} is there");
    }
    assert!(
        !report.contains_key(".git/config"),
        "not a repository, so git is left alone"
    );

    // Nothing beside the directory it was told to set up.
    let beside: Vec<_> = std::fs::read_dir(&sandbox.0)
        .expect("readable")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(beside, ["pricing"]);

    // Every JSON file an editor reads is JSON.
    for path in [
        ".vscode/extensions.json",
        ".vscode/settings.json",
        ".vscode/tasks.json",
    ] {
        let text = std::fs::read_to_string(dir.join(path)).expect("readable");
        serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    let settings = std::fs::read_to_string(dir.join(".vscode/settings.json")).expect("there");
    assert!(
        settings.contains("\"*.grind\": \"kdl\""),
        "the load-bearing line"
    );
}

/// The README written beside the starter names tasks by their labels and files by their
/// names; both must be what `init` actually wrote, or the first thing a person reads is wrong.
#[test]
fn the_readme_names_what_was_written() {
    let sandbox = Sandbox::new("readme");
    init(&sandbox.0, ".", &[]);
    let readme = std::fs::read_to_string(sandbox.0.join("README.md")).expect("written");
    let tasks: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(sandbox.0.join(".vscode/tasks.json")).expect("written"),
    )
    .expect("JSON");
    let labels: Vec<&str> = tasks["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .map(|t| t["label"].as_str().expect("label"))
        .collect();
    let mut named = 0;
    for part in readme.split("*grind: ").skip(1) {
        let label = format!("grind: {}", part.split('*').next().expect("closed"));
        assert!(
            labels.contains(&label.as_str()),
            "README names {label:?}, tasks are {labels:?}"
        );
        named += 1;
    }
    assert!(named >= 3, "the README's task names were found");
    for file in [
        "model.rhai",
        "data.json",
        ".vscode/settings.json",
        ".vscode/grind.code-snippets",
        ".vscode/grind-projection.code-snippets",
        "grind.d.rhai",
        ".gitattributes",
        ".gitignore",
    ] {
        assert!(readme.contains(file), "README mentions {file}");
        assert!(sandbox.0.join(file).is_file(), "and {file} exists");
    }
    // The two things measured to surprise people (doc/editor-setup.md §8).
    assert!(readme.contains("Restricted"), "the trust hint");
    assert!(readme.contains("Ctrl+Space"), "where a description is");
}

#[test]
fn the_starter_tests_builds_and_lints_clean() {
    let sandbox = Sandbox::new("starter");
    init(&sandbox.0, ".", &[]);
    let dir = &sandbox.0;

    let tests = ok_in(dir, &["test", "model.rhai"]);
    assert!(tests.contains("0 failed"), "{tests}");
    assert!(!tests.contains("0 passed"), "it has tests to run:\n{tests}");

    ok_in(dir, &["build", "model.rhai", "-o", "model.fods"]);
    // Spelling included: a starter that greets a newcomer with warnings teaches them to ignore
    // the linter.
    let lint = ok_in(dir, &["lint", "model.fods"]);
    assert!(lint.contains("no problems found"), "{lint}");

    // What it built is what .gitignore says is derived.
    let ignored = std::fs::read_to_string(dir.join(".gitignore")).expect("there");
    assert!(ignored.lines().any(|l| l == "/model.fods"), "{ignored}");
}

#[test]
fn the_vocabulary_is_this_binarys() {
    let sandbox = Sandbox::new("vocabulary");
    init(&sandbox.0, ".", &[]);
    let dir = &sandbox.0;
    let read = |path: &str| std::fs::read_to_string(dir.join(path)).expect("there");

    // Each generated file is the command its header names, under the header.
    for (path, args) in [
        (
            ".vscode/grind.code-snippets",
            &["definitions", "--snippets"][..],
        ),
        (
            ".vscode/grind-projection.code-snippets",
            &["definitions", "--projection"][..],
        ),
        ("grind.d.rhai", &["definitions"][..]),
    ] {
        let written = read(path);
        assert!(written.starts_with("// Generated by grind "), "{path}");
        assert!(
            written.contains(&format!("grind {}", args.join(" "))),
            "{path}"
        );
        let body: String = written
            .lines()
            .skip_while(|l| l.starts_with("//"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(body, ok_in(dir, args), "{path} is what the command prints");
    }
    assert!(jsonc(&read(".vscode/grind.code-snippets")).is_object());
    assert!(jsonc(&read(".vscode/grind-projection.code-snippets")).is_object());
}

#[test]
fn a_second_run_writes_nothing() {
    let sandbox = Sandbox::new("twice");
    init(&sandbox.0, ".", &[]);
    let before = snapshot(&sandbox.0);
    let again = init(&sandbox.0, ".", &[]);
    assert!(
        again.values().all(|action| action == "unchanged"),
        "{again:?}"
    );
    assert_eq!(snapshot(&sandbox.0), before);
}

#[test]
fn a_file_the_user_had_survives_byte_for_byte() {
    let sandbox = Sandbox::new("theirs");
    let dir = &sandbox.0;
    let theirs = "{\n    // tabs, please\n    \"editor.insertSpaces\": false\n}\n";
    std::fs::create_dir_all(dir.join(".vscode")).expect("made");
    std::fs::write(dir.join(".vscode/settings.json"), theirs).expect("written");

    let out = ok_in(dir, &["init"]);
    assert_eq!(
        std::fs::read_to_string(dir.join(".vscode/settings.json")).expect("there"),
        theirs
    );
    // And the report says what their copy is missing, rather than leaving a .grind silently
    // unhighlighted.
    let line = out
        .lines()
        .find(|l| l.contains(".vscode/settings.json"))
        .expect("reported");
    assert!(line.starts_with("kept"), "{line}");
    assert!(line.contains("files.associations"), "{line}");

    // --force is the one way past it.
    let forced = init(dir, ".", &["--force", "--template", "generator"]);
    assert_eq!(forced[".vscode/settings.json"], "replaced");
    assert_eq!(
        forced["model.rhai"], "unchanged",
        "an identical file is not replaced"
    );
}

#[test]
fn in_a_repository_git_learns_to_diff_a_package() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: no git on PATH");
        return;
    }
    let sandbox = Sandbox::new("git");
    let dir = &sandbox.0;
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git runs")
    };
    assert!(git(&["init", "-q"]).status.success());

    assert_eq!(init(dir, ".", &[])[".git/config"], "set");
    let value = git(&["config", "--get", "diff.grind.textconv"]);
    assert_eq!(
        String::from_utf8_lossy(&value.stdout).trim(),
        "grind project"
    );
    assert_eq!(init(dir, ".", &[])[".git/config"], "unchanged");

    // A value somebody chose is theirs, --force or not.
    assert!(
        git(&["config", "diff.grind.textconv", "my-own"])
            .status
            .success()
    );
    assert_eq!(init(dir, ".", &["--force"])[".git/config"], "kept");
    let value = git(&["config", "--get", "diff.grind.textconv"]);
    assert_eq!(String::from_utf8_lossy(&value.stdout).trim(), "my-own");
}

#[test]
fn a_file_where_the_directory_should_be_is_an_error() {
    let sandbox = Sandbox::new("file");
    std::fs::write(sandbox.0.join("taken"), "x").expect("written");
    let out = grind_in(&sandbox.0, &["init", "taken"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a directory"));
}

/// Every starter is what its README says, and lints clean — spelling included.
#[test]
fn every_starter_lints_clean() {
    for (template, starter) in [("sheet", "book.grind"), ("text", "notes.grind")] {
        let sandbox = Sandbox::new(template);
        let report = init(&sandbox.0, ".", &["--template", template]);
        assert_eq!(report[starter], "wrote", "{template}");
        assert!(
            !report.contains_key("model.rhai"),
            "only the starter asked for"
        );
        assert!(
            !report.contains_key(".gitignore"),
            "a projection is source: nothing derived"
        );
        let lint = ok_in(&sandbox.0, &["lint", starter]);
        assert!(lint.contains("no problems found"), "{template}: {lint}");
        let readme = std::fs::read_to_string(sandbox.0.join("README.md")).expect("written");
        assert!(
            readme.contains(starter),
            "{template}'s README names its starter"
        );
    }

    // The hand-written spreadsheet carries no answers until `recalc` writes them, one line each.
    let sandbox = Sandbox::new("sheet-recalc");
    init(&sandbox.0, ".", &["--template", "sheet"]);
    let before = std::fs::read_to_string(sandbox.0.join("book.grind")).expect("there");
    ok_in(&sandbox.0, &["sheet", "recalc", "book.grind"]);
    let after = std::fs::read_to_string(sandbox.0.join("book.grind")).expect("there");
    assert_eq!(
        before.lines().count(),
        after.lines().count(),
        "answers are written in place"
    );
    assert!(ok_in(&sandbox.0, &["sheet", "view", "book.grind"]).contains("20165.88"));

    let sandbox = Sandbox::new("none");
    let report = init(&sandbox.0, ".", &["--template", "none"]);
    assert_eq!(report["README.md"], "wrote");
    assert!(
        report
            .keys()
            .all(|p| !p.ends_with(".grind") && !p.ends_with(".rhai") || p == "grind.d.rhai")
    );
}

/// Running it again names no starter, so it must not add one beside the first.
#[test]
fn a_second_run_adds_no_second_starter() {
    let sandbox = Sandbox::new("one-starter");
    init(&sandbox.0, ".", &["--template", "sheet"]);
    let before = snapshot(&sandbox.0);
    let again = init(&sandbox.0, ".", &[]);
    assert!(
        again.values().all(|action| action == "unchanged"),
        "{again:?}"
    );
    assert!(!again.contains_key("model.rhai"));
    assert_eq!(snapshot(&sandbox.0), before);
}

/// **I2's exit criterion**: a vocabulary from another grind is reported stale, `--update`
/// rewrites it, and nothing seeded is touched — not even a file the user changed.
#[test]
fn update_rewrites_stale_vocabulary_and_nothing_else() {
    let sandbox = Sandbox::new("update");
    let dir = &sandbox.0;
    init(dir, ".", &[]);
    let current = std::fs::read_to_string(dir.join("grind.d.rhai")).expect("there");

    // An older grind registered one function fewer; the user has since edited a seeded file.
    let older = current.replacen("fn push(", "fn push_was_different(", 1);
    assert_ne!(older, current, "the vocabulary has a push to change");
    std::fs::write(dir.join("grind.d.rhai"), &older).expect("written");
    std::fs::write(dir.join(".vscode/settings.json"), "{ \"theirs\": true }\n").expect("written");
    std::fs::remove_file(dir.join("model.rhai")).expect("removed");

    let before = snapshot(dir);
    let report = init(dir, ".", &[]);
    assert_eq!(report["grind.d.rhai"], "stale");
    assert_eq!(report[".vscode/grind.code-snippets"], "unchanged");
    assert_eq!(
        snapshot(dir),
        before,
        "a plain run only reports a stale file"
    );

    let report = init(dir, ".", &["--update"]);
    assert_eq!(report["grind.d.rhai"], "updated");
    assert!(
        report
            .keys()
            .all(|p| p.ends_with(".code-snippets") || p == "grind.d.rhai"),
        "--update reports the generated files and nothing else: {report:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("grind.d.rhai")).expect("there"),
        current
    );
    let after = snapshot(dir);
    for (path, bytes) in &before {
        if path != Path::new("grind.d.rhai") {
            assert_eq!(
                after.get(path),
                Some(bytes),
                "{} is untouched",
                path.display()
            );
        }
    }
    assert!(
        !dir.join("model.rhai").exists(),
        "--update writes no seeded file, deleted or not"
    );
}

/// The header names the build that wrote a file; a different one over the same vocabulary is
/// not stale, so a new commit that registered nothing does not churn anybody's repository.
#[test]
fn only_the_vocabulary_makes_a_file_stale() {
    let sandbox = Sandbox::new("header");
    let dir = &sandbox.0;
    init(dir, ".", &[]);
    let path = dir.join(".vscode/grind.code-snippets");
    let text = std::fs::read_to_string(&path).expect("there");
    let (first, rest) = text.split_once('\n').expect("a header");
    assert!(first.starts_with("// Generated by grind "));
    let older = format!("// Generated by grind 0.0.0 (commit 0000000): an older build.\n{rest}");
    std::fs::write(&path, &older).expect("written");
    assert_eq!(
        init(dir, ".", &["--update"])[".vscode/grind.code-snippets"],
        "unchanged"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("there"),
        older,
        "and not rewritten"
    );

    // A file of that name grind did not write is not grind's to rewrite, --update or not.
    std::fs::write(dir.join("grind.d.rhai"), "// my own notes\n").expect("written");
    let report = init(dir, ".", &["--update"]);
    assert_eq!(report["grind.d.rhai"], "kept");
    assert_eq!(
        init(dir, ".", &["--update", "--force"])["grind.d.rhai"],
        "replaced"
    );
}

#[test]
fn update_and_a_template_are_two_different_requests() {
    let sandbox = Sandbox::new("conflict");
    let out = grind_in(&sandbox.0, &["init", "--update", "--template", "sheet"]);
    assert!(
        !out.status.success(),
        "--update writes no starter, so it takes no template"
    );
}
