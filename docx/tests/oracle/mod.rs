// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop D″'s machinery, shared by its two oracles (`loop_d.rs`, LibreOffice;
//! `loop_d_euro_office.rs`, Euro-Office): the block comparison and the run over the vendored
//! documents and, with `GRIND_LO_CORPUS`, LibreOffice's Word import tests. What differs per
//! oracle is how it is invoked, its divergences and its floor.

// Each test binary uses its own part of this module.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// One converter loop D″ asks. `convert` turns every `(digest, source)` into
/// `<cache>/<digest>.<output>` and leaves no output for a file it cannot convert.
pub struct Oracle {
    pub name: &'static str,
    /// Below the system temp directory, which is what the containerised oracles mount.
    pub cache: &'static str,
    pub output: &'static str,
    pub version: fn() -> Option<String>,
    pub convert: fn(cache: &Path, pending: &[(String, PathBuf)]),
    pub divergences: &'static [Divergence],
    /// Divergences of one block each, for an oracle whose documents combine several: a
    /// document is explained when its blocks pair up and every unequal pair is one of these.
    pub block_divergences: &'static [BlockDivergence],
    /// The ratchet on the LibreOffice corpus: documents accounted for, every block agreeing or
    /// differing only by a named divergence. **Raise it, never lower it.**
    pub floor: usize,
}

impl Oracle {
    fn cache_dir(&self, version: &str) -> PathBuf {
        let safe: String = version
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        std::env::temp_dir().join(self.cache).join(safe)
    }
}

/// Copies each source into a fresh `<cache>/stage` as `<digest>.docx`.
pub fn stage(cache: &Path, pending: &[(String, PathBuf)]) -> (PathBuf, Vec<PathBuf>) {
    let stage = cache.join("stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).unwrap();
    let inputs = pending
        .iter()
        .map(|(digest, source)| {
            let staged = stage.join(format!("{digest}.docx"));
            std::fs::copy(source, &staged).unwrap();
            staged
        })
        .collect();
    (stage, inputs)
}

/// A stable digest of some bytes — FNV-1a, which is plenty to key a cache by.
pub fn digest(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}{:08x}", bytes.len())
}

/// One block as the comparison sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub kind: String,
    pub cell: Option<(u32, u32)>,
    pub text: String,
}

pub fn blocks(doc: &grind_text::Document) -> Vec<Seen> {
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
pub fn normalise(text: &str) -> String {
    text.split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" \n", "\n")
        .replace("\n ", "\n")
}

/// A named divergence: a difference this loop accepts, by what it is.
pub struct Divergence {
    pub name: &'static str,
    pub why: &'static str,
    /// Whether the difference between our blocks and theirs is this one, given what our import
    /// reported.
    pub is: fn(&grind_docx::Report, &[Seen], &[Seen]) -> bool,
}

/// Whether every character of `short` appears in `long`, in order.
pub fn subsequence(short: &str, long: &str) -> bool {
    let mut long = long.chars();
    short.chars().all(|c| long.any(|l| l == c))
}

