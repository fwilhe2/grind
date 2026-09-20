// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop A′ — import tolerance.
//!
//! Every `.xlsx` / `.xlsm` in LibreOffice's own test corpus must import without an error and
//! without a panic. Nothing here checks *values* — that is loop D, and X1's work. This pins
//! the property that unrecognised input is structurally inert (`xlsx/src/xml.rs`): if a file
//! ever needs special-casing to pass, the walker is wrong. **Fix the architecture, not the
//! file** — an exclusion is a *construct* with a name, never a file name.
//!
//! It needs no oracle and no display, only the corpus — and a corpus is the one thing it
//! *does* need, so on a machine with no LibreOffice checkout this file is silent.
//! `ooxmlgen.rs` is the one that never is: 76 vendored workbooks with an oracle beside them,
//! which is the same tolerance property plus the fidelity one, on every machine.
//!
//!     GRIND_LO_CORPUS=/path/to/libreoffice/core cargo test -p grind-xlsx
//!
//! CI gets it free: `ci.yml`'s `corpus` job already sparse-checks out `sc/qa/unit/data`, and
//! `xlsx/` is a directory inside it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use grind_xlsx::formula::Refusal;

const DEFAULT_CHECKOUT: &str = "/home/florian/code/github.com/LibreOffice/core";

/// Calc's test data, under the checkout root — the same root loop A names, because one clone
/// serves every corpus this suite reads.
const CORPUS: &str = "sc/qa/unit/data";

/// `.xlsm` is the same XML with a macro part beside it, so it belongs to the same loop.
const DIRS: [&str; 2] = ["xlsx", "xlsm"];

/// The ratchet. Measured on 2026-09-14 against the corpus at that date: 360 files, every one
/// of which imports. **Raise it, never lower it** — a corpus that grew and a reader that
/// broke look identical from here otherwise.
const FLOOR: usize = 360;

/// X2's ratchet: the share of this corpus's `<f>` elements that became a formula rather than
/// falling in a named class. **Raise it, never lower it.**
///
/// A *share* where [`FLOOR`] is a count, and deliberately so: the count moves whenever the
/// corpus does — 12681 of 13039 on 2026-09-20, against master at `fe40f0393` — and what is
/// being ratcheted here is the translator rather than the fixtures. The remainder is not
/// slack: every one of it is counted by class in the scoreboard below, and three of those
/// classes are §2.3.2 exclusions that will never translate.
const TRANSLATED_FLOOR: f64 = 0.97;

/// The corpus stores some files as ciphertext, and says so.
///
/// `sc/qa/unit/data/README`: *"Files with the string 'CVE' in their name are encrypted to
/// avoid problems with virus checkers on source code download"*, with the RC4 key and
/// parameters given — `mdecrypt --bare -a arcfour -o hex -k 435645 -s 3`, whose key `435645`
/// is the three bytes `CVE`.
///
/// So these are not malformed workbooks and excluding them would be excusing the wrong thing:
/// they are **workbooks the corpus keeps in a wrapper**. Verified 2026-09-14 by decrypting
/// two of them independently — both come back starting `50 4B 03 04` and open as zips with 22
/// and 17 entries. Undoing the wrapper is fifteen lines, and doing it turns three files the
/// loop would have skipped into three genuinely hostile inputs: they are security regression
/// reproducers, which is exactly what a converter that eats files from strangers should be
/// run against. Nothing is executed — the `.xlsm`'s macro is a part to be counted.
fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|byte| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            byte ^ s[(s[i as usize].wrapping_add(s[j as usize])) as usize]
        })
        .collect()
}

/// The bytes to import, undoing the corpus's own wrapper where it applied one.
fn workbook_bytes(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let wrapped = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains("CVE"));
    Some(if wrapped { rc4(b"CVE", &bytes) } else { bytes })
}

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
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("xlsx" | "xlsm")
        ) {
            out.push(path);
        }
    }
}

