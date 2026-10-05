// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The third-party list's generator *and* its check (`doc/third-party.md`).
//!
//! Writes `core/src/third_party/data.rs` from what is actually there — never from a list
//! somebody keeps — and fails whenever the file it would write differs from the one checked in,
//! **rewriting it as it fails**, so a dependency added, removed or bumped is one `cargo test`
//! and one `git diff` to review. What it reads:
//!
//! 1. `cargo metadata`, walked from **every workspace member that builds a binary or a wasm
//!    module** along *normal* dependency edges, on every platform at once — so a new shell is
//!    covered without being named, and the list is the union of what all of them link. Build
//!    and dev dependencies are left out: they are never in an artifact.
//! 2. Each crate's own licence files, verbatim, out of the crate as Cargo unpacked it. A crate
//!    that ships none gets the standard text of a licence it states, from `LICENSES/`, and
//!    says so.
//! 3. [`EXTRAS`]: what Cargo does not know about — the bundled fonts and the Rust standard
//!    library. A *new* such thing cannot slip past: every file REUSE says is under a licence
//!    other than ours must be claimed by an extra or be on [`NOT_SHIPPED`], and REUSE's own
//!    lint (CI) already insists every file says what it is under.
//!
//! It also refuses a licence nobody has looked at: every crate's stated licence must be
//! satisfiable from [`ALLOWED`], which is a list of licences checked against AGPL-3.0-or-later.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Licences a component may be used under here — each one checked as compatible with
/// distributing the suite under AGPL-3.0-or-later. A crate whose stated licence cannot be
/// satisfied from this list fails the test: adding to it is a decision, not a chore.
const ALLOWED: &[&str] = &[
    "0BSD",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "CC0-1.0",
    "ISC",
    "LGPL-2.1-or-later",
    "MIT",
    "MPL-2.0",
    "MPL-2.0+",
    "OFL-1.1",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "Unlicense",
    "WTFPL",
    "Zlib",
];

/// Paths whose files are never part of anything shipped, so a licence other than ours there
/// is not a component: the vendored specifications, test fixtures, the CI's own recipe.
const NOT_SHIPPED: &[&str] = &[
    "doc/",
    "ci/",
    "LICENSES/",
    "sheet/tests/",
    "text/tests/",
    "xlsx/tests/",
    "print/tests/",
    // The container recipe: it builds an image of `grind`, and nothing of it is in one.
    "Containerfile.distroless-cli",
];

/// A component Cargo does not know about.
struct Extra {
    name: &'static str,
    version: &'static str,
    licence: &'static str,
    source: &'static str,
    kind: &'static str,
    /// The REUSE paths it accounts for, spelled exactly as `REUSE.toml` spells them.
    paths: &'static [&'static str],
    /// Its notice: the copyright lines (read from the files where they carry them) and then
    /// the licence's text from `LICENSES/`.
    notice: fn(&Path) -> String,
}

