// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop D's machinery, shared by its two oracles (`loop_d.rs`, LibreOffice;
//! `loop_d_euro_office.rs`, Euro-Office): the cell, style and track comparison, and the two
//! runs over the vendored corpus and over a directory from elsewhere. What differs per oracle is
//! how it is invoked and its list of [`Divergence`]s.

// Each test binary uses its own part of this module.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use grind_sheet::model::{CellValue, Document, NumberKind, Pos, Sheet};
use grind_sheet::style::{self, CellStyle};

/// A construct on which an oracle's conversion is known to differ from ours, with the
/// evidence. Each is a predicate over one difference, and each must still match at least one
/// in the corpus — the day it does not, the entry is deleted. **First match wins**, so a
/// narrow entry goes before a broad one.
///
/// Every entry was *looked at* before it was written: the oracle's output read for the
/// cell or sheet in question, and the fixture's own bytes beside it. None is a tolerance.
pub struct Divergence {
    pub name: &'static str,
    pub why: &'static str,
    pub scope: Scope,
}

pub enum Scope {
    /// One cell both sides have a sheet for.
    Cell(fn(ours: &Cell, theirs: &Cell) -> bool),
    /// One cell's **style**, where both sides carry one — its own scope, so a divergence about
    /// values cannot swallow a style difference on the same cell.
    Style(fn(ours: &CellStyle, theirs: &CellStyle) -> bool),
    /// One column or row of a sheet both sides have.
    Track(fn(&Track) -> bool),
    /// A sheet of ours the oracle's conversion lacks, by the name **the workbook** gave it — the
    /// import may have renamed it, and the construct is the spelling.
    OnlyOurs(fn(name: &str) -> bool),
    /// A sheet of the oracle's, by name, that ours does not have.
    OnlyTheirs(fn(name: &str) -> bool),
    /// A sheet both have, which the oracle's conversion holds nothing in and ours does.
    Unreached,
}

/// One side's view of one cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub value: CellValue,
    pub kind: Option<NumberKind>,
    /// Whether the cell holds a formula — ours except where a named class stopped it, the oracle's
    /// wherever the workbook had one. Read to *exclude* such cells from the value comparison: the
    /// oracle recalculates them and this import does not.
    pub formula: bool,
    pub formatted: bool,
    pub display: String,
    /// The cell's style after [`normalise`] — `None` where it looks like the document's
    /// default. Compared only where both sides carry a value: the ODF reader drops a styled
    /// empty cell (its `TODO:`), so the oracle's blanks arrive unstyled whatever it wrote.
    pub style: Option<CellStyle>,
}

pub fn cell(sheet: &Sheet, pos: Pos, null_date: i64, defaults: &Defaults) -> Cell {
    let value = sheet.get(pos);
    Cell {
        style: match value {
            CellValue::Empty => None,
            _ => normalise(sheet.style(pos), defaults),
        },
        value,
        kind: sheet.kind(pos),
        formula: sheet.formula(pos).is_some(),
        formatted: sheet.format(pos).is_some(),
        display: grind_sheet::render(sheet, pos, null_date),
    }
}

/// What a document's default cell looks like, as far as the comparison needs: the size and
/// colour its `Default` cell style carries. Ours has none — the model has no document default
/// (`odf/read.rs`'s ponytail on `style:default-style`) — and the oracle's is read out of its
/// output here, because the oracle **spells the default out on every automatic style**
/// (`fo:font-size="11pt"` on each of them, measured 2026-09-21), and a size equal to the
/// document's own default is the absence of one.
#[derive(Default)]
pub struct Defaults {
    pub size: Option<String>,
    pub color: Option<String>,
}