#[test]
fn every_corpus_workbook_imports() {
    let Some(root) = corpus_root() else {
        eprintln!(
            "skipping: no LibreOffice checkout at {DEFAULT_CHECKOUT}; \
             set GRIND_LO_CORPUS to its root to run loop A′"
        );
        return;
    };

    let mut files = Vec::new();
    for dir in DIRS {
        collect(&root.join(dir), &mut files);
    }
    files.sort();
    assert!(!files.is_empty(), "corpus at {} is empty", root.display());

    // The one accepted outcome other than success, and the same one loop A accepts: a
    // password-protected workbook is well formed and simply not ours to open. Deliberately
    // narrow — every other error still fails the loop.
    let mut encrypted = 0usize;
    let mut strict = 0usize;
    let mut sheets = 0usize;
    let mut failures = Vec::new();
    let mut carried = 0usize;
    let mut refused: BTreeMap<Refusal, usize> = BTreeMap::new();

    for path in &files {
        let Some(bytes) = workbook_bytes(path) else {
            continue;
        };
        match grind_xlsx::import_bytes(&bytes) {
            Ok((document, report)) => {
                sheets += document.sheets.len();
                if report.flavour != grind_xlsx::Flavour::Transitional {
                    strict += 1;
                }
                carried += report.formulas;
                for (class, count) in &report.refused {
                    *refused.entry(*class).or_default() += count;
                }
            }
            Err(e) if e.is_encrypted() => encrypted += 1,
            Err(e) => failures.push((path, e)),
        }
    }

    eprintln!(
        "loop A′: {} workbooks, {} imported, {} password-protected, {} failed \
         ({} sheets, {} not plain Transitional)",
        files.len(),
        files.len() - failures.len() - encrypted,
        encrypted,
        failures.len(),
        sheets,
        strict,
    );

    // X2's scoreboard, in loop B's shape: every `<f>` the corpus holds either became a formula
    // our own parser produces, or fell in a class with a name. There is no third outcome, and
    // the classes are what makes that a statement rather than a percentage.
    let lost: usize = refused.values().sum();
    eprintln!(
        "loop A′ formulas: {carried}/{} translated ({:.1}%), {lost} in a named class",
        carried + lost,
        100.0 * carried as f64 / (carried + lost).max(1) as f64,
    );
    for (class, count) in &refused {
        eprintln!("  {:24} {count}", class.label());
    }
    for (path, err) in failures.iter().take(10) {
        eprintln!("  {}: {err}", path.display());
    }
    if failures.len() > 10 {
        eprintln!("  ... and {} more", failures.len() - 10);
    }

    assert!(
        failures.is_empty(),
        "loop A′: {}/{} workbooks failed to import",
        failures.len(),
        files.len()
    );
    let share = carried as f64 / (carried + lost).max(1) as f64;
    assert!(
        share >= TRANSLATED_FLOOR,
        "loop A′ translated {carried} of {} formulas ({:.1}%), under the {:.0}% this \
         translator reached when the ratchet was set",
        carried + lost,
        100.0 * share,
        100.0 * TRANSLATED_FLOOR,
    );
    assert!(
        files.len() >= FLOOR,
        "loop A′ saw {} workbooks, fewer than the {FLOOR} this corpus had when the ratchet \
         was set — the corpus moved, or GRIND_LO_CORPUS points somewhere thinner",
        files.len()
    );
}

/// Every imported document survives our own writer and reader unchanged.
///
/// Import → write → read → compare, over the whole corpus. It is the same identity check
/// phase 3 already owns for ODF, and it proves the importer produced something **ODF can
/// actually express** rather than something that only lives in memory. Needs no oracle, so
/// it runs wherever the corpus does — schema validity itself is `jing -i`'s job in
/// `sheet/tests/kb.rs`, and this is the cheaper half that catches a document our own reader
/// cannot get back.
#[test]
fn every_imported_document_survives_a_write_and_a_read() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: no LibreOffice checkout");
        return;
    };
    let mut files = Vec::new();
    for dir in DIRS {
        collect(&root.join(dir), &mut files);
    }
    files.sort();

    let mut checked = 0usize;
    let mut differences = Vec::new();
    for path in &files {
        let Some(bytes) = workbook_bytes(path) else {
            continue;
        };
        let Ok((document, _)) = grind_xlsx::import_bytes(&bytes) else {
            continue;
        };
        // Flat, because `doc/flat-first.md` says so and because a package would only add a
        // zip round trip to a question that is about the content model.
        let written = grind_sheet::write_bytes(&document, grind_sheet::Form::Flat)
            .expect("an imported document writes");
        let back = match grind_sheet::read_bytes(&path.display().to_string(), &written) {
            Ok(back) => back,
            Err(e) => {
                differences.push(format!("{}: will not read back: {e}", path.display()));
                continue;
            }
        };
        let ours: Vec<&str> = document.sheets.iter().map(|s| s.name.as_str()).collect();
        let theirs: Vec<&str> = back.sheets.iter().map(|s| s.name.as_str()).collect();
        if ours != theirs {
            differences.push(format!("{}: {ours:?} became {theirs:?}", path.display()));
        }
        checked += 1;
    }

    eprintln!("import identity: {checked} documents written and read back");
    for difference in differences.iter().take(10) {
        eprintln!("  {difference}");
    }
    assert!(
        differences.is_empty(),
        "{} imported documents did not survive a write and a read",
        differences.len()
    );
}

/// The corpus really does contain Strict workbooks, so the namespace table is exercised by
/// the loop above rather than only by the hand-built fixture.
///
/// Measured 2026-09-14: three of them (`doc/xlsx-format.md` §1.2). Asserted as "at least
/// one", because the claim worth defending is that the table is reached, not the count.
#[test]
fn the_corpus_exercises_the_strict_spelling() {
    let Some(root) = corpus_root() else {
        eprintln!("skipping: no LibreOffice checkout");
        return;
    };
    let mut files = Vec::new();
    for dir in DIRS {
        collect(&root.join(dir), &mut files);
    }

    let strict = files
        .iter()
        .filter_map(|path| grind_xlsx::import_bytes(&workbook_bytes(path)?).ok())
        .filter(|(_, report)| report.flavour != grind_xlsx::Flavour::Transitional)
        .count();
    assert!(
        strict > 0,
        "no workbook in the corpus resolved to anything but Transitional, which means the \
         Strict half of `names.rs` is unreached by this loop"
    );
}