const EXTRAS: &[Extra] = &[
    Extra {
        name: "Liberation Fonts",
        version: "2.1.5",
        licence: "OFL-1.1",
        source: "https://github.com/liberationfonts/liberation-fonts",
        kind: "Font",
        paths: &["print/fonts/*.ttf"],
        notice: |root| {
            let mut lines = BTreeSet::new();
            for font in files_in(&root.join("print/fonts")) {
                let bytes = std::fs::read(&font).unwrap();
                let copyright = font_name(&bytes, 0)
                    .unwrap_or_else(|| panic!("{} names no copyright", font.display()));
                lines.insert(copyright);
            }
            let mut text = lines.into_iter().collect::<Vec<_>>().join("\n");
            text.push_str("\n\n");
            text.push_str(&licence_text(root, "OFL-1.1"));
            text
        },
    },
    // `catalog.rs` holds each function's `Syntax:` and `Summary:` lines verbatim from the
    // specification, and OASIS's terms ask for their notice on every derivative work.
    Extra {
        name: "OpenDocument 1.4 Part 4 (OpenFormula), function summaries",
        version: "",
        licence: "LicenseRef-OASIS-IPR",
        source: "https://docs.oasis-open.org/office/OpenDocument/v1.4/",
        kind: "Text",
        paths: &["sheet/src/formula/funcs/catalog.rs"],
        notice: |root| {
            let text = licence_text(root, "LicenseRef-OASIS-IPR");
            let (_, notice) = text
                .split_once("\n----------------------------------------------------------------------------\n")
                .expect("LICENSES/LicenseRef-OASIS-IPR.txt's rule before the notice");
            notice.to_owned()
        },
    },
    Extra {
        name: "Rust standard library",
        version: "",
        licence: "MIT OR Apache-2.0",
        source: "https://github.com/rust-lang/rust",
        kind: "Runtime",
        paths: &[],
        notice: |root| mit(root, "The Rust Project contributors"),
    },
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

const OUT: &str = "core/src/third_party/data.rs";

#[test]
fn the_third_party_list_is_current() {
    let root = root();
    let written = render(&collect(&root));
    let path = root.join(OUT);
    let checked_in = std::fs::read_to_string(&path).unwrap_or_default();
    if checked_in != written {
        std::fs::write(&path, &written).unwrap();
        panic!(
            "{OUT} was out of date and has been rewritten — review `git diff {OUT}` and commit it \
             (doc/third-party.md)"
        );
    }
}

#[test]
fn everything_reuse_says_is_not_ours_is_on_the_list_or_never_shipped() {
    let root = root();
    let mut claims: Vec<(String, String)> = Vec::new();
    // REUSE.toml's annotations.
    let reuse: toml::Table = std::fs::read_to_string(root.join("REUSE.toml"))
        .unwrap()
        .parse()
        .unwrap();
    for annotation in reuse["annotations"].as_array().unwrap() {
        let licence = annotation["SPDX-License-Identifier"].as_str().unwrap();
        let paths = match &annotation["path"] {
            toml::Value::String(path) => vec![path.clone()],
            toml::Value::Array(paths) => paths
                .iter()
                .map(|p| p.as_str().unwrap().to_owned())
                .collect(),
            other => panic!("{other:?}"),
        };
        for path in paths {
            claims.push((path, licence.to_owned()));
        }
    }
    // Every file's own tag, and every `.license` sidecar's.
    let mut files = Vec::new();
    walk(&root, &mut files);
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if relative == OUT {
            continue;
        }
        let Ok(text) = std::fs::read(&file) else {
            continue;
        };
        let head = String::from_utf8_lossy(&text[..text.len().min(4096)]).into_owned();
        for line in head.lines() {
            if let Some(at) = line.find("SPDX-License-Identifier:") {
                let licence = line[at + "SPDX-License-Identifier:".len()..].trim();
                let licence = licence.trim_end_matches("-->").trim();
                let path = relative.strip_suffix(".license").unwrap_or(&relative);
                claims.push((path.to_owned(), licence.to_owned()));
            }
        }
    }
    let claimed: BTreeSet<&str> = EXTRAS
        .iter()
        .flat_map(|e| e.paths.iter().copied())
        .collect();
    let mut unaccounted = Vec::new();
    for (path, licence) in &claims {
        if licence == "AGPL-3.0-or-later"
            || NOT_SHIPPED.iter().any(|p| path.starts_with(p))
            || claimed.contains(path.as_str())
        {
            continue;
        }
        unaccounted.push(format!("{path} ({licence})"));
    }
    assert!(
        unaccounted.is_empty(),
        "under a licence other than ours, and neither on the third-party list (EXTRAS in \
         core/tests/third_party.rs) nor on NOT_SHIPPED:\n  {}",
        unaccounted.join("\n  ")
    );
    // And every path an extra claims is still one REUSE knows, so a claim cannot go stale.
    for path in claimed {
        assert!(
            claims.iter().any(|(p, _)| p == path),
            "EXTRAS claims {path}, which REUSE no longer annotates"
        );
    }
}

