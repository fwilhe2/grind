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
//! **This build is X5**: every cell's value and kind is asserted against the manifest, one
//! claim per cell; so is every cell's **formula** — the text where the manifest states one,
//! and *the absence of one* where it states an `excel` and no translation, which is how the
//! exclusion classes are held to rather than merely described; and so is every **display** the
//! manifest states, which is the claim about a number *format*. X4's claims — what a style or a
//! track should become — are in the manifest's prose rather than its fields, so three tests
//! below write them out one assertion per note, and a fourth does the same for X5's names,
//! filters and renames. Nothing in the oracle is about a later milestone any more. Three tables carry the difference and
//! they are checked in *both* directions, which is the only arrangement that survives contact
//! with a growing filter:
//!
//! - [`PENDING`] — a claim this build does not satisfy yet, with the milestone that will.
//!   A claim that starts passing **fails this test**, and the entry must then be deleted. It
//!   is the loop-F idiom (`a test that fails the day it is projected`) applied to a roadmap.
//! - [`UNSPELLABLE`] — a claim the **model** cannot satisfy, because the piece of format it is
//!   about has no `Part` in `grind_sheet::numfmt`: a fraction, an exponent, an elapsed hour
//!   count. Not pending, since no milestone of this phase lands them — adding a part is a
//!   decision about the core's format model — and not decided otherwise, since the manifest is
//!   right and this build is the one that cannot say it.
//! - [`DECIDED_OTHERWISE`] — a claim this build will never satisfy because it answers the
//!   question differently *on purpose*. Two are about hostile files, three are manifests that
//!   disagree with their own fixture's bytes, and two are spellings where ours is as true as
//!   the one asked for. Each is a place where the corpus should change rather than the code.
//!
//! Everything not in either table is asserted. Run it like anything else:
//!
//!     cargo test -p grind-xlsx --test ooxmlgen

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use grind_sheet::formula::date;
use grind_sheet::model::{CellValue, NumberKind, Pos, Sheet};
use grind_sheet::style::CellStyle;
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
    // Empty since X5, which satisfied the last eleven: every claim the manifest makes is now
    // either held or named in one of the two tables below. Kept, and still checked in both
    // directions, because the corpus grows and X6 is still to come.
];

/// Claims this build answers differently **on purpose**: `(fixture, claim, why ours stands)`.
///
/// Not a pending list and not an excuse: each of these is a place where the fixture should
/// change rather than the filter. The two in `hostile/` encode a policy this project
/// deliberately does not have; the third is a manifest that disagrees with its own fixture's
/// bytes, which is the one kind of claim no filter could satisfy.
const DECIDED_OTHERWISE: &[(&str, &str, &str)] = &[
    (
        "styles/colors.xlsx",
        "dropped:ThemeColor",
        "The manifest expects fourteen; this build counts none, because every theme colour in \
         the file resolves — the workbook has a theme part, and all seventeen cells that name \
         a slot come out as the colour the fixture's own column C says they should (checked \
         by `the_colour_fixture_says_what_each_colour_is`) and as the oracle draws them. What \
         does not come through is the *link* to the theme, which changes nothing while the \
         theme does not change, and ODF has no theme to change. Fourteen also matches no count \
         of anything in the file: seventeen fonts name a theme slot. `ThemeColor` here means \
         a theme colour with **no theme to resolve it in** — the case below.",
    ),
    (
        "styles/borders.xlsx",
        "dropped:ThemeColor",
        "The manifest expects none; this build counts one. B16's top and bottom edges name \
         `theme=\"4\"`, and `borders.xlsx` has no theme part at all — its package holds no \
         `xl/theme/`. There is nothing to resolve the slot against, so the edges are drawn in \
         the ink and the cell is counted. The oracle does not resolve it either: it draws both \
         edges white, and writes no colour at all for the same reference on a font (measured \
         2026-09-21, `doc/xlsx-format.md` §4.2) — two different guesses where this build \
         makes none.",
    ),
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
    (
        "document/tables.xlsx",
        "dropped:StructuredReference",
        "The manifest expects four; the fixture holds seven. `xl/worksheets/sheet1.xml` has \
         seven `<f>` elements naming the table — C2:C5's `Sales[[#This Row],[Amount]]` and \
         E1:E3's `SUM(Sales[Amount])`, `COUNTA(Sales[#Headers])` and `ROWS(Sales[#All])`, read \
         on 2026-09-20. Four is the number of *distinct* forms among them. Every other kind in \
         this report is counted once per cell that lost something — `RichText` and \
         `ExternalLink` both are — and a person reading `structured reference ×4` over seven \
         emptied cells would be owed three more.",
    ),
    (
        "formulas/semantics-differ.xlsx",
        "unknown-functions",
        "The manifest expects CEILING, FLOOR, ROUNDDOWN and ROUNDUP; the fixture calls only \
         the first two. Its thirteen formulas are MOD×3, ROUND×5, CEILING×2, FLOOR, INT and \
         TRUNC, read out of `xl/worksheets/sheet1.xml` on 2026-09-20 — ROUNDDOWN and ROUNDUP \
         appear in the file's prose and nowhere in its cells. Reporting a name no formula \
         mentions is the one thing this field must never do.",
    ),
    (
        "formulas/references.xlsx",
        "formula:Refs!B11",
        "The manifest states no translation for `SUM(Data:Sheet3!A1:A2)` because grind's own \
         table did not cover a sheet span over a *range*. It does now: the cuboid is \
         `[Data.A1:Sheet3.A2]`, the same rule B10 already follows one cell at a time \
         (`doc/xlsx-import.md` Part II §3). The fixture should carry the claim rather than \
         decline it.",
    ),
    (
        "formulas/shared-groups.xlsx",
        "formula:Shared!F1",
        "The manifest expects `[#REF!]*10`; this build writes `#REF!*10`. The group's master \
         is at F2 and refers to A1, so the follower above it shifts off the sheet — both \
         spellings say exactly that. §5.8's bracketed form and §5.12's name are one value in \
         the core's AST (`Expr::Error`), and its serialiser writes the name; the oracle writes \
         the brackets. Changing that is a decision about ODF's own printer, not about this \
         filter, and it would move every formula in the workspace.",
    ),
    (
        "realworld/xml-space-preserve.xlsx",
        "cell:Sheet1!A5",
        "The manifest wants `a\\ttab and a\\n` followed by twenty spaces and `newline`; the \
         fixture's own `xl/sharedStrings.xml` holds `<t xml:space=\"preserve\">a\\ttab and \
         a\\nnewline</t>` — no indentation at all, read with `unzip -p` on 2026-09-19. The \
         import carries exactly what the file says, and whitespace is never trimmed \
         (`xml.rs`), so the claim is the generator's source indentation leaking into its \
         expectation rather than anything the workbook contains.",
    ),
];