impl Defaults {
    /// The document's default cell style's attributes, read out of the oracle's text. A
    /// substring search rather than a parse, because our own reader deliberately does not keep
    /// the style. LibreOffice names it `Default`, Euro-Office `Normal`, in either attribute order.
    pub fn of(text: &str) -> Self {
        // What the named default leaves unsaid, the default style says.
        let fallback = Self::in_element(text, "<style:default-style ", |_| true);
        let named = |name: &'static str| {
            Self::in_element(text, "<style:style ", move |open| {
                open.contains(&format!(r#" style:name="{name}""#))
            })
        };
        let own = [named("Default"), named("Normal")]
            .into_iter()
            .find(|d| d.size.is_some() || d.color.is_some())
            .unwrap_or_default();
        Defaults {
            size: own.size.or(fallback.size),
            color: own.color.or(fallback.color),
        }
    }

    /// The first `tag` element of the table-cell family whose opening tag `matches`.
    fn in_element(text: &str, tag: &str, matches: impl Fn(&str) -> bool) -> Self {
        let mut rest = text;
        while let Some(start) = rest.find(tag) {
            let element = &rest[start..];
            let open = &element[..element.find('>').unwrap_or(element.len())];
            rest = &element[tag.len()..];
            if !open.contains(r#" style:family="table-cell""#) || !matches(open) {
                continue;
            }
            // A self-closed `<style:style …/>` carries nothing, and reading on to the next
            // `</style:style>` would take another style's attributes for this one's.
            if open.ends_with('/') {
                return Defaults::default();
            }
            let element = &element[..element.find("</style:style>").unwrap_or(element.len())];
            let attr = |name: &str| {
                let at = element.find(&format!(" {name}=\""))? + name.len() + 3;
                Some(element[at..at + element[at..].find('"')?].to_owned())
            };
            return Defaults {
                size: attr("fo:font-size"),
                color: attr("fo:color"),
            };
        }
        Defaults::default()
    }
}

/// A style with every property that says "the default" taken out, so that the two sides are
/// compared on what they *mean*. The oracle writes `fo:font-weight="normal"`,
/// `fo:font-style="normal"`, `fo:wrap-option="no-wrap"` and `style:vertical-align="bottom"`
/// onto styles that set none of them, where this import writes nothing; both are the same cell.
pub fn normalise(style: Option<&CellStyle>, defaults: &Defaults) -> Option<CellStyle> {
    let mut s = style?.clone();
    // The schema's `color` is `#[0-9a-fA-F]{6}`, either case; Euro-Office writes upper case.
    for v in [&mut s.color, &mut s.background]
        .into_iter()
        .chain(s.borders.iter_mut())
        .flatten()
    {
        *v = v.to_ascii_lowercase();
    }
    let clear = |field: &mut Option<String>, plain: &[&str]| {
        if field.as_deref().is_some_and(|v| plain.contains(&v)) {
            *field = None;
        }
    };
    clear(&mut s.font_weight, &["normal"]);
    clear(&mut s.font_style, &["normal"]);
    clear(&mut s.wrap, &["no-wrap"]);
    clear(&mut s.vertical_align, &["bottom", "automatic"]);
    clear(&mut s.background, &["transparent"]);
    if s.font_size.is_some() && s.font_size == defaults.size {
        s.font_size = None;
    }
    if s.color.is_some()
        && s.color.as_deref().map(str::to_ascii_lowercase)
            == defaults.color.as_deref().map(str::to_ascii_lowercase)
    {
        s.color = None;
    }
    (!s.is_plain()).then_some(s)
}

/// Loop C's rule for two styles (`sheet/tests/roundtrip.rs`'s `same_style`): a border's width
/// numerically, since LibreOffice re-quantises it, and everything else exactly.
pub fn same_style(a: &Option<CellStyle>, b: &Option<CellStyle>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return a.is_none() && b.is_none();
    };
    let borders = a.borders.iter().zip(&b.borders).all(|(a, b)| {
        match (
            a.as_deref().and_then(style::border_parts),
            b.as_deref().and_then(style::border_parts),
        ) {
            (Some((wa, sa, ca)), Some((wb, sb, cb))) => {
                (wa - wb).abs() < 0.05 && sa == sb && ca == cb
            }
            _ => a == b,
        }
    });
    // An indent is a length the oracle spells in the converting machine's unit (`0.1in` for
    // `0.265cm`), so it is compared as one.
    // A tenth of a millimetre either way: the same step converted on a cm machine came back
    // `0.265cm` and on an inch machine `0.1in`, which is 0.11mm apart.
    let indents = match (a.indent_mm(), b.indent_mm()) {
        (Some(x), Some(y)) => (x - y).abs() < 0.15,
        (x, y) => x == y,
    };
    let bare = |s: &CellStyle| CellStyle {
        borders: Default::default(),
        indent: None,
        ..s.clone()
    };
    borders && indents && bare(a) == bare(b)
}