#[test]
fn a_licence_expression_is_read_as_spdx_reads_it() {
    let ok = |e: &str| satisfiable(e, &["MIT", "Apache-2.0"]);
    assert!(ok("MIT"));
    assert!(ok("MIT OR GPL-3.0"));
    assert!(ok("MIT/Apache-2.0"));
    assert!(!ok("MIT AND GPL-3.0"));
    assert!(ok("(MIT OR GPL-3.0) AND Apache-2.0"));
    assert!(!ok("GPL-3.0"));
    assert!(satisfiable(
        "Apache-2.0 WITH LLVM-exception",
        &["Apache-2.0 WITH LLVM-exception"]
    ));
}

/// One row of the list, before it is written out.
struct Row {
    name: String,
    version: String,
    licence: String,
    source: String,
    kind: &'static str,
    authors: Vec<String>,
    notices: Vec<String>,
}

fn collect(root: &Path) -> Vec<Row> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(root)
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "cargo metadata: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages: BTreeMap<&str, &Value> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p))
        .collect();
    let members: BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["id"].as_str().unwrap(), n))
        .collect();

    // Every member that produces something a person runs or loads.
    let shipped = |id: &&str| {
        packages[id]["targets"].as_array().unwrap().iter().any(|t| {
            t["kind"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k == "bin" || k == "cdylib")
        })
    };
    let mut stack: Vec<&str> = members.iter().copied().filter(shipped).collect();
    assert!(stack.len() >= 6, "every shell and the CLI: {stack:?}");
    let mut reached = BTreeSet::new();
    while let Some(id) = stack.pop() {
        if !reached.insert(id) {
            continue;
        }
        for dep in nodes[id]["deps"].as_array().unwrap() {
            let normal = dep["dep_kinds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k["kind"].is_null());
            if normal {
                stack.push(dep["pkg"].as_str().unwrap());
            }
        }
    }

    let mut rows = Vec::new();
    let mut refused = Vec::new();
    for id in reached.iter().filter(|id| !members.contains(*id)) {
        let package = packages[id];
        let name = package["name"].as_str().unwrap();
        let licence = package["license"].as_str().unwrap_or("").to_owned();
        if !satisfiable(&licence, ALLOWED) {
            refused.push(format!("{name}: {licence:?}"));
            continue;
        }
        let dir = Path::new(package["manifest_path"].as_str().unwrap())
            .parent()
            .unwrap();
        let mut paths: BTreeSet<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_licence_file(p))
            .collect();
        if let Some(file) = package["license_file"].as_str() {
            paths.insert(dir.join(file));
        }
        let mut notices: Vec<String> = paths
            .iter()
            .map(|p| normalise(&String::from_utf8_lossy(&std::fs::read(p).unwrap())))
            .filter(|t| !t.is_empty())
            .collect();
        if notices.is_empty() {
            notices.push(stand_in(root, package, &licence));
        }
        rows.push(Row {
            name: name.to_owned(),
            version: package["version"].as_str().unwrap().to_owned(),
            licence,
            source: package["repository"]
                .as_str()
                .or(package["homepage"].as_str())
                .map_or_else(|| format!("https://crates.io/crates/{name}"), str::to_owned),
            kind: "Crate",
            authors: package["authors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_owned())
                .collect(),
            notices,
        });
    }
    assert!(
        refused.is_empty(),
        "a licence not on ALLOWED in core/tests/third_party.rs — check it against \
         AGPL-3.0-or-later before adding it:\n  {}",
        refused.join("\n  ")
    );
    for extra in EXTRAS {
        rows.push(Row {
            name: extra.name.to_owned(),
            version: extra.version.to_owned(),
            licence: extra.licence.to_owned(),
            source: extra.source.to_owned(),
            kind: extra.kind,
            authors: Vec::new(),
            notices: vec![normalise(&(extra.notice)(root))],
        });
    }
    rows.sort_by(|a, b| {
        (a.name.to_lowercase(), &a.name, &a.version).cmp(&(
            b.name.to_lowercase(),
            &b.name,
            &b.version,
        ))
    });
    rows
}