/// Claims the **model** cannot satisfy: `(fixture, claim, the class and what is missing)`.
///
/// X3's own table, and a third one because neither of the others fits. These are not
/// pending — no milestone of this phase lands them, since what is missing is a piece of
/// `grind_sheet::numfmt` and adding one is a decision about the core's format model rather
/// than about an import filter (`doc/xlsx-import.md` Part II §4 says so, and
/// `grind_xlsx::numfmt::Unspellable` is the same list in code). They are not decided
/// otherwise either: the manifest is *right* about what Excel shows, and this build is the
/// one that cannot say it.
///
/// Every entry names its class, and is asserted to still fail exactly like [`PENDING`]: the
/// day `numfmt` grows a fraction part, these fail for passing and the table shrinks.
const UNSPELLABLE: &[(&str, &str, &str)] = &[
    (
        "numfmt/custom-numeric.xlsx",
        "display:Numeric!B6",
        "Scaling — `#,##0,` divides the displayed number by a thousand \
         (`number:display-factor`). The format is refused whole, so the cell shows its plain \
         value: a number shown a thousand times too large is worse than an unformatted one.",
    ),
    (
        "numfmt/custom-numeric.xlsx",
        "display:Numeric!B7",
        "Scaling — `#,##0,,\" M\"`, the same class twice over.",
    ),
    (
        "numfmt/custom-numeric.xlsx",
        "display:Numeric!B10",
        "Scientific — `0.00E+00`. `number:scientific-number` is an element this model has no \
         `Part` for, and dropping the exponent would show 12345.678 as `12345.68`.",
    ),
    (
        "numfmt/custom-numeric.xlsx",
        "display:Numeric!B11",
        "Scientific — `##0.0E+0`, the engineering spelling, whose exponent moves in threes.",
    ),
    (
        "numfmt/custom-numeric.xlsx",
        "display:Numeric!B12",
        "Fraction — `# ?/?`. `number:fraction` has no `Part` either; 3.75 shown as `4` is the \
         alternative, and it is worse than `3.75`.",
    ),
    (
        "numfmt/custom-numeric.xlsx",
        "display:Numeric!B13",
        "Fraction — `# ??/??`, two denominator digits.",
    ),
    (
        "numfmt/custom-numeric.xlsx",
        "display:Numeric!B14",
        "Fraction — `# ?/8`, a fixed denominator.",
    ),
    (
        "numfmt/custom-numeric.xlsx",
        "display:Numeric!B17",
        "BlankWidth — `_)` is a blank as wide as `)`, which is how Excel lines a bracketed \
         negative up under a positive one. LibreOffice carries it as \
         `loext:blank-width-char`, an extension attribute rather than ODF; this build drops \
         the trailing space and keeps the rest of the format, since everything else about \
         the cell is right.",
    ),
    (
        "numfmt/datetime-codes.xlsx",
        "display:DateTime!B8",
        "NarrowMonth — `mmmmm` is the month's initial. `number:month` is long, short or \
         numeric and has no fourth spelling, so this shows `Mar`. **The oracle loses it the \
         same way**, measured from its own conversion of this fixture on 2026-09-20.",
    ),
    (
        "numfmt/datetime-codes.xlsx",
        "display:DateTime!B20",
        "NarrowAmPm — `A/P` is the one-letter meridiem marker and `number:am-pm` has one \
         spelling. The oracle renders a marker here too, in its own case.",
    ),
    (
        "numfmt/datetime-codes.xlsx",
        "display:DateTime!B23",
        "Elapsed — `[h]:mm` counts hours past 24. The core names the attribute that would \
         carry it (`number:truncate-on-overflow=\"false\"`) and deliberately does not model \
         it, so the format is refused and the cell shows the clock reading its value is.",
    ),
    (
        "numfmt/datetime-codes.xlsx",
        "display:DateTime!B24",
        "Elapsed — `[m]`, elapsed minutes.",
    ),
    (
        "numfmt/datetime-codes.xlsx",
        "display:DateTime!B25",
        "Elapsed — `[s]`, elapsed seconds.",
    ),
    (
        "values/times.xlsx",
        "display:Times!C2",
        "Elapsed — the whole `C` column of this fixture is `[h]:mm:ss` over the same serials \
         column `B` shows as a clock, which is exactly the distinction the class is about. \
         Two of its seven cells (C4, C5) are under a day and read the same either way; these \
         five are where it shows.",
    ),
    (
        "values/times.xlsx",
        "display:Times!C3",
        "Elapsed — as above.",
    ),
    (
        "values/times.xlsx",
        "display:Times!C6",
        "Elapsed — as above, and the first where the hours pass 24.",
    ),
    (
        "values/times.xlsx",
        "display:Times!C7",
        "Elapsed — as above.",
    ),
    (
        "values/times.xlsx",
        "display:Times!C8",
        "Elapsed — as above.",
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
    /// Every cell the manifest makes a claim about, with the index of the sheet it is on.
    cells: Vec<(usize, Cell)>,
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
            let cells = f["sheets"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .flat_map(|(i, s)| {
                    s["cells"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(move |c| (i, Cell::from_json(c)))
                })
                .collect();
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
                cells,
                bytes: f["bytes"].as_u64().expect("a byte count") as usize,
                sha256: f["sha256"].as_str().expect("a digest").to_owned(),
            }
        })
        .collect()
}

