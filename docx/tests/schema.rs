// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! R2 for the import: **everything written validates** against the ODF 1.4 schema, and an
//! import is something this suite writes. Every vendored document and every hand-built fixture
//! in a few shapes is converted and handed to `jing -i`, as `sheet/tests/kb.rs` does for the
//! spreadsheet's writer — and with `GRIND_LO_CORPUS`, every Word document in `sw/qa` too.
//!
//! `jing` on `PATH` is the one thing that can skip, because a validator is not vendorable; the
//! pinned oracle image and `scripts/claude-vm.sh`'s carry one. Validating the corpus's imports
//! the first time found four things an import wrote that ODF does not allow — a column of no
//! width, a vulgar fraction in a style name, a negative line height, a table with no rows.

use std::path::{Path, PathBuf};
use std::process::Command;

fn jing(paths: &[PathBuf]) -> Option<Result<(), String>> {
    let schema = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../doc/OpenDocument-v1.4-schema.rng")
        .canonicalize()
        .ok()?;
    let out = Command::new("jing")
        .arg("-i")
        .arg(schema)
        .args(paths)
        .output()
        .ok()?;
    Some(if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

fn docx_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            docx_under(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("docx" | "docm" | "dotx")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn every_import_is_valid_odf() {
    let mut inputs = Vec::new();
    docx_under(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data"),
        &mut inputs,
    );
    if let Ok(root) = std::env::var("GRIND_LO_CORPUS") {
        docx_under(&PathBuf::from(root).join("sw/qa"), &mut inputs);
    }
    inputs.sort();
    let dir = std::env::temp_dir().join(format!("docx-schema-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut written = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let Ok((odf, _)) = grind_docx::convert(&std::fs::read(input).unwrap()) else {
            continue;
        };
        let path = dir.join(format!("{i}.fodt"));
        std::fs::write(&path, odf).unwrap();
        written.push(path);
    }
    let mut failures = Vec::new();
    for chunk in written.chunks(400) {
        match jing(chunk) {
            None => {
                eprintln!("skipping: no `jing` on PATH; schema validity unchecked");
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            Some(Err(errors)) => failures.push(errors),
            Some(Ok(())) => {}
        }
    }
    eprintln!("validated {} imports", written.len());
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
