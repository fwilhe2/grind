// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! R12's guard (`doc/local-only.md`): nothing grind builds talks to another computer. Three
//! ways that could come back — a client crate in the lockfile, a platform's network API turned
//! on in a manifest, a socket or a spawned downloader in the code — and one check for each.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Crates whose job is talking to another computer: HTTP and WebSocket clients, TLS stacks.
const CLIENTS: &[&str] = &[
    "attohttpc",
    "curl",
    "curl-sys",
    "h2",
    "hyper",
    "isahc",
    "minreq",
    "native-tls",
    "openssl",
    "reqwest",
    "rustls",
    "surf",
    "tungstenite",
    "ureq",
    "webpki-roots",
];

#[test]
fn no_network_client_is_in_the_lockfile() {
    let lock = std::fs::read_to_string(root().join("Cargo.lock")).expect("Cargo.lock");
    let names: Vec<&str> = lock
        .lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .collect();
    assert!(names.len() > 100, "read {} packages", names.len());
    let found: Vec<&&str> = names.iter().filter(|n| CLIENTS.contains(n)).collect();
    assert!(found.is_empty(), "network crates in Cargo.lock: {found:?}");
}

/// Every `.rs` and `Cargo.toml` under the workspace's own directories, `target/` aside.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                files(&path, out);
            }
        } else if name.ends_with(".rs") || name == "Cargo.toml" {
            out.push(path);
        }
    }
}

#[test]
fn no_manifest_turns_on_a_network_api_and_no_code_opens_a_socket() {
    let mut all = Vec::new();
    files(&root(), &mut all);
    assert!(all.len() > 200, "found {} files", all.len());
    // `web-sys` and `windows` name each browser and Win32 API as a feature.
    let manifest_words = [
        "\"RequestInit\"",
        "\"XmlHttpRequest\"",
        "\"WebSocket\"",
        "\"EventSource\"",
        "Win32_Networking",
    ];
    // A test may serve itself on loopback; shipped code may not even do that.
    let code_words = [
        "std::net",
        "TcpStream",
        "UdpSocket",
        "Command::new(\"curl\")",
        "Command::new(\"wget\")",
        "fetch_with_request",
        "fetch_with_str_and_init",
    ];
    let mut offences = Vec::new();
    for path in &all {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let relative = path.strip_prefix(root()).unwrap_or(path);
        let in_tests = relative.components().any(|c| c.as_os_str() == "tests");
        let words: &[&str] = match path.file_name().is_some_and(|n| n == "Cargo.toml") {
            true => &manifest_words,
            false if in_tests || relative.ends_with("cli/tests/local_only.rs") => &[],
            false => &code_words,
        };
        for word in words {
            if text.contains(word) {
                offences.push(format!("{}: {word}", relative.display()));
            }
        }
    }
    assert!(offences.is_empty(), "{offences:#?}");
}