/// One cell's claim: where it is, what kind of value it holds, and the value — in the
/// manifest's own spelling, which is ODF's (`office:date-value`, `office:time-value`).
struct Cell {
    address: String,
    kind: String,
    /// `None` where the manifest declines to assert one — serial 60 in the 1900 system, the day
    /// that does not exist, is the case it exists for.
    value: Option<String>,
    /// The OpenFormula text a conversion should produce, where the manifest states one.
    formula: Option<String>,
    /// What the cell's `<f>` says. Present on every formula cell, so a cell with an `excel`
    /// and no `formula` is one whose expression the conversion is expected **not** to carry —
    /// which is how the exclusion classes are asserted rather than merely described.
    excel: Option<String>,
    /// The text Excel shows in the cell, where the manifest states one — X3's claim, and the
    /// only one about a *format* rather than a value.
    display: Option<String>,
}

impl Cell {
    fn from_json(c: &serde_json::Value) -> Self {
        let text = |key: &str| c.get(key).and_then(|v| v.as_str()).map(str::to_owned);
        Cell {
            address: c["ref"].as_str().expect("a cell ref").to_owned(),
            kind: c["kind"].as_str().expect("a cell kind").to_owned(),
            value: text("value"),
            formula: text("formula"),
            excel: text("excel"),
            display: text("display"),
        }
    }

    /// The display claim: what one cell reads as, asked through the core's own
    /// `grind_sheet::render` so that this test cannot answer it differently from a viewport.
    ///
    /// A cell whose format this build refuses shows its plain value, which is a *different*
    /// string from the one Excel shows — that is the whole point of refusing, and it is why
    /// the misses here are named in [`UNSPELLABLE`] rather than waved through.
    fn check_display(&self, sheet: &Sheet, null_date: i64) -> Option<Result<(), String>> {
        let want = self.display.as_ref()?;
        let pos = grind_xlsx::address::cell(&self.address).expect("a manifest address");
        let got = grind_sheet::render(sheet, pos, null_date);
        Some(match got == *want {
            true => Ok(()),
            false => Err(format!("{got:?}")),
        })
    }

    /// The formula claim, or `None` where the cell makes none.
    ///
    /// The stored formula keeps the `=` every formula in this workspace is stored with
    /// (§5.2's intro), and the manifest states the expression alone, so the intro is taken off
    /// with the core's own function rather than by trimming a character.
    fn check_formula(&self, sheet: &Sheet) -> Option<Result<(), String>> {
        let pos = grind_xlsx::address::cell(&self.address).expect("a manifest address");
        let got = sheet
            .formula(pos)
            .map(|stored| stored[grind_sheet::formula::parse::intro(stored).len()..].to_owned());
        match (&self.formula, &self.excel) {
            (Some(want), _) => Some(match got.as_deref() == Some(want.as_str()) {
                true => Ok(()),
                false => Err(format!("{got:?}")),
            }),
            // An `<f>` the manifest states no translation for is one this build must refuse.
            (None, Some(_)) => Some(match got {
                None => Ok(()),
                Some(got) => Err(format!("{got:?}")),
            }),
            (None, None) => None,
        }
    }

