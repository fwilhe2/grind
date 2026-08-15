// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop A — read tolerance.
//!
//! Every `.ods` / `.fods` in LibreOffice's own test corpus must load without an
//! error and without a panic. Nothing here checks *values*; this pins the property
//! that unrecognised input is structurally inert (doc/ods-format.md §8–9). If a
//! file ever needs special-casing to pass, the context stack is wrong — fix the
//! architecture, not the file.
//!
//! Point it at a LibreOffice checkout:
//!
//!     SHEET_LO_CORPUS=/path/to/libreoffice/core/sc/qa/unit/data cargo test

use std::path::{Path, PathBuf};

const DEFAULT_CORPUS: &str = "/home/florian/code/github.com/LibreOffice/core/sc/qa/unit/data";

/// Loop A reads these two; `functions/` belongs to Loop B (phase 4).
const DIRS: [&str; 2] = ["ods", "fods"];

fn corpus_root() -> Option<PathBuf> {
    let root = PathBuf::from(
        std::env::var("SHEET_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CORPUS.to_owned()),
    );
    root.is_dir().then_some(root)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ods" | "fods")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn every_corpus_document_loads() {
    let Some(root) = corpus_root() else {
        eprintln!(
            "skipping: no LibreOffice corpus at {DEFAULT_CORPUS}; \
             set SHEET_LO_CORPUS to run loop A"
        );
        return;
    };

    let mut files = Vec::new();
    for dir in DIRS {
        collect(&root.join(dir), &mut files);
    }
    files.sort();
    assert!(!files.is_empty(), "corpus at {} is empty", root.display());

    // The one accepted outcome other than success. A password-protected document is well
    // formed and simply not ours to open; reporting that is correct behaviour, not a
    // tolerance failure. Named here rather than filtered away silently, and deliberately
    // narrow — every other error still fails the loop.
    let mut encrypted = 0usize;
    let failures: Vec<_> = files
        .iter()
        .filter_map(|path| match sheet_core::read_file(path) {
            Ok(_) => None,
            Err(sheet_core::Error::Encrypted) => {
                encrypted += 1;
                None
            }
            Err(e) => Some((path, e)),
        })
        .collect();
    eprintln!(
        "loop A: {} documents, {} read, {} password-protected, {} failed",
        files.len(),
        files.len() - failures.len() - encrypted,
        encrypted,
        failures.len(),
    );

    for (path, err) in failures.iter().take(10) {
        eprintln!("  {}: {err}", path.display());
    }
    if failures.len() > 10 {
        eprintln!("  ... and {} more", failures.len() - 10);
    }

    assert!(
        failures.is_empty(),
        "loop A: {}/{} documents failed to load",
        failures.len(),
        files.len()
    );
}