/// Are these the same style except where `colour` excuses a border colour — with the oracle's
/// own line names read as the XSL-FO ones this import writes? The divergences about borders
/// are built on it, so that each excuses exactly one thing.
pub fn same_but(ours: &CellStyle, theirs: &CellStyle, colour: fn(&str, &str) -> bool) -> bool {
    fn xsl(line: &str) -> &str {
        match line {
            "fine-dashed" | "dash-dot" | "dash-dot-dot" => "dashed",
            "double-thin" => "double",
            other => other,
        }
    }
    let edges = ours.borders.iter().zip(&theirs.borders).all(|(a, b)| {
        match (
            a.as_deref().and_then(style::border_parts),
            b.as_deref().and_then(style::border_parts),
        ) {
            (Some((wa, la, ca)), Some((wb, lb, cb))) => {
                (wa - wb).abs() < 0.05 && la == xsl(lb) && colour(ca, cb)
            }
            _ => a == b,
        }
    });
    let bare = |s: &CellStyle| CellStyle {
        borders: Default::default(),
        ..s.clone()
    };
    edges && bare(ours) == bare(theirs)
}

/// One column or row on which the two conversions disagree.
#[derive(Debug)]
pub struct Track {
    pub axis: Axis,
    pub index: u32,
    pub what: TrackDifference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Column,
    Row,
}

#[derive(Debug)]
pub enum TrackDifference {
    /// A size ours carries, in millimetres, against the oracle's — which it has for every
    /// track, since it measures the ones nobody sized.
    Size {
        ours: f64,
        theirs: Option<f64>,
    },
    Hidden {
        ours: bool,
        theirs: bool,
    },
}

/// Loop C's rule, restated: equal at 15 significant digits (`sheet/tests/roundtrip.rs`'s
/// `same`, which has the arithmetic for why 1e-14 is that), everything else exactly.
pub fn same(a: &Cell, b: &Cell) -> bool {
    let values = same_value(&a.value, &b.value);
    // The display is compared only where **both** sides format the cell: an unformatted one had
    // its code refused by a named class, asserted in `ooxmlgen.rs`, and the oracle would say the
    // same in a worse vocabulary — `"3.75" vs "3 3/4"` rather than `Fraction`.
    let display = !(a.formatted && b.formatted) || a.display == b.display;
    values && a.kind == b.kind && display
}

/// Two values at 15 significant digits, anything but a number exactly.
pub fn same_value(a: &CellValue, b: &CellValue) -> bool {
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => {
            x == y || (x - y).abs() <= 1e-14 * x.abs().max(y.abs())
        }
        (x, y) => x == y,
    }
}

/// One way the two conversions differ.
pub enum Difference {
    Cell(String, Pos, Cell, Cell),
    Style(String, Pos, Option<CellStyle>, Option<CellStyle>),
    Track(String, Track),
    OnlyOurs(String),
    OnlyTheirs(String),
    Unreached(String),
}

