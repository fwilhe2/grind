// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Loop A for text documents** — read tolerance.
//!
//! Every `.odt` / `.fodt` in LibreOffice's own Writer test corpus must load without an error
//! and without a panic. Nothing here checks *content*; this pins the property that
//! unrecognised input is structurally inert (doc/ods-format.md §8–9, which `grind-core`
//! implements once for both document types). **If a file ever needs special-casing to pass,
//! the context stack is wrong — fix the architecture, not the file.**
//!
//! This is the loop that matters most for a new reader, because until it runs, every
//! assertion about `grind-text` has been made against a document this project wrote. Writer's
//! corpus is an order of magnitude larger than Calc's and far stranger: `sw/qa/extras/` is
//! where the regression files for two decades of bug reports live.
//!
//! Point it at a LibreOffice checkout — the **root**, not a subdirectory, because two
//! applications need two corpora out of one clone:
//!
//!     GRIND_LO_CORPUS=/path/to/libreoffice/core cargo test

use std::path::{Path, PathBuf};

const DEFAULT_CHECKOUT: &str = "/home/florian/code/github.com/LibreOffice/core";

/// Writer's test data. One directory, walked recursively — unlike Calc's, it is not flat.
const CORPUS: &str = "sw/qa";

/// Files in the corpus that are **not documents**, and what an independent parser says about
/// each.
///
/// These are not tolerance failures. §8's tolerance is about *unrecognised* content — an
/// element we have no model for gets `Ignore` and its subtree with it. `Error::Xml` is the
/// separate, structural case (doc/ods-format.md §8.2): XML that is not well-formed, or a
/// container that will not open. Refusing those is correct, and refusing them is what this
/// build does.
///
/// **Every entry was verified against a parser that is not ours** before being written down —
/// Python's `xml.etree` for the flat files, its `zipfile` for the packages — because "our
/// reader rejects it" is not evidence that a file is bad. That verification is the whole
/// difference between an exclusion list and a list of excuses.
///
/// Three of the four are deliberate: `forcepoint*` are fuzzer crash reproducers, and
/// LibreOffice's own corpus files one of them under `sw/qa/core/data/odt/fail/` — a directory
/// whose name says the import is *expected* to fail.
const MALFORMED: [(&str, &str); 4] = [
    (
        "forcepoint-dtor-1.odt",
        "zipfile: Bad CRC-32 for content.xml. In LibreOffice's own `odt/fail/` directory",
    ),
    (
        "CVE-2012-4233-1.odt",
        "not a zip and not XML — 9021 bytes of binary noise, a fuzzed crash reproducer",
    ),
    (
        "forcepoint108.fodt",
        "xml.etree: mismatched tag, line 66 — `</draw:text-box>` never closed",
    ),
    (
        "threadedException.fodt",
        "xml.etree: unbound prefix, line 403 — a namespace prefix used but never declared",
    ),
];

fn corpus_root() -> Option<PathBuf> {
    let checkout = PathBuf::from(
        std::env::var("GRIND_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CHECKOUT.to_owned()),
    );
    let root = checkout.join(CORPUS);
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
            continue;
        }
        let is_odt = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("odt") || e.eq_ignore_ascii_case("fodt"));
        if is_odt {
            out.push(path);
        }
    }
}

#[test]
fn every_corpus_document_loads() {
    let Some(root) = corpus_root() else {
        eprintln!(
            "skipping: no LibreOffice checkout at {DEFAULT_CHECKOUT}; \
             set GRIND_LO_CORPUS to its root to run loop A for text"
        );
        return;
    };

    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "corpus at {} is empty", root.display());

    // The one accepted outcome other than success, and the same one loop A accepts for
    // spreadsheets. A password-protected document is well formed and simply not ours to open;
    // reporting that is correct behaviour, not a tolerance failure. Named here rather than
    // filtered away silently, and deliberately narrow — every other error fails the loop.
    let mut encrypted = 0usize;
    let mut malformed = 0usize;
    // Informational only. A file named `.odt` whose `mimetype` says otherwise is a fixture for
    // some *other* filter's tests, and reading it as text is a no-op rather than a failure —
    // exactly what `grind_core::kind` exists to notice. Counted so the scoreboard is honest
    // about what it actually read.
    let mut not_text = 0usize;
    let mut failures = Vec::new();
    // Exclusions that no longer apply. A standing excuse nobody rechecks is how a list like
    // this stops meaning anything — the same rule `core/tests/generic.rs` holds its one
    // exemption to.
    let mut stale: Vec<&PathBuf> = Vec::new();

    for path in &files {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                failures.push((path, format!("io: {e}")));
                continue;
            }
        };
        if !matches!(
            grind_text::kind(&bytes),
            Some(grind_text::DocumentKind::Text)
        ) {
            not_text += 1;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let excused = MALFORMED.iter().find(|(file, _)| *file == name);

        match grind_text::read_bytes(&path.display().to_string(), &bytes) {
            Ok(_) if excused.is_some() => stale.push(path),
            Ok(_) => {}
            Err(grind_core::Error::Encrypted) => encrypted += 1,
            // A structural failure over a file independently confirmed to be malformed. Any
            // *other* error over the same file is still a failure: the exclusion excuses the
            // file, not every way of going wrong on it.
            Err(grind_core::Error::Xml(_) | grind_core::Error::Package(_)) if excused.is_some() => {
                malformed += 1
            }
            Err(e) => failures.push((path, e.to_string())),
        }
    }

    eprintln!(
        "loop A (text): {} documents, {} read, {} password-protected, {} not documents at all, \
         {} failed  ({} of the files read are not text documents)",
        files.len(),
        files.len() - failures.len() - encrypted - malformed,
        encrypted,
        malformed,
        failures.len(),
        not_text,
    );

    for path in &stale {
        eprintln!("  now loads: {}", path.display());
    }
    assert!(
        stale.is_empty(),
        "{} file(s) named in MALFORMED now load. Either the reader grew a repair path — in \
         which case delete the entry and say so — or the corpus changed underneath. A standing \
         excuse nobody rechecks is how the list stops meaning anything.",
        stale.len()
    );

    for (path, err) in failures.iter().take(10) {
        eprintln!("  {}: {err}", path.display());
    }
    assert!(
        failures.is_empty(),
        "{} document(s) failed to load. §8's default-ignore architecture is supposed to make \
         this impossible by construction — an unrecognised element gets `Ignore` for its whole \
         subtree — so a failure here is an architecture bug, not a file to exclude. If a new \
         file really is malformed rather than merely strange, confirm it with an independent \
         parser first and add it to MALFORMED with what that parser said.",
        failures.len()
    );
}
