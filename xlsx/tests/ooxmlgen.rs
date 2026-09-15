// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The generated corpus — workbooks written by the Open XML SDK, with an oracle beside them.
//!
//! This is the second corpus this filter is held to, and the one that **never skips**. Loop A′
//! (`corpus_read.rs`) reads LibreOffice's own `sc/qa/unit/data`, which is a magnificent
//! regression suite and a poor sample of the world: `doc/xlsx-import.md`'s risk 3 says so in
//! as many words — *"LibreOffice's xlsx files are minimal reproductions of bugs, so they
//! over-represent the strange"* — and it needs a checkout nobody has by default, so the loop
//! is silent on most machines. `fixtures.rs` answers the other half, hand-assembling packages
//! from XML written out in full, and it stops where hand-assembly stops: nobody writes a
//! four-section number format, eighteen fill patterns and 500,000 cells by hand.
//!
//! So: <https://github.com/fwilhe2/ooxmlgen> generates them, from the Open XML SDK, which
//! means the bytes are a **real producer's** rather than our idea of one. Vendored at
//! `tests/data/corpus/` from run
//! <https://github.com/fwilhe2/ooxmlgen/actions/runs/34999727235> (ooxmlgen 0.2.0,
//! 2026-09-15). 76 fixtures, 5.4 MB of workbook and a 360 KB manifest, and R7's rule applies
//! to all of it: **vendored, so the requirement cannot skip**.
//!
//! What makes it worth its size is `manifest.json`. It is not an index — it is the
//! **expectation**, machine-readable, written in this project's own vocabulary: the `Dropped`
//! variants by name, `Flavour` by name, the `Error` variant a hostile file must produce, the
//! sheet list in workbook order, and for every one of 1341 cells both what the file literally
//! contains and what a conversion of it should produce. A corpus with an oracle travelling
//! beside it is the thing loop D needs and does not have until `soffice` is on `PATH`.
//!
//! **This build is X0**, so most of that oracle is about a future milestone. Two tables carry
//! the difference and they are checked in *both* directions, which is the only arrangement
//! that survives contact with a growing filter:
//!
//! - [`PENDING`] — a claim this build does not satisfy yet, with the milestone that will.
//!   A claim that starts passing **fails this test**, and the entry must then be deleted. It
//!   is the loop-F idiom (`a test that fails the day it is projected`) applied to a roadmap.
//! - [`DECIDED_OTHERWISE`] — a claim this build will never satisfy because it answers the
//!   question differently *on purpose*. Two entries, both about hostile files, and each one
//!   is a place where our answer is the better one and the corpus should change rather than
//!   the code. Reported upstream rather than worked around.
//!
//! Everything not in either table is asserted. Run it like anything else:
//!
//!     cargo test -p grind-xlsx --test ooxmlgen

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use grind_xlsx::{Dropped, Error, Flavour};

/// The ratchet, and the only number here that is about the corpus rather than about the
/// filter: 76 fixtures at the vendored revision. **Raise it, never lower it** — a corpus that
/// was thinned and a vendoring that half failed look identical from here otherwise.
const FLOOR: usize = 76;