impl Difference {
    pub fn explained_by(&self, divergence: &Divergence) -> bool {
        match (self, &divergence.scope) {
            (Difference::Cell(_, _, a, b), Scope::Cell(applies)) => applies(a, b),
            (Difference::Style(_, _, a, b), Scope::Style(applies)) => {
                let plain = CellStyle::default();
                applies(a.as_ref().unwrap_or(&plain), b.as_ref().unwrap_or(&plain))
            }
            (Difference::Track(_, track), Scope::Track(applies)) => applies(track),
            (Difference::OnlyOurs(name), Scope::OnlyOurs(applies)) => applies(name),
            (Difference::OnlyTheirs(name), Scope::OnlyTheirs(applies)) => applies(name),
            (Difference::Unreached(_), Scope::Unreached) => true,
            _ => false,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Difference::Cell(sheet, pos, a, b) => format!(
                "{sheet}!{} is {:?} {:?} shown {:?}, the oracle's {:?} {:?} shown {:?}",
                grind_sheet::a1::format(None, *pos),
                a.value,
                a.kind,
                a.display,
                b.value,
                b.kind,
                b.display
            ),
            Difference::Style(sheet, pos, a, b) => format!(
                "{sheet}!{} is styled {a:?}, the oracle's {b:?}",
                grind_sheet::a1::format(None, *pos),
            ),
            Difference::Track(sheet, track) => format!(
                "{sheet}: {} {} is {:?}",
                match track.axis {
                    Axis::Column => "column",
                    Axis::Row => "row",
                },
                track.index + 1,
                track.what
            ),
            Difference::OnlyOurs(name) => format!("sheet {name:?} is not in the oracle's"),
            Difference::OnlyTheirs(name) => format!("sheet {name:?} is only in the oracle's"),
            Difference::Unreached(name) => {
                format!("sheet {name:?} holds nothing in the oracle's and cells in ours")
            }
        }
    }
}

/// Every way the two documents disagree. Sheets are matched **by name** — the oracle drops
/// some and invents others, and matching by position would then compare the wrong pairs.
///
/// Cells are walked over the rows either side *carries* rather than over the used rectangle,
/// which for `scale/wide-and-sparse.xlsx` is the whole grid — seventeen billion cells.
pub fn differences(
    ours: &Document,
    theirs: &Document,
    defaults: &Defaults,
    renamed: &[(String, String)],
) -> Vec<Difference> {
    differences_with(ours, theirs, (&Defaults::default(), defaults), renamed)
}

/// [`differences`], with what each side's default cell looks like.
pub fn differences_with(
    ours: &Document,
    theirs: &Document,
    (own, defaults): (&Defaults, &Defaults),
    renamed: &[(String, String)],
) -> Vec<Difference> {
    let mut out = Vec::new();
    for other in &theirs.sheets {
        if !ours.sheets.iter().any(|s| s.name == other.name) {
            out.push(Difference::OnlyTheirs(other.name.clone()));
        }
    }
    for mine in &ours.sheets {
        let Some(other) = theirs.sheets.iter().find(|s| s.name == mine.name) else {
            let original = renamed
                .iter()
                .find(|(_, to)| *to == mine.name)
                .map_or(&mine.name, |(from, _)| from);
            out.push(Difference::OnlyOurs(original.clone()));
            continue;
        };
        if other.rows_carrying().is_empty() && !mine.rows_carrying().is_empty() {
            out.push(Difference::Unreached(mine.name.clone()));
            continue;
        }
        let cols = mine.used_cols().max(other.used_cols());
        let mut rows: Vec<u32> = mine
            .rows_carrying()
            .into_iter()
            .chain(other.rows_carrying())
            .flatten()
            .collect();
        rows.sort_unstable();
        rows.dedup();
        for row in rows {
            for col in 0..cols {
                let pos = Pos::new(row, col);
                let (a, b) = (
                    cell(mine, pos, ours.null_date, own),
                    cell(other, pos, theirs.null_date, defaults),
                );
                if !same_style(&a.style, &b.style) {
                    out.push(Difference::Style(
                        mine.name.clone(),
                        pos,
                        a.style.clone(),
                        b.style.clone(),
                    ));
                }
                if !same(&a, &b) {
                    out.push(Difference::Cell(mine.name.clone(), pos, a, b));
                }
            }
        }
        for track in tracks(mine, other) {
            out.push(Difference::Track(mine.name.clone(), track));
        }
    }
    out
}

