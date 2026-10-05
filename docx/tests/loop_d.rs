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
//! importers make different and defensible choices, said once in [`DIVERGENCES`] with why, and
//! recognised by what the difference *is* rather than by which file shows it. `FLOOR` is the
//! ratchet on how many corpus documents agree entirely.
//!
//! Needs `soffice` on `PATH` and skips with a notice without one, as loop C and the
//! spreadsheet's loop D do. Its answers are cached under the temp directory by content hash.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The ratchet on the LibreOffice corpus: documents whose every block agrees. Measured on
/// 2026-10-06 against LibreOffice 26.8 — 117 of 161, with 31 named divergences. **Raise it,
/// never lower it.** The vendored documents are held to more: every one agrees or diverges by
/// name.
const FLOOR: usize = 117;

fn soffice_version() -> Option<String> {
    let out = Command::new("soffice").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn cache_dir(version: &str) -> PathBuf {
    let safe: String = version
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    std::env::temp_dir().join("grind-loop-d-docx").join(safe)
}

/// A stable digest of some bytes — FNV-1a, which is plenty to key a cache by.
fn digest(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}{:08x}", bytes.len())
}

/// Convert whatever is not cached yet, in one `soffice` invocation, with a private profile so a
/// running LibreOffice cannot hold its lock.
fn convert(cache: &Path, pending: &[(String, PathBuf)]) {
    if pending.is_empty() {
        return;
    }
    let stage = cache.join("stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).unwrap();
    let inputs: Vec<PathBuf> = pending
        .iter()
        .map(|(digest, source)| {
            let staged = stage.join(format!("{digest}.docx"));
            std::fs::copy(source, &staged).unwrap();
            staged
        })
        .collect();
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

/// One block as the comparison sees it.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    kind: String,
    cell: Option<(u32, u32)>,
    text: String,
}

fn blocks(doc: &grind_text::Document) -> Vec<Seen> {
    doc.blocks
        .iter()
        .map(|b| Seen {
            kind: match b.kind {
                grind_text::BlockKind::Paragraph => "p".into(),
                grind_text::BlockKind::Heading { level } => format!("h{level}"),
                grind_text::BlockKind::ListItem { depth } => format!("li{depth}"),
            },
            cell: b.cell.as_ref().map(|c| (c.row, c.column)),
            text: normalise(&b.text()),
        })
        .collect()
}

/// Whitespace the two writers are free to spell differently: a run of spaces, and spaces at a
/// block's ends.
fn normalise(text: &str) -> String {
    text.split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" \n", "\n")
        .replace("\n ", "\n")
}

/// A named divergence: a difference this loop accepts, by what it is.
struct Divergence {
    name: &'static str,
    why: &'static str,
    /// Whether the difference between our blocks and theirs is this one, given what our import
    /// reported.
    is: fn(&grind_docx::Report, &[Seen], &[Seen]) -> bool,
}

const DIVERGENCES: &[Divergence] = &[
    Divergence {
        name: "a field's last result",
        why: "a date, a reference, a form field, a content control or a citation is kept here as the text Word last \
              showed for it; the oracle writes an ODF field element, whose text this suite's \
              reader does not show — so theirs is ours with characters missing, block for block",
        is: |report, ours, theirs| {
            use grind_docx::Dropped::{ContentControl, Field, FormField};
            let fields = [Field, FormField, ContentControl]
                .iter()
                .any(|kind| report.dropped.contains_key(kind));
            fields
                && ours.len() == theirs.len()
                && ours.iter().zip(theirs).all(|(a, b)| {
                    a.kind == b.kind && a.cell == b.cell && subsequence(&b.text, &a.text)
                })
        },
    },
    Divergence {
        name: "a floating table or paragraph",
        why: "a table or paragraph positioned on the page (`w:tblpPr`, `w:framePr`) stays in \
              the flow here; the oracle puts it in a frame, which this suite's reader shows as \
              nothing — so the two agree once its text is set aside",
        is: |report, ours, theirs| {
            let strip = |blocks: &[Seen]| -> Vec<Seen> {
                blocks
                    .iter()
                    .filter(|b| b.cell.is_none() && !b.text.is_empty())
                    .cloned()
                    .collect()
            };
            // A positioned *paragraph* is the same move: the oracle's frame leaves its paragraph
            // empty where it stood.
            let blanked = ours.len() == theirs.len()
                && ours
                    .iter()
                    .zip(theirs)
                    .all(|(a, b)| a == b || (b.text.is_empty() && a.cell == b.cell));
            report.dropped.contains_key(&grind_docx::Dropped::Frame)
                && (strip(ours) == strip(theirs) || blanked)
        },
    },
    Divergence {
        name: "a picture's anchor",
        why: "a picture is one character (U+FFFC) of the paragraph it is anchored in; the oracle \
              anchors some of the same pictures to a page, a frame or a link, where this suite's \
              reader does not see them — so the two agree once the pictures are set aside",
        is: |_, ours, theirs| {
            let strip = |blocks: &[Seen]| -> Vec<Seen> {
                blocks
                    .iter()
                    .map(|b| Seen {
                        text: normalise(&b.text.replace('\u{fffc}', "")),
                        ..b.clone()
                    })
                    .collect()
            };
            (ours
                .iter()
                .chain(theirs)
                .any(|b| b.text.contains('\u{fffc}')))
                && strip(ours) == strip(theirs)
        },
    },
];

