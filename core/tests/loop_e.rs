// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop E — the generated differential against LibreOffice (`doc/differential-fuzz.md`).
//!
//! Loop B compares our evaluator against the arguments somebody wrote a fixture for. This
//! one generates the arguments instead: a formula per catalog signature, filled from a pool
//! of deliberately awkward values (`-1`, `""`, an empty cell, text that looks like a number,
//! a range holding an error), evaluated by us and by LibreOffice, compared cell for cell.
//!
//! The harness emits the flat XML itself rather than going through our writer, so a writer
//! bug cannot show up here as an evaluator bug. Formula cells are shipped with **no** cached
//! value: LO has nothing to load for them and so writes its own answer back, which is the
//! oracle. An error comes back as `calcext:value-type="error"` with the name in the display
//! paragraph (`#DIV/0!`), or as one of LO's internal `Err:NNN` codes, which §5.12 does not
//! name — [`agrees`] treats those as "some error", exactly as loop B does.
//!
//! Deterministic: the same seed is the same document. Needs `soffice` on `PATH`; skips with
//! a notice without one.
//!
//!     cargo test --test loop_e
//!     SHEET_LOOP_E_DUMP=1 cargo test --test loop_e -- --nocapture
//!     SHEET_FUZZ_SEED=12345 SHEET_LOOP_E_FORMULAS=5000 cargo test --test loop_e

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use sheet_core::formula::eval::{Address, Engine};
use sheet_core::formula::funcs;
use sheet_core::formula::value::{FormulaError, Value};
use sheet_core::{CellValue, Pos};

/// Formulas that must keep agreeing with LibreOffice. Raise it when the scoreboard rises;
/// never lower it. Meaningful only at the default seed and default count — a run with either
/// overridden prints its scoreboard and skips the ratchet.
const FLOOR: usize = 912;

const DEFAULT_SEED: u64 = 0x5EED;
const DEFAULT_FORMULAS: usize = 1000;

/// Not a function of the document, so no two evaluators agree by construction.
const VOLATILE: [&str; 2] = ["NOW", "TODAY"];

/// First generated formula, 0-based. Rows 0..8 are the data block; the gap is so a mistake
/// in the emitter shows up as a hole rather than as a silently shifted reference.
const FIRST_ROW: u32 = 11;

// --- randomness -------------------------------------------------------------------------

/// SplitMix64. Twenty lines for what a dependency would do, and the reason a disagreement
/// replays exactly from `SHEET_FUZZ_SEED`.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
        from[self.below(from.len())]
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

// --- the value pools --------------------------------------------------------------------

/// Numbers, and the references that are numbers by coercion rather than by type: an empty
/// cell, text that looks like one, a logical, an error.
const NUMBERS: &[&str] = &[
    "0",
    "1",
    "-1",
    "2.5",
    "-3.7",
    "0.5",
    "10",
    "1e15",
    "1e-15",
    "-0.0000001",
    "[.A2]",
    "[.A4]",
    "[.A6]",
    "[.A7]",
    "[.B2]",
    "[.B3]",
    "[.C2]",
    "[.E2]",
    "[.F4]",
];
const INTEGERS: &[&str] = &[
    "0", "1", "-1", "2", "3", "10", "-2", "1.7", "-1.7", "[.A2]", "[.A7]", "[.B3]", "[.E2]",
];
const TEXTS: &[&str] = &[
    "\"\"",
    "\"abc\"",
    "\"12\"",
    "\"Grüße\"",
    "\" x \"",
    "\"a b c\"",
    "[.B2]",
    "[.B4]",
    "[.B3]",
    "[.A2]",
    "[.A7]",
    "[.C2]",
    "[.E2]",
];
const LOGICALS: &[&str] = &[
    "TRUE()", "FALSE()", "1", "0", "-1", "[.C2]", "[.C3]", "[.B2]", "[.A7]", "[.E2]",
];
const ANY: &[&str] = &[
    "0", "-1", "2.5", "\"\"", "\"abc\"", "\"12\"", "TRUE()", "[.A2]", "[.A7]", "[.B2]", "[.C2]",
    "[.D2]", "[.E2]", "[.F4]",
];
/// Ranges chosen so most of them span more than one type — a column of numbers with a hole,
/// a column of text, a block covering an error, a single cell where a range is expected.
const RANGES: &[&str] = &[
    "[.A2:.A8]",
    "[.A2:.C8]",
    "[.B2:.B8]",
    "[.C2:.C8]",
    "[.D2:.D8]",
    "[.E2:.E8]",
    "[.A2:.F8]",
    "[.A2:.A4]",
    "[.F2:.F8]",
    "[.A2]",
    "[.A7]",
];
const DATABASES: &[&str] = &["[.A1:.F8]", "[.A1:.C8]", "[.A1:.A8]"];
const FIELDS: &[&str] = &["1", "2", "0", "7", "\"num\"", "\"txt\"", "\"nope\""];
const CRITERIA: &[&str] = &["[.H1:.H2]", "[.A1:.A2]", "[.A1:.F2]"];
const CRITERION: &[&str] = &["1", "\">1\"", "\"abc\"", "0", "\"<>2\"", "[.A2]", "\"\""];
const DATES: &[&str] = &[
    "[.D2]", "[.D3]", "[.A2]", "[.A7]", "44000", "0", "1", "-1", "2.5", "60", "[.B3]",
];
const TIMES: &[&str] = &[
    "0", "0.5", "1.25", "-0.25", "[.D2]", "[.A2]", "[.A7]", "[.B3]",
];