/// The columns and rows on which two sheets disagree. A size is compared where **ours** carries
/// one — the oracle carries one for every track, measuring unsized rows in its own fonts — at loop
/// C's tolerance of a tenth of a millimetre; hidden-ness everywhere.
pub fn tracks(mine: &Sheet, other: &Sheet) -> Vec<Track> {
    let mut out = Vec::new();
    let mm = |s: Option<&str>| s.and_then(style::length_mm);
    let mut size = |axis, index, ours: Option<&str>, theirs: Option<&str>| {
        if let Some(ours) = mm(ours) {
            let theirs = mm(theirs);
            if theirs.is_none_or(|t| (t - ours).abs() >= 0.1) {
                out.push(Track {
                    axis,
                    index,
                    what: TrackDifference::Size { ours, theirs },
                });
            }
        }
    };
    for (col, width) in mine.col_widths() {
        size(Axis::Column, col, Some(width), other.col_width(col));
    }
    // A measured height is Excel's measurement of the row, and the oracle's another in its own
    // fonts (`doc/xlsx-format.md` §4.6): two measurements, not a size either chose.
    for (row, height) in mine
        .row_heights()
        .filter(|(row, _)| !mine.row_measured(*row))
    {
        size(Axis::Row, row, Some(height), other.row_height(row));
    }
    let hidden = |axis, a: Vec<u32>, b: Vec<u32>, out: &mut Vec<Track>| {
        for index in a
            .iter()
            .chain(&b)
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
        {
            let (ours, theirs) = (a.contains(&index), b.contains(&index));
            if ours != theirs {
                out.push(Track {
                    axis,
                    index,
                    what: TrackDifference::Hidden { ours, theirs },
                });
            }
        }
    };
    hidden(
        Axis::Column,
        mine.hidden_cols().collect(),
        other.hidden_cols().collect(),
        &mut out,
    );
    hidden(
        Axis::Row,
        mine.manually_hidden_rows().collect(),
        other.manually_hidden_rows().collect(),
        &mut out,
    );
    out
}

// ---- the oracle ----

/// One converter loop D asks. `convert` turns every `(digest, source)` into
/// `<cache>/<digest>.<output>` and leaves no output for a file it cannot convert.
pub struct Oracle {
    pub name: &'static str,
    /// Below the system temp directory, which is what the containerised oracles mount.
    pub cache: &'static str,
    pub output: &'static str,
    pub version: fn() -> Option<String>,
    pub convert: fn(cache: &Path, pending: &[(String, PathBuf)]),
    /// Fixtures this oracle cannot convert, each with the evidence; asserted both ways.
    pub unconverted: &'static [(&'static str, &'static str)],
    pub divergences: &'static [Divergence],
}

impl Oracle {
    /// Where one oracle build's answers are kept: named by its version string with everything
    /// that is not safe in a path replaced.
    fn cache_dir(&self, version: &str) -> PathBuf {
        let safe: String = version
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        std::env::temp_dir().join(self.cache).join(safe)
    }

    fn answer(&self, cache: &Path, digest: &str) -> PathBuf {
        cache.join(format!("{digest}.{}", self.output))
    }
}

/// Copies each source into a fresh `<cache>/stage` under its digest, so that two fixtures called
/// `sheets.xlsx` in different families cannot overwrite each other's output.
pub fn stage(cache: &Path, pending: &[(String, PathBuf)]) -> (PathBuf, Vec<PathBuf>) {
    let stage = cache.join("stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).unwrap();
    let inputs = pending
        .iter()
        .map(|(digest, source)| {
            let ext = source
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("xlsx");
            let staged = stage.join(format!("{digest}.{ext}"));
            std::fs::copy(source, &staged).unwrap();
            staged
        })
        .collect();
    (stage, inputs)
}

/// The text [`Defaults::of`] searches: the file itself when flat, its styles and content parts
/// when packed.
fn odf_text(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(&bytes)) else {
        return String::from_utf8_lossy(&bytes).into_owned();
    };
    let mut text = String::new();
    for part in ["styles.xml", "content.xml"] {
        if let Ok(mut entry) = zip.by_name(part) {
            let _ = entry.read_to_string(&mut text);
        }
    }
    text
}