/// Claims this build does not satisfy yet: `(fixture, claim, the milestone and the reason)`.
///
/// Every entry is asserted to *still* fail. Delete one the day its milestone lands — the test
/// will insist on it, which is the point: a roadmap nobody checks becomes a list of things
/// that quietly already work.
const PENDING: &[(&str, &str, &str)] = &[
    // X1 opens worksheet parts. Both of these are facts *about* a worksheet, and X0 reads the
    // workbook part and the relationship graph only, so neither is reachable from here.
    (
        "realworld/mixed-flavour.xlsx",
        "flavour",
        "X1 — Transitional workbook, Strict worksheet. `seen.strict` is fed by relationship \
         types and by the workbook part's own namespace; the Strict namespace in this file is \
         on `xl/worksheets/sheet2.xml`, which X0 never opens. Measured, not assumed.",
    ),
    (
        "realworld/mce-ignorable.xlsx",
        "must-understand",
        "X1 — the `mc:MustUnderstand=\"x15\"` is on the worksheet element. When X1 opens it \
         there is a second question waiting: the manifest expects the namespace **URI**, and \
         `mce::must_understand` deliberately yields the **prefix** (its own doc comment says \
         why). The URI is the stronger spelling and resolving it needs the declaring \
         element's scope, which `xml.rs` does not keep. Decide it there, at X1.",
    ),
    // X5 is sheet order and visibility. The *name* question is `doc/xlsx-import.md`'s
    // Verification item 3 — "a sheet name Excel allows and ODF does not" — which no milestone
    // in the table actually owns; this entry is where it waits.
    (
        "document/sheets.xlsx",
        "sheet-names",
        "X5 — `Has[Brackets]` and `Has/Slash` are names Excel allows and ODF cannot address. \
         The manifest wants `Has_Brackets_` and `Has_Slash`; this build carries both \
         verbatim. The other eight names in that file — spaces, an apostrophe, CJK, 31 \
         characters — already come through exactly.",
    ),
    // The report's remaining kinds. `Macro` and `HiddenSheet` are already counted, so these
    // are the fifteen (file, kind) pairs left, each behind the milestone that reads the part
    // the construct lives in.
    (
        "document/chart.xlsx",
        "dropped:Chart",
        "X5 — the drawing and chart parts are not opened yet",
    ),
    ("document/chart.xlsx", "dropped:Drawing", "X5 — as above"),
    (
        "document/comments.xlsx",
        "dropped:Comment",
        "X5 — the comments part is not opened yet",
    ),
    (
        "document/conditional-format.xlsx",
        "dropped:ConditionalFormat",
        "X5 — `<conditionalFormatting>` lives in the worksheet part",
    ),
    (
        "document/data-validation.xlsx",
        "dropped:DataValidation",
        "X5 — `<dataValidations>` lives in the worksheet part",
    ),
    (
        "document/defined-names.xlsx",
        "dropped:SheetLocalName",
        "X5 — `<definedNames>` is read at X5",
    ),
    (
        "document/external-link.xlsx",
        "dropped:ExternalLink",
        "X5 — the link is already never followed (`fixtures.rs`); counting it is X5",
    ),
    (
        "document/merged-cells.xlsx",
        "dropped:MergedCells",
        "X5 — `<mergeCells>` lives in the worksheet part",
    ),
    (
        "document/pivot-table.xlsx",
        "dropped:PivotTable",
        "X5 — the pivot parts are not opened yet",
    ),
    (
        "document/protection.xlsx",
        "dropped:Protection",
        "X5 — `<sheetProtection>` and `<workbookProtection>`",
    ),
    (
        "document/tables.xlsx",
        "dropped:StructuredReference",
        "X2 — a structured reference is a formula this build does not translate yet",
    ),
    (
        "formulas/excluded-classes.xlsx",
        "dropped:ArrayFormula",
        "X2 — `t=\"array\"` is one of that milestone's named exclusion classes",
    ),
    (
        "styles/colors.xlsx",
        "dropped:ThemeColor",
        "X4 — the theme part is not read yet",
    ),
    (
        "styles/fonts.xlsx",
        "dropped:FontFamily",
        "X4 — `<fonts>` is not read yet",
    ),
    (
        "values/rich-text.xlsx",
        "dropped:RichText",
        "X1 — a multi-run shared string flattens, and the flattening is what is counted",
    ),
    // X2 is the expression translator, so nothing knows a function's name yet.
    (
        "formulas/errors.xlsx",
        "unknown-functions",
        "X2 — `NOSUCHFUNCTION` is only unknown once a formula has been translated",
    ),
    (
        "formulas/semantics-differ.xlsx",
        "unknown-functions",
        "X2 — CEILING, FLOOR, ROUNDDOWN and ROUNDUP: same name, different rule. The corpus \
         calls them unknown because Excel's reading is not ODF's, which is the divergence \
         `doc/xlsx-import.md`'s opening argument is about.",
    ),
    (
        "formulas/xlfn.xlsx",
        "unknown-functions",
        "X2 — the six `_xlfn.` functions, none of which is in the Small Group",
    ),
];

