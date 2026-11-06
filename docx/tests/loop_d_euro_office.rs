// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop D″ against a second oracle, **Euro-Office**: the same block comparison as `loop_d.rs`,
//! with Euro-Office's converter in LibreOffice's place.
//!
//! ```text
//! ours   = grind_docx::import_bytes(<file>)                        → Document
//! theirs = x2t --convert-to odt --outdir <cache> <file> → our reader → Document
//! ```
//!
//! **Needs `x2t` on `PATH`; skips with a notice without one.** `scripts/euro-office-docker/x2t` is
//! the converter out of the image pinned in `ci/euro-office-image`, and
//! `scripts/euro-office-tests.sh` puts it first on `PATH`.
//!
//! scripts/euro-office-tests.sh

mod oracle;

use std::path::{Path, PathBuf};
use std::process::Command;

use oracle::{BlockDivergence, Divergence, Oracle};

/// The ratchet on the LibreOffice corpus, measured 2026-11-06 against the image in
/// `ci/euro-office-image`: 145 of 169 accounted for, 93 agreeing outright. **Raise it, never
/// lower it.** The vendored documents are held to more: every one agrees or diverges by name.
const FLOOR: usize = 145;

/// Where Euro-Office's conversion of a whole document differs from ours: the constructs
/// LibreOffice also shows differently (`loop_d.rs`), each confirmed in Euro-Office's own `.odt`
/// on 2026-11-06. **First match wins.**
const DIVERGENCES: &[Divergence] = &[
    Divergence {
        name: "a field's last result",
        why: "a field, form field or content control is kept here as the text Word last showed \
              for it; the oracle writes a field element or nothing (tdf121203.docx's date control \
              is an empty paragraph) — so theirs is ours with characters missing, block for block",
        is: oracle::field_result,
    },
    Divergence {
        name: "a floating table or paragraph",
        why: "a table positioned on the page stays in the flow here; the oracle wraps it in a \
              `draw:frame` named `TableFrame`, which this suite's reader shows as nothing",
        is: oracle::floating,
    },
    Divergence {
        name: "a picture's anchor",
        why: "a picture is one character (U+FFFC) of its paragraph here; the oracle writes some \
              as a `draw:frame` inside a `draw:a` link, where this suite's reader does not see \
              them (image-hyperlink.docx)",
        is: oracle::picture_anchor,
    },
    Divergence {
        name: "a text box's text",
        why: "a text box's or a shape group's paragraphs follow their anchor here; the oracle \
              keeps them in `draw:frame`/`draw:text-box`, or a `draw:g` of them, which this \
              suite's reader shows as nothing",
        is: oracle::text_box,
    },
];

/// Where one block of Euro-Office's conversion differs from ours, read in its `.odt`.
const BLOCK_DIVERGENCES: &[BlockDivergence] = &[
    BlockDivergence {
        name: "a heading is written as a paragraph",
        why: "Word's heading arrives as `text:p` in the heading's paragraph style, inside a \
              `text:list` when the style is numbered; only `text:h` has an outline level, so it \
              reads as a list item or a paragraph (generated.docx, n757890.docx)",
        is: |ours, theirs| {
            ours.kind.starts_with('h')
                && !theirs.kind.starts_with('h')
                && ours.cell == theirs.cell
                && ours.text == theirs.text
        },
    },
    BlockDivergence {
        name: "body text is a level-10 heading",
        why: "`<w:outlineLvl w:val=\"9\"/>` is Word's body-text level (0 to 8 are headings 1 to \
              9); the oracle writes `text:h text:outline-level=\"10\"` (bnc821804.docx)",
        is: |ours, theirs| {
            ours.kind == "p"
                && theirs.kind == "h10"
                && ours.cell == theirs.cell
                && ours.text == theirs.text
        },
    },
    BlockDivergence {
        name: "a line break is dropped",
        why: "`<w:br/>` and `<w:br w:type=\"textWrapping\"/>` arrive as nothing, where this \
              import writes `text:line-break` (sample.docx, xml_space.docx)",
        is: |ours, theirs| {
            ours.cell == theirs.cell
                && ours.kind == theirs.kind
                && newlines(&theirs.text) < newlines(&ours.text)
                && squeezed(&ours.text) == squeezed(&theirs.text)
        },
    },
    BlockDivergence {
        name: "a newline in a run's text is a line break",
        why: "a line feed inside `<w:t xml:space=\"preserve\">` is white space Word shows as a \
              space, as this import does; the oracle writes a `text:line-break` (tdf108806.docx)",
        is: |ours, theirs| {
            ours.cell == theirs.cell
                && ours.kind == theirs.kind
                && newlines(&theirs.text) > newlines(&ours.text)
                && squeezed(&ours.text) == squeezed(&theirs.text)
        },
    },
];

fn newlines(text: &str) -> usize {
    text.matches('\n').count()
}

/// The text with all white space taken out, line breaks included.
fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

const EURO_OFFICE: Oracle = Oracle {
    name: "Euro-Office",
    cache: "grind-loop-d-docx-euro-office",
    output: "odt",
    version: x2t_version,
    convert,
    divergences: DIVERGENCES,
    block_divergences: BLOCK_DIVERGENCES,
    floor: FLOOR,
};

fn x2t_version() -> Option<String> {
    let out = Command::new("x2t").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Everything not cached yet, a container per 200 files. Not asserted: a file x2t fails on
/// leaves no output, which the run counts as not converted.
fn convert(cache: &Path, pending: &[(String, PathBuf)]) {
    if pending.is_empty() {
        return;
    }
    let (stage, inputs) = oracle::stage(cache, pending);
    for chunk in inputs.chunks(200) {
        let _ = Command::new("x2t")
            .args(["--convert-to", "odt", "--outdir"])
            .arg(cache)
            .args(chunk)
            .status()
            .expect("x2t failed to start");
    }
    let _ = std::fs::remove_dir_all(&stage);
}

#[test]
fn our_import_agrees_with_euro_office() {
    oracle::run(&EURO_OFFICE);
}