/// The pool for a signature's type word. An unknown one falls back to [`ANY`] rather than
/// dropping the function: a new catalog entry then gets *some* coverage without touching
/// this table.
fn pool(ty: &str) -> &'static [&'static str] {
    match ty.to_ascii_lowercase().as_str() {
        "number" | "complex" | "basis" => NUMBERS,
        "integer" => INTEGERS,
        "text" => TEXTS,
        "logical" => LOGICALS,
        "reference" | "referencelist" | "array" | "numbersequence" | "numbersequencelist" => RANGES,
        "database" => DATABASES,
        "field" => FIELDS,
        "criteria" => CRITERIA,
        "criterion" => CRITERION,
        "dateparam" => DATES,
        "timeparam" => TIMES,
        _ => ANY,
    }
}

// --- signatures -------------------------------------------------------------------------

/// One argument position, as the spec's `Syntax:` line describes it.
struct Slot {
    /// The type word the pool is chosen by — the first word of the slot, `ForceArray` and
    /// the like skipped.
    ty: String,
    /// Inside a `[ … ]` group.
    optional: bool,
    /// A `{ … } +` group: one or more.
    repeat: bool,
}

/// Split a `Syntax:` line into argument slots.
///
/// The grammar is small enough to read with a bracket counter: `[ … ]` is optional, `{ … } +`
/// repeats, `;` at any depth starts the next slot, and the slot's first word is its type
/// (`Logical|NumberSequenceList` and `ReferenceList | Array` both start with a usable one).
/// Default values (`= 2`) are noise here and everything after `=` is dropped.
fn slots(signature: &str) -> Vec<Slot> {
    let Some(open) = signature.find('(') else {
        return Vec::new();
    };
    let Some(close) = signature.rfind(')') else {
        return Vec::new();
    };
    let spaced = signature[open + 1..close]
        .replace('[', " [ ")
        .replace(']', " ] ")
        .replace('{', " { ")
        .replace('}', " } ")
        .replace(';', " ; ");

    let mut out: Vec<Slot> = Vec::new();
    let mut depth = 0i32;
    let mut started = false;
    let mut skipping = false;
    for word in spaced.split_whitespace() {
        match word {
            "[" => depth += 1,
            "]" => depth -= 1,
            "{" => {}
            "}" => {}
            "+" => {
                if let Some(last) = out.last_mut() {
                    last.repeat = true;
                }
            }
            ";" => {
                started = false;
                skipping = false;
            }
            "=" => skipping = true,
            "|" | "ForceArray" => {}
            _ if skipping => {}
            _ if !started => {
                started = true;
                out.push(Slot {
                    // `Reference|Array` is one alternative set; the first is the one
                    // the pool is chosen by.
                    ty: word.split('|').next().unwrap_or(word).to_owned(),
                    optional: depth > 0,
                    repeat: false,
                });
            }
            _ => {}
        }
    }
    out
}

