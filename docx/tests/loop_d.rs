// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop D″ — import fidelity, for Word: our import of a document against LibreOffice's.
//!
//! ```text
//! ours   = grind_docx::import_bytes(<file>)                          → Document
//! theirs = soffice --headless --convert-to fodt <file> → our reader  → Document
//! ```
//!
//! and the two compared **block by block**: each block's kind (paragraph, heading level, list
//! depth), the table cell it is in, and its text. That is what a person reading the document
//! sees first and what every `grind text` address is made of; formatting is `fixtures.rs`'s,
//! where a reviewer can read the XML that states it.
//!
//! Two corpora. `tests/data/*.docx` is vendored and always runs where `soffice` is on `PATH`:
//! documents LibreOffice wrote from this suite's own ODF fixtures. And, with
//! `GRIND_LO_CORPUS`, LibreOffice's own Word import tests (`sw/qa/extras/ooxmlimport/data`),
//! which are overwhelmingly documents **Word** wrote — the dialect that matters.
//!
//! A difference is either a bug here or a **named divergence**: a construct where the two
//! importers make different and defensible choices, said once in `DIVERGENCES` with why, and
//! recognised by what the difference *is* rather than by which file shows it. `FLOOR` is the
//! ratchet on how many corpus documents agree entirely.
//!
//! Needs `soffice` on `PATH` and skips with a notice without one, as loop C and the
//! spreadsheet's loop D do. Its answers are cached under the temp directory by content hash.
//! The comparison and the run are `oracle/mod.rs`'s, shared with `loop_d_euro_office.rs`.

mod oracle;

use std::path::{Path, PathBuf};
use std::process::Command;

use oracle::{Divergence, Oracle};

/// The ratchet on the LibreOffice corpus: documents accounted for — every block agreeing, or
/// differing only by a named divergence. Measured on 2026-11-06 against the pinned 26.2: 157 of
/// 169, 108 agreeing outright. **Raise it, never lower it.** The vendored documents are held to
/// more: every one agrees or diverges by name.
const FLOOR: usize = 157;

const DIVERGENCES: &[Divergence] = &[
    Divergence {
        name: "a field's last result",
        why: "a date, a reference, a form field, a content control or a citation is kept here as the text Word last \
              showed for it; the oracle writes an ODF field element, whose text this suite's \
              reader does not show — so theirs is ours with characters missing, block for block",
        is: oracle::field_result,
    },
    Divergence {
        name: "a table formula's result",
        why: "a `= …` field in a table cell is kept here as the number Word last showed for it; \
              the oracle re-evaluates Word's expression as a `text:table-formula`, which this \
              suite's reader shows, and fails — so theirs has LibreOffice's error text where ours \
              has Word's result, block for block",
        is: oracle::table_formula,
    },
    Divergence {
        name: "a floating table or paragraph",
        why: "a table or paragraph positioned on the page (`w:tblpPr`, `w:framePr`) stays in \
              the flow here; the oracle puts it in a frame, which this suite's reader shows as \
              nothing — so the two agree once its text is set aside",
        is: oracle::floating,
    },
    Divergence {
        name: "a picture's anchor",
        why: "a picture is one character (U+FFFC) of the paragraph it is anchored in; the oracle \
              anchors some of the same pictures to a page, a frame or a link, where this suite's \
              reader does not see them — so the two agree once the pictures are set aside",
        is: oracle::picture_anchor,
    },
    Divergence {
        name: "a text box's text",
        why: "a text box's paragraphs follow the paragraph that anchors it here, since the text \
              model has nowhere to float them; the oracle keeps them in a frame, which this \
              suite's reader shows as nothing — so theirs is ours with those blocks taken out, \
              and with a picture in the same shape group set aside (tdf126426.docx)",
        is: oracle::text_box,
    },
];

const LIBREOFFICE: Oracle = Oracle {
    name: "LibreOffice",
    cache: "grind-loop-d-docx",
    output: "fodt",
    version: soffice_version,
    convert,
    divergences: DIVERGENCES,
    block_divergences: &[],
    floor: FLOOR,
};

fn soffice_version() -> Option<String> {
    let out = Command::new("soffice").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Convert whatever is not cached yet, in one `soffice` invocation, with a private profile so a
/// running LibreOffice cannot hold its lock.
fn convert(cache: &Path, pending: &[(String, PathBuf)]) {
    if pending.is_empty() {
        return;
    }
    let (stage, inputs) = oracle::stage(cache, pending);
    for chunk in inputs.chunks(200) {
        // Not asserted: a document the oracle cannot open makes it exit non-zero after
        // converting the rest, and has no answer to compare against — which the loop reads as
        // a missing output rather than a failure.
        let _ = Command::new("soffice")
            .arg("--headless")
            .arg(format!(
                "-env:UserInstallation=file://{}",
                cache.join("profile").display()
            ))
            .args(["--convert-to", "fodt", "--outdir"])
            .arg(cache)
            .args(chunk)
            .status()
            .expect("soffice failed to start");
    }
    let _ = std::fs::remove_dir_all(&stage);
}

#[test]
fn our_import_agrees_with_the_oracles() {
    oracle::run(&LIBREOFFICE);
}