    /// Does the imported sheet hold what this claim says? `Err` carries what it held instead.
    ///
    /// Numbers compare at 15 significant digits — loop C's one loosening, because that is all
    /// LibreOffice writes and the manifest's figures are the oracle's. Dates and times compare
    /// to the second, which is the resolution both of their spellings carry.
    fn check(&self, sheet: &Sheet, null_date: i64) -> Result<(), String> {
        let pos = grind_xlsx::address::cell(&self.address).expect("a manifest address");
        let got = sheet.get(pos);
        let kind = sheet.kind(pos);
        let Some(want) = &self.value else {
            return Ok(());
        };
        let held = || format!("{got:?} kind {kind:?}");
        let number = |n: f64| match got {
            CellValue::Number(g) if close(g, n) => Ok(()),
            _ => Err(held()),
        };
        match self.kind.as_str() {
            "Empty" => match got {
                CellValue::Empty => Ok(()),
                _ => Err(held()),
            },
            "Text" | "Error" => match &got {
                CellValue::Text(t) if t == want => Ok(()),
                _ => Err(held()),
            },
            "Bool" => match got {
                CellValue::Bool(b) if b.to_string() == *want => Ok(()),
                _ => Err(held()),
            },
            // Currency and percentage are a *format*'s business in this model (X3); the value
            // underneath is a plain number either way.
            "Number" | "Currency" | "Percentage" => {
                number(want.parse().expect("a manifest number"))
            }
            // `numfmt/` spells a date's value as its serial rather than in ISO, because what
            // those fixtures are about is the display. Every such serial is past 61, where the
            // 1900 system and ODF's epoch agree, so it compares as the number it is.
            "Date" | "Time" if want.parse::<f64>().is_ok() => {
                let wanted = match self.kind.as_str() {
                    "Date" => NumberKind::Date,
                    _ => NumberKind::Time,
                };
                match kind == Some(wanted) {
                    true => number(want.parse().expect("checked")),
                    false => Err(held()),
                }
            }
            "Date" => match (kind, date::parse_date(want, null_date)) {
                (Some(NumberKind::Date), Some(serial)) => second(&got, serial).ok_or_else(held),
                _ => Err(held()),
            },
            "Time" => match (kind, date::parse_time(want)) {
                (Some(NumberKind::Time), Some(fraction)) => second(&got, fraction).ok_or_else(held),
                _ => Err(held()),
            },
            other => Err(format!("a kind this test does not know: {other}")),
        }
    }
}

/// Equal to 15 significant digits.
fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-15 * a.abs().max(b.abs())
}

