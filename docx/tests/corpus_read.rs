// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop A″ — import tolerance, for Word.
//!
//! Every `.docx`, `.docm` and `.dotx` in LibreOffice's own Writer test corpus must import
//! without an error and without a panic, and what it imports to must be an ODF document our own
//! reader reads and our own writer saves in both forms. Nothing here checks what the document
//! *says* — that is loop D″ (`loop_d.rs`). This pins the property that unrecognised input is
//! structurally inert: if a file ever needs special-casing to pass, the walker is wrong. **Fix
//! the architecture, not the file** — an exclusion is a construct with a name, never a file
//! name.
//!
//!     GRIND_LO_CORPUS=/path/to/libreoffice/core cargo test -p grind-docx --test corpus_read
//!
//! It needs the checkout and is silent without one; `fixtures.rs` is the half that never is.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEFAULT_CHECKOUT: &str = "/home/florian/code/github.com/LibreOffice/core";

/// Writer's test data, under the checkout root — the same root loop A names.
const CORPUS: &str = "sw/qa";

/// The ratchet: how many of the corpus's documents import. Measured on 2026-10-05 against the
/// corpus at that date. **Raise it, never lower it.**
const FLOOR: usize = 2159;

fn corpus_root() -> Option<PathBuf> {
    let root = PathBuf::from(
        std::env::var("GRIND_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CHECKOUT.to_owned()),
    )
    .join(CORPUS);
    root.is_dir().then_some(root)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // LibreOffice's own `fail/` directories hold files the oracle refuses to open —
            // loop C skips them for the same reason.
            if path.file_name().is_some_and(|n| n == "fail") {
                continue;
            }
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("docx" | "docm" | "dotx")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn every_corpus_document_imports() {
    let Some(root) = corpus_root() else {
        eprintln!(
            "skipping: no LibreOffice checkout at {DEFAULT_CHECKOUT}; \
             set GRIND_LO_CORPUS to its root to run loop A″"
        );
        return;
    };
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "corpus at {} is empty", root.display());

    let mut imported = 0usize;
    let mut encrypted = 0usize;
    // Files named `.docx` that are not one at all — a `.doc`, an RTF, a broken zip the corpus
    // keeps to prove a crash is fixed. Counted by what they are, and each one checked to be
    // genuinely not a Word document rather than one this reader failed on.
    let mut not_documents: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut dropped: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut blocks = 0usize;
    let (mut tables, mut images, mut notes, mut lists) = (0, 0, 0, 0);

    for path in &files {
        let name = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        let Ok(bytes) = std::fs::read(path) else {
            failures.push(format!("{name}: unreadable"));
            continue;
        };
        let outcome = std::panic::catch_unwind(|| grind_docx::convert(&bytes));
        let (odf, report) = match outcome {
            Err(_) => {
                failures.push(format!("{name}: panicked"));
                continue;
            }
            Ok(Err(e)) if e.is_encrypted() => {
                encrypted += 1;
                continue;
            }
            Ok(Err(grind_docx::Error::Package(_) | grind_docx::Error::NotWordDocument))
                if !looks_like_a_document(&bytes) =>
            {
                not_documents.push(name);
                continue;
            }
            Ok(Err(e)) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
            Ok(Ok(converted)) => converted,
        };
        let document = match grind_text::read_bytes(&name, &odf) {
            Ok(document) => document,
            Err(e) => {
                failures.push(format!("{name}: our reader refused the import: {e}"));
                continue;
            }
        };
        // Both forms save: the flat one is the bytes as they are, the package one goes through
        // `odf::forms`, which refuses what it cannot carry rather than dropping it.
        for form in [grind_text::Form::Flat, grind_text::Form::Package] {
            if let Err(e) = grind_text::write_bytes(&document, form) {
                failures.push(format!("{name}: does not save as {form:?}: {e}"));
            }
        }
        imported += 1;
        blocks += document.blocks.len();
        tables += report.tables;
        images += report.images;
        notes += report.notes;
        lists += report.list_items;
        for (what, n) in &report.dropped {
            *dropped.entry(what.label()).or_default() += n;
        }
    }

    eprintln!(
        "loop A″: {} files; {imported} imported, {encrypted} password-protected, {} not Word \
         documents, {} failed",
        files.len(),
        not_documents.len(),
        failures.len()
    );
    eprintln!(
        "         {blocks} blocks, {lists} list items, {tables} tables, {images} pictures, \
         {notes} notes"
    );
    for (what, n) in &dropped {
        eprintln!("         not carried: {n:>6} {what}");
    }
    for name in &not_documents {
        eprintln!("         not a document: {name}");
    }
    assert!(
        failures.is_empty(),
        "{} documents failed to import:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        imported >= FLOOR,
        "{imported} imported, below the floor of {FLOOR}"
    );
}

/// Whether bytes that failed to open as a Word document might have been one: a zip holding a
/// `word/` part. A file that is not a zip, or a zip with no Word part, is something else wearing
/// the extension, and refusing it is right.
fn looks_like_a_document(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"PK\x03\x04") {
        return false;
    }
    let Ok(archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return false;
    };
    archive
        .file_names()
        .any(|name| name.to_ascii_lowercase().starts_with("word/document"))
}