/// A crate that packages no licence file: the standard text of a licence it states, with its
/// authors as the holders, and a line saying that is what this is.
fn stand_in(root: &Path, package: &Value, licence: &str) -> String {
    let name = package["name"].as_str().unwrap();
    let authors: Vec<&str> = package["authors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    let holders = match authors.is_empty() {
        true => format!("the {name} authors"),
        false => authors.join(", "),
    };
    let said = format!(
        "({name} packages no licence file; this is the standard text of the licence it states.)\n\n"
    );
    if satisfiable(licence, &["MIT"]) || licence.split(['/', ' ', '(', ')']).any(|t| t == "MIT") {
        said + &mit(root, &holders)
    } else if licence
        .split(['/', ' ', '(', ')'])
        .any(|t| t == "Apache-2.0")
    {
        said + &format!("Copyright {holders}\n\n") + &licence_text(root, "Apache-2.0")
    } else {
        panic!("{name} packages no licence file and states {licence:?}, which has no stand-in");
    }
}

fn mit(root: &Path, holders: &str) -> String {
    let text = licence_text(root, "MIT");
    assert!(text.contains("<year> <copyright holders>"));
    text.replace(
        "Copyright (c) <year> <copyright holders>",
        &format!("Copyright (c) {holders}"),
    )
}

fn licence_text(root: &Path, id: &str) -> String {
    std::fs::read_to_string(root.join("LICENSES").join(format!("{id}.txt")))
        .unwrap_or_else(|e| panic!("LICENSES/{id}.txt: {e}"))
}

fn is_licence_file(path: &Path) -> bool {
    let name = path.file_name().unwrap().to_string_lossy().to_lowercase();
    ["licen", "copying", "copyright", "notice", "unlicense"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// Line endings and trailing blanks are not the licence, and differing in them alone would
/// keep two copies of one text apart.
fn normalise(text: &str) -> String {
    let text = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    lines.join("\n").trim_matches('\n').to_owned()
}

/// The first line of `notices` that is a copyright statement rather than the licence talking
/// about one, up to the end of its first sentence — or, where none says so, who the crate
/// says its authors are.
fn copyright(notices: &[String], authors: &[String]) -> String {
    for text in notices {
        for line in text.lines() {
            let line = line.trim();
            let lower = line.to_lowercase();
            let year_after = |mark: &str| {
                line.strip_prefix(mark)
                    .is_some_and(|rest| rest.trim_start().starts_with(|c: char| c.is_ascii_digit()))
            };
            let starts = lower.starts_with("copyright") || year_after("(c)") || year_after("©");
            let placeholder = ['<', '[', '{'].iter().any(|c| line.contains(*c));
            let prose = [
                "notice",
                "license",
                "licence",
                "owner",
                "holders",
                "copyright law",
            ]
            .iter()
            .any(|w| lower.contains(w));
            if starts && !placeholder && !prose && line.len() > 10 {
                let mut end = line.len();
                for (at, _) in line.match_indices(". ") {
                    if line[at + 2..].starts_with(|c: char| c.is_uppercase()) {
                        end = at + 1;
                        break;
                    }
                }
                return line[..end].to_owned();
            }
        }
    }
    match authors.is_empty() {
        true => String::new(),
        false => format!("By {}", authors.join(", ")),
    }
}

/// Is `expression` satisfiable using only licences in `allowed`?
fn satisfiable(expression: &str, allowed: &[&str]) -> bool {
    let spaced = expression
        .replace('/', " OR ")
        .replace('(', " ( ")
        .replace(')', " ) ");
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    let mut at = 0;
    let answer = or(&tokens, &mut at, allowed);
    answer && at == tokens.len() && !tokens.is_empty()
}

fn or(tokens: &[&str], at: &mut usize, allowed: &[&str]) -> bool {
    let mut any = and(tokens, at, allowed);
    while tokens.get(*at) == Some(&"OR") {
        *at += 1;
        any |= and(tokens, at, allowed);
    }
    any
}

fn and(tokens: &[&str], at: &mut usize, allowed: &[&str]) -> bool {
    let mut all = atom(tokens, at, allowed);
    while tokens.get(*at) == Some(&"AND") {
        *at += 1;
        all &= atom(tokens, at, allowed);
    }
    all
}

fn atom(tokens: &[&str], at: &mut usize, allowed: &[&str]) -> bool {
    match tokens.get(*at) {
        Some(&"(") => {
            *at += 1;
            let inner = or(tokens, at, allowed);
            if tokens.get(*at) == Some(&")") {
                *at += 1;
            }
            inner
        }
        Some(id) => {
            *at += 1;
            let mut id = (*id).to_owned();
            if tokens.get(*at) == Some(&"WITH") {
                id = format!("{id} WITH {}", tokens.get(*at + 1).unwrap_or(&""));
                *at += 2;
            }
            allowed.contains(&id.as_str())
        }
        None => false,
    }
}

/// The text of a font's `name` record `id` (0 is its copyright), from the Windows Unicode
/// record — enough of the OpenType `name` table to read one string.
fn font_name(font: &[u8], id: u16) -> Option<String> {
    let be16 = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]);
    let be32 = |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().unwrap());
    let tables = be16(4) as usize;
    let name = (0..tables)
        .map(|t| 12 + 16 * t)
        .find(|&r| &font[r..r + 4] == b"name")
        .map(|r| be32(r + 8) as usize)?;
    let count = be16(name + 2) as usize;
    let strings = name + be16(name + 4) as usize;
    (0..count).map(|r| name + 6 + 12 * r).find_map(|r| {
        let (platform, encoding, name_id) = (be16(r), be16(r + 2), be16(r + 6));
        (platform == 3 && encoding == 1 && name_id == id).then(|| {
            let (length, offset) = (be16(r + 8) as usize, be16(r + 10) as usize);
            let units: Vec<u16> = font[strings + offset..strings + offset + length]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            String::from_utf16_lossy(&units)
        })
    })
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "ttf" || e == "otf"))
        .collect();
    files.sort();
    files
}

