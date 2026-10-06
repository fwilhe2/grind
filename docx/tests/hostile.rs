// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A converter is a program that eats files from strangers, so every vendored document is fed
//! to it broken in the ways files break: the package cut short anywhere, and the main part cut
//! short or with a byte changed. Whatever comes back — a document, or an `Error` — nothing
//! panics, and whatever imports is ODF our own reader reads.
//!
//! Deterministic rather than random: the cuts and flips are at fixed strides, so a failure
//! here is the same failure on every machine and in CI.

use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

fn vendored() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "docx"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

/// The package's entries, and the package rebuilt with `part` replaced.
fn with_part(bytes: &[u8], part: &str, content: &[u8]) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut out));
        let options: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            let name = entry.name().to_owned();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            zip.start_file(&name, options).unwrap();
            zip.write_all(if name == part { content } else { &data })
                .unwrap();
        }
        zip.finish().unwrap();
    }
    out
}

fn part(bytes: &[u8], name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut data = Vec::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    data
}

/// Import, and if it imports, read the result back. Panics are the only failure.
fn survives(label: &str, bytes: &[u8]) {
    let outcome = std::panic::catch_unwind(|| grind_docx::convert(bytes));
    match outcome {
        Err(_) => panic!("{label}: the import panicked"),
        Ok(Err(_)) => {}
        Ok(Ok((odf, _))) => {
            if let Err(e) = grind_text::read_bytes(label, &odf) {
                panic!("{label}: imported to ODF our reader refuses: {e}");
            }
        }
    }
}

#[test]
fn a_package_cut_short_anywhere_is_refused_or_read() {
    for (name, bytes) in vendored() {
        for cut in (0..bytes.len()).step_by(211) {
            survives(&format!("{name} cut at {cut}"), &bytes[..cut]);
        }
    }
}

#[test]
fn a_damaged_main_part_is_refused_or_read() {
    for (name, bytes) in vendored() {
        let document = part(&bytes, "word/document.xml");
        for cut in (0..document.len()).step_by(97) {
            let damaged = with_part(&bytes, "word/document.xml", &document[..cut]);
            survives(&format!("{name}'s document cut at {cut}"), &damaged);
        }
        for at in (0..document.len()).step_by(53) {
            for byte in [b'<', b'>', b'"', b'&', 0xFF] {
                let mut changed = document.clone();
                changed[at] = byte;
                let damaged = with_part(&bytes, "word/document.xml", &changed);
                survives(&format!("{name}'s byte {at} as {byte:#x}"), &damaged);
            }
        }
    }
}

#[test]
fn damaged_styles_and_numbering_are_refused_or_read() {
    for (name, bytes) in vendored() {
        for part_name in ["word/styles.xml", "word/numbering.xml"] {
            let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(&bytes[..])) else {
                continue;
            };
            if archive.by_name(part_name).is_err() {
                continue;
            }
            let original = part(&bytes, part_name);
            for cut in (0..original.len()).step_by(131) {
                let damaged = with_part(&bytes, part_name, &original[..cut]);
                survives(&format!("{name}'s {part_name} cut at {cut}"), &damaged);
            }
        }
    }
}
