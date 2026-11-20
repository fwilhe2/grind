// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ratchets of `doc/simplicity.md`: public modules per library crate, cargo features, and
//! milestone tags in comments. Each count must equal its ceiling, so lowering one is a commit
//! that lowers the ceiling with it and raising one fails here.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

fn sources(dir: &str) -> Vec<(PathBuf, String)> {
    let mut paths = Vec::new();
    rust_files(&root().join(dir), &mut paths);
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("reads");
            (path, text)
        })
        .collect()
}

/// `pub mod` lines per library crate's `src/`. Rule 4: a module is public when another crate
/// calls into it.
const PUB_MODS: &[(&str, usize)] = &[
    ("build", 8),
    ("core", 24),
    ("docx", 0),
    ("grammar", 1),
    ("lsp", 0),
    ("ooxml", 3),
    ("open", 0),
    ("print", 9),
    ("server", 1),
    ("sheet", 44),
    ("spell", 2),
    ("text", 29),
    ("xlsx", 5),
];

#[test]
fn public_modules_only_get_fewer() {
    let mut wrong = Vec::new();
    for &(krate, ceiling) in PUB_MODS {
        let count: usize = sources(&format!("{krate}/src"))
            .iter()
            .map(|(_, text)| {
                text.lines()
                    .filter(|line| line.trim_start().starts_with("pub mod "))
                    .count()
            })
            .sum();
        if count != ceiling {
            wrong.push(format!("{krate}: {count} `pub mod`, ceiling {ceiling}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "lower the ceiling when a module stops being public, and do not raise it: {wrong:#?}"
    );
}

/// The only cargo features in the workspace, each one a platform that needs the other answer:
/// the browser has no system fonts and fetches its dictionaries rather than carrying seven.
const FEATURES: &[(&str, &[&str])] = &[
    (
        "print",
        &[
            "default",
            "bundled-fonts",
            "bundled-hyphenation",
            "system-fonts",
        ],
    ),
    ("spell", &["default", "bundled"]),
];

#[test]
fn no_cargo_feature_beyond_the_ones_a_platform_needs() {
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    for entry in std::fs::read_dir(root()).expect("workspace").flatten() {
        let manifest = entry.path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let mut lines = text.lines().skip_while(|line| line.trim() != "[features]");
        if lines.next().is_none() {
            continue;
        }
        let names = lines
            .take_while(|line| !line.starts_with('['))
            .filter_map(|line| {
                let (name, _) = line.split_once('=')?;
                (!line.trim_start().starts_with('#')).then(|| name.trim().to_owned())
            })
            .collect();
        found.push((entry.file_name().to_string_lossy().into_owned(), names));
    }
    found.sort();
    let allowed: Vec<(String, Vec<String>)> = FEATURES
        .iter()
        .map(|(krate, names)| {
            (
                (*krate).to_owned(),
                names.iter().map(|name| (*name).to_owned()).collect(),
            )
        })
        .collect();
    assert_eq!(found, allowed, "doc/simplicity.md rule 3");
}

/// Comments naming a milestone — `PM`, `SV` or `DX` and its number: history, which is git's
/// (rule 5).
const MILESTONE_TAGS: usize = 0;

fn milestone_tags(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut count = 0;
    for prefix in ["PM", "SV", "DX"] {
        let mut from = 0;
        while let Some(at) = line[from..].find(prefix) {
            let start = from + at;
            let end = start + prefix.len();
            let digits = bytes[end..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            let before = start.checked_sub(1).map(|i| bytes[i]);
            let after = bytes.get(end + digits);
            if (1..=2).contains(&digits)
                && !before.is_some_and(|b| b.is_ascii_alphanumeric())
                && !after.is_some_and(u8::is_ascii_alphanumeric)
            {
                count += 1;
            }
            from = end;
        }
    }
    count
}

#[test]
fn comments_name_no_more_milestones() {
    let mut all = Vec::new();
    rust_files(&root(), &mut all);
    assert!(all.len() > 200, "found {} files", all.len());
    let count: usize = all
        .iter()
        .map(|path| {
            std::fs::read_to_string(path)
                .expect("reads")
                .lines()
                .filter(|line| line.trim_start().starts_with("//"))
                .map(milestone_tags)
                .sum::<usize>()
        })
        .sum();
    assert_eq!(
        count, MILESTONE_TAGS,
        "lower MILESTONE_TAGS when a tag goes, and say what the code does instead of adding one"
    );
}

#[test]
fn a_tag_is_a_prefix_and_one_or_two_digits() {
    assert_eq!(milestone_tags("// PM7 and SV2, then DX10"), 3);
    assert_eq!(milestone_tags("// MPM7, PM123, PMX, SPM1"), 0);
}