/// One generated call, in ODF syntax.
fn call(name: &str, slots: &[Slot], rng: &mut Rng) -> String {
    let mut args: Vec<String> = Vec::new();
    let mut stop = false;
    for slot in slots {
        // Optional arguments are dropped from the right: an evaluator does not accept a hole
        // followed by a value, and once one is skipped the rest have to go too.
        if slot.optional && (stop || !rng.chance(60)) {
            stop = true;
            continue;
        }
        let times = if slot.repeat { 1 + rng.below(3) } else { 1 };
        for _ in 0..times {
            args.push(rng.pick(pool(&slot.ty)).to_owned());
        }
    }
    format!("{name}({})", args.join(";"))
}

/// Every formula for one run, paired with the function it came from.
fn formulas(seed: u64, count: usize) -> Vec<(&'static str, String)> {
    let catalog: Vec<_> = funcs::catalog()
        .iter()
        .filter(|info| !VOLATILE.contains(&info.name))
        .collect();
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(count);
    // Round-robin rather than random choice, so every function appears in every run and the
    // count per function does not itself depend on the seed.
    for i in 0..count {
        let info = catalog[i % catalog.len()];
        out.push((info.name, call(info.name, &slots(info.signature), &mut rng)));
    }
    out
}

// --- the document -----------------------------------------------------------------------

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One data-block cell, spelled as ODF rather than as a model value: this file is the
/// harness's own, not the writer's.
enum Datum {
    Empty,
    Num(&'static str),
    Text(&'static str),
    Bool(bool),
    Date(&'static str),
    Formula(&'static str),
}

impl Datum {
    fn xml(&self) -> String {
        match self {
            Datum::Empty => "<table:table-cell/>".into(),
            Datum::Num(n) => {
                format!("<table:table-cell office:value-type=\"float\" office:value=\"{n}\"/>")
            }
            Datum::Text(t) => format!(
                "<table:table-cell office:value-type=\"string\"><text:p>{}</text:p>\
                 </table:table-cell>",
                escape(t)
            ),
            Datum::Bool(b) => format!(
                "<table:table-cell office:value-type=\"boolean\" office:boolean-value=\"{b}\"/>"
            ),
            Datum::Date(d) => {
                format!("<table:table-cell office:value-type=\"date\" office:date-value=\"{d}\"/>")
            }
            Datum::Formula(f) => format!("<table:table-cell table:formula=\"of:={}\"/>", escape(f)),
        }
    }
}

/// The values every generated formula points at: numbers including a huge one and a zero,
/// text including the empty string and text that looks like a number, logicals, dates around
/// the epoch, an error, and empty cells — which are their own semantics in §6.3 and the most
/// common source of divergence.
///
/// Columns A–F are the data, H1:H2 is a criteria range for the `D…` family. Row 1 is the
/// header row those functions need.
fn data() -> Vec<Vec<Datum>> {
    use Datum::*;
    vec![
        vec![
            Text("num"),
            Text("txt"),
            Text("log"),
            Text("date"),
            Text("err"),
            Text("mix"),
            Empty,
            Text("num"),
        ],
        vec![
            Num("1"),
            Text("abc"),
            Bool(true),
            Date("2026-08-19"),
            Formula("1/0"),
            Num("3"),
            Empty,
            Text(">1"),
        ],
        vec![
            Num("2.5"),
            Text("12"),
            Bool(false),
            Date("1899-12-31"),
            Num("1"),
            Text("abc"),
        ],
        vec![
            Num("-3"),
            Text(""),
            Bool(true),
            Date("1900-03-01"),
            Num("2"),
            Empty,
        ],
        vec![
            Num("0"),
            Text("x y"),
            Bool(true),
            Date("1983-01-31"),
            Num("3"),
            Bool(true),
        ],
        vec![
            Num("1e15"),
            Text("0"),
            Bool(false),
            Date("2000-02-29"),
            Num("4"),
            Num("-0.5"),
        ],
        vec![
            Empty,
            Text("TRUE"),
            Bool(true),
            Date("2026-01-01"),
            Num("5"),
            Text("7"),
        ],
        vec![
            Num("7"),
            Text("-1"),
            Bool(false),
            Date("1970-01-01"),
            Num("6"),
            Empty,
        ],
    ]
}

/// The whole document, as flat XML. The `of:` namespace declaration is load-bearing: without
/// it LibreOffice reads `of:=…` as an unprefixed formula and hands back `of:=of:=…`.
fn document(formulas: &[(&'static str, String)]) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <office:document \
         xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
         xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
         xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
         xmlns:of=\"urn:oasis:names:tc:opendocument:xmlns:of:1.2\" \
         xmlns:calcext=\"urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0\" \
         office:version=\"1.3\" \
         office:mimetype=\"application/vnd.oasis.opendocument.spreadsheet\">\
         <office:body><office:spreadsheet><table:table table:name=\"Fuzz\">",
    );
    for row in data() {
        xml.push_str("<table:table-row>");
        for cell in row {
            xml.push_str(&cell.xml());
        }
        xml.push_str("</table:table-row>");
    }
    let gap = FIRST_ROW - data().len() as u32;
    xml.push_str(&format!(
        "<table:table-row table:number-rows-repeated=\"{gap}\"><table:table-cell/>\
         </table:table-row>"
    ));
    for (_, formula) in formulas {
        xml.push_str(&format!(
            "<table:table-row><table:table-cell table:formula=\"of:={}\"/></table:table-row>",
            escape(formula)
        ));
    }
    xml.push_str("</table:table></office:spreadsheet></office:body></office:document>\n");
    xml
}

// --- driving LibreOffice ----------------------------------------------------------------

fn have_soffice() -> bool {
    Command::new("soffice")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A scratch directory that cleans itself up. The private `UserInstallation` profile is not
/// optional — without it this fights the developer's own running LibreOffice for the profile
/// lock and either blocks or silently does nothing (the same rule as loop C).
struct Lab {
    dir: PathBuf,
}

impl Lab {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("sheet-loop-e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("out")).unwrap();
        Self { dir }
    }

    fn convert(&self, xml: &str) -> PathBuf {
        let input = self.dir.join("fuzz.fods");
        std::fs::write(&input, xml).unwrap();
        let out = self.dir.join("out");
        let status = Command::new("soffice")
            .arg("--headless")
            .arg(format!(
                "-env:UserInstallation=file://{}",
                self.dir.join("profile").display()
            ))
            .args(["--convert-to", "fods", "--outdir"])
            .arg(&out)
            .arg(&input)
            .status()
            .expect("soffice failed to start");
        assert!(status.success(), "soffice exited with {status}");
        let path = out.join("fuzz.fods");
        assert!(path.exists(), "LibreOffice could not open what we wrote");
        path
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// --- comparison -------------------------------------------------------------------------

/// Loop B's rule, and for the same reasons: 15 significant digits is all LibreOffice writes,
/// an error's identity survives only as its display name, and `Err:NNN` has no name in §5.12
/// so any error agrees with it.
fn agrees(stored: &CellValue, computed: &Value) -> bool {
    match (stored, computed) {
        (CellValue::Number(x), Value::Number(y)) => {
            x == y || (x - y).abs() <= 1e-14 * x.abs().max(y.abs())
        }
        (CellValue::Text(s), Value::Error(e)) => {
            FormulaError::from_name(s) == Some(*e) || s.starts_with("Err:")
        }
        (CellValue::Empty, Value::Text(s)) => s.is_empty(),
        (CellValue::Text(s), Value::Text(t)) => s == t,
        (CellValue::Bool(a), Value::Bool(b)) => a == b,
        (CellValue::Number(x), Value::Bool(b)) => *x == u8::from(*b) as f64,
        (CellValue::Empty, Value::Empty) => true,
        _ => false,
    }
}

#[derive(Default)]
struct Tally {
    matched: usize,
    wrong: usize,
}

#[test]
fn generated_formulas_agree_with_libreoffice() {
    if !have_soffice() {
        eprintln!("skipping loop E: no soffice on PATH");
        return;
    }
    let seed = std::env::var("SHEET_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SEED);
    let count = std::env::var("SHEET_LOOP_E_FORMULAS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_FORMULAS);
    let dump = std::env::var("SHEET_LOOP_E_DUMP").is_ok();

    let formulas = formulas(seed, count);
    let xml = document(&formulas);

    let lab = Lab::new();
    let theirs = sheet_core::read_file(&lab.convert(&xml)).expect("re-reading LO's output");
    // Our side reads the *same* file, so nothing here depends on the writer.
    let ours =
        sheet_core::read_bytes("fuzz.fods", xml.as_bytes()).expect("reading our own document");
    let mut engine = Engine::new(&ours);
    let sheet = &theirs.sheets[0];

    let mut totals = Tally::default();
    let mut per_function: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut disagreements = Vec::new();

    for (i, (name, formula)) in formulas.iter().enumerate() {
        let pos = Pos::new(FIRST_ROW + i as u32, 0);
        let stored = sheet.get(pos);
        let computed = engine.value(Address::new(0, pos));
        let tally = per_function.entry(name).or_default();
        if agrees(&stored, &computed) {
            tally.matched += 1;
            totals.matched += 1;
        } else {
            tally.wrong += 1;
            totals.wrong += 1;
            disagreements.push(format!("{formula} — LO {stored:?}, ours {computed:?}"));
        }
    }

    eprintln!(
        "loop E: seed {seed}, {count} formulas — {} match, {} disagree",
        totals.matched, totals.wrong
    );
    for (name, tally) in per_function.iter().filter(|(_, t)| t.wrong > 0) {
        eprintln!(
            "  {name}: {} match, {} disagree",
            tally.matched, tally.wrong
        );
    }
    let shown = if dump { disagreements.len() } else { 20 };
    for line in disagreements.iter().take(shown) {
        eprintln!("    {line}");
    }

    // A run with either knob turned is a fishing trip, not the ratchet: the floor is a
    // statement about one specific document.
    if seed == DEFAULT_SEED && count == DEFAULT_FORMULAS {
        assert!(
            totals.matched >= FLOOR,
            "loop E: {} formulas match LibreOffice, below the floor of {FLOOR}",
            totals.matched
        );
    }
}

#[test]
fn signatures_parse_into_the_arguments_they_describe() {
    let count = |s: &str| slots(s).len();
    assert_eq!(count("PI()"), 0);
    assert_eq!(count("ABS( Number N )"), 1);
    assert_eq!(count("ATAN2( Number x ; Number y )"), 2);
    assert_eq!(count("LEFT( Text T [ ; Integer Length ] )"), 2);
    assert_eq!(
        count("IF( Logical Condition [ ; [ Any IfTrue ] [ ; [ Any IfFalse ] ] ] )"),
        3
    );
    assert_eq!(count("DCOUNT( Database D ; [ Field F ] ; Criteria C )"), 3);

    let left = slots("LEFT( Text T [ ; Integer Length ] )");
    assert_eq!(left[0].ty, "Text");
    assert!(!left[0].optional);
    assert_eq!(left[1].ty, "Integer");
    assert!(left[1].optional);

    // `{ … } +` is the repeated argument, and the default value in `RangeLookup = TRUE` is
    // not one.
    let sum = slots("SUM( { NumberSequenceList N } + )");
    assert_eq!(sum.len(), 1);
    assert!(sum[0].repeat);
    let vlookup = slots(
        "VLOOKUP( Any Lookup ; ForceArray Reference|Array DataSource ; Integer Column \
         [ ; Logical RangeLookup = TRUE() ] )",
    );
    assert_eq!(vlookup.len(), 4);
    assert_eq!(vlookup[1].ty, "Reference");
    assert_eq!(vlookup[3].ty, "Logical");

    // Every catalog entry has to yield slots the generator can fill, or a function is
    // silently being called with nothing.
    for info in funcs::catalog() {
        let parsed = slots(info.signature);
        let commas = info.signature.matches(';').count();
        assert!(
            parsed.len() >= commas + usize::from(commas > 0),
            "{}: {commas} separators, {} slots",
            info.name,
            parsed.len()
        );
    }
}