/// The same instant, to the second.
fn second(got: &CellValue, want: f64) -> Option<()> {
    match got {
        &CellValue::Number(g) if (g - want).abs() * 86_400.0 < 0.5 => Some(()),
        _ => None,
    }
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

fn unspellable(file: &str, claim: &str) -> bool {
    UNSPELLABLE
        .iter()
        .any(|(f, c, _)| *f == file && *c == claim)
}

/// Every claim this build is *not* held to, by any of the three tables.
fn excused(file: &str, claim: &str) -> bool {
    pending(file, claim) || decided_otherwise(file, claim) || unspellable(file, claim)
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

/// The whole corpus, every claim this build can answer, and the two tables held in both
/// directions.
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
    let mut cells_matching = 0usize;
    let mut formula_claims = 0usize;
    let mut formulas_matching = 0usize;
    let mut display_claims = 0usize;
    let mut displays_matching = 0usize;

    for fixture in &fixtures {
        let file = fixture.file.as_str();
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

        // 6. Every cell the manifest names — X1's half of the oracle, and the reason the
        // manifest exists. One claim per cell, spelled `cell:<sheet>!<address>` so a PENDING
        // entry can name exactly the cell it excuses.
        for (sheet, cell) in &fixture.cells {
            let claim = format!("cell:{}!{}", fixture.sheets[*sheet].0, cell.address);
            let outcome = document
                .sheets
                .get(*sheet)
                .ok_or_else(|| "no such sheet".to_owned())
                .and_then(|s| cell.check(s, document.null_date));
            cell_claims += 1;
            cells_matching += usize::from(outcome.is_ok());
            reached.check(file, &claim, outcome.is_ok());
            if let Err(held) = outcome
                && !excused(file, &claim)
            {
                failures.push(format!(
                    "{file}: {claim} is {} {:?}, the import holds {held}",
                    cell.kind,
                    cell.value.as_deref().unwrap_or("")
                ));
            }

            // 6c. And what it displays — X3's half, stated for the cells whose fixture is
            // about a format rather than a value.
            let claim = format!("display:{}!{}", fixture.sheets[*sheet].0, cell.address);
            if let Some(outcome) = document
                .sheets
                .get(*sheet)
                .and_then(|s| cell.check_display(s, document.null_date))
            {
                display_claims += 1;
                displays_matching += usize::from(outcome.is_ok());
                reached.check(file, &claim, outcome.is_ok());
                if let Err(held) = outcome
                    && !excused(file, &claim)
                {
                    failures.push(format!(
                        "{file}: {claim} should show {:?}, the import shows {held}",
                        cell.display.as_deref().unwrap_or(""),
                    ));
                }
            }

            // 6b. And its formula — X2's half. A cell with an `excel` and no `formula` claims
            // the opposite: that this build carries no formula for it at all.
            let claim = format!("formula:{}!{}", fixture.sheets[*sheet].0, cell.address);
            let Some(outcome) = document
                .sheets
                .get(*sheet)
                .and_then(|s| cell.check_formula(s))
            else {
                continue;
            };
            formula_claims += 1;
            formulas_matching += usize::from(outcome.is_ok());
            reached.check(file, &claim, outcome.is_ok());
            if let Err(held) = outcome
                && !excused(file, &claim)
            {
                failures.push(format!(
                    "{file}: {claim} should be {:?}, the import holds {held}",
                    cell.formula.as_deref().unwrap_or("nothing"),
                ));
            }
        }

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

        // 4. The report, per kind. Over-counting is a bug at every milestone — claiming to
        // have dropped something the file does not contain is worse than not counting — so it
        // fails on its own, outside the under-count claim, and only a named table entry
        // excuses it. One does: `tables.xlsx` expects four structured references where the
        // fixture holds seven.
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
            // A kind the manifest does not mention is an implicit claim of zero, and this is
            // the only place it is evaluated — recorded, so a table entry can name it.
            if !fixture.expect_dropped.contains_key(&name) {
                reached.check(file, &format!("dropped:{name}"), false);
            }
            if *got > want && !excused(file, &format!("dropped:{name}")) {
                failures.push(format!(
                    "{file}: dropped {name}×{got} — the manifest expects ×{want}, and a \
                     report that over-counts is worse than one that stops counting"
                ));
            }
        }

        // 5. `mc:MustUnderstand`, and the functions a translated formula names.
        if !fixture.expect_must_understand.is_empty() {
            // By URI, exactly: `xml.rs` resolves each prefix while its declaration is in scope,
            // which is the spelling the manifest uses and the one a bug report can act on.
            let satisfied = report.must_understand == fixture.expect_must_understand;
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
    for (file, claim, why) in PENDING.iter().chain(UNSPELLABLE) {
        if reached
            .satisfied
            .contains(&((*file).to_owned(), (*claim).to_owned()))
        {
            failures.push(format!(
                "{file}: `{claim}` now passes — delete its PENDING/UNSPELLABLE entry ({why})"
            ));
        } else if !reached
            .unsatisfied
            .contains(&((*file).to_owned(), (*claim).to_owned()))
        {
            failures.push(format!(
                "{file}: PENDING/UNSPELLABLE names `{claim}`, which no fixture makes a claim \
                 about"
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
         {}/{total} claims satisfied, {} pending, {} unspellable, {} decided otherwise",
        fixtures.len(),
        fixtures.len() - imported,
        reached.satisfied.len(),
        PENDING.len(),
        UNSPELLABLE.len(),
        DECIDED_OTHERWISE.len(),
    );
    eprintln!(
        "  cells: {cells_matching}/{cell_claims} claims hold, {cells_carried} cells carried in all"
    );
    eprintln!("  formulas: {formulas_matching}/{formula_claims} claims hold");
    eprintln!("  displays: {displays_matching}/{display_claims} claims hold");
    for failure in failures.iter().take(200) {
        eprintln!("  {failure}");
    }
    if failures.len() > 200 {
        eprintln!("  ... and {} more", failures.len() - 200);
    }
    assert!(
        failures.is_empty(),
        "{} claims are neither satisfied nor named",
        failures.len()
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

/// One fixture, imported — for the tests below that ask about a particular cell.
fn import(file: &str) -> (grind_sheet::model::Document, grind_xlsx::Report) {
    let bytes = std::fs::read(root().join(file)).expect("a vendored fixture");
    grind_xlsx::import_bytes(&bytes).expect("it imports")
}

fn style_at(document: &grind_sheet::model::Document, at: &str) -> Option<CellStyle> {
    let pos = grind_xlsx::address::cell(at).expect("an address");
    document.sheets[0].style(pos).cloned()
}

/// `styles/colors.xlsx` states its own answer: column C is the colour each row's B cell should
/// be, where the manifest could state one without the theme arithmetic. Every stated colour is
/// held here — the four spellings, the palette, and all twelve theme slots, including the two
/// whose order is swapped — and the rows the manifest calls automatic come out with no colour.
/// The tinted rows state `—`, and `color.rs` holds those to the oracle's own values instead.
#[test]
fn the_colour_fixture_says_what_each_colour_is() {
    let (document, report) = import("styles/colors.xlsx");
    let sheet = &document.sheets[0];
    let mut checked = 0;
    for row in 1..30 {
        let label = sheet.get(Pos::new(row, 0));
        let want = sheet.get(Pos::new(row, 2));
        let got = sheet.style(Pos::new(row, 1)).and_then(|s| s.color.clone());
        let (CellValue::Text(label), CellValue::Text(want)) = (label, want) else {
            continue;
        };
        if want.starts_with('#') {
            assert_eq!(
                got.as_deref(),
                Some(want.as_str()),
                "B{} ({label})",
                row + 1
            );
            checked += 1;
        } else if ["auto", "indexed 64", "indexed 65"].contains(&label.as_str()) {
            assert_eq!(got, None, "B{} ({label}) is automatic", row + 1);
            checked += 1;
        }
    }
    // 21 rows state a hex; three more are the automatic ones.
    assert_eq!(checked, 24, "the fixture's stated rows");
    assert_eq!(report.dropped.get(&Dropped::ThemeColor), None);
}

/// What the style and geometry fixtures' own notes say each cell is, one assertion per note —
/// the manifest states these in prose rather than as claims, so they are written out here.
#[test]
fn the_style_fixtures_look_as_their_notes_say() {
    use grind_xlsx::Appearance;
    let style = |pairs: &[(&str, &str)]| {
        let mut s = CellStyle::default();
        for (key, value) in pairs {
            let value = Some((*value).to_owned());
            match *key {
                "weight" => s.font_weight = value,
                "slant" => s.font_style = value,
                "size" => s.font_size = value,
                "color" => s.color = value,
                "background" => s.background = value,
                "align" => s.align = value,
                "valign" => s.vertical_align = value,
                "wrap" => s.wrap = value,
                "border" => s.set_border(value),
                "left" => s.borders[0] = value,
                _ => unreachable!("{key}"),
            }
        }
        Some(s)
    };

    let (fonts, report) = import("styles/fonts.xlsx");
    assert_eq!(style_at(&fonts, "B2"), None, "the default font is no style");
    assert_eq!(style_at(&fonts, "B3"), style(&[("weight", "bold")]));
    assert_eq!(style_at(&fonts, "B4"), style(&[("slant", "italic")]));
    assert_eq!(style_at(&fonts, "B10"), style(&[("size", "8pt")]));
    assert_eq!(style_at(&fonts, "B11"), None, "11pt is the default size");
    assert_eq!(style_at(&fonts, "B13"), style(&[("size", "10.5pt")]));
    assert_eq!(
        style_at(&fonts, "B21"),
        style(&[("weight", "bold"), ("slant", "italic"), ("size", "14pt")])
    );
    assert_eq!(report.appearance_lost[&Appearance::Underline], 4);
    assert_eq!(report.appearance_lost[&Appearance::Strike], 2);
    assert_eq!(report.appearance_lost[&Appearance::Script], 2);

    let (fills, report) = import("styles/fills.xlsx");
    assert_eq!(style_at(&fills, "B2"), style(&[("background", "#ffff00")]));
    assert_eq!(
        style_at(&fills, "B5"),
        None,
        "a pattern is not its foreground"
    );
    // Seventeen two-colour patterns: the file's "eighteen" counts `none` among them, which is
    // B3 and is no fill at all.
    assert_eq!(report.appearance_lost[&Appearance::PatternFill], 17);
    assert_eq!(report.appearance_lost[&Appearance::GradientFill], 1);

    let (borders, report) = import("styles/borders.xlsx");
    assert_eq!(
        style_at(&borders, "B2"),
        style(&[("border", "0.74pt solid #000000")])
    );
    assert_eq!(
        style_at(&borders, "C2"),
        style(&[("left", "0.74pt solid #000000")]),
        "one side only"
    );
    assert_eq!(
        style_at(&borders, "B5"),
        style(&[("border", "1.76pt double #000000")])
    );
    assert_eq!(style_at(&borders, "B15"), None, "an explicit none");
    assert_eq!(report.appearance_lost[&Appearance::Diagonal], 3);
    // dashDot, dashDotDot, mediumDashDot, mediumDashDotDot and slantDashDot, twice each.
    assert_eq!(report.appearance_lost[&Appearance::BorderPattern], 10);

    let (alignment, report) = import("styles/alignment.xlsx");
    for (at, key, value) in [
        ("B3", "align", "start"),
        ("B4", "align", "center"),
        ("B5", "align", "end"),
        ("B7", "align", "justify"),
        ("B10", "valign", "top"),
        ("B11", "valign", "middle"),
        ("B12", "valign", "bottom"),
        ("B15", "wrap", "wrap"),
    ] {
        assert_eq!(style_at(&alignment, at), style(&[(key, value)]), "{at}");
    }
    assert_eq!(style_at(&alignment, "B2"), None, "general is no alignment");
    for class in [
        Appearance::Fill,
        Appearance::CenterAcross,
        Appearance::Distributed,
        Appearance::Shrink,
    ] {
        assert_eq!(report.appearance_lost[&class], 1, "{class:?}");
    }
    assert_eq!(report.appearance_lost[&Appearance::VerticalJustify], 2);
    assert_eq!(report.appearance_lost[&Appearance::Indent], 3);
    assert_eq!(report.appearance_lost[&Appearance::Rotation], 6);

    // `applyFont="0"` does not hand A5 its named style's font: the cell format's own ids are
    // the ones in effect, as the oracle reads it (`doc/xlsx-format.md` §4.4).
    let (named, _) = import("styles/named-styles.xlsx");
    assert_eq!(
        style_at(&named, "A1"),
        style(&[("weight", "bold"), ("size", "16pt"), ("color", "#1f4e79")])
    );
    assert_eq!(
        style_at(&named, "A2"),
        style(&[("slant", "italic"), ("color", "#808080")])
    );
    assert_eq!(style_at(&named, "A3"), None);
    assert_eq!(style_at(&named, "A5"), style(&[("weight", "bold")]));
}

/// The geometry fixtures, by their notes. Widths are ECMA-376's unit — see
/// `grind_xlsx::sheet::col_width` for why that is not the oracle's.
#[test]
fn the_geometry_fixtures_size_and_hide_what_their_notes_say() {
    use grind_xlsx::Appearance;
    use grind_xlsx::sheet::col_width;

    let (columns, report) = import("geometry/columns.xlsx");
    let sheet = &columns.sheets[0];
    assert_eq!(sheet.col_width(0), Some(col_width(4.0).as_str()));
    assert_eq!(sheet.col_width(4), Some(col_width(12.75).as_str()));
    assert_eq!(
        sheet.col_width(8),
        Some(col_width(12.75).as_str()),
        "E:I is one element"
    );
    // J is hidden at width 0, and a `<col>` that mentions it is not one the sheet's default
    // width fills in: hidden, with no width of its own.
    assert!(
        sheet.col_hidden(9) && sheet.col_width(9).is_none(),
        "J: hidden, width 0"
    );
    assert!(sheet.col_hidden(10), "K: hidden…");
    assert_eq!(
        sheet.col_width(10),
        Some(col_width(15.0).as_str()),
        "…and remembers"
    );
    // N:XFD is one element: carried as far as the sheet goes, and no further.
    assert_eq!(sheet.col_width(13), Some(col_width(8.43).as_str()));
    // A to N, less J.
    assert_eq!(sheet.col_widths().count(), 13);
    assert!(
        report.appearance_lost.is_empty(),
        "{:?}",
        report.appearance_lost
    );
    // Excel's default column: 64 pixels, which is two-thirds of an inch. The file stores
    // 9.140625 — 64/7 truncated to a 256th — so the width is 64 pixels to within 0.004mm.
    let default = grind_sheet::style::length_mm(&col_width(9.140625)).unwrap();
    assert!((default - 25.4 * 64.0 / 96.0).abs() < 0.01, "{default}");

    let (rows, report) = import("geometry/rows.xlsx");
    let sheet = &rows.sheets[0];
    assert_eq!(sheet.row_height(0), None, "no `ht`: the default");
    assert_eq!(sheet.row_height(1), Some("15pt"));
    assert_eq!(sheet.row_height(2), Some("7.5pt"));
    assert_eq!(sheet.row_height(4), Some("120.75pt"));
    assert!(sheet.row_manually_hidden(5) && sheet.row_height(5).is_none());
    assert!(sheet.row_manually_hidden(6));
    assert_eq!(sheet.row_height(6), Some("25pt"), "unhiding restores 25pt");
    assert!(sheet.row_manually_hidden(7), "ht=0 is invisible");
    assert_eq!(sheet.row_height(8), Some("409pt"));
    assert_eq!(
        style_at(&rows, "A10").and_then(|s| s.font_weight),
        Some("bold".to_owned()),
        "a row's style reaches a cell that names none"
    );
    assert_eq!(report.appearance_lost[&Appearance::ZeroSize], 1);

    let (outlines, report) = import("geometry/outlines.xlsx");
    let sheet = &outlines.sheets[0];
    assert!(sheet.row_manually_hidden(5) && sheet.row_manually_hidden(6));
    assert!(sheet.col_hidden(4));
    assert_eq!(report.appearance_lost[&Appearance::Outline], 1);

    let (_, report) = import("geometry/panes.xlsx");
    assert_eq!(
        report.appearance_lost.get(&Appearance::Pane),
        Some(&3),
        "two frozen and one split; the gridlines and the zoom are view state"
    );
}

/// The document fixtures, by their notes (X5): what `Document::names` holds, what the
/// autofilter keeps, which sheets were renamed, and that a merge moves nothing.
#[test]
fn the_document_fixtures_carry_what_their_notes_say() {
    use grind_xlsx::Appearance;
    use grind_xlsx::formula::Refusal;

    let (document, report) = import("document/defined-names.xlsx");
    let names = &document.names;
    assert_eq!(
        names.get("singlecell").map(String::as_str),
        Some("[Data.$A$1]")
    );
    assert_eq!(
        names.get("range").map(String::as_str),
        Some("[Data.$A$1:.$B$5]")
    );
    assert_eq!(names.get("constant").map(String::as_str), Some("42"));
    assert_eq!(
        names.get("crosssheet").map(String::as_str),
        Some("[Other.$A$1]")
    );
    assert!(
        !names.contains_key("rate"),
        "sheet-local: dropped, not flattened"
    );
    assert!(
        !names.keys().any(|k| k.starts_with("_xlnm")),
        "print settings are not the author's names: {names:?}"
    );
    // `Multi_Area` is two ranges joined by a comma — a union, which the Small Group leaves out.
    assert_eq!(
        report.names_lost,
        [("Multi_Area".to_owned(), Refusal::Union)]
    );
    assert_eq!(report.refused.get(&Refusal::SheetLocalName), Some(&1), "D3");

    let (document, report) = import("document/autofilter.xlsx");
    let filter = |i: usize| document.sheets[i].filter().expect("a filter").clone();
    let plain = filter(0);
    assert_eq!((plain.start, plain.end), (Pos::new(0, 0), Pos::new(8, 2)));
    assert!(plain.keep.is_empty(), "a range with no criteria");
    let criteria = filter(1);
    assert_eq!(
        criteria
            .keep
            .get(&0)
            .map(|v| v.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["north", "south"]),
        "the dropdown's checkboxes are the model's own vocabulary"
    );
    assert!(
        !criteria.keep.contains_key(&1),
        "a custom comparison is not"
    );
    assert_eq!(
        report.appearance_lost[&Appearance::FilterCriterion],
        2,
        "B's range, the top ten"
    );
    // The rows the carried filter hides are its to hide, not hidden by hand as well.
    let sheet = &document.sheets[1];
    let null = document.null_date;
    for row in sheet.hidden_rows(null) {
        assert!(!sheet.row_manually_hidden(row), "row {}", row + 1);
    }

    let (document, report) = import("document/sheets.xlsx");
    assert_eq!(
        report.renamed,
        [
            ("Has[Brackets]".to_owned(), "Has_Brackets_".to_owned()),
            ("Has/Slash".to_owned(), "Has_Slash".to_owned()),
        ]
    );
    assert!(!report.lossless(), "a rename is reported");
    assert_eq!(document.sheets.len(), 10);

    // A5:A7's top-left is empty, and stays empty: dropping a merge moves nothing.
    let (document, _) = import("document/merged-cells.xlsx");
    let sheet = &document.sheets[0];
    for row in 4..7 {
        assert_eq!(
            sheet.get(Pos::new(row, 0)),
            CellValue::Empty,
            "A{}",
            row + 1
        );
    }
}

/// Every imported document survives our own writer and reader.
///
/// The same identity check `corpus_read.rs` runs over LibreOffice's corpus and phase 3 owns
/// for ODF: import → write → read → compare. It proves the importer produced something **ODF
/// can actually express** rather than something that only lives in memory, and it is the
/// cheapest test in this file to keep honest as X1–X5 add content to carry. Since X1 that is
/// every cell's value and kind as well as the sheet list; since X4, every cell's style and
/// every column's and row's size and hidden-ness.
///
/// And no longer the cheapest to run: `scale/large-sheet.xlsx` makes it a 47 MB ODF write and
/// read, which takes about two minutes in a debug build against a second and a half in a
/// release one. Nothing here is wrong — the debug ODF reader is simply slow on half a million
/// cells — but it is the one test in the suite whose cost is a corpus file's size.
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
        // Every carried cell, value and kind — X1's contribution to this check. Walked over
        // the rows that carry anything rather than the used rectangle, which for
        // `scale/wide-and-sparse.xlsx` is the whole grid.
        for (sheet, read) in document.sheets.iter().zip(&back.sheets) {
            let cols = sheet.used_cols();
            let rows = sheet.rows_carrying().into_iter().flatten();
            let changed = rows
                .flat_map(|row| (0..cols).map(move |col| Pos::new(row, col)))
                .find(|&pos| (sheet.get(pos), sheet.kind(pos)) != (read.get(pos), read.kind(pos)));
            if let Some(pos) = changed {
                differences.push(format!(
                    "{}: {}!{} was {:?} {:?}, read back as {:?} {:?}",
                    fixture.file,
                    sheet.name,
                    grind_sheet::a1::format(None, pos),
                    sheet.get(pos),
                    sheet.kind(pos),
                    read.get(pos),
                    read.kind(pos)
                ));
            }
            // X4's: every cell's style, exactly, and every track's size and hidden-ness.
            let styled = sheet.rows_carrying().into_iter().flatten();
            if let Some(pos) = styled
                .flat_map(|row| (0..cols).map(move |col| Pos::new(row, col)))
                .find(|&pos| sheet.style(pos) != read.style(pos))
            {
                differences.push(format!(
                    "{}: {}!{} was styled {:?}, read back as {:?}",
                    fixture.file,
                    sheet.name,
                    grind_sheet::a1::format(None, pos),
                    sheet.style(pos),
                    read.style(pos)
                ));
            }
            let tracks = |s: &grind_sheet::model::Sheet| {
                (
                    s.col_widths()
                        .map(|(c, w)| (c, w.to_owned()))
                        .collect::<Vec<_>>(),
                    s.row_heights()
                        .map(|(r, h)| (r, h.to_owned()))
                        .collect::<Vec<_>>(),
                    s.hidden_cols().collect::<Vec<_>>(),
                    s.manually_hidden_rows().collect::<Vec<_>>(),
                )
            };
            if tracks(sheet) != tracks(read) {
                differences.push(format!(
                    "{}: {}'s tracks were {:?}, read back as {:?}",
                    fixture.file,
                    sheet.name,
                    tracks(sheet),
                    tracks(read)
                ));
            }
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

        // `as_chunks` rather than `chunks_exact`: the block and word sizes are constants, so
        // the chunk type carries them and neither loop needs a `try_into` that cannot fail.
        for block in message.as_chunks::<64>().0 {
            let mut w = [0u32; 64];
            for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
                w[i] = u32::from_be_bytes(*word);
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