/// Claims this build answers differently **on purpose**: `(fixture, claim, why ours stands)`.
///
/// Not a pending list and not an excuse: each of these is a place where the corpus encodes a
/// policy this project deliberately does not have, and where the fixture should change rather
/// than the filter. Both are in `hostile/`, which is where a policy difference would be.
const DECIDED_OTHERWISE: &[(&str, &str, &str)] = &[
    (
        "hostile/no-workbook-part.xlsx",
        "error",
        "The manifest wants `Error::Package`; this build returns `Error::NotSpreadsheet`, \
         which is the variant whose doc comment is literally *\"a readable package that is \
         not a spreadsheet: no workbook part to be found\"*. The package opened fine — saying \
         otherwise would put the wrong sentence in front of a person, which is the whole \
         reason that variant exists beside `Encrypted`. `fixtures.rs` pins the same answer \
         from a hand-built package.",
    ),
    (
        "hostile/zip-slip.xlsx",
        "error",
        "The manifest wants a refusal; this build imports the workbook and reaches the sheet \
         called `Escape`. Both escape attempts fail *structurally* rather than by veto: a zip \
         entry name is a key in a package that is never extracted to disk, and the two \
         relationship targets that climb out (`../../../../../../etc/passwd` and \
         `/../../outside-the-package.xml`) resolve to no part at all. Refusing the file would \
         throw away a legitimate workbook over an attack that already missed — and `half a \
         broken file being readable beats none of it` is the rule `package.rs` is built on. \
         The property that actually matters is asserted directly, by \
         `nothing_outside_the_package_is_touched`.",
    ),
];

// ---- the vendored corpus, and the manifest that travels with it ----

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/corpus")
}

/// One fixture's row of the manifest, in the shape the tests below ask questions in.
struct Fixture {
    file: String,
    milestone: String,
    flavour: Flavour,
    /// The `Error` variant this file must produce, as the manifest spells it.
    error: Option<String>,
    /// Sheet names in workbook order, each with the name a conversion should *produce* where
    /// the manifest says that differs from the one the file carries (`expectName`).
    sheets: Vec<(String, Option<String>)>,
    expect_dropped: BTreeMap<String, usize>,
    expect_unknown_functions: BTreeSet<String>,
    expect_must_understand: BTreeSet<String>,
    /// How many cells the manifest makes a claim about. X1's work, counted here so the size
    /// of what is still owed is a number rather than an impression.
    cell_claims: usize,
    bytes: usize,
    sha256: String,
}

impl Fixture {
    /// The sheet names a conversion should produce — `expectName` where the manifest gives
    /// one, the file's own name otherwise.
    fn want_sheets(&self) -> Vec<&str> {
        self.sheets
            .iter()
            .map(|(name, expected)| expected.as_deref().unwrap_or(name.as_str()))
            .collect()
    }

    fn path(&self) -> PathBuf {
        root().join(&self.file)
    }
}