pub fn files(dir: &Path, out: &mut Vec<PathBuf>) {
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

/// Every document's import against the oracle's conversion, block by block. The vendored ones
/// must all agree or diverge by name; the corpus is held to the oracle's floor.
pub fn run(oracle: &Oracle) {
    let Some(version) = (oracle.version)() else {
        eprintln!(
            "skipping loop D″ ({}): its converter is not on PATH",
            oracle.name
        );
        return;
    };
    let cache = oracle.cache_dir(&version);
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
        .filter(|(d, _, _)| !cache.join(format!("{d}.{}", oracle.output)).exists())
        .map(|(d, p, _)| (d.clone(), p.clone()))
        .collect();
    (oracle.convert)(&cache, &pending);

    let mut agree = 0usize;
    let mut by_blocks = vec![0usize; oracle.block_divergences.len()];
    let mut named_by_blocks = 0usize;
    let mut unconverted = 0usize;
    let mut named = vec![0usize; oracle.divergences.len()];
    let mut disagreements = Vec::new();
    let mut vendored_disagreements: Vec<String> = Vec::new();
    for (digest, path, bytes) in &work {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Ok(theirs) = std::fs::read(cache.join(format!("{digest}.{}", oracle.output))) else {
            // The oracle would not open it; nothing to compare against.
            unconverted += 1;
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
        if let Some(i) = oracle
            .divergences
            .iter()
            .position(|d| (d.is)(&report, &ours, &theirs))
        {
            named[i] += 1;
            continue;
        }
        if let Some(used) = by_block(oracle.block_divergences, &ours, &theirs) {
            for i in used {
                by_blocks[i] += 1;
            }
            named_by_blocks += 1;
            continue;
        }
        let line = format!("{name}: {}", first_difference(&ours, &theirs));
        if path.starts_with(&vendored_dir) {
            vendored_disagreements.push(line.clone());
        }
        disagreements.push(line);
    }
    eprintln!(
        "loop D″ ({}): {version}; {} documents ({vendored} vendored), {agree} agree, {} named \
         divergences, {} disagreements, {unconverted} not converted",
        oracle.name,
        work.len(),
        named.iter().sum::<usize>() + named_by_blocks,
        disagreements.len()
    );
    for (d, n) in oracle.divergences.iter().zip(&named) {
        eprintln!("  {n:>4} {} — {}", d.name, d.why);
    }
    for (d, n) in oracle.block_divergences.iter().zip(&by_blocks) {
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
        let accounted = agree + named.iter().sum::<usize>() + named_by_blocks;
        assert!(
            accounted >= oracle.floor,
            "{accounted} accounted for, below the floor of {}",
            oracle.floor
        );
    }
}

// ---- constructs both oracles show differently ----

/// A field kept as Word's last result, against the oracle's field element whose text the reader
/// does not show: theirs is ours with characters missing, block for block.
pub fn field_result(report: &grind_docx::Report, ours: &[Seen], theirs: &[Seen]) -> bool {
    use grind_docx::Dropped::{ContentControl, Field, FormField};
    let fields = [Field, FormField, ContentControl]
        .iter()
        .any(|kind| report.dropped.contains_key(kind));
    fields
        && ours.len() == theirs.len()
        && ours
            .iter()
            .zip(theirs)
            .all(|(a, b)| a.kind == b.kind && a.cell == b.cell && subsequence(&b.text, &a.text))
}

/// A table formula kept as Word's result, against the oracle's re-evaluation that fails with its
/// error text.
pub fn table_formula(report: &grind_docx::Report, ours: &[Seen], theirs: &[Seen]) -> bool {
    const FAULTY: &str = "** Expression is faulty **";
    report.dropped.contains_key(&grind_docx::Dropped::Field)
        && theirs.iter().any(|b| b.text.contains(FAULTY))
        && ours.len() == theirs.len()
        && ours.iter().zip(theirs).all(|(a, b)| {
            a.kind == b.kind
                && a.cell == b.cell
                && subsequence(&b.text.replace(FAULTY, ""), &a.text)
        })
}

/// A floating table or paragraph that stays in the flow here and is a frame in the oracle's, shown
/// as nothing.
pub fn floating(report: &grind_docx::Report, ours: &[Seen], theirs: &[Seen]) -> bool {
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
}

/// Pictures (U+FFFC) the oracle anchors where the reader does not see them: equal once they are set
/// aside.
pub fn picture_anchor(_: &grind_docx::Report, ours: &[Seen], theirs: &[Seen]) -> bool {
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
}

/// A text box whose paragraphs follow their anchor here and are a frame in the oracle's: theirs is
/// ours with those blocks taken out. Pictures (U+FFFC) are set aside first, since a shape group
/// holding the box can hold a picture too, and the oracle frames the group whole.
pub fn text_box(report: &grind_docx::Report, ours: &[Seen], theirs: &[Seen]) -> bool {
    let strip = |blocks: &[Seen]| -> Vec<Seen> {
        blocks
            .iter()
            .map(|b| Seen {
                text: normalise(&b.text.replace('\u{fffc}', "")),
                ..b.clone()
            })
            .collect()
    };
    report.dropped.contains_key(&grind_docx::Dropped::TextBox) && {
        let (ours, theirs) = (strip(ours), strip(theirs));
        let mut rest = ours.iter();
        theirs.iter().all(|b| rest.any(|a| a == b))
    }
}

/// A difference in one block, by what it is.
pub struct BlockDivergence {
    pub name: &'static str,
    pub why: &'static str,
    pub is: fn(ours: &Seen, theirs: &Seen) -> bool,
}

/// The block divergences that explain every unequal pair, each once; `None` when the blocks do
/// not pair up or one pair is unexplained.
fn by_block(divergences: &[BlockDivergence], ours: &[Seen], theirs: &[Seen]) -> Option<Vec<usize>> {
    if divergences.is_empty() || ours.len() != theirs.len() {
        return None;
    }
    let mut used = Vec::new();
    for (a, b) in ours.iter().zip(theirs) {
        if a == b {
            continue;
        }
        let i = divergences.iter().position(|d| (d.is)(a, b))?;
        if !used.contains(&i) {
            used.push(i);
        }
    }
    Some(used)
}

pub fn first_difference(ours: &[Seen], theirs: &[Seen]) -> String {
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