// ---- the corpus ----

struct Fixture {
    file: String,
    sha256: String,
}

/// The vendored generated corpus (`ooxmlgen.rs`), every fixture whose manifest says
/// `oracleOpens` — LibreOffice's claim, which also keeps the hostile files out — and that is
/// not refused.
fn corpus() -> (PathBuf, Vec<Fixture>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/corpus");
    let text = std::fs::read_to_string(root.join("manifest.json")).expect("the manifest");
    let json: serde_json::Value = serde_json::from_str(&text).expect("the manifest is JSON");
    let fixtures = json["fixtures"]
        .as_array()
        .expect("a fixtures array")
        .iter()
        .filter(|f| f["oracleOpens"].as_bool() == Some(true) && f.get("error").is_none())
        .map(|f| Fixture {
            file: f["file"].as_str().unwrap().to_owned(),
            sha256: f["sha256"].as_str().unwrap().to_owned(),
        })
        .collect();
    (root, fixtures)
}

/// Every fixture's import against the oracle's conversion. Output is cached by fixture SHA-256
/// and the oracle's version string; at least `floor` fixtures must reach the comparison.
pub fn run(oracle: &Oracle, floor: usize) {
    let Some(version) = (oracle.version)() else {
        eprintln!(
            "skipping loop D ({}): its converter is not on PATH",
            oracle.name
        );
        return;
    };
    let cache = oracle.cache_dir(&version);
    std::fs::create_dir_all(&cache).unwrap();

    let (root, fixtures) = corpus();
    let pending: Vec<(String, PathBuf)> = fixtures
        .iter()
        .filter(|f| !oracle.answer(&cache, &f.sha256).exists())
        .map(|f| (f.sha256.clone(), root.join(&f.file)))
        .collect();
    let converted_now = pending.len();
    (oracle.convert)(&cache, &pending);

    let mut compared = 0usize;
    let mut cells = 0usize;
    let mut failures = Vec::new();
    let mut matched: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unconverted = BTreeSet::new();
    for fixture in &fixtures {
        let theirs_path = oracle.answer(&cache, &fixture.sha256);
        let named = oracle.unconverted.iter().any(|(f, _)| *f == fixture.file);
        let Ok(theirs) = grind_sheet::read_file(&theirs_path) else {
            match named {
                true => {
                    unconverted.insert(fixture.file.as_str());
                }
                false => failures.push(format!(
                    "{}: the oracle produced nothing we can read",
                    fixture.file
                )),
            }
            continue;
        };
        if named {
            failures.push(format!(
                "{}: the oracle converts it now — take it off the unconverted list",
                fixture.file
            ));
        }
        let bytes = std::fs::read(root.join(&fixture.file)).unwrap();
        let (ours, report) = match grind_xlsx::import_bytes(&bytes) {
            Ok(pair) => pair,
            Err(e) => {
                failures.push(format!(
                    "{}: the oracle opens it and we do not: {e}",
                    fixture.file
                ));
                continue;
            }
        };
        compared += 1;
        cells += report.cells;

        let defaults = Defaults::of(&odf_text(&theirs_path));
        for difference in differences(&ours, &theirs, &defaults, &report.renamed) {
            match oracle
                .divergences
                .iter()
                .find(|d| difference.explained_by(d))
            {
                Some(d) => *matched.entry(d.name).or_default() += 1,
                None => failures.push(format!("{}: {}", fixture.file, difference.describe())),
            }
        }
    }

    // The other direction: a divergence nothing matched any more is one the oracle fixed, or
    // a predicate that was never right.
    for d in oracle.divergences {
        if !matched.contains_key(d.name) {
            failures.push(format!(
                "divergence `{}` matched no cell — delete it if the oracle changed ({})",
                d.name, d.why
            ));
        }
    }
    for (file, why) in oracle.unconverted {
        if !fixtures.iter().any(|f| f.file == *file) {
            failures.push(format!(
                "{file} is listed as unconverted and not in the corpus ({why})"
            ));
        }
    }

    eprintln!(
        "loop D ({}): {version}; {compared} workbooks, {cells} cells, {} disagreements, \
         {} named divergences, {} unconverted ({converted_now} converted now, the rest cached)",
        oracle.name,
        failures.len(),
        matched.values().sum::<usize>(),
        unconverted.len(),
    );
    for (name, count) in &matched {
        eprintln!("  divergence ×{count}: {name}");
    }
    for failure in failures.iter().take(100) {
        eprintln!("  {failure}");
    }
    if failures.len() > 100 {
        eprintln!("  ... and {} more", failures.len() - 100);
    }
    assert!(
        compared >= floor,
        "{compared} workbooks reached the comparison, fewer than the ratchet's {floor}"
    );
    assert!(
        failures.is_empty(),
        "{} disagreements with the oracle",
        failures.len()
    );
}