/// Every file in the tree that is the project's own, skipping what is built or fetched.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            let skip = name.starts_with('.') && name != ".github" && name != ".vscode"
                || ["target", "node_modules", "dist", "pkg"].contains(&name.as_str());
            if !skip {
                walk(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

fn raw(text: &str) -> String {
    let mut hashes = 1;
    while text.contains(&format!("\"{}", "#".repeat(hashes))) {
        hashes += 1;
    }
    let fence = "#".repeat(hashes);
    format!("r{fence}\"{text}\"{fence}")
}

fn render(rows: &[Row]) -> String {
    let mut texts: Vec<&str> = Vec::new();
    let mut index = BTreeMap::new();
    let mut out = String::from(
        "// Generated by `core/tests/third_party.rs` from `cargo metadata`, each crate's own\n\
         // licence files and that test's EXTRAS. Do not edit: run `cargo test -p grind-core\n\
         // --test third_party`, which rewrites it (doc/third-party.md).\n\n\
         use super::{Component, Kind};\n\n\
         pub(super) const COMPONENTS: &[Component] = &[\n",
    );
    for row in rows {
        let ids: Vec<String> = row
            .notices
            .iter()
            .map(|text| {
                let id = *index.entry(text.as_str()).or_insert_with(|| {
                    texts.push(text);
                    texts.len() - 1
                });
                id.to_string()
            })
            .collect();
        out.push_str(&format!(
            "    Component {{\n        name: {:?},\n        version: {:?},\n        licence: {:?},\n        copyright: {:?},\n        source: {:?},\n        kind: Kind::{},\n        notices: &[{}],\n    }},\n",
            row.name,
            row.version,
            row.licence,
            copyright(&row.notices, &row.authors),
            row.source,
            row.kind,
            ids.join(", ")
        ));
    }
    out.push_str("];\n\npub(super) const NOTICES: &[&str] = &[\n");
    for text in texts {
        out.push_str("    ");
        out.push_str(&raw(text));
        out.push_str(",\n");
    }
    out.push_str("];\n");
    out
}