/// Whether every character of `short` appears in `long`, in order.
fn subsequence(short: &str, long: &str) -> bool {
    let mut long = long.chars();
    short.chars().all(|c| long.any(|l| l == c))
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "docx") {
            out.push(path);
        }
    }
}

#[test]
fn our_import_agrees_with_the_oracles() {
    let Some(version) = soffice_version() else {
        eprintln!("skipping loop D″: no soffice on PATH");
        return;
    };
    let cache = cache_dir(&version);
    std::fs::create_dir_all(&cache).unwrap();

    let mut inputs = Vec::new();
    files(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data"),
        &mut inputs,
    );
    let vendored = inputs.len();
    if let Ok(root) = std::env::var("GRIND_LO_CORPUS") {
        files(
            &PathBuf::from(root).join("sw/qa/extras/ooxmlimport/data"),
            &mut inputs,
        );
    }
    inputs.sort_by_key(|p| p.file_name().map(|n| n.to_owned()));

    let vendored_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let corpus = inputs.len() > vendored;
    let mut work = Vec::new();
    for path in &inputs {
        let bytes = std::fs::read(path).unwrap();
        // Not every file there is ours to read — an ODF document wearing the extension is the
        // reader's to refuse and loop A″'s to count.
        if !grind_docx::sniff(&bytes) {
            continue;
        }
        work.push((digest(&bytes), path.clone(), bytes));
    }
    let pending: Vec<(String, PathBuf)> = work
        .iter()
        .filter(|(d, _, _)| !cache.join(format!("{d}.fodt")).exists())
        .map(|(d, p, _)| (d.clone(), p.clone()))
        .collect();
    convert(&cache, &pending);

    let mut agree = 0usize;
    let mut named = vec![0usize; DIVERGENCES.len()];
    let mut disagreements = Vec::new();
    let mut vendored_disagreements: Vec<String> = Vec::new();
    for (digest, path, bytes) in &work {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Ok(theirs) = std::fs::read(cache.join(format!("{digest}.fodt"))) else {
            // The oracle would not open it; nothing to compare against.
            continue;
        };
        let theirs = match grind_text::read_bytes(&name, &theirs) {
            Ok(doc) => blocks(&doc),
            Err(_) => continue,
        };
        let (ours, report) = match grind_docx::import_bytes(bytes) {
            Ok((doc, report)) => (blocks(&doc), report),
            Err(e) => {
                disagreements.push(format!("{name}: we refused it: {e}"));
                continue;
            }
        };
        if ours == theirs {
            agree += 1;
            continue;
        }
        if let Some(i) = DIVERGENCES
            .iter()
            .position(|d| (d.is)(&report, &ours, &theirs))
        {
            named[i] += 1;
            continue;
        }
        let line = format!("{name}: {}", first_difference(&ours, &theirs));
        if path.starts_with(&vendored_dir) {
            vendored_disagreements.push(line.clone());
        }
        disagreements.push(line);
    }
    eprintln!(
        "loop D″: {version}; {} documents ({vendored} vendored), {agree} agree, {} named \
         divergences, {} disagreements",
        work.len(),
        named.iter().sum::<usize>(),
        disagreements.len()
    );
    for (d, n) in DIVERGENCES.iter().zip(&named) {
        eprintln!("  {n:>4} {} — {}", d.name, d.why);
    }
    for line in &disagreements {
        eprintln!("  {line}");
    }
    assert!(
        vendored_disagreements.is_empty(),
        "vendored documents disagree with the oracle:\n{}",
        vendored_disagreements.join("\n")
    );
    if corpus {
        assert!(agree >= FLOOR, "{agree} agree, below the floor of {FLOOR}");
    }
}

fn first_difference(ours: &[Seen], theirs: &[Seen]) -> String {
    for (i, (a, b)) in ours.iter().zip(theirs).enumerate() {
        if a != b {
            return format!(
                "block {}: ours {:?} {:?} {:?} / theirs {:?} {:?} {:?}",
                i + 1,
                a.kind,
                a.cell,
                clip(&a.text),
                b.kind,
                b.cell,
                clip(&b.text)
            );
        }
    }
    format!("{} blocks, theirs {}", ours.len(), theirs.len())
}

fn clip(text: &str) -> String {
    text.chars().take(60).collect()
}