fn manifest() -> Vec<Fixture> {
    let text = std::fs::read_to_string(root().join("manifest.json"))
        .expect("the vendored manifest — `tests/data/corpus/manifest.json`");
    let json: serde_json::Value = serde_json::from_str(&text).expect("the manifest is JSON");
    json["fixtures"]
        .as_array()
        .expect("the manifest has a `fixtures` array")
        .iter()
        .map(|f| {
            let sheets = f["sheets"]
                .as_array()
                .expect("a `sheets` array")
                .iter()
                .map(|s| {
                    (
                        s["name"].as_str().expect("a sheet name").to_owned(),
                        s.get("expectName")
                            .and_then(|n| n.as_str())
                            .map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>();
            let cell_claims = f["sheets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["cells"].as_array().map_or(0, Vec::len))
                .sum();
            Fixture {
                file: f["file"].as_str().expect("a file name").to_owned(),
                milestone: f["milestone"].as_str().unwrap_or_default().to_owned(),
                flavour: match f["flavour"].as_str().expect("a flavour") {
                    "Transitional" => Flavour::Transitional,
                    "Strict" => Flavour::Strict,
                    "Mixed" => Flavour::Mixed,
                    other => panic!("{other} is not a flavour this filter has"),
                },
                error: f.get("error").and_then(|e| e.as_str()).map(str::to_owned),
                sheets,
                expect_dropped: f["expectDropped"]
                    .as_object()
                    .expect("an `expectDropped` object")
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_u64().expect("a count") as usize))
                    .collect(),
                expect_unknown_functions: strings(&f["expectUnknownFunctions"]),
                expect_must_understand: strings(&f["expectMustUnderstand"]),
                cell_claims,
                bytes: f["bytes"].as_u64().expect("a byte count") as usize,
                sha256: f["sha256"].as_str().expect("a digest").to_owned(),
            }
        })
        .collect()
}

fn strings(value: &serde_json::Value) -> BTreeSet<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| v.as_str().expect("a string").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `Dropped` by the name the manifest spells it with — which is this crate's own variant
/// name, because the generator was written against `report.rs`.
///
/// A `None` here means the corpus named a construct this filter has no variant for, and that
/// is a finding rather than a skip: [`the_manifests_vocabulary_is_this_crates_own`] fails on
/// it.
fn dropped_by_name(name: &str) -> Option<Dropped> {
    Some(match name {
        "Chart" => Dropped::Chart,
        "PivotTable" => Dropped::PivotTable,
        "ConditionalFormat" => Dropped::ConditionalFormat,
        "DataValidation" => Dropped::DataValidation,
        "Comment" => Dropped::Comment,
        "Drawing" => Dropped::Drawing,
        "Macro" => Dropped::Macro,
        "ArrayFormula" => Dropped::ArrayFormula,
        "StructuredReference" => Dropped::StructuredReference,
        "ExternalLink" => Dropped::ExternalLink,
        "SheetLocalName" => Dropped::SheetLocalName,
        "MergedCells" => Dropped::MergedCells,
        "RichText" => Dropped::RichText,
        "HiddenSheet" => Dropped::HiddenSheet,
        "ThemeColor" => Dropped::ThemeColor,
        "FontFamily" => Dropped::FontFamily,
        "Protection" => Dropped::Protection,
        _ => return None,
    })
}

fn error_name(error: &Error) -> &'static str {
    match error {
        Error::Package(_) => "Package",
        Error::Encrypted => "Encrypted",
        Error::Xml(_) => "Xml",
        Error::NotSpreadsheet => "NotSpreadsheet",
        Error::Io(_) => "Io",
    }
}

// ---- the two tables, as lookups ----

fn pending(file: &str, claim: &str) -> bool {
    PENDING.iter().any(|(f, c, _)| *f == file && *c == claim)
}

fn decided_otherwise(file: &str, claim: &str) -> bool {
    DECIDED_OTHERWISE
        .iter()
        .any(|(f, c, _)| *f == file && *c == claim)
}

/// Every claim this build is *not* held to, by either table.
fn excused(file: &str, claim: &str) -> bool {
    pending(file, claim) || decided_otherwise(file, claim)
}

/// Record that a claim was reached, so the tables can be checked in the other direction:
/// an entry naming a claim nothing ever produced is a typo, and a typo in an exclusion table
/// is an exclusion of nothing.
#[derive(Default)]
struct Reached {
    satisfied: BTreeSet<(String, String)>,
    unsatisfied: BTreeSet<(String, String)>,
}

impl Reached {
    fn check(&mut self, file: &str, claim: &str, satisfied: bool) {
        let key = (file.to_owned(), claim.to_owned());
        if satisfied {
            self.satisfied.insert(key);
        } else {
            self.unsatisfied.insert(key);
        }
    }
}

// ---- the corpus is what the generator produced ----

/// The vendoring itself, checked: the manifest and the directory name the same files, and
/// every file is the exact bytes the generator wrote.
///
/// This is what makes the rest of this file trustworthy. A vendored corpus with an oracle in
/// it has two ways to rot — a fixture regenerated without its manifest row, or a row updated
/// without its fixture — and both of them look like a filter bug from anywhere else. The
/// digest is the manifest's own `sha256` field, so the claim being checked is precisely
/// *these bytes came out of ooxmlgen run 34999727235*.
#[test]
fn the_vendored_corpus_is_what_the_generator_produced() {
    let fixtures = manifest();
    assert!(
        fixtures.len() >= FLOOR,
        "the manifest lists {} fixtures, fewer than the {FLOOR} vendored with it — the corpus \
         was thinned, or a regeneration half landed",
        fixtures.len()
    );

    // The manifest names nothing that is not there…
    let mut problems = Vec::new();
    for fixture in &fixtures {
        let path = fixture.path();
        let Ok(bytes) = std::fs::read(&path) else {
            problems.push(format!(
                "{}: named by the manifest, not vendored",
                fixture.file
            ));
            continue;
        };
        if bytes.len() != fixture.bytes {
            problems.push(format!(
                "{}: {} bytes vendored, {} in the manifest",
                fixture.file,
                bytes.len(),
                fixture.bytes
            ));
            continue;
        }
        let digest = sha256::hex(&bytes);
        if digest != fixture.sha256 {
            problems.push(format!(
                "{}: sha256 {digest}, manifest says {}",
                fixture.file, fixture.sha256
            ));
        }
    }

    // …and nothing is there that the manifest does not name. Both directions, because a
    // fixture nothing asserts anything about is a 4 MB file doing no work.
    let named: BTreeSet<&str> = fixtures.iter().map(|f| f.file.as_str()).collect();
    let mut found = Vec::new();
    collect(&root(), &root(), &mut found);
    for file in &found {
        if !named.contains(file.as_str()) {
            problems.push(format!("{file}: vendored, not named by the manifest"));
        }
    }

    assert!(
        problems.is_empty(),
        "the vendored corpus and its manifest disagree:\n  {}",
        problems.join("\n  ")
    );
    eprintln!(
        "corpus: {} fixtures, {} bytes, every digest matching",
        fixtures.len(),
        fixtures.iter().map(|f| f.bytes).sum::<usize>(),
    );
}

/// The manifest speaks this crate's vocabulary — checked, because that is the property that
/// lets the tables above be spelled in variant names rather than in prose.
#[test]
fn the_manifests_vocabulary_is_this_crates_own() {
    let mut unknown = BTreeSet::new();
    for fixture in manifest() {
        for kind in fixture.expect_dropped.keys() {
            if dropped_by_name(kind).is_none() {
                unknown.insert(kind.clone());
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "the corpus expects constructs to be dropped that `report.rs` has no variant for: \
         {unknown:?} — either the enum is missing a kind or the generator invented one"
    );
}

fn collect(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, base, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("xlsx" | "xlsm")
        ) {
            out.push(
                path.strip_prefix(base)
                    .expect("inside the corpus")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

// ---- what the filter does with it ----

/// The whole corpus, every claim X0 can answer, and the two tables held in both directions.
///
/// One test rather than six, because the tables are what is being checked and they are one
/// thing: a claim is satisfied, or it is excused by name, and no third outcome exists.
#[test]
fn every_claim_is_satisfied_or_named() {
    let fixtures = manifest();
    let mut reached = Reached::default();
    let mut failures = Vec::new();
    let mut imported = 0usize;
    let mut cells_carried = 0usize;
    let mut cell_claims = 0usize;

    for fixture in &fixtures {
        let file = fixture.file.as_str();
        cell_claims += fixture.cell_claims;
        let bytes = std::fs::read(fixture.path()).expect("a vendored fixture");

        let result = grind_xlsx::import_bytes(&bytes);

        // 1. Does it fail, and with the variant the manifest names?
        if let Some(want) = &fixture.error {
            let satisfied = match &result {
                Err(e) => error_name(e) == want.as_str(),
                Ok(_) => false,
            };
            reached.check(file, "error", satisfied);
            if !satisfied && !excused(file, "error") {
                failures.push(match &result {
                    Ok((document, _)) => format!(
                        "{file}: wanted Error::{want}, imported {} sheets",
                        document.sheets.len()
                    ),
                    Err(e) => format!("{file}: wanted Error::{want}, got Error::{}", error_name(e)),
                });
            }
            continue;
        }

        // Everything else must import. This is loop A′'s property over a corpus that is
        // always here, and there is no table entry for it: a fixture the generator says is a
        // workbook and this filter cannot open is a bug, full stop.
        let (document, report) = match result {
            Ok(pair) => pair,
            Err(e) => {
                failures.push(format!("{file}: will not import: {e}"));
                continue;
            }
        };
        imported += 1;
        cells_carried += report.cells;

        // 2. The sheet list, in workbook order.
        //
        // An empty `sheets` array is the manifest declining to make a claim — the hostile
        // family uses it, since what matters about `billion-laughs.xlsx` is that it returns
        // rather than what it returns. No importable fixture has genuinely zero sheets.
        if !fixture.sheets.is_empty() {
            let got: Vec<&str> = document.sheets.iter().map(|s| s.name.as_str()).collect();
            let want = fixture.want_sheets();
            reached.check(file, "sheet-names", got == want);
            if got != want && !excused(file, "sheet-names") {
                failures.push(format!("{file}: sheets {got:?}, manifest says {want:?}"));
            }
        }

        // 3. The flavour — a fact about the file, stated rather than branched on.
        reached.check(file, "flavour", report.flavour == fixture.flavour);
        if report.flavour != fixture.flavour && !excused(file, "flavour") {
            failures.push(format!(
                "{file}: flavour {:?}, manifest says {:?}",
                report.flavour, fixture.flavour
            ));
        }

        // 4. The report, per kind. Pending is for a kind that is *under*-counted; there is no
        // table for over-counting, because claiming to have dropped something the file does
        // not contain is a bug at every milestone.
        for (name, want) in &fixture.expect_dropped {
            let kind = dropped_by_name(name).expect("checked by its own test");
            let got = report.dropped.get(&kind).copied().unwrap_or(0);
            let claim = format!("dropped:{name}");
            reached.check(file, &claim, got == *want);
            if got != *want && !excused(file, &claim) {
                failures.push(format!(
                    "{file}: dropped {name}×{got}, manifest says ×{want}"
                ));
            }
        }
        for (kind, got) in &report.dropped {
            let name = format!("{kind:?}");
            let want = fixture.expect_dropped.get(&name).copied().unwrap_or(0);
            if *got > want {
                failures.push(format!(
                    "{file}: dropped {name}×{got} — the manifest expects ×{want}, and a \
                     report that over-counts is worse than one that stops counting"
                ));
            }
        }

        // 5. `mc:MustUnderstand`, and the functions a translated formula names.
        if !fixture.expect_must_understand.is_empty() {
            let satisfied = !report.must_understand.is_empty();
            reached.check(file, "must-understand", satisfied);
            if !satisfied && !excused(file, "must-understand") {
                failures.push(format!(
                    "{file}: nothing reported as must-understand, manifest expects {:?}",
                    fixture.expect_must_understand
                ));
            }
        }
        if !fixture.expect_unknown_functions.is_empty() {
            let satisfied = report.unknown_functions == fixture.expect_unknown_functions;
            reached.check(file, "unknown-functions", satisfied);
            if !satisfied && !excused(file, "unknown-functions") {
                failures.push(format!(
                    "{file}: unknown functions {:?}, manifest says {:?}",
                    report.unknown_functions, fixture.expect_unknown_functions
                ));
            }
        }
    }

    // The other direction. An entry that names a claim which now passes has to go, and one
    // that names a claim nothing ever evaluated is a typo excusing nothing.
    for (file, claim, why) in PENDING {
        if reached
            .satisfied
            .contains(&((*file).to_owned(), (*claim).to_owned()))
        {
            failures.push(format!(
                "{file}: `{claim}` now passes — delete its PENDING entry ({why})"
            ));
        } else if !reached
            .unsatisfied
            .contains(&((*file).to_owned(), (*claim).to_owned()))
        {
            failures.push(format!(
                "{file}: PENDING names `{claim}`, which no fixture makes a claim about"
            ));
        }
    }
    for (file, claim, _) in DECIDED_OTHERWISE {
        let key = ((*file).to_owned(), (*claim).to_owned());
        if reached.satisfied.contains(&key) {
            failures.push(format!(
                "{file}: `{claim}` agrees with the manifest now — move it out of \
                 DECIDED_OTHERWISE, or the corpus changed under us"
            ));
        } else if !reached.unsatisfied.contains(&key) {
            failures.push(format!(
                "{file}: DECIDED_OTHERWISE names `{claim}`, which no fixture evaluates"
            ));
        }
    }

    let total = reached.satisfied.len() + reached.unsatisfied.len();
    eprintln!(
        "ooxmlgen corpus: {} fixtures, {imported} imported, {} refused as named; \
         {}/{total} claims satisfied, {} pending, {} decided otherwise",
        fixtures.len(),
        fixtures.len() - imported,
        reached.satisfied.len(),
        PENDING.len(),
        DECIDED_OTHERWISE.len(),
    );
    eprintln!(
        "  cell-level claims: {cell_claims} in the manifest, {cells_carried} cells carried \
         (X1's work)"
    );
    for failure in failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    if failures.len() > 20 {
        eprintln!("  ... and {} more", failures.len() - 20);
    }
    assert!(
        failures.is_empty(),
        "{} claims are neither satisfied nor named",
        failures.len()
    );

    // X0 carries no cells at all, which is why the whole cell half of the manifest sits in
    // neither table: listing 1341 pending claims would be noise where one sentence does. This
    // assertion is that sentence, and it is written to **fail the day X1 begins** — at which
    // point the manifest's `kind`, `value`, `display` and `formula` fields become the oracle
    // loop D compares against, and this line is replaced by that comparison.
    assert_eq!(
        cells_carried, 0,
        "cells are being carried, so X1 has begun: the manifest's {cell_claims} cell claims \
         are now assertable and this marker should become the comparison that asserts them"
    );
}

/// The corpus's hostile family terminates, and does so nowhere near its budget.
///
/// `doc/xlsx-import.md`'s risk 5 is that *a headless converter is a program that eats files
/// from strangers*, and its answer is "a test with a hostile fixture rather than a paragraph".
/// This is that test, over twelve of them: entity expansion, fifty thousand levels of
/// nesting, a DTD naming a remote URL, a quarter-gigabyte part in a 263 KB archive, ten
/// thousand parts, a truncated archive, a CFB container, and an HTML table with the wrong
/// extension.
///
/// The budget is deliberately loose — measured at 0.17 s for the whole family on a developer
/// machine, asserted at 30 s. A ratio of ~180 is not a performance test and is not meant to
/// be one: what it catches is the *catastrophic* case, where an expansion actually expands or
/// a walk goes quadratic, and it is loose enough that a slow shared CI runner cannot flake it.
#[test]
fn the_hostile_family_terminates() {
    let hostile: Vec<Fixture> = manifest()
        .into_iter()
        .filter(|f| f.file.starts_with("hostile/"))
        .collect();
    assert!(hostile.len() >= 12, "the hostile family lost fixtures");

    let start = std::time::Instant::now();
    for fixture in &hostile {
        let bytes = std::fs::read(fixture.path()).expect("a vendored fixture");
        // The assertion is that this returns at all. A panic fails the test by panicking, and
        // a hang fails it by never reaching the budget check below.
        let _ = grind_xlsx::import_bytes(&bytes);
    }
    let elapsed = start.elapsed();
    eprintln!("hostile: {} fixtures in {elapsed:?}", hostile.len());
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "the hostile family took {elapsed:?}; something expanded"
    );
}

/// Nothing outside the package is read, written or fetched.
///
/// `hostile/zip-slip.xlsx` is built for this and says so in its own entry names: one climbs
/// out by a single level, one is absolute, one climbs out from a subdirectory, and one names
/// `/tmp/ooxmlgen-zip-slip-marker.xml` specifically so that a converter which extracts to
/// disk leaves evidence. Its relationships climb out too, at `/etc/passwd`.
///
/// This is the assertion `DECIDED_OTHERWISE` points at: the corpus expects the file to be
/// *refused*, and refusing it is not what makes it safe — never extracting is.
#[test]
fn nothing_outside_the_package_is_touched() {
    let marker = Path::new("/tmp/ooxmlgen-zip-slip-marker.xml");
    let before = marker.exists();

    let path = root().join("hostile/zip-slip.xlsx");
    let bytes = std::fs::read(&path).expect("the zip-slip fixture");
    let (document, _) = grind_xlsx::import_bytes(&bytes).expect("it opens; see DECIDED_OTHERWISE");
    // The escape attempts resolve to no part, so the workbook found by relationship is the
    // one legitimate part in the file.
    let names: Vec<&str> = document.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Escape"]);

    assert_eq!(
        marker.exists(),
        before,
        "importing `hostile/zip-slip.xlsx` created {}, which means an entry name reached the \
         filesystem",
        marker.display()
    );
    // The absolutely-named entry, which is the other escape whose landing site does not
    // depend on where this checkout happens to be.
    assert!(
        !Path::new("/absolute-entry-name.xml").exists(),
        "an absolute zip entry name reached the filesystem"
    );
}

/// Every imported document survives our own writer and reader.
///
/// The same identity check `corpus_read.rs` runs over LibreOffice's corpus and phase 3 owns
/// for ODF: import → write → read → compare. It proves the importer produced something **ODF
/// can actually express** rather than something that only lives in memory, and it is the
/// cheapest test in this file to keep honest as X1–X5 add content to carry.
///
/// Flat, because `doc/flat-first.md` says so and because a package would only add a zip round
/// trip to a question about the content model.
#[test]
fn every_imported_document_survives_a_write_and_a_read() {
    let mut checked = 0usize;
    let mut differences = Vec::new();
    for fixture in manifest() {
        let bytes = std::fs::read(fixture.path()).expect("a vendored fixture");
        let Ok((document, _)) = grind_xlsx::import_bytes(&bytes) else {
            continue;
        };
        let written = grind_sheet::write_bytes(&document, grind_sheet::Form::Flat)
            .expect("an imported document writes");
        let back = match grind_sheet::read_bytes(&fixture.file, &written) {
            Ok(back) => back,
            Err(e) => {
                differences.push(format!("{}: will not read back: {e}", fixture.file));
                continue;
            }
        };
        let ours: Vec<&str> = document.sheets.iter().map(|s| s.name.as_str()).collect();
        let theirs: Vec<&str> = back.sheets.iter().map(|s| s.name.as_str()).collect();
        if ours != theirs {
            differences.push(format!("{}: {ours:?} became {theirs:?}", fixture.file));
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

/// Every fixture's milestone is one `doc/xlsx-import.md` actually has.
///
/// Cheap, and it keeps the corpus and the plan from drifting apart in the way that matters:
/// the manifest's `milestone` column is how the eventual loop D decides what to compare.
#[test]
fn every_milestone_is_one_the_plan_names() {
    const MILESTONES: [&str; 7] = ["X0", "X1", "X2", "X3", "X4", "X5", "X6"];
    let mut unknown = BTreeSet::new();
    for fixture in manifest() {
        if !MILESTONES.contains(&fixture.milestone.as_str()) {
            unknown.insert(format!("{}: {}", fixture.file, fixture.milestone));
        }
    }
    assert!(
        unknown.is_empty(),
        "the corpus names milestones the plan does not: {unknown:?}"
    );
}

/// SHA-256, so that the manifest's digests are *checked* rather than decorative.
///
/// Written out here for the same reason `corpus_read.rs` writes out RC4: the alternative is a
/// dependency this crate does not otherwise want, for forty lines of arithmetic that has one
/// right answer and published test vectors. FIPS 180-4; the vectors are in the test below.
mod sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    pub fn hex(data: &[u8]) -> String {
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];

        // The padded message: the data, a 1 bit, zeroes, and the bit length as 64 bits.
        let mut message = data.to_vec();
        message.push(0x80);
        while message.len() % 64 != 56 {
            message.push(0);
        }
        message.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());

        for block in message.chunks_exact(64) {
            let mut w = [0u32; 64];
            for (i, word) in block.chunks_exact(4).enumerate() {
                w[i] = u32::from_be_bytes(word.try_into().expect("four bytes"));
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
                *slot = slot.wrapping_add(value);
            }
        }

        h.iter().map(|word| format!("{word:08x}")).collect()
    }
}

/// FIPS 180-4's own vectors, plus the one-block boundary a padding bug hides in.
#[test]
fn the_digest_is_sha256() {
    assert_eq!(
        sha256::hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256::hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256::hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    // 55, 56 and 64 bytes: one byte under the padding boundary, one over, and exactly one
    // block. Every padding bug ever written lives in one of these three.
    assert_eq!(
        sha256::hex(&[b'a'; 55]),
        "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
    );
    assert_eq!(
        sha256::hex(&[b'a'; 56]),
        "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
    );
    assert_eq!(
        sha256::hex(&[b'a'; 64]),
        "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
    );
}