/// Loop D over a directory of workbooks from elsewhere — `GRIND_LOOP_D_CORPUS=<dir>`, skipped
/// without it. Nothing there has a manifest, so this asserts only that every workbook the oracle
/// converts imports, and prints the disagreements by class, most widespread first.
/// `GRIND_LOOP_D_EXAMPLES` is how many files to show per class, four by default.
pub fn run_outside(oracle: &Oracle) {
    let Some(dir) = std::env::var_os("GRIND_LOOP_D_CORPUS") else {
        return;
    };
    let Some(version) = (oracle.version)() else {
        eprintln!(
            "skipping loop D outside ({}): its converter is not on PATH",
            oracle.name
        );
        return;
    };
    let cache = oracle.cache_dir(&version);
    std::fs::create_dir_all(&cache).unwrap();

    let mut files = Vec::new();
    let mut stack = vec![PathBuf::from(dir)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                ["xlsx", "xlsm", "ods", "fods"]
                    .iter()
                    .any(|x| e.eq_ignore_ascii_case(x))
            }) {
                files.push(path);
            }
        }
    }
    files.sort();
    let digests: Vec<(String, PathBuf)> = files
        .iter()
        .filter_map(|path| Some((sha256(path)?, path.clone())))
        .collect();
    let pending: Vec<(String, PathBuf)> = digests
        .iter()
        .filter(|(digest, _)| {
            !oracle.answer(&cache, digest).exists()
                && !cache.join(format!("{digest}.unopened")).exists()
        })
        .cloned()
        .collect();
    // A workbook the oracle cannot open is remembered as such, rather than retried every run.
    for chunk in pending.chunks(20) {
        (oracle.convert)(&cache, chunk);
        for (digest, _) in chunk {
            if !oracle.answer(&cache, digest).exists() {
                let _ = std::fs::write(cache.join(format!("{digest}.unopened")), "");
            }
        }
    }

    let mut refused = Vec::new();
    let mut compared = 0usize;
    // class → (files, instances, examples)
    let mut classes: BTreeMap<String, (BTreeSet<String>, usize, Vec<String>)> = BTreeMap::new();
    let mut matched: BTreeMap<&str, usize> = BTreeMap::new();
    let examples: usize = std::env::var("GRIND_LOOP_D_EXAMPLES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(4);
    for (digest, path) in &digests {
        let theirs_path = oracle.answer(&cache, digest);
        let Ok(theirs) = grind_sheet::read_file(&theirs_path) else {
            continue;
        };
        let file = path.display().to_string();
        let bytes = std::fs::read(path).unwrap();
        // An ODF file is read as it is: the comparison is then our reading of it against our
        // reading of the oracle's re-save, which states everything it inherited.
        let odf = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("ods") || e.eq_ignore_ascii_case("fods"));
        let read = match odf {
            true => grind_sheet::read_bytes(&file, &bytes)
                .map(|doc| (doc, Vec::new()))
                .map_err(|e| e.to_string()),
            false => grind_xlsx::import_bytes(&bytes)
                .map(|(doc, report)| (doc, report.renamed))
                .map_err(|e| e.to_string()),
        };
        let (ours, renamed) = match read {
            Ok(pair) => pair,
            Err(e) => {
                refused.push(format!("{file}: {e}"));
                continue;
            }
        };
        compared += 1;
        let defaults = Defaults::of(&odf_text(&theirs_path));
        let mine = match odf {
            true => Defaults::of(&odf_text(path)),
            false => Defaults::default(),
        };
        for difference in differences_with(&ours, &theirs, (&mine, &defaults), &renamed) {
            if let Some(d) = oracle
                .divergences
                .iter()
                .find(|d| difference.explained_by(d))
            {
                *matched.entry(d.name).or_default() += 1;
                continue;
            }
            let entry = classes.entry(difference.class()).or_default();
            entry.0.insert(file.clone());
            entry.1 += 1;
            if entry.2.len() < examples && !entry.2.iter().any(|e| e.starts_with(&file)) {
                entry.2.push(format!("{file}: {}", difference.describe()));
            }
        }
    }

    eprintln!(
        "loop D outside ({}): {version}; {compared} workbooks compared",
        oracle.name
    );
    for (name, count) in &matched {
        eprintln!("  divergence ×{count}: {name}");
    }
    let mut ranked: Vec<_> = classes.into_iter().collect();
    ranked.sort_by_key(|(_, (files, n, _))| std::cmp::Reverse((files.len(), *n)));
    for (class, (files, n, examples)) in &ranked {
        eprintln!("{class}: {} files, {n} times", files.len());
        for example in examples {
            eprintln!("    {example}");
        }
    }
    for line in &refused {
        eprintln!("  refused: {line}");
    }
    assert!(
        refused.is_empty(),
        "{} workbooks the oracle opens and we do not",
        refused.len()
    );
}

/// The cache's key, from `sha256sum` rather than a dependency for a loop that runs on request.
fn sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg(path).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.split_whitespace().next()?.to_owned())
}

impl Difference {
    /// A coarse name for what kind of disagreement this is, so a corpus's thousands of them
    /// can be counted by cause.
    pub fn class(&self) -> String {
        match self {
            Difference::Cell(_, _, a, b) => {
                if a.value != b.value {
                    let shape = |v: &CellValue| match v {
                        CellValue::Empty => "empty",
                        CellValue::Number(_) => "number",
                        CellValue::Text(_) => "text",
                        CellValue::Bool(_) => "bool",
                    };
                    format!(
                        "cell value: ours {} theirs {}{}",
                        shape(&a.value),
                        shape(&b.value),
                        if b.formula { " (formula)" } else { "" }
                    )
                } else if a.kind != b.kind {
                    format!("cell kind: ours {:?} theirs {:?}", a.kind, b.kind)
                } else {
                    "cell display".to_owned()
                }
            }
            Difference::Style(_, _, a, b) => {
                let plain = CellStyle::default();
                let (a, b) = (a.as_ref().unwrap_or(&plain), b.as_ref().unwrap_or(&plain));
                let mut fields = Vec::new();
                macro_rules! field {
                    ($($f:ident),*) => {$(
                        if a.$f != b.$f { fields.push(stringify!($f)); }
                    )*};
                }
                field!(
                    font_weight,
                    font_style,
                    font_size,
                    color,
                    background,
                    align,
                    vertical_align,
                    wrap,
                    underline,
                    line_through,
                    indent,
                    rotation,
                    borders
                );
                format!("style: {}", fields.join(","))
            }
            Difference::Track(_, track) => match (&track.axis, &track.what) {
                (axis, TrackDifference::Size { .. }) => format!("{axis:?} size"),
                (axis, TrackDifference::Hidden { ours, .. }) => {
                    format!("{axis:?} hidden: ours {ours}")
                }
            },
            Difference::OnlyOurs(_) => "sheet only ours".to_owned(),
            Difference::OnlyTheirs(_) => "sheet only theirs".to_owned(),
            Difference::Unreached(_) => "sheet unreached".to_owned(),
        }
    }
}
